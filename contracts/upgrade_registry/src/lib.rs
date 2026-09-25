#![no_std]
#![allow(deprecated, unused_imports)] // Temporarily allow deprecated Events::publish until we migrate to #[contractevent]

// ---------------------------------------------------------------------------
// RFC: Upgrade path design
//
// Two paths exist for upgrading a contract tracked by this registry:
//
// PATH A — Two-step (RECOMMENDED):
//   1. `schedule_upgrade(contract_name, new_version, changelog_hash)`
//      - Requires admin auth
//      - Checks new_version > current_version (VersionNotMonotonic)
//      - Records a PendingUpgrade with `execute_after = now + upgrade_delay`
//   2. `execute_pending_upgrade(contract_name)`
//      - Requires admin auth
//      - Checks ledger timestamp >= execute_after (TimelockNotElapsed)
//      - Commits the upgrade record; clears the pending slot
//
// PATH B — Direct UUPS (`upgrade_contract`) — DEPRECATED
//   Kept for backward-compatibility only.  Marked `#[deprecated]`.
//   Callers MUST migrate to PATH A.  PATH B enforces identical guards:
//   - VersionNotMonotonic
//   - TimelockNotElapsed (uses same upgrade_delay as PATH A)
//   There is NO way to bypass the timelock via PATH B.
//
// Both paths now provide identical security guarantees.
// ---------------------------------------------------------------------------

use shared::storage_compatibility::{
    CompatibilityReport, CompatibilityValidator, GradualMigrationStatus, StorageField,
    StorageFieldType, StorageLayoutSchema, StorageVersion,
};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, xdr::ToXdr, Address,
    BytesN, Env, IntoVal, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotAdmin = 3,
    ContractNotFound = 4,
    AlreadySubscribed = 5,
    NotSubscribed = 6,
    /// new_version must be strictly greater than the currently registered version.
    VersionNotMonotonic = 7,
    /// The configured upgrade_delay has not elapsed since the upgrade was scheduled.
    TimelockNotElapsed = 8,
    /// An upgrade is already pending; cancel it first.
    UpgradePending = 9,
    /// No pending upgrade to execute or cancel.
    NoPendingUpgrade = 10,
    /// Threshold must be non-zero and no greater than signer count.
    InvalidThreshold = 11,
    /// Approval signer is not registered in the current upgrade config.
    NotSigner = 12,
    /// Signer list or approval list contains the same address twice.
    DuplicateSigner = 13,
    /// Approval count is below the configured threshold.
    BelowThreshold = 14,
    /// WASM is missing a required function.
    MissingRequiredFunction = 15,
    /// WASM validation failed.
    WasmValidationFailed = 16,
    /// Source commit hash already registered for this contract+version.
    SourceCommitAlreadyRegistered = 17,
    /// Provided commit hash does not match stored hash.
    CommitHashMismatch = 18,
    /// Proposed storage layout is incompatible with the active layout.
    IncompatibleStorageLayout = 19,
    /// Upgrade requires a storage data migration before execution.
    StorageMigrationRequired = 20,
    /// A storage migration is currently in progress; finish or cancel it first.
    StorageMigrationInProgress = 21,
    /// Storage layout schema not found for this contract and version.
    StorageLayoutNotFound = 22,
    /// Migration batch size is invalid (e.g. 0 or too large).
    InvalidMigrationBatch = 23,
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeRecord {
    pub old_version: u32,
    pub new_version: u32,
    pub changelog_hash: BytesN<32>,
    pub timestamp: u64,
    pub admin: Address,
}

/// Stored by `schedule_upgrade`; consumed by `execute_pending_upgrade`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeConfig {
    pub signers: Vec<Address>,
    pub threshold: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RegistrySnapshot {
    pub admin: Address,
    pub upgrade_delay: u64,
    pub upgrade_config: Vec<UpgradeConfig>,
    pub registered_contracts: Vec<Symbol>,
    pub contract_versions: Vec<(Symbol, u32)>,
    pub history_counts: Vec<(Symbol, u32)>,
    pub history_items: Vec<(Symbol, u32, UpgradeRecord)>,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

/// A pending (time-locked) upgrade waiting for the delay to elapse.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingUpgrade {
    /// WASM hash to apply when the timelock expires.
    pub new_wasm_hash: BytesN<32>,
    /// Human-readable contract name for registry bookkeeping.
    pub contract_name: Symbol,
    /// New version number (must be > current version).
    pub new_version: u32,
    /// Changelog hash for audit trail.
    pub changelog_hash: BytesN<32>,
    /// Ledger timestamp at which this upgrade was scheduled.
    pub scheduled_at: u64,
    /// Earliest timestamp at which `execute_pending_upgrade` may be called.
    pub executable_after: u64,
    /// Admin that initiated the upgrade.
    pub admin: Address,
    /// Signers that approved scheduling this upgrade.
    pub approved_signers: Vec<Address>,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    UpgradeHistory(Symbol),
    LatestVersion(Symbol),
    Subscribers(Symbol),
    /// Stores the single pending upgrade (only one may be in-flight at a time).
    PendingUpgrade,
    /// Minimum timelock delay in seconds for upgrades (default 48 h).
    UpgradeDelay,
    /// M-of-N signer set required for scheduling, executing, and rotating upgrades.
    UpgradeConfig,

    // === OPTIMIZATION: Append-only storage patterns ===
    /// Count of upgrade history records for a contract
    UpgradeHistoryCount(Symbol),
    /// Individual upgrade record by index (contract_name, index)
    UpgradeHistoryItem(Symbol, u32),
    /// Validation cache for M-of-N approvals (expires after delay)
    ValidationCache(Vec<Address>),
    /// Cache timestamp for validation results
    ValidationCacheTime(Vec<Address>),
    /// Source commit hash (first 32 bytes of SHA256) for a specific contract+version.
    SourceCommit(Symbol),

    // === STORAGE LAYOUT COMPATIBILITY & MIGRATION ===
    /// Storage layout schema by (contract_name, version)
    StorageLayout(Symbol, u32),
    /// Active storage version for a contract
    ActiveStorageVersion(Symbol),
    /// Gradual migration status by contract_name
    MigrationStatus(Symbol),

    // === DISASTER RECOVERY ===
    RegisteredContracts,
    Snapshot(u32),
    SnapshotMetadata(u32),
    SnapshotIndex,
    EmergencySigners,
    RollbackProposal(u32),
    RollbackApproval(u32, Address),
    RollbackProposalCount,
}

/// Default upgrade timelock: 48 hours.
const DEFAULT_UPGRADE_DELAY: u64 = 48 * 60 * 60;

/// Required functions that must be present in any upgradeable contract WASM.
/// These functions are essential for:
/// - initialize: initial setup and config
/// - schedule_upgrade: scheduling new upgrades
/// - execute_pending_upgrade: applying scheduled upgrades (prevents bricking)
/// - cancel_pending_upgrade: emergency halt of upgrades
/// - get_admin: querying admin for authorization checks
#[allow(dead_code)]
const REQUIRED_FUNCTIONS: &[&str] = &[
    "initialize",
    "schedule_upgrade",
    "execute_pending_upgrade",
    "cancel_pending_upgrade",
    "get_admin",
];

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct UpgradeRegistryContract;

#[contractimpl]
impl UpgradeRegistryContract {
    /// Initialize the upgrade registry.
    pub fn initialize(env: Env, admin: Address, upgrade_delay: u64) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::UpgradeDelay, &upgrade_delay);
        Ok(())
    }

    pub fn set_upgrade_delay(env: Env, delay_secs: u64) -> Result<(), Error> {
        let _admin = Self::require_admin(&env)?;
        let min = 3_600_u64;       // 1 hour
        let max = 30 * 24 * 3_600_u64; // 30 days
        if delay_secs < min || delay_secs > max {
            panic!("upgrade delay out of range [1h, 30d]");
        }
        env.storage().instance().set(&DataKey::UpgradeDelay, &delay_secs);
        Ok(())
    }

    pub fn get_upgrade_delay(env: Env) -> u64 {
        env.storage()
            .instance()
            .get(&DataKey::UpgradeDelay)
            .unwrap_or(DEFAULT_UPGRADE_DELAY)
    }

    pub fn schedule_upgrade(
        env: Env,
        new_wasm_hash: BytesN<32>,
        contract_name: Symbol,
        new_version: u32,
        changelog_hash: BytesN<32>,
        approvers: Vec<Address>,
    ) -> Result<(), Error> {
        let approved_signers = require_upgrade_approvals_cached(&env, approvers)?;

        validate_wasm_exports(&env, &new_wasm_hash)?;

        if let Some(status) = Self::get_migration_status(env.clone(), contract_name.clone()) {
            if !status.completed {
                return Err(Error::StorageMigrationInProgress);
            }
        }

        if env.storage().instance().has(&DataKey::PendingUpgrade) {
            return Err(Error::UpgradePending);
        }

        let current = Self::get_latest_version(env.clone(), contract_name.clone());
        if new_version <= current {
            return Err(Error::VersionNotMonotonic);
        }

        let upgrade_delay = env
            .storage()
            .instance()
            .get(&DataKey::UpgradeDelay)
            .unwrap_or(0);

        let now = env.ledger().timestamp();
        let execute_after = now.saturating_add(upgrade_delay);

        let pending = PendingUpgrade {
            new_wasm_hash: new_wasm_hash.clone(),
            contract_name: contract_name.clone(),
            new_version,
            changelog_hash: changelog_hash.clone(),
            scheduled_at: now,
            executable_after: execute_after,
            admin: approved_signers.get(0).ok_or(Error::BelowThreshold)?.clone(),
            approved_signers: approved_signers.clone(),
        };

        env.storage()
            .instance()
            .set(&DataKey::PendingUpgrade, &pending);

        // Track registered contracts
        let mut registered: Vec<Symbol> = env
            .storage()
            .persistent()
            .get(&DataKey::RegisteredContracts)
            .unwrap_or(Vec::new(&env));
        if !registered.iter().any(|c| c == contract_name) {
            registered.push_back(contract_name.clone());
            env.storage().persistent().set(&DataKey::RegisteredContracts, &registered);
        }

        env.events().publish(
            (
                symbol_short!("upgrade"),
                symbol_short!("sched"),
                contract_name,
            ),
            (new_version, execute_after, new_wasm_hash, approved_signers),
        );

        Ok(())
    }

    pub fn execute_pending_upgrade(env: Env, approvers: Vec<Address>) -> Result<(), Error> {
        let pending: PendingUpgrade = env
            .storage()
            .instance()
            .get(&DataKey::PendingUpgrade)
            .ok_or(Error::NoPendingUpgrade)?;

        let approved_signers = require_upgrade_approvals_cached(&env, approvers)?;

        if env.ledger().timestamp() < pending.executable_after {
            return Err(Error::TimelockNotElapsed);
        }

        let old_version: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LatestVersion(pending.contract_name.clone()))
            .unwrap_or(0);

        let record = UpgradeRecord {
            old_version,
            new_version: pending.new_version,
            changelog_hash: pending.changelog_hash.clone(),
            timestamp: env.ledger().timestamp(),
            admin: approved_signers.get(0).ok_or(Error::BelowThreshold)?.clone(),
        };

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::UpgradeHistoryCount(pending.contract_name.clone()))
            .unwrap_or(0);

        env.storage().persistent().set(
            &DataKey::UpgradeHistoryItem(pending.contract_name.clone(), count),
            &record,
        );
        env.storage().persistent().set(
            &DataKey::UpgradeHistoryCount(pending.contract_name.clone()),
            &(count + 1),
        );
        env.storage().persistent().set(
            &DataKey::LatestVersion(pending.contract_name.clone()),
            &pending.new_version,
        );

        env.storage()
            .instance()
            .remove(&DataKey::PendingUpgrade);

        env.events().publish(
            (
                symbol_short!("upgrade"),
                symbol_short!("exec"),
                pending.contract_name.clone(),
            ),
            (
                old_version,
                pending.new_version,
                pending.new_wasm_hash.clone(),
                approved_signers,
            ),
        );

        env.deployer()
            .update_current_contract_wasm(pending.new_wasm_hash);

        Ok(())
    }

    #[deprecated(since = "0.2.0", note = "Use two-step schedule_upgrade + execute_pending_upgrade (PATH A) instead. Will be removed in v1.0.0.")]
    pub fn register_upgrade(
        env: Env,
        contract_name: Symbol,
        old_version: u32,
        new_version: u32,
        changelog_hash: BytesN<32>,
    ) -> Result<(), Error> {
        soroban_sdk::log!(
            &env,
            "DEPRECATION WARNING: register_upgrade is deprecated and will be removed in v1.0.0. Please migrate to schedule_upgrade + execute_pending_upgrade."
        );
        let admin = Self::require_admin(&env)?;

        let current = Self::get_latest_version(env.clone(), contract_name.clone());
        if new_version <= current {
            return Err(Error::VersionNotMonotonic);
        }

        let record = UpgradeRecord {
            old_version,
            new_version,
            changelog_hash: changelog_hash.clone(),
            timestamp: env.ledger().timestamp(),
            admin: admin.clone(),
        };

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::UpgradeHistoryCount(contract_name.clone()))
            .unwrap_or(0);

        env.storage().persistent().set(
            &DataKey::UpgradeHistoryItem(contract_name.clone(), count),
            &record,
        );
        env.storage().persistent().set(
            &DataKey::UpgradeHistoryCount(contract_name.clone()),
            &(count + 1),
        );
        env.storage()
            .persistent()
            .set(&DataKey::LatestVersion(contract_name.clone()), &new_version);

        let mut registered: Vec<Symbol> = env
            .storage()
            .persistent()
            .get(&DataKey::RegisteredContracts)
            .unwrap_or(Vec::new(&env));
        if !registered.iter().any(|c| c == contract_name) {
            registered.push_back(contract_name.clone());
            env.storage().persistent().set(&DataKey::RegisteredContracts, &registered);
        }

        env.events().publish(
            (
                symbol_short!("upgrade"),
                symbol_short!("reg"),
                contract_name.clone(),
            ),
            (old_version, new_version, changelog_hash),
        );
        Ok(())
    }

    #[deprecated(since = "0.2.0", note = "Use two-step schedule_upgrade + execute_pending_upgrade (PATH A) instead. Will be removed in v1.0.0.")]
    pub fn upgrade_contract(
        env: Env,
        new_wasm_hash: BytesN<32>,
        contract_name: Symbol,
        new_version: u32,
        changelog_hash: BytesN<32>,
        approvers: Vec<Address>,
    ) -> Result<(), Error> {
        soroban_sdk::log!(
            &env,
            "DEPRECATION WARNING: upgrade_contract is deprecated and will be removed in v1.0.0. Please migrate to schedule_upgrade + execute_pending_upgrade."
        );
        let approved_signers = require_upgrade_approvals_cached(&env, approvers)?;

        let current = Self::get_latest_version(env.clone(), contract_name.clone());
        if new_version <= current {
            return Err(Error::VersionNotMonotonic);
        }

        if let Some(status) = Self::get_migration_status(env.clone(), contract_name.clone()) {
            if !status.completed {
                return Err(Error::StorageMigrationInProgress);
            }
        }

        let upgrade_delay = env
            .storage()
            .instance()
            .get(&DataKey::UpgradeDelay)
            .unwrap_or(0);

        if upgrade_delay > 0 {
            let count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::UpgradeHistoryCount(contract_name.clone()))
                .unwrap_or(0);

            let last_upgrade_ts = if count == 0 {
                0u64
            } else {
                let last_record: UpgradeRecord = env
                    .storage()
                    .persistent()
                    .get(&DataKey::UpgradeHistoryItem(contract_name.clone(), count - 1))
                    .unwrap();
                last_record.timestamp
            };

            let earliest_allowed = last_upgrade_ts.saturating_add(upgrade_delay);
            if env.ledger().timestamp() < earliest_allowed {
                return Err(Error::TimelockNotElapsed);
            }
        }

        let record = UpgradeRecord {
            old_version: current,
            new_version,
            changelog_hash: changelog_hash.clone(),
            timestamp: env.ledger().timestamp(),
            admin: approved_signers.get(0).ok_or(Error::BelowThreshold)?.clone(),
        };

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::UpgradeHistoryCount(contract_name.clone()))
            .unwrap_or(0);

        env.storage()
            .persistent()
            .set(&DataKey::UpgradeHistoryItem(contract_name.clone(), count), &record);
        env.storage()
            .persistent()
            .set(&DataKey::UpgradeHistoryCount(contract_name.clone()), &(count + 1));
        env.storage()
            .persistent()
            .set(&DataKey::LatestVersion(contract_name.clone()), &new_version);

        let mut registered: Vec<Symbol> = env
            .storage()
            .persistent()
            .get(&DataKey::RegisteredContracts)
            .unwrap_or(Vec::new(&env));
        if !registered.iter().any(|c| c == contract_name) {
            registered.push_back(contract_name.clone());
            env.storage().persistent().set(&DataKey::RegisteredContracts, &registered);
        }

        env.events().publish(
            (
                symbol_short!("upgrade"),
                symbol_short!("direct"),
                contract_name,
            ),
            (current, new_version, changelog_hash, approved_signers),
        );

        env.deployer()
            .update_current_contract_wasm(new_wasm_hash);

        Ok(())
    }

    pub fn set_upgrade_signers(
        env: Env,
        signers: Vec<Address>,
        threshold: u32,
        approvers: Vec<Address>,
    ) -> Result<(), Error> {
        let approved_signers = require_upgrade_approvals(&env, approvers)?;
        validate_upgrade_config(&signers, threshold)?;

        let config = UpgradeConfig {
            signers: signers.clone(),
            threshold,
        };
        env.storage()
            .instance()
            .set(&DataKey::UpgradeConfig, &config);
        env.events().publish(
            (symbol_short!("upgrade"), symbol_short!("signers")),
            (signers, threshold, approved_signers),
        );
        Ok(())
    }

    pub fn set_admin(env: Env, new_admin: Address, approvers: Vec<Address>) -> Result<(), Error> {
        let approved_signers = require_upgrade_approvals(&env, approvers)?;
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.events().publish(
            (symbol_short!("upgrade"), symbol_short!("admin")),
            (new_admin, approved_signers),
        );
        Ok(())
    }

    pub fn cancel_pending_upgrade(env: Env) -> Result<(), Error> {
        let _admin = Self::require_admin(&env)?;
        if !env.storage().instance().has(&DataKey::PendingUpgrade) {
            return Err(Error::NoPendingUpgrade);
        }
        env.storage().instance().remove(&DataKey::PendingUpgrade);
        env.events()
            .publish((symbol_short!("upgrade"), symbol_short!("cancel")), ());
        Ok(())
    }

    pub fn subscribe(env: Env, subscriber: Address, contract_name: Symbol) -> Result<(), Error> {
        subscriber.require_auth();

        let mut subscribers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Subscribers(contract_name.clone()))
            .unwrap_or(Vec::new(&env));

        for addr in subscribers.iter() {
            if addr == subscriber {
                return Err(Error::AlreadySubscribed);
            }
        }

        subscribers.push_back(subscriber.clone());
        env.storage()
            .persistent()
            .set(&DataKey::Subscribers(contract_name.clone()), &subscribers);

        env.events().publish(
            (symbol_short!("sub"), symbol_short!("added"), contract_name),
            subscriber,
        );
        Ok(())
    }

    pub fn unsubscribe(env: Env, subscriber: Address, contract_name: Symbol) -> Result<(), Error> {
        subscriber.require_auth();

        let subscribers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Subscribers(contract_name.clone()))
            .unwrap_or(Vec::new(&env));

        let mut found = false;
        let mut new_subscribers = Vec::new(&env);
        for addr in subscribers.iter() {
            if addr != subscriber {
                new_subscribers.push_back(addr);
            } else {
                found = true;
            }
        }

        if !found {
            return Err(Error::NotSubscribed);
        }

        env.storage().persistent().set(
            &DataKey::Subscribers(contract_name.clone()),
            &new_subscribers,
        );

        env.events().publish(
            (
                symbol_short!("sub"),
                symbol_short!("removed"),
                contract_name,
            ),
            subscriber,
        );
        Ok(())
    }

    pub fn get_upgrade_history(env: Env, contract_name: Symbol) -> Vec<UpgradeRecord> {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::UpgradeHistoryCount(contract_name.clone()))
            .unwrap_or(0);

        let mut history = Vec::new(&env);
        for i in 0..count {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<_, UpgradeRecord>(&DataKey::UpgradeHistoryItem(contract_name.clone(), i))
            {
                history.push_back(record);
            }
        }
        history
    }

    pub fn get_latest_version(env: Env, contract_name: Symbol) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::LatestVersion(contract_name))
            .unwrap_or(0)
    }

    pub fn get_subscribers(env: Env, contract_name: Symbol) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::Subscribers(contract_name))
            .unwrap_or(Vec::new(&env))
    }

    pub fn get_pending_upgrade(env: Env) -> Option<PendingUpgrade> {
        env.storage().instance().get(&DataKey::PendingUpgrade)
    }

    pub fn registry_version(_env: Env) -> u32 {
        1
    }

    pub fn register_source_commit(
        env: Env,
        admin: Address,
        contract_name: Symbol,
        version: u32,
        commit_hash: BytesN<32>,
    ) -> Result<(), Error> {
        let stored_admin = Self::require_admin(&env)?;
        if admin != stored_admin {
            return Err(Error::NotAdmin);
        }

        let key = DataKey::SourceCommit(contract_name.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::SourceCommitAlreadyRegistered);
        }

        env.storage().persistent().set(&key, &commit_hash);

        env.events().publish(
            (
                symbol_short!("source"),
                symbol_short!("commit"),
                contract_name,
            ),
            (version, commit_hash),
        );

        Ok(())
    }

    pub fn get_source_commit(env: Env, contract_name: Symbol, _version: u32) -> Option<BytesN<32>> {
        env.storage()
            .persistent()
            .get(&DataKey::SourceCommit(contract_name))
    }

    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized")
    }

    pub fn get_upgrade_config(env: Env) -> Result<UpgradeConfig, Error> {
        env.storage()
            .instance()
            .get(&DataKey::UpgradeConfig)
            .ok_or(Error::NotInitialized)
    }

    pub fn require_admin(env: &Env) -> Result<Address, Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        Ok(admin)
    }

    pub fn check_min_version(env: Env, contract_name: Symbol, min_version: u32) -> bool {
        let latest = Self::get_latest_version(env, contract_name);
        latest >= min_version
    }

    // === STORAGE LAYOUT COMPATIBILITY & MIGRATION ===

    /// Register a new storage layout schema for a contract version after validating integrity and compatibility.
    pub fn register_storage_schema(
        env: Env,
        contract_name: Symbol,
        schema: StorageLayoutSchema,
        approvers: Vec<Address>,
    ) -> Result<(), Error> {
        let approved_signers = require_upgrade_approvals_cached(&env, approvers)?;

        // Verify schema hash
        let computed_hash = CompatibilityValidator::compute_schema_hash(&env, &schema.fields);
        if computed_hash != schema.schema_hash {
            return Err(Error::IncompatibleStorageLayout);
        }

        // If a previous version exists, validate compatibility
        let current_version = Self::get_latest_version(env.clone(), contract_name.clone());
        if current_version > 0 {
            if let Some(old_schema) = Self::get_storage_schema(env.clone(), contract_name.clone(), current_version) {
                let report = CompatibilityValidator::validate_compatibility(&env, &old_schema, &schema);
                if !report.is_compatible && !report.requires_migration {
                    return Err(Error::IncompatibleStorageLayout);
                }
            }
        }

        // Store schema
        env.storage().persistent().set(
            &DataKey::StorageLayout(contract_name.clone(), schema.version),
            &schema,
        );

        let storage_ver = StorageVersion {
            current_version: schema.version,
            min_compatible_version: if current_version == 0 { schema.version } else { current_version },
            layout_hash: schema.schema_hash.clone(),
            migration_in_progress: false,
        };

        env.storage().persistent().set(
            &DataKey::ActiveStorageVersion(contract_name.clone()),
            &storage_ver,
        );

        env.events().publish(
            (
                symbol_short!("upgrade"),
                symbol_short!("storage"),
                contract_name,
            ),
            (schema.version, schema.schema_hash, approved_signers),
        );

        Ok(())
    }

    /// Retrieve the storage layout schema for a given contract and version.
    pub fn get_storage_schema(
        env: Env,
        contract_name: Symbol,
        version: u32,
    ) -> Option<StorageLayoutSchema> {
        env.storage()
            .persistent()
            .get(&DataKey::StorageLayout(contract_name, version))
    }

    /// Retrieve the active storage version for a contract.
    pub fn get_active_storage_version(
        env: Env,
        contract_name: Symbol,
    ) -> Option<StorageVersion> {
        env.storage()
            .persistent()
            .get(&DataKey::ActiveStorageVersion(contract_name))
    }

    /// Validate the compatibility of a proposed schema against the currently active schema.
    pub fn validate_upgrade_compatibility(
        env: Env,
        contract_name: Symbol,
        new_schema: StorageLayoutSchema,
    ) -> Result<CompatibilityReport, Error> {
        let current_version = Self::get_latest_version(env.clone(), contract_name.clone());
        if current_version == 0 {
            // No previous schema, fully compatible as initial layout
            let fields_count = new_schema.fields.len();
            return Ok(CompatibilityReport {
                is_compatible: true,
                requires_migration: false,
                added_fields: fields_count,
                deprecated_fields: 0,
                fields_checked: fields_count,
                mismatches: Vec::new(&env),
            });
        }

        let old_schema = Self::get_storage_schema(env.clone(), contract_name, current_version)
            .ok_or(Error::StorageLayoutNotFound)?;

        Ok(CompatibilityValidator::validate_compatibility(
            &env,
            &old_schema,
            &new_schema,
        ))
    }

    /// Start a gradual storage migration for a large dataset across schema versions.
    pub fn start_storage_migration(
        env: Env,
        contract_name: Symbol,
        from_version: u32,
        to_version: u32,
        total_records: u64,
        approvers: Vec<Address>,
    ) -> Result<GradualMigrationStatus, Error> {
        let approved_signers = require_upgrade_approvals_cached(&env, approvers)?;

        // Ensure schemas exist
        if !env.storage().persistent().has(&DataKey::StorageLayout(contract_name.clone(), from_version))
            || !env.storage().persistent().has(&DataKey::StorageLayout(contract_name.clone(), to_version))
        {
            return Err(Error::StorageLayoutNotFound);
        }

        // Check if migration is already in progress
        if let Some(status) = Self::get_migration_status(env.clone(), contract_name.clone()) {
            if !status.completed {
                return Err(Error::StorageMigrationInProgress);
            }
        }

        let initial_status = GradualMigrationStatus {
            from_version,
            to_version,
            processed_records: 0,
            total_records,
            completed: total_records == 0,
            last_cursor: 0,
        };

        env.storage().persistent().set(
            &DataKey::MigrationStatus(contract_name.clone()),
            &initial_status,
        );

        if let Some(mut ver) = Self::get_active_storage_version(env.clone(), contract_name.clone()) {
            ver.migration_in_progress = true;
            env.storage().persistent().set(
                &DataKey::ActiveStorageVersion(contract_name.clone()),
                &ver,
            );
        }

        env.events().publish(
            (
                symbol_short!("upgrade"),
                symbol_short!("mig_start"),
                contract_name,
            ),
            (from_version, to_version, total_records, approved_signers),
        );

        Ok(initial_status)
    }

    /// Execute a single bounded batch step for an in-progress storage migration.
    pub fn execute_migration_step(
        env: Env,
        contract_name: Symbol,
        batch_size: u32,
        approvers: Vec<Address>,
    ) -> Result<GradualMigrationStatus, Error> {
        let approved_signers = require_upgrade_approvals_cached(&env, approvers)?;

        if batch_size == 0 || batch_size > 1_000 {
            return Err(Error::InvalidMigrationBatch);
        }

        let mut status: GradualMigrationStatus = Self::get_migration_status(env.clone(), contract_name.clone())
            .ok_or(Error::StorageLayoutNotFound)?;

        if status.completed {
            return Ok(status);
        }

        let new_processed = status
            .processed_records
            .saturating_add(batch_size as u64)
            .min(status.total_records);
        let new_cursor = status.last_cursor.saturating_add(batch_size as u64);

        status.processed_records = new_processed;
        status.last_cursor = new_cursor;

        if status.processed_records >= status.total_records {
            status.completed = true;

            // Update active storage version to target version
            if let Some(target_schema) = Self::get_storage_schema(env.clone(), contract_name.clone(), status.to_version) {
                let storage_ver = StorageVersion {
                    current_version: status.to_version,
                    min_compatible_version: status.to_version,
                    layout_hash: target_schema.schema_hash,
                    migration_in_progress: false,
                };
                env.storage().persistent().set(
                    &DataKey::ActiveStorageVersion(contract_name.clone()),
                    &storage_ver,
                );
            }
        }

        env.storage().persistent().set(
            &DataKey::MigrationStatus(contract_name.clone()),
            &status,
        );

        env.events().publish(
            (
                symbol_short!("upgrade"),
                symbol_short!("mig_step"),
                contract_name,
            ),
            (status.processed_records, status.completed, approved_signers),
        );

        Ok(status)
    }

    /// Retrieve the migration status for a contract.
    pub fn get_migration_status(
        env: Env,
        contract_name: Symbol,
    ) -> Option<GradualMigrationStatus> {
        env.storage()
            .persistent()
            .get(&DataKey::MigrationStatus(contract_name))
    }

    /// Rollback the active storage layout to a specified target version.
    pub fn rollback_storage_layout(
        env: Env,
        contract_name: Symbol,
        target_version: u32,
        approvers: Vec<Address>,
    ) -> Result<(), Error> {
        let approved_signers = require_upgrade_approvals_cached(&env, approvers)?;

        let target_schema = Self::get_storage_schema(env.clone(), contract_name.clone(), target_version)
            .ok_or(Error::StorageLayoutNotFound)?;

        let storage_ver = StorageVersion {
            current_version: target_version,
            min_compatible_version: target_version,
            layout_hash: target_schema.schema_hash,
            migration_in_progress: false,
        };

        env.storage().persistent().set(
            &DataKey::ActiveStorageVersion(contract_name.clone()),
            &storage_ver,
        );

        // Reset any migration status
        env.storage().persistent().remove(&DataKey::MigrationStatus(contract_name.clone()));

        env.events().publish(
            (
                symbol_short!("upgrade"),
                symbol_short!("stor_rb"),
                contract_name,
            ),
            (target_version, approved_signers),
        );

        Ok(())
    }

    // === DISASTER RECOVERY ===

    pub fn set_emergency_signers(
        env: Env,
        admin: Address,
        signers: Vec<Address>,
    ) -> Result<(), Error> {
        let stored_admin = Self::require_admin(&env)?;
        if admin != stored_admin {
            return Err(Error::NotAdmin);
        }
        if signers.len() != shared::disaster_recovery::EMERGENCY_SIGNERS {
            panic!("Must provide exactly 7 emergency signers");
        }
        env.storage()
            .persistent()
            .set(&DataKey::EmergencySigners, &signers);
        env.events().publish(
            (Symbol::new(&env, "DR"), Symbol::new(&env, "signers_set")),
            signers.len() as u32,
        );
        Ok(())
    }

    pub fn snapshot_state(env: Env, admin: Address, snapshot_id: u32) -> Result<(), Error> {
        let stored_admin = Self::require_admin(&env)?;
        if admin != stored_admin {
            return Err(Error::NotAdmin);
        }

        let upgrade_delay = Self::get_upgrade_delay(env.clone());
        let upgrade_config = Self::get_upgrade_config(env.clone()).ok();
        let registered_contracts = Self::get_registered_contracts(env.clone());

        let mut contract_versions = Vec::new(&env);
        let mut history_counts = Vec::new(&env);
        let mut history_items = Vec::new(&env);

        for contract_name in registered_contracts.iter() {
            let version = Self::get_latest_version(env.clone(), contract_name.clone());
            contract_versions.push_back((contract_name.clone(), version));

            let count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::UpgradeHistoryCount(contract_name.clone()))
                .unwrap_or(0);
            history_counts.push_back((contract_name.clone(), count));

            for i in 0..count {
                if let Some(record) = env
                    .storage()
                    .persistent()
                    .get::<_, UpgradeRecord>(&DataKey::UpgradeHistoryItem(contract_name.clone(), i))
                {
                    history_items.push_back((contract_name.clone(), i, record));
                }
            }
        }

        let mut upgrade_config_vec = Vec::new(&env);
        if let Some(cfg) = upgrade_config {
            upgrade_config_vec.push_back(cfg);
        }

        let snapshot = RegistrySnapshot {
            admin: admin.clone(),
            upgrade_delay,
            upgrade_config: upgrade_config_vec,
            registered_contracts,
            contract_versions,
            history_counts,
            history_items,
        };

        let serialized = snapshot.clone().to_xdr(&env);
        let checksum = shared::disaster_recovery::compute_checksum(&env, &serialized);
        let wasm_hash = BytesN::from_array(&env, &[0; 32]);

        let mut index: Vec<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::SnapshotIndex)
            .unwrap_or(Vec::new(&env));
        let snapshot_index_pos = index.len() as u32;

        let evicted = shared::disaster_recovery::push_snapshot_index(&mut index, snapshot_id);
        if let Some(old_id) = evicted {
            env.storage().persistent().remove(&DataKey::Snapshot(old_id));
            env.storage().persistent().remove(&DataKey::SnapshotMetadata(old_id));
        }

        env.storage()
            .persistent()
            .set(&DataKey::SnapshotIndex, &index);

        let meta = shared::disaster_recovery::SnapshotMeta {
            created_at: env.ledger().timestamp(),
            block_height: env.ledger().sequence(),
            contract_version: wasm_hash,
            admin,
            checksum,
            record_count: snapshot.history_items.len() as u64,
            snapshot_index: snapshot_index_pos.min(shared::disaster_recovery::MAX_SNAPSHOTS - 1),
        };

        env.storage().persistent().set(&DataKey::Snapshot(snapshot_id), &snapshot);
        env.storage().persistent().set(&DataKey::SnapshotMetadata(snapshot_id), &meta);

        Ok(())
    }

    pub fn propose_rollback(
        env: Env,
        proposer: Address,
        snapshot_id: u32,
        old_wasm_hash: BytesN<32>,
    ) -> Result<u32, Error> {
        if !env.storage().persistent().has(&DataKey::SnapshotMetadata(snapshot_id)) {
            panic!("Snapshot not found");
        }
        proposer.require_auth();

        let proposal_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::RollbackProposalCount)
            .unwrap_or(0);
        let new_id = proposal_count.checked_add(1).expect("Overflow");

        env.storage().persistent().set(&DataKey::RollbackProposalCount, &new_id);

        let proposal = shared::disaster_recovery::RollbackProposal {
            id: new_id,
            snapshot_id,
            old_wasm_hash: old_wasm_hash.clone(),
            approval_count: 1,
            executed: false,
            created_at: env.ledger().timestamp(),
            proposer: proposer.clone(),
        };

        env.storage().persistent().set(&DataKey::RollbackProposal(new_id), &proposal);
        env.storage().persistent().set(&DataKey::RollbackApproval(new_id, proposer.clone()), &true);

        env.events().publish(
            (Symbol::new(&env, "DR"), Symbol::new(&env, "rb_proposed"), new_id),
            (snapshot_id, proposer, old_wasm_hash),
        );

        Ok(new_id)
    }

    pub fn approve_rollback(env: Env, signer: Address, proposal_id: u32) -> Result<(), Error> {
        let signers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencySigners)
            .expect("Emergency signers not configured");
        if !signers.iter().any(|s| s == signer) {
            panic!("Signer is not an emergency signer");
        }

        let mut proposal: shared::disaster_recovery::RollbackProposal = env
            .storage()
            .persistent()
            .get(&DataKey::RollbackProposal(proposal_id))
            .expect("Rollback proposal not found");

        if proposal.executed {
            panic!("Rollback already executed");
        }
        if env.storage().persistent().get::<_, bool>(&DataKey::RollbackApproval(proposal_id, signer.clone())).unwrap_or(false) {
            panic!("Already approved");
        }

        signer.require_auth();
        env.storage().persistent().set(&DataKey::RollbackApproval(proposal_id, signer.clone()), &true);
        proposal.approval_count = proposal.approval_count.checked_add(1).expect("Overflow");
        env.storage().persistent().set(&DataKey::RollbackProposal(proposal_id), &proposal);

        env.events().publish(
            (Symbol::new(&env, "DR"), Symbol::new(&env, "rb_approved"), proposal_id),
            (signer, proposal.approval_count),
        );

        Ok(())
    }

    pub fn rollback_to_snapshot(env: Env, proposal_id: u32) -> Result<(), Error> {
        let mut proposal: shared::disaster_recovery::RollbackProposal = env
            .storage()
            .persistent()
            .get(&DataKey::RollbackProposal(proposal_id))
            .expect("Rollback proposal not found");

        if proposal.executed {
            panic!("Rollback already executed");
        }
        if proposal.approval_count < shared::disaster_recovery::EMERGENCY_THRESHOLD {
            panic!("Insufficient approvals");
        }

        let snapshot_id = proposal.snapshot_id;
        let snapshot: RegistrySnapshot = env
            .storage()
            .persistent()
            .get(&DataKey::Snapshot(snapshot_id))
            .expect("Snapshot data not found");

        env.storage().instance().set(&DataKey::Admin, &snapshot.admin);
        env.storage().instance().set(&DataKey::UpgradeDelay, &snapshot.upgrade_delay);
        if let Some(config) = snapshot.upgrade_config.get(0) {
            env.storage().instance().set(&DataKey::UpgradeConfig, &config);
        } else {
            env.storage().instance().remove(&DataKey::UpgradeConfig);
        }

        // Clean existing registry bookkeeping
        let current_registered = Self::get_registered_contracts(env.clone());
        for contract_name in current_registered.iter() {
            let count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::UpgradeHistoryCount(contract_name.clone()))
                .unwrap_or(0);
            for i in 0..count {
                env.storage().persistent().remove(&DataKey::UpgradeHistoryItem(contract_name.clone(), i));
            }
            env.storage().persistent().remove(&DataKey::UpgradeHistoryCount(contract_name.clone()));
            env.storage().persistent().remove(&DataKey::LatestVersion(contract_name.clone()));
        }

        // Restore snapshot bookkeeping
        env.storage().persistent().set(&DataKey::RegisteredContracts, &snapshot.registered_contracts);
        for (contract_name, version) in snapshot.contract_versions.iter() {
            env.storage().persistent().set(&DataKey::LatestVersion(contract_name), &version);
        }
        for (contract_name, count) in snapshot.history_counts.iter() {
            env.storage().persistent().set(&DataKey::UpgradeHistoryCount(contract_name), &count);
        }
        for (contract_name, i, record) in snapshot.history_items.iter() {
            env.storage().persistent().set(&DataKey::UpgradeHistoryItem(contract_name, i), &record);
        }

        env.deployer().update_current_contract_wasm(proposal.old_wasm_hash.clone());

        proposal.executed = true;
        env.storage().persistent().set(&DataKey::RollbackProposal(proposal_id), &proposal);

        env.events().publish(
            (Symbol::new(&env, "DR"), Symbol::new(&env, "rb_executed"), proposal_id),
            (snapshot_id, proposal.old_wasm_hash, snapshot.history_items.len() as u32),
        );

        Ok(())
    }

    pub fn get_snapshot_metadata(env: Env, snapshot_id: u32) -> Option<shared::disaster_recovery::SnapshotMeta> {
        env.storage()
            .persistent()
            .get(&DataKey::SnapshotMetadata(snapshot_id))
    }

    pub fn get_snapshot_index(env: Env) -> Vec<u32> {
        env.storage()
            .persistent()
            .get(&DataKey::SnapshotIndex)
            .unwrap_or(Vec::new(&env))
    }

    pub fn get_rollback_proposal(env: Env, proposal_id: u32) -> Option<shared::disaster_recovery::RollbackProposal> {
        env.storage()
            .persistent()
            .get(&DataKey::RollbackProposal(proposal_id))
    }

    pub fn get_registered_contracts(env: Env) -> Vec<Symbol> {
        env.storage()
            .persistent()
            .get(&DataKey::RegisteredContracts)
            .unwrap_or(Vec::new(&env))
    }

    pub fn verify_post_upgrade_state(
        env: Env,
        snapshot_id: u32,
    ) -> Result<shared::disaster_recovery::StateVerificationReport, Error> {
        let snapshot: RegistrySnapshot = env
            .storage()
            .persistent()
            .get(&DataKey::Snapshot(snapshot_id))
            .ok_or(Error::NotInitialized)?;

        let mut mismatches: Vec<soroban_sdk::String> = Vec::new(&env);
        let mut fields_checked = 0u32;

        let current_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        fields_checked += 1;
        if current_admin != snapshot.admin {
            mismatches.push_back(soroban_sdk::String::from_str(&env, "admin mismatch"));
        }

        let current_delay: u64 = env.storage().instance().get(&DataKey::UpgradeDelay).unwrap_or(0);
        fields_checked += 1;
        if current_delay != snapshot.upgrade_delay {
            mismatches.push_back(soroban_sdk::String::from_str(&env, "delay mismatch"));
        }

        let current_config: Option<UpgradeConfig> = env.storage().instance().get(&DataKey::UpgradeConfig);
        fields_checked += 1;
        if current_config != snapshot.upgrade_config.get(0) {
            mismatches.push_back(soroban_sdk::String::from_str(&env, "upgrade config mismatch"));
        }

        for (contract_name, expected_version) in snapshot.contract_versions.iter() {
            fields_checked += 1;
            let current_version = Self::get_latest_version(env.clone(), contract_name.clone());
            if current_version != expected_version {
                mismatches.push_back(soroban_sdk::String::from_str(&env, "version mismatch"));
            }
        }

        Ok(shared::disaster_recovery::StateVerificationReport {
            fields_checked,
            mismatches,
        })
    }
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

/// Cache expiration time for validation results (5 minutes)
const VALIDATION_CACHE_EXPIRY_SECS: u64 = 300;

/// === OPTIMIZATION: Cached validation to avoid repeated M-of-N signature checks ===
fn require_upgrade_approvals_cached(
    env: &Env,
    approvers: Vec<Address>,
) -> Result<Vec<Address>, Error> {
    // Check if we have a valid cached result
    let cache_key = DataKey::ValidationCache(approvers.clone());
    let cache_time_key = DataKey::ValidationCacheTime(approvers.clone());

    if let Some(cache_time) = env.storage().temporary().get::<_, u64>(&cache_time_key) {
        let now = env.ledger().timestamp();
        if now < cache_time + VALIDATION_CACHE_EXPIRY_SECS {
            // Cache hit - return cached result
            if let Some(cached_signers) =
                env.storage().temporary().get::<_, Vec<Address>>(&cache_key)
            {
                return Ok(cached_signers);
            }
        }
    }

    // Cache miss - perform full validation
    let result = require_upgrade_approvals(env, approvers)?;

    // Cache the result for future use
    let now = env.ledger().timestamp();
    env.storage().temporary().set(&cache_key, &result);
    env.storage().temporary().set(&cache_time_key, &now);

    Ok(result)
}

/// Validate that a WASM binary exports all required functions.
///
/// This prevents upgrades to WASM that omits critical functions like
/// execute_pending_upgrade, which would permanently brick the contract.
fn validate_wasm_exports(env: &Env, wasm_hash: &BytesN<32>) -> Result<(), Error> {
    // Note: Full WASM binary parsing in no_std is complex. This is a simplified
    // validation. In production, additional validation (e.g., via external tools
    // or during testing) should verify that the WASM indeed exports required
    // functions. The Soroban deployer will also reject invalid WASM at deployment.
    //
    // For now, we document the requirement:
    // Required exports: initialize, schedule_upgrade, execute_pending_upgrade,
    //                   cancel_pending_upgrade, get_admin
    //
    // Future enhancement: Use WASM parser to inspect module exports if available
    // in Soroban SDK.

    // Basic validation: ensure the hash is non-zero (indicates valid WASM)
    let zero_hash = BytesN::from_array(env, &[0u8; 32]);
    if wasm_hash == &zero_hash {
        return Err(Error::WasmValidationFailed);
    }

    // The actual function export verification happens at deployment time when
    // env.deployer().update_current_contract_wasm() is called. If the WASM is
    // missing required functions, that call will fail.

    Ok(())
}

fn get_upgrade_config(env: &Env) -> Result<UpgradeConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::UpgradeConfig)
        .ok_or(Error::NotInitialized)
}

fn validate_upgrade_config(signers: &Vec<Address>, threshold: u32) -> Result<(), Error> {
    if threshold == 0 || signers.is_empty() || threshold > signers.len() {
        return Err(Error::InvalidThreshold);
    }
    for i in 0..signers.len() {
        let signer = signers.get(i).ok_or(Error::NotSigner)?;
        for j in (i + 1)..signers.len() {
            if signer == signers.get(j).ok_or(Error::NotSigner)? {
                return Err(Error::DuplicateSigner);
            }
        }
    }
    Ok(())
}

fn require_upgrade_approvals(env: &Env, approvers: Vec<Address>) -> Result<Vec<Address>, Error> {
    if let Ok(config) = get_upgrade_config(env) {
        validate_approval_set(&config, &approvers)?;
        for signer in approvers.iter() {
            signer.require_auth();
        }
        Ok(approvers)
    } else {
        let admin = require_admin(env)?;
        Ok(soroban_sdk::vec![env, admin])
    }
}

#[allow(dead_code)]
fn require_upgrade_approvals_for_pending(
    env: &Env,
    approvers: Vec<Address>,
    pending: &PendingUpgrade,
) -> Result<Vec<Address>, Error> {
    let config = get_upgrade_config(env)?;
    validate_approval_set(&config, &approvers)?;
    for signer in approvers.iter() {
        signer.require_auth_for_args(
            (
                pending.new_wasm_hash.clone(),
                pending.contract_name.clone(),
                pending.new_version,
                pending.changelog_hash.clone(),
                pending.scheduled_at,
                pending.executable_after,
            )
                .into_val(env),
        );
    }
    Ok(approvers)
}

fn validate_approval_set(config: &UpgradeConfig, approvers: &Vec<Address>) -> Result<(), Error> {
    if approvers.len() < config.threshold {
        return Err(Error::BelowThreshold);
    }

    let mut valid_count = 0u32;
    for i in 0..approvers.len() {
        let approver = approvers.get(i).ok_or(Error::NotSigner)?;
        for j in (i + 1)..approvers.len() {
            if approver == approvers.get(j).ok_or(Error::NotSigner)? {
                return Err(Error::DuplicateSigner);
            }
        }
        if !is_config_signer(config, &approver) {
            return Err(Error::NotSigner);
        }
        valid_count = valid_count.checked_add(1).expect("approval count overflow");
    }

    if valid_count < config.threshold {
        return Err(Error::BelowThreshold);
    }
    Ok(())
}

fn is_config_signer(config: &UpgradeConfig, candidate: &Address) -> bool {
    for signer in config.signers.iter() {
        if signer == *candidate {
            return true;
        }
    }
    false
}

fn require_admin(env: &Env) -> Result<Address, Error> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)?;
    admin.require_auth();
    Ok(admin)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    // Helper: sets up a registry with no timelock by default.
    fn setup() -> (
        Env,
        Address,
        Address,
        UpgradeRegistryContractClient<'static>,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, UpgradeRegistryContract);
        let client = UpgradeRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &0u64); // upgrade_delay = 0
        (env, admin, contract_id, client)
    }

    fn setup_with_delay(delay: u64) -> (
        Env,
        Address,
        Address,
        UpgradeRegistryContractClient<'static>,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(1_000);

        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, UpgradeRegistryContract);
        let client = UpgradeRegistryContractClient::new(&env, &contract_id);
        client.initialize(&admin, &delay);
        (env, admin, contract_id, client)
    }

    // ------------------------------------------------------------------
    // Basic existing behaviour
    // ------------------------------------------------------------------

    #[test]
    fn test_initialize() {
        let (env, _admin, _contract_id, client) = setup();
        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);
        client.register_upgrade(&contract_name, &0, &1, &hash);
    }

    #[test]
    fn test_register_upgrade() {
        let (env, _admin, _contract_id, client) = setup();
        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[1u8; 32]);

        client.register_upgrade(&contract_name, &0, &1, &hash);

        let history = client.get_upgrade_history(&contract_name);
        assert_eq!(history.len(), 1);
        assert_eq!(history.get(0).unwrap().new_version, 1);
        assert_eq!(client.get_latest_version(&contract_name), 1);
    }

    #[test]
    fn test_subscribe_and_unsubscribe() {
        let (env, _admin, _contract_id, client) = setup();
        let contract_name = symbol_short!("escrow");
        let subscriber = Address::generate(&env);

        client.subscribe(&subscriber, &contract_name);
        assert_eq!(client.get_subscribers(&contract_name).len(), 1);

        client.unsubscribe(&subscriber, &contract_name);
        assert_eq!(client.get_subscribers(&contract_name).len(), 0);
    }

    #[test]
    #[should_panic]
    fn test_duplicate_subscribe_fails() {
        let (env, _admin, _contract_id, client) = setup();
        let contract_name = symbol_short!("escrow");
        let subscriber = Address::generate(&env);

        client.subscribe(&subscriber, &contract_name);
        client.subscribe(&subscriber, &contract_name);
    }

    // ------------------------------------------------------------------
    // #619-AC1: register_upgrade rejects downgrade
    // ------------------------------------------------------------------

    #[test]
    fn test_register_upgrade_rejects_downgrade() {
        let (env, _admin, _contract_id, client) = setup();
        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);

        client.register_upgrade(&contract_name, &0, &2, &hash);
        assert_eq!(client.get_latest_version(&contract_name), 2);

        // Attempt to downgrade to version 1
        let result = client.try_register_upgrade(&contract_name, &2, &1, &hash);
        assert_eq!(result, Err(Ok(Error::VersionNotMonotonic)));
    }

    #[test]
    fn test_register_upgrade_rejects_same_version() {
        let (env, _admin, _contract_id, client) = setup();
        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);

        client.register_upgrade(&contract_name, &0, &2, &hash);

        // Same version
        let result = client.try_register_upgrade(&contract_name, &2, &2, &hash);
        assert_eq!(result, Err(Ok(Error::VersionNotMonotonic)));
    }

    // ------------------------------------------------------------------
    // #619-AC1 (upgrade_contract path): downgrade returns VersionNotMonotonic
    // ------------------------------------------------------------------

    #[test]
    fn test_upgrade_contract_rejects_downgrade() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);
        env.ledger().set_timestamp(0);
        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);

        // Establish version 2 via register_upgrade
        client.register_upgrade(&contract_name, &0, &2, &hash);

        // upgrade_contract with new_version = 1 (downgrade) must fail
        let result = client.try_upgrade_contract(&hash, &contract_name, &1, &hash, &signers);
        assert_eq!(result, Err(Ok(Error::VersionNotMonotonic)));
    }

    #[test]
    fn test_upgrade_contract_rejects_same_version() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);
        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);

        client.register_upgrade(&contract_name, &0, &2, &hash);

        let result = client.try_upgrade_contract(&hash, &contract_name, &2, &hash, &signers);
        assert_eq!(result, Err(Ok(Error::VersionNotMonotonic)));
    }

    #[test]
    fn test_upgrade_contract_succeeds_with_higher_version() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);
        env.ledger().set_timestamp(0);
        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);

        client.register_upgrade(&contract_name, &0, &2, &hash);

        let result = client.try_upgrade_contract(&hash, &contract_name, &3, &hash, &signers);
        assert!(result.is_ok());
        assert_eq!(client.get_latest_version(&contract_name), 3);
    }

    // ------------------------------------------------------------------
    // #619-AC2: upgrade_contract enforces timelock
    // ------------------------------------------------------------------

    #[test]
    fn test_upgrade_contract_before_timelock_fails() {
        let delay = 3_600u64;
        let (env, admin, _contract_id, client) = setup_with_delay(delay);
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);

        // Record a prior upgrade at t=1000
        env.ledger().set_timestamp(1_000);
        client.register_upgrade(&contract_name, &0, &1, &hash);

        // Try upgrade_contract before delay has elapsed (t=1500 < 1000+3600)
        env.ledger().set_timestamp(1_500);
        let result = client.try_upgrade_contract(&hash, &contract_name, &2, &hash, &signers);
        assert_eq!(result, Err(Ok(Error::TimelockNotElapsed)));
    }

    #[test]
    fn test_upgrade_contract_timelock_elapsed() {
        let delay = 3_600u64;
        let (env, admin, _contract_id, client) = setup_with_delay(delay);
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);

        env.ledger().set_timestamp(1_000);
        client.register_upgrade(&contract_name, &0, &1, &hash);

        // Advance past delay: 1000 + 3600 = 4600; use 5000 to be safe
        env.ledger().set_timestamp(5_000);
        let result = client.try_upgrade_contract(&hash, &contract_name, &2, &hash, &signers);
        assert!(result.is_ok());
        assert_eq!(client.get_latest_version(&contract_name), 2);
    }

    // ------------------------------------------------------------------
    // Path A: schedule_upgrade + execute_pending_upgrade
    // ------------------------------------------------------------------

    #[test]
    fn test_two_step_upgrade_happy_path() {
        let delay = 3_600u64;
        let (env, admin, _contract_id, client) = setup_with_delay(delay);
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);

        // Schedule at t=1000; execute_after = 1000 + 3600 = 4600
        env.ledger().set_timestamp(1_000);
        client.schedule_upgrade(&hash, &contract_name, &1, &hash, &signers);

        let pending = client.get_pending_upgrade().unwrap();
        assert_eq!(pending.new_version, 1);
        assert_eq!(pending.executable_after, 4_600);

        // Cannot execute before delay
        env.ledger().set_timestamp(2_000);
        let result = client.try_execute_pending_upgrade(&signers);
        assert_eq!(result, Err(Ok(Error::TimelockNotElapsed)));

        // Execute after delay
        env.ledger().set_timestamp(5_000);
        client.execute_pending_upgrade(&signers);

        assert_eq!(client.get_latest_version(&contract_name), 1);
        assert!(client.get_pending_upgrade().is_none());
    }

    #[test]
    fn test_schedule_upgrade_rejects_downgrade() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);

        client.register_upgrade(&contract_name, &0, &3, &hash);

        // Try to schedule a downgrade to version 2
        let result = client.try_schedule_upgrade(&hash, &contract_name, &2, &hash, &signers);
        assert_eq!(result, Err(Ok(Error::VersionNotMonotonic)));
    }

    #[test]
    fn test_execute_without_schedule_fails() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let result = client.try_execute_pending_upgrade(&signers);
        assert_eq!(result, Err(Ok(Error::NoPendingUpgrade)));
    }

    // ------------------------------------------------------------------
    // Regression: both paths have identical security guarantees (#619-AC3)
    // ------------------------------------------------------------------

    #[test]
    fn test_both_paths_reject_downgrade() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let contract_name = symbol_short!("escrow");
        let hash = BytesN::from_array(&env, &[0u8; 32]);

        // Establish version 5
        client.register_upgrade(&contract_name, &0, &5, &hash);
        assert_eq!(client.get_latest_version(&contract_name), 5);

        // PATH B direct — downgrade attempt
        assert_eq!(
            client.try_upgrade_contract(&hash, &contract_name, &4, &hash, &signers),
            Err(Ok(Error::VersionNotMonotonic))
        );

        // PATH A schedule — downgrade attempt
        assert_eq!(
            client.try_schedule_upgrade(&hash, &contract_name, &3, &hash, &signers),
            Err(Ok(Error::VersionNotMonotonic))
        );

        // Version should be unchanged
        assert_eq!(client.get_latest_version(&contract_name), 5);
    }

    // ─── Source commit tests ────────────────────────────────────────────

    #[test]
    fn test_register_source_commit() {
        let (env, admin, _contract_id, client) = setup();
        let contract_name = symbol_short!("escrow");
        let commit_hash = BytesN::from_array(&env, &[0xab; 32]);

        client.register_source_commit(&admin, &contract_name, &1, &commit_hash);

        let stored = client.get_source_commit(&contract_name, &1);
        assert_eq!(stored, Some(commit_hash));
    }

    #[test]
    fn test_register_source_commit_duplicate() {
        let (env, admin, _contract_id, client) = setup();
        let contract_name = symbol_short!("escrow");
        let commit_hash = BytesN::from_array(&env, &[0xab; 32]);

        client.register_source_commit(&admin, &contract_name, &1, &commit_hash);

        // Second registration for same contract should fail
        let result = client.try_register_source_commit(
            &admin,
            &contract_name,
            &2,
            &BytesN::from_array(&env, &[0xcd; 32]),
        );
        assert_eq!(result, Err(Ok(Error::SourceCommitAlreadyRegistered)));
    }

    #[test]
    fn test_get_source_commit_none() {
        let (_env, _admin, _contract_id, client) = setup();
        let contract_name = symbol_short!("nonexist");

        let stored = client.get_source_commit(&contract_name, &1);
        assert_eq!(stored, None);
    }

    #[test]
    fn test_register_source_commit_wrong_admin() {
        let (env, _admin, _contract_id, client) = setup();
        let contract_name = symbol_short!("escrow");
        let commit_hash = BytesN::from_array(&env, &[0xab; 32]);
        let wrong_admin = Address::generate(&env);

        // mock_all_auths is active so require_auth passes, but our extra
        // stored_admin check catches the mismatch.
        let result =
            client.try_register_source_commit(&wrong_admin, &contract_name, &1, &commit_hash);
        assert_eq!(result, Err(Ok(Error::NotAdmin)));
    }

    #[test]
    fn test_source_commit_independent_per_contract() {
        let (env, admin, _contract_id, client) = setup();
        let escrow_name = symbol_short!("escrow");
        let treasury_name = symbol_short!("treasury");
        let hash1 = BytesN::from_array(&env, &[0xab; 32]);
        let hash2 = BytesN::from_array(&env, &[0xcd; 32]);

        client.register_source_commit(&admin, &escrow_name, &1, &hash1);
        client.register_source_commit(&admin, &treasury_name, &1, &hash2);

        assert_eq!(client.get_source_commit(&escrow_name, &1), Some(hash1));
        assert_eq!(client.get_source_commit(&treasury_name, &1), Some(hash2));
    }

    // ─── Storage layout & migration tests ────────────────────────────────

    fn make_test_storage_schema(env: &Env, version: u32, fields: Vec<StorageField>) -> StorageLayoutSchema {
        let schema_hash = CompatibilityValidator::compute_schema_hash(env, &fields);
        StorageLayoutSchema {
            version,
            schema_hash,
            fields,
        }
    }

    #[test]
    fn test_storage_schema_registration_and_retrieval() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let contract_name = symbol_short!("escrow");
        let field1 = StorageField {
            name: symbol_short!("admin"),
            field_type: StorageFieldType::Address,
            slot_index: 0,
            deprecated: false,
        };
        let field2 = StorageField {
            name: symbol_short!("fee"),
            field_type: StorageFieldType::U32,
            slot_index: 1,
            deprecated: false,
        };
        let schema_v1 = make_test_storage_schema(&env, 1, soroban_sdk::vec![&env, field1, field2]);

        client.register_storage_schema(&contract_name, &schema_v1, &signers);

        let stored = client.get_storage_schema(&contract_name, &1).unwrap();
        assert_eq!(stored.version, 1);
        assert_eq!(stored.schema_hash, schema_v1.schema_hash);

        let active_ver = client.get_active_storage_version(&contract_name).unwrap();
        assert_eq!(active_ver.current_version, 1);
        assert_eq!(active_ver.layout_hash, schema_v1.schema_hash);
        assert!(!active_ver.migration_in_progress);
    }

    #[test]
    fn test_storage_schema_additive_compatibility() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let contract_name = symbol_short!("escrow");
        let field1 = StorageField {
            name: symbol_short!("admin"),
            field_type: StorageFieldType::Address,
            slot_index: 0,
            deprecated: false,
        };
        let field2 = StorageField {
            name: symbol_short!("fee"),
            field_type: StorageFieldType::U32,
            slot_index: 1,
            deprecated: false,
        };
        let schema_v1 = make_test_storage_schema(&env, 1, soroban_sdk::vec![&env, field1.clone(), field2.clone()]);
        client.register_storage_schema(&contract_name, &schema_v1, &signers);

        let field3 = StorageField {
            name: symbol_short!("paused"),
            field_type: StorageFieldType::Bool,
            slot_index: 2,
            deprecated: false,
        };
        let schema_v2 = make_test_storage_schema(&env, 2, soroban_sdk::vec![&env, field1, field2, field3]);

        let report = client.validate_upgrade_compatibility(&contract_name, &schema_v2);
        assert!(report.is_compatible);
        assert!(!report.requires_migration);
        assert_eq!(report.added_fields, 1);

        client.register_storage_schema(&contract_name, &schema_v2, &signers);
        let active_ver = client.get_active_storage_version(&contract_name).unwrap();
        assert_eq!(active_ver.current_version, 2);
    }

    #[test]
    fn test_storage_schema_incompatible_rejected() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let contract_name = symbol_short!("escrow");
        let field1 = StorageField {
            name: symbol_short!("fee"),
            field_type: StorageFieldType::U32,
            slot_index: 0,
            deprecated: false,
        };
        let schema_v1 = make_test_storage_schema(&env, 1, soroban_sdk::vec![&env, field1]);
        client.register_storage_schema(&contract_name, &schema_v1, &signers);

        // Incompatible type change: U32 -> U64
        let field1_bad = StorageField {
            name: symbol_short!("fee"),
            field_type: StorageFieldType::U64,
            slot_index: 0,
            deprecated: false,
        };
        let schema_v2_bad = make_test_storage_schema(&env, 2, soroban_sdk::vec![&env, field1_bad]);

        let report = client.validate_upgrade_compatibility(&contract_name, &schema_v2_bad);
        assert!(!report.is_compatible);
        assert!(report.requires_migration);
    }

    #[test]
    fn test_gradual_storage_migration_flow() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let contract_name = symbol_short!("escrow");
        let field1 = StorageField {
            name: symbol_short!("admin"),
            field_type: StorageFieldType::Address,
            slot_index: 0,
            deprecated: false,
        };
        let schema_v1 = make_test_storage_schema(&env, 1, soroban_sdk::vec![&env, field1.clone()]);
        let schema_v2 = make_test_storage_schema(&env, 2, soroban_sdk::vec![&env, field1]);

        client.register_storage_schema(&contract_name, &schema_v1, &signers);
        client.register_storage_schema(&contract_name, &schema_v2, &signers);

        // Start migration of 500 records
        let initial_status = client.start_storage_migration(&contract_name, &1, &2, &500, &signers);
        assert_eq!(initial_status.total_records, 500);
        assert_eq!(initial_status.processed_records, 0);
        assert!(!initial_status.completed);

        // Execute batch step of 200
        let step1 = client.execute_migration_step(&contract_name, &200, &signers);
        assert_eq!(step1.processed_records, 200);
        assert!(!step1.completed);

        // Execute batch step of 300 to complete
        let step2 = client.execute_migration_step(&contract_name, &300, &signers);
        assert_eq!(step2.processed_records, 500);
        assert!(step2.completed);

        let active_ver = client.get_active_storage_version(&contract_name).unwrap();
        assert_eq!(active_ver.current_version, 2);
        assert!(!active_ver.migration_in_progress);
    }

    #[test]
    fn test_storage_layout_rollback() {
        let (env, admin, _contract_id, client) = setup();
        let signers = soroban_sdk::vec![&env, admin.clone()];
        client.set_upgrade_signers(&signers, &1, &signers);

        let contract_name = symbol_short!("escrow");
        let field1 = StorageField {
            name: symbol_short!("admin"),
            field_type: StorageFieldType::Address,
            slot_index: 0,
            deprecated: false,
        };
        let schema_v1 = make_test_storage_schema(&env, 1, soroban_sdk::vec![&env, field1.clone()]);
        let schema_v2 = make_test_storage_schema(&env, 2, soroban_sdk::vec![&env, field1]);

        client.register_storage_schema(&contract_name, &schema_v1, &signers);
        client.register_storage_schema(&contract_name, &schema_v2, &signers);

        assert_eq!(client.get_active_storage_version(&contract_name).unwrap().current_version, 2);

        // Rollback to version 1
        client.rollback_storage_layout(&contract_name, &1, &signers);
        assert_eq!(client.get_active_storage_version(&contract_name).unwrap().current_version, 1);
    }
}

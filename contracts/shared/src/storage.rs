/// Eternal Storage pattern for MentorsMind Soroban contracts.
///
/// Separates storage layout from contract logic so that contract upgrades
/// can add new fields without breaking existing data. All storage access
/// goes through typed key enums, making the layout explicit and auditable.
///
/// # Pattern
/// - Each contract defines its own `StorageKey` enum (or reuses these helpers).
/// - Logic contracts call `EternalStorage::get / set / remove`.
/// - On upgrade, new keys are simply added; old keys remain readable.
///
/// # Usage
/// ```rust
/// use shared::storage::{EternalStorage, StorageType};
///
/// // Write
/// EternalStorage::set_persistent(&env, &MyKey::Config, &config_value);
///
/// // Read with default
/// let fee: u32 = EternalStorage::get_persistent(&env, &MyKey::Fee).unwrap_or(500);
///
/// // Remove
/// EternalStorage::remove_persistent(&env, &MyKey::OldField);
/// ```

use soroban_sdk::{contracterror, contracttype, symbol_short, xdr::ToXdr, Bytes, BytesN, Env, IntoVal, Symbol, TryFromVal, Val};

// ---------------------------------------------------------------------------
// Storage type selector
// ---------------------------------------------------------------------------

/// Which Soroban storage tier to use.
#[derive(Copy, Clone, Eq, PartialEq)]
pub enum StorageType {
    /// Instance storage: cheap, lives as long as the contract instance.
    /// Use for config that is read on every invocation (admin, fee, flags).
    Instance,
    /// Persistent storage: survives ledger expiry extensions.
    /// Use for per-entity records (escrows, proposals, balances).
    Persistent,
    /// Temporary storage: cheapest, expires after a few ledgers.
    /// Use for nonces, rate-limit counters, short-lived locks.
    Temporary,
}

// ---------------------------------------------------------------------------
// EternalStorage helper
// ---------------------------------------------------------------------------

/// Stateless helper that wraps Soroban storage with a uniform API.
/// All methods are free functions (no state) — just pass `&env`.
pub struct EternalStorage;

impl EternalStorage {
    // -----------------------------------------------------------------------
    // Instance storage
    // -----------------------------------------------------------------------

    pub fn set_instance<K, V>(env: &Env, key: &K, value: &V)
    where
        K: IntoVal<Env, Val>,
        V: IntoVal<Env, Val>,
    {
        env.storage().instance().set(key, value);
    }

    pub fn get_instance<K, V>(env: &Env, key: &K) -> Option<V>
    where
        K: IntoVal<Env, Val>,
        V: TryFromVal<Env, Val>,
    {
        env.storage().instance().get(key)
    }

    pub fn has_instance<K>(env: &Env, key: &K) -> bool
    where
        K: IntoVal<Env, Val>,
    {
        env.storage().instance().has(key)
    }

    pub fn remove_instance<K>(env: &Env, key: &K)
    where
        K: IntoVal<Env, Val>,
    {
        env.storage().instance().remove(key);
    }

    // -----------------------------------------------------------------------
    // Persistent storage
    // -----------------------------------------------------------------------

    pub fn set_persistent<K, V>(env: &Env, key: &K, value: &V)
    where
        K: IntoVal<Env, Val>,
        V: IntoVal<Env, Val>,
    {
        env.storage().persistent().set(key, value);
    }

    pub fn get_persistent<K, V>(env: &Env, key: &K) -> Option<V>
    where
        K: IntoVal<Env, Val>,
        V: TryFromVal<Env, Val>,
    {
        env.storage().persistent().get(key)
    }

    pub fn has_persistent<K>(env: &Env, key: &K) -> bool
    where
        K: IntoVal<Env, Val>,
    {
        env.storage().persistent().has(key)
    }

    pub fn remove_persistent<K>(env: &Env, key: &K)
    where
        K: IntoVal<Env, Val>,
    {
        env.storage().persistent().remove(key);
    }

    // -----------------------------------------------------------------------
    // Temporary storage
    // -----------------------------------------------------------------------

    pub fn set_temporary<K, V>(env: &Env, key: &K, value: &V)
    where
        K: IntoVal<Env, Val>,
        V: IntoVal<Env, Val>,
    {
        env.storage().temporary().set(key, value);
    }

    pub fn get_temporary<K, V>(env: &Env, key: &K) -> Option<V>
    where
        K: IntoVal<Env, Val>,
        V: TryFromVal<Env, Val>,
    {
        env.storage().temporary().get(key)
    }

    pub fn has_temporary<K>(env: &Env, key: &K) -> bool
    where
        K: IntoVal<Env, Val>,
    {
        env.storage().temporary().has(key)
    }

    pub fn remove_temporary<K>(env: &Env, key: &K)
    where
        K: IntoVal<Env, Val>,
    {
        env.storage().temporary().remove(key);
    }

    // -----------------------------------------------------------------------
    // TTL Extension helpers
    // -----------------------------------------------------------------------

    /// Extend instance storage using the unified TTL policy.
    pub fn extend_instance_ttl(env: &Env) {
        crate::ttl_utils::TTLManager::extend_instance(env);
    }

    /// Extend a persistent storage entry using the unified TTL policy.
    pub fn extend_persistent_ttl<K>(env: &Env, key: &K)
    where
        K: IntoVal<Env, Val>,
    {
        crate::ttl_utils::TTLManager::extend_persistent(env, key);
    }

    /// Extend a temporary storage entry using the unified TTL policy.
    pub fn extend_temporary_ttl<K>(env: &Env, key: &K)
    where
        K: IntoVal<Env, Val>,
    {
        crate::ttl_utils::TTLManager::extend_temporary(env, key);
    }
}

// ---------------------------------------------------------------------------
// Canonical storage key definitions
// ---------------------------------------------------------------------------
// These are the shared keys used across contracts. Each contract may define
// additional contract-local keys in its own module.

/// Common instance-storage keys (config, flags).
#[contracttype]
#[derive(Clone)]
pub enum InstanceKey {
    /// Contract admin address.
    Admin,
    /// Platform fee in basis points (e.g. 500 = 5%).
    PlatformFee,
    /// Whether the contract is paused.
    Paused,
    /// Schema version — increment on breaking storage changes.
    SchemaVersion,
    /// Approval threshold (multisig).
    Threshold,
    /// Number of signers (multisig).
    SignerCount,
    /// Proposal counter (multisig).
    ProposalCount,
    /// Operation counter (timelock).
    OpCount,
}

/// Common persistent-storage keys (per-entity records).
#[contracttype]
#[derive(Clone)]
pub enum PersistentKey {
    /// Escrow record by id.
    Escrow(u64),
    /// Signer flag by address.
    Signer(soroban_sdk::Address),
    /// Multisig proposal by id.
    Proposal(u32),
    /// Multisig approval by (proposal_id, signer).
    Approval(u32, soroban_sdk::Address),
    /// Timelock operation by id.
    TimelockOp(soroban_sdk::BytesN<32>),
    /// Upgrade history by contract name.
    UpgradeHistory(soroban_sdk::Symbol),
    /// Latest version by contract name.
    LatestVersion(soroban_sdk::Symbol),
    /// Subscribers list by contract name.
    Subscribers(soroban_sdk::Symbol),
    /// Treasury allocation history.
    AllocHistory,
    /// Generic key-value for future extensibility.
    Custom(soroban_sdk::Symbol),
}

/// Temporary-storage keys (nonces, rate limits, locks).
#[contracttype]
#[derive(Clone)]
pub enum TempKey {
    /// Reentrancy lock by name.
    ReentrancyLock(soroban_sdk::Symbol),
    /// Rate-limit counter by (address, window).
    RateLimit(soroban_sdk::Address, u64),
    /// Short-lived nonce.
    Nonce(soroban_sdk::Address),
}

// ---------------------------------------------------------------------------
// Storage security (#826): namespace isolation, collision detection, integrity
// ---------------------------------------------------------------------------

/// Errors raised by secure storage helpers.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StorageSecurityError {
    /// Namespace has not been installed for this contract instance.
    NamespaceNotInstalled = 1,
    /// A write would collide with an existing key fingerprint.
    KeyCollision = 2,
    /// Caller contract does not match the namespace owner.
    NamespaceMismatch = 3,
    /// Stored value failed integrity verification on read.
    IntegrityFailure = 4,
    /// Cross-contract storage access was rejected.
    CrossContractAccessDenied = 5,
}

/// Contract-local namespace derived from contract address + scope symbol.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageNamespace {
    pub scope: Symbol,
    /// SHA-256(contract_address || scope).
    pub prefix: BytesN<32>,
}

/// Fingerprint recorded for each logical storage key.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageKeyFingerprint {
    pub namespace_prefix: BytesN<32>,
    pub key_hash: BytesN<32>,
    pub registered_at: u64,
}

/// Integrity checksum stored alongside logical values.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageIntegrityRecord {
    pub value_hash: BytesN<32>,
    pub updated_at: u64,
}

#[contracttype]
#[derive(Clone)]
enum CollisionRegistryKey {
    Fingerprint(BytesN<32>),
}

#[contracttype]
#[derive(Clone)]
enum SecureEntryKey {
    Data(BytesN<32>),
}

#[contracttype]
#[derive(Clone)]
enum IntegrityRegistryKey {
    Checksum(BytesN<32>),
}

/// Context symbol mixed into storage key derivation hashes.
pub const STORAGE_DERIVE_CTX: Symbol = symbol_short!("mm_store");

pub struct StorageKeyDerivation;

impl StorageKeyDerivation {
    /// Derive a contract-specific namespace prefix.
    pub fn namespace(env: &Env, scope: Symbol) -> StorageNamespace {
        let contract = env.current_contract_address();
        let mut payload = Bytes::new(env);
        payload.append(&contract.to_xdr(env));
        payload.append(&scope.clone().to_xdr(env));
        let prefix = env.crypto().sha256(&payload).into();
        StorageNamespace { scope, prefix }
    }

    /// Derive a unique key hash from namespace, context, and logical key.
    pub fn key_hash<K>(env: &Env, ns: &StorageNamespace, context: Symbol, key: &K) -> BytesN<32>
    where
        K: IntoVal<Env, Val>,
    {
        let mut payload = Bytes::new(env);
        payload.append(&ns.prefix.clone().to_xdr(env));
        payload.append(&context.clone().to_xdr(env));
        payload.append(&key.into_val(env).to_xdr(env));
        env.crypto().sha256(&payload).into()
    }
}

pub struct CollisionDetector;

impl CollisionDetector {
    fn registry_key(key_hash: &BytesN<32>) -> CollisionRegistryKey {
        CollisionRegistryKey::Fingerprint(key_hash.clone())
    }

    /// Reject writes that would collide with an existing fingerprint from another namespace.
    pub fn assert_no_collision<K>(
        env: &Env,
        ns: &StorageNamespace,
        context: Symbol,
        key: &K,
    ) -> Result<BytesN<32>, StorageSecurityError>
    where
        K: IntoVal<Env, Val>,
    {
        let key_hash = StorageKeyDerivation::key_hash(env, ns, context, key);
        let reg_key = Self::registry_key(&key_hash);
        if let Some(existing) = EternalStorage::get_instance::<_, StorageKeyFingerprint>(
            env,
            &reg_key,
        ) {
            if existing.namespace_prefix != ns.prefix {
                return Err(StorageSecurityError::KeyCollision);
            }
            return Ok(key_hash);
        }
        Ok(key_hash)
    }

    pub fn register<K>(
        env: &Env,
        ns: &StorageNamespace,
        context: Symbol,
        key: &K,
    ) -> Result<(), StorageSecurityError>
    where
        K: IntoVal<Env, Val>,
    {
        let key_hash = Self::assert_no_collision(env, ns, context, key)?;
        let fingerprint = StorageKeyFingerprint {
            namespace_prefix: ns.prefix.clone(),
            key_hash: key_hash.clone(),
            registered_at: env.ledger().timestamp(),
        };
        EternalStorage::set_instance(env, &Self::registry_key(&key_hash), &fingerprint);
        Ok(())
    }
}

pub struct StorageIntegrity;

impl StorageIntegrity {
    fn registry_key(key_hash: &BytesN<32>) -> IntegrityRegistryKey {
        IntegrityRegistryKey::Checksum(key_hash.clone())
    }

    pub fn hash_value<V>(env: &Env, value: &V) -> BytesN<32>
    where
        V: IntoVal<Env, Val>,
    {
        let payload = value.into_val(env).to_xdr(env);
        env.crypto().sha256(&payload).into()
    }

    pub fn record(env: &Env, key_hash: &BytesN<32>, value: &impl IntoVal<Env, Val>) {
        let record = StorageIntegrityRecord {
            value_hash: Self::hash_value(env, value),
            updated_at: env.ledger().timestamp(),
        };
        EternalStorage::set_instance(env, &Self::registry_key(key_hash), &record);
    }

    pub fn verify<V>(env: &Env, key_hash: &BytesN<32>, value: &V) -> Result<(), StorageSecurityError>
    where
        V: IntoVal<Env, Val>,
    {
        let expected = Self::hash_value(env, value);
        match EternalStorage::get_instance::<_, StorageIntegrityRecord>(
            env,
            &Self::registry_key(key_hash),
        ) {
            Some(record) if record.value_hash == expected => Ok(()),
            Some(_) => Err(StorageSecurityError::IntegrityFailure),
            None => Ok(()),
        }
    }
}

pub struct StorageAccessControl;

impl StorageAccessControl {
    /// Ensure the active namespace belongs to the executing contract.
    pub fn validate_namespace(env: &Env, ns: &StorageNamespace) -> Result<(), StorageSecurityError> {
        let expected = StorageKeyDerivation::namespace(env, ns.scope.clone());
        if expected.prefix != ns.prefix {
            return Err(StorageSecurityError::NamespaceMismatch);
        }
        Ok(())
    }

    /// Reject attempts to use a namespace derived for a different contract address.
    pub fn reject_foreign_namespace(
        env: &Env,
        foreign: &StorageNamespace,
    ) -> Result<(), StorageSecurityError> {
        let local = StorageKeyDerivation::namespace(env, foreign.scope.clone());
        if local.prefix != foreign.prefix {
            return Err(StorageSecurityError::CrossContractAccessDenied);
        }
        Ok(())
    }
}

/// High-level secure storage API with namespace prefixing and write-time checks.
pub struct SecureStorageAccess;

impl SecureStorageAccess {
    /// Install and persist a namespace under `namespace_root_key` (typically `DataKey::NamespaceRoot`).
    pub fn install_namespace<K>(env: &Env, namespace_root_key: &K, scope: Symbol) -> StorageNamespace
    where
        K: IntoVal<Env, Val>,
    {
        let ns = StorageKeyDerivation::namespace(env, scope);
        EternalStorage::set_instance(env, namespace_root_key, &ns);
        ns
    }

    /// Load a previously installed namespace.
    pub fn load_namespace<K>(env: &Env, namespace_root_key: &K) -> Result<StorageNamespace, StorageSecurityError>
    where
        K: IntoVal<Env, Val>,
    {
        EternalStorage::get_instance(env, namespace_root_key)
            .ok_or(StorageSecurityError::NamespaceNotInstalled)
    }

    /// Prefix a logical key with the namespace for isolated storage.
    fn entry_key<K>(env: &Env, ns: &StorageNamespace, inner_key: &K) -> SecureEntryKey
    where
        K: IntoVal<Env, Val>,
    {
        let key_hash = StorageKeyDerivation::key_hash(env, ns, STORAGE_DERIVE_CTX, inner_key);
        SecureEntryKey::Data(key_hash)
    }

    pub fn set_persistent_checked<K, V, R>(
        env: &Env,
        namespace_root_key: &R,
        context: Symbol,
        inner_key: &K,
        value: &V,
    ) -> Result<(), StorageSecurityError>
    where
        K: IntoVal<Env, Val>,
        V: IntoVal<Env, Val>,
        R: IntoVal<Env, Val>,
    {
        let _ = context;
        let ns = Self::load_namespace(env, namespace_root_key)?;
        StorageAccessControl::validate_namespace(env, &ns)?;
        let key_hash = CollisionDetector::assert_no_collision(env, &ns, STORAGE_DERIVE_CTX, inner_key)?;
        let entry = Self::entry_key(env, &ns, inner_key);
        EternalStorage::set_persistent(env, &entry, value);
        CollisionDetector::register(env, &ns, STORAGE_DERIVE_CTX, inner_key)?;
        StorageIntegrity::record(env, &key_hash, value);
        Ok(())
    }

    pub fn get_persistent_checked<K, V, R>(
        env: &Env,
        namespace_root_key: &R,
        inner_key: &K,
    ) -> Result<Option<V>, StorageSecurityError>
    where
        K: IntoVal<Env, Val>,
        V: TryFromVal<Env, Val> + IntoVal<Env, Val>,
        R: IntoVal<Env, Val>,
    {
        let ns = Self::load_namespace(env, namespace_root_key)?;
        StorageAccessControl::validate_namespace(env, &ns)?;
        let entry = Self::entry_key(env, &ns, inner_key);
        let value: Option<V> = EternalStorage::get_persistent(env, &entry);
        if let Some(ref v) = value {
            let key_hash =
                StorageKeyDerivation::key_hash(env, &ns, STORAGE_DERIVE_CTX, inner_key);
            StorageIntegrity::verify(env, &key_hash, v)?;
        }
        Ok(value)
    }

    pub fn has_persistent_checked<K, R>(
        env: &Env,
        namespace_root_key: &R,
        inner_key: &K,
    ) -> Result<bool, StorageSecurityError>
    where
        K: IntoVal<Env, Val>,
        R: IntoVal<Env, Val>,
    {
        let ns = Self::load_namespace(env, namespace_root_key)?;
        StorageAccessControl::validate_namespace(env, &ns)?;
        let entry = Self::entry_key(env, &ns, inner_key);
        Ok(EternalStorage::has_persistent(env, &entry))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;

    use super::*;
    use soroban_sdk::{contract, symbol_short, Address, Env};

    #[contract]
    struct TestStorageContract;

    fn setup_env() -> (Env, Address) {
        let env = Env::default();
        let addr = env.register_contract(None, TestStorageContract);
        (env, addr)
    }

    #[test]
    fn test_instance_set_get() {
        let (env, addr) = setup_env();
        env.as_contract(&addr, || {
            EternalStorage::set_instance(&env, &InstanceKey::PlatformFee, &500u32);
            let fee: Option<u32> = EternalStorage::get_instance(&env, &InstanceKey::PlatformFee);
            assert_eq!(fee, Some(500u32));
        });
    }

    #[test]
    fn test_instance_has_remove() {
        let (env, addr) = setup_env();
        env.as_contract(&addr, || {
            assert!(!EternalStorage::has_instance(&env, &InstanceKey::Paused));
            EternalStorage::set_instance(&env, &InstanceKey::Paused, &true);
            assert!(EternalStorage::has_instance(&env, &InstanceKey::Paused));
            EternalStorage::remove_instance(&env, &InstanceKey::Paused);
            assert!(!EternalStorage::has_instance(&env, &InstanceKey::Paused));
        });
    }

    #[test]
    fn test_persistent_set_get() {
        let (env, addr) = setup_env();
        env.as_contract(&addr, || {
            EternalStorage::set_persistent(&env, &PersistentKey::Escrow(42u64), &9999i128);
            let val: Option<i128> =
                EternalStorage::get_persistent(&env, &PersistentKey::Escrow(42u64));
            assert_eq!(val, Some(9999i128));
        });
    }

    #[test]
    fn test_persistent_remove() {
        let (env, addr) = setup_env();
        env.as_contract(&addr, || {
            EternalStorage::set_persistent(&env, &PersistentKey::AllocHistory, &1u32);
            assert!(EternalStorage::has_persistent(&env, &PersistentKey::AllocHistory));
            EternalStorage::remove_persistent(&env, &PersistentKey::AllocHistory);
            assert!(!EternalStorage::has_persistent(&env, &PersistentKey::AllocHistory));
        });
    }

    #[test]
    fn test_temporary_set_get() {
        let (env, addr) = setup_env();
        env.as_contract(&addr, || {
            let key = TempKey::ReentrancyLock(symbol_short!("escrow"));
            EternalStorage::set_temporary(&env, &key, &true);
            let val: Option<bool> = EternalStorage::get_temporary(&env, &key);
            assert_eq!(val, Some(true));
        });
    }

    #[test]
    fn test_schema_version_tracking() {
        let (env, addr) = setup_env();
        env.as_contract(&addr, || {
            let v: Option<u32> = EternalStorage::get_instance(&env, &InstanceKey::SchemaVersion);
            assert_eq!(v, None);
            EternalStorage::set_instance(&env, &InstanceKey::SchemaVersion, &1u32);
            let v: Option<u32> = EternalStorage::get_instance(&env, &InstanceKey::SchemaVersion);
            assert_eq!(v, Some(1u32));
            EternalStorage::set_instance(&env, &InstanceKey::SchemaVersion, &2u32);
            let v: Option<u32> = EternalStorage::get_instance(&env, &InstanceKey::SchemaVersion);
            assert_eq!(v, Some(2u32));
        });
    }

    #[contracttype]
    #[derive(Clone)]
    enum TestRootKey {
        NamespaceRoot,
        Value,
    }

    #[test]
    fn namespace_is_contract_specific() {
        let (env, addr) = setup_env();
        env.as_contract(&addr, || {
            let scope = symbol_short!("test_ns");
            let ns =
                SecureStorageAccess::install_namespace(&env, &TestRootKey::NamespaceRoot, scope);
            StorageAccessControl::validate_namespace(&env, &ns).unwrap();
            SecureStorageAccess::set_persistent_checked(
                &env,
                &TestRootKey::NamespaceRoot,
                STORAGE_DERIVE_CTX,
                &TestRootKey::Value,
                &42u32,
            )
            .unwrap();
            let loaded: u32 = SecureStorageAccess::get_persistent_checked(
                &env,
                &TestRootKey::NamespaceRoot,
                &TestRootKey::Value,
            )
            .unwrap()
            .unwrap();
            assert_eq!(loaded, 42);
        });
    }

    #[test]
    fn collision_detector_rejects_foreign_namespace() {
        let (env, addr) = setup_env();
        env.as_contract(&addr, || {
            let scope = symbol_short!("test_ns");
            let ns = StorageKeyDerivation::namespace(&env, scope.clone());
            let foreign = StorageNamespace {
                scope,
                prefix: BytesN::from_array(&env, &[0xFFu8; 32]),
            };
            assert_eq!(
                StorageAccessControl::reject_foreign_namespace(&env, &foreign),
                Err(StorageSecurityError::CrossContractAccessDenied)
            );
            CollisionDetector::register(&env, &ns, STORAGE_DERIVE_CTX, &TestRootKey::Value)
                .unwrap();
            assert!(
                CollisionDetector::register(&env, &ns, STORAGE_DERIVE_CTX, &TestRootKey::Value)
                    .is_ok()
            );
        });
    }
}

//! Unified Time-To-Live (TTL) management, dependency tracking, expiration monitoring,
//! and data recovery utilities for Soroban contracts.
//!
//! Enforces consistent TTL extension policies across all contract storage tiers (instance,
//! persistent, temporary) to prevent unexpected data expiration during active operations.
//!
//! # Ledger arithmetic
//!
//! Soroban expresses TTLs in *ledgers*, not seconds. Every constant in this module
//! assumes the Stellar mainnet target close time of ~5 seconds per ledger:
//!
//! ```text
//! 1 day  = 86_400 s / 5 s  =  17_280 ledgers   (ONE_DAY_LEDGERS)
//! 7 days = 7  * 17_280     = 120_960 ledgers   (SEVEN_DAYS_LEDGERS)
//! 30 days = 30 * 17_280    = 518_400 ledgers   (THIRTY_DAYS_LEDGERS)
//! ```
//!
//! If the network close time changes, the *ledger* counts stay the same but the
//! wall-clock durations they represent shift proportionally.
//!
//! # Which constants to use for which storage tier
//!
//! `extend_ttl(threshold, extend_to)` is a no-op while the entry's remaining TTL is
//! above `threshold`; once it drops to or below `threshold` the TTL is raised to
//! `extend_to`. Each tier therefore has a *threshold* / *bump* pair:
//!
//! | Storage tier | Threshold constant              | Bump constant            | Meaning                                |
//! |--------------|---------------------------------|--------------------------|----------------------------------------|
//! | Instance     | [`INSTANCE_LIFETIME_THRESHOLD`]   | [`INSTANCE_BUMP_AMOUNT`]   | < 7 days left → extend to 30 days      |
//! | Persistent   | [`PERSISTENT_LIFETIME_THRESHOLD`] | [`PERSISTENT_BUMP_AMOUNT`] | < 7 days left → extend to 30 days      |
//! | Temporary    | [`TEMPORARY_LIFETIME_THRESHOLD`]  | [`TEMPORARY_BUMP_AMOUNT`]  | < ~4.8 hours left → extend to 1 day    |
//!
//! Rules of thumb for new contracts:
//!
//! - **Instance** storage holds contract-wide config (admin, token addresses,
//!   counters). Bump it on every state-changing entry point.
//! - **Persistent** storage holds per-user / per-record data that must never be
//!   lost (escrows, stakes, balances). Bump the specific key on every read *and*
//!   write that the protocol depends on.
//! - **Temporary** storage holds cheap, disposable data (nonces, rate-limit
//!   windows, in-flight dependency markers). It is deleted permanently on expiry and
//!   cannot be restored, so never put funds-bearing state here.
//!
//! [`SAFETY_MARGIN_LEDGERS`] and [`WARNING_THRESHOLD_LEDGERS`] are *monitoring*
//! constants used by [`ExpirationMonitor`]; they are not passed to `extend_ttl`
//! directly (except as the temporary-tier threshold).
//!
//! # Usage
//!
//! Calling `extend_ttl` directly with the shared constants for all three tiers:
//!
//! ```ignore
//! use shared::ttl_utils::{
//!     INSTANCE_BUMP_AMOUNT, INSTANCE_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT,
//!     PERSISTENT_LIFETIME_THRESHOLD, TEMPORARY_BUMP_AMOUNT, TEMPORARY_LIFETIME_THRESHOLD,
//! };
//! use soroban_sdk::{contracttype, Env};
//!
//! #[contracttype]
//! pub enum DataKey {
//!     Escrow(u64),
//!     Nonce(u64),
//! }
//!
//! fn touch_storage(env: &Env, escrow_id: u64, nonce: u64) {
//!     // Instance tier: contract-wide config, bumped on every call.
//!     env.storage()
//!         .instance()
//!         .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
//!
//!     // Persistent tier: long-lived, funds-bearing records.
//!     env.storage().persistent().extend_ttl(
//!         &DataKey::Escrow(escrow_id),
//!         PERSISTENT_LIFETIME_THRESHOLD,
//!         PERSISTENT_BUMP_AMOUNT,
//!     );
//!
//!     // Temporary tier: short-lived, disposable data.
//!     env.storage().temporary().extend_ttl(
//!         &DataKey::Nonce(nonce),
//!         TEMPORARY_LIFETIME_THRESHOLD,
//!         TEMPORARY_BUMP_AMOUNT,
//!     );
//! }
//! ```
//!
//! The same policy is wrapped by [`TTLManager`], which is the preferred entry point
//! so that every contract stays on one policy:
//!
//! ```ignore
//! use shared::ttl_utils::TTLManager;
//!
//! TTLManager::extend_instance(&env);
//! TTLManager::extend_persistent(&env, &DataKey::Escrow(escrow_id));
//! TTLManager::extend_temporary(&env, &DataKey::Nonce(nonce));
//! ```

use soroban_sdk::{
    contracttype, symbol_short, xdr::ToXdr, Bytes, BytesN, Env, IntoVal, Symbol, Val,
};

// ---------------------------------------------------------------------------
// Unified TTL Constants & Safety Margins (Assuming ~5s Stellar ledger close time)
// ---------------------------------------------------------------------------

/// One day expressed in ledgers: `86_400 s / 5 s = 17_280` ledgers.
///
/// Base unit for every other constant in this module. Not tied to a single
/// storage tier; also used directly as [`TEMPORARY_BUMP_AMOUNT`] and
/// [`WARNING_THRESHOLD_LEDGERS`].
pub const ONE_DAY_LEDGERS: u32 = 17_280;

/// Seven days expressed in ledgers: `7 × 17_280 = 120_960` ledgers.
///
/// Used as the extension *threshold* for the instance and persistent tiers
/// ([`INSTANCE_LIFETIME_THRESHOLD`], [`PERSISTENT_LIFETIME_THRESHOLD`]).
pub const SEVEN_DAYS_LEDGERS: u32 = 7 * ONE_DAY_LEDGERS;

/// Thirty days expressed in ledgers: `30 × 17_280 = 518_400` ledgers.
///
/// Used as the extension *target* for the instance and persistent tiers
/// ([`INSTANCE_BUMP_AMOUNT`], [`PERSISTENT_BUMP_AMOUNT`]).
pub const THIRTY_DAYS_LEDGERS: u32 = 30 * ONE_DAY_LEDGERS;

/// Safety margin: `17_280 / 5 = 3_456` ledgers (~4.8 hours).
///
/// Monitoring constant. [`ExpirationMonitor`] reports [`AlertLevel::Critical`]
/// once an entry has this many ledgers or fewer remaining. It is also the
/// extension threshold for the temporary tier ([`TEMPORARY_LIFETIME_THRESHOLD`]),
/// so a temporary entry is refreshed before it gets within ~4.8 hours of expiry.
pub const SAFETY_MARGIN_LEDGERS: u32 = ONE_DAY_LEDGERS / 5;

/// Advance-warning window: `17_280` ledgers (24 hours).
///
/// Monitoring constant, applies to any tier. [`ExpirationMonitor`] reports
/// [`AlertLevel::Warning`] once an entry has this many ledgers or fewer remaining
/// (and more than [`SAFETY_MARGIN_LEDGERS`]).
pub const WARNING_THRESHOLD_LEDGERS: u32 = ONE_DAY_LEDGERS;

/// **Instance tier** threshold: `120_960` ledgers (7 days).
///
/// Pass as the first argument to `env.storage().instance().extend_ttl(..)`.
/// While the contract instance has more than 7 days left, the call is a no-op.
pub const INSTANCE_LIFETIME_THRESHOLD: u32 = SEVEN_DAYS_LEDGERS;

/// **Instance tier** bump target: `518_400` ledgers (30 days).
///
/// Pass as the second argument to `env.storage().instance().extend_ttl(..)`.
/// When the threshold is crossed, the instance TTL is raised back to 30 days.
pub const INSTANCE_BUMP_AMOUNT: u32 = THIRTY_DAYS_LEDGERS;

/// **Persistent tier** threshold: `120_960` ledgers (7 days).
///
/// Pass as the `threshold` argument to `env.storage().persistent().extend_ttl(key, ..)`.
/// Use for long-lived records such as escrows, stakes and balances.
pub const PERSISTENT_LIFETIME_THRESHOLD: u32 = SEVEN_DAYS_LEDGERS;

/// **Persistent tier** bump target: `518_400` ledgers (30 days).
///
/// Pass as the `extend_to` argument to `env.storage().persistent().extend_ttl(key, ..)`.
/// An archived persistent entry can be restored, but restoring costs a fee and
/// blocks the contract until it happens, so keep it bumped.
pub const PERSISTENT_BUMP_AMOUNT: u32 = THIRTY_DAYS_LEDGERS;

/// **Temporary tier** threshold: `3_456` ledgers (~4.8 hours, equal to
/// [`SAFETY_MARGIN_LEDGERS`]).
///
/// Pass as the `threshold` argument to `env.storage().temporary().extend_ttl(key, ..)`.
pub const TEMPORARY_LIFETIME_THRESHOLD: u32 = SAFETY_MARGIN_LEDGERS;

/// **Temporary tier** bump target: `17_280` ledgers (1 day).
///
/// Pass as the `extend_to` argument to `env.storage().temporary().extend_ttl(key, ..)`.
/// Temporary entries are deleted permanently when they expire, so only use this
/// tier for data that is safe to lose (nonces, rate-limit windows, markers).
pub const TEMPORARY_BUMP_AMOUNT: u32 = ONE_DAY_LEDGERS;

// ---------------------------------------------------------------------------
// Types & Enums
// ---------------------------------------------------------------------------

/// Severity of a storage entry's remaining lifetime, as computed by
/// [`ExpirationMonitor::assess_lifetime`].
///
/// Levels are ordered (`Safe < Warning < Critical < Expired`), so callers can
/// compare them directly, e.g. `if alert.level >= AlertLevel::Critical { .. }`.
/// The numeric discriminants are stable and are what gets published in the
/// `("ttl", "alert", key)` event payload.
///
/// | Level      | Remaining ledgers                                               |
/// |------------|-----------------------------------------------------------------|
/// | `Safe`     | more than [`WARNING_THRESHOLD_LEDGERS`] (> 24 h)                |
/// | `Warning`  | ≤ [`WARNING_THRESHOLD_LEDGERS`] and > [`SAFETY_MARGIN_LEDGERS`] |
/// | `Critical` | ≤ [`SAFETY_MARGIN_LEDGERS`] (≤ ~4.8 h) and > 0                  |
/// | `Expired`  | 0                                                               |
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AlertLevel {
    /// Safe: Remaining lifetime exceeds warning thresholds.
    Safe = 1,
    /// Warning: Remaining lifetime is within 24-hour warning window.
    Warning = 2,
    /// Critical: Remaining lifetime is within safety margin; immediate bump required.
    Critical = 3,
    /// Expired: Key has reached zero remaining ledgers.
    Expired = 4,
}

/// TTL health report for a single storage key, returned by
/// [`ExpirationMonitor::assess_lifetime`] and [`ExpirationMonitor::monitor_and_notify`].
///
/// A `TTLAlert` is a pure computation from ledger numbers; producing one never
/// reads or extends storage. Off-chain monitors typically consume it through the
/// `("ttl", "alert", key_symbol)` event emitted for any non-[`AlertLevel::Safe`] level.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TTLAlert {
    /// Caller-chosen label identifying the monitored key (e.g. `symbol_short!("escrow")`).
    pub key_symbol: Symbol,
    /// Severity bucket derived from `remaining_ledgers`.
    pub level: AlertLevel,
    /// Ledgers left before the entry expires, saturating at `0`.
    pub remaining_ledgers: u32,
    /// Warning window used for the assessment; always [`WARNING_THRESHOLD_LEDGERS`].
    pub warning_threshold: u32,
    /// `true` iff `remaining_ledgers == 0` (equivalently, `level == AlertLevel::Expired`).
    pub is_expired: bool,
}

/// A registered storage key dependency for an ongoing operation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyItem {
    pub key_hash: BytesN<32>,
    pub registered_at: u32,
}

/// Backup record for data recovery.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataBackupRecord {
    pub backup_id: Symbol,
    pub key_hash: BytesN<32>,
    pub payload: Bytes,
    pub timestamp: u64,
    pub restored: bool,
}

// ---------------------------------------------------------------------------
// Legacy / Heuristic TTL Helper Functions
// ---------------------------------------------------------------------------

/// Suggests a next bump interval (in seconds) based on the current TTL.
pub fn next_bump_interval(_env: &Env, current_ttl_secs: u64) -> u64 {
    if current_ttl_secs >= 86_400 {
        core::cmp::min(current_ttl_secs / 2, 86_400)
    } else if current_ttl_secs >= 3_600 {
        core::cmp::min(current_ttl_secs / 2, 1_800)
    } else {
        core::cmp::max(60, current_ttl_secs / 4)
    }
}

/// Decide whether to bump TTL now given remaining TTL and time since last bump.
pub fn should_bump_ttl(
    _env: &Env,
    remaining_ttl_secs: u64,
    time_since_last_bump_secs: u64,
    desired_persist_secs: u64,
) -> bool {
    if desired_persist_secs == 0 {
        return false;
    }
    if remaining_ttl_secs * 4 <= desired_persist_secs {
        return true;
    }
    if time_since_last_bump_secs * 2 >= desired_persist_secs {
        return true;
    }
    false
}

// ---------------------------------------------------------------------------
// Unified TTL Manager
// ---------------------------------------------------------------------------

/// Unified manager for consistent, standard TTL extensions across all storage tiers.
pub struct TTLManager;

impl TTLManager {
    /// Extend instance storage using the unified standard policy (7-day threshold, 30-day bump).
    pub fn extend_instance(env: &Env) {
        env.storage()
            .instance()
            .extend_ttl(INSTANCE_LIFETIME_THRESHOLD, INSTANCE_BUMP_AMOUNT);
    }

    /// Extend a persistent storage key using the unified standard policy (7-day threshold, 30-day bump).
    pub fn extend_persistent<K: IntoVal<Env, Val>>(env: &Env, key: &K) {
        env.storage().persistent().extend_ttl(
            key,
            PERSISTENT_LIFETIME_THRESHOLD,
            PERSISTENT_BUMP_AMOUNT,
        );
    }

    /// Extend a temporary storage key using the unified standard policy (safety-margin threshold, 1-day bump).
    pub fn extend_temporary<K: IntoVal<Env, Val>>(env: &Env, key: &K) {
        env.storage().temporary().extend_ttl(
            key,
            TEMPORARY_LIFETIME_THRESHOLD,
            TEMPORARY_BUMP_AMOUNT,
        );
    }

    /// Extend persistent storage with an extra safety margin to prevent mid-operation expiry.
    pub fn extend_persistent_with_margin<K: IntoVal<Env, Val>>(
        env: &Env,
        key: &K,
        extra_margin_ledgers: u32,
    ) {
        let threshold = PERSISTENT_LIFETIME_THRESHOLD.saturating_add(extra_margin_ledgers);
        let bump = PERSISTENT_BUMP_AMOUNT.saturating_add(extra_margin_ledgers);
        env.storage().persistent().extend_ttl(key, threshold, bump);
    }

    /// Automatically extend instance and persistent storage for an active operation.
    pub fn extend_active_operation<K: IntoVal<Env, Val>>(env: &Env, key: &K) {
        Self::extend_instance(env);
        Self::extend_persistent(env, key);
    }
}

// ---------------------------------------------------------------------------
// Data Dependency Tracking
// ---------------------------------------------------------------------------

/// Tracks critical storage keys that ongoing operations depend upon, preventing mid-operation expiration.
pub struct DataDependencyTracker;

impl DataDependencyTracker {
    /// Compute a deterministic 32-byte hash identifier for any serializable key.
    pub fn hash_key<K: ToXdr + Clone>(env: &Env, key: &K) -> BytesN<32> {
        let serialized = key.clone().to_xdr(env);
        env.crypto().sha256(&serialized).into()
    }

    /// Register a dependency key for an operation into temporary tracking storage.
    pub fn register_dependency<K: ToXdr + Clone>(
        env: &Env,
        operation_id: Symbol,
        key: &K,
    ) {
        let key_hash = Self::hash_key(env, key);
        let current_ledger = env.ledger().sequence();
        let item = DependencyItem {
            key_hash: key_hash.clone(),
            registered_at: current_ledger,
        };

        // Store under temporary storage namespace
        let dep_storage_key = (symbol_short!("dep_trk"), operation_id.clone(), key_hash.clone());
        env.storage().temporary().set(&dep_storage_key, &item);
        TTLManager::extend_temporary(env, &dep_storage_key);

        env.events().publish(
            (symbol_short!("ttl"), symbol_short!("dep_reg"), operation_id),
            (key_hash, current_ledger),
        );
    }

    /// Verify if a dependency is currently registered and active for an operation.
    pub fn is_dependency_active<K: ToXdr + Clone>(
        env: &Env,
        operation_id: Symbol,
        key: &K,
    ) -> bool {
        let key_hash = Self::hash_key(env, key);
        let dep_storage_key = (symbol_short!("dep_trk"), operation_id, key_hash);
        env.storage().temporary().has(&dep_storage_key)
    }

    /// Clear a dependency when an operation completes.
    pub fn clear_dependency<K: ToXdr + Clone>(
        env: &Env,
        operation_id: Symbol,
        key: &K,
    ) {
        let key_hash = Self::hash_key(env, key);
        let dep_storage_key = (symbol_short!("dep_trk"), operation_id, key_hash);
        env.storage().temporary().remove(&dep_storage_key);
    }
}

// ---------------------------------------------------------------------------
// Expiration Monitoring & Alerts
// ---------------------------------------------------------------------------

/// Monitors remaining storage lifetimes and generates advance warnings.
pub struct ExpirationMonitor;

impl ExpirationMonitor {
    /// Assess the health of a key based on elapsed ledgers since last bump.
    pub fn assess_lifetime(
        current_ledger: u32,
        last_bump_ledger: u32,
        total_bump_ledgers: u32,
        key_symbol: Symbol,
    ) -> TTLAlert {
        let elapsed = current_ledger.saturating_sub(last_bump_ledger);
        let remaining = total_bump_ledgers.saturating_sub(elapsed);

        let (level, is_expired) = if remaining == 0 {
            (AlertLevel::Expired, true)
        } else if remaining <= SAFETY_MARGIN_LEDGERS {
            (AlertLevel::Critical, false)
        } else if remaining <= WARNING_THRESHOLD_LEDGERS {
            (AlertLevel::Warning, false)
        } else {
            (AlertLevel::Safe, false)
        };

        TTLAlert {
            key_symbol,
            level,
            remaining_ledgers: remaining,
            warning_threshold: WARNING_THRESHOLD_LEDGERS,
            is_expired,
        }
    }

    /// Publish an alert event if the key is in warning, critical, or expired status.
    pub fn monitor_and_notify(
        env: &Env,
        current_ledger: u32,
        last_bump_ledger: u32,
        total_bump_ledgers: u32,
        key_symbol: Symbol,
    ) -> TTLAlert {
        let alert = Self::assess_lifetime(
            current_ledger,
            last_bump_ledger,
            total_bump_ledgers,
            key_symbol.clone(),
        );

        if alert.level != AlertLevel::Safe {
            env.events().publish(
                (symbol_short!("ttl"), symbol_short!("alert"), key_symbol),
                (alert.level as u32, alert.remaining_ledgers, alert.is_expired),
            );
        }

        alert
    }
}

// ---------------------------------------------------------------------------
// Data Recovery & Restoration
// ---------------------------------------------------------------------------

/// Manages backup snapshots and restoration for expired or recoverable storage data.
pub struct TTLRecoveryManager;

impl TTLRecoveryManager {
    /// Backup serialized state to persistent storage for disaster recovery.
    pub fn backup_data(
        env: &Env,
        backup_id: Symbol,
        key_hash: BytesN<32>,
        payload: Bytes,
    ) {
        let record = DataBackupRecord {
            backup_id: backup_id.clone(),
            key_hash: key_hash.clone(),
            payload,
            timestamp: env.ledger().timestamp(),
            restored: false,
        };

        let storage_key = (symbol_short!("backup"), backup_id.clone(), key_hash.clone());
        env.storage().persistent().set(&storage_key, &record);
        TTLManager::extend_persistent(env, &storage_key);

        env.events().publish(
            (symbol_short!("ttl"), symbol_short!("backup"), backup_id),
            (key_hash, env.ledger().timestamp()),
        );
    }

    /// Check if a backup exists for a key.
    pub fn has_backup(
        env: &Env,
        backup_id: Symbol,
        key_hash: &BytesN<32>,
    ) -> bool {
        let storage_key = (symbol_short!("backup"), backup_id, key_hash.clone());
        env.storage().persistent().has(&storage_key)
    }

    /// Retrieve and restore backed-up data for an accidentally expired key.
    pub fn restore_data(
        env: &Env,
        backup_id: Symbol,
        key_hash: &BytesN<32>,
    ) -> Option<Bytes> {
        let storage_key = (symbol_short!("backup"), backup_id.clone(), key_hash.clone());
        if let Some(mut record) = env
            .storage()
            .persistent()
            .get::<_, DataBackupRecord>(&storage_key)
        {
            record.restored = true;
            env.storage().persistent().set(&storage_key, &record);
            TTLManager::extend_persistent(env, &storage_key);

            env.events().publish(
                (symbol_short!("ttl"), symbol_short!("restore"), backup_id),
                key_hash.clone(),
            );

            Some(record.payload)
        } else {
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{symbol_short, Address, Env};

    #[test]
    fn test_constants_and_safety_margins() {
        assert_eq!(ONE_DAY_LEDGERS, 17_280);
        assert_eq!(SEVEN_DAYS_LEDGERS, 120_960);
        assert_eq!(THIRTY_DAYS_LEDGERS, 518_400);
        assert!(SAFETY_MARGIN_LEDGERS > 0);
        assert!(WARNING_THRESHOLD_LEDGERS >= ONE_DAY_LEDGERS);
        assert!(INSTANCE_LIFETIME_THRESHOLD < INSTANCE_BUMP_AMOUNT);
        assert!(PERSISTENT_LIFETIME_THRESHOLD < PERSISTENT_BUMP_AMOUNT);
    }

    #[test]
    fn test_heuristic_bump_logic() {
        let env = Env::default();
        assert_eq!(next_bump_interval(&env, 100_000), 50_000);
        assert_eq!(next_bump_interval(&env, 10_000), 1_800);
        assert_eq!(next_bump_interval(&env, 100), 60);

        assert!(should_bump_ttl(&env, 10, 100, 100)); // remaining (10*4=40) <= 100
        assert!(!should_bump_ttl(&env, 80, 10, 100)); // safe
        assert!(!should_bump_ttl(&env, 80, 10, 0)); // zero desired
    }

    #[test]
    fn test_expiration_monitor_alert_levels() {
        let sym = symbol_short!("escrow");

        // Safe: 30 days bump, 5 days elapsed -> ~25 days remaining
        let safe_alert = ExpirationMonitor::assess_lifetime(
            5 * ONE_DAY_LEDGERS,
            0,
            THIRTY_DAYS_LEDGERS,
            sym.clone(),
        );
        assert_eq!(safe_alert.level, AlertLevel::Safe);
        assert!(!safe_alert.is_expired);

        // Warning: 29.5 days elapsed -> 0.5 days (8640 ledgers) remaining (< 1 day warning window)
        let warn_alert = ExpirationMonitor::assess_lifetime(
            THIRTY_DAYS_LEDGERS - 8_640,
            0,
            THIRTY_DAYS_LEDGERS,
            sym.clone(),
        );
        assert_eq!(warn_alert.level, AlertLevel::Warning);
        assert!(!warn_alert.is_expired);

        // Critical: remaining within safety margin (e.g. 1000 ledgers < 3456)
        let crit_alert = ExpirationMonitor::assess_lifetime(
            THIRTY_DAYS_LEDGERS - 1_000,
            0,
            THIRTY_DAYS_LEDGERS,
            sym.clone(),
        );
        assert_eq!(crit_alert.level, AlertLevel::Critical);
        assert!(!crit_alert.is_expired);

        // Expired
        let exp_alert = ExpirationMonitor::assess_lifetime(
            THIRTY_DAYS_LEDGERS + 100,
            0,
            THIRTY_DAYS_LEDGERS,
            sym,
        );
        assert_eq!(exp_alert.level, AlertLevel::Expired);
        assert!(exp_alert.is_expired);
        assert_eq!(exp_alert.remaining_ledgers, 0);
    }

    #[test]
    fn test_dependency_tracking_lifecycle() {
        let env = Env::default();
        let op_id = symbol_short!("esc_101");
        let sample_key = symbol_short!("data_key");

        assert!(!DataDependencyTracker::is_dependency_active(
            &env,
            op_id.clone(),
            &sample_key
        ));

        // Register dependency
        DataDependencyTracker::register_dependency(&env, op_id.clone(), &sample_key);
        assert!(DataDependencyTracker::is_dependency_active(
            &env,
            op_id.clone(),
            &sample_key
        ));

        // Clear dependency
        DataDependencyTracker::clear_dependency(&env, op_id.clone(), &sample_key);
        assert!(!DataDependencyTracker::is_dependency_active(
            &env,
            op_id,
            &sample_key
        ));
        assert!(!DataDependencyTracker::is_dependency_active(&env, op_id.clone(), &sample_key));

        // Register dependency
        DataDependencyTracker::register_dependency(&env, op_id.clone(), &sample_key);
        assert!(DataDependencyTracker::is_dependency_active(&env, op_id.clone(), &sample_key));

        // Clear dependency
        DataDependencyTracker::clear_dependency(&env, op_id.clone(), &sample_key);
        assert!(!DataDependencyTracker::is_dependency_active(&env, op_id, &sample_key));
    }

    #[test]
    fn test_ttl_recovery_manager() {
        let env = Env::default();
        let backup_id = symbol_short!("dr_01");
        let key_hash = BytesN::from_array(&env, &[0xfe; 32]);
        let mut sample_payload = Bytes::new(&env);
        sample_payload.push_back(42);
        sample_payload.push_back(99);

        assert!(!TTLRecoveryManager::has_backup(
            &env,
            backup_id.clone(),
            &key_hash
        ));

        TTLRecoveryManager::backup_data(
            &env,
            backup_id.clone(),
            key_hash.clone(),
            sample_payload.clone(),
        );
        assert!(!TTLRecoveryManager::has_backup(&env, backup_id.clone(), &key_hash));

        TTLRecoveryManager::backup_data(&env, backup_id.clone(), key_hash.clone(), sample_payload.clone());
        assert!(TTLRecoveryManager::has_backup(&env, backup_id.clone(), &key_hash));

        let restored = TTLRecoveryManager::restore_data(&env, backup_id, &key_hash);
        assert_eq!(restored, Some(sample_payload));
    }
}

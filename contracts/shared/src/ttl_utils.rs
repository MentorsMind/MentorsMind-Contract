//! Unified Time-To-Live (TTL) management, dependency tracking, expiration monitoring,
//! and data recovery utilities for Soroban contracts.
//!
//! Enforces consistent TTL extension policies across all contract storage tiers (instance,
//! persistent, temporary) to prevent unexpected data expiration during active operations.

use soroban_sdk::{
    contracttype, symbol_short, xdr::ToXdr, Bytes, BytesN, Env, IntoVal, Symbol, Val,
};

// ---------------------------------------------------------------------------
// Unified TTL Constants & Safety Margins (Assuming ~5s Stellar ledger close time)
// ---------------------------------------------------------------------------

/// 1 day in ledgers (~17,280 ledgers assuming 5-second close times).
pub const ONE_DAY_LEDGERS: u32 = 17_280;

/// 7 days in ledgers (~120,960 ledgers).
pub const SEVEN_DAYS_LEDGERS: u32 = 7 * ONE_DAY_LEDGERS;

/// 30 days in ledgers (~518,400 ledgers).
pub const THIRTY_DAYS_LEDGERS: u32 = 30 * ONE_DAY_LEDGERS;

/// Safety margin ledgers (~4.8 hours = 3,456 ledgers) added to prevent race conditions.
pub const SAFETY_MARGIN_LEDGERS: u32 = ONE_DAY_LEDGERS / 5;

/// 24-hour advance warning threshold for expiration monitoring (~17,280 ledgers).
pub const WARNING_THRESHOLD_LEDGERS: u32 = ONE_DAY_LEDGERS;

/// Standard threshold below which instance storage should be extended.
pub const INSTANCE_LIFETIME_THRESHOLD: u32 = SEVEN_DAYS_LEDGERS;

/// Standard bump amount for instance storage (extends to 30 days).
pub const INSTANCE_BUMP_AMOUNT: u32 = THIRTY_DAYS_LEDGERS;

/// Standard threshold below which persistent storage entries should be extended.
pub const PERSISTENT_LIFETIME_THRESHOLD: u32 = SEVEN_DAYS_LEDGERS;

/// Standard bump amount for persistent storage entries (extends to 30 days).
pub const PERSISTENT_BUMP_AMOUNT: u32 = THIRTY_DAYS_LEDGERS;

/// Standard threshold below which temporary storage entries should be extended.
pub const TEMPORARY_LIFETIME_THRESHOLD: u32 = SAFETY_MARGIN_LEDGERS;

/// Standard bump amount for temporary storage entries (extends to 1 day).
pub const TEMPORARY_BUMP_AMOUNT: u32 = ONE_DAY_LEDGERS;

// ---------------------------------------------------------------------------
// Types & Enums
// ---------------------------------------------------------------------------

/// Alert severity for storage expiration monitoring.
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

/// Detailed TTL monitoring report.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TTLAlert {
    pub key_symbol: Symbol,
    pub level: AlertLevel,
    pub remaining_ledgers: u32,
    pub warning_threshold: u32,
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

        assert!(!DataDependencyTracker::is_dependency_active(&env, op_id, &sample_key));

        // Register dependency
        DataDependencyTracker::register_dependency(&env, op_id, &sample_key);
        assert!(DataDependencyTracker::is_dependency_active(&env, op_id, &sample_key));

        // Clear dependency
        DataDependencyTracker::clear_dependency(&env, op_id, &sample_key);
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

        assert!(!TTLRecoveryManager::has_backup(&env, backup_id, &key_hash));

        TTLRecoveryManager::backup_data(&env, backup_id, key_hash.clone(), sample_payload.clone());
        assert!(TTLRecoveryManager::has_backup(&env, backup_id, &key_hash));

        let restored = TTLRecoveryManager::restore_data(&env, backup_id, &key_hash);
        assert_eq!(restored, Some(sample_payload));
    }
}

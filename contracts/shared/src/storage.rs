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

use soroban_sdk::{contracttype, Env, IntoVal, TryFromVal, Val};

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
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;

    use super::*;
    use soroban_sdk::{symbol_short, Env};

    #[test]
    fn test_instance_set_get() {
        let env = Env::default();
        EternalStorage::set_instance(&env, &InstanceKey::PlatformFee, &500u32);
        let fee: Option<u32> = EternalStorage::get_instance(&env, &InstanceKey::PlatformFee);
        assert_eq!(fee, Some(500u32));
    }

    #[test]
    fn test_instance_has_remove() {
        let env = Env::default();
        assert!(!EternalStorage::has_instance(&env, &InstanceKey::Paused));
        EternalStorage::set_instance(&env, &InstanceKey::Paused, &true);
        assert!(EternalStorage::has_instance(&env, &InstanceKey::Paused));
        EternalStorage::remove_instance(&env, &InstanceKey::Paused);
        assert!(!EternalStorage::has_instance(&env, &InstanceKey::Paused));
    }

    #[test]
    fn test_persistent_set_get() {
        let env = Env::default();
        EternalStorage::set_persistent(&env, &PersistentKey::Escrow(42u64), &9999i128);
        let val: Option<i128> = EternalStorage::get_persistent(&env, &PersistentKey::Escrow(42u64));
        assert_eq!(val, Some(9999i128));
    }

    #[test]
    fn test_persistent_remove() {
        let env = Env::default();
        EternalStorage::set_persistent(&env, &PersistentKey::AllocHistory, &1u32);
        assert!(EternalStorage::has_persistent(&env, &PersistentKey::AllocHistory));
        EternalStorage::remove_persistent(&env, &PersistentKey::AllocHistory);
        assert!(!EternalStorage::has_persistent(&env, &PersistentKey::AllocHistory));
    }

    #[test]
    fn test_temporary_set_get() {
        let env = Env::default();
        let key = TempKey::ReentrancyLock(symbol_short!("escrow"));
        EternalStorage::set_temporary(&env, &key, &true);
        let val: Option<bool> = EternalStorage::get_temporary(&env, &key);
        assert_eq!(val, Some(true));
    }

    #[test]
    fn test_schema_version_tracking() {
        let env = Env::default();
        // Default: no version stored
        let v: Option<u32> = EternalStorage::get_instance(&env, &InstanceKey::SchemaVersion);
        assert_eq!(v, None);
        // Set version 1
        EternalStorage::set_instance(&env, &InstanceKey::SchemaVersion, &1u32);
        let v: Option<u32> = EternalStorage::get_instance(&env, &InstanceKey::SchemaVersion);
        assert_eq!(v, Some(1u32));
        // Upgrade to version 2
        EternalStorage::set_instance(&env, &InstanceKey::SchemaVersion, &2u32);
        let v: Option<u32> = EternalStorage::get_instance(&env, &InstanceKey::SchemaVersion);
        assert_eq!(v, Some(2u32));
    }
}

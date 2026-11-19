# Storage Pattern Guide

## Overview

This guide explains the storage architecture in MentorsMind contracts, covering the three storage tiers, when to use `EternalStorage` vs raw storage, and how to leverage `SecureStorageAccess` for namespace isolation.

## The Three Storage Tiers

Soroban provides three storage tiers, each with different cost and durability characteristics:

### 1. Instance Storage

**Purpose:** Contract-wide configuration that is read on (almost) every invocation.

**Characteristics:**
- Lives as long as the contract instance itself
- Cheap reads (lowest cost tier)
- Automatically shares lifetime with the contract
- Cannot be selectively archived or restored

**Use for:**
- Admin address
- Platform fee percentage
- Pause state
- Global counters (total escrow count, proposal count)
- Schema version for migrations

**Example:**
```rust
use shared::storage::{EternalStorage, InstanceKey};

// Write admin during initialization
EternalStorage::set_instance(&env, &InstanceKey::Admin, &admin_address);

// Read admin on every guarded operation
let admin: Address = EternalStorage::get_instance(&env, &InstanceKey::Admin)
    .expect("Contract not initialized");
```

**TTL Management:**
Instance storage TTL is extended with `INSTANCE_LIFETIME_THRESHOLD` (7 days) and `INSTANCE_BUMP_AMOUNT` (30 days). Extend it on every state-changing entry point:

```rust
use shared::ttl_utils::TTLManager;

pub fn my_entry_point(env: Env, ...) {
    TTLManager::extend_instance(&env);
    // ... rest of function
}
```

### 2. Persistent Storage

**Purpose:** Long-lived, per-entity data that must never be lost (funds-bearing state).

**Characteristics:**
- Survives indefinitely with regular TTL extension
- Mid-tier cost
- Can be archived and restored if TTL expires
- Requires explicit per-key TTL extension

**Use for:**
- Escrow records (amount, mentor, learner, status)
- Staking balances
- Proposal details
- Referral aggregates
- Audit ledger entries (reconciliation records)

**Example:**
```rust
use shared::storage::{EternalStorage, PersistentKey};

// Write escrow record
EternalStorage::set_persistent(&env, &PersistentKey::Escrow(escrow_id), &escrow);

// Read escrow record
let escrow: Option<Escrow> = EternalStorage::get_persistent(&env, &PersistentKey::Escrow(escrow_id));
```

**TTL Management:**
Persistent storage uses `PERSISTENT_LIFETIME_THRESHOLD` (7 days) and `PERSISTENT_BUMP_AMOUNT` (30 days). Extend TTL on **every read and write** of critical keys:

```rust
use shared::ttl_utils::TTLManager;

pub fn release_escrow(env: Env, escrow_id: u64) {
    let key = PersistentKey::Escrow(escrow_id);
    
    // Extend TTL when reading
    let mut escrow: Escrow = EternalStorage::get_persistent(&env, &key)
        .expect("Escrow not found");
    TTLManager::extend_persistent(&env, &key);
    
    // ... perform release logic ...
    
    // Extend TTL when writing
    escrow.status = EscrowStatus::Released;
    EternalStorage::set_persistent(&env, &key, &escrow);
    TTLManager::extend_persistent(&env, &key);
}
```

**Critical: Audit Trail TTL**

For immutable audit records (like reconciliation ledger entries), use the maximum allowed TTL to ensure they **never expire**:

```rust
env.storage().persistent().set(&DataKey::Record(id), &record);
env.storage().persistent().extend_ttl(
    &DataKey::Record(id),
    PERSISTENT_LIFETIME_THRESHOLD,
    PERSISTENT_BUMP_AMOUNT  // 30 days, effectively permanent with regular access
);
```

### 3. Temporary Storage

**Purpose:** Short-lived, disposable data that can be safely discarded.

**Characteristics:**
- Cheapest storage tier
- Expires after ~1 day by default
- **Cannot be restored** once expired (permanently deleted)
- Useful for transient state

**Use for:**
- Anti-replay nonces (valid for one transaction)
- Rate-limit counters (hourly/daily windows)
- Reentrancy locks (cleared after operation completes)
- Dependency tracking markers (operations in flight)

**Example:**
```rust
use shared::storage::{EternalStorage, TempKey};

// Set a nonce
EternalStorage::set_temporary(&env, &TempKey::Nonce(user_address.clone()), &nonce);

// Check and consume nonce
if let Some(stored_nonce) = EternalStorage::get_temporary::<_, u64>(&env, &TempKey::Nonce(user_address.clone())) {
    if stored_nonce == provided_nonce {
        // Consume by removing
        EternalStorage::remove_temporary(&env, &TempKey::Nonce(user_address.clone()));
        // ... proceed with operation
    }
}
```

**TTL Management:**
Temporary storage uses `TEMPORARY_LIFETIME_THRESHOLD` (safety margin, ~4.8 hours) and `TEMPORARY_BUMP_AMOUNT` (1 day):

```rust
use shared::ttl_utils::TTLManager;

// Extend temporary key TTL
TTLManager::extend_temporary(&env, &TempKey::RateLimit(user.clone(), window));
```

**Warning:** Never store funds-bearing state in temporary storage. Once expired, it is **gone forever** with no restore mechanism.

## EternalStorage Helper

The `EternalStorage` abstraction provides a unified API across all three tiers and makes storage tier choice explicit at the call site.

### Benefits

1. **Explicit Tier Selection:** The function name tells you which tier you're using
2. **Type Safety:** All keys and values are strongly typed
3. **Centralized Logic:** TTL helpers, collision detection, and integrity checks in one place
4. **Upgrade Safety:** Adding new fields doesn't break old data layout

### Migration Example

**Before (raw storage):**
```rust
pub fn set_admin(env: Env, admin: Address) {
    env.storage().persistent().set(&Symbol::short("ADMIN"), &admin);
}

pub fn get_admin(env: Env) -> Address {
    env.storage().persistent()
        .get(&Symbol::short("ADMIN"))
        .expect("Not initialized")
}
```

**After (EternalStorage):**
```rust
use shared::storage::{EternalStorage, InstanceKey};

pub fn set_admin(env: Env, admin: Address) {
    EternalStorage::set_instance(&env, &InstanceKey::Admin, &admin);
    EternalStorage::extend_instance_ttl(&env);  // Built-in TTL helper
}

pub fn get_admin(env: Env) -> Address {
    EternalStorage::get_instance(&env, &InstanceKey::Admin)
        .expect("Not initialized")
}
```

### Automatic TTL Extension

`EternalStorage` includes helper methods that automatically call the correct TTL extension for each tier:

```rust
// Extend instance storage (contract-wide)
EternalStorage::extend_instance_ttl(&env);

// Extend persistent key
EternalStorage::extend_persistent_ttl(&env, &PersistentKey::Escrow(id));

// Extend temporary key
EternalStorage::extend_temporary_ttl(&env, &TempKey::Nonce(user));
```

These are thin wrappers around `TTLManager` and ensure you use the correct threshold/bump constants.

## SecureStorageAccess: Namespace Isolation

The `SecureStorageAccess` API provides **contract-isolated namespaces** to prevent key collisions and unauthorized cross-contract access.

### Why Namespaces?

In a multi-contract system, two contracts might accidentally use the same storage key format, causing collisions. Namespaces solve this by deriving unique prefixes per contract.

### Usage

#### 1. Install a Namespace (during initialization)

```rust
use shared::storage::{SecureStorageAccess, DataKey};
use soroban_sdk::symbol_short;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    NamespaceRoot,
    Config,
    Record(u64),
}

pub fn initialize(env: Env, admin: Address) {
    // Install namespace with scope identifier
    let _ns = SecureStorageAccess::install_namespace(
        &env,
        &DataKey::NamespaceRoot,
        symbol_short!("escrow_v2")
    );
    
    // Store admin using raw storage (NamespaceRoot itself isn't namespaced)
    env.storage().instance().set(&DataKey::NamespaceRoot, &admin);
}
```

#### 2. Write Data with Collision Detection

```rust
use soroban_sdk::symbol_short;

pub fn create_record(env: Env, id: u64, data: RecordData) {
    SecureStorageAccess::set_persistent_checked(
        &env,
        &DataKey::NamespaceRoot,     // namespace root key
        symbol_short!("record"),     // context symbol
        &DataKey::Record(id),        // logical key
        &data                        // value
    ).expect("Storage collision or namespace error");
}
```

**What happens:**
- `set_persistent_checked` derives a unique hash from (contract_address, namespace_scope, context, logical_key)
- It checks the collision registry to ensure no other namespace has used this hash
- It registers the fingerprint to prevent future collisions
- It records an integrity checksum for tamper detection

#### 3. Read Data with Integrity Verification

```rust
pub fn get_record(env: Env, id: u64) -> Option<RecordData> {
    SecureStorageAccess::get_persistent_checked(
        &env,
        &DataKey::NamespaceRoot,
        &DataKey::Record(id)
    )
    .expect("Integrity check failed")
}
```

**What happens:**
- Derives the same unique hash
- Reads the value from persistent storage
- Verifies the value's checksum matches the stored integrity record
- Returns the value or an integrity error if tampered

### When to Use SecureStorageAccess

**Use SecureStorageAccess when:**
- Building a multi-contract system where contracts might share storage patterns
- You need tamper detection (integrity checksums)
- Paranoid security requirements justify the extra overhead
- You're implementing sensitive infrastructure (admin rotation, treasury, multisig)

**Use plain EternalStorage when:**
- Simple, single-contract use case
- Performance is critical (SecureStorageAccess has ~2x overhead)
- You trust the contract environment and don't need checksums

## Upgrade Safety Patterns

### Adding New Fields

**Safe:**
```rust
// Old version
#[contracttype]
enum DataKey {
    Config,
    Admin,
}

// New version — just add new keys
#[contracttype]
enum DataKey {
    Config,
    Admin,
    PauseGuardian,  // NEW
    FeeOverride,    // NEW
}
```

Contracts upgraded this way can read old data (Config, Admin) and write new data (PauseGuardian, FeeOverride) without any migration.

### Renaming or Removing Fields

**Requires migration:**
```rust
// V1 stored data under DataKey::OldName
// V2 wants to read it as DataKey::NewName

pub fn migrate_v1_to_v2(env: Env) {
    if let Some(value) = EternalStorage::get_persistent(&env, &DataKey::OldName) {
        EternalStorage::set_persistent(&env, &DataKey::NewName, &value);
        EternalStorage::remove_persistent(&env, &DataKey::OldName);
    }
}
```

Call `migrate_v1_to_v2` as part of the upgrade initialization flow (guarded by a schema version check).

### Schema Versioning

```rust
pub fn initialize_or_migrate(env: Env) {
    let version: u32 = EternalStorage::get_instance(&env, &InstanceKey::SchemaVersion)
        .unwrap_or(0);
    
    match version {
        0 => {
            // Fresh initialization
            EternalStorage::set_instance(&env, &InstanceKey::SchemaVersion, &1u32);
        },
        1 => {
            // Migrate from v1 to v2
            migrate_v1_to_v2(&env);
            EternalStorage::set_instance(&env, &InstanceKey::SchemaVersion, &2u32);
        },
        2 => {
            // Current version, no migration needed
        },
        _ => panic!("Unknown schema version"),
    }
}
```

## Summary Table

| Tier       | Cost    | Lifetime           | Restoration | Use Case                        |
|------------|---------|--------------------|-------------|---------------------------------|
| Instance   | Cheapest| Contract lifetime  | N/A         | Config, admin, flags, counters  |
| Persistent | Mid     | Indefinite w/ TTL  | Can restore | Escrows, balances, records      |
| Temporary  | Low     | ~1 day             | **Cannot** restore | Nonces, rate limits, locks |

## Best Practices

1. **Always extend TTL** on read and write for persistent keys that hold funds or critical state
2. **Never store funds in temporary storage** — it expires permanently
3. **Prefer EternalStorage** over raw `env.storage()` for production code
4. **Use SecureStorageAccess** for multi-contract systems or high-security requirements
5. **Version your schema** and write migration logic for breaking storage changes
6. **Audit ledger records** should use maximum TTL (PERSISTENT_BUMP_AMOUNT) to ensure indefinite retention

## References

- [Soroban Storage Documentation](https://soroban.stellar.org/docs/fundamentals-and-concepts/storage)
- [TTL Management (shared::ttl_utils)](../contracts/shared/src/ttl_utils.rs)
- [EternalStorage Implementation](../contracts/shared/src/storage.rs)
- [CONTRIBUTING.md](../CONTRIBUTING.md)

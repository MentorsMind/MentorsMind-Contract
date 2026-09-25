# Upgrade Guide — MentorsMind Storage Migration

This guide explains how to safely upgrade MentorsMind Soroban contracts
without corrupting existing ledger state.

## How Soroban storage serialisation works

Soroban serialises `#[contracttype]` enums and structs using XDR. The encoding
is **position-sensitive**:

- Enum discriminants are determined by **variant order**, not variant name.
- Struct fields are encoded in **declaration order**.

This means renaming a variant is safe (the discriminant is unchanged), but
**reordering variants or fields is always breaking**.

## The Eternal Storage pattern in this codebase

Every contract stores state through typed `DataKey` enums:

```rust
#[contracttype]
pub enum DataKey {
    Admin,
    Stake(Address),      // key for a staker's record
    EpochReward(u64),    // key for epoch N's reward
}
```

Storage values are `#[contracttype]` structs:

```rust
#[contracttype]
pub struct StakeRecord {
    pub mentor: Address,
    pub amount: i128,
    pub staked_at: u64,
    pub unlock_at: u64,
    pub tier: u32,
}
```

The `shared::storage::EternalStorage` wrapper provides a uniform API so all
access goes through one place, making audits straightforward.

---

## Breaking vs. compatible changes

### ❌ Breaking — requires migration before upgrade

| Change | Why it breaks |
|--------|--------------|
| Remove an enum variant | Shifts XDR discriminants of all following variants |
| Reorder enum variants | Discriminants no longer match on-disk values |
| Remove a struct field | XDR decoder expects a fixed field count |
| Change a field's type | On-disk bytes cannot decode to the new type |
| Reorder struct fields | XDR is position-sensitive |
| Change variant payload types | Existing keys cannot decode with new payload |

### 🟡 Warning — migration recommended

| Change | Why it needs care |
|--------|------------------|
| Add a field to an existing struct | Old entries lack the new field; code must handle `None` or a default |
| Rename a storage key enum | Old key strings become orphaned until a migration reads and deletes them |

### 🟢 Compatible — safe to deploy

| Change | Why it's safe |
|--------|--------------|
| Add a new variant at the end of a DataKey enum | Appending preserves existing discriminants |
| Add a new struct | No existing storage to migrate |
| Add a new contract | No existing storage to migrate |

---

## Pre-upgrade checklist

Before scheduling an upgrade in the `UpgradeRegistry`:

1. **Run the storage validator locally**

   ```bash
   # Build the tool
   cargo build --release -p state-transition-analyzer --bin storage-validator

   # Snapshot the current (production) schemas
   ./target/release/storage-validator snapshot --version v1-prod

   # After making your changes, check for breaking changes
   ./target/release/storage-validator check --baseline v1-prod
   ```

2. **Review the migration report**

   The tool writes `storage-snapshots/migration-report.md` with a categorised
   list of all detected changes. All `Breaking` items must be resolved.

3. **Write a migration script** if breaking changes are unavoidable

   Strategies:
   - **Key migration**: write a one-time admin function that reads old keys,
     transforms them, writes to new keys, then removes old keys.
   - **Versioned structs**: use an explicit `schema_version: u32` field and
     handle old versions with a fallback in the getter.
   - **Lazy migration**: read-and-upgrade on first access, so migration is
     amortised across user interactions.

4. **Update `InstanceKey::SchemaVersion`** in `shared/src/storage.rs`

   Increment the `SchemaVersion` instance key so on-chain tooling can detect
   which schema version is active.

5. **Schedule the upgrade through the UpgradeRegistry**

   ```bash
   # The 48-hour timelock gives the team time to abort if issues arise.
   upgrade-registry schedule_upgrade <new_wasm_hash> <contract_name> <new_version> <changelog_hash> <approvers>
   ```

---

## Automated CI validation

Every PR that touches contract source files triggers the
`Storage Migration Validation` workflow. It:

1. Builds the `storage-validator` binary.
2. Scans the current workspace for `#[contracttype]` definitions.
3. Compares against the latest committed baseline snapshot in `storage-snapshots/baseline/`.
4. Posts a report to the PR comment.
5. Fails the check (exit 1) if any breaking changes are detected.

Inline GitHub Actions annotations are emitted for each breaking change, so they
appear directly in the PR diff view.

### Updating the baseline

When a migration has been completed and the new schema is intentionally the
new baseline:

**Option A — CI (recommended):**
Trigger the `Storage Migration Validation` workflow manually from the Actions
tab with `update_baseline = true`.

**Option B — local:**
```bash
./target/release/storage-validator snapshot --version baseline
git add storage-snapshots/baseline/
git commit -m "chore(storage): update schema baseline after migration"
```

---

## Snapshot format

Snapshots are stored as JSON in `storage-snapshots/<version>/schema.json`:

```json
{
  "version": "baseline",
  "captured_at": "2026-01-15",
  "ref_name": "main",
  "sha": "abc12345",
  "contracts": [
    {
      "contract": "mentorminds-staking",
      "source_path": "contracts/staking/src/lib.rs",
      "enums": [
        {
          "name": "DataKey",
          "inferred_tier": "persistent",
          "variants": [
            { "name": "Admin", "fields": [], "comment": null },
            { "name": "Stake", "fields": ["Address"], "comment": null }
          ]
        }
      ],
      "structs": [
        {
          "name": "StakeRecord",
          "fields": [
            { "name": "mentor", "ty": "Address" },
            { "name": "amount", "ty": "i128" }
          ]
        }
      ]
    }
  ]
}
```

---

## Adding a new contract to validation

The scanner discovers contracts automatically by walking the workspace for
`src/lib.rs` files containing `#[contracttype]` definitions. No manual
registration is needed — add the contract to the workspace `Cargo.toml`
members and it will appear in the next snapshot.

# Upgrade Guide

Step-by-step procedures for upgrading MentorsMind Soroban contracts.

---

## Overview

MentorsMind uses the **UUPS pattern** for contract upgrades on Soroban:

- The upgrade function lives inside the contract itself (`upgrade_contract`).
- Upgrades are authorized by the admin (in production: the multisig contract).
- After upgrade, the contract runs new WASM at the **same contract address**.
- All storage is preserved — no migration needed for additive changes.

---

## Prerequisites

```bash
# Install Soroban CLI
cargo install --locked soroban-cli

# Configure testnet
soroban config network add testnet \
  --rpc-url https://soroban-testnet.stellar.org:443 \
  --network-passphrase "Test SDF Network ; September 2015"

# Set your identity
export ADMIN_KEY=<your-secret-key>
soroban config identity add admin --secret-key $ADMIN_KEY
```

---

## Step 1: Build the New WASM

```bash
# Build the contract you want to upgrade (e.g. upgrade_registry)
cd contracts/upgrade_registry
cargo build --target wasm32-unknown-unknown --release

# Optimize
soroban contract optimize \
  --wasm ../../target/wasm32-unknown-unknown/release/mentorminds_upgrade_registry.wasm
```

---

## Step 2: Upload the New WASM

Upload the WASM to the network and get its hash. The contract code is stored on-chain separately from the contract instance.

```bash
NEW_WASM_HASH=$(soroban contract upload \
  --source admin \
  --network testnet \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_upgrade_registry.optimized.wasm)

echo "New WASM hash: $NEW_WASM_HASH"
```

---

## Step 3: Schedule via Timelock (Production)

In production, upgrades go through the timelock for community review.

```bash
# Schedule the upgrade (48h delay for contract upgrades)
OP_ID=$(soroban contract invoke \
  --id $TIMELOCK_CONTRACT_ID \
  --source admin \
  --network testnet \
  -- schedule \
  --caller $ADMIN_ADDRESS \
  --target $UPGRADE_REGISTRY_CONTRACT_ID \
  --function upgrade_contract \
  --args "[\"$NEW_WASM_HASH\", \"upgrade_registry\", 2, \"$CHANGELOG_HASH\"]" \
  --delay 172800)  # 48 hours

echo "Operation ID: $OP_ID"
```

Wait 48 hours, then execute:

```bash
soroban contract invoke \
  --id $TIMELOCK_CONTRACT_ID \
  --source admin \
  --network testnet \
  -- execute \
  --operation_id $OP_ID
```

---

## Step 4: Direct Upgrade (Testnet / Emergency)

For testnet or emergency upgrades, call `upgrade_contract` directly:

```bash
soroban contract invoke \
  --id $UPGRADE_REGISTRY_CONTRACT_ID \
  --source admin \
  --network testnet \
  -- upgrade_contract \
  --new_wasm_hash $NEW_WASM_HASH \
  --contract_name upgrade_registry \
  --new_version 2 \
  --changelog_hash $CHANGELOG_HASH
```

---

## Step 5: Verify the Upgrade

```bash
# Check the latest version in the registry
soroban contract invoke \
  --id $UPGRADE_REGISTRY_CONTRACT_ID \
  --network testnet \
  -- get_latest_version \
  --contract_name upgrade_registry

# Check upgrade history
soroban contract invoke \
  --id $UPGRADE_REGISTRY_CONTRACT_ID \
  --network testnet \
  -- get_upgrade_history \
  --contract_name upgrade_registry
```

---

## Upgrading the Escrow Contract

The escrow contract (`escrow/`) follows the same pattern. After upgrade:

1. Verify existing escrows are still readable:
   ```bash
   soroban contract invoke --id $ESCROW_ID --network testnet -- get_escrow --id 1
   ```

2. Verify new fields default correctly:
   - `dispute_reason` defaults to empty symbol
   - `resolved_at` defaults to `0`
   - `auto_release_delay` uses stored value or 72h default

3. Test new functions on old records:
   ```bash
   # These should work on pre-upgrade escrows
   soroban contract invoke --id $ESCROW_ID --network testnet -- try_auto_release --escrow_id 1
   ```

---

## Storage Migration

For **additive changes** (new fields, new keys): no migration needed. New keys simply don't exist yet and return `None` / defaults.

For **breaking changes** (renamed keys, changed types):

```rust
// In the new contract's initialize_v2() or migrate() function:
pub fn migrate(env: Env) {
    let admin: Address = env.storage().instance().get(&InstanceKey::Admin).unwrap();
    admin.require_auth();

    // Read old key
    let old_value: OldType = env.storage().persistent()
        .get(&OldKey::Something)
        .unwrap_or_default();

    // Write to new key
    env.storage().persistent()
        .set(&NewKey::Something, &NewType::from(old_value));

    // Remove old key
    env.storage().persistent().remove(&OldKey::Something);

    // Bump schema version
    env.storage().instance().set(&InstanceKey::SchemaVersion, &2u32);
}
```

Always increment `InstanceKey::SchemaVersion` on breaking storage changes.

---

## Multi-Sig Upgrade Flow (Production)

When the admin is the multisig contract:

```
1. Signer A: multisig.propose_action(upgrade_registry, "upgrade_contract", [hash, name, ver, changelog])
2. Signer B: multisig.sign_action(proposal_id)
3. Signer C: multisig.sign_action(proposal_id)   ← threshold reached (3-of-5)
4. Anyone:   multisig.execute_action(proposal_id)
             → calls upgrade_registry.upgrade_contract(...)
             → WASM swapped at same address
```

For extra safety, wrap the multisig execution in a timelock:

```
1. Multisig approves → calls timelock.schedule(upgrade_registry, "upgrade_contract", ..., 48h)
2. 48h community review window
3. Anyone calls timelock.execute(op_id)
```

---

## Emergency Cancel

If a scheduled upgrade needs to be blocked:

```bash
# Cancel a timelock operation (admin or proposer only)
soroban contract invoke \
  --id $TIMELOCK_CONTRACT_ID \
  --source admin \
  --network testnet \
  -- cancel \
  --operation_id $OP_ID
```

---

## Rollback

Soroban does not support automatic rollback. To revert to a previous version:

1. Re-upload the old WASM (if not already on-chain).
2. Call `upgrade_contract` with the old WASM hash and a rollback version number.
3. Record the rollback in the upgrade registry with a descriptive changelog hash.

---

## Checklist

Before any production upgrade:

- [ ] New WASM built and optimized
- [ ] New WASM uploaded, hash recorded
- [ ] Changelog hash computed and documented
- [ ] Upgrade tested on testnet
- [ ] Existing data verified readable after upgrade
- [ ] New functions tested on old records
- [ ] Multi-sig proposal created and approved (3-of-5)
- [ ] Timelock scheduled (48h delay)
- [ ] Community notified via events / off-chain channels
- [ ] Timelock executed after delay
- [ ] Post-upgrade verification complete
- [ ] Upgrade recorded in registry

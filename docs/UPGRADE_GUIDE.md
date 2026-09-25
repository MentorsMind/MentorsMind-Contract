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
# Contract Upgrade Guide

## Overview

This guide documents the procedures for upgrading MentorsMind smart contracts on the Stellar Soroban blockchain. All upgrades follow semantic versioning and include comprehensive migration paths.

## Table of Contents

1. [Upgrade Process](#upgrade-process)
2. [Migration Procedures](#migration-procedures)
3. [Rollback Steps](#rollback-steps)
4. [Breaking Changes](#breaking-changes)
5. [Upgrade Checklist](#upgrade-checklist)
6. [Upgrade Scripts](#upgrade-scripts)

---

## Upgrade Process

### Phase 1: Preparation

1. **Review Changes**
   - Examine all code changes in the new version
   - Identify breaking changes using VERSION_POLICY.md
   - Assess impact on existing data and integrations

2. **Build New Version**

   ```bash
   npm run build
   npm run optimize
   ```

3. **Test Locally**

   ```bash
   npm run local:reset
   npm run test
   npm run local:seed
   ```

4. **Verify Compatibility**
   - Run integration tests against old data
   - Test state machine transitions
   - Validate event emissions

### Phase 2: Staging Deployment

1. **Deploy to Testnet**

   ```bash
   soroban contract deploy \
     --wasm contracts/escrow/target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm \
     --network testnet \
     --source-account <admin-account>
   ```

2. **Register Upgrade**
   - Call `upgrade_registry.register_upgrade()` with:
     - Contract name
     - Old version number
     - New version number
     - Changelog hash (SHA-256 of CHANGELOG.md)

3. **Run Staging Tests**
   - Execute full integration test suite
   - Verify all contract functions
   - Test cross-contract interactions

### Phase 3: Production Deployment

1. **Announce Upgrade**
   - Notify all integrators 48 hours in advance
   - Provide migration guide for breaking changes
   - Share upgrade window and expected downtime

2. **Deploy to Mainnet**

   ```bash
   soroban contract deploy \
     --wasm contracts/escrow/target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm \
     --network public \
     --source-account <admin-account>
   ```

3. **Register Upgrade**
   - Call `upgrade_registry.register_upgrade()` on mainnet
   - Record transaction hash for audit trail

4. **Verify Deployment**
   - Call contract functions to confirm deployment
   - Check event emissions
   - Validate state consistency

---

## Migration Procedures

### Data Migration Pattern

For breaking changes that modify storage structure:

1. **Create Migration Contract**

   ```rust
   pub fn migrate_data(env: Env, old_contract: Address) -> Result<(), Error> {
       // 1. Read data from old contract
       let old_data = old_contract.read_state()?;

       // 2. Transform data to new format
       let new_data = transform_data(old_data)?;

       // 3. Write to new storage
       write_new_state(env, new_data)?;

       // 4. Emit migration event
       env.events().publish(
           (symbol_short!("migrate"),),
           MigrationEvent {
               old_version: 1,
               new_version: 2,
               timestamp: env.ledger().timestamp(),
           }
       );

       Ok(())
   }
   ```

2. **Execute Migration**
   - Call migration function with old contract address
   - Verify data integrity post-migration
   - Validate all records transformed correctly

3. **Verify Migration**
   - Compare record counts before/after
   - Spot-check random records
   - Run consistency checks

### Stream Migration Example

For V1 to V2 stream migrations:

```bash
# 1. Identify V1 streams
soroban contract invoke \
  --id <stream-contract-id> \
  --network testnet \
  -- list_streams_v1

# 2. Migrate each stream
soroban contract invoke \
  --id <stream-contract-id> \
  --network testnet \
  -- migrate_stream \
  --sender <account> \
  --v1_id <stream-id>

# 3. Verify migration event
soroban events \
  --network testnet \
  --topic "migrate"
```

### State Synchronization

For upgrades affecting backend integration:

1. **Pause New Operations**
   - Stop accepting new escrows/streams
   - Allow existing operations to complete

2. **Sync Database**
   - Query all on-chain records
   - Update backend database with new state
   - Verify consistency

3. **Resume Operations**
   - Re-enable new operations
   - Monitor for anomalies

---

## Rollback Steps

### Immediate Rollback (Within 1 Hour)

If critical issues are discovered:

1. **Stop New Operations**

   ```bash
   # Call pause function on new contract
   soroban contract invoke \
     --id <new-contract-id> \
     --network public \
     -- pause
   ```

2. **Revert to Previous Version**

   ```bash
   # Update contract reference to old version
   soroban contract deploy \
     --wasm contracts/escrow/target/wasm32-unknown-unknown/release/mentorminds_escrow_v1.wasm \
     --network public \
     --source-account <admin-account>
   ```

3. **Verify Rollback**
   - Confirm old contract is active
   - Test critical functions
   - Monitor event stream

### Delayed Rollback (After 1 Hour)

If issues are discovered after operations have resumed:

1. **Assess Data Integrity**
   - Identify affected records
   - Determine if data can be recovered
   - Calculate potential losses

2. **Execute Rollback**
   - Deploy previous version
   - Run data recovery procedures
   - Notify affected users

3. **Post-Mortem**
   - Document root cause
   - Update testing procedures
   - Implement preventive measures

### Rollback Checklist

- [ ] Pause new operations on new contract
- [ ] Deploy previous version
- [ ] Verify old contract is active
- [ ] Test critical functions
- [ ] Check event stream
- [ ] Notify integrators
- [ ] Document incident
- [ ] Schedule post-mortem

---

## Breaking Changes

### Identifying Breaking Changes

Refer to VERSION_POLICY.md for complete list. Common examples:

**Storage Changes**

- Modifying existing storage keys
- Removing storage keys
- Changing data types
- Reordering storage layout

**Function Interface Changes**

- Changing parameter types or order
- Changing return types
- Removing public functions
- Changing function behavior

**Event Changes**

- Modifying event structure
- Removing events
- Changing event topics

**Business Logic Changes**

- Fee calculation changes
- Authorization changes
- Timing changes
- Token economics changes

### Migration for Breaking Changes

1. **Escrow Status Changes**

   ```rust
   // Old: Active, Released, Disputed, Refunded
   // New: Active, Released, Disputed, Refunded, Resolved

   // Migration: No data change needed, new status only used for new escrows
   ```

2. **Fee Calculation Changes**

   ```rust
   // Old: flat 5% fee
   // New: tiered fee (1-5% based on amount)

   // Migration: Recalculate fees for existing escrows
   let new_fee = calculate_tiered_fee(escrow.amount);
   escrow.platform_fee = new_fee;
   ```

3. **Authorization Changes**

   ```rust
   // Old: Only mentor can release
   // New: Mentor or admin can release

   // Migration: No data change, new logic applies to new operations
   ```

---

## Upgrade Checklist

### Pre-Upgrade

- [ ] All tests passing locally
- [ ] Code review completed
- [ ] Security audit passed (if applicable)
- [ ] CHANGELOG.md updated
- [ ] VERSION_POLICY.md reviewed
- [ ] Migration guide prepared
- [ ] Integrators notified
- [ ] Rollback plan documented
- [ ] Testnet deployment successful
- [ ] Staging tests passed

### During Upgrade

- [ ] Announce upgrade window
- [ ] Deploy to mainnet
- [ ] Register upgrade in upgrade_registry
- [ ] Verify deployment
- [ ] Monitor event stream
- [ ] Check error rates
- [ ] Verify state consistency

### Post-Upgrade

- [ ] All functions working correctly
- [ ] Events emitting properly
- [ ] No error spikes
- [ ] Database synchronized
- [ ] Integrators confirmed success
- [ ] Update documentation
- [ ] Archive old version
- [ ] Schedule post-upgrade review

---

## Upgrade Scripts

### Build and Optimize

```bash
#!/bin/bash
# scripts/upgrade.sh - Complete upgrade workflow

set -e

CONTRACT=${1:-escrow}
NETWORK=${2:-testnet}

echo "🔨 Building $CONTRACT contract..."
cargo build --package $CONTRACT --target wasm32-unknown-unknown --release

echo "⚙️  Optimizing WASM..."
soroban contract optimize \
  --wasm contracts/$CONTRACT/target/wasm32-unknown-unknown/release/mentorminds_$CONTRACT.wasm

echo "📦 Deploying to $NETWORK..."
CONTRACT_ID=$(soroban contract deploy \
  --wasm contracts/$CONTRACT/target/wasm32-unknown-unknown/release/mentorminds_$CONTRACT.wasm \
  --network $NETWORK \
  --source-account default)

echo "✅ Deployed: $CONTRACT_ID"
echo "📝 Register upgrade with:"
echo "   soroban contract invoke --id <upgrade-registry-id> -- register_upgrade"
```

### Verify Upgrade

```bash
#!/bin/bash
# scripts/verify-upgrade.sh - Verify upgrade success

CONTRACT_ID=$1
NETWORK=${2:-testnet}

echo "🔍 Verifying contract deployment..."

# Test basic function
echo "Testing contract functions..."
soroban contract invoke \
  --id $CONTRACT_ID \
  --network $NETWORK \
  -- get_contract_info

# Check events
echo "Checking recent events..."
soroban events \
  --network $NETWORK \
  --contract $CONTRACT_ID \
  --limit 10

echo "✅ Verification complete"
```

### Rollback Script

```bash
#!/bin/bash
# scripts/rollback.sh - Rollback to previous version

CONTRACT=${1:-escrow}
NETWORK=${2:-testnet}
OLD_VERSION=${3:-v1}

echo "⚠️  Rolling back $CONTRACT to $OLD_VERSION..."

# Pause new contract
echo "Pausing new contract..."
soroban contract invoke \
  --id <new-contract-id> \
  --network $NETWORK \
  -- pause

# Deploy old version
echo "Deploying old version..."
soroban contract deploy \
  --wasm contracts/$CONTRACT/target/wasm32-unknown-unknown/release/mentorminds_${CONTRACT}_${OLD_VERSION}.wasm \
  --network $NETWORK \
  --source-account default

echo "✅ Rollback complete"
```

---

## Version History

See CHANGELOG.md for complete version history and upgrade notes.

## Support

For upgrade assistance:

- Review ERRORS.md for error codes
- Check TROUBLESHOOTING.md for common issues
- Contact security@mentorminds.io for security concerns

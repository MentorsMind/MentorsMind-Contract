# Staking Contract - README

## Overview
The Staking Contract allows mentors to stake MNT tokens to achieve reputation tiers and earn revenue share. With the addition of slashing in v1.0.0, the contract now provides economic deterrence against mentor misbehavior.

## Features

### Core Staking
- **Stake MNT** for configurable lock periods
- **Automatic tier assignment** based on amount:
  - Tier 0 (None): < 100 tokens
  - Tier 1 (Bronze): ≥ 100 tokens
  - Tier 2 (Silver): ≥ 500 tokens
  - Tier 3 (Gold): ≥ 2,000 tokens
- **Unstake** after lock period expires
- **Revenue distribution** pro-rata based on stake
- **Rewards claiming** for accumulated revenue share

### Slashing (New in v1.0.0)
- **Economic penalties** for misbehavior (disputes, sanctions, etc.)
- **Dual authorization** via MultisigAdmin OR Governance
- **Maximum 50% slash per event** prevents total loss
- **Automatic tier recalculation** after slash
- **Immutable audit trail** for transparency
- **Insurance pool funding** from slashed tokens

## Quick Start

### Deploy
```bash
stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/staking.wasm \
  --source deployer \
  --network testnet
```

### Initialize
```rust
staking.initialize(admin_address, mnt_token_address);
```

### Configure (Required for Slashing)
```rust
staking.set_insurance_pool(admin, insurance_pool_address);
staking.set_multisig_admin(admin, multisig_admin_address);
staking.set_governance(admin, governance_address);
```

### Stake
```rust
staking.stake(
    mentor_address,
    1000,  // 1000 tokens
    30     // 30 day lock
);
```

### Slash (via Multisig)
```rust
// After multisig proposal executed
staking.slash(
    multisig_caller,
    mentor_address,
    1000u32,  // 10% = 1000 bps
    Symbol::new("dispute"),
    Some(multisig_proposal_id),
    None
);
```

## Documentation

### For Developers
- **[SLASHING_QUICK_REFERENCE.md](./SLASHING_QUICK_REFERENCE.md)** - Function signatures, examples, troubleshooting

### For Architects
- **[SLASHING_IMPLEMENTATION.md](../../SLASHING_IMPLEMENTATION.md)** - Complete technical specification
- **[SLASHING_SUMMARY.md](../../SLASHING_SUMMARY.md)** - Implementation summary and status
- **[SLASHING_CHANGELOG.md](../../SLASHING_CHANGELOG.md)** - Detailed change log

## API Reference

### Staking Functions
```rust
pub fn initialize(env: Env, admin: Address, mnt_token: Address) -> Result<(), Error>
pub fn stake(env: Env, mentor: Address, amount: i128, lock_period_days: u32) -> Result<(), Error>
pub fn unstake(env: Env, mentor: Address) -> Result<(), Error>
pub fn claim_rewards(env: Env, staker: Address, token: Address) -> Result<(), Error>
```

### Query Functions
```rust
pub fn get_stake(env: Env, mentor: Address) -> Result<StakeRecord, Error>
pub fn get_tier(env: Env, mentor: Address) -> u32
pub fn get_staker_count(env: Env) -> u32
pub fn get_stakers(env: Env) -> Vec<Address>
pub fn get_total_staked(env: Env) -> i128
pub fn get_pending_rewards(env: Env, staker: Address) -> i128
pub fn get_slash_history(env: Env, mentor: Address) -> Vec<SlashRecord>
```

### Admin Functions
```rust
pub fn set_insurance_pool(env: Env, admin: Address, insurance_pool: Address) -> Result<(), Error>
pub fn set_multisig_admin(env: Env, admin: Address, multisig_admin: Address) -> Result<(), Error>
pub fn set_governance(env: Env, admin: Address, governance: Address) -> Result<(), Error>
```

### Slashing Functions
```rust
pub fn slash(
    env: Env,
    caller: Address,
    mentor: Address,
    slash_bps: u32,
    slash_reason: Symbol,
    multisig_proposal_id: Option<u32>,
    governance_proposal_id: Option<u32>
) -> Result<(), Error>
```

### Batch Operations
```rust
pub fn distribute_revenue_batch(env: Env, token: Address, amount: i128, offset: u32, limit: u32)
pub fn migrate_stakers(env: Env)
```

## Data Types

### StakeRecord
```rust
struct StakeRecord {
    mentor: Address,
    amount: i128,
    staked_at: u64,
    unlock_at: u64,
    unlock_cooldown_until: Option<u64>,
    tier: u32,
}
```

### SlashRecord
```rust
struct SlashRecord {
    amount: i128,
    slash_bps: u32,
    reason: Symbol,
    timestamp: u64,
    governance_proposal_id: Option<u32>,
}
```

## Error Codes
```rust
AlreadyInitialized = 1       // Contract already initialized
NotInitialized = 2           // Contract not initialized
InvalidAmount = 3            // Invalid token amount
AlreadyStaked = 4            // Mentor already has stake
NoStakeFound = 5             // No stake record found
StillLocked = 6              // Stake still in lock period
Unauthorized = 7             // Caller not authorized
SlashExceedsMax = 8          // Slash > 50% (5000 bps)
InvalidSlashBps = 9          // Invalid basis points
NoMultisigApproval = 10      // No approval from multisig/governance
InsuranceTransferFailed = 11 // Insurance pool not configured
```

## Events

### Staked
```rust
topics: ("staking", 1, "staked")
data: StakedEventData {
    mentor: Address,
    amount: i128,
    unlock_at: u64,
    unlock_cooldown_until: Option<u64>,
    tier: u32,
}
```

### Unstaked
```rust
topics: ("staking", 1, "unstaked")
data: UnstakedEventData {
    mentor: Address,
    amount: i128,
}
```

### Slashed
```rust
topics: ("staking", 1, "slashed")
data: SlashedEventData {
    mentor: Address,
    slash_amount: i128,
    slash_bps: u32,
    reason: Symbol,
    new_amount: i128,
    new_tier: u32,
    governance_proposal_id: Option<u32>,
}
```

## Testing

### Run Tests
```bash
# All tests
cargo test --package staking

# Specific test
cargo test --package staking test_slash_removes_10_percent

# With output
cargo test --package staking -- --nocapture
```

### Test Coverage
- ✅ Staking operations (stake, unstake)
- ✅ Tier assignment and calculation
- ✅ Revenue distribution
- ✅ Slashing (basic, max, tier recalc)
- ✅ Authorization enforcement
- ✅ History recording
- ✅ Governance integration

## Security

### Protection Mechanisms
- **Reentrancy guards** on state-changing functions
- **Authorization checks** on admin functions
- **Checked arithmetic** prevents overflow/underflow
- **Maximum slash cap** prevents catastrophic loss
- **Dual authorization** prevents unilateral slashing
- **Immutable history** ensures transparency

### Security Considerations
- Slashing requires external approval (multisig or governance)
- Insurance pool must be configured before slashing
- Maximum 50% slash per event
- Tier automatically adjusts after slash
- History is append-only and immutable

## Integration

### Required Contracts
1. **MNT Token** - Stellar Asset Contract for MNT
2. **Insurance Pool** - Receives slashed tokens
3. **MultisigAdmin** - Multisig approval for slashing
4. **Governance** - DAO governance for slashing

### Integration Flow
```
1. Deploy Staking Contract
2. Initialize with admin and MNT token
3. Configure insurance pool address
4. Configure multisig admin address
5. Configure governance address
6. Add slash function to governance allowlist
7. Test slash on testnet
8. Deploy to mainnet
```

## Examples

### Example 1: Mentor Stakes for Gold Tier
```rust
// Mentor stakes 2000 tokens for 90 days
staking.stake(mentor, 2_000, 90);

// Check tier
let tier = staking.get_tier(mentor);
assert_eq!(tier, 3); // Gold

// After 90 days, unstake
staking.unstake(mentor);
```

### Example 2: Slash for Dispute Loss
```rust
// Multisig approves slash proposal
let proposal_id = multisig.propose_action(
    proposer,
    staking_contract,
    Symbol::new("slash"),
    vec![mentor, 2000u32, Symbol::new("dispute")]
);

multisig.sign_action(signer1, proposal_id);
multisig.sign_action(signer2, proposal_id);

// Execute slash (20%)
staking.slash(
    multisig,
    mentor,
    2_000u32,
    Symbol::new("dispute"),
    Some(proposal_id),
    None
);

// Check updated stake
let record = staking.get_stake(mentor);
// amount reduced by 20%
// tier may have changed
```

### Example 3: Query Slash History
```rust
let history = staking.get_slash_history(mentor);

for record in history.iter() {
    println!("Slashed {} tokens ({} bps) on {} for {}",
        record.amount,
        record.slash_bps,
        record.timestamp,
        record.reason
    );
}
```

## Deployment Checklist

### Pre-Deployment
- [ ] Compile contract: `stellar contract build`
- [ ] Run tests: `cargo test --package staking`
- [ ] Deploy to testnet
- [ ] Test all functions on testnet
- [ ] Security audit completed

### Deployment
- [ ] Deploy to mainnet
- [ ] Call `initialize(admin, mnt_token)`
- [ ] Call `set_insurance_pool(admin, insurance)`
- [ ] Call `set_multisig_admin(admin, multisig)`
- [ ] Call `set_governance(admin, governance)`
- [ ] Add slash to governance allowlist
- [ ] Verify configuration

### Post-Deployment
- [ ] Monitor first stakes
- [ ] Verify tier calculations
- [ ] Test slash flow (small amount)
- [ ] Verify insurance pool integration
- [ ] Monitor events
- [ ] Update frontend/SDK

## Troubleshooting

### Common Issues

**Q: Error: NoMultisigApproval**
- A: Execute multisig proposal or pass governance vote first

**Q: Error: SlashExceedsMax**
- A: Reduce slash_bps to 5000 or less (50% max)

**Q: Error: InsuranceTransferFailed**
- A: Call `set_insurance_pool()` to configure insurance address

**Q: Error: StillLocked**
- A: Wait for lock period to expire before unstaking

**Q: Tier not updating after slash**
- A: Tier updates automatically - check `get_stake()` result

## Support

### Resources
- Soroban Docs: https://soroban.stellar.org/docs
- Stellar Discord: https://discord.gg/stellar
- GitHub Issues: [Repository URL]

### Contact
- Technical Questions: [Discord Channel]
- Security Issues: security@mentorsmind.io
- General Support: support@mentorsmind.io

## License
[Your License Here]

## Version
v1.0.0-slashing (2026-07-24)

---

**Status:** ✅ Production Ready (Pending Security Audit)

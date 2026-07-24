# Slashing Mechanism - Quick Reference

## Function Signatures

### Configuration (Admin Only)
```rust
pub fn set_insurance_pool(env: Env, admin: Address, insurance_pool: Address) -> Result<(), Error>
pub fn set_multisig_admin(env: Env, admin: Address, multisig_admin: Address) -> Result<(), Error>
pub fn set_governance(env: Env, admin: Address, governance: Address) -> Result<(), Error>
```

### Slashing
```rust
pub fn slash(
    env: Env,
    caller: Address,                    // Multisig or governance contract
    mentor: Address,                    // Mentor to slash
    slash_bps: u32,                     // Basis points (1-5000)
    slash_reason: Symbol,               // Reason code
    multisig_proposal_id: Option<u32>,  // Multisig approval (if any)
    governance_proposal_id: Option<u32> // Governance approval (if any)
) -> Result<(), Error>
```

### Query
```rust
pub fn get_slash_history(env: Env, mentor: Address) -> Vec<SlashRecord>
```

## Data Types

### SlashRecord
```rust
struct SlashRecord {
    amount: i128,                      // Tokens slashed
    slash_bps: u32,                    // Basis points applied
    reason: Symbol,                    // Reason code
    timestamp: u64,                    // Block timestamp
    governance_proposal_id: Option<u32> // Gov proposal (if any)
}
```

## Error Codes
```rust
Unauthorized = 7               // Not admin
SlashExceedsMax = 8           // > 50% (5000 bps)
InvalidSlashBps = 9           // bps = 0 or > 10000
NoMultisigApproval = 10       // No multisig/gov approval
InsuranceTransferFailed = 11  // Insurance pool not set
```

## Common Slash Reasons
```rust
Symbol::new("dispute")     // Lost dispute
Symbol::new("sanction")    // Sanctions violation
Symbol::new("failed")      // Failed session
Symbol::new("fraud")       // Fraudulent activity
Symbol::new("abuse")       // Platform abuse
```

## Basis Points Reference
```
   100 bps =  1%
   500 bps =  5%
 1,000 bps = 10%
 2,000 bps = 20%
 5,000 bps = 50% (MAX)
```

## Tier Thresholds
```
Tier 0 (None):    < 100 tokens
Tier 1 (Bronze):  ≥ 100 tokens
Tier 2 (Silver):  ≥ 500 tokens
Tier 3 (Gold):    ≥ 2,000 tokens
```

## Usage Examples

### Example 1: Slash 10% via Multisig
```rust
// 1. Create multisig proposal
let proposal_id = multisig.propose_action(
    proposer,
    staking_contract,
    Symbol::new("slash"),
    vec![mentor, 1000u32, Symbol::new("dispute")]
);

// 2. Signers approve
multisig.sign_action(signer1, proposal_id);
multisig.sign_action(signer2, proposal_id); // Reaches threshold

// 3. Execute slash
staking.slash(
    multisig,
    mentor,
    1000u32,
    Symbol::new("dispute"),
    Some(proposal_id),
    None
);
```

### Example 2: Slash 25% via Governance
```rust
// 1. Create proposal
let prop_id = governance.create_proposal(
    proposer,
    "Slash mentor for failed sessions",
    hash,
    ProposalAction::ExecuteCall(staking, Symbol::new("slash"))
);

// 2. Vote & finalize
governance.cast_vote(voter, prop_id, true);
// ... after voting period ...
governance.finalize_proposal(prop_id);

// 3. Execute slash
staking.slash(
    governance,
    mentor,
    2500u32,
    Symbol::new("failed"),
    None,
    Some(prop_id)
);
```

### Example 3: Query History
```rust
let history = staking.get_slash_history(mentor);
for record in history {
    log!("Slashed {} ({} bps) on {} for {}",
        record.amount,
        record.slash_bps,
        record.timestamp,
        record.reason
    );
}
```

## Authorization Matrix

| Caller | Multisig ID | Gov ID | Result |
|--------|-------------|--------|--------|
| Any | None | None | ❌ NoMultisigApproval |
| Any | Some(valid) | None | ✅ Executes |
| Any | None | Some(valid) | ✅ Executes |
| Any | Some(valid) | Some(valid) | ✅ Executes |
| Any | Some(invalid) | None | ❌ NoMultisigApproval |

## Slash Amount Calculation

```rust
// Formula
slash_amount = (current_stake * slash_bps) / 10_000

// Example: 10% slash on 1,000 tokens
slash_amount = (1_000 * 1_000) / 10_000 = 100 tokens

// Result
new_amount = 1_000 - 100 = 900 tokens
```

## Tier Recalculation

```rust
fn compute_tier(amount: i128) -> u32 {
    if amount >= 2_000 { 3 }      // Gold
    else if amount >= 500 { 2 }   // Silver
    else if amount >= 100 { 1 }   // Bronze
    else { 0 }                    // None
}
```

**Example:** 
- Start: 1,000 tokens → Tier 2 (Silver)
- Slash 60%: 400 tokens → Tier 1 (Bronze)

## Event Structure

```rust
Event Topics:
  [0] = Symbol("staking")
  [1] = u32(1)  // Schema version
  [2] = Symbol("slashed")

Event Data:
  SlashedEventData {
    mentor: Address,
    slash_amount: i128,
    slash_bps: u32,
    reason: Symbol,
    new_amount: i128,
    new_tier: u32,
    governance_proposal_id: Option<u32>
  }
```

## Testing Commands

```bash
# Run all staking tests
cargo test --package staking

# Run specific slash tests
cargo test --package staking test_slash

# Run with output
cargo test --package staking -- --nocapture
```

## Deployment Checklist

- [ ] Deploy insurance pool contract
- [ ] Deploy multisig admin contract  
- [ ] Deploy governance contract
- [ ] Call `initialize(admin, mnt_token)`
- [ ] Call `set_insurance_pool(admin, insurance_addr)`
- [ ] Call `set_multisig_admin(admin, multisig_addr)`
- [ ] Call `set_governance(admin, gov_addr)`
- [ ] Add slash to governance allowlist
- [ ] Test slash on testnet

## Troubleshooting

### Error: NoMultisigApproval
**Cause:** No valid multisig or governance approval provided
**Solution:** Execute multisig proposal or pass governance vote first

### Error: SlashExceedsMax
**Cause:** slash_bps > 5000 (50%)
**Solution:** Reduce slash_bps to 5000 or less

### Error: InvalidSlashBps  
**Cause:** slash_bps = 0 or > 10000
**Solution:** Use valid range: 1-5000

### Error: InsuranceTransferFailed
**Cause:** Insurance pool address not configured
**Solution:** Call `set_insurance_pool()` first

### Error: NoStakeFound
**Cause:** Mentor has no active stake
**Solution:** Verify mentor address and stake status

## Security Notes

⚠️ **Important:**
- Slash requires reentrancy guard - don't disable
- Maximum 50% per slash prevents total loss
- Authorization required - no direct admin slash
- History is immutable - cannot delete/modify records
- Tier recalculation is automatic - don't override

## Links

- Full Documentation: `SLASHING_IMPLEMENTATION.md`
- Summary: `SLASHING_SUMMARY.md`
- Staking Contract: `contracts/staking/src/lib.rs`
- Shared Types: `contracts/shared/src/lib.rs`

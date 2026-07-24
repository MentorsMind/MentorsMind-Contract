# Staking Slashing Mechanism Implementation

## Overview
This document describes the implementation of the slashing mechanism for the MentorsMind StakingContract, addressing the security vulnerability where staked funds could not be penalized for misbehavior.

## Problem Statement
The original StakingContract allowed mentors to stake MNT tokens for reputation tiers, but lacked any mechanism to slash (penalize) staked funds when mentors misbehaved (failed sessions, dispute losses, sanctions violations). Without slashing, the stake provided no real economic deterrent.

## Solution Overview
Implemented a comprehensive slashing mechanism with the following features:

### 1. **Authorization-Gated Slashing**
- Requires either **MultisigAdmin approval** OR **Governance proposal approval**
- Single admin calls without multisig are rejected
- Prevents unauthorized or malicious slashing

### 2. **Configurable Slashing Parameters**
- `slash_bps`: Basis points to slash (1 bps = 0.01%)
- Maximum 50% per slash event (5000 bps cap)
- `slash_reason`: Symbol describing the reason (e.g., "dispute", "sanction")

### 3. **Automatic Tier Recalculation**
- Tier is recalculated after each slash based on remaining amount
- If amount drops below tier threshold, tier is automatically downgraded
- Ensures tier always reflects current economic stake

### 4. **Insurance Pool Integration**
- Slashed tokens are transferred to the insurance pool via cross-contract call
- Provides funding for insurance claims and platform security
- Configurable insurance pool address

### 5. **Immutable Audit Trail**
- Every slash is recorded in `DataKey::SlashHistory(Address)`
- SlashRecord includes: amount, bps, reason, timestamp, governance_proposal_id
- History is queryable but immutable (append-only)

## Implementation Details

### New Data Types

#### SlashRecord
```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashRecord {
    pub amount: i128,           // Actual amount slashed
    pub slash_bps: u32,         // Basis points applied
    pub reason: Symbol,         // Reason code
    pub timestamp: u64,         // When slash occurred
    pub governance_proposal_id: Option<u32>, // Optional governance link
}
```

#### SlashedEventData
```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashedEventData {
    pub mentor: Address,
    pub slash_amount: i128,
    pub slash_bps: u32,
    pub reason: Symbol,
    pub new_amount: i128,
    pub new_tier: u32,
    pub governance_proposal_id: Option<u32>,
}
```

### New Storage Keys
```rust
SlashHistory(Address),     // Vec<SlashRecord> per mentor
InsurancePool,             // Insurance pool contract address
MultisigAdmin,             // Multisig admin contract address
Governance,                // Governance contract address
```

### New Error Codes
```rust
Unauthorized = 7,              // Caller not authorized
SlashExceedsMax = 8,           // Slash > 50% (5000 bps)
InvalidSlashBps = 9,           // Slash bps = 0 or > 10000
NoMultisigApproval = 10,       // Neither multisig nor governance approved
InsuranceTransferFailed = 11,  // Insurance pool not configured
```

## Core Functions

### 1. Configuration Functions (Admin Only)

#### set_insurance_pool
```rust
pub fn set_insurance_pool(env: Env, admin: Address, insurance_pool: Address) -> Result<(), Error>
```
- Sets the insurance pool contract address
- Admin-only authorization required
- Must be called before slashing can occur

#### set_multisig_admin
```rust
pub fn set_multisig_admin(env: Env, admin: Address, multisig_admin: Address) -> Result<(), Error>
```
- Sets the multisig admin contract address
- Required for multisig-based slashing authorization

#### set_governance
```rust
pub fn set_governance(env: Env, admin: Address, governance: Address) -> Result<(), Error>
```
- Sets the governance contract address
- Required for governance-based slashing authorization

### 2. Slashing Function

#### slash
```rust
pub fn slash(
    env: Env,
    caller: Address,
    mentor: Address,
    slash_bps: u32,
    slash_reason: Symbol,
    multisig_proposal_id: Option<u32>,
    governance_proposal_id: Option<u32>,
) -> Result<(), Error>
```

**Authorization Flow:**
1. Validates `slash_bps` (0 < bps ≤ 5000)
2. Checks multisig approval (if proposal ID provided)
3. Checks governance approval (if proposal ID provided)
4. Requires at least one approval mechanism
5. Requires caller authentication

**Execution Flow:**
1. Retrieves mentor's stake record
2. Calculates slash amount: `(amount * slash_bps) / 10,000`
3. Calculates new amount: `amount - slash_amount`
4. Recalculates tier based on new amount
5. Updates stake record with new amount and tier
6. Updates total staked amount
7. Transfers slashed tokens to insurance pool
8. Records slash in immutable history
9. Emits `slashed` event

**Safety Mechanisms:**
- Reentrancy guard prevents recursive calls
- Checked arithmetic prevents overflow/underflow
- Maximum 50% slash per event prevents catastrophic loss
- Audit trail provides transparency and accountability

### 3. Query Functions

#### get_slash_history
```rust
pub fn get_slash_history(env: Env, mentor: Address) -> Vec<SlashRecord>
```
- Returns all slash events for a mentor
- Immutable audit trail
- Empty vector if no slashes recorded

## Authorization Verification

### verify_multisig_approval
```rust
fn verify_multisig_approval(env: &Env, proposal_id: u32, caller: &Address) -> Result<bool, Error>
```
- Cross-contract call to MultisigAdmin contract
- Calls `is_executed(proposal_id)` function
- Returns `true` if proposal exists and is executed

### verify_governance_approval
```rust
fn verify_governance_approval(env: &Env, proposal_id: u32) -> Result<bool, Error>
```
- Cross-contract call to Governance contract
- Calls `get_proposal_status(proposal_id)` function
- Returns `true` if proposal is in Executed or Passed status

## Test Coverage

### Basic Functionality Tests
1. ✅ `test_slash_removes_10_percent` - Basic 10% slash operation
2. ✅ `test_slash_beyond_50_percent_rejected` - Validates 50% cap
3. ✅ `test_slash_recalculates_tier` - Verifies automatic tier downgrade
4. ✅ `test_slash_history_queryable` - Audit trail functionality
5. ✅ `test_slash_without_multisig_rejected` - Authorization enforcement
6. ✅ `test_slash_with_governance_approval` - Governance path testing

### Test Scenarios

#### Scenario 1: 10% Slash with Tier Downgrade
- Initial: 1000 tokens (Gold tier)
- Slash: 10% (1000 bps)
- Result: 900 tokens (Silver tier)
- Insurance: +100 tokens

#### Scenario 2: 50% Max Slash
- Initial: 600 tokens (Silver tier)
- Slash: 50% (5000 bps)
- Result: 300 tokens (Bronze tier)
- Validates maximum slash enforcement

#### Scenario 3: Authorization Rejection
- Attempt slash without multisig/governance approval
- Result: `Error::NoMultisigApproval`

## Integration Points

### 1. Insurance Pool Contract
- **Function Called:** `transfer()` via token client
- **Purpose:** Receive slashed MNT tokens
- **Configuration:** Set via `set_insurance_pool()`

### 2. MultisigAdmin Contract
- **Function Called:** `is_executed(proposal_id)`
- **Purpose:** Verify multisig approval
- **Configuration:** Set via `set_multisig_admin()`

### 3. Governance Contract
- **Function Called:** `get_proposal_status(proposal_id)`
- **Purpose:** Verify governance proposal passed
- **Configuration:** Set via `set_governance()`

### 4. Token Contract (MNT)
- **Function Called:** `transfer(from, to, amount)`
- **Purpose:** Transfer slashed tokens to insurance pool
- **Configuration:** Set at initialization

## Event Emission

### Slashed Event
```rust
(
    Symbol::new("staking"),
    1u32,  // Schema version
    Symbol::new("slashed"),
)
```

**Event Data:**
- `mentor`: Address of slashed mentor
- `slash_amount`: Actual tokens slashed
- `slash_bps`: Basis points applied
- `reason`: Reason symbol
- `new_amount`: Remaining stake
- `new_tier`: New tier after slash
- `governance_proposal_id`: Optional governance link

## Usage Examples

### Example 1: Slash via Multisig for Dispute Loss
```rust
// 1. Admin proposes slash action to MultisigAdmin
multisig_admin.propose_action(
    admin,
    staking_contract,
    Symbol::new("slash"),
    vec![mentor, 2000u32, Symbol::new("dispute_loss")]
);

// 2. Signers approve (reaching threshold)
multisig_admin.sign_action(signer1, proposal_id);
multisig_admin.sign_action(signer2, proposal_id);

// 3. Execute multisig proposal
multisig_admin.execute_action(proposal_id);

// 4. Call slash with multisig approval
staking_contract.slash(
    multisig_admin,
    mentor,
    2000u32,  // 20%
    Symbol::new("dispute_loss"),
    Some(proposal_id),
    None
);
```

### Example 2: Slash via Governance for Sanctions Violation
```rust
// 1. Create governance proposal
let proposal_id = governance.create_proposal(
    proposer,
    "Slash mentor for sanctions violation",
    description_hash,
    ProposalAction::ExecuteCall(staking_contract, Symbol::new("slash"))
);

// 2. Community votes
governance.cast_vote(voter1, proposal_id, true);
governance.cast_vote(voter2, proposal_id, true);
// ... more votes ...

// 3. Finalize proposal (after voting period)
governance.finalize_proposal(proposal_id);  // Sets to Passed/Executed

// 4. Execute slash with governance approval
staking_contract.slash(
    governance,
    mentor,
    5000u32,  // 50% max
    Symbol::new("sanctions"),
    None,
    Some(proposal_id)
);
```

## Security Considerations

### 1. Reentrancy Protection
- ReentrancyGuard used on `slash()` function
- Prevents recursive slashing attacks

### 2. Authorization Enforcement
- Requires multisig OR governance approval
- Single admin cannot unilaterally slash
- Prevents centralization of slashing power

### 3. Slashing Limits
- Maximum 50% per event prevents total loss
- Mentor retains economic stake and participation
- Can still unstake remaining amount after lock period

### 4. Audit Trail
- Every slash recorded with timestamp and reason
- Governance proposal ID links to on-chain vote
- Immutable history prevents tampering

### 5. Tier Integrity
- Automatic tier recalculation after slash
- Tier always reflects current economic stake
- Prevents tier/stake mismatch

## Future Enhancements

### Potential Improvements
1. **Progressive Slashing Caps**
   - Cumulative slash tracking
   - Total lifetime slash limit (e.g., 80%)

2. **Slash Appeal Mechanism**
   - Time window for appeals
   - Independent arbitrator review
   - Potential slash reversal

3. **Automated Slashing Triggers**
   - Integration with dispute resolution
   - Automatic slash on dispute loss
   - Smart contract-driven penalties

4. **Graduated Slashing**
   - Different caps by violation severity
   - Minor: 10% max, Major: 50% max
   - Critical: Full stake slash

5. **Slash Recovery Pool**
   - Slashed mentors can earn back reputation
   - Good behavior rewards
   - Stake restoration mechanism

## Deployment Checklist

### Pre-Deployment
- [ ] Deploy Insurance Pool contract
- [ ] Deploy MultisigAdmin contract
- [ ] Deploy Governance contract
- [ ] Configure MultisigAdmin signers and threshold

### Post-Deployment
- [ ] Call `set_insurance_pool()` with insurance contract address
- [ ] Call `set_multisig_admin()` with multisig contract address
- [ ] Call `set_governance()` with governance contract address
- [ ] Add slash function to governance allowlist
- [ ] Test slash with small amount on testnet

### Verification
- [ ] Verify multisig approval flow works
- [ ] Verify governance approval flow works
- [ ] Verify insurance pool receives slashed tokens
- [ ] Verify tier recalculation works correctly
- [ ] Verify slash history is recorded
- [ ] Verify unauthorized slash attempts fail

## Migration from Existing Staking

### For Existing Deployments
1. **Deploy New Contract Version**
   - Includes slashing functionality
   - Maintains backward compatibility with existing stakes

2. **Migrate Configuration**
   - Copy admin address
   - Copy MNT token address
   - Set new contract addresses (insurance, multisig, governance)

3. **Migrate Stake Data**
   - Use existing `migrate_stakers()` function
   - All existing stakes remain valid
   - No tier recalculation needed (unless slashed)

4. **Update Frontend/SDK**
   - Add slash history display
   - Show slashing risk warnings
   - Display tier downgrade scenarios

## Conclusion

This implementation provides a robust, secure, and transparent slashing mechanism for the MentorsMind StakingContract. Key achievements:

✅ **Economic Deterrent:** Mentors now face real financial risk for misbehavior
✅ **Decentralized Governance:** Requires multisig or community approval
✅ **Automatic Tier Adjustment:** Tier always reflects current stake
✅ **Transparent Audit Trail:** Immutable record of all slashes
✅ **Insurance Pool Funding:** Slashed funds support platform security
✅ **Learner Protection:** On-chain assurance of mentor accountability

The slashing mechanism transforms the staking system from a pure reputation signal into a credible economic commitment backed by real penalties for misconduct.

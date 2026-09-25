# Multisig Admin Contract Formal Specifications

## Overview

The Multisig Admin contract implements M-of-N threshold signatures for administrative operations. It allows a group of signers to collectively approve and execute actions, preventing single points of failure and enabling decentralized governance.

---

## Contract Metadata

- **Contract**: `MultisigAdminContract`
- **Location**: `contracts/multisig_admin/src/lib.rs`
- **Primary State**: `ProposalRecord`
- **Key Pattern**: M-of-N threshold (e.g., 3-of-5 multisig)

---

## State Types

### Proposal Record
```rust
pub struct ProposalRecord {
    pub id: u32,
    pub proposer: Address,
    pub target: Address,        // Contract to invoke
    pub function: Symbol,       // Function name to call
    pub args: Vec<Val>,         // Function arguments
    pub approval_count: u32,    // Current approvals
    pub expiry: u64,            // Expiry timestamp
    pub executed: bool,         // Execution flag
    pub cancelled: bool,        // Cancellation flag
}
```

### Storage Keys
```rust
pub enum DataKey {
    Threshold,                  // Required approval count
    SignerCount,                // Total number of signers
    ProposalCount,              // Proposal counter
    Signer(Address),            // Signer membership: Address → bool
    Proposal(u32),              // Proposal by ID
    Approval(u32, Address),     // Approval record: (proposal_id, signer) → bool
}
```

---

## Safety Properties

### SP1: Threshold Validity
**Property**: The approval threshold is always between 1 and signer_count (inclusive).

**Formal**:
```
∀ states: 1 ≤ threshold ≤ signer_count
```

**Invariant Enforcement Points**:
- `initialize`: `threshold ≤ signers.len()`
- `add_signer`: `signer_count + 1 ≥ threshold`
- `remove_signer`: `signer_count - 1 ≥ threshold`
- `update_threshold`: `new_threshold ≤ signer_count ∧ new_threshold ≥ 1`

**Verification Strategy**:
- Boundary tests: threshold = 1, threshold = signer_count
- Invalid cases: threshold = 0, threshold > signer_count
- Mutation testing: add/remove signers and verify threshold remains valid

### SP2: Approval Uniqueness
**Property**: Each signer can approve a proposal at most once.

**Formal**:
```
∀ proposal p, signer s:
  storage.get(Approval(p.id, s)) ∈ {None, Some(true)}
```

**Implementation**:
```rust
// In sign_action:
if env.storage().persistent().get(&DataKey::Approval(action_id, signer.clone())).unwrap_or(false) {
    return Err(Error::AlreadySigned);
}
```

**Verification Strategy**:
- Double-approval test: signer calls `sign_action` twice
- Concurrent approval test: simulate parallel signatures
- Storage key uniqueness proof

### SP3: Execution Guard
**Property**: A proposal executes only when all preconditions are met.

**Formal**:
```
execute(p) succeeds ⟹
  (p.approval_count ≥ threshold) ∧
  (now ≤ p.expiry) ∧
  (¬p.executed) ∧
  (¬p.cancelled)
```

**Implementation Checks**:
```rust
if proposal.executed  { return Err(Error::AlreadyExecuted); }
if proposal.cancelled { return Err(Error::Cancelled); }
if env.ledger().timestamp() > proposal.expiry { return Err(Error::Expired); }
if proposal.approval_count < threshold { return Err(Error::BelowThreshold); }
```

**Verification Strategy**:
- Exhaustive precondition testing (all combinations of flags)
- Boundary test: approval_count = threshold - 1, = threshold, = threshold + 1
- Expiry boundary: now = expiry - 1, = expiry, = expiry + 1

### SP4: Single Execution
**Property**: Each proposal executes at most once.

**Formal**:
```
∀ proposal p:
  execute(p) succeeds ⟹ p.executed = true
  p.executed = true ⟹ ∀ future calls: execute(p) fails
```

**Implementation**:
```rust
// Mark executed BEFORE dispatch (prevents reentrancy)
proposal.executed = true;
env.storage().persistent().set(&DataKey::Proposal(action_id), &proposal);

// Then dispatch
env.invoke_contract::<()>(&proposal.target, &proposal.function, proposal.args.clone());
```

**Verification Strategy**:
- Reentrancy test: malicious target contract calls back into multisig
- Double-execution test: attempt to execute same proposal twice
- State persistence verification

### SP5: Signer Authorization
**Property**: Only registered signers can propose and approve.

**Formal**:
```
propose_action(proposer, ...) succeeds ⟹ is_signer(proposer)
sign_action(signer, ...) succeeds ⟹ is_signer(signer)
```

**Verification Strategy**:
- Unauthorized proposer test
- Unauthorized signer test
- Removed signer test (signer removed after proposal creation)

---

## Liveness Properties

### LP1: Proposal Progression
**Property**: If a proposal receives sufficient approvals before expiry, it can be executed.

**Formal**:
```
(p.approval_count ≥ threshold) ∧
(now ≤ p.expiry) ∧
(¬p.executed) ∧
(¬p.cancelled)
⟹ can_execute(p)
```

### LP2: Proposer Auto-Approval
**Property**: When a proposal is created, the proposer counts as the first approval.

**Formal**:
```
propose_action(proposer, ...) ⟹
  proposal.approval_count = 1 ∧
  storage.get(Approval(proposal.id, proposer)) = true
```

**Rationale**: Reduces steps for 1-of-N multisigs and prevents proposer from having to explicitly approve their own proposal.

### LP3: Cancellation Availability
**Property**: Non-executed, non-expired, non-cancelled proposals can be cancelled by proposer or any signer.

**Formal**:
```
(¬p.executed) ∧ (¬p.cancelled) ∧ (now ≤ p.expiry) ⟹
  (caller = p.proposer ∨ is_signer(caller)) ⟹ can_cancel(p)
```

---

## Functional Correctness Properties

### FC1: Self-Targeted Operations
**Property**: Proposals targeting the multisig contract itself use internal helpers for signer management and threshold updates.

**Operations**:
- `add_signer(Address)` → `apply_add_signer`
- `remove_signer(Address)` → `apply_remove_signer`
- `update_threshold(u32)` → `apply_update_threshold`

**Formal**:
```
execute(p) where p.target = current_contract ⟹
  use_internal_helper(p.function)
```

**Rationale**: Prevents external invoke loops and ensures consistency checks.

**Verification Strategy**:
- Test add_signer via proposal + execution
- Test remove_signer via proposal + execution
- Test update_threshold via proposal + execution
- Verify threshold invariant maintained after each operation

### FC2: External Contract Invocation
**Property**: Proposals targeting external contracts invoke them directly.

**Formal**:
```
execute(p) where p.target ≠ current_contract ⟹
  env.invoke_contract(&p.target, &p.function, p.args)
```

**Verification Strategy**:
- Mock external contract and verify invocation occurs
- Test with various argument types (Address, Symbol, Vec, etc.)

### FC3: Signer Addition Safety
**Property**: Adding a signer increments signer_count and sets membership flag.

**Formal**:
```
apply_add_signer(addr) succeeds ⟹
  is_signer(addr) = true ∧
  signer_count_after = signer_count_before + 1
```

**Preconditions**:
- `¬is_signer(addr)` (cannot add duplicate)

### FC4: Signer Removal Safety
**Property**: Removing a signer decrements signer_count while maintaining threshold validity.

**Formal**:
```
apply_remove_signer(addr) succeeds ⟹
  is_signer(addr) = false ∧
  signer_count_after = signer_count_before - 1 ∧
  signer_count_after ≥ threshold
```

**Preconditions**:
- `is_signer(addr)` (cannot remove non-signer)
- `signer_count - 1 ≥ threshold` (maintains threshold validity)

### FC5: Threshold Update Safety
**Property**: Updating threshold maintains validity bounds.

**Formal**:
```
apply_update_threshold(new_thresh) succeeds ⟹
  1 ≤ new_thresh ≤ signer_count
```

---

## Temporal Properties

### TP1: Expiry Enforcement
**Property**: After expiry, proposals cannot be signed or executed.

**Formal**:
```
now > p.expiry ⟹
  ¬can_sign(p) ∧ ¬can_execute(p)
```

**Expiry Calculation**:
```rust
const EXPIRY_SECONDS: u64 = 7 * 24 * 60 * 60; // 7 days
expiry = env.ledger().timestamp() + EXPIRY_SECONDS
```

**Verification Strategy**:
- Boundary test: sign/execute at expiry - 1, = expiry, = expiry + 1
- Time-travel test: advance ledger timestamp

### TP2: Proposal Lifespan
**Property**: Proposals expire 7 days after creation (unless executed or cancelled earlier).

**Formal**:
```
propose_action(...) at time t ⟹
  proposal.expiry = t + EXPIRY_SECONDS
```

---

## Accounting Properties

### AC1: Approval Count Accuracy
**Property**: Approval count equals the number of unique signers who have approved.

**Formal**:
```
p.approval_count = |{s | storage.get(Approval(p.id, s)) = true}|
```

**Verification Strategy**:
- Property test: generate random approval sequences, verify count
- Overflow test: approval_count < u32::MAX

### AC2: Proposal ID Uniqueness
**Property**: Proposal IDs are unique and monotonically increasing.

**Formal**:
```
∀ proposals p1, p2:
  p1.id = p2.id ⟹ p1 = p2
  p1 created before p2 ⟹ p1.id < p2.id
```

**Implementation**:
```rust
let count: u32 = env.storage().instance().get(&DataKey::ProposalCount).unwrap_or(0);
let new_id = count.checked_add(1).expect("proposal count overflow");
env.storage().instance().set(&DataKey::ProposalCount, &new_id);
```

---

## Attack Resistance Properties

### AR1: Unauthorized Execution Prevention
**Property**: Only meeting threshold approval requirement allows execution; no backdoor execution paths.

**Verification**:
- Code audit: ensure all execution paths check `approval_count >= threshold`
- Test: attempt execution with threshold - 1 approvals

### AR2: Replay Attack Prevention
**Property**: Executed or cancelled proposals cannot be re-executed.

**Formal**:
```
(p.executed ∨ p.cancelled) ⟹ ∀ future calls: execute(p) fails
```

### AR3: Signer Manipulation Resistance
**Property**: Signer set changes require multisig approval (proposals targeting self).

**Verification**:
- Test: attempt to add/remove signer without proposal
- Verify add_signer/remove_signer are not public functions

### AR4: Threshold Manipulation Resistance
**Property**: Threshold changes require multisig approval.

**Verification**:
- Test: attempt to update threshold without proposal
- Verify update_threshold is not a public function

---

## Reentrancy Properties

### RE1: Execution Order Safety
**Property**: Marking `executed = true` before external call prevents reentrancy attacks.

**Implementation Pattern** (Checks-Effects-Interactions):
```rust
// 1. Checks
if proposal.executed { return Err(Error::AlreadyExecuted); }
if proposal.approval_count < threshold { return Err(Error::BelowThreshold); }

// 2. Effects
proposal.executed = true;
env.storage().persistent().set(&DataKey::Proposal(action_id), &proposal);

// 3. Interactions
env.invoke_contract::<()>(&proposal.target, &proposal.function, proposal.args);
```

**Verification**:
- Reentrancy test: malicious contract calls back into multisig
- State persistence verification before external call

### RE2: Storage Consistency
**Property**: All storage updates are persisted before external calls.

**Verification**:
- Manual audit of external call sites
- Verify no storage writes after `invoke_contract`

---

## Configuration Properties

### CP1: Valid M-of-N Configurations
**Property**: Common multisig configurations are supported and valid.

**Examples**:
- 1-of-1 (single signer, no multisig)
- 2-of-3 (simple multisig)
- 3-of-5 (standard multisig)
- 5-of-7 (high-security multisig)

**Formal**:
```
∀ valid configs (m, n):
  initialize(signers[1..n], m) succeeds ⟺ (1 ≤ m ≤ n)
```

### CP2: Dynamic Configuration
**Property**: Signer set and threshold can be updated via proposals.

**Verification**:
- Test: 3-of-5 → add signer → 3-of-6
- Test: 3-of-5 → remove signer → 3-of-4
- Test: 3-of-5 → update threshold → 4-of-5

---

## Test Coverage Requirements

### Unit Tests
- ✅ Initialize with valid configurations
- ✅ Initialize with invalid configurations (threshold violations)
- ✅ Propose action as signer
- ✅ Propose action as non-signer (fails)
- ✅ Sign action as signer
- ✅ Sign action as non-signer (fails)
- ✅ Double-sign rejection
- ✅ Execute with sufficient approvals
- ✅ Execute with insufficient approvals (fails)
- ✅ Execute after expiry (fails)
- ✅ Execute already-executed proposal (fails)
- ✅ Cancel by proposer
- ✅ Cancel by signer
- ✅ Cancel by non-signer (fails)
- ✅ Self-targeted add_signer
- ✅ Self-targeted remove_signer
- ✅ Self-targeted update_threshold
- ✅ External contract invocation

### State Machine Tests
- ✅ Proposal lifecycle: created → approved → executed
- ✅ Proposal lifecycle: created → approved → cancelled
- ✅ Proposal lifecycle: created → expired

### Property Tests (Future)
- ⏳ Approval count accuracy over random approval sequences
- ⏳ Threshold validity maintained over random signer mutations

### Integration Tests
- ⏳ Multisig + Timelock: multisig proposes timelock schedule
- ⏳ Multisig + Escrow: multisig proposes escrow admin actions
- ⏳ Multisig + Treasury: multisig proposes treasury allocations

---

## Kani Harness Templates

### Template 1: Threshold Validity
```rust
#[cfg(kani)]
#[kani::proof]
fn verify_threshold_validity() {
    let env = Env::default();
    let contract = MultisigAdminContractClient::new(&env, &env.register_contract(None, MultisigAdminContract));
    
    let signers = generate_signers(&env, kani::any::<u8>() as usize % 10 + 1);
    let threshold = kani::any::<u32>() % (signers.len() as u32 + 2);
    
    let result = contract.try_initialize(&signers, &threshold);
    
    if threshold == 0 || threshold > signers.len() as u32 {
        kani::assert(result.is_err(), "Invalid threshold must fail");
    } else {
        kani::assert(result.is_ok(), "Valid threshold must succeed");
        kani::assert(contract.get_threshold() == threshold, "Threshold must match");
        kani::assert(contract.get_signer_count() == signers.len() as u32, "Signer count must match");
    }
}
```

### Template 2: Single Execution
```rust
#[cfg(kani)]
#[kani::proof]
fn verify_single_execution() {
    // Setup multisig with 2-of-3
    let (env, contract, signers) = setup_multisig_2_of_3();
    
    let proposal_id = contract.propose_action(&signers[0], &target, &function, &args);
    contract.sign_action(&signers[1], &proposal_id); // Threshold met
    
    // First execution succeeds
    let result1 = contract.try_execute_action(&proposal_id);
    kani::assert(result1.is_ok(), "First execution must succeed");
    
    // Second execution fails
    let result2 = contract.try_execute_action(&proposal_id);
    kani::assert(result2.is_err(), "Second execution must fail");
}
```

---

## MIRAI Annotation Examples

```rust
#[pre(!signers.is_empty(), "Signers list must not be empty")]
#[pre(threshold > 0, "Threshold must be positive")]
#[pre(threshold <= signers.len() as u32, "Threshold must not exceed signer count")]
#[post(result.is_ok() => self.threshold() == threshold, "Threshold must be set correctly")]
pub fn initialize(
    env: Env,
    signers: Vec<Address>,
    threshold: u32,
) -> Result<(), Error> {
    // implementation
}
```

---

## Formal Verification Roadmap

### Phase 1: Specifications (Current)
- ✅ Document all properties
- ✅ Define invariants
- ✅ Create test coverage matrix

### Phase 2: Tooling Setup
- ⏳ Configure Kani for multisig contract
- ⏳ Write proof harnesses for core properties

### Phase 3: Verification
- ⏳ Verify threshold validity invariant
- ⏳ Verify single execution property
- ⏳ Verify approval uniqueness

### Phase 4: Integration
- ⏳ Cross-contract property verification
- ⏳ Multisig + Timelock composition proofs

---

## References

- [Multisig Contract Source](../../../contracts/multisig_admin/src/lib.rs)
- [Multisig Tests](../../../contracts/multisig_admin/src/lib.rs#tests)
- [Kani Rust Verifier](https://model-checking.github.io/kani/)
- [EIP-4337 Account Abstraction (inspiration)](https://eips.ethereum.org/EIPS/eip-4337)

---

**Last Updated**: 2026-07-24  
**Version**: 1.0.0  
**Status**: Ready for Verification Implementation

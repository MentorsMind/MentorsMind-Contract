# Escrow Contract Formal Specifications

## Overview

The Escrow contract manages fund custody for mentoring sessions. It holds tokens from learners, releases them to mentors upon completion, handles disputes, and calculates platform fees. This document provides formal specifications for verification.

---

## Contract Metadata

- **Contract**: `EscrowContract`
- **Location**: `contracts/escrow/src/lib.rs`
- **Primary State**: `Escrow` (aka `EscrowRecord`)
- **State Machine**: `Active → {Released, Disputed, Refunded}; Disputed → {Resolved, Refunded}`

---

## State Types

### Escrow Status Enum
```rust
pub enum EscrowStatus {
    Active,      // Initial state after creation
    Released,    // Funds transferred to mentor (terminal)
    Disputed,    // Dispute opened by mentor or learner
    Resolved,    // Dispute resolved by admin (terminal)
    Refunded,    // Funds returned to learner (terminal)
}
```

### Escrow Record
```rust
pub struct EscrowRecord {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,                    // Remaining amount in contract
    pub quoted_token_amount: i128,       // Original deposited amount
    pub session_id: Symbol,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    pub platform_fee: i128,              // Accumulated fees paid
    pub net_amount: i128,                // Accumulated net paid to mentor
    pub session_end_time: u64,           // When session ends
    pub auto_release_delay: u64,         // Delay after session_end_time
    pub dispute_reason: Symbol,
    pub resolved_at: u64,
    pub total_sessions: u32,             // For multi-session escrows
    pub sessions_completed: u32,
}
```

---

## Safety Properties

### SP1: Fund Conservation
**Property**: At any point in time, the contract's token balance for each token equals the sum of all active/disputed escrow amounts for that token.

**Formal**:
```
∀ token T, time t:
  balance(contract, T, t) = 
    Σ { e.amount | e ∈ escrows, e.token = T, e.status ∈ {Active, Disputed} }
```

**Verification Strategy**:
- State invariant check before/after each state transition
- Property-based testing: generate random escrow operations, verify balance
- Symbolic execution: prove no path violates this property

### SP2: No Fund Loss
**Property**: Funds can only leave the contract through authorized releases, refunds, or dispute resolutions.

**Formal**:
```
balance_decrease(contract, T) ⟹ 
  ∃ operation op ∈ {release, refund, resolve_dispute}:
    authorized(op) ∧ amount(op) = balance_decrease
```

**Verification Strategy**:
- Audit all token transfer calls
- Verify each transfer has corresponding authorization check
- Ensure no "backdoor" transfer functions

### SP3: State Machine Integrity
**Property**: All state transitions follow the defined state machine diagram.

**Valid Transitions**:
```
Active → Released (via release_funds, release_partial, admin_release, try_auto_release)
Active → Disputed (via dispute)
Active → Refunded (via refund)
Disputed → Resolved (via resolve_dispute)
Disputed → Refunded (via refund)
Released → Released (idempotent, no-op)
Resolved → Resolved (idempotent, no-op)
Refunded → Refunded (idempotent, no-op)
```

**Verification Strategy**:
- Exhaustive state transition testing (see `tests/state_machine_tests.rs`)
- Kani proof harness exploring all possible transition sequences
- MIRAI annotation: `#[invariant(valid_state_transition(old_state, new_state))]`

### SP4: Double-Spend Prevention
**Property**: Each escrow's funds can be released exactly once, across all release mechanisms.

**Formal**:
```
∀ escrow e:
  count(release_events(e)) + count(refund_events(e)) + count(resolve_events(e)) = 1
```

**Verification Strategy**:
- Check `status` flag before and after each operation
- Verify terminal states cannot transition
- Test concurrent release attempts (reentrancy simulation)

### SP5: Authorization Correctness
**Property**: Each operation requires authorization from the correct party.

**Authorization Matrix**:
| Operation           | Required Auth                |
|---------------------|------------------------------|
| `create_escrow`     | `learner`                    |
| `release_funds`     | `learner` OR `admin`         |
| `release_partial`   | `learner` OR `admin`         |
| `admin_release`     | `admin`                      |
| `try_auto_release`  | None (permissionless)        |
| `dispute`           | `mentor` OR `learner`        |
| `resolve_dispute`   | `admin`                      |
| `refund`            | `admin`                      |

**Verification Strategy**:
- Access control test suite with unauthorized callers
- Code audit for `require_auth()` placement
- Kani harness: for each operation, prove `unauthorized_caller ⟹ panic`

---

## Liveness Properties

### LP1: Auto-Release Availability
**Property**: If `now >= session_end_time + auto_release_delay + TOLERANCE` and escrow is Active, then `try_auto_release` succeeds.

**Formal**:
```
(e.status = Active) ∧ 
(now ≥ e.session_end_time + e.auto_release_delay + TIMESTAMP_TOLERANCE_SECS) ∧
(now < e.session_end_time + e.auto_release_delay + OPERATION_EXPIRY_SECS)
⟹ can_auto_release(e)
```

**Verification Strategy**:
- Time-based test suite advancing ledger timestamp
- Boundary testing at exact threshold moments
- Symbolic execution over timestamp ranges

### LP2: Dispute Openability
**Property**: While an escrow is Active, both mentor and learner can open disputes.

**Formal**:
```
(e.status = Active) ⟹ 
  can_dispute(e, mentor) ∧ can_dispute(e, learner)
```

### LP3: Admin Override
**Property**: Admin can always force-release Active escrows or refund Active/Disputed escrows.

**Formal**:
```
(e.status = Active) ⟹ can_admin_release(e)
(e.status ∈ {Active, Disputed}) ⟹ can_refund(e)
```

---

## Functional Correctness Properties

### FC1: Fee Calculation Accuracy
**Property**: Platform fee is calculated correctly as `(amount * fee_bps) / 10_000`.

**Formal**:
```
∀ escrow e after release:
  e.platform_fee = floor((e.quoted_token_amount * fee_bps) / 10_000)
  e.net_amount = e.quoted_token_amount - e.platform_fee
```

**Verification Strategy**:
- Arithmetic property tests with various fee_bps values
- Overflow/underflow tests (all ops are `.checked_*()`)
- Symbolic execution over fee_bps ∈ [0, MAX_FEE_BPS]

### FC2: Partial Release Correctness
**Property**: Multi-session escrows release equal amounts per session, with remainder on last session.

**Formal**:
```
∀ escrow e with total_sessions n:
  per_session_amount = floor(e.quoted_token_amount / n)
  ∀ session i < n: release_i = per_session_amount
  release_n = e.amount (remaining after n-1 releases)
  Σ(release_i for i ∈ [1..n]) = e.quoted_token_amount
```

**Verification Strategy**:
- Property-based tests: random total_sessions, verify sum
- Edge cases: total_sessions = 1, prime numbers, large values
- Arithmetic proofs for rounding error bounds

### FC3: Dispute Resolution Split
**Property**: Disputed funds are split according to `mentor_pct` without deducting platform fee.

**Formal**:
```
resolve_dispute(e, mentor_pct) ⟹
  mentor_share = floor(e.amount * mentor_pct / 100)
  learner_share = e.amount - mentor_share
  platform_fee = 0 (no fee on dispute resolution)
```

### FC4: Token Whitelist Enforcement
**Property**: Escrows can only be created with approved tokens.

**Formal**:
```
create_escrow(token) succeeds ⟹ 
  storage.get(ApprovedToken(token)) = true
```

---

## Temporal Properties

### TP1: Auto-Release Window Correctness
**Property**: Auto-release is available only within the valid time window.

**Formal**:
```
auto_release_ready(e) ⟺
  e.status = Active ∧
  now ≥ e.session_end_time + e.auto_release_delay + TIMESTAMP_TOLERANCE_SECS ∧
  now < e.session_end_time + e.auto_release_delay + OPERATION_EXPIRY_SECS
```

**Constants**:
- `TIMESTAMP_TOLERANCE_SECS = 60` (1 minute buffer)
- `OPERATION_EXPIRY_SECS = 14 * 24 * 60 * 60` (14 days)

### TP2: Creation Timestamp Validity
**Property**: Escrow creation timestamp matches ledger timestamp.

**Formal**:
```
create_escrow(...) ⟹ escrow.created_at = env.ledger().timestamp()
```

---

## Accounting Properties

### AC1: Balance Reconciliation
**Property**: For each escrow release, the distribution satisfies:

**Formal**:
```
release_funds(e) ⟹
  amount_to_treasury = e.platform_fee
  amount_to_mentor = e.net_amount
  amount_to_treasury + amount_to_mentor = e.quoted_token_amount
```

### AC2: Zero-Fee Edge Case
**Property**: If `fee_bps = 0`, entire amount goes to mentor.

**Formal**:
```
(fee_bps = 0) ⟹ (e.platform_fee = 0 ∧ e.net_amount = e.quoted_token_amount)
```

### AC3: Maximum Fee Enforcement
**Property**: Platform fee never exceeds 10% (MAX_FEE_BPS = 1000).

**Formal**:
```
∀ escrow e:
  e.platform_fee ≤ (e.quoted_token_amount * 1000) / 10_000
```

---

## Data Integrity Properties

### DI1: ID Uniqueness
**Property**: Each escrow has a unique ID generated by incrementing a counter.

**Formal**:
```
∀ escrows e1, e2:
  e1.id = e2.id ⟹ e1 = e2
```

### DI2: Address Validity
**Property**: Mentor and learner addresses are non-zero and distinct (recommended practice).

**Formal**:
```
create_escrow(mentor, learner, ...) succeeds ⟹
  mentor ≠ Address::zero() ∧
  learner ≠ Address::zero() ∧
  mentor ≠ learner (best practice, not enforced)
```

### DI3: Amount Positivity
**Property**: Escrow amounts are always positive.

**Formal**:
```
create_escrow(amount, ...) succeeds ⟹ amount > 0
```

---

## Reentrancy Properties

### RE1: No Cross-Function Reentrancy
**Property**: External contract calls (token transfers, invoke_contract) cannot re-enter escrow contract to modify state.

**Mitigation**:
- State updates before external calls (checks-effects-interactions pattern)
- Soroban runtime prevents direct reentrancy (platform guarantee)

**Verification Strategy**:
- Manual audit of external call sites
- Verify state changes happen before transfers
- Test with malicious token contract

### RE2: Idempotent Terminal States
**Property**: Calling release/refund on terminal states is idempotent (panics or no-op).

**Formal**:
```
(e.status ∈ {Released, Refunded, Resolved}) ⟹
  operation(e) panics or is no-op
```

---

## Upgrade Safety Properties

### UP1: Storage Layout Compatibility
**Property**: Adding new fields to `EscrowRecord` maintains backward compatibility via eternal storage pattern.

**Mitigation**:
- Use typed `DataKey` enum for storage keys
- Adding variants to DataKey is safe (no key collision)
- Old escrows remain readable with default values for new fields

### UP2: State Machine Extension Safety
**Property**: Adding new states to `EscrowStatus` requires explicit transition rules.

**Verification**:
- Exhaustive pattern matching (Rust compiler enforces)
- State machine tests cover all new transitions
- Migration path documented

---

## Attack Resistance Properties

### AR1: Front-Running Resistance
**Property**: Auto-release is permissionless; front-running has no economic advantage.

**Rationale**: Any caller can trigger auto-release once window opens; funds always go to mentor (not caller).

### AR2: Griefing Resistance
**Property**: Malicious actors cannot lock funds indefinitely.

**Mitigation**:
- Auto-release provides escape hatch after delay
- Admin can force-release or refund
- Dispute resolution by admin prevents deadlock

### AR3: Timestamp Manipulation Resistance
**Property**: Timestamp values come from ledger (platform-guaranteed monotonic).

**Assumption**: Stellar consensus prevents timestamp manipulation.

---

## Test Coverage Requirements

### Unit Tests
- ✅ Create escrow with valid parameters
- ✅ Release funds (learner and admin)
- ✅ Partial releases for multi-session escrows
- ✅ Auto-release after delay
- ✅ Dispute opening by mentor/learner
- ✅ Dispute resolution with various splits
- ✅ Refund by admin
- ✅ Token whitelist enforcement
- ✅ Fee calculation accuracy
- ✅ Unauthorized caller rejection

### State Machine Tests
- ✅ All valid state transitions
- ✅ All invalid transition attempts
- ✅ Terminal state immutability

### Property Tests (Future)
- ⏳ Fund conservation across random operations
- ⏳ Fee calculation over random amounts and fee_bps
- ⏳ Partial release sum equals total amount

### Integration Tests
- ⏳ Escrow + Treasury integration (fee distribution)
- ⏳ Escrow + Verification integration (session completion)
- ⏳ Escrow + Dispute Evidence integration

---

## Kani Harness Templates

### Template 1: Fund Conservation
```rust
#[cfg(kani)]
#[kani::proof]
fn verify_fund_conservation() {
    let env = Env::default();
    let contract = EscrowContractClient::new(&env, &env.register_contract(None, EscrowContract));
    
    // ... initialize contract ...
    
    let initial_balance = get_total_balance(&env, &contract);
    
    // Non-deterministic operation sequence
    let op = kani::any();
    match op {
        Op::Create => { /* create escrow */ },
        Op::Release => { /* release escrow */ },
        Op::Refund => { /* refund escrow */ },
    }
    
    let final_balance = get_total_balance(&env, &contract);
    
    // Verify balance change matches operation
    kani::assert(balance_matches_operation(initial_balance, final_balance, op));
}
```

### Template 2: State Machine Validity
```rust
#[cfg(kani)]
#[kani::proof]
fn verify_state_transitions() {
    let initial_state: EscrowStatus = kani::any();
    let operation: Operation = kani::any();
    
    let result = apply_operation(initial_state, operation);
    
    kani::assert(
        is_valid_transition(initial_state, result.new_state),
        "Invalid state transition detected"
    );
}
```

---

## MIRAI Annotation Examples

```rust
#[pre(amount > 0, "Amount must be positive")]
#[pre(is_token_approved(&env, &token_address), "Token not approved")]
#[post(result.status == EscrowStatus::Active, "New escrow must be Active")]
#[post(result.amount == amount, "Escrow amount must match input")]
pub fn create_escrow(
    env: Env,
    mentor: Address,
    learner: Address,
    amount: i128,
    token_address: Address,
    ...
) -> u64 {
    // implementation
}
```

---

## Formal Verification Roadmap

### Phase 1: Specifications (Current)
- ✅ Document all properties
- ✅ Define invariants
- ✅ Create test coverage matrix

### Phase 2: Tooling Setup (Next)
- ⏳ Install Kani and configure workspace
- ⏳ Write initial proof harnesses
- ⏳ Set up CI integration

### Phase 3: Incremental Verification
- ⏳ Verify arithmetic properties (fee calculation)
- ⏳ Verify state machine integrity
- ⏳ Verify authorization correctness

### Phase 4: Full Verification
- ⏳ Fund conservation proof
- ⏳ Cross-contract property proofs
- ⏳ Integration with multisig/timelock specs

---

## References

- [Escrow Contract Source](../../../contracts/escrow/src/lib.rs)
- [Escrow State Machine Tests](../../../tests/state_machine_tests.rs)
- [Soroban Token Interface](https://soroban.stellar.org/docs/reference/interfaces/token-interface)
- [Kani Rust Verifier](https://model-checking.github.io/kani/)

---

**Last Updated**: 2026-07-24  
**Version**: 1.0.0  
**Status**: Ready for Verification Implementation

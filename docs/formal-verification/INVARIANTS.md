# Core Protocol Invariants

This document defines the fundamental invariants that must hold across the MentorMinds contract suite. These invariants form the foundation for formal verification and security auditing.

## Table of Contents

1. [Cross-Contract Invariants](#cross-contract-invariants)
2. [Escrow Invariants](#escrow-invariants)
3. [Multisig Invariants](#multisig-invariants)
4. [Timelock Invariants](#timelock-invariants)
5. [Treasury Invariants](#treasury-invariants)
6. [Verification Methodology](#verification-methodology)

---

## Cross-Contract Invariants

### I1: Authorization Monotonicity
**Statement**: Once a `require_auth()` check passes for an address, all subsequent operations in that transaction context maintain that authorization.

**Rationale**: Ensures authorization cannot be revoked mid-transaction.

**Formal**: ∀ tx, addr: `auth(tx, addr) ⟹ ∀ op ∈ tx: auth_valid(op, addr)`

### I2: Timestamp Monotonicity
**Statement**: Within a ledger, `env.ledger().timestamp()` returns a constant value. Across ledgers, timestamps are monotonically increasing.

**Rationale**: Prevents time-based attacks and ensures temporal ordering.

**Formal**: 
- `∀ calls c1, c2 in ledger L: timestamp(c1) = timestamp(c2)`
- `∀ ledgers L1, L2: sequence(L1) < sequence(L2) ⟹ timestamp(L1) ≤ timestamp(L2)`

### I3: Storage Isolation
**Statement**: Contract A cannot read or modify Contract B's storage without invoking Contract B.

**Rationale**: Platform-level guarantee enforced by Soroban runtime.

**Formal**: `∀ contracts A, B: A ≠ B ⟹ storage(A) ∩ storage(B) = ∅`

### I4: Arithmetic Safety
**Statement**: All arithmetic operations use checked variants and panic on overflow/underflow rather than wrapping.

**Rationale**: Prevents silent arithmetic errors in financial calculations.

**Formal**: `∀ op ∈ {+, -, *, /}: result = op.checked() or panic`

---

## Escrow Invariants

### E1: Fund Conservation
**Statement**: The sum of all escrow amounts plus fees equals the contract's token balance.

**Formal**:
```
∀ token T:
  balance(contract, T) = 
    Σ(escrow_i.amount where escrow_i.token = T ∧ escrow_i.status ∈ {Active, Disputed})
```

**Verification**: Check at state transitions (create, release, refund, resolve_dispute).

### E2: Single Terminal State
**Statement**: Once an escrow reaches a terminal state (Released, Refunded, Resolved), it cannot transition to any other state.

**Formal**:
```
∀ escrow e, states s1, s2:
  (e.status = s1 ∧ s1 ∈ {Released, Refunded, Resolved} ∧ transition(e, s2))
  ⟹ s2 = s1
```

**Verification**: State machine exhaustive testing + runtime assertions.

### E3: Authorization Correctness
**Statement**: State transitions require authorization from specific parties:
- `create_escrow`: learner
- `release_funds`: learner OR admin
- `dispute`: mentor OR learner
- `resolve_dispute`: admin only
- `refund`: admin only

**Formal**:
```
transition(escrow, state) ⟹ authorized(caller, transition, escrow)
```

**Verification**: Access control tests + manual audit.

### E4: Fee Accounting
**Statement**: Released fees satisfy: `platform_fee = (gross_amount * fee_bps) / 10_000` and `net_amount = gross_amount - platform_fee`.

**Formal**:
```
∀ escrow e after release:
  e.platform_fee = (e.quoted_token_amount * fee_bps) / 10_000 ∧
  e.net_amount = e.quoted_token_amount - e.platform_fee
```

**Verification**: Arithmetic property tests + symbolic execution.

### E5: No Double Release
**Statement**: An escrow's funds can be released exactly once across all release mechanisms (manual, partial, auto, admin).

**Formal**:
```
∀ escrow e:
  released(e) ⟹ 
    ¬(can_release(e) ∨ can_auto_release(e) ∨ can_admin_release(e))
```

**Verification**: State machine testing + runtime guards.

### E6: Dispute Window Safety
**Statement**: A dispute can only be opened on Active escrows and prevents releases until resolved or refunded.

**Formal**:
```
dispute(e) ⟹ 
  (e.status = Active) ∧
  (∀ future operations: ¬release(e) until resolve_dispute(e) ∨ refund(e))
```

### E7: Auto-Release Temporal Correctness
**Statement**: Auto-release can succeed only when:
```
now >= session_end_time + auto_release_delay + TOLERANCE
```

**Formal**:
```
auto_release(e, now) succeeds ⟹
  now >= e.session_end_time + e.auto_release_delay + TIMESTAMP_TOLERANCE_SECS
```

### E8: Token Whitelist Enforcement
**Statement**: Escrows can only be created with approved tokens.

**Formal**:
```
create_escrow(token) succeeds ⟹ approved_tokens[token] = true
```

### E9: Partial Release Consistency
**Statement**: Multi-session escrows release exactly `total_amount / total_sessions` per session, with remainder handling on final session.

**Formal**:
```
∀ escrow e with total_sessions n:
  Σ(released_amounts_i for i ∈ [1..n]) = e.total_amount ∧
  e.sessions_completed ≤ e.total_sessions
```

---

## Multisig Invariants

### M1: Threshold Validity
**Statement**: The approval threshold is always between 1 and the number of signers (inclusive).

**Formal**:
```
∀ states: 1 ≤ threshold ≤ signer_count
```

**Verification**: Initialization checks + mutation testing.

### M2: Approval Uniqueness
**Statement**: Each signer can approve a proposal at most once.

**Formal**:
```
∀ proposal p, signer s:
  approval_count(p, s) ≤ 1
```

**Verification**: Storage key uniqueness + double-approval test.

### M3: Execution Guard
**Statement**: A proposal can execute only when:
1. `approval_count >= threshold`
2. `now <= expiry`
3. `!executed`
4. `!cancelled`

**Formal**:
```
execute(p) succeeds ⟹
  (p.approval_count ≥ threshold) ∧
  (now ≤ p.expiry) ∧
  (¬p.executed) ∧
  (¬p.cancelled)
```

### M4: Single Execution
**Statement**: Each proposal executes at most once.

**Formal**:
```
∀ proposal p:
  executed(p) ⟹ ∀ future attempts: ¬can_execute(p)
```

**Verification**: Reentrancy testing + executed flag check.

### M5: Proposer Auto-Approval
**Statement**: When a proposal is created, the proposer is automatically recorded as the first approval.

**Formal**:
```
propose(proposer, ...) ⟹
  approval_count(proposal) = 1 ∧
  has_approved(proposal, proposer) = true
```

### M6: Signer Set Consistency
**Statement**: Adding/removing signers maintains threshold validity.

**Formal**:
```
remove_signer(s) succeeds ⟹ (signer_count - 1) ≥ threshold
add_signer(s) succeeds ⟹ ¬is_signer(s)
```

### M7: Cancellation Authorization
**Statement**: Only the proposer or any current signer can cancel a non-executed, non-expired, non-cancelled proposal.

**Formal**:
```
cancel(p, caller) succeeds ⟹
  (caller = p.proposer ∨ is_signer(caller)) ∧
  ¬p.executed ∧ ¬p.cancelled ∧ now ≤ p.expiry
```

### M8: Self-Targeted Operations
**Statement**: Proposals targeting the multisig contract itself (add_signer, remove_signer, update_threshold) execute via internal helpers, not external invoke.

**Formal**:
```
execute(p) where p.target = current_contract ⟹
  use_internal_helper(p.function)
```

---

## Timelock Invariants

### T1: Operation Uniqueness
**Statement**: Operation IDs are collision-resistant (SHA-256 over full operation payload including salt and nonce).

**Formal**:
```
∀ operations op1, op2:
  op1.id = op2.id ⟹ op1 = op2 (except salt)
```

**Verification**: Hash function cryptographic assumption + salt uniqueness tests.

### T2: Delay Bounds
**Statement**: All scheduled operations have delays within `[MIN_DELAY, MAX_DELAY]`.

**Formal**:
```
schedule(op, delay) succeeds ⟹
  MIN_DELAY ≤ delay ≤ MAX_DELAY
```

### T3: Execution Temporal Correctness
**Statement**: An operation executes only when:
```
ready_at + TOLERANCE ≤ now < ready_at + EXPIRY
```

**Formal**:
```
execute(op) succeeds ⟹
  (op.ready_at + TIMESTAMP_TOLERANCE_SECS ≤ now) ∧
  (now < op.ready_at + OPERATION_EXPIRY_SECS)
```

### T4: Single Execution
**Statement**: Each operation executes at most once.

**Formal**:
```
∀ operation op:
  op.done ⟹ ∀ future attempts: ¬can_execute(op)
```

### T5: Cancellation Authorization
**Statement**: An operation can be cancelled by:
- The proposer (self-cancellation)
- The admin (admin override)

**Formal**:
```
cancel(op, caller) succeeds ⟹
  (caller = op.proposer ∨ caller = admin) ∧
  ¬op.done
```

### T6: Operation Immutability
**Statement**: Once scheduled, operation parameters (target, function, args, ready_at) cannot be modified.

**Formal**:
```
∀ operation op after schedule:
  immutable(op.target) ∧ immutable(op.function) ∧ 
  immutable(op.args) ∧ immutable(op.ready_at)
```

### T7: Expiry Prevents Execution
**Statement**: Expired operations cannot be executed.

**Formal**:
```
∀ operation op:
  now ≥ op.ready_at + OPERATION_EXPIRY_SECS ⟹ ¬can_execute(op)
```

---

## Treasury Invariants

### TR1: Buyback Authorization
**Statement**: `buyback_and_burn` can only be called by the registered timelock contract.

**Formal**:
```
buyback_and_burn(...) succeeds ⟹ caller = registered_timelock
```

### TR2: Approve-Pull Atomicity
**Statement**: In `buyback_and_burn`, the approve → swap → validate → burn sequence is atomic. If swap fails or returns insufficient MNT, allowance is revoked and no XLM leaves treasury.

**Formal**:
```
buyback_and_burn(xlm_amount, min_mnt_out) fails ⟹
  balance(treasury, XLM) unchanged ∧
  allowance(treasury → dex, XLM) = 0
```

### TR3: Token Whitelist Enforcement
**Statement**: `deposit`, `allocate`, `distribute_to_stakers`, and `buyback_and_burn` only accept approved tokens.

**Formal**:
```
∀ operations op involving token T:
  op succeeds ⟹ approved_tokens[T] = true
```

### TR4: Slippage Protection
**Statement**: Buyback fails if received MNT < min_mnt_out.

**Formal**:
```
buyback_and_burn(xlm_amt, min_mnt) where swap_result < min_mnt ⟹
  operation reverts ∧ emit(BuybackFailed)
```

---

## Verification Methodology

### Static Analysis
1. **Type Safety**: Rust type system enforces memory safety
2. **Ownership**: Borrow checker prevents aliasing bugs
3. **Checked Arithmetic**: All financial operations use `.checked_*()` variants

### Dynamic Testing
1. **State Machine Tests**: Exhaustive state transition validation
2. **Property-Based Tests**: Randomized input testing (QuickCheck/proptest)
3. **Fuzz Testing**: Boundary condition exploration

### Formal Methods (Future)
1. **Kani**: Bounded model checking for critical functions
2. **MIRAI**: Abstract interpretation for invariant propagation
3. **Creusot**: Deductive verification with SMT solvers

### Manual Auditing
1. **Authorization Audit**: Verify `require_auth()` coverage
2. **Reentrancy Audit**: Check cross-contract call safety
3. **Integer Overflow Audit**: Confirm checked arithmetic usage

---

## Cryptographic Assumptions

### Hash Function (SHA-256)
- **Collision Resistance**: Finding two inputs with same hash is computationally infeasible
- **Pre-image Resistance**: Finding input from hash is computationally infeasible
- **Usage**: Timelock operation IDs, merkle tree constructions

### Digital Signatures (Stellar Native)
- **Unforgeability**: Only key holder can produce valid signatures
- **Platform Enforcement**: Soroban `require_auth()` validates signatures
- **Usage**: Transaction authorization, access control

---

## Economic Assumptions

### Token Standards
- **SEP-41 Compliance**: All approved tokens follow Stellar token standard
- **Transfer Atomicity**: Token transfers succeed or revert atomically
- **Balance Consistency**: Token balances reflect transfer history

### Fee Bounds
- **Max Platform Fee**: `fee_bps ≤ MAX_FEE_BPS (1000)` enforced at initialization and update
- **Arithmetic Safety**: Fee calculations cannot overflow (checked operations)

### Price Oracle (Dynamic Fees)
- **Freshness**: Cached prices refresh per ledger
- **Fallback**: Returns default fee if oracle unavailable
- **Non-manipulation**: Assumes external price feed integrity (out of scope)

---

## Platform Assumptions (Soroban/Stellar)

### Ledger Integrity
- **Append-Only**: Past ledgers cannot be modified
- **Consensus**: Network reaches agreement on ledger state
- **Finality**: Confirmed transactions are permanent

### Runtime Guarantees
- **Memory Safety**: Host functions prevent buffer overflows
- **Gas Metering**: Execution halts if resource limits exceeded
- **Storage Isolation**: Contracts cannot access each other's storage

### Timestamp Properties
- **Monotonicity**: Ledger timestamps increase
- **Bounded Drift**: Timestamps are within reasonable bounds of wall-clock time
- **Determinism**: Same ledger timestamp for all operations in that ledger

---

## Invariant Violation Handling

### Detection
- **Assertions**: Runtime `assert!()` and `expect()` for invariant checks
- **Events**: Emit events on state transitions for off-chain monitoring
- **View Functions**: Expose contract state for health checks

### Response
- **Panic on Violation**: Revert transaction if invariant broken
- **Emergency Pause**: Admin can pause operations (if pause mechanism implemented)
- **Upgrade Path**: UUPS upgrade mechanism for critical fixes

### Post-Mortem
- **Event Log Analysis**: Reconstruct violation from emitted events
- **Snapshot Testing**: Compare state snapshots before/after
- **Formal Verification**: Update specifications to prevent recurrence

---

## Future Work

1. **Kani Integration**: Add `#[kani::proof]` harnesses for critical invariants
2. **MIRAI Annotations**: Embed `#[pre]` and `#[post]` conditions in code
3. **Symbolic Execution**: Use KLEE or similar for path exploration
4. **Automated Theorem Proving**: Encode invariants in Coq/Isabelle
5. **Continuous Verification**: CI pipeline runs verification on every commit

---

## References

- [Soroban Security Best Practices](https://soroban.stellar.org/docs/learn/security)
- [Kani Rust Verifier](https://model-checking.github.io/kani/)
- [MIRAI Abstract Interpretation](https://github.com/facebookexperimental/MIRAI)
- [Smart Contract Verification Survey](https://arxiv.org/abs/2008.02712)
- [Formal Verification of Financial Smart Contracts](https://eprint.iacr.org/2020/1062)

---

**Last Updated**: 2026-07-24  
**Version**: 1.0.0  
**Status**: Ready for Formal Verification Tooling Integration

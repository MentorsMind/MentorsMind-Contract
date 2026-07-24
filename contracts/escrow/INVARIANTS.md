# Escrow Contract Invariants

This document defines escrow invariants and state transition constraints that must hold at all times.

## Escrow States

Escrow status mirrors used across contracts:
- `Pending`
- `Active`
- `Disputed`
- `Released`
- `Refunded`
- `Resolved`

Terminal states:
- `Released`
- `Refunded`
- `Resolved`

## Transition Rules

Valid transitions:
- `Pending -> Active`
- `Pending -> Refunded`
- `Active -> Released`
- `Active -> Disputed`
- `Active -> Refunded`
- `Disputed -> Resolved`
- `Disputed -> Refunded`

Invalid transitions (examples):
- `Released -> Active`
- `Refunded -> Active`
- `Resolved -> Active`
- `Disputed -> Active`
- `Active -> Pending`

Transition guards:
- `Pending -> Active`: learner deposits funds successfully
- `Active -> Released`: authorized release, or delay condition for auto-release
- `Active -> Disputed`: authorized participant challenge
- `Active -> Refunded`: admin workflow
- `Disputed -> Resolved`: administrative/arbitration resolution
- `Disputed -> Refunded`: admin refund from disputed state

## Invariant 1: Token Balance Consistency

**Statement:**
```
sum(all active escrow amounts) <= contract.token_balance
```

**Rationale:** Prevents insolvency and double-spend conditions.

**Verification:** Re-check after every amount-changing state transition.

## Invariant 2: State Transition Validity

**Statement:** Every transition must be in the allowed set above.

**Rationale:** Prevents invalid reverse transitions and double-release paths.

**Verification:** Validate transition before persistence; reject invalid transitions.

## Invariant 3: Session Completion Bounds

**Statement:**
```
sessions_completed <= total_sessions
```

**Rationale:** Prevents impossible lifecycle bookkeeping.

**Verification:** Check after any session completion update.

## Invariant 4: Fund Conservation on Release

**Statement:**
```
platform_fee + net_amount_to_recipient == original_amount
```

**Rationale:** Ensures exact fund accounting.

**Verification:** Validate arithmetic before token transfer.

## Invariant 5: Exclusive Fund Distribution

**Statement:** Exactly one release beneficiary path applies per escrow outcome.

**Rationale:** Prevents duplicate payout.

**Verification:** Enforce mutually exclusive resolution path selection.

## Invariant 6: Escrow Amount Non-Negativity

**Statement:**
```
escrow.amount >= 0
```

**Rationale:** Prevents invalid token arithmetic.

**Verification:** Validate amount on create/update.

## Invariant 7: Timestamp Consistency

**Statement:**
- `created_at <= current_time`
- `created_at <= release_time` (if released)
- `resolved_at >= created_at` (if resolved)

**Rationale:** Preserves temporal consistency for release/dispute windows.

**Verification:** Validate timestamps before persisting terminal transitions.

## Transition Condition Examples

### Example A: Happy Path Release
1. Escrow created as `Active`.
2. Authorized release occurs.
3. Transition: `Active -> Released`.
4. Fund conservation and exclusive distribution checks pass.

### Example B: Dispute Resolution
1. Escrow created as `Active`.
2. Participant raises dispute.
3. Transition: `Active -> Disputed`.
4. Resolution executes.
5. Transition: `Disputed -> Resolved`.

### Example C: Invalid Reverse Transition (Rejected)
1. Escrow reaches `Released`.
2. Attempted transition to `Active`.
3. Rejected by transition validity invariant.

## Testing Strategy

### Unit Tests
- Assert each valid transition is accepted.
- Assert representative invalid transitions panic/fail.

### Property-Based Tests
- Generate random operation sequences and assert invariants after each operation.

### Snapshot/Integration Tests
- Capture complete state before and after lifecycle operations.
- Validate state machine behavior across interconnected contracts.

## Failure Mode

If any invariant fails:
1. The transaction aborts.
2. State changes are rolled back.
3. No partial mutation persists.

## Related Docs

- `docs/STATE_MACHINE.md`
- `docs/state-machines.md`
- `ARCHITECTURE.md`

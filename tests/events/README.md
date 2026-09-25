# Event Emission Test Cases

This folder tracks event-emission coverage for state-changing contract operations.

## Covered automated suites

- `contracts/dispute_evidence/src/lib.rs`
  - evidence submission emits `evidence_submitted`
  - dispute resolution emits `dispute_resolved`
  - event payload fields decode and match expected values
  - event ordering is asserted (`evidence_submitted` before `dispute_resolved`)

- `contracts/governance/src/lib.rs`
  - proposal lifecycle emits expected governance events
  - arbitrator registration emits governance registry event

## Target escrow lifecycle events

Escrow lifecycle event test expectations are tracked as:

1. escrow creation emits `Escrow.Created` with correct participants and amount.
2. escrow release emits `Escrow.Released` with fee/net split accuracy.
3. escrow refund emits `Escrow.Refunded` with learner/amount/token accuracy.
4. dispute flow emits open + resolution events in deterministic order.

These cases should stay aligned with `docs/TESTING.md` when event payload schemas change.

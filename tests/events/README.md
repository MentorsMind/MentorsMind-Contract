# Event Emission Test Cases

This folder tracks event-emission coverage for state-changing contract operations.

For comprehensive event schema documentation, see [`docs/events.md`](../../docs/events.md).

## Event Schema Reference

All MentorsMind contracts use a standardized 3-element topic tuple:
- `(contract: Symbol, version: u32, event_type: Symbol)`

See the main [Event Documentation](../../docs/events.md) for:
- Complete event types by contract
- Schema version policy
- Usage examples and best practices
- Guidelines for adding new events

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

## Related Documentation

- [`docs/events.md`](../../docs/events.md) - Complete event schema reference
- [`event_ordering_matrix.md`](event_ordering_matrix.md) - Expected event sequences
- [`events_schema.json`](../../events_schema.json) - Machine-readable schema

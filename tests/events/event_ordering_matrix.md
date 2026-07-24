# Event Ordering Matrix

| Flow | Expected ordered events |
| --- | --- |
| Dispute evidence flow | `evidence_submitted` -> `dispute_resolved` |
| Governance proposal execution | `proposal_created` -> `vote_cast` -> `proposal_passed`/`proposal_failed` -> `proposal_executed` (if passed) |
| Escrow full lifecycle | `Created` -> `DisputeOpened` (optional) -> `Released`/`Refunded`/`DisputeResolved` |

## Payload checks

- Topic contract/function identifiers are stable and explicit.
- Numeric fields (`amount`, `net_amount`, `platform_fee`) are verified for arithmetic correctness.
- Address fields (`mentor`, `learner`, `arbitrator`) are verified to match caller/state.

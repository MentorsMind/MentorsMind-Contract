# Protocol Observability & Monitoring (Issue #597)

This document defines the MentorsMind protocol observability layer: the health
indicators to track and the standardized on-chain events that off-chain
indexers consume to compute them.

It complements `docs/EVENTS.md` (per-event field reference) by focusing on the
**standardized topic layout** and the **operational metrics** derived from
events.

## Standardized event envelope

All monitoring events use the canonical 3-element topic tuple defined in
`contracts/shared/src/events.rs`:

```text
(contract: Symbol, version: u32, event_type: Symbol)
```

- `contract` — originating contract, e.g. `"escrow"`, `"governance"`,
  `"treasury"`, `"staking"`, `"timelock"`.
- `version` — `EVENT_SCHEMA_VERSION` (currently `1`); indexers reject unknown
  versions.
- `event_type` — the specific event, e.g. `"created"`, `"released"`.

This layout is parseable without per-contract knowledge: an indexer routes on
`topic[0]`, selects a schema by `topic[1]`, and decodes the payload by
`topic[2]`. Field ordering within each payload struct is stable and defined in
`docs/EVENTS.md`.

Contracts emit standardized events via the typed helpers in `shared::events`
(`emit_escrow_event`, `emit_governance_event`, `emit_treasury_event`, …). The
escrow contract additionally retains its historical ad-hoc
`(Symbol("Escrow"), Symbol(EventType), id)` topics for backward compatibility;
new indexers should prefer the standardized envelope.

## Protocol health indicators

| Indicator | Definition | Derived from events |
| --- | --- | --- |
| Active escrow value (TVL) | Σ `amount` of escrows created − released − refunded − resolved | escrow `created` / `released` / `refunded` / `resolved` |
| Release throughput | count of `released` per window | escrow `released` |
| Auto-release rate | `auto_released` / total releases | escrow `auto_released` |
| Dispute rate | `disputed` / `created` | escrow `disputed` |
| Dispute resolution latency | `resolved.time` − `disputed` ledger time | escrow `disputed` / `resolved` |
| Refund rate | `refunded` / `created` | escrow `refunded` |
| Effective fee rate | `FeeApplied.effective_bps` distribution by tier | escrow `FeeApplied` (#676) |
| Fee revenue | Σ `platform_fee` | escrow `released` / `fee_distrib` |
| Governance activity | proposals created / voted / executed per window | governance `prop_created` / `vote_cast` / `prop_executed` |
| Treasury flow | deposits, allocations, distributions per window | treasury `deposited` / `allocated` / `distributed` |
| Timelock queue | scheduled vs executed vs cancelled ops | timelock `scheduled` / `executed` / `cancelled` |

## Event catalog (monitoring surface)

### Escrow (`contract = "escrow"`)

| event_type | Emitted when | Payload struct |
| --- | --- | --- |
| `created` | escrow created / funded | `EscrowCreatedEventData` |
| `released` | funds released to mentor | `EscrowReleasedEventData` |
| `auto_released` | permissionless auto-release fired | `EscrowAutoReleasedEventData` |
| `disputed` | dispute opened | `DisputeOpenedEventData` |
| `resolved` | dispute resolved (split) | `DisputeResolvedEventData` |
| `refunded` | escrow refunded to learner | `EscrowRefundedEventData` |
| `fee_distrib` | fee split recorded on release | `FeeDistributedEventData` |
| `FeeApplied` * | graduated fee applied on release (#676) | `FeeAppliedEventData` |

\* `FeeApplied` currently uses the ad-hoc `(Symbol("Escrow"),
Symbol("FeeApplied"), escrow_id)` topic. Fields:
`{ mentor, tier, base_bps, effective_bps, fee_amount }`.

### Governance (`contract = "governance"`)

| event_type | Emitted when |
| --- | --- |
| `prop_created` | proposal created |
| `vote_cast` | vote cast |
| `prop_queued` | proposal queued in timelock |
| `prop_executed` | proposal executed |
| `prop_cancelled` | proposal cancelled |

### Treasury (`contract = "treasury"`)

| event_type | Emitted when |
| --- | --- |
| `deposited` | funds deposited |
| `allocated` | funds allocated to a budget line |
| `distributed` | funds distributed to a recipient |

### Timelock (`contract = "timelock"`)

| event_type | Emitted when |
| --- | --- |
| `scheduled` | operation scheduled |
| `executed` | operation executed |
| `cancelled` | operation cancelled |

## Indexer guidance

- Subscribe per contract via the Horizon/RPC event stream; route on `topic[0]`.
- Persist `(contract, version, event_type, ledger_seq, timestamp, payload)`.
- Reject events whose `topic[1]` version is unknown to your decoder.
- Compute the indicators above with windowed aggregation keyed on
  `ledger_seq` / `timestamp`.

## Tests

Standardized-event emission is covered by unit tests in
`escrow/src/lib.rs` (`test_standard_created_and_released_events_emitted`,
`test_standard_dispute_and_resolve_events_emitted`,
`test_standard_refunded_event_emitted`), which assert the canonical
`(escrow, 1, event_type)` topic layout fires for each lifecycle transition.

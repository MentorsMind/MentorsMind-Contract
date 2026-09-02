# MentorsMind Event Schema

## Overview

All MentorsMind Soroban contracts emit events using a standardized 3-element topic tuple:

```
(contract: Symbol, version: u32, event_type: Symbol)
```

- **contract** — identifies the originating contract (e.g., `"escrow"`, `"governance"`)
- **version** — schema version; currently `EVENT_SCHEMA_VERSION = 1`
- **event_type** — identifies the specific event within the contract

This layout is stable and parseable without per-contract knowledge: an indexer can always read `topic[0]` to route to the right decoder, `topic[1]` to select the schema version, and `topic[2]` to select the field definition.

## Topic Layout

Every event MUST use exactly 3 topics:

| Index | Type    | Description                              |
|-------|---------|------------------------------------------|
| 0     | Symbol  | Contract identifier                      |
| 1     | u32     | Schema version (currently 1)             |
| 2     | Symbol  | Event type identifier                    |

The event data payload follows the topics as the second argument to `env.events().publish()`.

## Contract Identifiers

Each contract has a canonical identifier used in events:

| Contract           | Identifier    | Notes                          |
|--------------------|---------------|--------------------------------|
| escrow             | `escrow`      | Escrow lifecycle events        |
| governance         | `governance`  | Proposal and voting events     |
| staking            | `staking`     | Staking/unstaking events       |
| timelock           | `timelock`    | Timelock operation events      |
| bounty             | `bounty`      | Bounty lifecycle events        |
| allowance          | `allowance`   | Payment allowance events       |
| anomaly_detector   | `anomaly`     | Anomaly detection events       |
| referral           | `referral`    | Referral events                |
| verification       | `verify`      | Credential verification events |
| vesting            | `vesting`     | Vesting schedule events        |
| multisig           | `multisig`    | Multisig operation events      |
| treasury           | `treasury`    | Treasury management events     |
| subscription       | `subscript`   | Subscription lifecycle events  |
| streak_rewards     | `streak`      | Streak reward events           |
| velocity_limits    | `velocity`    | Velocity limit events          |
| upgrade_registry   | `upgrade`     | Upgrade governance events      |
| treasury_analytics | `trs_anlyt`   | Treasury analytics events      |
| subscription_analytics | `sub_anlyt` | Subscription analytics events |
| delegation         | `delegation`  | Delegation events              |
| escrow_factory     | `esc_factory` | Escrow factory events          |
| endorsements       | `endorsmnt`   | Endorsement events             |
| isa                | `isa`         | ISA lifecycle events           |
| rate_limiter       | `rate_limit`  | Rate limiting events           |

## Event Types by Contract

### Escrow (`escrow`)
- `created` — Escrow created for a mentoring session
- `released` — Escrow funds released to mentor
- `auto_released` — Escrow auto-released on timeout
- `disputed` — Escrow disputed by a party
- `resolved` — Dispute resolved
- `refunded` — Escrow refunded to funder
- `partial_rel` — Partial release from escrow
- `admin_rel` — Admin-triggered release
- `stuck_reported` — Stuck escrow reported
- `emergency_release` — Emergency release executed
- `tok_approved` — Token approved for escrow
- `fee_distrib` — Fee distributed from escrow

### Governance (`governance`)
- `prop_created` — New proposal created
- `vote_cast` — Vote cast on proposal
- `prop_passed` — Proposal passed quorum
- `prop_failed` — Proposal failed
- `prop_queued` — Proposal queued for timelock
- `prop_executed` — Proposal executed
- `prop_cancelled` — Proposal cancelled
- `prop_cxl_cd` — Proposal cancelled with cooldown
- `timelock_set` — Timelock period updated
- `call_allowed` — Governance call authorized
- `arb_registered` — Arbiter registered
- `arb_unreg` — Arbiter unregistered
- `appeal_sub` — Appeal submitted
- `appeal_res` — Appeal resolved
- `admin_prop` — Admin change proposed
- `admin_acc` — Admin change accepted

### Staking (`staking`)
- `staked` — Tokens staked
- `unstaked` — Tokens unstaked
- `admin_prop` — Admin change proposed

### Timelock (`timelock`)
- `initialized` — Timelock initialized
- `scheduled` — Operation scheduled
- `executed` — Operation executed
- `cancelled` — Operation cancelled
- `admin_xfr` — Admin transfer initiated
- `em_cancel` — Emergency cancellation
- `guard_set` — Guardian set

### Bounty (`bounty`)
- `posted` — New bounty posted
- `claimed` — Bounty claimed by learner
- `verified` — Bounty completion verified
- `disputed` — Bounty claim disputed
- `refunded` — Bounty refunded to poster

### Delegation (`delegation`)
- `delegated` — Voting power delegated
- `undelegated` — Delegation revoked
- `suspended` — Delegation suspended

### Treasury (`treasury`)
- `tok_approved` — Token approved for treasury
- `tok_rejected` — Token rejected
- `deposited` — Funds deposited
- `allocated` — Funds allocated
- `distributed` — Funds distributed
- `admin_prop` — Admin change proposed
- `admin_acc` — Admin change accepted
- `burn_exec` — Token burn executed
- `auth_added` — Authorized caller added
- `pricing_coord` — Pricing coordination detected
- `rg_resumed` — Rate guard resumed
- `oplog` — Operation logged
- `deposit_from` — Deposit from address
- `alloc_exec` — Allocation executed
- `sched_dist` — Scheduled distribution
- `distrib_exec` — Distribution executed
- `buyback_ok` — Buyback succeeded
- `buyback_fail` — Buyback failed

## Standardized Event Payloads

### AdminChangeProposedEvent

Used by contracts that support admin transfer. Canonical definition in `shared::events`:

```rust
#[contracttype]
pub struct AdminChangeProposedEvent {
    pub contract: Address,
    pub old_admin: Address,
    pub new_admin: Address,
    pub permissive_at: u64,
}
```

This struct replaces the duplicate definitions previously found in 5+ contracts.

### AdminChangeAcceptedEvent

```rust
#[contracttype]
pub struct AdminChangeAcceptedEvent {
    pub contract: Address,
    pub old_admin: Address,
    pub new_admin: Address,
}
```

## Adding New Events

1. Add a variant to the appropriate event type constants in `shared/src/events.rs`
2. Add the emit helper function if the contract doesn't have one yet
3. Add an entry to `events_schema.json` at the workspace root
4. If this is a breaking payload change, increment `EVENT_SCHEMA_VERSION`

## Usage Example

```rust
use shared::events::{emit_escrow_event, evt_escrow_created};

// Emit a standardized escrow event
emit_escrow_event(
    &env,
    evt_escrow_created(&env),
    EscrowCreatedEvent { /* fields */ },
);
```

## Indexer Integration

Indexers should:

1. Filter events by `topic[0]` (contract name) to route to the correct decoder
2. Check `topic[1]` (schema version) to select the correct field definition
3. Use `topic[2]` (event type) to determine the specific event
4. Reject events with unknown schema versions

## Schema Validation

The `topic_is_valid()` helper in `shared::events` can be used in tests to verify events conform to the canonical 3-element layout:

```rust
use shared::events::topic_is_valid;

// In test code
let events = env.events().all();
for event in events.iter() {
    assert!(topic_is_valid(&event.topics, "escrow", &env));
}
```

# MentorsMind Events Documentation

This document provides a comprehensive reference for the standardized event schema used across all MentorsMind Soroban contracts. It serves as the authoritative guide for external contributors, indexer authors, and off-chain systems that need to parse contract events.

## Table of Contents

- [Topic Layout](#topic-layout)
- [Schema Version Policy](#schema-version-policy)
- [Event Types by Contract](#event-types-by-contract)
- [Usage Examples](#usage-examples)
- [Adding New Events](#adding-new-events)
- [Testing and Validation](#testing-and-validation)
- [Related Documentation](#related-documentation)

## Topic Layout

Every event emitted by MentorsMind contracts **must** use exactly the 3-element topic tuple:

```text
(contract: Symbol, version: u32, event_type: Symbol)
```

### Topic Elements

| Position | Field | Type | Description | Example |
|----------|-------|------|-------------|---------|
| 0 | `contract` | `Symbol` | Identifies the originating contract | `"escrow"`, `"governance"` |
| 1 | `version` | `u32` | Schema version (currently `1`) | `1` |
| 2 | `event_type` | `Symbol` | Specific event within the contract | `"created"`, `"released"` |

### Benefits

This standardized layout enables:
- **Parseable without per-contract knowledge**: Indexers can always read `topic[0]` to route to the right decoder
- **Schema versioning**: `topic[1]` allows backward-compatible schema evolution
- **Event identification**: `topic[2]` provides the specific event type for field definition lookup
- **Stable routing**: Contract routing logic remains consistent across all contracts

## Schema Version Policy

### Current Version

```rust
pub const EVENT_SCHEMA_VERSION: u32 = 1;
```

### Version Update Rules

Increment `EVENT_SCHEMA_VERSION` when:
- **Breaking changes** to the topic layout itself
- **Required field changes** that would break existing parsers
- **Payload format changes** that affect indexer compatibility

Do **NOT** increment for:
- Adding new optional fields to event payloads
- Adding new event types to existing contracts
- Adding new contracts with existing event patterns

### Backward Compatibility

- Indexers should **reject events with unknown versions** to prevent parsing errors
- When incrementing the schema version, provide migration documentation
- Test compatibility with existing off-chain systems before deployment

## Event Types by Contract

The following sections document all event types organized by contract. Each contract entry includes:
- Contract symbol used in topic position 0
- All event types with descriptions
- Common payload patterns (where applicable)

### Escrow (`"escrow"`)

The core escrow contract for mentoring session payments.

| Event Type | Description | Typical Payload Fields |
|------------|-------------|----------------------|
| `created` | Escrow created for a mentoring session | `escrow_id`, `mentor`, `learner`, `amount` |
| `released` | Escrow funds released to mentor | `escrow_id`, `amount`, `net_amount`, `platform_fee` |
| `auto_released` | Escrow auto-released on timeout | `escrow_id`, `amount`, `release_time` |
| `disputed` | Escrow disputed by a party | `escrow_id`, `disputer`, `reason` |
| `resolved` | Dispute resolved | `escrow_id`, `resolution`, `arbitrator` |
| `refunded` | Escrow refunded to funder | `escrow_id`, `amount`, `refund_reason` |
| `partial_rel` | Partial release from escrow | `escrow_id`, `amount`, `remaining` |
| `admin_rel` | Admin-triggered release | `escrow_id`, `admin`, `reason` |
| `stuck_reported` | Stuck escrow reported | `escrow_id`, `reporter` |
| `emergency_release` | Emergency release executed | `escrow_id`, `guardian`, `amount` |
| `tok_approved` | Token approved for escrow | `token`, `approved_by` |
| `fee_distrib` | Fee distributed from escrow | `escrow_id`, `platform_fee`, `referral_fee` |

### Governance (`"governance"`)

Proposal creation, voting, and execution events.

| Event Type | Description | Typical Payload Fields |
|------------|-------------|----------------------|
| `prop_created` | New proposal created | `proposal_id`, `proposer`, `description` |
| `vote_cast` | Vote cast on proposal | `proposal_id`, `voter`, `vote_weight`, `in_favor` |
| `prop_passed` | Proposal passed quorum | `proposal_id`, `total_votes`, `votes_for` |
| `prop_failed` | Proposal failed | `proposal_id`, `total_votes`, `votes_for` |
| `prop_queued` | Proposal queued for timelock | `proposal_id`, `execution_time` |
| `prop_executed` | Proposal executed | `proposal_id`, `executor` |
| `prop_cancelled` | Proposal cancelled | `proposal_id`, `canceller` |
| `prop_cxl_cd` | Proposal cancelled with cooldown | `proposal_id`, `canceller`, `cooldown_secs` |
| `timelock_set` | Timelock period updated | `old_delay`, `new_delay` |
| `call_allowed` | Governance call authorized | `target`, `function`, `authorized_by` |
| `arb_registered` | Arbiter registered | `arbiter`, `registered_by` |
| `arb_unreg` | Arbiter unregistered | `arbiter`, `reason` |
| `appeal_sub` | Appeal submitted | `appeal_id`, `appellant`, `target_decision` |
| `appeal_res` | Appeal resolved | `appeal_id`, `resolution`, `resolver` |

### Staking (`"staking"`)

Token staking and delegation events.

| Event Type | Description | Typical Payload Fields |
|------------|-------------|----------------------|
| `staked` | Tokens staked | `staker`, `amount`, `duration` |
| `unstaked` | Tokens unstaked | `staker`, `amount`, `penalty` |
| `admin_prop` | Admin change proposed | `old_admin`, `new_admin`, `effective_at` |
| `admin_acc` | Admin change accepted | `old_admin`, `new_admin` |
| `admin_cancel` | Admin change cancelled | `cancelled_by`, `cancelled_new_admin` |

### Timelock (`"timelock"`)

Operation scheduling and execution events.

| Event Type | Description | Typical Payload Fields |
|------------|-------------|----------------------|
| `initialized` | Timelock initialized | `admin`, `min_delay`, `max_delay` |
| `scheduled` | Operation scheduled | `operation_id`, `target`, `execution_time` |
| `executed` | Operation executed | `operation_id`, `executor` |
| `cancelled` | Operation cancelled | `operation_id`, `canceller` |
| `admin_xfr` | Admin transfer initiated | `old_admin`, `new_admin` |
| `em_cancel` | Emergency cancellation | `operation_id`, `guardian` |
| `guard_set` | Guardian set | `guardian`, `set_by` |

### Bounty (`"bounty"`)

Skill verification and bounty completion events.

| Event Type | Description | Typical Payload Fields |
|------------|-------------|----------------------|
| `posted` | New bounty posted | `bounty_id`, `poster`, `amount`, `skill` |
| `claimed` | Bounty claimed by learner | `bounty_id`, `claimer` |
| `verified` | Bounty completion verified | `bounty_id`, `verifier` |
| `disputed` | Bounty claim disputed | `bounty_id`, `disputer`, `reason` |
| `refunded` | Bounty refunded to poster | `bounty_id`, `amount` |

### Allowance (`"allowance"`)

Payment authorization and pull-payment events.

| Event Type | Description | Typical Payload Fields |
|------------|-------------|----------------------|
| `authorized` | Payment allowance authorized | `payer`, `payee`, `amount`, `token` |
| `payment_pull` | Payment pulled | `payee`, `amount`, `remaining_allowance` |
| `revoked` | Allowance revoked | `payer`, `payee`, `revoked_amount` |

### Anomaly Detection (`"anomaly"`)

Security and anomaly detection events.

| Event Type | Description | Typical Payload Fields |
|------------|-------------|----------------------|
| `hold_placed` | Anomaly hold placed | `target`, `reason`, `severity` |
| `detected` | Anomaly detected | `anomaly_type`, `confidence`, `details` |
| `hold_cleared` | Anomaly hold cleared | `target`, `cleared_by` |

### Additional Contracts

The system includes many additional contracts with their own event types. For complete event type listings for all contracts including:

- **Referral** (`"referral"`) - Referral registration and rewards
- **Verification** (`"verify"`) - Credential verification
- **Vesting** (`"vesting"`) - Token vesting schedules
- **Multisig** (`"multisig"`) - Multi-signature operations
- **Treasury** (`"treasury"`) - Treasury management
- **Subscription** (`"subscript"`) - Subscription lifecycle
- **And many more...**

Refer to the [events_schema.json](../events_schema.json) file for the complete authoritative list.

## Usage Examples

### Rust Contract Implementation

```rust
use shared::events::{emit_escrow_event, evt_escrow_created};

// Emit an escrow creation event
let payload = EscrowCreatedPayload {
    escrow_id: 123,
    mentor: mentor_address,
    learner: learner_address,
    amount: 1000,
};

emit_escrow_event(&env, evt_escrow_created(&env), payload);
```

### Generic Event Emission

For contracts without a dedicated helper:

```rust
use shared::events::emit_generic_event;

emit_generic_event(
    &env,
    Symbol::new(&env, "my_contract"),
    Symbol::new(&env, "custom_event"),
    my_payload
);
```

### Off-chain Event Parsing (Pseudo-code)

```javascript
// Parse standardized event topics
function parseEvent(event) {
    const [contract, version, eventType] = event.topics;
    
    // Route to appropriate handler based on contract
    switch (contract) {
        case "escrow":
            return parseEscrowEvent(eventType, version, event.data);
        case "governance":
            return parseGovernanceEvent(eventType, version, event.data);
        // ... other contracts
    }
}

// Version-aware parsing
function parseEscrowEvent(eventType, version, data) {
    if (version !== 1) {
        throw new Error(`Unsupported schema version: ${version}`);
    }
    
    switch (eventType) {
        case "created":
            return parseEscrowCreated(data);
        case "released":
            return parseEscrowReleased(data);
        // ... other event types
    }
}
```

## Adding New Events

When contributing new events to the MentorsMind contract suite, follow these steps:

### 1. Define the Event Type Symbol

Add your event type to the appropriate section in `contracts/shared/src/events.rs`:

```rust
// Add to the contract-specific section
pub fn evt_my_contract_new_event(env: &Env) -> Symbol { 
    Symbol::new(env, "new_event") 
}
```

### 2. Update events_schema.json

Add the new event to the appropriate contract section in [`events_schema.json`](../events_schema.json):

```json
{
  "contracts": {
    "my_contract": {
      "events": {
        "new_event": { 
          "description": "Description of what this event represents" 
        }
      }
    }
  }
}
```

### 3. Use Standardized Emission

In your contract, use the appropriate emit helper:

```rust
use shared::events::{emit_my_contract_event, evt_my_contract_new_event};

emit_my_contract_event(&env, evt_my_contract_new_event(&env), payload);
```

### 4. Document Payload Structure

If your event has a complex payload, document the expected fields in this file and consider adding a standardized payload struct to `events.rs`.

### 5. Add Tests

Include event emission in your contract tests and verify the topic structure:

```rust
#[test]
fn test_event_emission() {
    // ... setup contract ...
    
    let events = env.events().all();
    let event = events.last().unwrap();
    
    // Verify topic structure
    assert_eq!(event.topics.len(), 3);
    assert_eq!(event.topics.get(0).unwrap(), Symbol::new(&env, "my_contract"));
    assert_eq!(event.topics.get(1).unwrap(), 1u32); // schema version
    assert_eq!(event.topics.get(2).unwrap(), Symbol::new(&env, "new_event"));
}
```

### 6. Schema Version Considerations

Only increment `EVENT_SCHEMA_VERSION` if your changes would break existing parsers. Most additions are backward compatible and don't require version changes.

## Testing and Validation

### Schema Compliance

The `shared::events` module provides test utilities for validating event structure:

```rust
#[cfg(test)]
use shared::events::topic_is_valid;

#[test]
fn test_event_compliance() {
    // ... emit event ...
    let events = env.events().all();
    let event = events.last().unwrap();
    
    assert!(topic_is_valid(&event.topics, "escrow", &env));
}
```

### Event Ordering Tests

For complex workflows, verify event ordering using the patterns documented in [`event_ordering_matrix.md`](../tests/events/event_ordering_matrix.md):

```rust
#[test]
fn test_escrow_lifecycle_events() {
    // Create -> Dispute -> Resolve
    // Verify events are emitted in correct order
    let events = env.events().all();
    assert_eq!(events.get(0).topics.get(2), Symbol::new(&env, "created"));
    assert_eq!(events.get(1).topics.get(2), Symbol::new(&env, "disputed"));
    assert_eq!(events.get(2).topics.get(2), Symbol::new(&env, "resolved"));
}
```

## Related Documentation

- [`events_schema.json`](../events_schema.json) - Complete machine-readable event schema
- [`tests/events/event_ordering_matrix.md`](../tests/events/event_ordering_matrix.md) - Expected event sequences for complex workflows
- [`contracts/shared/src/events.rs`](../contracts/shared/src/events.rs) - Event infrastructure implementation
- [`CONTRIBUTING.md`](../CONTRIBUTING.md) - General contribution guidelines
- [`docs/TESTING.md`](TESTING.md) - Testing strategy and patterns

## Best Practices

### For Contract Developers

1. **Always use standardized emit helpers** instead of calling `env.events().publish()` directly
2. **Keep event payloads focused** - include only data relevant to off-chain indexing
3. **Use descriptive event names** that clearly indicate the state change
4. **Test event emission** as part of your contract test suite
5. **Document complex payloads** with field descriptions and examples

### For Indexer Developers

1. **Always check schema version** before parsing event data
2. **Fail gracefully** on unknown versions or event types
3. **Use topic[0] for routing** to appropriate parsing logic
4. **Implement retry logic** for temporary parsing failures
5. **Validate payload structure** against expected schemas

### For Contributors

1. **Update both events.rs and events_schema.json** when adding events
2. **Follow naming conventions** established by existing events
3. **Add comprehensive tests** including event structure validation
4. **Document breaking changes** clearly in pull requests
5. **Consider off-chain impact** when modifying existing event structures

---

For questions about event schemas or to report issues with event documentation, please open an issue in the main repository.
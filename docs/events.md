# Event Documentation

## Overview

This document provides comprehensive documentation for all events emitted by MentorsMind smart contracts. Events enable off-chain systems to track state changes and maintain synchronized data.

## Table of Contents

1. [Event Structure](#event-structure)
2. [Escrow Events](#escrow-events)
3. [Verification Events](#verification-events)
4. [Upgrade Events](#upgrade-events)
5. [Event Schema](#event-schema)
6. [Event Examples](#event-examples)
7. [Event Monitoring](#event-monitoring)

---

## Event Structure

### Event Format

All events follow this structure:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventData {
    // Event-specific fields
}

// Emitted as:
env.events().publish(
    (topic_symbol,),
    EventData { /* fields */ }
);
```

### Event Components

| Component | Description               | Example                |
| --------- | ------------------------- | ---------------------- |
| Topic     | Event identifier (Symbol) | "EscrowCreated"        |
| Data      | Event payload (struct)    | EscrowCreatedEventData |
| Timestamp | Ledger timestamp          | 1234567890             |
| Contract  | Emitting contract address | CAAAA...               |

---

## Indexing Strategy

### Primary Index Fields

Every escrow event includes `escrow_id` as the primary indexed field. This allows off-chain indexers and backend services to:

- Reconstruct the full lifecycle of any escrow by querying `escrow_id`
- Filter events by participant (`mentor`, `learner`) where present
- Sort events chronologically using the ledger `timestamp`

### Recommended Indexes

| Index | Fields | Use Case |
|---|---|---|
| Primary | `escrow_id` | Fetch all events for a single escrow |
| Mentor | `mentor` + `timestamp` | Mentor dashboard, payment history |
| Learner | `learner` + `timestamp` | Learner dashboard, session history |
| Topic | `topic` + `timestamp` | Event-type analytics, monitoring |
| Token | `token_address` + `timestamp` | Per-asset volume reporting |

### Indexer Integration

Backend services should subscribe to contract events via the Horizon event stream and persist them with the following schema:

```sql
CREATE TABLE contract_events (
    id            BIGSERIAL PRIMARY KEY,
    contract_id   TEXT NOT NULL,
    topic         TEXT NOT NULL,
    escrow_id     BIGINT,          -- indexed, present on all escrow events
    ledger_seq    BIGINT NOT NULL,
    timestamp     BIGINT NOT NULL,
    payload       JSONB NOT NULL,
    created_at    TIMESTAMPTZ DEFAULT NOW()
);

CREATE INDEX idx_events_escrow_id  ON contract_events (escrow_id);
CREATE INDEX idx_events_topic      ON contract_events (topic, timestamp);
CREATE INDEX idx_events_timestamp  ON contract_events (timestamp);
```

### Cursor-Based Polling

Use Horizon's `cursor` parameter to resume event streaming after restarts:

```typescript
server.events()
  .forContract(escrowContractId)
  .cursor(lastProcessedCursor)
  .stream({ onmessage: handleEvent });
```

---

## Escrow Events

### EscrowCreated

**Topic**: `EscrowCreated`

**Emitted When**: New escrow is created

**Data Structure**:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowCreatedEventData {
    pub escrow_id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub session_id: Symbol,
    pub token_address: Address,
    pub session_end_time: u64,
}
```

**Fields**:

- `escrow_id`: Unique escrow identifier (primary index key)
- `mentor`: Address of the mentor
- `learner`: Address of the learner
- `amount`: Escrow amount in stroops
- `session_id`: Unique session identifier
- `token_address`: Token contract address
- `session_end_time`: Unix timestamp when session ends

**Example**:

```json
{
  "topic": "EscrowCreated",
  "data": {
    "escrow_id": 42,
    "mentor": "GAAAA...",
    "learner": "GBBBB...",
    "amount": 1000000000,
    "session_id": "session-123",
    "token_address": "CAAAA...",
    "session_end_time": 1704067200
  },
  "timestamp": 1704067100
}
```

**Use Cases**:

- Track new escrow creation
- Notify learner of session booking
- Update mentor's active sessions
- Trigger payment processing

---

### EscrowReleased

**Topic**: `EscrowReleased`

**Emitted When**: Escrow funds are released to mentor

**Data Structure**:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowReleasedEventData {
    pub escrow_id: u64,
    pub mentor: Address,
    pub amount: i128,
    pub net_amount: i128,
    pub platform_fee: i128,
    pub token_address: Address,
}
```

**Fields**:

- `escrow_id`: Unique escrow identifier (primary index key)
- `mentor`: Address receiving funds
- `amount`: Gross amount released
- `net_amount`: Amount after fees
- `platform_fee`: Fee deducted
- `token_address`: Token contract address

**Example**:

```json
{
  "topic": "EscrowReleased",
  "data": {
    "escrow_id": 42,
    "mentor": "GAAAA...",
    "amount": 1000000000,
    "net_amount": 950000000,
    "platform_fee": 50000000,
    "token_address": "CAAAA..."
  },
  "timestamp": 1704067200
}
```

**Use Cases**:

- Confirm payment to mentor
- Update mentor's balance
- Record transaction for accounting
- Trigger withdrawal notifications

---

### EscrowAutoReleased

**Topic**: `EscrowAutoReleased`

**Emitted When**: Escrow is automatically released after timeout

**Data Structure**:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowAutoReleasedEventData {
    pub escrow_id: u64,
    pub time: u64,
}
```

**Fields**:

- `escrow_id`: Unique escrow identifier (primary index key)
- `time`: Unix timestamp of auto-release

**Example**:

```json
{
  "topic": "EscrowAutoReleased",
  "data": {
    "time": 1704153600
  },
  "timestamp": 1704153600
}
```

**Use Cases**:

- Track auto-release events
- Alert mentor of automatic payment
- Update escrow status
- Trigger follow-up actions

---

### DisputeOpened

**Topic**: `DisputeOpened`

**Emitted When**: Dispute is opened on an escrow

**Data Structure**:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeOpenedEventData {
    pub escrow_id: u64,
    pub caller: Address,
    pub reason: Symbol,
    pub token_address: Address,
}
```

**Fields**:

- `escrow_id`: Unique escrow identifier (primary index key)
- `caller`: Address that opened dispute
- `reason`: Dispute reason (max 500 chars)
- `token_address`: Token contract address

**Example**:

```json
{
  "topic": "DisputeOpened",
  "data": {
    "caller": "GBBBB...",
    "reason": "Session quality was poor",
    "token_address": "CAAAA..."
  },
  "timestamp": 1704067300
}
```

**Use Cases**:

- Alert admin of dispute
- Notify other party
- Trigger dispute resolution workflow
- Record dispute for analytics

---

### DisputeResolved

**Topic**: `DisputeResolved`

**Emitted When**: Dispute is resolved by admin

**Data Structure**:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeResolvedEventData {
    pub escrow_id: u64,
    pub mentor_pct: u32,
    pub mentor_amount: i128,
    pub learner_amount: i128,
    pub token_address: Address,
    pub time: u64,
}
```

**Fields**:

- `escrow_id`: Unique escrow identifier (primary index key)
- `mentor_pct`: Percentage of funds to mentor (0-100)
- `mentor_amount`: Amount paid to mentor
- `learner_amount`: Amount refunded to learner
- `token_address`: Token contract address
- `time`: Resolution timestamp

**Example**:

```json
{
  "topic": "DisputeResolved",
  "data": {
    "mentor_pct": 50,
    "mentor_amount": 500000000,
    "learner_amount": 500000000,
    "token_address": "CAAAA...",
    "time": 1704153600
  },
  "timestamp": 1704153600
}
```

**Use Cases**:

- Confirm dispute resolution
- Process refunds
- Update both parties' balances
- Close dispute case

---

### EscrowRefunded

**Topic**: `EscrowRefunded`

**Emitted When**: Escrow is refunded to learner

**Data Structure**:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowRefundedEventData {
    pub escrow_id: u64,
    pub learner: Address,
    pub amount: i128,
    pub reason: Symbol,
    pub token_address: Address,
}
```

**Fields**:

- `escrow_id`: Unique escrow identifier (primary index key)
- `learner`: Address receiving refund
- `amount`: Refund amount
- `reason`: Refund reason
- `token_address`: Token contract address

**Example**:

```json
{
  "topic": "EscrowRefunded",
  "data": {
    "learner": "GBBBB...",
    "amount": 1000000000,
    "reason": "session_cancelled",
    "token_address": "CAAAA..."
  },
  "timestamp": 1704067400
}
```

**Use Cases**:

- Confirm refund to learner
- Update learner's balance
- Record cancellation
- Trigger refund notifications

---

## Verification Events

### MentorVerified

**Topic**: `MentorVerified` (or `Verify`/`VrfyOk`)

**Emitted When**: Mentor is verified by admin

**Data Structure**:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MentorVerifiedEventData {
    pub credential_hash: BytesN<32>,
    pub verified_at: u64,
    pub expiry: u64,
}
```

**Fields**:

- `credential_hash`: SHA-256 hash of credentials
- `verified_at`: Verification timestamp
- `expiry`: Credential expiry timestamp

**Example**:

```json
{
  "topic": "MentorVerified",
  "data": {
    "credential_hash": "0x1234567890abcdef...",
    "verified_at": 1704067100,
    "expiry": 1735603100
  },
  "timestamp": 1704067100
}
```

**Use Cases**:

- Confirm mentor verification
- Enable mentor to accept sessions
- Update mentor profile status
- Trigger welcome notifications

---

### VerificationRevoked

**Topic**: `VerificationRevoked` (or `Verify`/`Revoke`)

**Emitted When**: Mentor verification is revoked

**Data Structure**:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationRevokedEventData {
    pub revoked: bool,
}
```

**Fields**:

- `revoked`: Always true

**Example**:

```json
{
  "topic": "VerificationRevoked",
  "data": {
    "revoked": true
  },
  "timestamp": 1704153600
}
```

**Use Cases**:

- Disable mentor account
- Prevent new session bookings
- Notify mentor of revocation
- Trigger compliance review

---

## Upgrade Events

### UpgradeRegistered

**Topic**: `UpgradeRegistered`

**Emitted When**: Contract upgrade is registered

**Data Structure**:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UpgradeRegisteredEventData {
    pub contract_name: Symbol,
    pub old_version: u32,
    pub new_version: u32,
    pub timestamp: u64,
}
```

**Fields**:

- `contract_name`: Name of upgraded contract
- `old_version`: Previous version number
- `new_version`: New version number
- `timestamp`: Upgrade timestamp

**Example**:

```json
{
  "topic": "UpgradeRegistered",
  "data": {
    "contract_name": "escrow",
    "old_version": 1,
    "new_version": 2,
    "timestamp": 1704153600
  },
  "timestamp": 1704153600
}
```

**Use Cases**:

- Track version history
- Notify integrators of upgrades
- Trigger compatibility checks
- Update documentation

---

### MigrationEvent

**Topic**: `migrate`

**Emitted When**: Stream is migrated from V1 to V2

**Data Structure**:

```rust
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationEvent {
    pub v1_id: u64,
    pub v2_id: u64,
    pub sender: Address,
    pub remaining_balance: i128,
}
```

**Fields**:

- `v1_id`: V1 stream ID
- `v2_id`: V2 stream ID
- `sender`: Address performing migration
- `remaining_balance`: Remaining stream balance

**Example**:

```json
{
  "topic": "migrate",
  "data": {
    "v1_id": 123,
    "v2_id": 456,
    "sender": "GAAAA...",
    "remaining_balance": 5000000000
  },
  "timestamp": 1704153600
}
```

**Use Cases**:

- Link V1 and V2 stream records
- Update backend database
- Verify migration success
- Track migration history

---

## Event Schema

### JSON Schema

```json
{
  "$schema": "http://json-schema.org/draft-07/schema#",
  "title": "MentorsMind Event",
  "type": "object",
  "required": ["topic", "data", "timestamp", "contract"],
  "properties": {
    "topic": {
      "type": "string",
      "description": "Event topic identifier",
      "enum": [
        "EscrowCreated",
        "EscrowReleased",
        "EscrowAutoReleased",
        "DisputeOpened",
        "DisputeResolved",
        "EscrowRefunded",
        "MentorVerified",
        "VerificationRevoked",
        "UpgradeRegistered",
        "migrate"
      ]
    },
    "data": {
      "type": "object",
      "description": "Event-specific data"
    },
    "timestamp": {
      "type": "integer",
      "description": "Unix timestamp of event"
    },
    "contract": {
      "type": "string",
      "description": "Contract address that emitted event"
    }
  }
}
```

---

## Event Examples

### Complete Escrow Lifecycle

```json
[
  {
    "topic": "EscrowCreated",
    "data": {
      "mentor": "GAAAA...",
      "learner": "GBBBB...",
      "amount": 1000000000,
      "session_id": "session-123",
      "token_address": "CAAAA...",
      "session_end_time": 1704067200
    },
    "timestamp": 1704067100
  },
  {
    "topic": "EscrowReleased",
    "data": {
      "mentor": "GAAAA...",
      "amount": 1000000000,
      "net_amount": 950000000,
      "platform_fee": 50000000,
      "token_address": "CAAAA..."
    },
    "timestamp": 1704067200
  }
]
```

### Disputed Escrow Lifecycle

```json
[
  {
    "topic": "EscrowCreated",
    "data": {
      /* ... */
    },
    "timestamp": 1704067100
  },
  {
    "topic": "DisputeOpened",
    "data": {
      "caller": "GBBBB...",
      "reason": "Session quality was poor",
      "token_address": "CAAAA..."
    },
    "timestamp": 1704067300
  },
  {
    "topic": "DisputeResolved",
    "data": {
      "mentor_pct": 50,
      "mentor_amount": 500000000,
      "learner_amount": 500000000,
      "token_address": "CAAAA...",
      "time": 1704153600
    },
    "timestamp": 1704153600
  }
]
```

---

## Event Monitoring

### Subscribe to Events

```typescript
// Monitor all escrow events
const eventStream = server
  .events()
  .forContract(escrowContractId)
  .cursor("now")
  .stream({
    onmessage: (event) => {
      console.log("Event received:", event);
      handleEscrowEvent(event);
    },
    onerror: (error) => {
      console.error("Event stream error:", error);
    },
  });
```

### Filter Events by Topic

```typescript
function filterEventsByTopic(events: Event[], topic: string): Event[] {
  return events.filter((event) => event.topic === topic);
}

// Get only EscrowCreated events
const createdEvents = filterEventsByTopic(allEvents, "EscrowCreated");
```

### Parse Event Data

```typescript
interface EscrowCreatedEvent {
  mentor: string;
  learner: string;
  amount: bigint;
  session_id: string;
  token_address: string;
  session_end_time: number;
}

function parseEscrowCreatedEvent(event: Event): EscrowCreatedEvent {
  return {
    mentor: event.data.mentor,
    learner: event.data.learner,
    amount: BigInt(event.data.amount),
    session_id: event.data.session_id,
    token_address: event.data.token_address,
    session_end_time: event.data.session_end_time,
  };
}
```

### Event Processing Pipeline

```typescript
async function processEvents(events: Event[]): Promise<void> {
  for (const event of events) {
    try {
      switch (event.topic) {
        case "EscrowCreated":
          await handleEscrowCreated(event);
          break;
        case "EscrowReleased":
          await handleEscrowReleased(event);
          break;
        case "DisputeOpened":
          await handleDisputeOpened(event);
          break;
        case "DisputeResolved":
          await handleDisputeResolved(event);
          break;
        default:
          console.warn(`Unknown event topic: ${event.topic}`);
      }
    } catch (error) {
      console.error(`Error processing event: ${error}`);
      // Log error for manual review
      await logEventError(event, error);
    }
  }
}
```

---

## Support

For event-related questions:

- Review INTEGRATION_GUIDE.md for integration patterns
- Check ERRORS.md for error handling
- See TROUBLESHOOTING.md for common issues
- Contact support@mentorminds.io for assistance

# Type Documentation

Complete reference for all contract types, enums, and structs used across the
MentorsMind Soroban contracts, including field constraints, usage examples, and
type relationships.

---

## Table of Contents

1. [Core Escrow Types](#core-escrow-types)
   - [EscrowStatus](#escrowstatus)
   - [Escrow](#escrow)
   - [MilestoneStatus](#milestonestatus)
   - [MilestoneSpec](#milestonespec)
   - [MilestoneEscrow](#milestoneescrow)
2. [Event Data Types](#event-data-types)
3. [Dispute Evidence Types](#dispute-evidence-types)
   - [EvidenceItem](#evidenceitem)
   - [DisputeResolution](#disputeresolution)
4. [Signature / Meta-Transaction Types](#signature--meta-transaction-types)
5. [State Machine Types](#state-machine-types)
6. [Shared Error Type](#shared-error-type)
7. [Storage Key Enums](#storage-key-enums)
8. [Type Relationships](#type-relationships)

---

## Core Escrow Types

### EscrowStatus

**File:** `escrow/src/lib.rs`

Lifecycle state of an escrow. Transitions are strictly enforced.

```rust
pub enum EscrowStatus {
    Active,
    Released,
    Disputed,
    Refunded,
    Resolved,
}
```

| Variant | Description | Allowed next states |
|---------|-------------|---------------------|
| `Active` | Funds locked; session ongoing | `Released`, `Disputed`, `Refunded` |
| `Released` | Funds paid to mentor (net of fee) | — terminal |
| `Disputed` | Dispute opened; evidence may be submitted | `Resolved` |
| `Refunded` | Full amount returned to learner | — terminal |
| `Resolved` | Dispute resolved by admin arbitration | — terminal |

**Constraints:**
- `release_funds`, `dispute`, and `try_auto_release` require `Active`.
- `resolve_dispute` requires `Disputed`.
- `refund` may be called on `Active` or `Disputed`.

---

### Escrow

**File:** `escrow/src/lib.rs`

Primary data structure representing a single escrow agreement.

```rust
pub struct Escrow {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub session_id: Symbol,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    pub platform_fee: i128,
    pub net_amount: i128,
    pub session_end_time: u64,
    pub auto_release_delay: u64,
    pub dispute_reason: Symbol,
    pub resolved_at: u64,
    pub usd_amount: i128,
    pub quoted_token_amount: i128,
    pub send_asset: Address,
    pub dest_asset: Address,
    pub total_sessions: u32,
    pub sessions_completed: u32,
}
```

| Field | Type | Constraint | Description |
|-------|------|-----------|-------------|
| `id` | `u64` | > 0, auto-incremented | Unique escrow identifier |
| `mentor` | `Address` | | Recipient of released funds |
| `learner` | `Address` | | Funder; can release or dispute |
| `amount` | `i128` | > 0 at creation; ≥ 0 after | Remaining locked amount |
| `session_id` | `Symbol` | ≤ 32 chars | Opaque session identifier |
| `status` | `EscrowStatus` | | Current lifecycle state |
| `created_at` | `u64` | unix seconds | Creation timestamp |
| `token_address` | `Address` | on approved list | SEP-41 payment token |
| `platform_fee` | `i128` | ≥ 0 | Fee paid to treasury; repurposed as learner share in `Resolved` state |
| `net_amount` | `i128` | ≥ 0 | Amount paid to mentor; repurposed as mentor share in `Resolved` state |
| `session_end_time` | `u64` | unix seconds | When session ends; gates auto-release window |
| `auto_release_delay` | `u64` | seconds; default 72 h | Delay after `session_end_time` before permissionless auto-release |
| `dispute_reason` | `Symbol` | ≤ 32 chars; empty until disputed | Short reason supplied when dispute was opened |
| `resolved_at` | `u64` | unix seconds; 0 until resolved | Timestamp of `resolve_dispute` call |
| `usd_amount` | `i128` | ≥ 0 | USD value at creation (0 for non-USD escrows) |
| `quoted_token_amount` | `i128` | ≥ 0 | Token amount at USD-rate creation |
| `send_asset` | `Address` | | Source asset for path-payment escrows |
| `dest_asset` | `Address` | | Destination asset for path-payment escrows |
| `total_sessions` | `u32` | ≥ 1 | Sessions covered by this escrow |
| `sessions_completed` | `u32` | 0 ≤ n ≤ `total_sessions` | Sessions individually released |

---

### MilestoneStatus

**File:** `escrow/src/lib.rs`

```rust
pub enum MilestoneStatus { Pending, Completed, Disputed }
```

---

### MilestoneSpec

**File:** `escrow/src/lib.rs`

```rust
pub struct MilestoneSpec {
    pub description_hash: BytesN<32>,
    pub amount: i128,
}
```

**Constraint:** Sum of all `MilestoneSpec.amount` must equal `MilestoneEscrow.total_amount`.

---

### MilestoneEscrow

**File:** `escrow/src/lib.rs`

Escrow with milestone-based fund release.

```rust
pub struct MilestoneEscrow {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub total_amount: i128,
    pub milestones: Vec<MilestoneSpec>,
    pub milestone_statuses: Vec<MilestoneStatus>,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    pub platform_fee: i128,
    pub net_amount: i128,
}
```

---

## Event Data Types

All event types are `#[contracttype]` emitted via `env.events().publish()`.

| Type | Topics | Description |
|------|--------|-------------|
| `EscrowCreatedEventData` | `("Escrow", "Created", id)` | New escrow created |
| `EscrowReleasedEventData` | `("Escrow", "Released", id)` | Funds released to mentor |
| `EscrowAutoReleasedEventData` | `("Escrow", "AutoReleased", id)` | Permissionless auto-release triggered |
| `DisputeOpenedEventData` | `("Escrow", "DisputeOpened", id)` | Dispute opened by mentor or learner |
| `DisputeResolvedEventData` | `("Escrow", "DisputeResolved", id)` | Dispute resolved with percentage split |
| `EscrowRefundedEventData` | `("Escrow", "Refunded", id)` | Full amount refunded to learner |
| `ReviewSubmittedEventData` | `("Escrow", "ReviewSubmitted", id)` | Learner submitted a post-session review |

### DisputeResolvedEventData

```rust
pub struct DisputeResolvedEventData {
    pub mentor_pct: u32,      // 0–100 awarded to mentor
    pub mentor_amount: i128,  // tokens transferred to mentor
    pub learner_amount: i128, // tokens returned to learner
    pub token_address: Address,
    pub time: u64,
}
```

**Invariant:** `mentor_amount + learner_amount == original escrow.amount`.

---

## Dispute Evidence Types

**File:** `contracts/dispute_evidence/src/lib.rs`

### EvidenceItem

A single piece of off-chain evidence attached to a disputed escrow.

```rust
pub struct EvidenceItem {
    pub submitter: Address,   // mentor or learner
    pub evidence_ref: Symbol, // off-chain reference (IPFS CID, hash)
    pub submitted_at: u64,    // ledger timestamp
}
```

**Constraints:**
- Maximum **5** items per escrow.
- Must be submitted within `WindowSecs` of `session_end_time` (default 48 h).
- Each party must wait `SUBMISSION_COOLDOWN_SECS` (1 h) between submissions.
- Only submittable while escrow status is `Disputed`.

### DisputeResolution

On-chain record written by an arbitrator after reviewing evidence.

```rust
pub struct DisputeResolution {
    pub arbitrator: Address,
    pub release_to_mentor: bool,
    pub note: Symbol,
    pub resolved_at: u64,
}
```

**Constraint:** Arbitrator must wait `MIN_RESOLUTION_DELAY_SECS` (24 h) after the dispute was opened before submitting a resolution.

---

## Signature / Meta-Transaction Types

**File:** `contracts/shared/src/sig_validation.rs`

### MetaTxPayload

```rust
pub struct MetaTxPayload {
    pub contract_id: Address,
    pub nonce: u64,
    pub deadline: u64,
    pub action: MetaTxAction,
    pub params_hash: BytesN<32>,
}
```

| Field | Constraint | Purpose |
|-------|-----------|---------|
| `contract_id` | must equal executing contract | prevents cross-contract replay |
| `nonce` | must equal stored nonce | prevents same-contract replay |
| `deadline` | `now + 60s < deadline ≤ now + 24h` | limits validity window |
| `action` | | prevents cross-function replay |
| `params_hash` | 32-byte commitment | binds signature to exact parameters |

### MetaTxAction

```rust
pub enum MetaTxAction {
    DeployEscrow     = 0,
    ReleaseEscrow    = 1,
    CancelSubscription = 2,
    ClaimVested      = 3,
}
```

---

## State Machine Types

**File:** `contracts/shared/src/state_machine.rs`

### SubscriptionStatus

```
Trial → Active | Cancelled
Active → GracePeriod | Paused | Cancelled
GracePeriod → Active | Expired
Paused → Active | Cancelled
```

### LoanStatus

```
Pending → Active | Cancelled
Active → Repaid | Defaulted
```

### ISAStatus

```
Pending → StudyPeriod → GracePeriod → Repayment → Completed | Defaulted
```

---

## Shared Error Type

**File:** `contracts/shared/src/lib.rs`

| Code | Variant | Typical cause |
|------|---------|---------------|
| 1 | `AlreadyInitialized` | `initialize` called twice |
| 2 | `NotInitialized` | Used before `initialize` |
| 3 | `Unauthorized` | Caller lacks required role |
| 4 | `NotFound` | Record does not exist |
| 5 | `InvalidAmount` | Amount ≤ 0 or out of range |
| 6 | `InvalidState` | Operation invalid for current state |
| 7 | `DuplicateEntry` | Record already exists |
| 8 | `UnsupportedOperation` | Not supported in this configuration |
| 9 | `Overflow` | Arithmetic overflow |
| 10 | `Underflow` | Arithmetic underflow |

---

## Storage Key Enums

### DataKey (Escrow)

| Variant | Storage | Description |
|---------|---------|-------------|
| `Admin` | Persistent | Admin address |
| `Treasury` | Persistent | Treasury receiving fees |
| `FeeBps` | Persistent | Fee in basis points (0–1000) |
| `EscrowCount` | Persistent | Auto-increment counter |
| `AutoRelDelay` | Persistent | Default auto-release delay (seconds) |
| `Escrow(id)` | Persistent | Full `Escrow` struct |
| `ApprovedToken(addr)` | Persistent | Boolean approval flag |

### DataKey (DisputeEvidence)

| Variant | Storage | Description |
|---------|---------|-------------|
| `Admin` | Instance | Admin address |
| `EscrowContract` | Instance | Linked escrow contract |
| `Evidence(id)` | Persistent | `Vec<EvidenceItem>` for escrow |
| `Resolution(id)` | Persistent | `DisputeResolution` for escrow |
| `WindowSecs` | Instance | Evidence submission window (seconds) |
| `LastSubmission(id, addr)` | Persistent | Cooldown tracker per party |
| `DisputeOpenedAt(id)` | Persistent | Timestamp when dispute was opened |
| `CooldownEnabled` | Instance | Toggle for anti-spam cooldown |

---

## Type Relationships

```
Escrow
└── status: EscrowStatus
      Active ──► Released (terminal)
      Active ──► Refunded (terminal)
      Active ──► Disputed
                 └── resolve_dispute(mentor_pct) ──► Resolved (terminal)

DisputeEvidenceContract
├── Evidence(id): Vec<EvidenceItem>      (max 5; cooldown enforced)
└── Resolution(id): DisputeResolution    (written after MIN_RESOLUTION_DELAY)

MetaTxPayload
└── action: MetaTxAction  (binds signature to one operation)
└── params_hash: BytesN<32>  (binds signature to exact call parameters)
```

---

*Last updated: 2026-05-29. See contract source files for canonical type definitions.*

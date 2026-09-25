# Escrow Contract Features

## Escrow Metadata
Supports structured metadata for sessions.

## Escrow Ratings
Mentor/learner reputation tracking and rating system.

---

## Escrow Cancellation

Allows a learner or mentor to cancel an active escrow before the session starts, with a full refund to the learner.

**Policy**
- Only the learner or mentor may cancel.
- Cancellation is only permitted before `session_end_time` (when set).
- An optional per-escrow cancellation deadline can be set by the admin via `set_cancel_deadline`.
- No platform fee is charged — the full escrowed amount is returned to the learner.

**Functions (escrow contract)**
- `cancel_escrow(caller, escrow_id)` — learner or mentor
- `set_cancel_deadline(escrow_id, deadline)` — admin only

**Events**
- `(Escrow, Cancelled, escrow_id)` → `EscrowCancelledEventData { escrow_id, learner, amount, cancelled_by, token_address }`

---

## Multi-Mentor Group Sessions

Supports sessions with multiple mentors, each receiving a proportional share of the net payment.

**Rules**
- At least 2 mentors must be specified.
- `share_bps` values must sum to exactly 10 000 (100%).
- Platform fee is deducted first; net is split proportionally. Rounding dust goes to the last mentor.

**Functions (escrow contract)**
- `create_multi_mentor_escrow(learner, mentors, amount, token, session_id, session_end_time)` — learner
- `release_multi_mentor_escrow(caller, escrow_id)` — learner or admin
- `get_multi_mentor_escrow(escrow_id)` — view

**Events**
- `(Escrow, MMCreated, escrow_id)` → `MultiMentorCreatedEventData`
- `(Escrow, MMReleased, escrow_id)` → `MultiMentorReleasedEventData`

---

## Escrow Insurance

Optional insurance pool that learners can pay into to protect against disputed sessions.

**How It Works**
1. Admin registers the insurance contract via `set_insurance_contract`.
2. Learner calls `pay_insurance_premium(learner, escrow_id, premium_bps)` (1–500 bps of escrow amount).
3. On a dispute resolved in the learner's favour, admin calls `claim` on the insurance contract.
4. Liquidity providers earn 0.1% yield on platform fees via `accrue_yield`.

**Functions (escrow contract)**
- `set_insurance_contract(insurance)` — admin
- `pay_insurance_premium(learner, escrow_id, premium_bps)` — learner
- `get_insurance_contract()` — view

**Functions (insurance contract)**
- `deposit(provider, amount)` / `withdraw(provider, amount)` — liquidity management
- `claim(escrow_id, learner, amount)` — admin, pays learner from pool
- `calculate_premium(escrow_amount, premium_bps)` — view
- `get_coverage_ratio()` — pool health in bps (alert below 500 bps)

---

## Referral Rewards

Incentivises user growth by rewarding referrers with MNT tokens when referred users complete sessions.

**Reward Amounts (base, before multiplier)**
- Mentor referee: 50 MNT
- Learner referee: 20 MNT

**Leaderboard Multipliers:** rank 1–3 → 2×, rank 4–10 → 1.5×, rank 11–50 → 1.25×, else 1×

**Functions (escrow contract)**
- `set_referral_contract(referral)` — admin
- `notify_referral_fulfilled(referee)` — admin, called after successful release
- `get_referral_contract()` — view

**Functions (referral contract)**
- `register_referral(referrer, referee, is_mentor)` — admin
- `fulfill_referral(referee)` — admin, queues reward
- `distribute_from_fee(referrer, platform_fee, reward_bps)` — admin, adds fee share to pending rewards
- `claim_reward(referrer)` — referrer, mints MNT with multiplier applied

**Events**
- `(Referral, Registered, referrer)` → `ReferralRegisteredEventData`
- `(Referral, RewardClaimed, referrer)` → `RewardClaimedEventData`
- `(Referral, FeeReward, referrer)` → `(reward,)`
- `(Escrow, RefFulf)` → `(referee,)`
## Partial Refund

Allows an admin to refund a configurable percentage of the remaining escrowed amount to the learner without fully closing the escrow.

### Function

```rust
pub fn partial_refund(env: Env, escrow_id: u64, refund_bps: u32)
```

### Parameters

| Parameter    | Type  | Description                                                  |
|-------------|-------|--------------------------------------------------------------|
| `escrow_id` | `u64` | ID of the escrow to partially refund                         |
| `refund_bps` | `u32` | Basis points of the remaining amount to refund (1–10 000)   |

### Authorization

Admin only (`require_auth` on the stored admin address).

### Behavior

- `refund_bps = 5000` refunds 50% of the current `escrow.amount` to the learner.
- `refund_bps = 10000` refunds 100% and transitions the escrow to `Refunded`.
- Works on `Active` and `Disputed` escrows.
- No platform fee is deducted from partial refunds.

### Events

Topic: `("prt_rfnd", escrow_id)`  
Data: `(escrow_id, refund_bps, refund_amount, learner, token_address)`

### Error conditions

- `refund_bps` is 0 or > 10 000 → panics `"refund_bps must be between 1 and 10000"`
- Escrow status is not `Active` or `Disputed` → panics `"Escrow must be Active or Disputed for partial refund"`
- Computed refund amount is 0 → panics `"Refund amount is zero"`

---

## Escrow Transfer

Allows transferring an escrow to a different mentor or learner. Both current parties must authorize the transfer.

### Function

```rust
pub fn transfer_escrow(
    env: Env,
    escrow_id: u64,
    new_mentor: Option<Address>,
    new_learner: Option<Address>,
)
```

### Parameters

| Parameter     | Type              | Description                                      |
|--------------|-------------------|--------------------------------------------------|
| `escrow_id`  | `u64`             | ID of the escrow to transfer                     |
| `new_mentor` | `Option<Address>` | New mentor address, or `None` to keep current    |
| `new_learner`| `Option<Address>` | New learner address, or `None` to keep current   |

### Authorization

Both the current `mentor` **and** `learner` must authorize (`require_auth` on both).

### Behavior

- Escrow must be `Active`.
- Updates the `mentor` and/or `learner` fields on the stored escrow.
- Maintains the per-address escrow index lists (`MENTOR_ESCROWS`, `LEARNER_ESCROWS`) so queries remain consistent.
- Passing `None` for either party leaves that party unchanged.

### Events

Topic: `("esc_xfer", escrow_id)`  
Data: `(escrow_id, old_mentor, old_learner, new_mentor, new_learner)`

### Error conditions

- Escrow status is not `Active` → panics `"Escrow must be Active to transfer"`

---

## Auto-Expiration

Prevents indefinite fund lockup by allowing anyone to trigger a full refund on an escrow that has been unclaimed for 1 year (365 days).

### Function

```rust
pub fn expire_escrow(env: Env, escrow_id: u64)
```

### Parameters

| Parameter    | Type  | Description                    |
|-------------|-------|--------------------------------|
| `escrow_id` | `u64` | ID of the escrow to expire     |

### Authorization

**Permissionless** — no `require_auth`. Anyone may call this function once the expiration condition is met.

### Behavior

- Escrow must be `Active`.
- Expiration condition: `current_timestamp >= escrow.created_at + 365 days`.
- On expiration, the full `escrow.amount` is transferred back to the learner.
- Escrow status transitions to `Refunded`.
- The session ID reservation is released.

### Events

Topic: `("expired", escrow_id)`  
Data: `(escrow_id, learner, refund_amount, token_address, timestamp)`

### Error conditions

- Escrow status is not `Active` → panics `"Escrow not active"`
- Expiration time not reached → panics `"Escrow has not expired yet"`

---

## Pause / Resume

Allows either party to pause an escrow for rescheduled sessions. The auto-release deadline is automatically extended by the paused duration when the escrow is resumed.

### Functions

```rust
pub fn pause_escrow(env: Env, caller: Address, escrow_id: u64)
pub fn resume_escrow(env: Env, caller: Address, escrow_id: u64)
pub fn is_paused(env: Env, escrow_id: u64) -> bool
```

### Parameters

| Parameter    | Type      | Description                                  |
|-------------|-----------|----------------------------------------------|
| `caller`    | `Address` | Address of the party pausing/resuming        |
| `escrow_id` | `u64`     | ID of the escrow to pause or resume          |

### Authorization

Either the `mentor` or `learner` of the escrow (`require_auth` on `caller`).

### Behavior

**pause_escrow**
- Escrow must be `Active` and not already paused.
- Records the current timestamp as the pause start time.

**resume_escrow**
- Escrow must be `Active` and currently paused.
- Computes `paused_duration = now - paused_at`.
- Extends `escrow.session_end_time` by `paused_duration`, pushing out the auto-release window.
- Removes the pause record.

**is_paused**
- Returns `true` if the escrow currently has an active pause record.

### Events

Pause — Topic: `("paused", escrow_id)` / Data: `(escrow_id, caller, paused_at)`  
Resume — Topic: `("resumed", escrow_id)` / Data: `(escrow_id, caller, paused_duration, new_session_end_time)`

### Error conditions

**pause_escrow**
- Escrow not `Active` → panics `"Escrow must be Active to pause"`
- Already paused → panics `"Escrow already paused"`
- Caller is not mentor or learner → panics `"Caller not authorized to pause"`

**resume_escrow**
- Escrow not `Active` → panics `"Escrow must be Active to resume"`
- Not paused → panics `"Escrow is not paused"`
- Caller is not mentor or learner → panics `"Caller not authorized to resume"`

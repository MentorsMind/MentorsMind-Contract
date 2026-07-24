# Flash Loan Attack Prevention — Issue #402

## Overview

Flash loan attacks exploit the ability to borrow large amounts of capital within a single transaction (or ledger sequence in Soroban), manipulate prices or balances, and repay before the block closes — leaving the attacker with risk-free profit at the protocol's expense.

This document describes the safeguards implemented across the MentorsMind contracts.

---

## Threat Model

| Attack Vector | Target | Impact |
|---|---|---|
| Same-block deposit + withdraw | Lending pool | Inflate apparent liquidity, manipulate yield calculations |
| Same-block deposit + release/refund | Escrow | Bypass time-lock, extract funds in one transaction |
| Per-block pool drain | Lending pool | Borrow entire pool in one block, leaving nothing for legitimate users |
| Price feed manipulation | Oracle | Inject a single extreme price to trigger false liquidations or skew yield |
| Coordinated feeder spike | Oracle | Multiple compromised feeders push median above TWAP to manipulate collateral values |

---

## Protections Implemented

### 1. Lending Pool (`contracts/lending_pool/src/lib.rs`)

#### Same-Block Deposit/Withdraw Guard

When a lender deposits, the current ledger sequence number is stored:

```rust
env.storage().instance().set(
    &DataKey::LenderDepositLedger(lender),
    &env.ledger().sequence(),
);
```

On withdrawal, the guard checks:

```rust
let deposit_ledger: u32 = env.storage().instance()
    .get(&DataKey::LenderDepositLedger(lender.clone()))
    .unwrap_or(0);
if deposit_ledger == env.ledger().sequence() {
    return Err(Error::SameBlockDepositWithdraw);
}
```

This prevents an attacker from depositing and immediately withdrawing within the same transaction to manipulate the pool's apparent liquidity.

#### Per-Block Borrow Cap

At the start of each new ledger sequence, a snapshot of total pool liquidity is taken. Within that sequence, any single address is capped at **10%** of the snapshot:

```rust
const PER_BLOCK_BORROW_CAP_BPS: i128 = 1_000; // 10%

let per_block_cap = liquidity_snapshot
    .checked_mul(PER_BLOCK_BORROW_CAP_BPS)
    .unwrap_or(i128::MAX)
    .checked_div(10_000)
    .unwrap_or(i128::MAX);
```

The per-address accumulator resets automatically when the ledger sequence advances. This prevents a single actor from draining the pool in one block.

**New query functions:**
- `get_block_borrow_total(borrower)` — returns the cumulative borrow amount for an address in the current block
- `get_liquidity_snapshot()` — returns the reference liquidity used for the current block's cap

**New error codes:**
- `SameBlockDepositWithdraw = 11`
- `PerBlockBorrowLimitExceeded = 12`

---

### 2. Oracle (`contracts/oracle/src/lib.rs`)

#### Time-Weighted Average Price (TWAP)

A rolling TWAP is maintained per asset using the timestamps embedded in each `PricePoint`. The TWAP is recalculated from scratch over the last `TWAP_WINDOW = 5` price points on every submission:

```
TWAP = Σ(price_i × Δt_i) / Σ(Δt_i)
```

where `Δt_i` is the time each price was "active" (gap to the next observation).

**New query functions:**
- `get_twap(asset)` — returns the current TWAP for an asset
- `is_price_manipulated(asset, threshold_bps)` — returns `true` if the spot price (median) deviates from the TWAP by more than `threshold_bps` basis points

#### Circuit Breaker

Every price submission is validated against the current TWAP before being accepted. If the deviation exceeds **50%** (5,000 bps), the submission is rejected:

```rust
const MAX_PRICE_DEVIATION_BPS: i128 = 5_000; // 50%

let deviation_bps = diff
    .checked_mul(10_000)
    .unwrap_or(i128::MAX)
    .checked_div(twap_state.twap)
    .unwrap_or(i128::MAX);
if deviation_bps > MAX_PRICE_DEVIATION_BPS {
    panic!("price deviation exceeds circuit breaker threshold");
}
```

This prevents a single compromised feeder from injecting an extreme price in one update. The circuit breaker only activates once a TWAP has been established (requires at least 2 price points with different timestamps).

**How callers use `is_price_manipulated`:**

Yield contracts, escrow release logic, and liquidation engines should call this before acting on a price:

```rust
// Example: gate yield deployment on oracle health
let manipulated = oracle_client.is_price_manipulated(&asset, &500i128); // 5% threshold
if manipulated {
    panic!("oracle price appears manipulated — yield deployment blocked");
}
```

---

### 3. Escrow (`escrow/src/lib.rs`)

#### Same-Block Deposit/Release/Refund/Dispute Guard

When an escrow is created, the current ledger sequence is recorded:

```rust
const ESCROW_CREATE_LEDGER: Symbol = symbol_short!("ESC_CLED");
const MIN_LEDGERS_BEFORE_ACTION: u32 = 1;

// In create_escrow:
let create_ledger_key = (ESCROW_CREATE_LEDGER, count);
env.storage().persistent().set(&create_ledger_key, &env.ledger().sequence());
```

The guard is checked in `release_funds`, `refund`, and `dispute`:

```rust
let create_ledger: u32 = env.storage().persistent()
    .get(&create_ledger_key)
    .unwrap_or(0);
if create_ledger == env.ledger().sequence() {
    panic!("same-block deposit and release not allowed");
}
```

This prevents an attacker from creating an escrow and immediately releasing, refunding, or disputing it within the same transaction — which could be used to manipulate yield calculations or bypass time-based release conditions.

**New query function:**
- `get_escrow_create_ledger(escrow_id)` — returns the ledger sequence at which the escrow was created

---

## Transaction-Level Balance Tracking

The lending pool now maintains a **per-block liquidity snapshot** (`DataKey::BlockLiquiditySnapshot`) that captures the pool's total liquidity at the start of each ledger sequence. This snapshot is the reference for:

1. Computing the per-block borrow cap (10% of snapshot)
2. Detecting intra-block balance manipulation

The snapshot is refreshed automatically on the first borrow of each new ledger sequence.

---

## Testing

Flash loan attack scenarios are covered in `tests/flash_loan_tests.rs`:

| Test | Scenario |
|---|---|
| `test_lp_same_block_deposit_withdraw_rejected` | Deposit + withdraw in same block → `SameBlockDepositWithdraw` |
| `test_lp_withdraw_succeeds_after_block_advance` | Withdraw in next block → succeeds |
| `test_lp_per_block_borrow_cap_enforced` | Borrow 10% cap → second borrow rejected |
| `test_lp_borrow_cap_resets_next_block` | Cap resets in next ledger sequence |
| `test_lp_multiple_small_borrows_accumulate` | Per-address accumulation tracked correctly |
| `test_lp_get_block_borrow_total_tracks_correctly` | Query function returns correct running total |
| `test_oracle_circuit_breaker_rejects_extreme_deviation` | 200× TWAP price rejected |
| `test_oracle_twap_computed_correctly` | TWAP math verified against known values |
| `test_oracle_is_price_manipulated_detects_spike` | 40% coordinated spike flagged at 30% threshold |
| `test_oracle_gradual_price_movement_not_flagged` | Legitimate ~8% drift not flagged at 15% threshold |
| `test_oracle_circuit_breaker_allows_within_window` | 49% deviation accepted (under 50% limit) |
| `test_lp_block_borrow_total_zero_new_block` | Accumulator resets to 0 in new block |

---

## Configuration

| Parameter | Contract | Value | Description |
|---|---|---|---|
| `PER_BLOCK_BORROW_CAP_BPS` | Lending Pool | 1,000 (10%) | Max borrow per address per block as % of pool snapshot |
| `MAX_PRICE_DEVIATION_BPS` | Oracle | 5,000 (50%) | Max single-update price deviation from TWAP |
| `TWAP_WINDOW` | Oracle | 5 | Number of price points in the TWAP rolling window |
| `MIN_LEDGERS_BEFORE_ACTION` | Escrow | 1 | Minimum ledger sequences between deposit and any fund movement |

All parameters are defined as named constants and can be adjusted by the admin without contract redeployment (for the oracle circuit breaker threshold, via `is_price_manipulated`'s `threshold_bps` argument).

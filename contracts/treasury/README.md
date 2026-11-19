# Treasury Contract

Soroban contract (`mentorminds-treasury`) that custody treasury funds for the MentorMinds platform: deposits, admin-gated allocations, scheduled staker distributions, XLM/MNT buyback-and-burn, dynamic fee collection, and anti-manipulation pricing controls.

## Overview

- Holds whitelisted tokens and pays out via `allocate` / `batch_allocate`, with a multisig threshold routing large allocations to a two-approver pending queue.
- Schedules and executes staker distributions with a minimum interval gate, an announced-schedule commitment flag, and a 4-hour buffer for the staking pattern detector.
- Swaps XLM for MNT on a DEX and burns the received MNT (`buyback_and_burn`), gated by the timelock, an optional price oracle, and a rolling buyback velocity window that can defer large buybacks.
- Enforces fair pricing: admin benchmark rates, price floor/ceiling, max deviation in bps, and rolling-window coordination detection.
- Records every operation in an on-chain operation log with an accompanying `oplog` event, plus distribution receipts and allocation history pages.
- Protections: reentrancy guard per operation, pause guardian, state snapshots with fund-conservation invariants, per-tx amount caps, MEV/rate-limit throttling, and large-transaction regulatory reporting.

## Entry Points

### Initialization / Admin

- `initialize(admin, staking_contract, timelock, pause_guardian) -> Result<(), Error>` — one-time setup; seeds counters, self-address, and the authorized caller list (timelock + admin).
- `propose_admin_change(current_admin, new_admin) -> Result<(), Error>` — starts a timelocked admin transfer after the cooling-off period.
- `accept_admin_change(new_admin) -> Result<(), Error>` — new admin accepts once the timelock has expired.
- `cancel_admin_change(multisig) -> Result<(), Error>` — removes a pending admin transfer.
- `revoke_admin_emergency(new_admin) -> Result<(), Error>` — timelock-authorized emergency admin replacement.
- `set_regulatory_reporting(reporting_address) -> Result<(), Error>` — admin sets the reporting contract for large transactions.
- `add_authorized_caller(admin, new_caller) -> Result<(), Error>` — admin appends an authorized caller.
- `set_approved_token(token_address, approved) -> Result<(), Error>` — admin toggles the token whitelist; emits approval/rejection.
- `set_multisig_threshold(threshold) -> Result<(), Error>` — admin sets the allocation amount requiring a second approval.
- `set_auto_burn_rate(admin, bps) -> Result<(), Error>` — admin sets the auto-burn rate in basis points.
- `admin_resume_rg(admin, lock_name) -> Result<(), Error>` — admin resumes a paused reentrancy-guard lock.

### Deposits

- `deposit(from, token, amount) -> Result<(), Error>` — pulls an approved token into the treasury; rate-limited, fee-evasion checked, logged.
- `collect_fees(from, token, amount, system_load, reputation) -> Result<i128, Error>` — computes a dynamic fee and deposits it, rejecting fee-gaming patterns.
- `distribute_fee_revenue(admin, token, amount, destination) -> Result<(), Error>` — authorized caller transfers collected fee revenue out.

### Allocation

- `allocate(token, recipient, amount) -> Result<(), Error>` — admin payout; amounts above the multisig threshold become pending instead of executing.
- `batch_allocate(requests) -> Result<(), Error>` — pre-validates and executes multiple allocations atomically (all-or-nothing).
- `approve_pending_allocation(approver, pending_id) -> Result<(), Error>` — second approval executes a pending allocation.

### Burn / Buyback

- `execute_burn_queue() -> Result<i128, Error>` — flushes and returns the queued auto-burn amount.
- `buyback_and_burn(xlm_token, mnt_token, dex_contract, xlm_amount, min_mnt_out, dex_iface, oracle_contract, mnt_asset_symbol) -> Result<(), Error>` — timelock-authorized XLM→MNT swap with oracle health checks, slippage guard, velocity deferral, then MNT burn.

### Distribution

- `schedule_staker_distribution(admin, distribution_at) -> Result<(), Error>` — announces the next distribution and notifies the staking contract.
- `set_require_sched_distrib(admin, require) -> Result<(), Error>` — toggles whether distributions must be preceded by a schedule.
- `distribute_to_stakers(token, total_amount) -> Result<(), Error>` — sends funds to the staking contract (90% stakers / 10% LP pool), enforcing interval, schedule, and invariant checks.
- `verify_reward_allocation(total_reward, allocations) -> bool` — validates an allocation vector and records a reward-distribution invariant.
- `monitor_token_flows(distribution_id) -> bool` — checks a receipt's amount against the per-tx distribution cap.
- `audit_distribution_fairness(distribution_id) -> bool` — reports whether a completed distribution stayed within configured bounds.
- `correct_distribution(distribution_id) -> bool` — marks a failed-fairness receipt as not processed for review.

### Pricing / Fairness

- `set_benchmark_rate(admin, token, rate) -> Result<(), Error>` — admin sets the reference market rate for a token.
- `set_pricing_bounds(admin, token, floor, ceiling, max_deviation_bps) -> Result<(), Error>` — admin sets fair-pricing bounds and max deviation (capped by the shared ceiling).
- `validate_market_rates(token, proposed_price) -> Result<MarketRateValidation, Error>` — validates a price against the stored benchmark.
- `protect_pricing_algorithms(caller, token, proposed_price) -> Result<PriceCoordinationFlag, Error>` — appends to a rolling 20-entry price log and scores coordinated price setting.
- `enforce_fair_pricing(token, proposed_price) -> Result<FairPricingResult, Error>` — clamps a price into floor/ceiling and the benchmark deviation band.
- `calculate_platform_fees(amount, system_load, reputation) -> i128` — returns the dynamic fee for an amount.

### Views

- `get_pending_admin_change() -> Option<AdminTransfer>`, `get_admin() -> Result<Address, Error>`
- `get_balance(token) -> i128` — treasury balance of a token.
- `get_pending_allocation(pending_id) -> Option<PendingAllocation>`, `pending_allocation_count() -> u32`
- `get_scheduled_distribution_at() -> Option<u64>`, `get_require_sched_distrib() -> bool`, `get_last_distribution_at() -> Option<u64>`
- `get_distribution_receipt(distribution_id) -> Option<DistributionReceipt>`
- `get_operation_log(op_id) -> Option<TreasuryOperationLog>`, `get_operation_log_count() -> u64`
- `get_last_deferred_buyback() -> Option<BuybackDeferred>`, `get_buyback_window_volume() -> i128`
- `get_history_page(offset, limit) -> Vec<AllocationHistory>` — paged allocation history.
- `get_timelock() -> Address`, `get_staking_contract() -> Result<Address, Error>`
- `rg_is_paused(lock_name) -> bool`

## Storage

| DataKey variant | Purpose |
| --- | --- |
| `NamespaceRoot` | Storage namespace root; also a temporary global request counter |
| `Admin` | Current admin address |
| `Timelock` | Timelock address (auth for buybacks / emergency admin) |
| `StakingContract` | Staking contract receiving distributions |
| `PauseGuardian` | Optional pause guardian address |
| `AllocationCount`, `Allocation(u32)` | Executed allocation history and counter |
| `ApprovedToken(Address)` | Token whitelist flag |
| `RegulatoryReporting` | Reporting contract address |
| `MultisigThreshold` | Amount above which allocations need two approvals |
| `PendingAllocationCount`, `PendingAllocation(u32)`, `AllocationApproval(u32, Address)` | Pending multisig allocations and per-approver votes |
| `PendingAdminTransfer`, `LastAdminChange` | In-flight admin transfer and last change time |
| `AutoBurnRateBps`, `BurnQueue` | Auto-burn rate and queued burn amount |
| `DistributionReceipt(u64)`, `LastDistributionId` | Distribution receipts and counter |
| `OperationLogCount`, `OperationLog(u64)` | On-chain operation log |
| `AuthorizedCallers` | List of callers authorized alongside admin |
| `TreasuryContractSelf` | The contract's own address |
| `LastDistributionAt`, `ScheduledNextDistributionAt`, `RequireScheduledDistribution` | Distribution timing gates |
| `BenchmarkRate(Address)`, `PriceFloor(Address)`, `PriceCeiling(Address)`, `MaxPriceDeviationBps(Address)` | Fair-pricing configuration |
| `RecentPricesForToken(Address)`, `RecentPriceTimestampsForToken(Address)` | Rolling price/coordination log |
| `MevInteractionCount(Address, u32)` | Per-ledger MEV interaction counter (temporary) |
| `TotalDeposits(Address)`, `DepositCount(Address)` | Per-depositor totals for fee-evasion audit |
| `TokenFlowRecord(u64)`, `FairnessAuditRecord(u64)` | Token-flow and fairness-audit records |
| `BuybackWindowStart`, `BuybackWindowVolume`, `BuybackLifetimeVolume` | Buyback velocity accounting |
| `LastDeferredBuyback` | Most recent deferred buyback request |

## Events

- `("admin", "proposed")` — admin change proposed (`AdminChangeProposedEvent`).
- `("admin", "accepted")` — admin change accepted (`AdminChangeAcceptedEvent`).
- `("auth", "added")` — new authorized caller added.
- `("treasury", "tok_appr")` / `("treasury", "tok_rej")` — token whitelisted / rejected (`TreasuryTokenApprovalEvent`).
- `("treasury", "sched")` — staker distribution scheduled at a timestamp.
- `("treasury", "velocity")` — allocation tripped the token-velocity guard.
- `("deposit", from, token)` — deposit amount received.
- `("allocate", recipient, token)` — direct allocation executed (amount).
- `("allocate", "pending")` — allocation queued for multisig approval.
- `("allocate", "executed")` — pending allocation executed on second approval.
- `("batch", "alloc_ok")` — batch allocation completed.
- `("distrib", staking_contract, token)` — distribution executed (amount, id, penalty pool snapshot).
- `("burn", "executed")` — queued burn amount flushed.
- `("buyback", "ok")` / `("buyback", "deferred")` — buyback succeeded / deferred by velocity protection.
- `("buyback", "failed")` — with reason: `invalid_xlm_amount`, `invalid_min_out`, `oracle_stale`, `oracle_cb`, `insufficient_xlm`, `zero_output`, `slippage`.
- `("pricing", "coord")` — price-coordination suspicion flagged with a risk score.
- `("rg", "resumed")` — reentrancy-guard lock resumed by admin.
- `("oplog", op_type, op_id)` — operation log entry (caller, token, amount, success).

## Cross-Contract Dependencies

- **Timelock** — stored at initialize; `require_auth` for `buyback_and_burn` and `revoke_admin_emergency`; seeded into the authorized caller list; read via `get_timelock`.
- **Staking contract** — client trait `StakingCoordinationTrait` (`set_next_distribution_at`, `get_epoch_eligible_total`, `get_penalty_redistribution_pool`); `distribute_to_stakers` invokes `receive_treasury_distribution` and `add_to_lp_reward_pool`.
- **Regulatory reporting** — `record_large_tx` invoked on the `RegulatoryReporting` address for allocations over a 10,000 USD-equivalent threshold (fire-and-forget `try_invoke_contract`).
- **Oracle** — client trait `OracleContractTrait::get_oracle_health`; staleness, feeder-count, and circuit-breaker checks gate buybacks.
- **DEX** — `DexInterface.swap_fn` called with `(token_in, token_out, amount_in, min_out, recipient)`.
- **Tokens (Soroban token interface)** — `transfer`, `balance`, `approve` for deposits/payouts/swaps; MNT `burn` invoked after a successful swap.
- **`shared` crate** — `Validator`, `ReentrancyGuard`, `StateSnapshot`, `AtomicBatch`, pause guard, admin-transfer types, and the pricing/economic/fee helper functions.
- **Pause guardian** — `require_not_paused` checked in `deposit`, `allocate`, `batch_allocate`, and `distribute_to_stakers`.

## Testing

```bash
cargo test -p mentorminds-treasury
```

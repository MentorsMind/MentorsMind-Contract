# Economic Attack Simulations

This document describes the economic attack simulations implemented in `tests/economic_attacks.rs` and the observed security posture.

## Scope

The simulations target the requested attack classes:

1. Flash loan governance manipulation
2. Oracle price manipulation
3. Liquidity drain and price crash
4. Sybil-style reputation/review abuse

## 1) Flash Loan Attack on Governance (SC-32)

### Attack model

An attacker borrows a large amount of MNT, attempts to vote in the same transaction, and repays immediately.

### Simulation

Test: `simulate_flash_loan_governance_attack_blocked_by_snapshot_sc32`

Flow:

1. Create proposal (snapshot captured at proposal creation).
2. Borrow tokens (simulated transfer from lender to attacker).
3. Attempt vote.
4. Repay borrowed tokens.

### Result

The vote reverts because voting power at the proposal snapshot is zero for the attacker account. Post-attack vote weight remains `0`.

### Mitigation verified

Snapshot-based voting power at proposal creation prevents same-transaction flash-loan voting.

## 2) Oracle Manipulation (SC-42)

### Attack model

An attacker submits extreme prices to force bad liquidations.

### Simulation

Test: `simulate_oracle_manipulation_requires_three_compromised_feeders_sc42`

Flow:

1. Configure 5 oracle feeders.
2. Round 1: 2 manipulated feeders submit extreme low prices, 3 honest feeders submit normal prices.
3. Round 2: 3 manipulated feeders submit low prices.

### Result

- With 2 manipulated feeders, median remains honest and liquidation condition is not triggered.
- With 3 manipulated feeders, median is manipulated and liquidation condition is triggered.

### Mitigation verified

Median oracle requires compromising at least 3 of 5 feeders to manipulate the effective price.

## 3) Liquidity Drain / Price Crash (SC-36)

### Attack model

A very large swap drains pool depth and crashes spot price.

### Simulation

Test: `simulate_liquidity_drain_enforces_sc36_fee_floor`

Flow:

1. Simulate a constant-product AMM pool.
2. Execute a large swap (`dx = 900,000` against a `1,000,000 / 1,000,000` pool).
3. Validate major price collapse.
4. Check on-chain fee/discount control by attempting to set escrow fee to `0`.

### Result

- Price crashes significantly under large swap pressure.
- Contract rejects zero-fee update and preserves prior fee.

### Mitigation verified

SC-36 fee floor guard is enforced in escrow fee controls; zero-fee updates are rejected.

## 4) Sybil Attack on Reputation / Self-Review

### Attack model

Create many accounts to submit biased self-reviews.

### Simulation

Test: `simulate_sybil_self_review_attempts_blocked_by_session_verification_requirement`

Flow:

1. Create and release an escrow session.
2. Mentor attempts to review their own session.
3. Multiple sybil addresses attempt review submission.
4. Legitimate learner submits review.

### Result

- Mentor self-review and sybil review attempts revert.
- Only the learner account can submit review on the released escrow.

### Mitigation verified

Session verification requirements in escrow review flow (released session plus learner-only caller) block direct self-review and arbitrary sybil review submissions.

## Summary Matrix

| Attack | Simulation Status | Outcome | Mitigation Status |
|---|---|---|---|
| Flash-loan governance vote | Implemented | Blocked | SC-32 validated |
| Oracle manipulation | Implemented | Needs 3+ compromised feeders | SC-42 validated |
| Liquidity drain / price crash | Implemented | Crash reproducible | SC-36 validated |
| Sybil/self-review | Implemented | Unauthorized reviews blocked | Role-gate validated |

## Notes

- These simulations are deterministic unit/integration tests and are intended to be run in CI.
- Verified in this workspace with `cargo test -p mentorminds-integration-tests --test economic_attacks`.

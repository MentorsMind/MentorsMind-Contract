# Cross-Contract Call Audit

This document lists every `env.invoke_contract` / `token::Client` call site across
the MentorsMind Soroban contracts, with reentrancy risk classification and
mitigation status.

---

## Risk Classification

| Level | Meaning |
|-------|---------|
| **HIGH** | Read state → external call → write state (classic reentrancy window) |
| **MEDIUM** | External call with no mutable state updated after it, but caller-controlled contract |
| **LOW** | Trusted token (Stellar native SAC) or state fully committed before call |

---

## contracts/referral/src/lib.rs

### `claim_reward` — `env.invoke_contract` → leaderboard `get_multiplier`

| Field | Value |
|-------|-------|
| Target | `leaderboard` (stored address, set at init) |
| Entry point | `get_multiplier(referrer)` |
| Call position | After reading `pending`, before writing state |
| Risk | **MEDIUM** — read-only query; leaderboard is admin-set but could be replaced with a malicious contract |
| Mitigation | `ReentrancyGuard::enter` on `claim_reward` blocks re-entry from any path through this call |

### `claim_reward` — `env.invoke_contract` → mnt_token `mint`

| Field | Value |
|-------|-------|
| Target | `mnt_token` (stored address, set at init) |
| Entry point | `mint(referrer, amount)` |
| Call position | After all state writes (CEI applied) |
| Risk | **HIGH** (pre-fix: state written after mint; post-fix: **mitigated**) |
| Mitigation | **Checks-Effects-Interactions applied**: `PendingReward`, `LifetimeClaimed`, and `GlobalMinted` are all cleared/updated **before** `invoke_contract`. `ReentrancyGuard::enter` additionally blocks any re-entrant path. |

### `fulfill_referral` — `env.invoke_contract` → leaderboard `record_referral`

| Field | Value |
|-------|-------|
| Target | `leaderboard` (stored address) |
| Entry point | `record_referral(referrer, count)` |
| Call position | After `info.completed = true` and `PendingReward` is updated |
| Risk | **MEDIUM** — state is committed before the call, but the leaderboard could theoretically callback |
| Mitigation | `ReentrancyGuard::enter` on `fulfill_referral` blocks re-entry |

---

## contracts/treasury/src/lib.rs

### `deposit` — `token::Client::transfer`

| Field | Value |
|-------|-------|
| Target | Whitelisted token contract |
| Entry point | `transfer(from, treasury, amount)` |
| Call position | No mutable treasury state written before or after |
| Risk | **LOW** — inbound transfer only; no treasury state change after the call |
| Mitigation | Token whitelist enforced; no guard needed (no state written after call) |

### `allocate` — `token::Client::transfer`

| Field | Value |
|-------|-------|
| Target | Whitelisted token contract |
| Entry point | `transfer(treasury, recipient, amount)` |
| Call position | Transfer happens before allocation history is written |
| Risk | **MEDIUM** — history write happens after transfer; token could callback |
| Mitigation | `ReentrancyGuard::enter(&env, "allocate")` applied |

### `distribute_to_stakers` — `token::Client::transfer`

| Field | Value |
|-------|-------|
| Target | Whitelisted token contract |
| Entry point | `transfer(treasury, staking_contract, amount)` |
| Risk | **MEDIUM** — external token + subsequent cross-contract call |
| Mitigation | `ReentrancyGuard::enter(&env, "distribute")` applied |

### `distribute_to_stakers` — `env.invoke_contract` → staking `distribute_revenue`

| Field | Value |
|-------|-------|
| Target | `staking_contract` (stored address) |
| Entry point | `distribute_revenue(token, amount)` |
| Call position | After token transfer |
| Risk | **MEDIUM** — staking contract is admin-set; callback possible |
| Mitigation | `ReentrancyGuard` on `distribute_to_stakers` covers this call site |

### `buyback_and_burn` — `token::Client::transfer` (XLM → DEX)

| Field | Value |
|-------|-------|
| Target | Whitelisted XLM token |
| Entry point | `transfer(treasury, dex_contract, xlm_amount)` |
| Risk | **MEDIUM** — DEX is caller-supplied (validated via whitelist check on tokens only) |
| Mitigation | `ReentrancyGuard::enter(&env, "buyback")` applied |

### `buyback_and_burn` — `env.invoke_contract` → DEX `swap`

| Field | Value |
|-------|-------|
| Target | `dex_contract` (caller-supplied parameter) |
| Entry point | `swap(xlm_token, mnt_token, xlm_amount) → i128` |
| Risk | **HIGH** — caller-supplied contract with mutable return value; no pre-validation of DEX address |
| Mitigation | `ReentrancyGuard` blocks re-entry. Slippage guard (`mnt_received < min_mnt_out`) prevents manipulation of output. Consider adding a DEX address whitelist in a future hardening pass. |

### `buyback_and_burn` — `env.invoke_contract` → mnt_token `burn`

| Field | Value |
|-------|-------|
| Target | Whitelisted MNT token |
| Entry point | `burn(treasury, mnt_received)` |
| Risk | **MEDIUM** — mnt_token is whitelisted but could be upgraded |
| Mitigation | `ReentrancyGuard` on `buyback_and_burn` covers this call site |

---

## contracts/staking/src/lib.rs

### `stake` — `token::Client::transfer`

| Field | Value |
|-------|-------|
| Target | `mnt_token` (stored address) |
| Entry point | `transfer(mentor, staking_contract, amount)` |
| Risk | **MEDIUM** |
| Mitigation | `ReentrancyGuard::enter(&env, "stake")` — already applied prior to this audit |

### `unstake` — `token::Client::transfer`

| Field | Value |
|-------|-------|
| Target | `mnt_token` |
| Entry point | `transfer(staking_contract, mentor, amount)` |
| Risk | **HIGH** (pre-existing guard mitigates) — transfer before state removal without guard would be exploitable |
| Mitigation | `ReentrancyGuard::enter(&env, "unstake")` — already applied prior to this audit |

### `claim_rewards` — `token::Client::transfer`

| Field | Value |
|-------|-------|
| Target | Token address supplied by caller |
| Entry point | `transfer(staking_contract, staker, pending)` |
| Risk | **HIGH** — caller-supplied token; `PendingRewards` removed after transfer (pre-existing code) |
| Mitigation | `ReentrancyGuard::enter(&env, "claim_rewards")` — already applied prior to this audit. Note: `PendingRewards` is removed **after** the transfer; recommend applying CEI (remove before transfer) in a future pass. |

---

## Contracts with No External Calls

The following contracts contain no `env.invoke_contract` or `token::Client` calls
and require no reentrancy mitigations:

- `allowance`
- `anomaly_detector`
- `badges`
- `certificates`
- `cert_showcase`
- `credit_score`
- `delegation`
- `dispute_evidence`
- `endorsements`
- `referral_leaderboard`
- `shared` (library crate)

---

## Summary of Changes in This Audit

| Contract | Function | Change |
|----------|----------|--------|
| `referral` | `claim_reward` | CEI applied (state cleared before mint); `ReentrancyGuard` added |
| `referral` | `fulfill_referral` | `ReentrancyGuard` added |
| `treasury` | `allocate` | `ReentrancyGuard` added |
| `treasury` | `distribute_to_stakers` | `ReentrancyGuard` added |
| `treasury` | `buyback_and_burn` | `ReentrancyGuard` added |
| `staking` | `stake`, `unstake`, `claim_rewards` | Guards already present (no change) |

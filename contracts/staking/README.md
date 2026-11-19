# Staking Contract

A Soroban contract (`mentorminds-staking`) that manages MNT token staking for mentors on the MentorMinds platform: locking tokens, assigning multi-factor tiers, distributing treasury and epoch-based revenue rewards with lockups, and monitoring the economic sustainability of the staking pool.

## Overview

- Mentors stake MNT tokens for a lock period; one active stake per mentor, with a minimum staking duration before reward eligibility.
- Tiers (Bronze/Silver/Gold/0) require stake amount plus, when reputation and session_registry are configured, minimum reputation rating and completed-session count.
- Revenue is distributed pro-rata using per-epoch, per-staker snapshots so late deposits cannot dilute earlier stakers; rewards settle into 30-day `RewardLockup` entries with a duration-based multiplier.
- Early unstaking incurs a computed penalty; the penalty pool is redistributed to remaining eligible stakers at the next epoch close.
- Economic-protection layer detects extraction attacks, trading/exit coordination, governance accumulation, and token-velocity risk, and audits sustainability metrics per epoch.
- Disaster-recovery support: state snapshots, multi-signer rollback proposals, and post-upgrade state verification.

## Entry Points

### Initialization / Admin

```rust
initialize(admin: Address, mnt_token: Address, pause_guardian: Option<Address>) -> Result<(), Error>
propose_admin_change(env, new_admin: Address) -> Result<(), Error>   // 48h timelock
accept_admin_change(env, new_admin: Address) -> Result<(), Error>
cancel_admin_change(env, multisig: Address) -> Result<(), Error>
set_pause_guardian(env, guardian: Address) -> Result<(), Error>
set_anomaly_detector(env, detector: Address)
set_bypass_anomaly_check(env, bypass: bool)
set_treasury_contract(env, admin: Address, treasury: Address) -> Result<(), Error>
set_reputation_contract(env, reputation: Address)
set_session_registry_contract(env, session_registry: Address)
set_health_dashboard(env, health_dashboard: Address)
set_tier_requirements(env, admin: Address, requirements: TierRequirements)
set_next_distribution_at(env, admin: Address, timestamp: u64) -> Result<(), Error>
set_emergency_signers(env, admin: Address, signers: Vec<Address>) -> Result<(), Error>
```

### Staking

```rust
stake(env, mentor: Address, amount: i128, lock_period_days: u32) -> Result<(), Error>
register_lp_position(env, lp_holder: Address, lp_token_contract: Address, lp_amount: i128, token_a: Address, token_b: Address) -> Result<(), Error>
```

### Unstaking / Migration

```rust
unstake(env, mentor: Address) -> Result<(), Error>
preview_early_unstake_penalty(env, mentor: Address) -> Result<PenaltyCalculation, Error>
migrate_stakers(env)                        // rebuild staker index from legacy Vec
initiate_fair_exit(env, user: Address) -> Result<(), Error>
verify_migration_integrity(env, user: Address) -> Result<bool, Error>
```

### Rewards & Distribution

```rust
distribute_revenue(env, token: Address, amount: i128)          // snapshot-based, dilution-resistant
distribute_revenue_batch(env, token: Address, amount: i128, offset: u32, limit: u32)  // legacy, current-stake pro-rata
receive_treasury_distribution(env, distribution_id: u64, treasury: Address, token: Address, staker_amount: i128, treasury_timestamp: u64) -> Result<(), Error>
claim_rewards(env, staker: Address, token: Address) -> Result<(), Error>
preview_claimable_rewards(env, staker: Address) -> Result<(i128, i128, i128), Error>  // (claimable, locked, ...)
add_to_lp_reward_pool(env, amount: i128) -> Result<(), Error>
claim_lp_rewards(env, lp_holder: Address, mnt_token: Address) -> Result<(), Error>
```

### Economic Monitoring & Game Theory

```rust
verify_incentive_compatibility(env, mentor: Address) -> IncentiveCompatibilityResult
adjust_game_theory_parameters(env, collusion_score_bps: u32)
monitor_tokenomics_stability(env) -> Result<SustainabilityMetricsData, Error>
detect_extraction_attack(env, staker: Address, amount: i128) -> Result<bool, Error>
detect_trading_coordination(env, staker: Address, _amount: i128) -> Result<bool, Error>
monitor_governance_accumulation(env, staker: Address) -> Result<u32, Error>
audit_token_velocity(env, observed_volume: i128, concentration_bps: u32) -> Result<EconomicVelocityReport, Error>
correlate_threat_vectors(env, governance_risk: u32, economic_risk: u32, technical_risk: u32, social_risk: u32) -> MultiVectorThreatReport
validate_sustainability_metrics(env) -> Result<(), Error>
assess_ecosystem_health(env) -> Result<EcosystemHealthSnapshot, Error>
apply_sustainability_interv(env) -> Result<(), Error>
detect_exit_coordination(env, epoch: u64) -> Result<bool, Error>
calculate_rewards_securely(env, staker: Address, epoch: u64) -> Result<i128, Error>
detect_staking_coordination(env, staker: Address) -> bool
audit_epoch_fairness(env, epoch: u64) -> TokenomicsAuditResult
```

### Disaster Recovery

```rust
snapshot_state(env, admin: Address, snapshot_id: u32) -> Result<(), Error>
verify_post_upgrade_state(env, snapshot_id: u32) -> Result<StateVerificationReport, Error>
propose_rollback(env, proposer: Address, snapshot_id: u32, old_wasm_hash: BytesN<32>) -> Result<u32, Error>
approve_rollback(env, signer: Address, proposal_id: u32) -> Result<(), Error>
rollback_to_snapshot(env, proposal_id: u32) -> Result<(), Error>
```

### Views

```rust
get_admin(env) -> Result<Address, Error>
get_pending_admin_change(env) -> Option<PendingAdminChange>
get_treasury_contract(env) -> Option<Address>
get_tier_requirements(env) -> TierRequirements
get_next_distribution_at(env) -> Option<u64>
get_stake(env, mentor: Address) -> Result<StakeRecord, Error>
get_staking_tier(env, mentor: Address) -> u32
get_staker_count(env) -> u32
get_staker_at(env, index: u32) -> Option<Address>
get_stakers(env, offset: u32, limit: u32) -> Vec<Address>
get_total_staked(env) -> i128
get_current_epoch(env) -> u64
get_epoch_total_staked(env, epoch: u64) -> Option<i128>
get_epoch_reward(env, epoch: u64) -> Option<i128>
get_epoch_staker_snapshot(env, epoch: u64, staker: Address) -> Option<i128>
get_epoch_eligible_total(env, epoch: u64) -> Option<i128>
get_staker_epoch_entry(env, staker: Address) -> Option<u64>
get_penalty_redistribution_pool(env) -> i128
get_treasury_dist_receipt(env, distribution_id: u64) -> Option<TreasuryDistributionReceipt>
is_distribution_processed(env, distribution_id: u64) -> bool
get_treasury_distribution_count(env) -> u64
get_pending_rewards(env, staker: Address) -> i128
get_reward_lockup(env, staker: Address, epoch: u64) -> Option<RewardLockup>
get_total_lifetime_rewards(env, staker: Address) -> i128
get_reward_history_page(env, staker: Address, offset: u32, limit: u32) -> Vec<RewardLockup>
get_reward_multiplier_bps(env, staker: Address) -> u32
verify_lp_position(env, lp_holder: Address) -> bool
lp_staking_tier_boost(env, lp_holder: Address) -> u32
get_stake_snapshot_meta(env, snapshot_id: u32) -> Option<SnapshotMeta>
get_stake_snapshot_index(env) -> Vec<u32>
get_stake_rollback_proposal(env, proposal_id: u32) -> Option<RollbackProposal>
```

## Storage

| Key | Variant(s) | Description |
| --- | --- | --- |
| Namespace | `NamespaceRoot` | Contract-isolated storage namespace root. |
| Admin | `Admin`, `PendingAdmin` | Current admin and pending two-step admin change. |
| Config | `MNTToken`, `PauseGuardian`, `AnomalyDetector`, `BypassAnomalyCheck`, `TreasuryContract`, `ReputationContract`, `SessionRegistryContract`, `HealthDashboard`, `TierRequirements`, `NextScheduledDistributionAt` | Configured contract addresses and parameters. |
| Stakes | `Stake(Address)`, `TotalStaked` | Per-mentor `StakeRecord` and aggregate staked amount. |
| Staker index | `Stakers`, `StakerAt(u32)`, `StakerCount`, `StakerIndex(Address)` | Enumerable staker list and index. |
| Epochs & rewards | `EpochId`, `EpochTotalStaked(u64)`, `EpochReward(u64)`, `StakerEpochEntry(Address)`, `StakerNextClaimEpoch(Address)`, `EpochStakerSnapshot(u64, Address)`, `EpochEligibleTotal(u64)`, `PendingRewards(Address)` | Epoch bookkeeping and pro-rata reward snapshots. |
| Reward lockups | `StakerRewardLockup(Address, u64)`, `StakerLockupSettledUntil(Address)` | Per-epoch `RewardLockup` entries and settlement pointer. |
| Treasury | `ProcessedDistribution(u64)`, `TreasuryDistributionReceipt(u64)`, `TreasuryDistributionCount` | Replay protection and audit receipts for treasury distributions. |
| LP rewards | `LPRewardPool`, `LiquidityProviderRecord(Address)` | LP reward pool balance and per-holder LP position. |
| Pattern detection | `StakerActionLog(Address)`, `StakerActionLogIndex(Address)`, `CollusionSignal(Address)`, `GameTheoryState`, `IncentiveCompatibility(Address)` | Action ring buffers and collusion/game-theory state. |
| Penalties | `PenaltyRedistributionPool` | Accumulated early-unstake penalty pool. |
| Disaster recovery | `StakeSnapshot(u32)`, `StakeSnapshotMeta(u32)`, `StakeSnapshotIndex`, `NextStakeSnapshotId`, `StakeEmergencySigners`, `StakeRollbackProposal(u32)`, `StakeRollbackApproval(u32, Address)`, `StakeRollbackProposalCount` | Snapshots, emergency signers, and rollback proposals. |
| Economic protection | `EpochExtractionRate(u64)`, `SustainabilityMetrics(u64)`, `TradingCoordinationFlags(Address)`, `GovernanceAccumulation(Address)`, `MigrationState(Address)`, `ConcurrentExits(u64)`, `EconomicAnomalyFlags`, `SustainabilityHealthScore`, `LongevityMetrics`, `EcosystemHealthIndicators`, `EconomicVelocityReport`, `MultiVectorThreatReport` | Sustainability metrics, coordination flags, and threat reports. |

## Events

- `staked` — emitted on successful `stake` (mentor, amount, unlock, tier).
- `unstaked` — emitted on successful `unstake` (mentor, amount).
- `admin_prop` / `admin_acc` / `admin_cancel` — two-step admin change lifecycle.
- `treasury` / `set` — treasury contract configured.
- `treasury` / `dist_received` — treasury distribution accepted and processed.
- `revenue_distributed` — epoch closed by `distribute_revenue` (epoch, totals, combined reward, penalty pool).
- `reward` / `claimed` — rewards withdrawn by a staker.
- `reward_lockup` / `created` — a `RewardLockup` was materialised during settlement.
- `Staking` / `RewardAudit` — per-staker share audit emitted by `distribute_revenue_batch`.
- `anomaly_warning` — anomaly detector flagged a stake.
- `pattern_flag` with topic `stake` / `unstake` / `claim` — suspicious-pattern detector fired.
- `penalty` / `early_unstake` — early-unstake penalty applied.
- `lp_pool_funded` — LP reward pool balance increased.
- `economic` / `velocity_risk` — token-velocity audit requires stabilization.
- `threat` / `multi_vector` — multi-vector threat correlation flagged.
- `DR` / `stk_sgns` — emergency signers registered.
- `DR` / `stk_snap` — state snapshot captured.
- `DR` / `srb_prop` / `srb_aprv` / `srb_exec` — rollback proposed, approved, executed.

## Cross-Contract Dependencies

- **MNT token** — token contract stored in `MNTToken`; `token::Client` used for stake transfers in/out and LP reward payouts.
- **Treasury contract** — stored in `TreasuryContract`; its address must match and sign for `receive_treasury_distribution`.
- **Reputation contract** — stored in `ReputationContract`; `get_mentor_rating` is invoked by `compute_tier`.
- **Session registry** — stored in `SessionRegistryContract`; `get_sessions_by_mentor` is invoked by `compute_tier`.
- **Health dashboard** — stored in `HealthDashboard`; receives `report_metric` calls for staking TVL after stake/unstake.
- **Anomaly detector** — stored in `AnomalyDetector`; queried (`check_anomaly`) during staking.
- **Pause guardian** — stored in `PauseGuardian`; consulted by `require_not_paused` before state mutations.
- **LP token contracts** — per-position contracts in `LiquidityProviderRecord`; `token::Client::balance` verifies LP positions.

## Testing

```bash
cargo test -p mentorminds-staking
```

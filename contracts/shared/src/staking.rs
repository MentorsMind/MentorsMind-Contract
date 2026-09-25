//! Shared staking primitives (cross-crate shared between `staking` and `snapshot` contracts).
//!
//! Source-of-truth definition of `StakeRecord` lives here. Both crates MUST
//! import it from this shared crate rather than redefining locally, because
//! Soroban serializes `#[contracttype]` structs positionally by field-order
//! in XDR. Any local re-definition that diverges in field count, field order,
//! or field type will silently produce corrupted values on `from_val` — the
//! exact class of bug this module was extracted to fix (see GitHub issue
//! #646).
//!
//!     StakeRecord
//!     ============
//!     Field order is PART OF THE WIRE FORMAT and MUST NOT CHANGE without a
//!     coordinated redeployment of BOTH contracts together with a migration.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Staking duration & reward constants
// ---------------------------------------------------------------------------

/// Minimum staking duration (in seconds) before a staker becomes eligible
/// for any rewards. Blocks reward-dilution attackers who stake just before
/// a distribution and immediately unstake after claiming.
pub const MIN_STAKING_DURATION_SECS: u64 = 14 * 24 * 60 * 60;

/// Lockup period (in seconds) that a received reward must remain in the
/// contract before it can be withdrawn by the staker. Prevents the
/// stake-claim-unstake same-block attack pattern.
pub const REWARD_LOCKUP_SECS: u64 = 30 * 24 * 60 * 60;

/// Maximum duration (in seconds) at which the reward multiplier reaches
/// its peak. Staking longer than this does not increase the multiplier.
pub const MAX_SCALING_DURATION_SECS: u64 = 365 * 24 * 60 * 60;

/// Minimum reward multiplier (applied once MIN_STAKING_DURATION_SECS has
/// elapsed). Represented in basis points (10000 = 1x).
pub const REWARD_MULTIPLIER_MIN_BPS: u32 = 10_000;

/// Maximum reward multiplier (applied once MAX_SCALING_DURATION_SECS has
/// elapsed). Represented in basis points (30000 = 3x).
pub const REWARD_MULTIPLIER_MAX_BPS: u32 = 30_000;

/// Minimum early-unstaking penalty, in basis points (1000 = 10%). Applies
/// when the staker unstakes just after the minimum duration.
pub const EARLY_UNSTAKE_PENALTY_MIN_BPS: u32 = 1_000;

/// Maximum early-unstaking penalty, in basis points (5000 = 50%). Applies
/// when the staker unstakes immediately after staking.
pub const EARLY_UNSTAKE_PENALTY_MAX_BPS: u32 = 5_000;

/// Number of basis points per whole unit (10000 bps = 100%).
pub const BASIS_POINTS: u32 = 10_000;

/// Maximum number of recent staking operations kept per account for the
/// suspicious-pattern detector.
pub const PATTERN_DETECTION_WINDOW: u32 = 10;

/// Threshold (in seconds) below which a stake→unstake cycle is considered
/// a potential flash-stake dilution attempt.
pub const SUSPICIOUS_CYCLE_THRESHOLD_SECS: u64 = 7 * 24 * 60 * 60;

// ---------------------------------------------------------------------------
// Core shared types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeRecord {
    pub mentor: Address,
    pub amount: i128,
    pub staked_at: u64,
    pub unlock_at: u64,
    pub unlock_cooldown_until: Option<u64>,
    /// Tier of the mentor: 0 = None, 1 = Bronze, 2 = Silver, 3 = Gold.
    ///
    /// Stored as `u32` for alignment with governance/escrow tiers and
    /// future tier enums. The previous snapshot contract originally declared
    /// this as `u8` inside a loop body — that created a silent positional XDR
    /// mismatch whenever tier read the Option discriminant bytes instead and
    /// produced tier = 0 even for Gold mentors.
    pub tier: u32,
}

/// Companion event payload matching the StakeRecord shape, also shared so
/// the staking→governance event consumers reuse the same definition.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakedEventData {
    pub mentor: Address,
    pub amount: i128,
    pub unlock_at: u64,
    pub unlock_cooldown_until: Option<u64>,
    /// Matches `StakeRecord.tier`
    pub tier: u32,
}

// ---------------------------------------------------------------------------
// Snapshot-based reward calculation
// ---------------------------------------------------------------------------

/// Historical snapshot of a single staker's position, captured at a
/// well-defined instant (typically at the beginning of a distribution
/// window). Rewards are computed against these snapshots rather than
/// live stake amounts, so a large deposit made immediately before
/// distribution cannot retroactively dilute rewards already earned.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakingSnapshot {
    /// Timestamp (ledger time) at which this snapshot was taken.
    pub snapshot_at: u64,
    /// The epoch / distribution id this snapshot belongs to.
    pub epoch_id: u64,
    /// Total staked across all participants at snapshot time. Used as the
    /// denominator in pro-rata reward shares.
    pub total_staked: i128,
    /// Per-staker stake amount at the moment of the snapshot. A staker not
    /// present in this map had zero stake (and therefore zero share) for
    /// this epoch.
    pub staker_addresses: Vec<Address>,
    pub staker_amounts: Vec<i128>,
}

/// Per-staker record tracking the rewards earned at each epoch, including
/// the lockup-unlock timestamp that prevents immediate withdrawal.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardLockup {
    /// The epoch this reward entry belongs to.
    pub epoch_id: u64,
    /// Raw reward amount before multiplier scaling.
    pub base_amount: i128,
    /// Final reward amount after applying the duration-based multiplier.
    pub scaled_amount: i128,
    /// The multiplier applied (in basis points). Stored so analytics can
    /// reproduce the scaling offline.
    pub multiplier_bps: u32,
    /// Timestamp at which `scaled_amount` can be claimed / withdrawn.
    pub unlocks_at: u64,
    /// When `true` this entry has already been claimed by the staker.
    pub claimed: bool,
}

/// Result of a penalty calculation: how much to deduct from a departing
/// staker's principal, and how much to redistribute among the remaining
/// long-term participants.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PenaltyCalculation {
    /// Raw number of seconds the stake was active.
    pub staked_duration_secs: u64,
    /// Penalty rate in basis points applied to the principal.
    pub penalty_bps: u32,
    /// Absolute amount deducted from the departing staker's principal.
    pub penalty_amount: i128,
    /// Absolute amount returned to the departing staker after penalty.
    pub returned_amount: i128,
    /// If `true` the staker had not yet reached the minimum-duration bar
    /// and no rewards are due to them.
    pub below_min_duration: bool,
}

/// Flags emitted by the suspicious-pattern detector so off-chain analytics
/// can review risky accounts without blocking legitimate users on-chain.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SuspiciousPatternFlag {
    /// No suspicious behaviour detected.
    None = 0,
    /// Stake → unstake cycle shorter than `SUSPICIOUS_CYCLE_THRESHOLD_SECS`.
    ShortCycle = 1,
    /// Stake size is disproportionate (e.g. > 10% of TotalStaked) when
    /// immediately before a scheduled distribution.
    LargeLateStake = 2,
    /// Account has performed multiple short cycles within the detection
    /// window.
    RepeatedShortCycle = 3,
    /// Stake appeared shortly before a distribution window then fully
    /// unstaked immediately after claiming.
    DistributionSniping = 4,
}

/// Record of a single staking action used by the pattern detector.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakingActionRecord {
    pub action: Symbol, // "stake" | "unstake" | "claim"
    pub timestamp: u64,
    pub amount: i128,
    pub epoch_id: u64,
}

// ---------------------------------------------------------------------------
// Reward multiplier computation
// ---------------------------------------------------------------------------

/// Computes the duration-based reward multiplier in basis points.
///
/// Returns `REWARD_MULTIPLIER_MIN_BPS` (1x) at exactly
/// `MIN_STAKING_DURATION_SECS`, linearly interpolating up to
/// `REWARD_MULTIPLIER_MAX_BPS` (3x) at `MAX_SCALING_DURATION_SECS`.
///
/// Returns `0` if the staked duration is strictly below the minimum,
/// meaning the staker is not yet eligible for rewards.
pub fn compute_reward_multiplier_bps(staked_duration_secs: u64) -> u32 {
    if staked_duration_secs < MIN_STAKING_DURATION_SECS {
        return 0;
    }
    if staked_duration_secs >= MAX_SCALING_DURATION_SECS {
        return REWARD_MULTIPLIER_MAX_BPS;
    }
    let elapsed = staked_duration_secs.saturating_sub(MIN_STAKING_DURATION_SECS);
    let total = MAX_SCALING_DURATION_SECS.saturating_sub(MIN_STAKING_DURATION_SECS);
    if total == 0 {
        return REWARD_MULTIPLIER_MIN_BPS;
    }
    let range = REWARD_MULTIPLIER_MAX_BPS.saturating_sub(REWARD_MULTIPLIER_MIN_BPS);
    let addend = ((elapsed as u128) * (range as u128) / (total as u128)) as u32;
    REWARD_MULTIPLIER_MIN_BPS.saturating_add(addend)
}

// ---------------------------------------------------------------------------
// Early-unstake penalty calculation
// ---------------------------------------------------------------------------

/// Calculates the early-unstake penalty using a decreasing linear schedule.
///
/// - At t = 0 (immediately after staking) penalty is 50%.
/// - At t = MIN_STAKING_DURATION_SECS penalty drops to 10%.
/// - At t ≥ original unlock_at → 0% penalty (normal unstake).
///
/// Penalties are deducted from the staker's principal and redistributed
/// to the remaining long-term stakers via the epoch reward pool.
pub fn compute_early_unstake_penalty(
    staked_at: u64,
    current_time: u64,
    original_unlock_at: u64,
    principal: i128,
) -> PenaltyCalculation {
    let staked_duration_secs = current_time.saturating_sub(staked_at);
    let below_min_duration = staked_duration_secs < MIN_STAKING_DURATION_SECS;

    if current_time >= original_unlock_at || principal <= 0 {
        return PenaltyCalculation {
            staked_duration_secs,
            penalty_bps: 0,
            penalty_amount: 0,
            returned_amount: principal,
            below_min_duration,
        };
    }

    // Linear ramp from EARLY_UNSTAKE_PENALTY_MAX_BPS (t=0) to
    // EARLY_UNSTAKE_PENALTY_MIN_BPS (t=MIN_STAKING_DURATION_SECS).
    // Beyond the minimum but before original_unlock_at we keep the min
    // penalty so there is still a cost to leaving early even after the
    // rewards-eligibility threshold.
    let penalty_bps = if staked_duration_secs >= MIN_STAKING_DURATION_SECS {
        EARLY_UNSTAKE_PENALTY_MIN_BPS
    } else {
        let remaining = MIN_STAKING_DURATION_SECS.saturating_sub(staked_duration_secs);
        let total = MIN_STAKING_DURATION_SECS;
        let range = EARLY_UNSTAKE_PENALTY_MAX_BPS.saturating_sub(EARLY_UNSTAKE_PENALTY_MIN_BPS);
        let addend = ((remaining as u128) * (range as u128) / (total as u128)) as u32;
        EARLY_UNSTAKE_PENALTY_MIN_BPS.saturating_add(addend)
    };

    let penalty_amount =
        ((principal as u128) * (penalty_bps as u128) / (BASIS_POINTS as u128)) as i128;
    let returned_amount = principal.saturating_sub(penalty_amount);

    PenaltyCalculation {
        staked_duration_secs,
        penalty_bps,
        penalty_amount,
        returned_amount,
        below_min_duration,
    }
}

// ---------------------------------------------------------------------------
// Suspicious pattern detection
// ---------------------------------------------------------------------------

/// Lightweight on-chain heuristic that flags staking/unstaking patterns
/// consistent with reward-dilution attacks. The function never reverts —
/// it simply returns a severity flag that is emitted as an event so
/// off-chain tooling can review and (if needed) freeze accounts via the
/// anomaly_detector contract.
pub fn detect_suspicious_pattern(
    env: &Env,
    actions: &Vec<StakingActionRecord>,
    current_stake: i128,
    total_staked: i128,
    next_distribution_at: Option<u64>,
    now: u64,
) -> SuspiciousPatternFlag {
    let stake_sym = action_stake(env);
    let unstake_sym = action_unstake(env);
    let len = actions.len();
    if len < 2 {
        return SuspiciousPatternFlag::None;
    }

    // Count short stake->unstake cycles
    let mut short_cycles: u32 = 0;
    let mut last_stake_idx: Option<u32> = None;

    for i in 0..len {
        let act = actions.get(i).unwrap();
        let is_stake = act.action == stake_sym;
        let is_unstake = act.action == unstake_sym;

        if is_stake {
            last_stake_idx = Some(i);
        } else if is_unstake {
            if let Some(ls_idx) = last_stake_idx {
                let last_stake = actions.get(ls_idx).unwrap();
                let cycle = act.timestamp.saturating_sub(last_stake.timestamp);
                if cycle < SUSPICIOUS_CYCLE_THRESHOLD_SECS {
                    short_cycles = short_cycles.saturating_add(1);
                }
                last_stake_idx = None;
            }
        }
    }

    if short_cycles >= 2 {
        return SuspiciousPatternFlag::RepeatedShortCycle;
    }

    // Distribution sniping: large stake very close to a known distribution
    if let Some(dist_at) = next_distribution_at {
        let until = dist_at.saturating_sub(now);
        let within_window = until < SUSPICIOUS_CYCLE_THRESHOLD_SECS;
        let is_large = total_staked > 0
            && current_stake > 0
            && (current_stake as u128) * 10u128 >= (total_staked as u128); // >=10%
        if within_window && is_large {
            return SuspiciousPatternFlag::DistributionSniping;
        }
        if within_window && is_large && short_cycles >= 1 {
            return SuspiciousPatternFlag::LargeLateStake;
        }
    }

    if short_cycles >= 1 {
        return SuspiciousPatternFlag::ShortCycle;
    }

    SuspiciousPatternFlag::None
}

/// Return the default Symbol constants for the three action types so
/// downstream crates don't have to hardcode strings.
pub fn action_stake(env: &Env) -> Symbol {
    Symbol::new(env, "stake")
}
pub fn action_unstake(env: &Env) -> Symbol {
    Symbol::new(env, "unstake")
}
pub fn action_claim(env: &Env) -> Symbol {
    Symbol::new(env, "claim")
}

/// Applies a basis-point multiplier to an amount. Returns `0` if `bps`
/// is `0` (staker not yet eligible). Safe for i128 principal amounts.
pub fn apply_bps_multiplier(amount: i128, bps: u32) -> i128 {
    if bps == 0 || amount <= 0 {
        return 0;
    }
    ((amount as u128) * (bps as u128) / (BASIS_POINTS as u128)) as i128
}

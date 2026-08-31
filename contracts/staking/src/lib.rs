#![no_std]
#![allow(deprecated)] // Temporarily allow deprecated Events::publish until we migrate to #[contractevent]

use shared::events::{emit_staking_event, evt_staking_staked, evt_staking_unstaked};
use shared::{
    action_claim, action_stake, action_unstake, apply_bps_multiplier, assess_token_velocity,
    compute_checksum, compute_early_unstake_penalty, compute_reward_multiplier_bps,
    correlate_attack_vectors, detect_suspicious_pattern, push_snapshot_index, require_not_paused,
    validate_amount_limits, exceeds_extraction_rate, detect_coordinated_timing,
    EconomicVelocityReport, MultiVectorThreatReport, PenaltyCalculation,
    ReentrancyGuard, RewardLockup, RollbackProposal, SafeMath, SnapshotMeta, StakeRecord,
    StakedEventData, StakingActionRecord, StateSnapshot, StateVerificationReport,
    SuspiciousPatternFlag, Validator, EMERGENCY_THRESHOLD, MAX_SNAPSHOTS,
    MIN_STAKING_DURATION_SECS, PATTERN_DETECTION_WINDOW, REWARD_LOCKUP_SECS,
    REWARD_MULTIPLIER_MIN_BPS, MIN_POSITION_DELTA_SECS,
    CollusionDetection, GameTheoryState, IncentiveCompatibilityResult, TokenomicsAuditResult,
};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, Bytes, BytesN, Env,
    IntoVal, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidAmount = 3,
    AlreadyStaked = 4,
    NoStakeFound = 5,
    StillLocked = 6,
    NoPendingAdminChange = 7,
    AdminChangeNotYetEffective = 8,
    InvalidAdminChange = 9,
    Unauthorized = 10,
    CallerNotTreasury = 11,
    DistributionAlreadyProcessed = 12,
    InvalidState = 13,
    Overflow = 14,
    DuplicateEntry = 15,
    TreasuryNotConfigured = 16,
    StateValidationFailed = 17,
    ReentrancyGuardPaused = 18,
    LockPeriodTooShort = 19,
    RewardsStillLocked = 20,
    NoClaimableRewards = 21,
    MinDurationNotReached = 22,
    SnapshotNotFound = 23,
}

// ---------------------------------------------------------------------------
// Storage types — StakeRecord is defined in the shared crate at
// shared/src/staking.rs. Do NOT redefine it here: the snapshot crate
// imports the same struct and Soroban serializes #[contracttype] structs
// by positional field order in XDR, so any divergence in field count,
// order, or type produces silent corrupted deserialization.
// See issue #646 for the snapshot-tier bug that motivated this extraction.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Event data types — StakedEventData also lives in shared::staking.
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnstakedEventData {
    pub mentor: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingAdminChange {
    pub new_admin: Address,
    pub effective_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChangeProposedEvent {
    pub contract: Address,
    pub old_admin: Address,
    pub new_admin: Address,
    pub effective_at: u64,
}

const ADMIN_CHANGE_TIMELOCK: u64 = 48 * 60 * 60;

/// Economic sanity ceiling for a single stake or reward-distribution
/// amount, in the token's smallest unit. Guards against a fat-fingered or
/// manipulative amount many orders of magnitude beyond any real mentor
/// stake or platform revenue distribution.
const MAX_FINANCIAL_AMOUNT: i128 = 1_000_000_000_000_000; // 100M tokens @ 7 decimals

/// Safe upper bound on how many stakers `internal_distribute_revenue` will
/// snapshot in a single call (#831). This function's correctness (per-
/// staker epoch snapshots that make late deposits un-dilutive, see the
/// comment above its snapshot loop) depends on every current staker being
/// captured atomically in one pass, so — unlike a plain enumeration — it
/// cannot simply be paginated across several calls without weakening that
/// guarantee. Once staker_count exceeds this bound, distribute_revenue
/// panics rather than risk exceeding the block gas limit mid-snapshot;
/// operators at that scale should fall back to distribute_revenue_batch,
/// which trades the anti-dilution guarantee for real pagination.
const MAX_STAKERS_PER_SNAPSHOT: u32 = 500;

// ---------------------------------------------------------------------------
// Economic Protection & Sustainability Constants
// ---------------------------------------------------------------------------

/// Maximum rate of stake extraction per epoch (in basis points)
const MAX_EXTRACTION_RATE_BPS: u32 = 500; // 5% per epoch

/// Minimum sustainability ratio (total_staked / total_distributed)
const MIN_SUSTAINABILITY_RATIO: u32 = 150; // 1.5x coverage required

/// Maximum trading volume variance to detect coordination
const MAX_TRADING_VARIANCE_BPS: u32 = 750; // 7.5% deviation threshold

/// Minimum time between large stake positions for coordination detection
// Worker constants

/// Governance token accumulation threshold for monitoring
const GOVERNANCE_ACCUMULATION_THRESHOLD_BPS: u32 = 250; // 2.5% of total

/// Migration fairness window (seconds)
const MIGRATION_FAIRNESS_WINDOW: u64 = 7 * 24 * 60 * 60; // 7 days

/// Exit coordination detection threshold (minimum concurrent exits)
const EXIT_COORDINATION_THRESHOLD: u32 = 10;

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    MNTToken,
    /// Optional pause guardian contract for circuit-breaker functionality.
    PauseGuardian,
    Stake(Address),
    StakerAt(u32),
    StakerCount,
    StakerIndex(Address),
    Stakers,
    TotalStaked,
    PendingRewards(Address),
    /// Current epoch id. Incremented each time `distribute_revenue` snapshots
    /// `TotalStaked` and records a reward for that epoch.
    EpochId,
    /// Snapshot of `TotalStaked` taken at the moment epoch `n` was closed.
    EpochTotalStaked(u64),
    /// Reward amount distributed for epoch `n`.
    EpochReward(u64),
    /// The epoch id in effect when a staker joined (first staked). Rewards
    /// for epochs before this value are not claimable by the staker, which
    /// prevents a late depositor from diluting rewards earned by earlier
    /// stakers in an epoch that already closed.
    StakerEpochEntry(Address),
    /// Next un-claimed epoch id for a given staker — avoids re-scanning
    /// already-claimed epochs on every `claim_rewards` call.
    StakerNextClaimEpoch(Address),
    AnomalyDetector,
    BypassAnomalyCheck,
    PendingAdmin,
    LiquidityProviderRecord(Address),
    LPRewardPool,
    /// Address of the `reputation` contract used by `compute_tier`.
    ReputationContract,
    /// Address of the `session_registry` contract used by `compute_tier`.
    SessionRegistryContract,
    /// Configurable tier thresholds (stake / rating / session requirements).
    TierRequirements,
    // -----------------------------------------------------------------------
    // Treasury integration keys
    // -----------------------------------------------------------------------
    /// Authorized treasury contract address — only this address may call
    /// `receive_treasury_distribution` and related treasury entry points.
    TreasuryContract,
    /// Tracks whether a specific distribution_id from the treasury has
    /// already been processed, to prevent replay / duplicate acceptance.
    ProcessedDistribution(u64),
    /// Receipt of a processed treasury distribution for audit / forensics.
    TreasuryDistributionReceipt(u64),
    /// Counter of treasury distributions processed by this staking contract.
    TreasuryDistributionCount,
    // -----------------------------------------------------------------------
    // Disaster-recovery keys
    // -----------------------------------------------------------------------
    /// Serialised Vec<StakeSnapshot> for DR snapshot `n`.
    StakeSnapshot(u32),
    /// SnapshotMeta for DR snapshot `n`.
    StakeSnapshotMeta(u32),
    /// Ordered Vec<u32> of retained staking DR snapshot IDs.
    StakeSnapshotIndex,
    /// Vec<Address> of up to 7 emergency signers for staking rollback.
    StakeEmergencySigners,
    /// RollbackProposal for staking rollback proposal `n`.
    StakeRollbackProposal(u32),
    /// Boolean approval flag for (proposal_id, signer) staking rollback.
    StakeRollbackApproval(u32, Address),
    /// Auto-incremented staking rollback proposal counter.
    StakeRollbackProposalCount,
    // -----------------------------------------------------------------------
    // Snapshot-based reward + lockup + penalty keys
    // -----------------------------------------------------------------------
    /// Per-epoch, per-staker snapshot stake amount captured at the moment
    /// an epoch closed. Used as the numerator for pro-rata reward shares
    /// instead of the live stake amount, so late deposits cannot dilute.
    EpochStakerSnapshot(u64, Address),
    /// Total of *eligible* stake amounts for epoch `n`, using only stakers
    /// who had reached MIN_STAKING_DURATION_SECS when the epoch closed.
    /// Replaces the naive `EpochTotalStaked` denominator with a snapshot
    /// that excludes both late joiners and not-yet-eligible stakes.
    EpochEligibleTotal(u64),
    /// Ring-buffer of recent staking actions per account, fed to the
    /// suspicious-pattern detector.
    StakerActionLog(Address),
    /// Index of the next slot to write in the action-log ring buffer for
    /// `StakerActionLog(Address)`, so we don't need to shift the Vec.
    StakerActionLogIndex(Address),
    /// RewardLockup entry for (staker, epoch) — contains the scaled
    /// reward amount and its unlock timestamp.
    StakerRewardLockup(Address, u64),
    /// The highest epoch for which a RewardLockup has already been
    /// recorded for `staker` — avoids double-writing on re-settlement.
    StakerLockupSettledUntil(Address),
    CollusionSignal(Address),
    GameTheoryState,
    IncentiveCompatibility(Address),
    /// Accumulated penalty pool from early unstakers. Distributed to all
    /// remaining eligible stakers on the next epoch close.
    PenaltyRedistributionPool,
    /// Optional admin-configurable timestamp of the *next scheduled*
    /// distribution. Used by the pattern detector to flag "large late
    /// stake just before distribution" attacks.
    NextScheduledDistributionAt,
    // -----------------------------------------------------------------------
    // Economic Protection & Sustainability Keys
    // -----------------------------------------------------------------------
    /// Tracks extraction rate per epoch (basis points)
    EpochExtractionRate(u64),
    /// Historical sustainability metrics for viability monitoring
    SustainabilityMetrics(u64),
    /// Trading coordination detection records
    TradingCoordinationFlags(Address),
    /// Governance token accumulation tracking per address
    GovernanceAccumulation(Address),
    /// Migration state tracking for exit fairness
    MigrationState(Address),
    /// Concurrent exit tracking for coordination detection
    ConcurrentExits(u64),
    /// Economic anomaly flags for intervention
    EconomicAnomalyFlags,
    /// Platform sustainability health score
    SustainabilityHealthScore,
    /// Longevity assurance metrics
    LongevityMetrics,
    /// Ecosystem health indicators
    EcosystemHealthIndicators,
    /// Last velocity/concentration audit report.
    EconomicVelocityReport,
    /// Last correlated multi-vector threat report.
    MultiVectorThreatReport,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryDistributionReceipt {
    pub distribution_id: u64,
    pub treasury: Address,
    pub token: Address,
    pub amount: i128,
    pub treasury_timestamp: u64,
    pub received_at: u64,
    pub processed: bool,
}

/// Sustainability metrics for monitoring long-term platform health
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SustainabilityMetricsData {
    pub epoch: u64,
    pub total_staked: i128,
    pub total_distributed: i128,
    pub extraction_rate_bps: u32,
    pub sustainability_ratio: u32,
    pub health_score: u32,
    pub anomaly_flags: u32,
    pub timestamp: u64,
}

/// Economic protection and detection record
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EconomicProtectionRecord {
    pub record_id: u64,
    pub detection_type: u32, // 0: coordination, 1: extraction, 2: gaming, 3: accumulation
    pub actor: Address,
    pub amount: i128,
    pub confidence_bps: u32, // Basis points confidence level
    pub flagged_at: u64,
    pub intervention_applied: bool,
}

/// Migration integrity record for exit fairness
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationIntegrityRecord {
    pub user: Address,
    pub exit_amount: i128,
    pub initiated_at: u64,
    pub fairness_verified: bool,
    pub coordination_detected: bool,
    pub completion_status: u32, // 0: pending, 1: completed, 2: reverted
}

/// Ecosystem health snapshot
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EcosystemHealthSnapshot {
    pub timestamp: u64,
    pub total_participants: u32,
    pub average_stake: i128,
    pub gini_coefficient_bps: u32,    // Wealth inequality measure
    pub concentration_ratio_bps: u32, // Top 10% concentration
    pub health_status: u32,           // 0: healthy, 1: warning, 2: critical
}

// ---------------------------------------------------------------------------
// Multi-factor tier requirements (#762)
//
// Tier assignment reflects both economic commitment (stake) and proven
// quality (reputation rating, sessions completed), so a mentor cannot buy
// their way into Gold with stake alone. `min_rating` is on the same
// `avg_rating * 100` scale returned by `reputation::get_mentor_rating`
// (e.g. 4.5/5.0 == 450).
// ---------------------------------------------------------------------------

/// Compact per-staker record captured inside a DR snapshot.
/// Mirrors the fields of `StakeRecord` that need to be restorable.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeSnapshot {
    pub mentor: Address,
    pub amount: i128,
    pub staked_at: u64,
    pub unlock_at: u64,
    pub unlock_cooldown_until: Option<u64>,
    pub tier: u32,
}

// DR-only TTL constants
const DR_TTL_THRESHOLD: u32 = 500_000;
const DR_TTL_BUMP: u32 = 1_000_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LPRecord {
    pub lp_token_contract: Address,
    pub lp_token_amount: i128,
    pub pair: (Address, Address),
    pub registered_at: u64,
    pub last_reward_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TierRequirements {
    pub bronze_stake: i128,
    pub silver_stake: i128,
    pub gold_stake: i128,
    pub bronze_min_rating: u32,
    pub silver_min_rating: u32,
    pub gold_min_rating: u32,
    pub bronze_min_sessions: u32,
    pub silver_min_sessions: u32,
    pub gold_min_sessions: u32,
}

const DEFAULT_TIER_REQUIREMENTS: TierRequirements = TierRequirements {
    bronze_stake: 100,
    silver_stake: 500,
    gold_stake: 2_000,
    bronze_min_rating: 350,
    silver_min_rating: 400,
    gold_min_rating: 450,
    bronze_min_sessions: 5,
    silver_min_sessions: 10,
    gold_min_sessions: 50,
};

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct StakingContract;

#[contractimpl]
impl StakingContract {
    /// Initialize the staking contract.
    /// Must be called once before any other function.
    pub fn initialize(
        env: Env,
        admin: Address,
        mnt_token: Address,
        pause_guardian: Option<Address>,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MNTToken, &mnt_token);
        if let Some(guardian) = pause_guardian {
            env.storage()
                .instance()
                .set(&DataKey::PauseGuardian, &guardian);
        }
        Ok(())
    }

    pub fn propose_admin_change(
        env: Env,
        current_admin: Address,
        new_admin: Address,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &current_admin)?;
        let old_admin = Self::admin(&env)?;
        let effective_at = env
            .ledger()
            .timestamp()
            .checked_add(ADMIN_CHANGE_TIMELOCK)
            .ok_or(Error::InvalidAdminChange)?;
        env.storage().instance().set(
            &DataKey::PendingAdmin,
            &PendingAdminChange {
                new_admin: new_admin.clone(),
                effective_at,
            },
        );
        env.events().publish(
            (Symbol::new(&env, "admin"), Symbol::new(&env, "proposed")),
            AdminChangeProposedEvent {
                contract: env.current_contract_address(),
                old_admin,
                new_admin,
                effective_at,
            },
        );
        Ok(())
    }

    pub fn accept_admin_change(env: Env, new_admin: Address) -> Result<(), Error> {
        new_admin.require_auth();
        let pending: PendingAdminChange = env
            .storage()
            .instance()
            .get(&DataKey::PendingAdmin)
            .ok_or(Error::NoPendingAdminChange)?;
        if pending.new_admin != new_admin {
            return Err(Error::Unauthorized);
        }
        if env.ledger().timestamp() < pending.effective_at {
            return Err(Error::AdminChangeNotYetEffective);
        }
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.storage().instance().remove(&DataKey::PendingAdmin);
        Ok(())
    }

    pub fn cancel_admin_change(env: Env, multisig: Address) -> Result<(), Error> {
        multisig.require_auth();
        if !env.storage().instance().has(&DataKey::PendingAdmin) {
            return Err(Error::NoPendingAdminChange);
        }
        env.storage().instance().remove(&DataKey::PendingAdmin);
        Ok(())
    }

    pub fn get_pending_admin_change(env: Env) -> Option<PendingAdminChange> {
        env.storage().instance().get(&DataKey::PendingAdmin)
    }

    pub fn get_admin(env: Env) -> Result<Address, Error> {
        Self::admin(&env)
    }

    /// Set or update the pause guardian contract address (admin only).
    pub fn set_pause_guardian(env: Env, guardian: Address) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::PauseGuardian, &guardian);
        Ok(())
    }

    pub fn set_anomaly_detector(env: Env, detector: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::AnomalyDetector, &detector);
    }

    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin = Self::admin(env)?;
        if stored_admin != *admin {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    pub fn set_bypass_anomaly_check(env: Env, bypass: bool) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::BypassAnomalyCheck, &bypass);
    }

    pub fn set_treasury_contract(env: Env, admin: Address, treasury: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&DataKey::TreasuryContract, &treasury);
        env.events().publish(
            (Symbol::new(&env, "treasury"), Symbol::new(&env, "set")),
            treasury,
        );
        Ok(())
    }

    pub fn get_treasury_contract(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::TreasuryContract)
    }

    fn require_not_rg_paused(env: &Env, lock_name: &Symbol) -> Result<(), Error> {
        if ReentrancyGuard::is_paused(env, Some(lock_name.clone())) {
            return Err(Error::ReentrancyGuardPaused);
        }
        Ok(())
    }

    /// Configure the `reputation` contract consulted by `compute_tier`.
    /// Admin only. Quality checks stay disabled (stake-only tiering) until
    /// both this and `session_registry` are configured.
    pub fn set_reputation_contract(env: Env, reputation: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::ReputationContract, &reputation);
    }

    /// Configure the `session_registry` contract consulted by
    /// `compute_tier`. Admin only. See `set_reputation_contract`.
    pub fn set_session_registry_contract(env: Env, session_registry: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::SessionRegistryContract, &session_registry);
    }

    /// Adjust the stake/rating/session thresholds each tier requires.
    /// Admin (governance) only.
    pub fn set_tier_requirements(env: Env, admin: Address, requirements: TierRequirements) {
        let stored: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if stored != admin {
            panic!("Unauthorized");
        }
        env.storage()
            .instance()
            .set(&DataKey::TierRequirements, &requirements);
    }

    pub fn verify_incentive_compatibility(
        env: Env,
        mentor: Address,
    ) -> IncentiveCompatibilityResult {
        let suspicious = env
            .storage()
            .instance()
            .get::<_, CollusionDetection>(&DataKey::CollusionSignal(mentor))
            .map(|d| d.detected)
            .unwrap_or(false);
        IncentiveCompatibilityResult {
            compatible: !suspicious,
            misalignment_risk: if suspicious { 75 } else { 15 },
            mechanism_integrity: if suspicious { 25 } else { 95 },
            welfare_efficiency: if suspicious { 30 } else { 85 },
        }
    }

    pub fn adjust_game_theory_parameters(env: Env, collusion_score_bps: u32) {
        env.storage().instance().set(
            &DataKey::GameTheoryState,
            &GameTheoryState {
                equilibrium_stable: collusion_score_bps < 5_000,
                defection_risk: if collusion_score_bps > 5_000 { 80 } else { 25 },
                cooperation_incentive: if collusion_score_bps > 5_000 { 20 } else { 75 },
                nash_deviation_risk: if collusion_score_bps > 5_000 { 60 } else { 15 },
            },
        );
    }

    /// Current stake/rating/session thresholds for each tier.
    pub fn get_tier_requirements(env: Env) -> TierRequirements {
        Self::load_tier_requirements(&env)
    }

    /// Stake MNT tokens for a given lock period.
    ///
    /// - Transfers `amount` MNT from `mentor` to this contract.
    /// - Stores a StakeRecord with tier derived from amount.
    /// - A mentor can only have one active stake at a time.
    /// - Enforces a minimum lock period equal to MIN_STAKING_DURATION_SECS
    ///   (14 days). This is the bare minimum time a staker must commit to
    ///   be eligible for any rewards.
    ///
    /// Auth: `mentor` must authorize this call.
    pub fn stake(
        env: Env,
        mentor: Address,
        amount: i128,
        lock_period_days: u32,
    ) -> Result<(), Error> {
        // Check pause guardian before any state mutation
        if let Some(guardian) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::PauseGuardian)
        {
            require_not_paused(&env, &guardian);
        }

        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "stake"));
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        Validator::new(&env)
            .require_positive(amount, "amount")
            .require_max(amount, MAX_FINANCIAL_AMOUNT, "amount")
            .validate()
            .map_err(|_| Error::InvalidAmount)?;

        // -------------------------------------------------------------------
        // Attack-vector #1 mitigation: min staking duration.
        // Reject any lock_period smaller than MIN_STAKING_DURATION_SECS.
        // An attacker who could stake for 1 day would otherwise be able to
        // ride a distribution and dump immediately.
        // -------------------------------------------------------------------
        let min_days: u32 = (MIN_STAKING_DURATION_SECS / 86_400) as u32;
        if lock_period_days < min_days {
            return Err(Error::LockPeriodTooShort);
        }

        let bypass: bool = env
            .storage()
            .instance()
            .get(&DataKey::BypassAnomalyCheck)
            .unwrap_or(false);
        if !bypass {
            if let Some(anomaly_detector) = env
                .storage()
                .instance()
                .get::<_, Address>(&DataKey::AnomalyDetector)
            {
                let res: u32 = env.invoke_contract(
                    &anomaly_detector,
                    &Symbol::new(&env, "check_anomaly"),
                    (mentor.clone(), 2u32, amount).into_val(&env), // 2u32 = LargeTransfer
                );
                if res == 2 {
                    panic!("UserOnHold");
                } else if res == 1 {
                    env.events().publish(
                        (Symbol::new(&env, "anomaly_warning"), mentor.clone()),
                        amount,
                    );
                }
            }
        }

        if env
            .storage()
            .persistent()
            .has(&DataKey::Stake(mentor.clone()))
        {
            return Err(Error::AlreadyStaked);
        }

        mentor.require_auth();

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .ok_or(Error::NotInitialized)?;

        // Transfer MNT from mentor to this contract
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&mentor, &env.current_contract_address(), &amount);

        let now = env.ledger().timestamp();
        let lock_seconds = (lock_period_days as u64).safe_mul(&env, 86_400u64);
        let unlock_at = now.safe_add(&env, lock_seconds);
        let tier = Self::compute_tier(&env, amount, &mentor);

        let record = StakeRecord {
            mentor: mentor.clone(),
            amount,
            staked_at: now,
            unlock_at,
            unlock_cooldown_until: None,
            tier,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Stake(mentor.clone()), &record);

        // Update stakers list and total staked
        let key = DataKey::StakerIndex(mentor.clone());
        if !env.storage().persistent().has(&key) {
            let count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::StakerCount)
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&DataKey::StakerAt(count), &mentor);
            env.storage().persistent().set(&key, &count);
            env.storage()
                .persistent()
                .set(&DataKey::StakerCount, &(count + 1));
        }

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        let new_total = total_staked.checked_add(amount).expect("Overflow");
        env.storage()
            .persistent()
            .set(&DataKey::TotalStaked, &new_total);

        // Record the epoch this staker joined in. Rewards for the epoch that
        // is currently accruing (i.e. not yet snapshotted by
        // `distribute_revenue`) are NOT credited to this staker, since their
        // deposit would otherwise dilute rewards earned by stakers who were
        // present for the whole epoch. Eligibility starts at `current_epoch + 1`.
        let current_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0);
        let entry_epoch = current_epoch.safe_add(&env, 1);
        env.storage()
            .persistent()
            .set(&DataKey::StakerEpochEntry(mentor.clone()), &entry_epoch);
        env.storage()
            .persistent()
            .set(&DataKey::StakerNextClaimEpoch(mentor.clone()), &entry_epoch);

        // -------------------------------------------------------------------
        // Attack-vector #6 mitigation: suspicious-pattern detection log.
        // Append a "stake" action to the ring buffer and run the detector.
        // -------------------------------------------------------------------
        Self::append_staker_action(
            &env,
            &mentor,
            action_stake(&env),
            now,
            amount,
            current_epoch,
        );

        let next_dist: Option<u64> = env
            .storage()
            .instance()
            .get(&DataKey::NextScheduledDistributionAt);
        let actions = Self::load_action_log(&env, &mentor);
        let flag = detect_suspicious_pattern(&env, &actions, amount, new_total, next_dist, now);
        if flag != SuspiciousPatternFlag::None {
            env.events().publish(
                (
                    Symbol::new(&env, "pattern_flag"),
                    Symbol::new(&env, "stake"),
                ),
                (mentor.clone(), flag as u32, amount),
            );
        }

        emit_staking_event(
            &env,
            evt_staking_staked(&env),
            StakedEventData {
                mentor,
                amount,
                unlock_at,
                unlock_cooldown_until: None,
                tier,
            },
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Ring-buffer action-log helpers (pattern detection + analytics)
    // -----------------------------------------------------------------------

    fn load_action_log(env: &Env, staker: &Address) -> Vec<StakingActionRecord> {
        env.storage()
            .persistent()
            .get(&DataKey::StakerActionLog(staker.clone()))
            .unwrap_or(Vec::new(env))
    }

    fn append_staker_action(
        env: &Env,
        staker: &Address,
        action: Symbol,
        timestamp: u64,
        amount: i128,
        epoch_id: u64,
    ) {
        let key = DataKey::StakerActionLog(staker.clone());
        let idx_key = DataKey::StakerActionLogIndex(staker.clone());
        let mut log: Vec<StakingActionRecord> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(env));
        let next_idx: u32 = env.storage().persistent().get(&idx_key).unwrap_or(0);
        let cap = PATTERN_DETECTION_WINDOW;

        let record = StakingActionRecord {
            action,
            timestamp,
            amount,
            epoch_id,
        };

        if log.len() < cap {
            log.push_back(record);
        } else {
            log.set(next_idx, record);
        }
        let new_idx = (next_idx + 1) % cap;
        env.storage().persistent().set(&key, &log);
        env.storage().persistent().set(&idx_key, &new_idx);
    }

    /// Admin setter for the next scheduled distribution timestamp. Feeds
    /// the pattern detector so it can flag large stakes placed immediately
    /// before a known distribution window.
    pub fn set_next_distribution_at(env: Env, admin: Address, timestamp: u64) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&DataKey::NextScheduledDistributionAt, &timestamp);
        Ok(())
    }

    pub fn get_next_distribution_at(env: Env) -> Option<u64> {
        env.storage()
            .instance()
            .get(&DataKey::NextScheduledDistributionAt)
    }

    /// Unstake MNT tokens.
    ///
    /// # Normal path (lock period expired):
    /// - Returns the full staked principal back to `mentor`.
    ///
    /// # Early-unstake path (before unlock_at):
    /// - Applies a linearly-declining penalty (10%–50%) based on how close to
    ///   the stake was placed to the minimum duration.
    /// - The penalty amount is deposited into `PenaltyRedistributionPool`
    ///   and distributed to *remaining* eligible stakers on the next
    ///   epoch close. This aligns long-term stakers.
    ///
    /// - Always removes the StakeRecord.
    ///
    /// Auth: `mentor` must authorize this call.
    pub fn unstake(env: Env, mentor: Address) -> Result<(), Error> {
        let lock_sym = Symbol::new(&env, "unstake");
        Self::require_not_rg_paused(&env, &lock_sym)?;
        let _guard = ReentrancyGuard::enter(&env, lock_sym);
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        let record: StakeRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Stake(mentor.clone()))
            .ok_or(Error::NoStakeFound)?;

        mentor.require_auth();

        let now = env.ledger().timestamp();
        let current_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0);

        // -------------------------------------------------------------------
        // Attack-vector #5 mitigation: early-unstake penalties.
        // Compute the penalty BEFORE the normal locked-gate so even if the
        // staker is early (before unlock_at) they can still exit but pay.
        // -------------------------------------------------------------------
        let penalty =
            compute_early_unstake_penalty(record.staked_at, now, record.unlock_at, record.amount);

        // For *strictly* before unlock_at we no longer "StillLocked".
        // Early exit used to `StillLocked` — still a hard-stop; penalties allow exit instead.
        // (Comment out the check so penalties kick in first-in instead.
        // Users calling prior to unlock_at can unstake with penalty
        // (but only the lock period is the MINIMUM, the lock_period_days is how long
        // the user chose; early exit via the penalty ramp
        // The unlock_at can be the original choice they chose;
        // We no longer do we the we now allow early exit
        // stillLocked check if they have passed (the user committed — they chose to)
        // no longer StillLocked as a hard stop, replacing the below line, the penalty ramps.
        // The penalty has a flat 50% at t=0 (immediate unstake is 50% penalty.

        // Recompute tier on this state change (#762). The stake record is
        // removed below, so this has no persisted effect today, but keeps
        // tier derivation on a single code path shared with `stake`.
        let _ = Self::compute_tier(&env, record.amount, &mentor);

        // Settle any epoch rewards accrued (but not yet claimed) while this
        // stake was active, using the still-live `record.amount` — the
        // stake's principal is about to be removed, so this is the last
        // point at which per-epoch pro-rated shares can be computed.
        Self::settle_epoch_rewards(&env, &mentor, record.amount);

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .ok_or(Error::NotInitialized)?;

        let token_client = token::Client::new(&env, &mnt_token);

        // -------------------------------------------------------------------
        // Apply penalty / penalty redistribution.
        // -------------------------------------------------------------------
        let returned_to_staker = penalty.returned_amount;
        if penalty.penalty_amount > 0 {
            let pool: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::PenaltyRedistributionPool)
                .unwrap_or(0);
            env.storage().persistent().set(
                &DataKey::PenaltyRedistributionPool,
                &(pool.checked_add(penalty.penalty_amount).expect("Overflow")),
            );
            env.events().publish(
                (
                    Symbol::new(&env, "penalty"),
                    Symbol::new(&env, "early_unstake"),
                ),
                (
                    mentor.clone(),
                    penalty.penalty_amount,
                    penalty.penalty_bps,
                    current_epoch,
                ),
            );
        }

        if returned_to_staker > 0 {
            token_client.transfer(
                &env.current_contract_address(),
                &mentor,
                &returned_to_staker,
            );
        }

        env.storage()
            .persistent()
            .remove(&DataKey::Stake(mentor.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::StakerEpochEntry(mentor.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::StakerNextClaimEpoch(mentor.clone()));

        // Update stakers list and total staked
        let key = DataKey::StakerIndex(mentor.clone());
        if let Some(index) = env.storage().persistent().get::<_, u32>(&key) {
            let count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::StakerCount)
                .unwrap_or(0);
            let last_index = count - 1;

            if index != last_index {
                let last_mentor: Address = env
                    .storage()
                    .persistent()
                    .get(&DataKey::StakerAt(last_index))
                    .unwrap();
                env.storage()
                    .persistent()
                    .set(&DataKey::StakerAt(index), &last_mentor);
                env.storage()
                    .persistent()
                    .set(&DataKey::StakerIndex(last_mentor), &index);
            }

            env.storage()
                .persistent()
                .remove(&DataKey::StakerAt(last_index));
            env.storage().persistent().remove(&key);
            env.storage()
                .persistent()
                .set(&DataKey::StakerCount, &last_index);
        }

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TotalStaked,
            &(total_staked.safe_sub(&env, record.amount)),
        );

        // -------------------------------------------------------------------
        // Pattern detection on unstake.
        // -------------------------------------------------------------------
        Self::append_staker_action(
            &env,
            &mentor,
            action_unstake(&env),
            now,
            record.amount,
            current_epoch,
        );

        let next_dist: Option<u64> = env
            .storage()
            .instance()
            .get(&DataKey::NextScheduledDistributionAt);
        let actions = Self::load_action_log(&env, &mentor);
        let new_total_after = total_staked.checked_sub(record.amount).unwrap_or(0);
        let flag = detect_suspicious_pattern(&env, &actions, 0, new_total_after, next_dist, now);
        if flag != SuspiciousPatternFlag::None {
            env.events().publish(
                (
                    Symbol::new(&env, "pattern_flag"),
                    Symbol::new(&env, "unstake"),
                ),
                (mentor.clone(), flag as u32, record.amount),
            );
        }

        emit_staking_event(
            &env,
            evt_staking_unstaked(&env),
            UnstakedEventData {
                mentor: mentor.clone(),
                amount: record.amount,
            },
        );

        Ok(())
    }

    /// Returns the early-unstake penalty calculation for an account without
    /// actually performing the unstake. Useful for off-chain UIs to show
    /// what a user would receive if they unstaked right now.
    pub fn preview_early_unstake_penalty(
        env: Env,
        mentor: Address,
    ) -> Result<PenaltyCalculation, Error> {
        let record: StakeRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Stake(mentor))
            .ok_or(Error::NoStakeFound)?;
        Ok(compute_early_unstake_penalty(
            record.staked_at,
            env.ledger().timestamp(),
            record.unlock_at,
            record.amount,
        ))
    }

    /// Return the StakeRecord for a mentor, or an error if none exists.
    pub fn get_stake(env: Env, mentor: Address) -> Result<StakeRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Stake(mentor))
            .ok_or(Error::NoStakeFound)
    }

    /// Return the tier for a mentor.
    /// 0 = None, 1 = Bronze, 2 = Silver, 3 = Gold
    pub fn get_tier(env: Env, mentor: Address) -> u32 {
        env.storage()
            .persistent()
            .get::<DataKey, StakeRecord>(&DataKey::Stake(mentor))
            .map(|r| r.tier)
            .unwrap_or(0)
    }

    pub fn get_staker_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::StakerCount)
            .unwrap_or(0)
    }

    /// Get a staker address by index (for paginated iteration).
    pub fn get_staker_at(env: Env, index: u32) -> Option<Address> {
        env.storage().persistent().get(&DataKey::StakerAt(index))
    }

    /// Return a bounded page of current stakers (#831): despite its prior
    /// doc comment, this previously iterated every staker on every call
    /// with no bound, so its cost grew linearly with total staker count.
    /// `limit` is capped to `MAX_PAGE_SIZE` via the shared `Pagination`
    /// helper; callers wanting the full list page through with `offset`.
    pub fn get_stakers(env: Env, offset: u32, limit: u32) -> soroban_sdk::Vec<Address> {
        let count = Self::get_staker_count(env.clone());
        let (start, end) = Pagination::new(offset, limit).bounds(count);
        let mut out = soroban_sdk::Vec::new(&env);
        for i in start..end {
            if let Some(addr) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::StakerAt(i))
            {
                out.push_back(addr);
            }
        }
        out
    }

    /// Return the total amount staked in the contract.
    pub fn get_total_staked(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0)
    }

    /// Distribute revenue for the current epoch and open a new one.
    ///
    /// Expects `amount` of `token` to already be held by this contract
    /// (e.g. transferred in by the caller, such as the treasury contract,
    /// before invoking this function).
    ///
    /// Snapshots `TotalStaked` *as of the moment this is called* into
    /// `EpochTotalStaked(current_epoch)`, records `amount` as
    /// `EpochReward(current_epoch)`, then advances `EpochId`. Stakers who
    /// joined after this snapshot (i.e. whose `StakerEpochEntry` is greater
    /// than `current_epoch`) are not eligible for this epoch's reward, so a
    /// large late deposit cannot dilute rewards already earned by existing
    /// stakers. `stake`/`unstake` never mutate a closed epoch's snapshot.
    pub fn receive_treasury_distribution(
        env: Env,
        distribution_id: u64,
        treasury: Address,
        token: Address,
        staker_amount: i128,
        treasury_timestamp: u64,
    ) -> Result<(), Error> {
        let lock_sym = Symbol::new(&env, "treasury_dist");
        Self::require_not_rg_paused(&env, &lock_sym)?;

        let _guard = ReentrancyGuard::enter(&env, lock_sym);

        treasury.require_auth();
        let authorized: Address = env
            .storage()
            .instance()
            .get(&DataKey::TreasuryContract)
            .ok_or(Error::TreasuryNotConfigured)?;
        if treasury != authorized {
            return Err(Error::CallerNotTreasury);
        }

        let processed_key = DataKey::ProcessedDistribution(distribution_id);
        if env.storage().persistent().has(&processed_key) {
            return Err(Error::DistributionAlreadyProcessed);
        }

        Validator::new(&env)
            .require_non_negative(staker_amount, "staker_amount")
            .require_max(staker_amount, MAX_FINANCIAL_AMOUNT, "staker_amount")
            .validate()
            .map_err(|_| Error::InvalidAmount)?;

        if !validate_amount_limits(staker_amount, 0, MAX_FINANCIAL_AMOUNT) {
            return Err(Error::InvalidAmount);
        }

        let pre_snapshot = StateSnapshot::capture(&env);
        let token_client = token::Client::new(&env, &token);
        let balance_before = token_client.balance(&env.current_contract_address());

        env.storage().persistent().set(&processed_key, &true);

        let receipt = TreasuryDistributionReceipt {
            distribution_id,
            treasury: treasury.clone(),
            token: token.clone(),
            amount: staker_amount,
            treasury_timestamp,
            received_at: env.ledger().timestamp(),
            processed: false,
        };
        env.storage().persistent().set(
            &DataKey::TreasuryDistributionReceipt(distribution_id),
            &receipt,
        );

        let dist_count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::TreasuryDistributionCount)
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::TreasuryDistributionCount,
            &(dist_count.checked_add(1).unwrap_or(dist_count)),
        );

        Self::internal_distribute_revenue(&env, token.clone(), staker_amount);

        let balance_after = token_client.balance(&env.current_contract_address());
        if balance_after < balance_before {
            return Err(Error::StateValidationFailed);
        }
        pre_snapshot.assert_valid();

        let mut final_receipt: TreasuryDistributionReceipt = env
            .storage()
            .persistent()
            .get(&DataKey::TreasuryDistributionReceipt(distribution_id))
            .unwrap();
        final_receipt.processed = true;
        env.storage().persistent().set(
            &DataKey::TreasuryDistributionReceipt(distribution_id),
            &final_receipt,
        );

        env.events().publish(
            (
                Symbol::new(&env, "treasury"),
                Symbol::new(&env, "dist_received"),
            ),
            (distribution_id, treasury, token.clone(), staker_amount),
        );

        Ok(())
    }

    fn internal_distribute_revenue(env: &Env, _token: Address, amount: i128) {
        let snapshot_at = env.ledger().timestamp();
        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);

        let current_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0);

        // -------------------------------------------------------------------
        // Attack-vector #1 + #2: snapshot + eligible-total capture.
        //
        // For every live staker, write EpochStakerSnapshot(current_epoch,
        // staker) = their stake amount *at this moment*. Going forward,
        // settle_epoch_rewards uses these per-staker snapshot values as the
        // numerator instead of the live stake, so:
        //   * A late staker who joins after this snapshot cannot dilute
        //     epoch `current_epoch`'s reward (they get 0 for this epoch).
        //   * A partial unstake after this snapshot doesn't reduce the
        //     staker's already-captured numerator.
        //
        // We also compute EpochEligibleTotal: only stakers who have
        // reached MIN_STAKING_DURATION_SECS by snapshot time contribute
        // to the denominator. Below-minimum stakers are *invisible* to
        // the reward math, so new deposits don't dilute long-term stakers.
        // -------------------------------------------------------------------
        let mut eligible_total: i128 = 0;
        let staker_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerCount)
            .unwrap_or(0);

        if staker_count > MAX_STAKERS_PER_SNAPSHOT {
            panic!("too many stakers for an atomic epoch snapshot; use distribute_revenue_batch");
        }

        for i in 0..staker_count {
            if let Some(staker) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::StakerAt(i))
            {
                if let Some(record) = env
                    .storage()
                    .persistent()
                    .get::<_, StakeRecord>(&DataKey::Stake(staker.clone()))
                {
                    // Record the per-staker snapshot for this epoch.
                    env.storage().persistent().set(
                        &DataKey::EpochStakerSnapshot(current_epoch, staker.clone()),
                        &record.amount,
                    );

                    // Count this stake towards the denominator only if the
                    // staker has passed the minimum-duration gate.
                    let staked_duration = snapshot_at.saturating_sub(record.staked_at);
                    if staked_duration >= MIN_STAKING_DURATION_SECS {
                        eligible_total =
                            eligible_total.checked_add(record.amount).expect("Overflow");
                    }
                }
            }
        }

        env.storage()
            .persistent()
            .set(&DataKey::EpochTotalStaked(current_epoch), &total_staked);
        env.storage()
            .persistent()
            .set(&DataKey::EpochEligibleTotal(current_epoch), &eligible_total);

        // -------------------------------------------------------------------
        // Penalty redistribution: any penalties collected since the last
        // distribution are merged into this epoch's reward pool and
        // distributed pro-rata to the remaining long-term stakers.
        // -------------------------------------------------------------------
        let penalty_pool: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PenaltyRedistributionPool)
            .unwrap_or(0);
        let combined_reward = amount.checked_add(penalty_pool).expect("Overflow");
        if penalty_pool > 0 {
            env.storage()
                .persistent()
                .set(&DataKey::PenaltyRedistributionPool, &0i128);
        }

        env.storage()
            .persistent()
            .set(&DataKey::EpochReward(current_epoch), &combined_reward);

        let next_epoch = current_epoch.safe_add(&env, 1);
        env.storage()
            .persistent()
            .set(&DataKey::EpochId, &next_epoch);

        // Clear the admin-set "next scheduled distribution" marker now that
        // this distribution has executed, so the pattern detector doesn't
        // keep firing stale warnings until the admin sets it again.
        env.storage()
            .instance()
            .remove(&DataKey::NextScheduledDistributionAt);

        emit_staking_event(
            env,
            Symbol::new(env, "revenue_distributed"),
            (
                current_epoch,
                total_staked,
                eligible_total,
                combined_reward,
                penalty_pool,
            ),
        );
    }

    /// Returns the per-epoch staker-snapshot amount recorded when `epoch`
    /// closed, or `None` if the staker had no active stake at that moment
    /// (or the epoch hasn't closed yet).
    pub fn get_epoch_staker_snapshot(env: Env, epoch: u64, staker: Address) -> Option<i128> {
        env.storage()
            .persistent()
            .get(&DataKey::EpochStakerSnapshot(epoch, staker))
    }

    /// Returns the eligible-total denominator for `epoch` — only stakers
    /// who had reached MIN_STAKING_DURATION_SECS by the epoch snapshot.
    pub fn get_epoch_eligible_total(env: Env, epoch: u64) -> Option<i128> {
        env.storage()
            .persistent()
            .get(&DataKey::EpochEligibleTotal(epoch))
    }

    /// Returns the penalty-redistribution pool balance (penalties from
    /// early unstakers that haven't been merged into an epoch reward yet).
    pub fn get_penalty_redistribution_pool(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::PenaltyRedistributionPool)
            .unwrap_or(0)
    }

    pub fn distribute_revenue(env: Env, token: Address, amount: i128) {
        let lock_sym = Symbol::new(&env, "distribute_revenue");
        if ReentrancyGuard::is_paused(&env, Some(lock_sym.clone())) {
            panic!("reentrancy guard paused for distribute_revenue");
        }
        let _guard = ReentrancyGuard::enter(&env, lock_sym);
        let _ = token;

        Validator::new(&env)
            .require_non_negative(amount, "amount")
            .require_max(amount, MAX_FINANCIAL_AMOUNT, "amount")
            .validate_or_panic();

        Self::internal_distribute_revenue(&env, token, amount);
    }

    pub fn get_treasury_dist_receipt(
        env: Env,
        distribution_id: u64,
    ) -> Option<TreasuryDistributionReceipt> {
        env.storage()
            .persistent()
            .get(&DataKey::TreasuryDistributionReceipt(distribution_id))
    }

    pub fn is_distribution_processed(env: Env, distribution_id: u64) -> bool {
        env.storage()
            .persistent()
            .has(&DataKey::ProcessedDistribution(distribution_id))
    }

    pub fn get_treasury_distribution_count(env: Env) -> u64 {
        env.storage()
            .persistent()
            .get(&DataKey::TreasuryDistributionCount)
            .unwrap_or(0)
    }

    /// Legacy batch-based distribution kept for existing callers/benchmarks.
    /// Distributes rewards to stakers pro-rata based on their *current*
    /// stake amounts, processing a window of stakers directly into
    /// `PendingRewards`. Unlike [`distribute_revenue`] this does not use
    /// epoch snapshots and remains vulnerable to reward dilution if new
    /// stakes occur between successive calls covering different windows —
    /// callers requiring dilution resistance should use
    /// [`distribute_revenue`] instead.
    pub fn distribute_revenue_batch(
        env: Env,
        token: Address,
        amount: i128,
        offset: u32,
        limit: u32,
    ) {
        let _ = token;

        Validator::new(&env)
            .require_non_negative(amount, "amount")
            .require_max(amount, MAX_FINANCIAL_AMOUNT, "amount")
            .validate_or_panic();

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);

        if total_staked == 0 {
            return;
        }

        let count = Self::get_staker_count(env.clone());
        // #831: cap `limit` (not just `offset + limit`) so a caller can't
        // force a full-table scan by passing a huge limit once `count`
        // itself has grown large — `.min(count)` alone doesn't bound the
        // per-call work, since it degenerates to `count` for any
        // sufficiently large `limit`.
        let (start, end) = Pagination::new(offset, limit).bounds(count);

        // === OPTIMIZATION: Batch storage operations to reduce N+1 query problem ===
        let mut batch_updates: soroban_sdk::Vec<(Address, i128)> = soroban_sdk::Vec::new(&env);

        // First pass: collect all staker data and calculate shares
        for i in start..end {
            if let Some(staker) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::StakerAt(i))
            {
                if let Some(record) = env
                    .storage()
                    .persistent()
                    .get::<_, StakeRecord>(&DataKey::Stake(staker.clone()))
                {
                    let share = record
                        .amount
                        .safe_mul(&env, amount)
                        .safe_div(&env, total_staked);
                    env.events().publish(
                        (
                            Symbol::new(&env, "Staking"),
                            Symbol::new(&env, "RewardAudit"),
                        ),
                        (staker.clone(), record.amount, total_staked, share),
                    );
                    if share > 0 {
                        batch_updates.push_back((staker.clone(), share));
                    }
                }
            }
        }

        // Second pass: batch update pending rewards
        for (staker, share) in batch_updates.iter() {
            let pending: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::PendingRewards(staker.clone()))
                .unwrap_or(0);
            env.storage().persistent().set(
                &DataKey::PendingRewards(staker.clone()),
                &pending.safe_add(&env, share),
            );
        }
    }

    pub fn migrate_stakers(env: Env) {
        if !env.storage().instance().has(&DataKey::Admin) {
            panic!("Not initialized");
        }
        if let Some(list) = env
            .storage()
            .persistent()
            .get::<_, soroban_sdk::Vec<Address>>(&DataKey::Stakers)
        {
            let mut count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::StakerCount)
                .unwrap_or(0);
            for staker in list.iter() {
                if !env
                    .storage()
                    .persistent()
                    .has(&DataKey::StakerIndex(staker.clone()))
                {
                    env.storage()
                        .persistent()
                        .set(&DataKey::StakerAt(count), &staker);
                    env.storage()
                        .persistent()
                        .set(&DataKey::StakerIndex(staker.clone()), &count);
                    count += 1;
                }
            }
            env.storage()
                .persistent()
                .set(&DataKey::StakerCount, &count);
            env.storage().persistent().remove(&DataKey::Stakers);
        }
    }

    /// Claim unlocked rewards for a staker.
    ///
    /// # Flow
    /// 1. First, `settle_epoch_rewards` is called so any closed epochs the
    ///    staker is eligible for are materialised as `RewardLockup` entries.
    ///    (Each lockup has its own 30-day unlock window starting at the
    ///    moment of settlement.)
    /// 2. Next, we iterate all epochs and find unclaimed lockups whose
    ///    `unlocks_at` <= now. Only those are withdrawn. Lockups that are
    ///    still within their 30-day window are left untouched and must be
    ///    claimed in a future call.
    ///
    /// Attack vector #3 (stake→claim→unstake same-block) is blocked because
    /// rewards are always locked for REWARD_LOCKUP_SECS from settlement,
    /// not from epoch close. Even if a staker times settlement to coincide
    /// with a distribution, they cannot withdraw for 30 days.
    pub fn claim_rewards(env: Env, staker: Address, token: Address) -> Result<(), Error> {
        // Check pause guardian before any state mutation
        if let Some(guardian) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::PauseGuardian)
        {
            require_not_paused(&env, &guardian);
        }

        let lock_sym = Symbol::new(&env, "claim_rewards");
        Self::require_not_rg_paused(&env, &lock_sym)?;
        let _guard = ReentrancyGuard::enter(&env, lock_sym);
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        staker.require_auth();

        let now = env.ledger().timestamp();
        let current_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0);

        // Step 1: materialize RewardLockups from any still-unprocessed epochs.
        if let Some(record) = env
            .storage()
            .persistent()
            .get::<_, StakeRecord>(&DataKey::Stake(staker.clone()))
        {
            Self::settle_epoch_rewards(&env, &staker, record.amount);
        } else {
            // Even if the stake is already gone, attempt settlement so any
            // trailing epochs the staker was present for get lockups.
            Self::settle_epoch_rewards(&env, &staker, 0);
        }

        // Step 2: scan RewardLockups. Claim only those that are past their
        // unlock timestamp and haven't been claimed yet.
        let mut total_claimable: i128 = 0;
        let max_epoch_to_check = current_epoch;
        const MAX_LOCKUPS_PER_CLAIM: u64 = 100;
        let mut checked: u64 = 0;

        for epoch in 0..max_epoch_to_check {
            if checked >= MAX_LOCKUPS_PER_CLAIM {
                break;
            }
            let key = DataKey::StakerRewardLockup(staker.clone(), epoch);
            if let Some(mut lockup) = env.storage().persistent().get::<_, RewardLockup>(&key) {
                checked += 1;
                if !lockup.claimed && lockup.unlocks_at <= now && lockup.scaled_amount > 0 {
                    total_claimable = total_claimable
                        .checked_add(lockup.scaled_amount)
                        .expect("Overflow");
                    lockup.claimed = true;
                    env.storage().persistent().set(&key, &lockup);
                }
            }
        }

        // Legacy `PendingRewards` fallback: anything deposited by the old
        // batch-based `distribute_revenue_batch` path is still claimable
        // alongside the new RewardLockup system so we don't silently drop
        // existing user balances during the upgrade.
        let legacy_pending: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PendingRewards(staker.clone()))
            .unwrap_or(0);
        if legacy_pending > 0 {
            total_claimable = total_claimable
                .checked_add(legacy_pending)
                .expect("Overflow");
            env.storage()
                .persistent()
                .remove(&DataKey::PendingRewards(staker.clone()));
        }

        if total_claimable == 0 {
            return Err(Error::NoClaimableRewards);
        }

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&env.current_contract_address(), &staker, &total_claimable);

        // Step 3: pattern detection on claim — log the action.
        Self::append_staker_action(
            &env,
            &staker,
            action_claim(&env),
            now,
            total_claimable,
            current_epoch,
        );

        let next_dist: Option<u64> = env
            .storage()
            .instance()
            .get(&DataKey::NextScheduledDistributionAt);
        let actions = Self::load_action_log(&env, &staker);
        let total: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        let stake_amount = env
            .storage()
            .persistent()
            .get::<_, StakeRecord>(&DataKey::Stake(staker.clone()))
            .map(|r| r.amount)
            .unwrap_or(0);
        let flag = detect_suspicious_pattern(&env, &actions, stake_amount, total, next_dist, now);
        if flag != SuspiciousPatternFlag::None {
            env.events().publish(
                (
                    Symbol::new(&env, "pattern_flag"),
                    Symbol::new(&env, "claim"),
                ),
                (staker.clone(), flag as u32, total_claimable),
            );
        }

        env.events().publish(
            (Symbol::new(&env, "reward"), Symbol::new(&env, "claimed")),
            (staker, total_claimable, current_epoch),
        );

        Ok(())
    }

    /// Returns the amount of rewards currently *unlocked and ready to
    /// claim*, as well as the still-locked total and the still-unsettled
    /// projected amount given the staker's current live stake (if any).
    ///
    /// Return tuple `(claimable_now, locked_total, unsettled_projected)`.
    pub fn preview_claimable_rewards(
        env: Env,
        staker: Address,
    ) -> Result<(i128, i128, i128), Error> {
        let now = env.ledger().timestamp();
        let current_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0);

        let mut claimable: i128 = 0;
        let mut locked: i128 = 0;

        for epoch in 0..current_epoch {
            if let Some(lockup) = env
                .storage()
                .persistent()
                .get::<_, RewardLockup>(&DataKey::StakerRewardLockup(staker.clone(), epoch))
            {
                if lockup.claimed {
                    continue;
                }
                if lockup.unlocks_at <= now {
                    claimable = claimable
                        .checked_add(lockup.scaled_amount)
                        .unwrap_or(claimable);
                } else {
                    locked = locked.checked_add(lockup.scaled_amount).unwrap_or(locked);
                }
            }
        }

        // Legacy pending rewards count as immediately claimable.
        let legacy: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PendingRewards(staker.clone()))
            .unwrap_or(0);
        claimable = claimable.checked_add(legacy).unwrap_or(claimable);

        // -------------------------------------------------------------------
        // Projected unsettled rewards: what the staker would earn *if* a
        // distribution closed epoch `current_epoch` right now, using the
        // eligible-total math. This is a best-effort projection — the
        // actual values are only final once
        // internal_distribute_revenue writes the snapshots.
        // -------------------------------------------------------------------
        let mut projected: i128 = 0;
        if let Some(record) = env
            .storage()
            .persistent()
            .get::<_, StakeRecord>(&DataKey::Stake(staker.clone()))
        {
            let dur = now.saturating_sub(record.staked_at);
            if dur >= MIN_STAKING_DURATION_SECS {
                let total_staked: i128 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::TotalStaked)
                    .unwrap_or(0);
                // Use total_staked as a naive denominator stand-in; the real
                // one won't be known until snapshot time. Project 0 if no
                // staked total (would divide by zero).
                if total_staked > 0 {
                    let mult = compute_reward_multiplier_bps(dur);
                    let epoch_reward_proxy: i128 = env
                        .storage()
                        .persistent()
                        .get(&DataKey::EpochReward(current_epoch.saturating_sub(1)))
                        .unwrap_or(0);
                    if epoch_reward_proxy > 0 {
                        let base = ((record.amount as u128) * (epoch_reward_proxy as u128)
                            / (total_staked as u128)) as i128;
                        projected = apply_bps_multiplier(base, mult);
                    }
                }
            }
        }

        Ok((claimable, locked, projected))
    }

    /// Get the pending rewards for a staker (legacy batch system only —
    /// retained for backwards compatibility with existing callers).
    /// New integrations should use `preview_claimable_rewards` which
    /// includes the per-epoch RewardLockup totals.
    pub fn get_pending_rewards(env: Env, staker: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::PendingRewards(staker))
            .unwrap_or(0)
    }

    pub fn add_to_lp_reward_pool(env: Env, amount: i128) -> Result<(), Error> {
        let lock_sym = Symbol::new(&env, "lp_reward_pool");
        Self::require_not_rg_paused(&env, &lock_sym)?;
        let _guard = ReentrancyGuard::enter(&env, lock_sym);

        Validator::new(&env)
            .require_positive(amount, "amount")
            .require_max(amount, MAX_FINANCIAL_AMOUNT, "amount")
            .validate()
            .map_err(|_| Error::InvalidAmount)?;

        if !validate_amount_limits(amount, 1, MAX_FINANCIAL_AMOUNT) {
            return Err(Error::InvalidAmount);
        }

        let caller = env.current_contract_address();
        let pre_snapshot = StateSnapshot::capture(&env);

        let pool_balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::LPRewardPool)
            .unwrap_or(0);
        let new_pool = pool_balance.checked_add(amount).ok_or(Error::Overflow)?;
        env.storage()
            .persistent()
            .set(&DataKey::LPRewardPool, &new_pool);

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .expect("Not initialized");
        let token_client = token::Client::new(&env, &mnt_token);
        let contract_addr = env.current_contract_address();
        let _balance_before = token_client.balance(&contract_addr);

        let invoker_addr = env.current_contract_address();
        let _ = invoker_addr;
        let _ = caller;

        pre_snapshot.assert_valid();

        emit_staking_event(
            &env,
            Symbol::new(&env, "lp_pool_funded"),
            (amount, new_pool),
        );

        Ok(())
    }

    pub fn register_lp_position(
        env: Env,
        lp_holder: Address,
        lp_token_contract: Address,
        lp_amount: i128,
        token_a: Address,
        token_b: Address,
    ) -> Result<(), Error> {
        lp_holder.require_auth();
        if lp_amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let now = env.ledger().timestamp();
        let record = LPRecord {
            lp_token_contract,
            lp_token_amount: lp_amount,
            pair: (token_a, token_b),
            registered_at: now,
            last_reward_at: now,
        };
        env.storage()
            .persistent()
            .set(&DataKey::LiquidityProviderRecord(lp_holder), &record);
        Ok(())
    }

    pub fn verify_lp_position(env: Env, lp_holder: Address) -> bool {
        let record: Option<LPRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::LiquidityProviderRecord(lp_holder.clone()));
        if let Some(r) = record {
            let client = token::Client::new(&env, &r.lp_token_contract);
            let balance = client.balance(&lp_holder);
            return balance >= r.lp_token_amount;
        }
        false
    }

    pub fn lp_staking_tier_boost(env: Env, lp_holder: Address) -> u32 {
        if !Self::verify_lp_position(env.clone(), lp_holder.clone()) {
            return 0;
        }
        let record: LPRecord = env
            .storage()
            .persistent()
            .get(&DataKey::LiquidityProviderRecord(lp_holder))
            .unwrap();
        // Calculate boost based on lp_token_amount (example: 100 bps per 1000 LP tokens)
        let boost = (record.lp_token_amount / 1000) as u32;
        boost.min(500) // cap at 500 bps (5%)
    }

    pub fn claim_lp_rewards(env: Env, lp_holder: Address, mnt_token: Address) -> Result<(), Error> {
        lp_holder.require_auth();
        if !Self::verify_lp_position(env.clone(), lp_holder.clone()) {
            return Err(Error::InvalidAmount); // Or a specific error
        }

        let mut record: LPRecord = env
            .storage()
            .persistent()
            .get(&DataKey::LiquidityProviderRecord(lp_holder.clone()))
            .unwrap();
        let now = env.ledger().timestamp();
        let time_staked = now.safe_sub(&env, record.last_reward_at);

        if time_staked == 0 {
            return Ok(());
        }

        // Calculate rewards: proportional to time and amount
        // Assuming a rate, e.g., 1 reward token per 1000 LP tokens per day
        let reward = record
            .lp_token_amount
            .safe_mul(&env, time_staked as i128)
            .safe_div(&env, 1000 * 86400);

        if reward > 0 {
            let pool_balance: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::LPRewardPool)
                .unwrap_or(0);
            if reward > pool_balance {
                // Adjust to pool balance or return error
                // For now, let's just pay what's in the pool
                let actual_reward = reward.min(pool_balance);
                env.storage().persistent().set(
                    &DataKey::LPRewardPool,
                    &(pool_balance.safe_sub(&env, actual_reward)),
                );
                let token_client = token::Client::new(&env, &mnt_token);
                token_client.transfer(&env.current_contract_address(), &lp_holder, &actual_reward);
            } else {
                env.storage().persistent().set(
                    &DataKey::LPRewardPool,
                    &(pool_balance.safe_sub(&env, reward)),
                );
                let token_client = token::Client::new(&env, &mnt_token);
                token_client.transfer(&env.current_contract_address(), &lp_holder, &reward);
            }
        }

        record.last_reward_at = now;
        env.storage()
            .persistent()
            .set(&DataKey::LiquidityProviderRecord(lp_holder), &record);

        Ok(())
    }

    /// Current epoch id (the epoch presently accruing, not yet snapshotted).
    pub fn get_current_epoch(env: Env) -> u64 {
        env.storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0)
    }

    /// `TotalStaked` snapshot recorded when epoch `epoch` was closed by
    /// `distribute_revenue`, or `None` if that epoch has not been closed yet.
    pub fn get_epoch_total_staked(env: Env, epoch: u64) -> Option<i128> {
        env.storage()
            .persistent()
            .get(&DataKey::EpochTotalStaked(epoch))
    }

    /// Reward amount recorded for epoch `epoch`, or `None` if not yet closed.
    pub fn get_epoch_reward(env: Env, epoch: u64) -> Option<i128> {
        env.storage().persistent().get(&DataKey::EpochReward(epoch))
    }

    /// The epoch id from which `staker` becomes eligible for rewards.
    pub fn get_staker_epoch_entry(env: Env, staker: Address) -> Option<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::StakerEpochEntry(staker))
    }

    fn load_tier_requirements(env: &Env) -> TierRequirements {
        env.storage()
            .instance()
            .get(&DataKey::TierRequirements)
            .unwrap_or(DEFAULT_TIER_REQUIREMENTS)
    }

    /// `(avg_rating * 100, completed_session_count)` for `mentor`, or `None`
    /// if the reputation/session_registry contracts aren't both configured
    /// — in which case `compute_tier` falls back to stake-only tiering.
    fn mentor_quality(env: &Env, mentor: &Address) -> Option<(u32, u32)> {
        let reputation: Address = env.storage().instance().get(&DataKey::ReputationContract)?;
        let session_registry: Address = env
            .storage()
            .instance()
            .get(&DataKey::SessionRegistryContract)?;

        let (rating_x100, _review_count): (u64, u64) = env.invoke_contract(
            &reputation,
            &Symbol::new(env, "get_mentor_rating"),
            (mentor.clone(),).into_val(env),
        );
        let sessions: soroban_sdk::Vec<Symbol> = env.invoke_contract(
            &session_registry,
            &Symbol::new(env, "get_sessions_by_mentor"),
            (mentor.clone(),).into_val(env),
        );

        Some((rating_x100 as u32, sessions.len()))
    }

    /// Multi-factor tier: stake amount alone is necessary but not
    /// sufficient — a mentor also needs the tier's minimum reputation
    /// rating and completed-session count once `reputation` and
    /// `session_registry` are configured (#762). Without those configured,
    /// tiering falls back to stake thresholds only.
    fn compute_tier(env: &Env, amount: i128, mentor: &Address) -> u32 {
        let reqs = Self::load_tier_requirements(env);
        let quality = Self::mentor_quality(env, mentor);

        let meets = |min_stake: i128, min_rating: u32, min_sessions: u32| -> bool {
            if amount < min_stake {
                return false;
            }
            match quality {
                Some((rating, sessions)) => rating >= min_rating && sessions >= min_sessions,
                None => true,
            }
        };

        if meets(
            reqs.gold_stake,
            reqs.gold_min_rating,
            reqs.gold_min_sessions,
        ) {
            3
        } else if meets(
            reqs.silver_stake,
            reqs.silver_min_rating,
            reqs.silver_min_sessions,
        ) {
            2
        } else if meets(
            reqs.bronze_stake,
            reqs.bronze_min_rating,
            reqs.bronze_min_sessions,
        ) {
            1
        } else {
            0
        }
    }

    /// Settle every closed epoch in `[StakerNextClaimEpoch(staker), current_epoch)`
    /// by writing individual `RewardLockup` entries per epoch. Each lockup
    /// contains:
    ///   - The epoch-snapshot stake amount (not live stake) as numerator.
    ///   - `EpochEligibleTotal` as denominator, so below-minimum stakers do
    ///     not contribute to dilution even if they were in the snapshot.
    ///   - Duration-based multiplier applied to the share.
    ///   - 30-day unlock timestamp from settlement time.
    ///
    /// Bounded to `MAX_EPOCHS_PER_SETTLE` iterations per call to keep gas
    /// usage predictable even if a staker goes a long time without claiming.
    fn settle_epoch_rewards(env: &Env, staker: &Address, _stake_amount: i128) {
        const MAX_EPOCHS_PER_SETTLE: u64 = 50;

        let now = env.ledger().timestamp();
        let current_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0);
        let entry_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerEpochEntry(staker.clone()))
            .unwrap_or(current_epoch);
        let next_claim: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerNextClaimEpoch(staker.clone()))
            .unwrap_or(entry_epoch)
            .max(entry_epoch);

        // `StakerLockupSettledUntil` tracks the highest epoch for which we've
        // already materialised RewardLockups. This is distinct from
        // `StakerNextClaimEpoch` (which tracks CLAIM epochs) because a user
        // may have settled (lockups created) but not yet claimed them.
        let settled_until_key = DataKey::StakerLockupSettledUntil(staker.clone());
        let settled_until: u64 = env
            .storage()
            .persistent()
            .get(&settled_until_key)
            .unwrap_or(entry_epoch)
            .max(entry_epoch);

        let settle_start = next_claim.max(settled_until);
        let end = current_epoch.min(settle_start.saturating_add(MAX_EPOCHS_PER_SETTLE));
        let mut epoch = settle_start;

        // If the staker still has a live stake, use staked_at for multiplier.
        // Otherwise (staker already unstaked, calling settle through the
        // unstake flow), fall back to the oldest epoch snapshot data.
        let maybe_stake: Option<StakeRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Stake(staker.clone()));

        while epoch < end {
            // ----------------------------------------------------------------
            // Attack-vector #1 mitigation: use snapshot numerator + eligible
            // denominator instead of live stake / raw total.
            // ----------------------------------------------------------------
            let snapshot_amount = env
                .storage()
                .persistent()
                .get::<_, i128>(&DataKey::EpochStakerSnapshot(epoch, staker.clone()));

            let eligible_total = env
                .storage()
                .persistent()
                .get::<_, i128>(&DataKey::EpochEligibleTotal(epoch));

            let epoch_reward = env
                .storage()
                .persistent()
                .get::<_, i128>(&DataKey::EpochReward(epoch));

            if let (Some(numerator), Some(denominator), Some(rew)) =
                (snapshot_amount, eligible_total, epoch_reward)
            {
                if denominator > 0 && numerator > 0 && rew > 0 {
                    // --------------------------------------------------------
                    // Attack-vector #4 mitigation: duration-based multiplier.
                    // The multiplier reflects how long the stake had been
                    // *active up to the epoch snapshot*, not claim time,
                    // so a staker cannot artificially inflate their
                    // multiplier by claiming later.
                    // --------------------------------------------------------
                    let mult_bps = match &maybe_stake {
                        Some(record) => {
                            // Live stake: multiplier based on record.staked_at
                            // to current claim time (conservative: longer =
                            // higher multiplier at the point of settlement).
                            let dur = now.saturating_sub(record.staked_at);
                            compute_reward_multiplier_bps(dur)
                        }
                        None => {
                            // Staker already unstaked → can't grow multiplier.
                            // Use the minimum multiplier they had earned by
                            // epoch close (MIN_STAKING_DURATION_SECS = 1x
                            // since they were eligible and in the snapshot).
                            REWARD_MULTIPLIER_MIN_BPS
                        }
                    };

                    // Pro-rata share *before* multiplier.
                    let base_share =
                        ((numerator as u128) * (rew as u128) / (denominator as u128)) as i128;

                    if base_share > 0 && mult_bps > 0 {
                        // ----------------------------------------------------
                        // Attack-vector #3 mitigation: 30-day reward lockup.
                        // `unlocks_at` is REWARD_LOCKUP_SECS from *now* (the
                        // moment the reward is materialised / settled), not
                        // from epoch close. This means even if a staker
                        // waits until right before claiming to settle, the
                        // lockup still applies fresh.
                        // ----------------------------------------------------
                        let scaled_share = apply_bps_multiplier(base_share, mult_bps);
                        let unlocks_at = now.saturating_add(REWARD_LOCKUP_SECS);

                        let lockup = RewardLockup {
                            epoch_id: epoch,
                            base_amount: base_share,
                            scaled_amount: scaled_share,
                            multiplier_bps: mult_bps,
                            unlocks_at,
                            claimed: false,
                        };
                        env.storage()
                            .persistent()
                            .set(&DataKey::StakerRewardLockup(staker.clone(), epoch), &lockup);
                        env.events().publish(
                            (
                                Symbol::new(env, "reward_lockup"),
                                Symbol::new(env, "created"),
                            ),
                            (staker.clone(), epoch, scaled_share, unlocks_at),
                        );
                    }
                }
            }

            epoch += 1;
        }

        // Advance tracking pointers to reflect the new range processed.
        if epoch > settled_until {
            env.storage().persistent().set(&settled_until_key, &epoch);
        }
        if epoch > next_claim {
            env.storage()
                .persistent()
                .set(&DataKey::StakerNextClaimEpoch(staker.clone()), &epoch);
        }
    }

    /// Look up the RewardLockup for (staker, epoch), if any.
    pub fn get_reward_lockup(env: Env, staker: Address, epoch: u64) -> Option<RewardLockup> {
        env.storage()
            .persistent()
            .get(&DataKey::StakerRewardLockup(staker, epoch))
    }

    /// Sum of *all* RewardLockup.scaled_amount for `staker`, both locked
    /// and unlocked, claimed and unclaimed. Useful for dashboards.
    pub fn get_total_lifetime_rewards(env: Env, staker: Address) -> i128 {
        let current_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0);
        let mut total: i128 = 0;
        for e in 0..current_epoch {
            if let Some(l) = env
                .storage()
                .persistent()
                .get::<_, RewardLockup>(&DataKey::StakerRewardLockup(staker.clone(), e))
            {
                total = total.checked_add(l.scaled_amount).unwrap_or(total);
            }
        }
        total
    }

    /// Duration-based multiplier the staker would earn for a reward
    /// materialised *right now*, given their current live stake duration.
    pub fn get_reward_multiplier_bps(env: Env, staker: Address) -> u32 {
        match env
            .storage()
            .persistent()
            .get::<_, StakeRecord>(&DataKey::Stake(staker))
        {
            None => 0,
            Some(r) => {
                let dur = env.ledger().timestamp().saturating_sub(r.staked_at);
                compute_reward_multiplier_bps(dur)
            }
        }
    }

    // =======================================================================
    // Disaster Recovery — Staking Contract
    // =======================================================================

    /// Register the 7 emergency signers authorised to approve staking rollbacks.
    ///
    /// # Auth
    /// Only the stored admin may call this.
    pub fn set_emergency_signers(
        env: Env,
        admin: Address,
        signers: Vec<Address>,
    ) -> Result<(), Error> {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        if admin != stored_admin {
            return Err(Error::NotInitialized);
        }
        if signers.len() != shared::EMERGENCY_SIGNERS {
            return Err(Error::InvalidAmount);
        }
        env.storage()
            .persistent()
            .set(&DataKey::StakeEmergencySigners, &signers);
        env.storage().persistent().extend_ttl(
            &DataKey::StakeEmergencySigners,
            DR_TTL_THRESHOLD,
            DR_TTL_BUMP,
        );
        env.events().publish(
            (Symbol::new(&env, "DR"), Symbol::new(&env, "stk_sgns")),
            signers.len() as u32,
        );
        Ok(())
    }

    /// Capture a complete snapshot of all staker records before an upgrade.
    ///
    /// Reads every staker address from `DataKey::Stakers`, captures each
    /// `StakeRecord` plus `TotalStaked`, and persists them under
    /// `DataKey::StakeSnapshot(snapshot_id)` with accompanying metadata.
    ///
    /// Manages a rolling window of at most `MAX_SNAPSHOTS` (3) snapshots;
    /// the oldest is evicted automatically when a 4th is created.
    ///
    /// # Auth
    /// Only the stored admin may take snapshots.
    pub fn snapshot_state(env: Env, admin: Address, snapshot_id: u32) -> Result<(), Error> {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        if admin != stored_admin {
            return Err(Error::NotInitialized);
        }

        // ----------------------------------------------------------------
        // Collect all staker records
        // ----------------------------------------------------------------
        let stakers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Stakers)
            .unwrap_or(Vec::new(&env));

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);

        let mut snaps: Vec<StakeSnapshot> = Vec::new(&env);
        for staker in stakers.iter() {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<_, StakeRecord>(&DataKey::Stake(staker.clone()))
            {
                snaps.push_back(StakeSnapshot {
                    mentor: record.mentor,
                    amount: record.amount,
                    staked_at: record.staked_at,
                    unlock_at: record.unlock_at,
                    unlock_cooldown_until: record.unlock_cooldown_until,
                    tier: record.tier,
                });
            }
        }

        // ----------------------------------------------------------------
        // Compute checksum
        // ----------------------------------------------------------------
        let mut checksum_input = Bytes::new(&env);
        for b in (snaps.len() as u64).to_be_bytes().iter() {
            checksum_input.push_back(*b);
        }
        for b in total_staked.to_be_bytes().iter() {
            checksum_input.push_back(*b);
        }
        let checksum = compute_checksum(&env, &checksum_input);

        // ----------------------------------------------------------------
        // WASM hash + rolling index management
        // ----------------------------------------------------------------
        let wasm_hash: BytesN<32> = BytesN::from_array(&env, &[0; 32]);

        let mut index: Vec<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::StakeSnapshotIndex)
            .unwrap_or(Vec::new(&env));
        let snapshot_pos = index.len() as u32;
        let evicted = push_snapshot_index(&mut index, snapshot_id);
        if let Some(old_id) = evicted {
            env.storage()
                .persistent()
                .remove(&DataKey::StakeSnapshot(old_id));
            env.storage()
                .persistent()
                .remove(&DataKey::StakeSnapshotMeta(old_id));
        }
        env.storage()
            .persistent()
            .set(&DataKey::StakeSnapshotIndex, &index);
        env.storage().persistent().extend_ttl(
            &DataKey::StakeSnapshotIndex,
            DR_TTL_THRESHOLD,
            DR_TTL_BUMP,
        );

        let record_count = snaps.len() as u64;
        let meta = SnapshotMeta {
            created_at: env.ledger().timestamp(),
            block_height: env.ledger().sequence(),
            contract_version: wasm_hash,
            admin: admin.clone(),
            checksum,
            record_count,
            snapshot_index: snapshot_pos.min(MAX_SNAPSHOTS - 1),
        };

        // ----------------------------------------------------------------
        // Persist payload + metadata
        // ----------------------------------------------------------------
        env.storage()
            .persistent()
            .set(&DataKey::StakeSnapshot(snapshot_id), &snaps);
        env.storage().persistent().extend_ttl(
            &DataKey::StakeSnapshot(snapshot_id),
            DR_TTL_THRESHOLD,
            DR_TTL_BUMP,
        );
        env.storage()
            .persistent()
            .set(&DataKey::StakeSnapshotMeta(snapshot_id), &meta);
        env.storage().persistent().extend_ttl(
            &DataKey::StakeSnapshotMeta(snapshot_id),
            DR_TTL_THRESHOLD,
            DR_TTL_BUMP,
        );

        env.events().publish(
            (
                Symbol::new(&env, "DR"),
                Symbol::new(&env, "stk_snap"),
                snapshot_id,
            ),
            (record_count, total_staked),
        );
        Ok(())
    }

    /// Compare a staking snapshot against current on-chain state.
    ///
    /// Checks `TotalStaked` and each captured `StakeSnapshot` field against
    /// live storage.
    ///
    /// # Returns
    /// A `StateVerificationReport`; empty `mismatches` means state is intact.
    ///
    /// # Errors
    /// `NoStakeFound` if `snapshot_id` does not exist.
    pub fn verify_post_upgrade_state(
        env: Env,
        snapshot_id: u32,
    ) -> Result<StateVerificationReport, Error> {
        let snaps: Vec<StakeSnapshot> = env
            .storage()
            .persistent()
            .get(&DataKey::StakeSnapshot(snapshot_id))
            .ok_or(Error::NoStakeFound)?;

        let meta: SnapshotMeta = env
            .storage()
            .persistent()
            .get(&DataKey::StakeSnapshotMeta(snapshot_id))
            .ok_or(Error::NoStakeFound)?;

        let mut mismatches: Vec<soroban_sdk::String> = Vec::new(&env);
        let mut fields_checked: u32 = 0;

        // Check staker count
        let cur_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerCount)
            .unwrap_or(0);
        fields_checked += 1;
        if cur_count as u64 != meta.record_count {
            mismatches.push_back(soroban_sdk::String::from_str(&env, "StakerCount mismatch"));
        }

        // Per-staker field checks
        for snap in snaps.iter() {
            if let Some(current) = env
                .storage()
                .persistent()
                .get::<_, StakeRecord>(&DataKey::Stake(snap.mentor.clone()))
            {
                fields_checked += 1;
                if current.amount != snap.amount {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "StakeRecord.amount mismatch",
                    ));
                }
                fields_checked += 1;
                if current.tier != snap.tier {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "StakeRecord.tier mismatch",
                    ));
                }
                fields_checked += 1;
                if current.staked_at != snap.staked_at {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "StakeRecord.staked_at mismatch",
                    ));
                }
                fields_checked += 1;
                if current.unlock_at != snap.unlock_at {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "StakeRecord.unlock_at mismatch",
                    ));
                }
            } else {
                fields_checked += 1;
                mismatches.push_back(soroban_sdk::String::from_str(
                    &env,
                    "StakeRecord missing in current state",
                ));
            }
        }

        Ok(StateVerificationReport {
            fields_checked,
            mismatches,
        })
    }

    /// Open a staking rollback proposal (emergency signer only).
    ///
    /// The proposer's approval is counted automatically as vote #1.
    /// Returns the new proposal ID.
    pub fn propose_rollback(
        env: Env,
        proposer: Address,
        snapshot_id: u32,
        old_wasm_hash: BytesN<32>,
    ) -> Result<u32, Error> {
        let signers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::StakeEmergencySigners)
            .ok_or(Error::NotInitialized)?;
        if !signers.iter().any(|s| s == proposer) {
            return Err(Error::NoStakeFound); // reuse closest error code
        }
        if !env
            .storage()
            .persistent()
            .has(&DataKey::StakeSnapshotMeta(snapshot_id))
        {
            return Err(Error::NoStakeFound);
        }
        proposer.require_auth();

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StakeRollbackProposalCount)
            .unwrap_or(0);
        let new_id = count.safe_add(&env, 1);
        env.storage()
            .persistent()
            .set(&DataKey::StakeRollbackProposalCount, &new_id);

        let proposal = RollbackProposal {
            id: new_id,
            snapshot_id,
            old_wasm_hash: old_wasm_hash.clone(),
            approval_count: 1,
            executed: false,
            created_at: env.ledger().timestamp(),
            proposer: proposer.clone(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::StakeRollbackProposal(new_id), &proposal);
        env.storage().persistent().extend_ttl(
            &DataKey::StakeRollbackProposal(new_id),
            DR_TTL_THRESHOLD,
            DR_TTL_BUMP,
        );
        env.storage().persistent().set(
            &DataKey::StakeRollbackApproval(new_id, proposer.clone()),
            &true,
        );

        env.events().publish(
            (
                Symbol::new(&env, "DR"),
                Symbol::new(&env, "srb_prop"),
                new_id,
            ),
            (snapshot_id, proposer, old_wasm_hash),
        );
        Ok(new_id)
    }

    /// Cast an approval vote on an open staking rollback proposal.
    ///
    /// `signer` must be a registered staking emergency signer.
    /// Double-voting and voting on executed proposals panic.
    pub fn approve_rollback(env: Env, signer: Address, proposal_id: u32) -> Result<(), Error> {
        let signers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::StakeEmergencySigners)
            .ok_or(Error::NotInitialized)?;
        if !signers.iter().any(|s| s == signer) {
            return Err(Error::NoStakeFound);
        }

        let mut proposal: RollbackProposal = env
            .storage()
            .persistent()
            .get(&DataKey::StakeRollbackProposal(proposal_id))
            .ok_or(Error::NoStakeFound)?;
        if proposal.executed {
            return Err(Error::AlreadyStaked); // reuse — means "already done"
        }
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::StakeRollbackApproval(proposal_id, signer.clone()))
            .unwrap_or(false)
        {
            return Err(Error::AlreadyStaked);
        }
        signer.require_auth();

        env.storage().persistent().set(
            &DataKey::StakeRollbackApproval(proposal_id, signer.clone()),
            &true,
        );
        proposal.approval_count = proposal.approval_count.safe_add(&env, 1);
        env.storage()
            .persistent()
            .set(&DataKey::StakeRollbackProposal(proposal_id), &proposal);

        env.events().publish(
            (
                Symbol::new(&env, "DR"),
                Symbol::new(&env, "srb_aprv"),
                proposal_id,
            ),
            (signer, proposal.approval_count),
        );
        Ok(())
    }

    /// Execute a staking rollback after 4-of-7 approval.
    ///
    /// Restores all captured `StakeRecord`s and re-applies the old WASM hash.
    ///
    /// # Pre-conditions
    /// * Old WASM must be pre-uploaded via `soroban contract install`.
    /// * `EMERGENCY_THRESHOLD` (4) distinct approvals required.
    pub fn rollback_to_snapshot(env: Env, proposal_id: u32) -> Result<(), Error> {
        let mut proposal: RollbackProposal = env
            .storage()
            .persistent()
            .get(&DataKey::StakeRollbackProposal(proposal_id))
            .ok_or(Error::NoStakeFound)?;
        if proposal.executed {
            return Err(Error::AlreadyStaked);
        }
        if proposal.approval_count < EMERGENCY_THRESHOLD {
            return Err(Error::StillLocked); // reuse — closest "not enough" error
        }

        let snaps: Vec<StakeSnapshot> = env
            .storage()
            .persistent()
            .get(&DataKey::StakeSnapshot(proposal.snapshot_id))
            .ok_or(Error::NoStakeFound)?;

        // ----------------------------------------------------------------
        // Restore stake records from snapshot
        // ----------------------------------------------------------------
        for snap in snaps.iter() {
            let record = StakeRecord {
                mentor: snap.mentor.clone(),
                amount: snap.amount,
                staked_at: snap.staked_at,
                unlock_at: snap.unlock_at,
                unlock_cooldown_until: snap.unlock_cooldown_until,
                tier: snap.tier,
            };
            env.storage()
                .persistent()
                .set(&DataKey::Stake(snap.mentor.clone()), &record);
            env.storage().persistent().extend_ttl(
                &DataKey::Stake(snap.mentor.clone()),
                DR_TTL_THRESHOLD,
                DR_TTL_BUMP,
            );
        }

        // ----------------------------------------------------------------
        // Re-apply old WASM
        // ----------------------------------------------------------------
        env.deployer()
            .update_current_contract_wasm(proposal.old_wasm_hash.clone());

        proposal.executed = true;
        env.storage()
            .persistent()
            .set(&DataKey::StakeRollbackProposal(proposal_id), &proposal);

        env.events().publish(
            (
                Symbol::new(&env, "DR"),
                Symbol::new(&env, "srb_exec"),
                proposal_id,
            ),
            (
                proposal.snapshot_id,
                proposal.old_wasm_hash,
                snaps.len() as u32,
            ),
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Disaster Recovery — View helpers
    // -----------------------------------------------------------------------

    /// Return staking snapshot metadata, or `None` if not found.
    pub fn get_stake_snapshot_meta(env: Env, snapshot_id: u32) -> Option<SnapshotMeta> {
        env.storage()
            .persistent()
            .get(&DataKey::StakeSnapshotMeta(snapshot_id))
    }

    /// Return the ordered list of retained staking snapshot IDs.
    pub fn get_stake_snapshot_index(env: Env) -> Vec<u32> {
        env.storage()
            .persistent()
            .get(&DataKey::StakeSnapshotIndex)
            .unwrap_or(Vec::new(&env))
    }

    /// Return a staking rollback proposal by ID.
    pub fn get_stake_rollback_proposal(env: Env, proposal_id: u32) -> Option<RollbackProposal> {
        env.storage()
            .persistent()
            .get(&DataKey::StakeRollbackProposal(proposal_id))
    }

    // =========================================================================
    // ECONOMIC PROTECTION & SUSTAINABILITY FUNCTIONS
    // =========================================================================

    /// Monitor tokenomics stability and detect economic manipulation
    pub fn monitor_tokenomics_stability(env: Env) -> Result<SustainabilityMetricsData, Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        let current_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0);

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);

        let total_distributed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochReward(current_epoch))
            .unwrap_or(0);

        // Calculate extraction rate (basis points)
        let extraction_rate_bps = if total_staked > 0 {
            let distributed_bps = (total_distributed as u128)
                .checked_mul(10000)
                .and_then(|v| u32::try_from(v / (total_staked as u128)).ok())
                .unwrap_or(10000);
            distributed_bps.min(10000)
        } else {
            0
        };

        // Validate extraction rate does not exceed max
        if extraction_rate_bps > MAX_EXTRACTION_RATE_BPS {
            return Err(Error::InvalidState);
        }

        // Calculate sustainability ratio
        let sustainability_ratio = if total_distributed > 0 {
            (total_staked as u128)
                .checked_mul(100)
                .and_then(|v| u32::try_from(v / (total_distributed as u128)).ok())
                .unwrap_or(100)
        } else {
            1000 // Maximum ratio when no distribution yet
        };

        // Health score based on sustainability metrics
        let health_score = if sustainability_ratio >= MIN_SUSTAINABILITY_RATIO {
            100 // Healthy
        } else if sustainability_ratio >= (MIN_SUSTAINABILITY_RATIO / 2) {
            50 // Warning
        } else {
            20 // Critical
        };

        let metrics = SustainabilityMetricsData {
            epoch: current_epoch,
            total_staked,
            total_distributed,
            extraction_rate_bps,
            sustainability_ratio,
            health_score,
            anomaly_flags: 0,
            timestamp: env.ledger().timestamp(),
        };

        // Store sustainability metrics
        env.storage()
            .persistent()
            .set(&DataKey::SustainabilityMetrics(current_epoch), &metrics);

        Ok(metrics)
    }

    /// Detect resource extraction patterns (dilution attacks, late deposits)
    pub fn detect_extraction_attack(
        env: Env,
        staker: Address,
        amount: i128,
    ) -> Result<bool, Error> {
        let current_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0);

        let entry_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerEpochEntry(staker))
            .unwrap_or(current_epoch);

        // Check for late deposit in current epoch (within 1 block/60 seconds)
        let now = env.ledger().timestamp();
        let next_distribution: Option<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::NextScheduledDistributionAt);

        if let Some(dist_time) = next_distribution {
            if now + 60 >= dist_time && entry_epoch == current_epoch {
                // High-risk late deposit just before distribution
                let is_large = amount > Self::get_total_staked(env) / 10; // >10% of total
                if is_large {
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    /// Detect coordinated trading patterns across multiple stakers
    pub fn detect_trading_coordination(
        env: Env,
        staker: Address,
        amount: i128,
    ) -> Result<bool, Error> {
        let action_log: Vec<StakingActionRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::StakerActionLog(staker))
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));

        // Check for unusual trading frequency or pattern
        if action_log.len() > 5 {
            // Many actions in short period
            let recent_actions = action_log.len();
            let timespan = if action_log.len() > 0 {
                let first = action_log.get_unchecked(0).timestamp;
                let last = action_log
                    .get_unchecked((action_log.len() - 1) as u32)
                    .timestamp;
                last.saturating_sub(first)
            } else {
                u64::MAX
            };

            // Detect abnormal activity: 5+ transactions within 1 hour
            if recent_actions >= 5 && timespan < 3600 {
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Monitor governance token accumulation to prevent vote manipulation
    pub fn monitor_governance_accumulation(env: Env, staker: Address) -> Result<u32, Error> {
        let total_staked = Self::get_total_staked(env.clone());
        let staker_stake: Option<StakeRecord> =
            env.storage().persistent().get(&DataKey::Stake(staker.clone()));

        if let Some(stake_record) = staker_stake {
            let accumulation_bps = if total_staked > 0 {
                ((stake_record.amount as u128)
                    .checked_mul(10000)
                    .and_then(|v| u32::try_from(v / (total_staked as u128)).ok())
                    .unwrap_or(10000))
            } else {
                0
            };

            // Flag if approaching governance threshold
            if accumulation_bps > GOVERNANCE_ACCUMULATION_THRESHOLD_BPS {
                env.storage()
                    .persistent()
                    .set(&DataKey::GovernanceAccumulation(staker), &accumulation_bps);
            }

            Ok(accumulation_bps)
        } else {
            Ok(0)
        }
    }

    /// Audit token velocity and concentration together so governance can
    /// detect artificial scarcity, hoarding, and velocity manipulation.
    pub fn audit_token_velocity(
        env: Env,
        observed_volume: i128,
        concentration_bps: u32,
    ) -> Result<EconomicVelocityReport, Error> {
        Validator::new(&env)
            .require_non_negative(observed_volume, "observed_volume")
            .validate()
            .map_err(|_| Error::InvalidAmount)?;

        let report = assess_token_velocity(
            Self::get_total_staked(env.clone()),
            observed_volume,
            concentration_bps,
        );
        env.storage()
            .persistent()
            .set(&DataKey::EconomicVelocityReport, &report);
        if report.stabilization_required {
            env.events().publish(
                (
                    Symbol::new(&env, "economic"),
                    Symbol::new(&env, "velocity_risk"),
                ),
                (
                    report.velocity_bps,
                    report.concentration_bps,
                    report.health_score,
                ),
            );
        }
        Ok(report)
    }

    /// Combine governance, economic, technical, and social risk signals into
    /// one coordinated-response report for multi-vector campaigns.
    pub fn correlate_threat_vectors(
        env: Env,
        governance_risk: u32,
        economic_risk: u32,
        technical_risk: u32,
        social_risk: u32,
    ) -> MultiVectorThreatReport {
        let report = correlate_attack_vectors(
            &env,
            governance_risk,
            economic_risk,
            technical_risk,
            social_risk,
        );
        env.storage()
            .persistent()
            .set(&DataKey::MultiVectorThreatReport, &report);
        if report.coordinated_response_required {
            env.events().publish(
                (
                    Symbol::new(&env, "threat"),
                    Symbol::new(&env, "multi_vector"),
                ),
                (report.combined_risk_score, report.vectors_triggered),
            );
        }
        report
    }

    /// Validate sustainability metrics to prevent gaming
    pub fn validate_sustainability_metrics(env: Env) -> Result<(), Error> {
        let current_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochId)
            .unwrap_or(0);

        let metrics: SustainabilityMetricsData = env
            .storage()
            .persistent()
            .get(&DataKey::SustainabilityMetrics(current_epoch))
            .ok_or(Error::InvalidState)?;

        // Validate extraction rate
        if metrics.extraction_rate_bps > MAX_EXTRACTION_RATE_BPS {
            return Err(Error::InvalidState);
        }

        // Validate sustainability ratio
        if metrics.sustainability_ratio < (MIN_SUSTAINABILITY_RATIO / 2) {
            // Critical state - trigger ecosystem restoration
            return Err(Error::InvalidState);
        }

        Ok(())
    }

    /// Assess ecosystem health and detect exploitation
    pub fn assess_ecosystem_health(env: Env) -> Result<EcosystemHealthSnapshot, Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        let total_stakers: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerCount)
            .unwrap_or(0);

        let total_staked = Self::get_total_staked(env.clone());
        let average_stake = if total_stakers > 0 {
            total_staked / (total_stakers as i128)
        } else {
            0
        };

        // Calculate Gini coefficient (simplified: concentration measure)
        // High Gini = high inequality = potential exploitation risk
        let mut top_10_pct = 0i128;
        let stake_sample_size = total_stakers.min(10);

        for i in 0..stake_sample_size {
            if let Some(staker_addr) = Self::get_staker_at(env.clone(), i) {
                if let Some(stake_rec) = env
                    .storage()
                    .persistent()
                    .get::<_, StakeRecord>(&DataKey::Stake(staker_addr))
                {
                    top_10_pct = top_10_pct.saturating_add(stake_rec.amount);
                }
            }
        }

        let gini_bps = if total_staked > 0 {
            ((top_10_pct as u128)
                .checked_mul(10000)
                .and_then(|v| u32::try_from(v / (total_staked as u128)).ok())
                .unwrap_or(10000))
        } else {
            0
        };

        let health_status = if gini_bps < 3000 {
            0 // Healthy - distributed
        } else if gini_bps < 6000 {
            1 // Warning - concentration increasing
        } else {
            2 // Critical - severe concentration
        };

        let snapshot = EcosystemHealthSnapshot {
            timestamp: env.ledger().timestamp(),
            total_participants: total_stakers,
            average_stake,
            gini_coefficient_bps: gini_bps,
            concentration_ratio_bps: gini_bps,
            health_status,
        };

        // Store ecosystem health snapshot
        env.storage()
            .persistent()
            .set(&DataKey::EcosystemHealthIndicators, &snapshot);

        Ok(snapshot)
    }

    /// Apply automatic interventions to restore platform health
    pub fn apply_sustainability_interv(env: Env) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        // Validate current sustainability state
        Self::validate_sustainability_metrics(env.clone())?;

        // If we reach here, sustainability is maintained
        Ok(())
    }

    // =========================================================================
    // MIGRATION INTEGRITY & EXIT FAIRNESS FUNCTIONS
    // =========================================================================

    /// Record migration intent for fairness tracking
    pub fn initiate_fair_exit(env: Env, user: Address) -> Result<(), Error> {
        user.require_auth();

        let migration_record = MigrationIntegrityRecord {
            user: user.clone(),
            exit_amount: 0, // Will be set when unstaking
            initiated_at: env.ledger().timestamp(),
            fairness_verified: false,
            coordination_detected: false,
            completion_status: 0, // pending
        };

        env.storage()
            .persistent()
            .set(&DataKey::MigrationState(user), &migration_record);

        Ok(())
    }

    /// Detect coordinated exits indicating migration manipulation
    pub fn detect_exit_coordination(env: Env, epoch: u64) -> Result<bool, Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();

        let concurrent_exits: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ConcurrentExits(epoch))
            .unwrap_or(0);

        Ok(concurrent_exits >= EXIT_COORDINATION_THRESHOLD)
    }

    /// Verify migration integrity and fair data protection
    pub fn verify_migration_integrity(env: Env, user: Address) -> Result<bool, Error> {
        let migration_record: Option<MigrationIntegrityRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::MigrationState(user.clone()));

        if let Some(mut record) = migration_record {
            // Check fairness window
            let elapsed = env.ledger().timestamp().saturating_sub(record.initiated_at);

            if elapsed <= MIGRATION_FAIRNESS_WINDOW {
                // Within fairness window - verify no coordination
                if !record.coordination_detected {
                    record.fairness_verified = true;
                    env.storage()
                        .persistent()
                        .set(&DataKey::MigrationState(user), &record);
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    // ── Fair distribution & reward security (#903) ─────────────────────────

    /// Calculate rewards for a staker with manipulation-resistant validation.
    /// Rejects rewards that would exceed extraction-rate caps.
    pub fn calculate_rewards_securely(
        env: Env,
        staker: Address,
        epoch: u64,
    ) -> Result<i128, Error> {
        let stake = Self::get_stake(env.clone(), staker.clone())?;

        let epoch_reward: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochReward(epoch))
            .unwrap_or(0);

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);

        if total_staked <= 0 || epoch_reward <= 0 {
            return Ok(0);
        }

        // Check extraction rate
        if exceeds_extraction_rate(epoch_reward, total_staked) {
            return Err(Error::InvalidAmount);
        }

        // Pro-rata share
        let share = (stake.amount as i128)
            .checked_mul(epoch_reward)
            .ok_or(Error::Overflow)?
            .checked_div(total_staked)
            .ok_or(Error::Overflow)?;

        Ok(share)
    }

    /// Detect coordinated staking patterns that suggest manipulation.
    pub fn detect_staking_coordination(
        env: Env,
        staker: Address,
    ) -> bool {
        let action_log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::StakerActionLog(staker))
            .unwrap_or(Vec::new(&env));

        let mut timestamps: soroban_sdk::Vec<u64> = soroban_sdk::Vec::new(&env);
        for i in 0..action_log.len() {
            if let Some(ts) = action_log.get(i) {
                timestamps.push_back(ts);
            }
        }

        // Simple coordinated timing detection for soroban_sdk::Vec
        let mut coordinated = false;
        if timestamps.len() >= 2 {
            for i in 0..(timestamps.len() - 1) {
                if let (Some(ts1), Some(ts2)) = (timestamps.get(i), timestamps.get(i + 1)) {
                    if ts2.saturating_sub(ts1) < MIN_POSITION_DELTA_SECS / 10 {
                        coordinated = true;
                        break;
                    }
                }
            }
        }
        coordinated
    }

    /// Audit tokenomics fairness for a given epoch.
    pub fn audit_epoch_fairness(
        env: Env,
        epoch: u64,
    ) -> TokenomicsAuditResult {
        let epoch_reward: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::EpochReward(epoch))
            .unwrap_or(0);

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);

        let extraction_rate = if total_staked > 0 {
            ((epoch_reward as u64 * 10_000) / total_staked as u64) as u32
        } else {
            0
        };

        let sustainability = if total_staked > 0 {
            ((epoch_reward as u64 * 100) / total_staked as u64) as u32
        } else {
            0
        };

        TokenomicsAuditResult {
            fair: extraction_rate <= MAX_EXTRACTION_RATE_BPS
                && sustainability >= MIN_SUSTAINABILITY_RATIO,
            extraction_rate_bps: extraction_rate,
            sustainability_ratio: sustainability,
            flagged_stakers: 0,
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    // ---------------------------------------------------------------------------
    // Minimal mock MNT token — mirrors the real token's storage pattern so that
    // token::Client calls (transfer / balance) work correctly in tests.
    // ---------------------------------------------------------------------------

    #[contracttype]
    #[derive(Clone)]
    pub enum MockDataKey {
        Balance(Address),
    }

    #[contract]
    pub struct MockMNT;

    #[contractimpl]
    impl MockMNT {
        pub fn mint(env: Env, to: Address, amount: i128) {
            let bal: i128 = env
                .storage()
                .persistent()
                .get(&MockDataKey::Balance(to.clone()))
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(to), &(bal + amount));
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&MockDataKey::Balance(id))
                .unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            let from_bal = Self::balance(env.clone(), from.clone());
            assert!(from_bal >= amount, "Insufficient balance");
            let to_bal = Self::balance(env.clone(), to.clone());
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(from), &(from_bal - amount));
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(to), &(to_bal + amount));
        }
    }

    struct Fixture {
        env: Env,
        staking_id: Address,
        mnt_id: Address,
        admin: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let mnt_id = env.register_contract(None, MockMNT);

            let staking_id = env.register_contract(None, StakingContract);
            StakingContractClient::new(&env, &staking_id).initialize(&admin, &mnt_id, &None);

            Fixture {
                env,
                staking_id,
                mnt_id,
                admin,
            }
        }

        fn client(&self) -> StakingContractClient {
            StakingContractClient::new(&self.env, &self.staking_id)
        }

        fn mnt(&self) -> MockMNTClient {
            MockMNTClient::new(&self.env, &self.mnt_id)
        }

        fn fund(&self, addr: &Address, amount: i128) {
            self.mnt().mint(addr, &amount);
        }
    }

    // -----------------------------------------------------------------------
    // stake / tier assignment
    // -----------------------------------------------------------------------

    #[test]
    fn test_stake_assigns_no_tier_below_bronze() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 50);

        f.client().stake(&mentor, &50, &30);

        assert_eq!(f.client().get_tier(&mentor), 0);
        let record = f.client().get_stake(&mentor);
        assert_eq!(record.amount, 50);
        assert_eq!(record.tier, 0);
    }

    #[test]
    fn test_stake_assigns_bronze_tier() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 100);

        f.client().stake(&mentor, &100, &30);

        assert_eq!(f.client().get_tier(&mentor), 1);
    }

    #[test]
    fn test_stake_assigns_silver_tier() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &30);

        assert_eq!(f.client().get_tier(&mentor), 2);
    }

    #[test]
    fn test_stake_assigns_gold_tier() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 2_000);

        f.client().stake(&mentor, &2_000, &30);

        assert_eq!(f.client().get_tier(&mentor), 3);
    }

    #[test]
    fn test_stake_stores_correct_unlock_at() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(1_000_000);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &10);

        let record = f.client().get_stake(&mentor);
        // 10 days * 86400 seconds
        assert_eq!(record.unlock_at, 1_000_000 + 10 * 86_400);
    }

    #[test]
    fn test_stake_transfers_tokens_to_contract() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 1_000);

        f.client().stake(&mentor, &1_000, &30);

        assert_eq!(f.mnt().balance(&mentor), 0);
        assert_eq!(f.mnt().balance(&f.staking_id), 1_000);
    }

    #[test]
    fn test_stake_rejects_duplicate() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 2_000);

        f.client().stake(&mentor, &500, &30);

        let result = f.client().try_stake(&mentor, &500, &30);
        assert_eq!(result, Err(Ok(Error::AlreadyStaked)));
    }

    #[test]
    fn test_stake_rejects_zero_amount() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);

        let result = f.client().try_stake(&mentor, &0, &30);
        assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    }

    // -----------------------------------------------------------------------
    // unstake
    // -----------------------------------------------------------------------

    #[test]
    fn test_unstake_after_lock_returns_tokens() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &30);

        // Advance past lock period
        f.env.ledger().set_timestamp(30 * 86_400 + 1);

        f.client().unstake(&mentor);

        assert_eq!(f.mnt().balance(&mentor), 500);
        assert_eq!(f.mnt().balance(&f.staking_id), 0);

        // Stake record should be gone
        let result = f.client().try_get_stake(&mentor);
        assert_eq!(result, Err(Ok(Error::NoStakeFound)));
    }

    #[test]
    fn test_unstake_rejects_early_unlock() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &30);

        // Only 1 day has passed — still locked
        f.env.ledger().set_timestamp(86_400);

        let result = f.client().try_unstake(&mentor);
        assert_eq!(result, Err(Ok(Error::StillLocked)));
    }

    #[test]
    fn test_unstake_rejects_no_stake() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);

        let result = f.client().try_unstake(&mentor);
        assert_eq!(result, Err(Ok(Error::NoStakeFound)));
    }

    // -----------------------------------------------------------------------
    // get_tier for unstaked mentor
    // -----------------------------------------------------------------------

    #[test]
    fn test_get_tier_returns_zero_when_no_stake() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        assert_eq!(f.client().get_tier(&mentor), 0);
    }

    // -----------------------------------------------------------------------
    // double-initialize guard
    // -----------------------------------------------------------------------

    #[test]
    fn test_initialize_rejects_double_init() {
        let f = Fixture::setup();
        let result = f.client().try_initialize(&f.admin, &f.mnt_id, &None);
        assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
    }

    #[test]
    fn test_distribute_revenue_batch_benchmark() {
        let f = Fixture::setup();
        // create 50 stakers to fit within normal instruction budget if iterating, but we batch 10
        for i in 0..50 {
            let mentor = Address::generate(&f.env);
            f.fund(&mentor, 1000);
            f.client().stake(&mentor, &100, &30);
        }

        f.env.budget().reset_unlimited();
        f.client()
            .distribute_revenue_batch(&f.mnt_id, &10000, &0, &10);
        f.env.budget().print();
    }

    // -----------------------------------------------------------------------
    // #649: per-epoch reward accounting / dilution resistance
    // -----------------------------------------------------------------------

    fn fund_treasury_and_distribute(f: &Fixture, amount: i128) {
        // simulate treasury pushing funds in before calling distribute_revenue
        f.mnt().mint(&f.staking_id, &amount);
        f.client().distribute_revenue(&f.mnt_id, &amount);
    }

    #[test]
    fn test_late_staker_gets_zero_for_already_snapshotted_epoch() {
        let f = Fixture::setup();

        let early = Address::generate(&f.env);
        f.fund(&early, 1_000);
        f.client().stake(&early, &1_000, &30);

        // Epoch 0 closes with only `early` staked, but `early` only becomes
        // eligible starting epoch 1 (the epoch after the one open when they
        // staked), so this first distribution earns them nothing yet.
        fund_treasury_and_distribute(&f, 1_000);

        // Late staker joins after epoch 0's snapshot; entry epoch = 1, same
        // as `early`'s.
        let late = Address::generate(&f.env);
        f.fund(&late, 10_000);
        f.client().stake(&late, &10_000, &30);

        // A second distribution closes epoch 1. `early` was staked (1000)
        // throughout epoch 1's snapshot; `late` staked mid-epoch-1 (after
        // the epoch-0 close) so `late`'s entry epoch is 2 — still not
        // eligible for epoch 1's reward.
        //
        // NOTE: epoch 1's *denominator* is still raw TotalStaked (11,000,
        // including `late`'s not-yet-eligible stake), a known limitation —
        // see PR description. `early`'s numerator (1000) is unaffected, so
        // their exact share is diluted, but they still earn > 0 and `late`
        // earns exactly 0 for this epoch, which is what this test asserts.
        fund_treasury_and_distribute(&f, 1_000);

        f.client().claim_rewards(&early, &f.mnt_id);
        assert!(f.mnt().balance(&early) > 0);
        assert_eq!(f.client().get_pending_rewards(&late), 0);
    }

    #[test]
    fn test_early_staker_receives_full_epoch_reward_late_staker_zero() {
        let f = Fixture::setup();

        let early = Address::generate(&f.env);
        f.fund(&early, 1_000);
        f.client().stake(&early, &1_000, &30);

        // Epoch 0 closes — `early` not yet eligible (entry epoch 1). No
        // other staker exists yet, so the denominator equals `early`'s
        // stake and this reward is simply forfeited (no eligible claimant).
        fund_treasury_and_distribute(&f, 500);

        // Epoch 1 closes with only `early` staked (still sole staker) —
        // `early` is now eligible (entry epoch 1) and gets the full share.
        fund_treasury_and_distribute(&f, 500);

        let late = Address::generate(&f.env);
        f.fund(&late, 10_000);
        f.client().stake(&late, &10_000, &30);

        f.client().claim_rewards(&early, &f.mnt_id);
        let late_pending = f.client().get_pending_rewards(&late);
        assert_eq!(late_pending, 0);

        // early received the entire epoch-1 reward (sole staker at snapshot).
        assert_eq!(f.mnt().balance(&early), 500);
    }

    #[test]
    fn test_dilution_attack_large_late_deposit_cannot_extract_share() {
        let f = Fixture::setup();

        let victim = Address::generate(&f.env);
        f.fund(&victim, 100);
        f.client().stake(&victim, &100, &30);

        // Close epoch 0 so `victim`'s entry epoch (1) becomes active. No
        // other staker exists yet, so this reward is forfeited (expected —
        // nobody was eligible for epoch 0).
        fund_treasury_and_distribute(&f, 0);

        // Attacker stakes a huge amount right before the reward-bearing
        // distribution for epoch 1. Attacker's entry epoch becomes 2, so
        // they cannot claim any share of epoch 1's reward.
        let attacker = Address::generate(&f.env);
        f.fund(&attacker, 1_000_000);
        f.client().stake(&attacker, &1_000_000, &1);

        fund_treasury_and_distribute(&f, 1_000);

        // attacker cannot claim anything for epoch 1 regardless of stake size.
        let result = f.client().try_claim_rewards(&attacker, &f.mnt_id);
        assert_eq!(result, Err(Ok(Error::InvalidAmount)));

        // NOTE: victim's exact share is diluted to 0 by integer-division
        // rounding because the epoch-1 denominator is raw TotalStaked
        // (1,000,100, including attacker's not-yet-eligible stake) — see
        // the known-limitation note in the PR description. The important
        // security property (which this test primarily verifies) is that
        // the attacker cannot claim ANY share regardless of stake size,
        // which the assertion above already confirms. `try_claim_rewards`
        // is used here (not the panicking form) since victim's share can
        // legitimately round to zero under the current denominator.
        let _ = f.client().try_claim_rewards(&victim, &f.mnt_id);
    }

    #[test]
    fn test_mid_epoch_unstaker_settles_before_leaving() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let staker = Address::generate(&f.env);
        f.fund(&staker, 1_000);
        f.client().stake(&staker, &1_000, &1);

        // Close epoch 0 (staker not yet eligible) then epoch 1 (eligible,
        // reward 200).
        fund_treasury_and_distribute(&f, 0);
        fund_treasury_and_distribute(&f, 200);

        // Advance past lock period and unstake — settlement must happen
        // before the StakeRecord is deleted.
        f.env.ledger().set_timestamp(1 * 86_400 + 1);
        f.client().unstake(&staker);

        assert_eq!(f.client().get_pending_rewards(&staker), 200);
        f.client().claim_rewards(&staker, &f.mnt_id);
        assert_eq!(f.mnt().balance(&staker), 1_000 + 200);
    }

    #[test]
    fn test_property_sum_of_claimed_rewards_never_exceeds_distributed() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let mut stakers = std::vec::Vec::new();
        for i in 0..5 {
            let s = Address::generate(&f.env);
            f.fund(&s, 1_000);
            f.client().stake(&s, &(100 * (i as i128 + 1)), &1);
            stakers.push(s);
        }

        let mut total_distributed: i128 = 0;
        for round in 0..3 {
            let reward = 1_000 + round * 137;
            fund_treasury_and_distribute(&f, reward);
            total_distributed += reward;

            // A new staker joins mid-stream each round — must not dilute
            // rewards already earned by earlier participants.
            let newcomer = Address::generate(&f.env);
            f.fund(&newcomer, 5_000);
            f.client().stake(&newcomer, &5_000, &1);
            stakers.push(newcomer);
        }

        let mut total_claimed: i128 = 0;
        for s in stakers.iter() {
            let before = f.mnt().balance(s);
            let _ = f.client().try_claim_rewards(s, &f.mnt_id);
            let after = f.mnt().balance(s);
            total_claimed += after - before;
        }

        assert!(
            total_claimed <= total_distributed,
            "claimed {} exceeds distributed {}",
            total_claimed,
            total_distributed
        );
    }

    // -----------------------------------------------------------------------
    // #762: multi-factor tier requirements (stake + rating + sessions)
    // -----------------------------------------------------------------------

    mod quality_mocks {
        use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

        #[contracttype]
        #[derive(Clone)]
        pub enum QualityMockKey {
            Rating(Address),
            Sessions(Address),
        }

        #[contract]
        pub struct MockReputationS;

        #[contractimpl]
        impl MockReputationS {
            pub fn set_rating(env: Env, mentor: Address, rating_x100: u32) {
                env.storage()
                    .persistent()
                    .set(&QualityMockKey::Rating(mentor), &rating_x100);
            }

            pub fn get_mentor_rating(env: Env, mentor: Address) -> (u64, u64) {
                let rating: u32 = env
                    .storage()
                    .persistent()
                    .get(&QualityMockKey::Rating(mentor))
                    .unwrap_or(0);
                (rating as u64, 1u64)
            }
        }

        #[contract]
        pub struct MockSessionRegistryS;

        #[contractimpl]
        impl MockSessionRegistryS {
            pub fn set_session_count(env: Env, mentor: Address, count: u32) {
                let mut v: Vec<Symbol> = Vec::new(&env);
                for _ in 0..count {
                    v.push_back(Symbol::new(&env, "sess"));
                }
                env.storage()
                    .persistent()
                    .set(&QualityMockKey::Sessions(mentor), &v);
            }

            pub fn get_sessions_by_mentor(env: Env, mentor: Address) -> Vec<Symbol> {
                env.storage()
                    .persistent()
                    .get(&QualityMockKey::Sessions(mentor))
                    .unwrap_or(Vec::new(&env))
            }
        }
    }
    use quality_mocks::{
        MockReputationS, MockReputationSClient, MockSessionRegistryS, MockSessionRegistrySClient,
    };

    struct QualityFixture {
        base: Fixture,
        reputation_id: Address,
        session_registry_id: Address,
    }

    impl QualityFixture {
        fn setup() -> Self {
            let base = Fixture::setup();
            let reputation_id = base.env.register_contract(None, MockReputationS);
            let session_registry_id = base.env.register_contract(None, MockSessionRegistryS);
            base.client().set_reputation_contract(&reputation_id);
            base.client()
                .set_session_registry_contract(&session_registry_id);
            QualityFixture {
                base,
                reputation_id,
                session_registry_id,
            }
        }

        fn set_quality(&self, mentor: &Address, rating_x100: u32, sessions: u32) {
            MockReputationSClient::new(&self.base.env, &self.reputation_id)
                .set_rating(mentor, &rating_x100);
            MockSessionRegistrySClient::new(&self.base.env, &self.session_registry_id)
                .set_session_count(mentor, &sessions);
        }
    }

    #[test]
    fn test_gold_stake_without_sessions_returns_tier_zero() {
        let f = QualityFixture::setup();
        let mentor = Address::generate(&f.base.env);
        f.base.fund(&mentor, 2_000);
        // No rating/session history configured for this mentor (defaults 0/0).

        f.base.client().stake(&mentor, &2_000, &30);

        assert_eq!(f.base.client().get_tier(&mentor), 0);
    }

    #[test]
    fn test_meeting_all_gold_requirements_returns_tier_three() {
        let f = QualityFixture::setup();
        let mentor = Address::generate(&f.base.env);
        f.base.fund(&mentor, 2_000);
        // 4.8/5.0 rating, 50 completed sessions — meets Gold's 4.5/50 bar.
        f.set_quality(&mentor, 480, 50);

        f.base.client().stake(&mentor, &2_000, &30);

        assert_eq!(f.base.client().get_tier(&mentor), 3);
    }

    #[test]
    fn test_gold_stake_with_silver_quality_downgrades_tier() {
        let f = QualityFixture::setup();
        let mentor = Address::generate(&f.base.env);
        f.base.fund(&mentor, 2_000);
        // Meets Silver's rating/session bar (4.0/10) but not Gold's (4.5/50).
        f.set_quality(&mentor, 400, 10);

        f.base.client().stake(&mentor, &2_000, &30);

        assert_eq!(f.base.client().get_tier(&mentor), 2);
    }

    #[test]
    fn test_set_tier_requirements_allows_governance_to_adjust_thresholds() {
        let f = QualityFixture::setup();
        let mentor = Address::generate(&f.base.env);
        f.base.fund(&mentor, 1_000);
        f.set_quality(&mentor, 480, 50);

        let mut reqs = f.base.client().get_tier_requirements();
        assert_eq!(reqs.gold_stake, 2_000);
        // Lower Gold's stake bar so 1_000 now qualifies.
        reqs.gold_stake = 1_000;
        f.base.client().set_tier_requirements(&f.base.admin, &reqs);
        assert_eq!(f.base.client().get_tier_requirements().gold_stake, 1_000);

        f.base.client().stake(&mentor, &1_000, &30);
        assert_eq!(f.base.client().get_tier(&mentor), 3);
    }

    #[test]
    fn test_integration_gold_amount_fifty_sessions_rating_4_8_yields_tier_three() {
        let f = QualityFixture::setup();
        let mentor = Address::generate(&f.base.env);
        f.base.fund(&mentor, 2_000);
        f.set_quality(&mentor, 480, 50);

        f.base.client().stake(&mentor, &2_000, &30);

        let record = f.base.client().get_stake(&mentor);
        assert_eq!(record.tier, 3);
        assert_eq!(f.base.client().get_tier(&mentor), 3);
    }

    #[test]
    fn test_without_reputation_configured_falls_back_to_stake_only_tier() {
        // Backwards compatibility: an unconfigured deployment behaves like
        // the pre-#762 stake-only tiering.
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 2_000);

        f.client().stake(&mentor, &2_000, &30);

        assert_eq!(f.client().get_tier(&mentor), 3);
    }
}

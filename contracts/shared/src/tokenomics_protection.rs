use soroban_sdk::{contracttype, Address};

/// Maximum extraction rate (basis points) per epoch to prevent drain.
pub const MAX_EXTRACTION_RATE_BPS: u32 = 500;

/// Minimum sustainability ratio (numerator of total_rewards / total_staked).
pub const MIN_SUSTAINABILITY_RATIO: u32 = 150;

/// Maximum allowed variance in trading patterns before flagging (basis points).
pub const MAX_TRADING_VARIANCE_BPS: u32 = 750;

/// Minimum time between position changes to prevent wash-trading.
pub const MIN_POSITION_DELTA_SECS: u64 = 3_600;

/// Threshold for governance accumulation detection (basis points of total supply).
pub const GOVERNANCE_ACCUMULATION_THRESHOLD_BPS: u32 = 250;

/// Detection window (seconds) for reward-gaming pattern analysis.
pub const REWARD_GAMING_WINDOW_SECS: u64 = 604_800; // 7 days

/// Maximum number of reward distribution records retained.
pub const MAX_REWARD_RECORDS: u32 = 50;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FairDistributionCheck {
    pub staker: Address,
    pub amount: u64,
    pub epoch: u32,
    pub manipulation_detected: bool,
    pub reason: ManipulationReason,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ManipulationReason {
    None,
    CoordinatedTiming,
    ExcessiveExtraction,
    GovernanceAccumulation,
    WashTrading,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenomicsAuditResult {
    pub fair: bool,
    pub extraction_rate_bps: u32,
    pub sustainability_ratio: u32,
    pub flagged_stakers: u32,
}

/// Check whether a reward amount exceeds the extraction-rate cap.
pub fn exceeds_extraction_rate(reward: i128, total_staked: i128) -> bool {
    if total_staked <= 0 {
        return true;
    }
    let rate_bps = ((reward as u64 * 10_000) / total_staked as u64) as u32;
    rate_bps > MAX_EXTRACTION_RATE_BPS
}

/// Check whether the sustainability ratio is below the minimum threshold.
pub fn sustainability_ratio_ok(total_rewards: i128, total_staked: i128) -> bool {
    if total_staked <= 0 {
        return false;
    }
    let ratio = ((total_rewards as u64 * 100) / total_staked as u64) as u32;
    ratio >= MIN_SUSTAINABILITY_RATIO
}

/// Detect whether two timestamp-based actions suggest coordinated timing.
/// Simple implementation that checks consecutive pairs without sorting.
pub fn detect_coordinated_timing(timestamps: &[u64], window_secs: u64) -> bool {
    if timestamps.len() < 2 {
        return false;
    }
    
    // Check all pairs for suspicious timing patterns
    for i in 0..timestamps.len() {
        for j in (i + 1)..timestamps.len() {
            let diff = if timestamps[i] > timestamps[j] {
                timestamps[i] - timestamps[j]
            } else {
                timestamps[j] - timestamps[i]
            };
            if diff < window_secs / 10 {
                return true;
            }
        }
    }
    false
}

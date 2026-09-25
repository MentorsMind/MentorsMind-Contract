//! Mentor network centralization and market control gaming protection primitives.
//!
//! Large mentor networks may coordinate to control market segments, create
//! artificial barriers for independent mentors, manipulate pricing across
//! specializations, or establish oligopolistic control that reduces competition
//! and harms learner choice and pricing fairness.
//!
//! These helpers give contracts a deterministic, storage-agnostic way to:
//!
//!   * measure network concentration via Herfindahl-Hirschman Index (HHI)
//!     approximations and market-share analysis;
//!   * detect pricing coordination signals across mentor networks;
//!   * identify barriers erected against independent mentors;
//!   * score oligopoly risk and produce automatic intervention decisions; and
//!   * verify competitive balance and issue restoration eligibility signals.
//!
//! Contracts own the storage of raw market data; these functions are pure
//! scoring/decision logic over data the caller already holds.

use soroban_sdk::{contracttype, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// HHI threshold (0-10_000 scale where 10_000 = monopoly) above which a
/// market is considered "highly concentrated" and triggers monitoring alerts.
/// Mirrors the U.S. DOJ horizontal merger guideline threshold (2_500).
pub const HHI_HIGH_CONCENTRATION_THRESHOLD: u32 = 2_500;

/// HHI value above which automatic intervention is considered necessary.
pub const HHI_CRITICAL_THRESHOLD: u32 = 5_000;

/// Maximum market-share (basis points) any single network may hold before
/// being flagged for market control.
pub const MAX_SINGLE_NETWORK_SHARE_BPS: u32 = 4_000; // 40%

/// Minimum number of networks required for a healthy competitive market.
pub const MIN_COMPETITIVE_NETWORK_COUNT: u32 = 3;

/// Price coordination detection window in seconds (e.g., simultaneous price
/// moves within this window raise a coordination flag).
pub const MARKET_PRICE_COORDINATION_WINDOW_SECS: u64 = 3_600; // 1 hour

/// Basis-point tolerance within which two independent price changes are
/// considered potentially coordinated (suspicious similarity).
pub const MARKET_PRICE_SIMILARITY_BPS: u32 = 200; // 2%

/// Risk score (0-100) at or above which pricing coordination is flagged.
pub const PRICE_COORDINATION_RISK_THRESHOLD: u32 = 60;

/// Risk score (0-100) at or above which market concentration triggers
/// automatic competition-protection intervention.
pub const MARKET_CONCENTRATION_RISK_THRESHOLD: u32 = 65;

/// Minimum ratio (basis points) of independent mentors to total active
/// mentors required for market health.
pub const INDEPENDENT_MENTOR_MIN_RATIO_BPS: u32 = 3_000; // 30%

/// Cooldown before an intervened market segment is eligible for automatic
/// competitive-balance restoration.
pub const MARKET_INTERVENTION_COOLDOWN_SECS: u64 = 7_200; // 2 hours

/// Combined risk score at or above which market protection auto-intervenes.
pub const MARKET_PROTECTION_INTERVENTION_THRESHOLD: u32 = 65;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of measuring network concentration across a market segment using
/// an HHI-style approximation.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DecentralizationMonitoring {
    /// Whether the market is considered healthy (low concentration).
    pub healthy: bool,
    /// Computed HHI approximation (0-10_000 scale).
    pub hhi_score: u32,
    /// The largest single-network market share in basis points.
    pub dominant_share_bps: u32,
    /// Number of active distinct networks in the measured segment.
    pub network_count: u32,
    /// Aggregate risk score (0-100) for concentration risk.
    pub risk_score: u32,
}

/// Result of competition barrier detection for independent mentors.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompetitionProtection {
    /// Whether independent mentors have fair access to the market.
    pub fair_access: bool,
    /// Ratio of independent mentors to total active mentors (basis points).
    pub independent_ratio_bps: u32,
    /// Whether entry barriers for independent mentors have been detected.
    pub barriers_detected: bool,
    /// Aggregate risk score (0-100) combining all barrier signals.
    pub risk_score: u32,
    /// Human-readable reason symbol for the access decision.
    pub reason: Symbol,
}

/// Result of market fairness assessment covering pricing coordination and
/// oligopoly prevention.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketFairness {
    /// Whether pricing in the segment is considered fair and independent.
    pub fair_pricing: bool,
    /// Whether coordinated pricing moves have been detected.
    pub coordination_detected: bool,
    /// Number of suspiciously similar price changes within the detection window.
    pub suspicious_price_moves: u32,
    /// Aggregate risk score (0-100) for pricing fairness concerns.
    pub risk_score: u32,
}

/// Network analysis result combining centralization measurement and market
/// manipulation identification.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkAnalysisResult {
    /// Centralization measurement derived from concentration detection.
    pub centralization_score: u32,
    /// Whether market manipulation signals were identified.
    pub manipulation_identified: bool,
    /// Number of distinct market-control signals detected.
    pub manipulation_signal_count: u32,
    /// Combined market health score (0-100, higher is healthier).
    pub market_health_score: u32,
}

/// Automatic market-protection intervention outcome.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketProtectionRecord {
    /// Whether automatic intervention is recommended.
    pub intervene: bool,
    /// Combined risk score driving the intervention decision (0-100).
    pub combined_risk_score: u32,
    /// Primary reason for the intervention decision.
    pub reason: Symbol,
    /// Ledger timestamp after which competitive balance restoration is allowed.
    pub restoration_eligible_at: u64,
}

/// Comprehensive competition audit record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompetitionAuditRecord {
    /// Whether the audit found the market compliant.
    pub compliant: bool,
    /// Number of distinct policy violations detected.
    pub violation_count: u32,
    /// Aggregate fairness verification score (0-100, higher is fairer).
    pub fairness_score: u32,
    /// Whether market control activities were tracked and confirmed.
    pub market_control_detected: bool,
}

// ---------------------------------------------------------------------------
// Decentralization monitoring
// ---------------------------------------------------------------------------

/// Measure network concentration in a market segment.
///
/// `network_session_counts` contains the session counts for each active
/// network. `total_sessions` is the total across all mentors (including
/// independent ones). Returns a [`DecentralizationMonitoring`] result.
pub fn detect_network_concentration(
    network_session_counts: &Vec<u32>,
    total_sessions: u32,
) -> DecentralizationMonitoring {
    let network_count = network_session_counts.len();

    if total_sessions == 0 || network_count == 0 {
        return DecentralizationMonitoring {
            healthy: true,
            hhi_score: 0,
            dominant_share_bps: 0,
            network_count,
            risk_score: 0,
        };
    }

    // Compute HHI = Σ (share_i)^2  on a 0-10_000 scale.
    // share_i = (count_i * 10_000 / total_sessions)
    let mut hhi: u32 = 0;
    let mut dominant_share_bps: u32 = 0;

    for i in 0..network_count {
        let count = network_session_counts.get(i).unwrap_or(0);
        let share_bps = (count.saturating_mul(10_000)) / total_sessions.max(1);
        if share_bps > dominant_share_bps {
            dominant_share_bps = share_bps;
        }
        // HHI component = share^2 / 10_000 to stay in 0-10_000 range
        hhi = hhi.saturating_add((share_bps.saturating_mul(share_bps)) / 10_000);
    }

    // Risk scoring
    let mut risk: u32 = 0;
    if hhi >= HHI_CRITICAL_THRESHOLD {
        risk = risk.saturating_add(60);
    } else if hhi >= HHI_HIGH_CONCENTRATION_THRESHOLD {
        risk = risk.saturating_add(35);
    }
    if dominant_share_bps > MAX_SINGLE_NETWORK_SHARE_BPS {
        risk = risk.saturating_add(30);
    }
    if network_count < MIN_COMPETITIVE_NETWORK_COUNT {
        risk = risk.saturating_add(20);
    }
    risk = risk.min(100);

    DecentralizationMonitoring {
        healthy: risk < MARKET_CONCENTRATION_RISK_THRESHOLD,
        hhi_score: hhi,
        dominant_share_bps,
        network_count,
        risk_score: risk,
    }
}

/// Prevent market control: verify that the concentration level is within
/// acceptable bounds and return whether intervention is warranted.
pub fn prevent_market_control(monitoring: DecentralizationMonitoring) -> bool {
    monitoring.risk_score >= MARKET_CONCENTRATION_RISK_THRESHOLD
}

// ---------------------------------------------------------------------------
// Competition protection
// ---------------------------------------------------------------------------

/// Assess whether independent mentors have fair access to the market.
///
/// `independent_mentor_count` is the count of non-network-affiliated active
/// mentors. `total_active_mentors` includes both network and independent
/// mentors. `barrier_signal_count` is the number of externally detected
/// barrier signals (e.g., coordinated review bombing, price undercutting).
pub fn assess_competition_barriers(
    env: &Env,
    independent_mentor_count: u32,
    total_active_mentors: u32,
    barrier_signal_count: u32,
) -> CompetitionProtection {
    if total_active_mentors == 0 {
        return CompetitionProtection {
            fair_access: true,
            independent_ratio_bps: 10_000,
            barriers_detected: false,
            risk_score: 0,
            reason: Symbol::new(env, "no_mentors"),
        };
    }

    let independent_ratio_bps =
        (independent_mentor_count.saturating_mul(10_000)) / total_active_mentors.max(1);

    let barriers_detected = barrier_signal_count > 0
        || independent_ratio_bps < INDEPENDENT_MENTOR_MIN_RATIO_BPS;

    let mut risk: u32 = 0;
    if independent_ratio_bps < INDEPENDENT_MENTOR_MIN_RATIO_BPS {
        // Scale risk by how far below the threshold the ratio is
        let deficit_bps = INDEPENDENT_MENTOR_MIN_RATIO_BPS
            .saturating_sub(independent_ratio_bps);
        risk = risk.saturating_add((deficit_bps / 100).min(50));
    }
    if barrier_signal_count >= 3 {
        risk = risk.saturating_add(40);
    } else if barrier_signal_count >= 1 {
        risk = risk.saturating_add(20);
    }
    risk = risk.min(100);

    let (fair_access, reason) = if risk >= MARKET_CONCENTRATION_RISK_THRESHOLD {
        (false, Symbol::new(env, "barriers_high_risk"))
    } else if barriers_detected {
        (false, Symbol::new(env, "barriers_detected"))
    } else {
        (true, Symbol::new(env, "fair_access"))
    };

    CompetitionProtection {
        fair_access,
        independent_ratio_bps,
        barriers_detected,
        risk_score: risk,
        reason,
    }
}

/// Support independent mentor access: determine whether the market state
/// calls for active support measures (e.g., fee reductions, visibility boosts).
pub fn support_independent_mentors(protection: &CompetitionProtection) -> bool {
    !protection.fair_access || protection.barriers_detected
}

// ---------------------------------------------------------------------------
// Market fairness – pricing coordination detection
// ---------------------------------------------------------------------------

/// Detect pricing coordination across mentor networks.
///
/// `price_timestamps` contains the timestamps of recent price-change events.
/// `price_changes_bps` contains the corresponding price-change magnitudes
/// (as basis points of deviation from the previous price). Both slices must
/// be the same length and sorted chronologically.
pub fn detect_pricing_coordination(
    price_timestamps: &Vec<u64>,
    price_changes_bps: &Vec<u32>,
) -> MarketFairness {
    let n = price_timestamps.len().min(price_changes_bps.len());
    if n < 2 {
        return MarketFairness {
            fair_pricing: true,
            coordination_detected: false,
            suspicious_price_moves: 0,
            risk_score: 0,
        };
    }

    let mut suspicious_moves: u32 = 0;

    for i in 1..n {
        let t_prev = price_timestamps.get(i - 1).unwrap_or(0);
        let t_cur = price_timestamps.get(i).unwrap_or(t_prev);
        let change_prev = price_changes_bps.get(i - 1).unwrap_or(0);
        let change_cur = price_changes_bps.get(i).unwrap_or(0);

        // Flag if two price moves happen within the coordination window AND
        // their magnitudes are suspiciously similar (within tolerance).
        let within_window =
            t_cur.saturating_sub(t_prev) <= MARKET_PRICE_COORDINATION_WINDOW_SECS;
        let diff = if change_cur > change_prev {
            change_cur.saturating_sub(change_prev)
        } else {
            change_prev.saturating_sub(change_cur)
        };
        let similar = diff <= MARKET_PRICE_SIMILARITY_BPS;

        if within_window && similar {
            suspicious_moves = suspicious_moves.saturating_add(1);
        }
    }

    let mut risk: u32 = 0;
    if suspicious_moves >= 5 {
        risk = risk.saturating_add(70);
    } else if suspicious_moves >= 3 {
        risk = risk.saturating_add(45);
    } else if suspicious_moves >= 1 {
        risk = risk.saturating_add(20);
    }
    risk = risk.min(100);

    let coordination_detected = risk >= PRICE_COORDINATION_RISK_THRESHOLD;

    MarketFairness {
        fair_pricing: !coordination_detected,
        coordination_detected,
        suspicious_price_moves: suspicious_moves,
        risk_score: risk,
    }
}

/// Prevent oligopoly: check whether the combined concentration and pricing
/// coordination signals indicate oligopolistic market control.
pub fn prevent_oligopoly(
    monitoring: &DecentralizationMonitoring,
    fairness: &MarketFairness,
) -> bool {
    monitoring.risk_score >= MARKET_CONCENTRATION_RISK_THRESHOLD
        || fairness.coordination_detected
}

// ---------------------------------------------------------------------------
// Network analysis
// ---------------------------------------------------------------------------

/// Combine concentration measurement and competition-barrier signals into a
/// comprehensive network-analysis result.
pub fn analyze_market_networks(
    monitoring: &DecentralizationMonitoring,
    protection: &CompetitionProtection,
    fairness: &MarketFairness,
) -> NetworkAnalysisResult {
    // Centralization score: blend HHI (0-10_000 scaled to 0-100) and
    // dominant-share risk.
    let hhi_component = (monitoring.hhi_score / 100).min(100);
    let centralization_score = (hhi_component
        .saturating_add(monitoring.risk_score))
        / 2;

    // Count distinct manipulation signals
    let mut signal_count: u32 = 0;
    if monitoring.risk_score >= MARKET_CONCENTRATION_RISK_THRESHOLD {
        signal_count = signal_count.saturating_add(1);
    }
    if protection.barriers_detected {
        signal_count = signal_count.saturating_add(1);
    }
    if fairness.coordination_detected {
        signal_count = signal_count.saturating_add(1);
    }
    if monitoring.network_count < MIN_COMPETITIVE_NETWORK_COUNT {
        signal_count = signal_count.saturating_add(1);
    }

    let manipulation_identified = signal_count >= 2;

    // Market health score: 100 minus average of three risk components.
    let avg_risk = (monitoring
        .risk_score
        .saturating_add(protection.risk_score)
        .saturating_add(fairness.risk_score))
        / 3;
    let market_health_score = 100u32.saturating_sub(avg_risk);

    NetworkAnalysisResult {
        centralization_score,
        manipulation_identified,
        manipulation_signal_count: signal_count,
        market_health_score,
    }
}

// ---------------------------------------------------------------------------
// Competition audit
// ---------------------------------------------------------------------------

/// Produce a competition audit record from the sub-component results.
pub fn audit_market_competition(
    monitoring: &DecentralizationMonitoring,
    protection: &CompetitionProtection,
    fairness: &MarketFairness,
    analysis: &NetworkAnalysisResult,
) -> CompetitionAuditRecord {
    let mut violation_count: u32 = 0;
    if !monitoring.healthy {
        violation_count = violation_count.saturating_add(1);
    }
    if !protection.fair_access {
        violation_count = violation_count.saturating_add(1);
    }
    if !fairness.fair_pricing {
        violation_count = violation_count.saturating_add(1);
    }
    if analysis.manipulation_identified {
        violation_count = violation_count.saturating_add(1);
    }

    // Fairness score: blend market health and inversion of violation severity.
    let fairness_score = analysis
        .market_health_score
        .saturating_sub(violation_count.saturating_mul(10))
        .min(100);

    CompetitionAuditRecord {
        compliant: violation_count == 0,
        violation_count,
        fairness_score,
        market_control_detected: analysis.manipulation_identified
            || !monitoring.healthy,
    }
}

// ---------------------------------------------------------------------------
// Automatic market protection & restoration
// ---------------------------------------------------------------------------

/// Combine all market-control signals into a single automatic protection
/// intervention decision.
pub fn compute_market_protection_intervention(
    env: &Env,
    monitoring: &DecentralizationMonitoring,
    protection: &CompetitionProtection,
    fairness: &MarketFairness,
    restoration_cooldown_secs: u64,
) -> MarketProtectionRecord {
    let combined = (monitoring
        .risk_score
        .saturating_add(protection.risk_score)
        .saturating_add(fairness.risk_score))
        / 3;
    let combined = combined.min(100);

    let (intervene, reason) = if !monitoring.healthy
        && monitoring.risk_score >= MARKET_CONCENTRATION_RISK_THRESHOLD
    {
        (true, Symbol::new(env, "high_concentration"))
    } else if !protection.fair_access {
        (true, Symbol::new(env, "barriers_blocking"))
    } else if fairness.coordination_detected {
        (true, Symbol::new(env, "price_coordination"))
    } else if combined >= MARKET_PROTECTION_INTERVENTION_THRESHOLD {
        (true, Symbol::new(env, "combined_risk"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    let now = env.ledger().timestamp();
    MarketProtectionRecord {
        intervene,
        combined_risk_score: combined,
        reason,
        restoration_eligible_at: if intervene {
            now.saturating_add(restoration_cooldown_secs)
        } else {
            now
        },
    }
}

/// Whether a previously-intervened market is now eligible for automatic
/// competitive-balance restoration.
pub fn is_market_restoration_eligible(record: &MarketProtectionRecord, now: u64) -> bool {
    !record.intervene || now >= record.restoration_eligible_at
}

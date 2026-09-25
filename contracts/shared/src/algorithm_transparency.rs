//! Platform algorithm transparency and black-box exploitation protection primitives.
//!
//! Users may exploit lack of algorithm transparency to reverse-engineer
//! recommendation systems, manipulate ranking algorithms, coordinate gaming
//! strategies, or exploit black-box behaviours to gain unfair advantages in
//! mentor visibility and learner matching.
//!
//! These helpers give contracts a deterministic, storage-agnostic way to:
//!
//!   * produce explainable-AI-style factor breakdowns for ranking decisions
//!     so legitimate contributors understand how scores are computed;
//!   * detect reverse-engineering attempts by monitoring probe patterns
//!     (high-frequency score queries with systematically varied inputs);
//!   * balance transparency with gaming prevention by controlling how much
//!     detail is disclosed and to whom;
//!   * monitor algorithm operation for manipulation signals; and
//!   * produce automatic adjustment decisions with restoration eligibility.
//!
//! Contracts own the storage of raw query/probe history; these functions are
//! pure scoring/decision logic over data the caller already holds.

use soroban_sdk::{contracttype, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Time window (seconds) within which repeated score-probing queries from a
/// single address are tracked for reverse-engineering detection.
pub const PROBE_DETECTION_WINDOW_SECS: u64 = 3_600; // 1 hour

/// Number of distinct probes within the detection window above which an
/// address is considered to be reverse-engineering the algorithm.
pub const PROBE_HIGH_FREQUENCY_THRESHOLD: u32 = 20;

/// Risk score (0-100) at or above which a probe pattern is flagged as a
/// reverse-engineering attempt.
pub const REVERSE_ENGINEERING_RISK_THRESHOLD: u32 = 60;

/// Maximum basis-point fraction of the scoring formula that may be disclosed
/// in a single transparency response (prevents full formula reconstruction).
pub const MAX_TRANSPARENCY_DISCLOSURE_BPS: u32 = 7_000; // 70% of factor weights

/// Minimum basis-point fraction of the scoring formula that must be disclosed
/// to satisfy the explainability requirement.
pub const MIN_TRANSPARENCY_DISCLOSURE_BPS: u32 = 3_000; // 30%

/// Number of distinct factor categories used in ranking/recommendation scores.
/// Keeping this explicit helps consumers understand the explanation depth.
pub const RANKING_FACTOR_COUNT: u32 = 5;

/// Risk score (0-100) at or above which a manipulation signal triggers the
/// automatic algorithm-protection intervention.
pub const ALGO_MANIPULATION_RISK_THRESHOLD: u32 = 65;

/// Cooldown (seconds) before an algorithm that was placed under protection
/// is eligible for automatic fair-operation restoration.
pub const ALGO_PROTECTION_COOLDOWN_SECS: u64 = 3_600; // 1 hour

/// Combined risk score at or above which the algorithm protection layer
/// automatically intervenes.
pub const ALGO_INTERVENTION_THRESHOLD: u32 = 65;

/// Window (seconds) over which ranking-score variance is measured to detect
/// coordinated gaming (multiple actors inflating scores simultaneously).
pub const RANKING_GAMING_WINDOW_SECS: u64 = 1_800; // 30 minutes

/// Maximum acceptable ranking-score deviation (basis points above baseline)
/// before a gaming flag is raised.
pub const RANKING_SCORE_DEVIATION_BPS: u32 = 3_000; // 30%

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Explainable-AI factor breakdown for a single ranking or recommendation
/// score, containing the contribution of each scoring component.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlgorithmTransparency {
    /// Whether the algorithm is operating fairly (no gaming detected).
    pub fair_operation: bool,
    /// Composite transparency score (0-100, higher = more explainable).
    pub transparency_score: u32,
    /// Fraction of the scoring formula disclosed (basis points, 0-10_000).
    pub disclosure_bps: u32,
    /// Number of scoring factors included in the explanation.
    pub factor_count: u32,
    /// Whether manipulation was detected in recent interactions.
    pub manipulation_detected: bool,
}

/// Result of analysing probe patterns for reverse-engineering attempts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReverseEngineeringProtection {
    /// Whether the probe pattern is within acceptable bounds.
    pub safe: bool,
    /// Number of probes observed in the detection window.
    pub probe_count: u32,
    /// Estimated risk that the probes represent a reverse-engineering attempt.
    pub risk_score: u32,
    /// Whether the probes appear systematically varied (adversarial).
    pub systematic_variation: bool,
}

/// Balance between transparency (explainability) and gaming prevention
/// (limited disclosure).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransparencyBalance {
    /// Whether the current disclosure level is within the safe range.
    pub balanced: bool,
    /// Actual disclosure fraction chosen (basis points).
    pub chosen_disclosure_bps: u32,
    /// Whether full disclosure was requested but capped for gaming prevention.
    pub capped: bool,
    /// Human-readable reason for the chosen disclosure level.
    pub reason: Symbol,
}

/// Result of monitoring algorithm operation for manipulation signals.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AlgorithmMonitoringResult {
    /// Whether the algorithm is operating within expected bounds.
    pub operating_normally: bool,
    /// Number of suspicious ranking-score deviations detected.
    pub suspicious_deviations: u32,
    /// Combined manipulation risk score (0-100).
    pub manipulation_risk_score: u32,
    /// Whether a coordinated gaming pattern was identified.
    pub coordinated_gaming: bool,
}

/// Comprehensive algorithm transparency audit record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransparencyAuditRecord {
    /// Whether the audit found the algorithm compliant.
    pub compliant: bool,
    /// Number of distinct fairness violations detected.
    pub violation_count: u32,
    /// Algorithm fairness score (0-100, higher is fairer).
    pub fairness_score: u32,
    /// Whether algorithm security was verified (no active exploitation).
    pub security_verified: bool,
}

/// Automatic algorithm-protection intervention outcome.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AlgorithmProtectionRecord {
    /// Whether automatic protection adjustment is active.
    pub intervention_active: bool,
    /// Combined risk score driving the intervention decision (0-100).
    pub combined_risk_score: u32,
    /// Primary reason for the intervention.
    pub reason: Symbol,
    /// Timestamp after which fair-operation restoration is eligible.
    pub restoration_eligible_at: u64,
}

// ---------------------------------------------------------------------------
// Explainable AI / transparency
// ---------------------------------------------------------------------------

/// Produce an algorithm-transparency assessment.
///
/// `factor_weights_bps` contains the contribution weights (in basis points,
/// summing to ≤10_000) of each scoring factor that is safe to disclose.
/// `total_factors` is the total number of factors in the full formula (some
/// may be withheld for gaming prevention). `manipulation_signal_count` is
/// the number of active manipulation signals observed in the current window.
pub fn assess_algorithm_transparency(
    factor_weights_bps: &Vec<u32>,
    total_factors: u32,
    manipulation_signal_count: u32,
) -> AlgorithmTransparency {
    let disclosed_factors = factor_weights_bps.len();
    let factor_count = disclosed_factors.min(total_factors);

    // Compute total disclosed weight.
    let mut disclosure_bps: u32 = 0;
    for i in 0..disclosed_factors {
        disclosure_bps =
            disclosure_bps.saturating_add(factor_weights_bps.get(i).unwrap_or(0));
    }
    disclosure_bps = disclosure_bps.min(10_000);

    // Transparency score: ratio of disclosed factors × quality bonus.
    let factor_ratio_bps = if total_factors == 0 {
        10_000
    } else {
        (factor_count.saturating_mul(10_000)) / total_factors.max(1)
    };
    let transparency_score = ((factor_ratio_bps / 100) + (disclosure_bps / 200)).min(100);

    let manipulation_detected = manipulation_signal_count > 0;

    AlgorithmTransparency {
        fair_operation: !manipulation_detected
            && disclosure_bps >= MIN_TRANSPARENCY_DISCLOSURE_BPS,
        transparency_score,
        disclosure_bps,
        factor_count,
        manipulation_detected,
    }
}

// ---------------------------------------------------------------------------
// Reverse-engineering protection
// ---------------------------------------------------------------------------

/// Analyse probe timestamps from a single address to determine whether the
/// probing pattern represents a reverse-engineering attempt.
///
/// `probe_timestamps` should be the timestamps of score-query calls made by
/// the address within the current `PROBE_DETECTION_WINDOW_SECS`. The
/// `distinct_input_variations` parameter captures how many systematically
/// distinct inputs were used (higher values indicate adversarial probing).
pub fn detect_reverse_engineering(
    probe_timestamps: &Vec<u64>,
    distinct_input_variations: u32,
) -> ReverseEngineeringProtection {
    let probe_count = probe_timestamps.len();

    // Systematic variation: if most probes use distinct inputs, it is likely
    // an adversarial sweep of the input space.
    let systematic_variation = probe_count > 0
        && distinct_input_variations >= probe_count.saturating_mul(7) / 10; // ≥70% variation

    let mut risk: u32 = 0;
    if probe_count >= PROBE_HIGH_FREQUENCY_THRESHOLD * 2 {
        risk = risk.saturating_add(60);
    } else if probe_count >= PROBE_HIGH_FREQUENCY_THRESHOLD {
        risk = risk.saturating_add(35);
    } else if probe_count >= PROBE_HIGH_FREQUENCY_THRESHOLD / 2 {
        risk = risk.saturating_add(15);
    }
    if systematic_variation {
        risk = risk.saturating_add(35);
    }
    risk = risk.min(100);

    ReverseEngineeringProtection {
        safe: risk < REVERSE_ENGINEERING_RISK_THRESHOLD,
        probe_count,
        risk_score: risk,
        systematic_variation,
    }
}

/// Check whether an address should be blocked from receiving further score
/// transparency responses based on its probing history.
pub fn should_block_transparency_response(protection: &ReverseEngineeringProtection) -> bool {
    !protection.safe
}

// ---------------------------------------------------------------------------
// Transparency balance
// ---------------------------------------------------------------------------

/// Choose a disclosure level that satisfies explainability requirements while
/// preventing gaming.
///
/// `requested_disclosure_bps` is the fraction the caller would like disclosed.
/// `probe_risk_score` is the probing risk for the requesting address.
/// Returns the chosen disclosure level and a reason.
pub fn compute_transparency_balance(
    env: &Env,
    requested_disclosure_bps: u32,
    probe_risk_score: u32,
) -> TransparencyBalance {
    // Under active probing pressure, cap the disclosure.
    let effective_max = if probe_risk_score >= REVERSE_ENGINEERING_RISK_THRESHOLD {
        MIN_TRANSPARENCY_DISCLOSURE_BPS // only minimum disclosure allowed
    } else {
        MAX_TRANSPARENCY_DISCLOSURE_BPS
    };

    let capped = requested_disclosure_bps > effective_max;
    let chosen_disclosure_bps = requested_disclosure_bps.min(effective_max);

    let (balanced, reason) = if chosen_disclosure_bps < MIN_TRANSPARENCY_DISCLOSURE_BPS {
        (
            false,
            Symbol::new(env, "below_min_disclosure"),
        )
    } else if capped {
        (
            true,
            Symbol::new(env, "capped_for_security"),
        )
    } else {
        (
            true,
            Symbol::new(env, "balanced"),
        )
    };

    TransparencyBalance {
        balanced,
        chosen_disclosure_bps,
        capped,
        reason,
    }
}

// ---------------------------------------------------------------------------
// Algorithm monitoring
// ---------------------------------------------------------------------------

/// Monitor ranking-score time-series for manipulation signals.
///
/// `score_timestamps` contains the timestamps of recent score-update events.
/// `score_deviations_bps` contains the corresponding deviation of each score
/// from the mentor's baseline (as basis points above/below). Both slices
/// must be the same length and sorted chronologically.
/// `baseline_bps` is the expected normal score level in basis points (e.g.
/// 5_000 = 50%).
pub fn monitor_ranking_algorithm(
    score_timestamps: &Vec<u64>,
    score_deviations_bps: &Vec<u32>,
    coordinated_actor_count: u32,
) -> AlgorithmMonitoringResult {
    let n = score_timestamps
        .len()
        .min(score_deviations_bps.len());

    if n == 0 {
        return AlgorithmMonitoringResult {
            operating_normally: true,
            suspicious_deviations: 0,
            manipulation_risk_score: 0,
            coordinated_gaming: false,
        };
    }

    // Count deviations that exceed the gaming threshold.
    let mut suspicious: u32 = 0;
    for i in 0..n {
        let dev = score_deviations_bps.get(i).unwrap_or(0);
        if dev > RANKING_SCORE_DEVIATION_BPS {
            suspicious = suspicious.saturating_add(1);
        }
    }

    // Detect coordinated gaming: multiple actors acting near-simultaneously.
    let coordinated_gaming = coordinated_actor_count >= 3 && suspicious >= 2;

    let mut risk: u32 = 0;
    if suspicious >= n as u32 / 2 {
        risk = risk.saturating_add(50);
    } else if suspicious >= 2 {
        risk = risk.saturating_add(25);
    }
    if coordinated_gaming {
        risk = risk.saturating_add(35);
    }
    risk = risk.min(100);

    AlgorithmMonitoringResult {
        operating_normally: risk < ALGO_MANIPULATION_RISK_THRESHOLD,
        suspicious_deviations: suspicious,
        manipulation_risk_score: risk,
        coordinated_gaming,
    }
}

// ---------------------------------------------------------------------------
// Transparency audit
// ---------------------------------------------------------------------------

/// Produce a comprehensive transparency audit from the sub-component results.
pub fn audit_algorithm_transparency(
    transparency: &AlgorithmTransparency,
    protection: &ReverseEngineeringProtection,
    balance: &TransparencyBalance,
    monitoring: &AlgorithmMonitoringResult,
) -> TransparencyAuditRecord {
    let mut violations: u32 = 0;
    if !transparency.fair_operation {
        violations = violations.saturating_add(1);
    }
    if !protection.safe {
        violations = violations.saturating_add(1);
    }
    if !balance.balanced {
        violations = violations.saturating_add(1);
    }
    if !monitoring.operating_normally {
        violations = violations.saturating_add(1);
    }

    // Fairness score: algorithm health minus violation penalty.
    let base_fairness = transparency.transparency_score;
    let fairness_score = base_fairness
        .saturating_sub(violations.saturating_mul(15))
        .min(100);

    TransparencyAuditRecord {
        compliant: violations == 0,
        violation_count: violations,
        fairness_score,
        security_verified: protection.safe && !monitoring.coordinated_gaming,
    }
}

// ---------------------------------------------------------------------------
// Automatic protection & restoration
// ---------------------------------------------------------------------------

/// Combine all algorithm-protection signals into a single automatic
/// intervention decision.
pub fn compute_algo_protection_intervention(
    env: &Env,
    transparency: &AlgorithmTransparency,
    protection: &ReverseEngineeringProtection,
    monitoring: &AlgorithmMonitoringResult,
    restoration_cooldown_secs: u64,
) -> AlgorithmProtectionRecord {
    let combined = (transparency
        .transparency_score
        .saturating_sub(protection.risk_score) // transparency reduces risk
        .saturating_add(monitoring.manipulation_risk_score))
        / 2;
    // Invert: high transparency → lower combined risk
    let combined_risk = 100u32.saturating_sub(combined).min(100);
    let combined_risk = combined_risk
        .max(protection.risk_score / 2)
        .max(monitoring.manipulation_risk_score / 2);

    let (intervene, reason) = if !protection.safe {
        (true, Symbol::new(env, "reverse_engineering"))
    } else if monitoring.coordinated_gaming {
        (true, Symbol::new(env, "coordinated_gaming"))
    } else if transparency.manipulation_detected {
        (true, Symbol::new(env, "manipulation_detected"))
    } else if combined_risk >= ALGO_INTERVENTION_THRESHOLD {
        (true, Symbol::new(env, "combined_risk"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    let now = env.ledger().timestamp();
    AlgorithmProtectionRecord {
        intervention_active: intervene,
        combined_risk_score: combined_risk,
        reason,
        restoration_eligible_at: if intervene {
            now.saturating_add(restoration_cooldown_secs)
        } else {
            now
        },
    }
}

/// Whether a previously-intervened algorithm is eligible for fair-operation
/// restoration.
pub fn is_algo_restoration_eligible(record: &AlgorithmProtectionRecord, now: u64) -> bool {
    !record.intervention_active || now >= record.restoration_eligible_at
}

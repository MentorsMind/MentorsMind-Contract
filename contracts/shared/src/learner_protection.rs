//! Learner vulnerability and predatory-mentoring protection primitives.
//!
//! Vulnerable learners – those under financial pressure, new to a subject,
//! or emotionally invested in rapid progress – can be exploited by predatory
//! mentors who inflate prices, manufacture urgency, or engage in
//! psychologically manipulative behaviour. These helpers give contracts a
//! deterministic, storage-agnostic way to:
//!
//!   * assess learner vulnerability from observable on-chain signals;
//!   * score mentor behaviour patterns for predatory indicators;
//!   * enforce per-learner price fairness (affordability caps);
//!   * track exploitation patterns for transparency; and
//!   * produce an emergency-intervention decision when the combined risk
//!     is high enough to warrant immediate action (mentor suspension).
//!
//! Contracts own the storage of raw interaction history; these functions are
//! pure scoring/decision logic over data the caller already holds.

use soroban_sdk::{contracttype, Symbol};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Number of sessions in a rolling window used to assess learner recurrence
/// patterns.  A learner booking many sessions in a short span with the same
/// mentor may be under pressure or dependent.
pub const VULNERABILITY_SESSION_WINDOW: u32 = 5;

/// Sessions-per-window rate above which a learner is considered at elevated
/// risk of dependency-based exploitation.
pub const VULNERABILITY_HIGH_RECURRENCE_THRESHOLD: u32 = 4;

/// Risk score (0-100) at or above which a vulnerability assessment flags a
/// learner as "at risk" and activates protection mechanisms.
pub const VULNERABILITY_RISK_THRESHOLD: u32 = 60;

/// Price deviation (basis points) above the learner's historical average
/// spend that triggers an affordability concern.
pub const AFFORDABILITY_DEVIATION_BPS: u32 = 5_000; // 50%

/// Maximum allowed session price for a learner flagged as financially
/// vulnerable, expressed as a multiplier (basis points) over their average
/// spend.  Prices above this cap are rejected as exploitative.
pub const FINANCIAL_PROTECTION_CAP_BPS: u32 = 15_000; // 150% of avg (i.e. 1.5×)

/// Number of consecutive low-quality sessions (rating ≤ 2) from the same
/// mentor that constitute a predatory-quality pattern.
pub const PREDATORY_LOW_QUALITY_THRESHOLD: u32 = 3;

/// Ratio of complaints/disputes to total sessions (basis points) above which
/// a mentor is considered predatory.
pub const PREDATORY_COMPLAINT_RATIO_BPS: u32 = 3_000; // 30%

/// Risk score (0-100) at or above which a mentor's behaviour triggers
/// automatic intervention.
pub const PREDATORY_RISK_THRESHOLD: u32 = 65;

/// Number of identified exploitation patterns that directly triggers an
/// emergency intervention recommendation.
pub const EMERGENCY_PATTERN_THRESHOLD: u32 = 2;

/// Cooldown (seconds) before a mentor who has been suspended via emergency
/// intervention is eligible for reinstatement review.
pub const EMERGENCY_SUSPENSION_COOLDOWN_SECS: u64 = 30 * 24 * 3_600; // 30 days

/// Cooldown (seconds) before a learner-protection intervention can be
/// auto-restored (7 days – mirrors community_protection cooldown).
pub const LEARNER_PROTECTION_COOLDOWN_SECS: u64 = 7 * 24 * 3_600;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of assessing a learner's vulnerability from on-chain signals.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct VulnerabilityAssessment {
    /// Whether this learner is considered at risk and should have active
    /// protection mechanisms enabled.
    pub at_risk: bool,
    /// Composite risk score in 0-100.
    pub risk_score: u32,
    /// The learner books the same mentor frequently (dependency risk).
    pub high_recurrence: bool,
    /// The learner's latest session price significantly exceeds their
    /// historical spending average (financial-exploitation signal).
    pub affordability_concern: bool,
    /// Number of consecutive sessions with the same mentor in the window.
    pub recurrence_count: u32,
}

/// Result of scoring a mentor's behaviour for predatory indicators.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PredatoryBehaviorDetection {
    /// Whether this mentor's behaviour is classified as predatory.
    pub predatory: bool,
    /// Composite risk score in 0-100.
    pub risk_score: u32,
    /// Mentor has delivered multiple consecutive low-quality sessions.
    pub low_quality_pattern: bool,
    /// Mentor has an abnormally high dispute/complaint rate.
    pub high_complaint_rate: bool,
    /// Mentor has charged prices significantly above the platform average
    /// to learners that exhibit vulnerability signals.
    pub price_exploitation_flag: bool,
    /// Raw complaint count observed.
    pub complaint_count: u32,
    /// Total sessions observed.
    pub total_sessions: u32,
}

/// A single identified exploitation pattern linking a mentor to a learner.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExploitationPattern {
    /// Human-readable pattern label (e.g. "price_gouging", "dependency_trap").
    pub pattern_type: Symbol,
    /// Risk contribution of this pattern in 0-100.
    pub severity: u32,
    /// Whether this pattern alone is severe enough to warrant immediate action.
    pub immediate_action_required: bool,
}

/// Welfare status summary for a learner.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WelfareStatus {
    /// Overall welfare is considered healthy when false.
    pub support_required: bool,
    /// Combined welfare risk score in 0-100.
    pub welfare_risk_score: u32,
    /// Advocacy or support services should be activated.
    pub activate_support_services: bool,
    /// How many active exploitation patterns were detected.
    pub active_pattern_count: u32,
}

/// Emergency intervention record produced when combined risk is critical.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyIntervention {
    /// Immediate suspension of the implicated mentor is recommended.
    pub suspend_mentor: bool,
    /// Learner's in-progress sessions should be halted / refunded.
    pub halt_active_sessions: bool,
    /// Combined risk score that triggered this decision.
    pub combined_risk_score: u32,
    /// Symbolic reason label.
    pub reason: Symbol,
    /// Ledger timestamp after which the mentor may request reinstatement.
    pub reinstatement_eligible_at: u64,
}

/// Top-level learner-protection intervention record persisted per mentor.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LearnerProtectionRecord {
    /// Whether an active intervention is in place.
    pub intervene: bool,
    /// Combined risk score at the time of the last assessment.
    pub combined_risk_score: u32,
    /// Symbolic primary reason.
    pub reason: Symbol,
    /// Ledger timestamp after which the intervention can be auto-restored.
    pub restoration_eligible_at: u64,
    /// Whether an emergency suspension was issued.
    pub emergency_suspension: bool,
}

// ---------------------------------------------------------------------------
// Vulnerability assessment
// ---------------------------------------------------------------------------

/// Assess a learner's vulnerability from observable on-chain signals.
///
/// * `session_count_with_mentor` – number of sessions this learner has booked
///   with this specific mentor in the rolling window.
/// * `latest_session_price` – the price (in token units) of the learner's
///   most recent session.
/// * `avg_historical_price` – the learner's average session price across all
///   mentors (0 = no history, skip affordability check).
pub fn assess_vulnerability(
    session_count_with_mentor: u32,
    latest_session_price: i128,
    avg_historical_price: i128,
) -> VulnerabilityAssessment {
    let high_recurrence = session_count_with_mentor >= VULNERABILITY_HIGH_RECURRENCE_THRESHOLD;

    let affordability_concern = if avg_historical_price > 0 && latest_session_price > 0 {
        let allowed = avg_historical_price
            + (avg_historical_price * AFFORDABILITY_DEVIATION_BPS as i128) / 10_000;
        latest_session_price > allowed
    } else {
        false
    };

    let mut risk = 0u32;
    if session_count_with_mentor >= VULNERABILITY_SESSION_WINDOW {
        risk = risk.saturating_add(25);
    }
    if high_recurrence {
        risk = risk.saturating_add(40);
    }
    if affordability_concern {
        risk = risk.saturating_add(35);
    }
    risk = risk.min(100);

    VulnerabilityAssessment {
        at_risk: risk >= VULNERABILITY_RISK_THRESHOLD,
        risk_score: risk,
        high_recurrence,
        affordability_concern,
        recurrence_count: session_count_with_mentor,
    }
}

// ---------------------------------------------------------------------------
// Predatory behaviour detection
// ---------------------------------------------------------------------------

/// Score a mentor's aggregate behaviour for predatory indicators.
///
/// * `consecutive_low_quality_sessions` – count of consecutive sessions where
///   the learner-given rating was ≤ 2 (sustained underdelivery).
/// * `complaint_count` – total disputes/complaints filed against this mentor.
/// * `total_sessions` – total completed sessions for this mentor.
/// * `price_above_market_bps` – how far above the platform-average price this
///   mentor charges learners who are flagged as vulnerable (basis points).
pub fn detect_predatory_behavior(
    consecutive_low_quality_sessions: u32,
    complaint_count: u32,
    total_sessions: u32,
    price_above_market_bps: u32,
) -> PredatoryBehaviorDetection {
    let low_quality_pattern = consecutive_low_quality_sessions >= PREDATORY_LOW_QUALITY_THRESHOLD;

    let complaint_ratio_bps = if total_sessions == 0 {
        0
    } else {
        (complaint_count.saturating_mul(10_000)) / total_sessions
    };
    let high_complaint_rate = complaint_ratio_bps >= PREDATORY_COMPLAINT_RATIO_BPS;

    // A mentor who charges vulnerable learners significantly more than the
    // market rate is a price-exploitation signal.
    let price_exploitation_flag = price_above_market_bps > AFFORDABILITY_DEVIATION_BPS;

    let mut risk = 0u32;
    if low_quality_pattern {
        risk = risk.saturating_add(35);
    }
    if high_complaint_rate {
        risk = risk.saturating_add(30);
    }
    if price_exploitation_flag {
        risk = risk.saturating_add(25);
    }
    // Compound risk: all three signals together push above the threshold.
    if low_quality_pattern && high_complaint_rate && price_exploitation_flag {
        risk = risk.saturating_add(15);
    }
    risk = risk.min(100);

    PredatoryBehaviorDetection {
        predatory: risk >= PREDATORY_RISK_THRESHOLD,
        risk_score: risk,
        low_quality_pattern,
        high_complaint_rate,
        price_exploitation_flag,
        complaint_count,
        total_sessions,
    }
}

// ---------------------------------------------------------------------------
// Fair pricing enforcement for vulnerable learners
// ---------------------------------------------------------------------------

/// Enforce affordability-based price protection for a learner flagged as
/// financially vulnerable.
///
/// When `vulnerability.affordability_concern` is set, the price is clamped
/// to `avg_historical_price * FINANCIAL_PROTECTION_CAP_BPS / 10_000`.
/// When `vulnerability.at_risk` is set without an affordability concern,
/// an additional soft ceiling equal to `platform_avg_price * 2` is applied.
/// Returns `(enforced_price, was_adjusted)`.
pub fn enforce_learner_fair_pricing(
    proposed_price: i128,
    avg_historical_price: i128,
    platform_avg_price: i128,
    vulnerability: VulnerabilityAssessment,
) -> (i128, bool) {
    if proposed_price <= 0 {
        return (proposed_price, false);
    }

    // Hard cap based on learner's own spending history.
    if vulnerability.affordability_concern && avg_historical_price > 0 {
        let cap =
            (avg_historical_price * FINANCIAL_PROTECTION_CAP_BPS as i128) / 10_000;
        if proposed_price > cap {
            return (cap, true);
        }
    }

    // Soft ceiling: at-risk learners are shielded from extreme platform
    // outlier prices (more than 2× platform average).
    if vulnerability.at_risk && platform_avg_price > 0 {
        let soft_cap = platform_avg_price.saturating_mul(2);
        if proposed_price > soft_cap {
            return (soft_cap, true);
        }
    }

    (proposed_price, false)
}

// ---------------------------------------------------------------------------
// Exploitation pattern identification
// ---------------------------------------------------------------------------

/// Identify active exploitation patterns from vulnerability and behaviour
/// assessment results.  Returns a fixed-size vector of detected patterns
/// (the SDK's `Vec` is owned by `env`; callers wanting a plain slice can
/// iterate the returned `Vec`).
pub fn identify_exploitation_patterns(
    env: &soroban_sdk::Env,
    vulnerability: VulnerabilityAssessment,
    behavior: PredatoryBehaviorDetection,
) -> soroban_sdk::Vec<ExploitationPattern> {
    let mut patterns = soroban_sdk::Vec::new(env);

    if vulnerability.high_recurrence && behavior.predatory {
        patterns.push_back(ExploitationPattern {
            pattern_type: Symbol::new(env, "dependency_trap"),
            severity: 80,
            immediate_action_required: true,
        });
    }

    if vulnerability.affordability_concern && behavior.price_exploitation_flag {
        patterns.push_back(ExploitationPattern {
            pattern_type: Symbol::new(env, "price_gouging"),
            severity: 75,
            immediate_action_required: true,
        });
    }

    if behavior.low_quality_pattern && vulnerability.at_risk {
        patterns.push_back(ExploitationPattern {
            pattern_type: Symbol::new(env, "quality_fraud"),
            severity: 70,
            immediate_action_required: false,
        });
    }

    if behavior.high_complaint_rate && vulnerability.high_recurrence {
        patterns.push_back(ExploitationPattern {
            pattern_type: Symbol::new(env, "repeat_exploitation"),
            severity: 85,
            immediate_action_required: true,
        });
    }

    patterns
}

// ---------------------------------------------------------------------------
// Welfare monitoring
// ---------------------------------------------------------------------------

/// Compute the welfare status for a learner from their vulnerability
/// assessment and the number of active exploitation patterns detected.
pub fn compute_welfare_status(
    vulnerability: VulnerabilityAssessment,
    active_pattern_count: u32,
) -> WelfareStatus {
    let welfare_risk = vulnerability
        .risk_score
        .saturating_add(active_pattern_count.saturating_mul(15))
        .min(100);

    let support_required = welfare_risk >= VULNERABILITY_RISK_THRESHOLD;
    let activate_support_services = active_pattern_count > 0 || vulnerability.at_risk;

    WelfareStatus {
        support_required,
        welfare_risk_score: welfare_risk,
        activate_support_services,
        active_pattern_count,
    }
}

// ---------------------------------------------------------------------------
// Combined intervention & emergency protection
// ---------------------------------------------------------------------------

/// Combine vulnerability, predatory-behaviour, and welfare signals into a
/// top-level learner-protection intervention decision. `now` should be
/// `env.ledger().timestamp()`.
pub fn compute_learner_protection_intervention(
    env: &soroban_sdk::Env,
    vulnerability: VulnerabilityAssessment,
    behavior: PredatoryBehaviorDetection,
    welfare: WelfareStatus,
    now: u64,
) -> LearnerProtectionRecord {
    let combined = vulnerability
        .risk_score
        .saturating_add(behavior.risk_score)
        .saturating_add(welfare.welfare_risk_score)
        / 3;
    let combined = combined.min(100);

    let (intervene, reason) = if behavior.predatory
        && vulnerability.at_risk
        && welfare.active_pattern_count >= EMERGENCY_PATTERN_THRESHOLD
    {
        (true, Symbol::new(env, "predatory_exploitation"))
    } else if behavior.predatory {
        (true, Symbol::new(env, "predatory_behavior"))
    } else if vulnerability.at_risk && welfare.support_required {
        (true, Symbol::new(env, "learner_at_risk"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    let emergency_suspension = behavior.predatory
        && vulnerability.at_risk
        && welfare.active_pattern_count >= EMERGENCY_PATTERN_THRESHOLD;

    LearnerProtectionRecord {
        intervene,
        combined_risk_score: combined,
        reason,
        restoration_eligible_at: if intervene {
            now.saturating_add(LEARNER_PROTECTION_COOLDOWN_SECS)
        } else {
            now
        },
        emergency_suspension,
    }
}

/// Build an `EmergencyIntervention` record when `compute_learner_protection_intervention`
/// has produced a record with `emergency_suspension = true`.
pub fn compute_emergency_intervention(
    env: &soroban_sdk::Env,
    protection: &LearnerProtectionRecord,
    now: u64,
) -> EmergencyIntervention {
    let _ = env; // env available for future event emission or cross-contract calls
    EmergencyIntervention {
        suspend_mentor: protection.emergency_suspension,
        halt_active_sessions: protection.emergency_suspension,
        combined_risk_score: protection.combined_risk_score,
        reason: protection.reason.clone(),
        reinstatement_eligible_at: if protection.emergency_suspension {
            now.saturating_add(EMERGENCY_SUSPENSION_COOLDOWN_SECS)
        } else {
            now
        },
    }
}

/// Whether a learner-protection intervention is eligible for auto-restoration.
pub fn is_protection_restoration_eligible(record: &LearnerProtectionRecord, now: u64) -> bool {
    !record.intervene || now >= record.restoration_eligible_at
}

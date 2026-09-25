//! Learning-outcome measurement integrity protection primitives.
//!
//! Mentors can manipulate learning outcome measurements: bursts of
//! self-attested completions from a narrow set of graders, success metrics
//! that jump implausibly relative to historical baselines, or assessment
//! criteria proposed by a coordinated bloc rather than set independently.
//! These helpers give contracts a deterministic, storage-agnostic way to
//! score outcome/metric/assessment patterns and decide when to intervene.
//! Contracts own the storage of raw measurement history; these functions are
//! pure scoring/decision logic over data the caller already has on hand.

use soroban_sdk::{contracttype, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Outcome measurements landing within this window of each other are
/// treated as a burst for manipulation-detection purposes.
pub const OUTCOME_BURST_WINDOW_SECS: u64 = 1_800;

/// Minimum distinct-evaluator ratio (basis points) for a batch of outcome
/// measurements to be considered genuine.
pub const OUTCOME_MIN_DISTINCT_BPS: u32 = 3_000; // 30%

/// Risk score (0-100) at or above which outcome authenticity is doubted.
pub const OUTCOME_RISK_THRESHOLD: u32 = 60;

/// Deviation (basis points) between a newly reported metric and its trusted
/// baseline at or above which the metric is flagged as gamed.
pub const METRIC_GAMING_DEVIATION_BPS: u32 = 2_500; // 25%

/// Assessment-criteria proposals within this window are treated as
/// clustered (characteristic of a coordinated bloc).
pub const ASSESSMENT_COORDINATION_WINDOW_SECS: u64 = 3_600;

/// Risk score at or above which assessment criteria are considered
/// non-independent.
pub const ASSESSMENT_RISK_THRESHOLD: u32 = 60;

/// Combined risk score at or above which outcome protection auto-intervenes.
pub const OUTCOME_INTERVENTION_THRESHOLD: u32 = 65;

/// Default cooldown before an intervened outcome measurement is eligible for
/// automatic restoration.
pub const OUTCOME_RESTORATION_COOLDOWN_SECS: u64 = 7 * 24 * 3_600;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Genuineness assessment for a batch of learning-outcome measurements.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct OutcomeAuthenticity {
    pub genuine: bool,
    pub manipulation_risk_score: u32,
    pub distinct_evaluator_bps: u32,
    pub burst_count: u32,
}

/// Gaming assessment for a single reported success metric relative to a
/// trusted historical baseline.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SuccessMetricProtection {
    pub gaming_detected: bool,
    pub gaming_risk_score: u32,
    pub deviation_bps: u32,
}

/// Independence assessment for a batch of assessment-criteria proposals.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AssessmentValidation {
    pub objective: bool,
    pub coordination_risk_score: u32,
    pub clustered_timing_count: u32,
}

/// Automatic outcome-protection intervention outcome.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutcomeInterventionRecord {
    pub intervene: bool,
    pub combined_risk_score: u32,
    pub reason: Symbol,
    pub restoration_eligible_at: u64,
}

// ---------------------------------------------------------------------------
// Outcome authenticity
// ---------------------------------------------------------------------------

/// Authenticate a batch of learning-outcome measurements (certificate
/// issuances, completion attestations) for one subject.
/// `measurement_timestamps` are when outcomes were recorded;
/// `distinct_evaluators` counts unique mentors/graders/oracles behind them.
pub fn authenticate_learning_outcomes(
    measurement_timestamps: &Vec<u64>,
    distinct_evaluators: u32,
) -> OutcomeAuthenticity {
    let total = measurement_timestamps.len();
    let distinct_bps = if total == 0 {
        10_000
    } else {
        (distinct_evaluators.saturating_mul(10_000)) / total
    };

    let mut burst = 0u32;
    if total >= 2 {
        for i in 1..total {
            let prev = measurement_timestamps.get(i - 1).unwrap_or(0);
            let cur = measurement_timestamps.get(i).unwrap_or(prev);
            if cur.saturating_sub(prev) < OUTCOME_BURST_WINDOW_SECS {
                burst = burst.saturating_add(1);
            }
        }
    }

    let mut risk = 0u32;
    if distinct_bps < OUTCOME_MIN_DISTINCT_BPS {
        risk = risk.saturating_add(50);
    }
    if burst >= 3 {
        risk = risk.saturating_add(40);
    } else if burst >= 1 {
        risk = risk.saturating_add(15);
    }
    risk = risk.min(100);

    OutcomeAuthenticity {
        genuine: risk < OUTCOME_RISK_THRESHOLD,
        manipulation_risk_score: risk,
        distinct_evaluator_bps: distinct_bps,
        burst_count: burst,
    }
}

// ---------------------------------------------------------------------------
// Success metric protection
// ---------------------------------------------------------------------------

/// Protect a success metric (completion rate, satisfaction score, etc.)
/// against gaming by comparing a newly reported value to a trusted
/// historical baseline and flagging implausibly large jumps. Both values are
/// expressed in basis points of their respective scale (e.g. 9500 = 95%).
pub fn protect_success_metrics(
    reported_value_bps: u32,
    baseline_value_bps: u32,
) -> SuccessMetricProtection {
    let diff = if reported_value_bps > baseline_value_bps {
        reported_value_bps - baseline_value_bps
    } else {
        baseline_value_bps - reported_value_bps
    };

    let deviation_bps = if baseline_value_bps == 0 {
        if reported_value_bps == 0 {
            0
        } else {
            10_000
        }
    } else {
        (diff.saturating_mul(10_000)) / baseline_value_bps
    };

    let gaming_detected = deviation_bps >= METRIC_GAMING_DEVIATION_BPS;
    let gaming_risk_score = (deviation_bps / 100).min(100);

    SuccessMetricProtection {
        gaming_detected,
        gaming_risk_score,
        deviation_bps,
    }
}

// ---------------------------------------------------------------------------
// Assessment validation
// ---------------------------------------------------------------------------

/// Validate that a batch of assessment-criteria proposals were set
/// independently rather than coordinated by a small bloc to bias evaluation
/// standards. `proposal_timestamps` are when criteria changes were proposed;
/// `distinct_proposers` counts unique accounts behind them.
pub fn validate_assessment_criteria(
    proposal_timestamps: &Vec<u64>,
    distinct_proposers: u32,
) -> AssessmentValidation {
    let total = proposal_timestamps.len();
    let mut clustered = 0u32;

    if total >= 2 {
        for i in 1..total {
            let prev = proposal_timestamps.get(i - 1).unwrap_or(0);
            let cur = proposal_timestamps.get(i).unwrap_or(prev);
            if cur.saturating_sub(prev) < ASSESSMENT_COORDINATION_WINDOW_SECS {
                clustered = clustered.saturating_add(1);
            }
        }
    }

    let mut risk = 0u32;
    if clustered >= 2 {
        risk = risk.saturating_add(45);
    } else if clustered >= 1 {
        risk = risk.saturating_add(20);
    }
    if total >= 2 && distinct_proposers <= 1 {
        risk = risk.saturating_add(35);
    }
    risk = risk.min(100);

    AssessmentValidation {
        objective: risk < ASSESSMENT_RISK_THRESHOLD,
        coordination_risk_score: risk,
        clustered_timing_count: clustered,
    }
}

// ---------------------------------------------------------------------------
// Automatic intervention & restoration
// ---------------------------------------------------------------------------

/// Combine outcome-authenticity, metric-gaming, and assessment-validation
/// signals into a single automatic outcome-protection intervention decision.
/// `restoration_cooldown_secs` controls how long an intervened subject must
/// wait before authentic measurement is automatically restored.
pub fn compute_outcome_intervention(
    env: &Env,
    outcome: OutcomeAuthenticity,
    metric: SuccessMetricProtection,
    assessment: AssessmentValidation,
    restoration_cooldown_secs: u64,
) -> OutcomeInterventionRecord {
    let combined = outcome
        .manipulation_risk_score
        .saturating_add(metric.gaming_risk_score)
        .saturating_add(assessment.coordination_risk_score)
        / 3;
    let combined = combined.min(100);

    let (intervene, reason) = if !outcome.genuine {
        (true, Symbol::new(env, "outcome_manipulation"))
    } else if metric.gaming_detected {
        (true, Symbol::new(env, "metric_gaming"))
    } else if !assessment.objective {
        (true, Symbol::new(env, "assessment_coordination"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    let now = env.ledger().timestamp();
    OutcomeInterventionRecord {
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

/// Whether a previously-intervened subject is now eligible to have authentic
/// outcome measurement automatically restored.
pub fn is_outcome_restoration_eligible(record: &OutcomeInterventionRecord, now: u64) -> bool {
    !record.intervene || now >= record.restoration_eligible_at
}

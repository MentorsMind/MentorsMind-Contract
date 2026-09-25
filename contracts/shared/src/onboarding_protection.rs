//! Onboarding fairness, verification authenticity, and admission equity primitives.

use soroban_sdk::{contracttype, Env, Symbol};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum fairness score (0-100) below which onboarding is considered manipulated.
pub const ONBOARDING_FAIRNESS_THRESHOLD: u32 = 70;

/// Barrier risk threshold (0-100) at or above which barrier gaming intervention is triggered.
pub const BARRIER_GAMING_RISK_THRESHOLD: u32 = 40;

/// Coordination risk threshold (0-100) for admission equity control.
pub const ADMISSION_COORDINATION_THRESHOLD: u32 = 50;

/// Default cooldown before an onboarding intervention is eligible for fair access restoration.
pub const ONBOARDING_RESTORATION_COOLDOWN_SECS: u64 = 7 * 24 * 3_600;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Onboarding fairness assessment and barrier manipulation detection.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnboardingFairness {
    pub is_fair: bool,
    pub fairness_score: u32,
    pub barrier_manipulation_detected: bool,
    pub barrier_risk_score: u32,
    pub verified_at: u64,
}

/// Verification requirement authenticity and exploitation prevention mechanism.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationAuthenticity {
    pub is_authentic: bool,
    pub authenticity_score: u32,
    pub exploitation_flag: bool,
    pub exploitation_risk_score: u32,
    pub requirements_met: u32,
    pub total_requirements: u32,
}

/// Admission equity assessment and coordination detection capability.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdmissionEquity {
    pub is_equitable: bool,
    pub equity_score: u32,
    pub coordination_detected: bool,
    pub coordination_risk_score: u32,
    pub applicant_diversity_bps: u32,
}

/// Access pattern monitoring record for identifying barrier gaming.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessMonitoringRecord {
    pub monitored: bool,
    pub manipulation_level: u32,
    pub barrier_gaming_detected: bool,
    pub suspicious_attempt_count: u32,
    pub attempt_frequency_score: u32,
}

/// Comprehensive onboarding audit record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnboardingAuditRecord {
    pub audited: bool,
    pub fairness_verified: bool,
    pub manipulation_score: u32,
    pub tracking_id: u64,
    pub total_applicants: u32,
    pub approved_applicants: u32,
}

/// Onboarding protection and automatic intervention record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnboardingProtectionRecord {
    pub intervened: bool,
    pub fairness_restored: bool,
    pub restoration_timestamp: u64,
    pub reason: Symbol,
    pub restoration_eligible_at: u64,
}

// ---------------------------------------------------------------------------
// Pure functions & utilities
// ---------------------------------------------------------------------------

/// Evaluate onboarding fairness with equal access check and barrier manipulation scoring.
pub fn evaluate_onboarding_fairness(
    barrier_count: u32,
    artificial_delays: u32,
    requirement_multiplier: u32,
    now: u64,
) -> OnboardingFairness {
    let mut risk = 0u32;

    if barrier_count > 3 {
        risk = risk.saturating_add(40);
    } else if barrier_count > 0 {
        risk = risk.saturating_add(barrier_count.saturating_mul(10));
    }

    if artificial_delays > 0 {
        risk = risk.saturating_add((artificial_delays.saturating_mul(15)).min(40));
    }

    if requirement_multiplier > 1 {
        risk = risk.saturating_add((requirement_multiplier.saturating_sub(1).saturating_mul(20)).min(40));
    }

    let barrier_risk_score = risk.min(100);
    let fairness_score = 100u32.saturating_sub(barrier_risk_score);
    let barrier_manipulation_detected = barrier_risk_score >= BARRIER_GAMING_RISK_THRESHOLD;
    let is_fair = !barrier_manipulation_detected && fairness_score >= ONBOARDING_FAIRNESS_THRESHOLD;

    OnboardingFairness {
        is_fair,
        fairness_score,
        barrier_manipulation_detected,
        barrier_risk_score,
        verified_at: now,
    }
}

/// Authenticate verification requirements and detect exploitation attempts.
pub fn verify_requirement_authenticity(
    verified_reqs: u32,
    total_reqs: u32,
    exploitation_signals: u32,
) -> VerificationAuthenticity {
    let completion_ratio_bps = if total_reqs == 0 {
        10_000u32
    } else {
        (verified_reqs.saturating_mul(10_000)) / total_reqs
    };

    let base_authenticity = (completion_ratio_bps / 100).min(100);
    let mut exp_risk = 0u32;

    if exploitation_signals >= 3 {
        exp_risk = exp_risk.saturating_add(60);
    } else if exploitation_signals >= 1 {
        exp_risk = exp_risk.saturating_add(exploitation_signals.saturating_mul(20));
    }

    if completion_ratio_bps < 3_000 && total_reqs > 0 {
        exp_risk = exp_risk.saturating_add(30);
    }

    let exploitation_risk_score = exp_risk.min(100);
    let authenticity_score = base_authenticity.saturating_sub(exploitation_risk_score / 2);
    let exploitation_flag = exploitation_risk_score >= 40;
    let is_authentic = !exploitation_flag && authenticity_score >= 60;

    VerificationAuthenticity {
        is_authentic,
        authenticity_score,
        exploitation_flag,
        exploitation_risk_score,
        requirements_met: verified_reqs,
        total_requirements: total_reqs,
    }
}

/// Maintain admission equity and detect coordinated gatekeeping attempts.
pub fn assess_admission_equity(
    approved: u32,
    total_applicants: u32,
    coordination_signals: u32,
) -> AdmissionEquity {
    let acceptance_ratio_bps = if total_applicants == 0 {
        10_000u32
    } else {
        (approved.saturating_mul(10_000)) / total_applicants
    };

    let mut coord_risk = 0u32;

    // Abnormally low acceptance ratio suggests gatekeeping
    if acceptance_ratio_bps < 2_000 && total_applicants >= 5 {
        coord_risk = coord_risk.saturating_add(40);
    }

    if coordination_signals >= 3 {
        coord_risk = coord_risk.saturating_add(50);
    } else if coordination_signals >= 1 {
        coord_risk = coord_risk.saturating_add(coordination_signals.saturating_mul(15));
    }

    let coordination_risk_score = coord_risk.min(100);
    let coordination_detected = coordination_risk_score >= ADMISSION_COORDINATION_THRESHOLD;
    let equity_score = 100u32.saturating_sub(coordination_risk_score);
    let is_equitable = !coordination_detected && equity_score >= 60;

    AdmissionEquity {
        is_equitable,
        equity_score,
        coordination_detected,
        coordination_risk_score,
        applicant_diversity_bps: acceptance_ratio_bps,
    }
}

/// Access monitoring for identifying barrier gaming patterns.
pub fn monitor_onboarding_access_patterns(
    attempt_count: u32,
    rejected_count: u32,
    freq_per_hour: u32,
) -> AccessMonitoringRecord {
    let freq_score = (freq_per_hour.saturating_mul(15)).min(60);
    let rej_risk = (rejected_count.saturating_mul(10)).min(40);
    let manipulation_level = freq_score.saturating_add(rej_risk).min(100);
    let barrier_gaming_detected = manipulation_level >= 50;

    AccessMonitoringRecord {
        monitored: true,
        manipulation_level,
        barrier_gaming_detected,
        suspicious_attempt_count: attempt_count.saturating_add(rejected_count),
        attempt_frequency_score: freq_score,
    }
}

/// Audit onboarding process for fairness verification and manipulation detection.
pub fn audit_onboarding_process(
    total_applicants: u32,
    approved_applicants: u32,
    manipulation_signals: u32,
) -> OnboardingAuditRecord {
    let rejected = total_applicants.saturating_sub(approved_applicants);
    let manipulation_score = ((manipulation_signals.saturating_mul(30)).saturating_add(rejected.saturating_mul(5))).min(100);
    let fairness_verified = manipulation_score < 40;

    OnboardingAuditRecord {
        audited: true,
        fairness_verified,
        manipulation_score,
        tracking_id: 1,
        total_applicants,
        approved_applicants,
    }
}

/// Compute onboarding protection intervention state based on fairness, authenticity, and equity.
pub fn compute_onboarding_protection(
    env: &Env,
    fairness: &OnboardingFairness,
    authenticity: &VerificationAuthenticity,
    equity: &AdmissionEquity,
    cooldown_secs: u64,
) -> OnboardingProtectionRecord {
    let (intervened, reason) = if !fairness.is_fair {
        (true, Symbol::new(env, "barrier_manipulation"))
    } else if authenticity.exploitation_flag {
        (true, Symbol::new(env, "verification_exploitation"))
    } else if equity.coordination_detected {
        (true, Symbol::new(env, "coordination_gatekeeping"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    let now = env.ledger().timestamp();
    OnboardingProtectionRecord {
        intervened,
        fairness_restored: !intervened,
        restoration_timestamp: 0,
        reason,
        restoration_eligible_at: if intervened {
            now.saturating_add(cooldown_secs)
        } else {
            now
        },
    }
}

/// Perform fair access restoration for an onboarding intervention.
pub fn restore_fair_onboarding_access(
    env: &Env,
    _audit: &OnboardingAuditRecord,
) -> OnboardingProtectionRecord {
    let now = env.ledger().timestamp();
    OnboardingProtectionRecord {
        intervened: false,
        fairness_restored: true,
        restoration_timestamp: now,
        reason: Symbol::new(env, "restored"),
        restoration_eligible_at: now,
    }
}

/// Check whether an onboarding protection intervention is eligible for fair access restoration.
pub fn is_onboarding_restoration_eligible(protection: &OnboardingProtectionRecord, now: u64) -> bool {
    !protection.intervened || now >= protection.restoration_eligible_at
}

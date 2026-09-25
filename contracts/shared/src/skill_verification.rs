//! Mentor skill verification & specialization-fraud protection primitives.
//!
//! Mentors can misrepresent skills, claim specializations they don't hold,
//! or rely on stale/forged credentials to attract learners. These helpers
//! give contracts a deterministic, storage-agnostic way to score practical
//! assessments, peer-validation consensus, external credential
//! authentication, and ongoing expertise tracking. Contracts own the
//! storage of raw skill-claim history; these functions are pure
//! scoring/decision logic over data the caller already has on hand.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum number of distinct peer validators required to accept a
/// practical skill assessment as validated.
pub const MIN_PEER_VALIDATORS: u32 = 2;

/// Minimum score (basis points out of 10,000) a practical assessment must
/// reach to be considered a pass.
pub const PASSING_ASSESSMENT_SCORE_BPS: u32 = 6_000;

/// Default recertification interval: mentors must re-validate a claimed
/// specialization on this cadence (180 days).
pub const RECERTIFICATION_PERIOD_SECS: u64 = 180 * 24 * 3_600;

/// Risk score (0-100) at or above which a skill claim is flagged as likely
/// fraud/misrepresentation.
pub const SKILL_FRAUD_RISK_THRESHOLD: u32 = 60;

/// Minimum tracked sessions in a claimed specialization before performance
/// history is considered statistically meaningful.
pub const MIN_SESSIONS_FOR_EXPERTISE_TRACKING: u32 = 3;

/// Outcome score (bps) below which claimed-specialization performance is
/// considered underperforming relative to the claim.
pub const EXPERTISE_UNDERPERFORMANCE_THRESHOLD_BPS: u32 = 4_000;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of scoring a mentor's practical skill demonstration.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PracticalAssessment {
    pub mentor: Address,
    pub specialization: Symbol,
    pub score_bps: u32,
    pub passed: bool,
    pub assessed_at: u64,
}

/// Outcome of peer-validator consensus over a practical assessment.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerValidationRecord {
    pub validator_count: u32,
    pub approvals: u32,
    pub consensus_reached: bool,
}

/// Result of authenticating an externally-issued credential.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpertiseAuthenticationRecord {
    pub mentor: Address,
    pub specialization: Symbol,
    pub credential_verified: bool,
    pub verified_at: u64,
    pub valid_until: u64,
}

/// Domain-expert governance decision over a specialization category.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpecializationGovernanceRecord {
    pub specialization: Symbol,
    pub domain_experts: u32,
    pub approvals: u32,
    pub standards_met: bool,
}

/// Fraud/misrepresentation risk flag for a mentor's skill claim.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillFraudFlag {
    pub fraud_suspected: bool,
    pub risk_score: u32,
    pub underperformance_count: u32,
    pub credential_mismatch: bool,
}

/// Recertification schedule for a mentor's claimed specialization.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RecertificationSchedule {
    pub mentor: Address,
    pub specialization: Symbol,
    pub last_certified_at: u64,
    pub due_at: u64,
    pub overdue: bool,
}

// ---------------------------------------------------------------------------
// Practical assessment scoring
// ---------------------------------------------------------------------------

/// Score a practical skill demonstration out of 10,000 basis points and
/// determine pass/fail against `PASSING_ASSESSMENT_SCORE_BPS`.
pub fn score_practical_assessment(
    env: &Env,
    mentor: &Address,
    specialization: &Symbol,
    raw_criteria_scores_bps: &Vec<u32>,
) -> PracticalAssessment {
    let count = raw_criteria_scores_bps.len();
    let score_bps = if count == 0 {
        0
    } else {
        let mut total: u64 = 0;
        for score in raw_criteria_scores_bps.iter() {
            total = total.saturating_add(score as u64);
        }
        (total / count as u64) as u32
    };

    PracticalAssessment {
        mentor: mentor.clone(),
        specialization: specialization.clone(),
        score_bps,
        passed: score_bps >= PASSING_ASSESSMENT_SCORE_BPS,
        assessed_at: env.ledger().timestamp(),
    }
}

/// Validate a set of peer-reviewer votes on a practical assessment,
/// requiring at least `MIN_PEER_VALIDATORS` distinct validators and a
/// strict majority of approvals for consensus.
pub fn validate_peer_consensus(validator_votes: &Vec<bool>) -> PeerValidationRecord {
    let validator_count = validator_votes.len();
    let mut approvals = 0u32;
    for vote in validator_votes.iter() {
        if vote {
            approvals = approvals.saturating_add(1);
        }
    }
    let consensus_reached =
        validator_count >= MIN_PEER_VALIDATORS && approvals.saturating_mul(2) > validator_count;

    PeerValidationRecord {
        validator_count,
        approvals,
        consensus_reached,
    }
}

// ---------------------------------------------------------------------------
// External credential authentication
// ---------------------------------------------------------------------------

/// Authenticate an externally-verified credential for a claimed
/// specialization, given the external verifier's attestation validity
/// window. Ongoing validation is enforced by callers re-invoking this on
/// the `RECERTIFICATION_PERIOD_SECS` cadence.
pub fn authenticate_external_credential(
    env: &Env,
    mentor: &Address,
    specialization: &Symbol,
    credential_valid: bool,
    credential_expiry: u64,
) -> ExpertiseAuthenticationRecord {
    let now = env.ledger().timestamp();
    let verified = credential_valid && credential_expiry > now;
    ExpertiseAuthenticationRecord {
        mentor: mentor.clone(),
        specialization: specialization.clone(),
        credential_verified: verified,
        verified_at: now,
        valid_until: credential_expiry,
    }
}

// ---------------------------------------------------------------------------
// Skill category governance
// ---------------------------------------------------------------------------

/// Evaluate domain-expert oversight votes for a specialization category,
/// requiring a strict majority of domain experts to endorse the standard.
pub fn evaluate_domain_governance(
    specialization: &Symbol,
    domain_expert_votes: &Vec<bool>,
) -> SpecializationGovernanceRecord {
    let domain_experts = domain_expert_votes.len();
    let mut approvals = 0u32;
    for vote in domain_expert_votes.iter() {
        if vote {
            approvals = approvals.saturating_add(1);
        }
    }
    let standards_met = domain_experts > 0 && approvals.saturating_mul(2) > domain_experts;

    SpecializationGovernanceRecord {
        specialization: specialization.clone(),
        domain_experts,
        approvals,
        standards_met,
    }
}

// ---------------------------------------------------------------------------
// Fraud detection
// ---------------------------------------------------------------------------

/// Detect skill misrepresentation by combining claimed-specialization
/// performance history with credential-authentication state.
pub fn detect_skill_fraud(
    session_outcome_scores_bps: &Vec<u32>,
    credential_verified: bool,
) -> SkillFraudFlag {
    let mut underperformance_count = 0u32;
    let mut total: u64 = 0;
    let count = session_outcome_scores_bps.len();
    for score in session_outcome_scores_bps.iter() {
        total = total.saturating_add(score as u64);
        if score < EXPERTISE_UNDERPERFORMANCE_THRESHOLD_BPS {
            underperformance_count = underperformance_count.saturating_add(1);
        }
    }

    let mut risk_score = 0u32;
    if count >= MIN_SESSIONS_FOR_EXPERTISE_TRACKING {
        let underperf_ratio_bps = underperformance_count.saturating_mul(10_000) / count.max(1);
        risk_score = risk_score.saturating_add(underperf_ratio_bps / 200); // up to 50
    }
    let credential_mismatch = !credential_verified;
    if credential_mismatch {
        risk_score = risk_score.saturating_add(40);
    }
    if risk_score > 100 {
        risk_score = 100;
    }

    SkillFraudFlag {
        fraud_suspected: risk_score >= SKILL_FRAUD_RISK_THRESHOLD,
        risk_score,
        underperformance_count,
        credential_mismatch,
    }
}

// ---------------------------------------------------------------------------
// Recertification
// ---------------------------------------------------------------------------

/// Compute the next recertification due date for a mentor's claimed
/// specialization and whether it is currently overdue.
pub fn compute_recertification_due(
    env: &Env,
    mentor: &Address,
    specialization: &Symbol,
    last_certified_at: u64,
) -> RecertificationSchedule {
    let now = env.ledger().timestamp();
    let due_at = last_certified_at.saturating_add(RECERTIFICATION_PERIOD_SECS);
    RecertificationSchedule {
        mentor: mentor.clone(),
        specialization: specialization.clone(),
        last_certified_at,
        due_at,
        overdue: now > due_at,
    }
}

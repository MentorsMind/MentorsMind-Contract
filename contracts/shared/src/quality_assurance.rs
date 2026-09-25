use soroban_sdk::{contracttype, Address, BytesN, Symbol, Vec};

/// Minimum number of sessions required before quality assessment is valid.
pub const MIN_SESSIONS_FOR_QUALITY: u32 = 5;

/// Threshold (basis points) below which a mentor's quality score is flagged.
pub const QUALITY_RISK_THRESHOLD_BPS: u32 = 3_000;

/// Maximum number of quality-assessment records retained per mentor.
pub const MAX_QUALITY_RECORDS: u32 = 20;

/// Window (seconds) for computing rolling quality metrics.
pub const QUALITY_WINDOW_SECS: u64 = 604_800; // 7 days

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QualityAssessment {
    pub mentor: Address,
    pub session_id: Symbol,
    pub score: u32,
    pub assessed_at: u64,
    pub criteria_hash: BytesN<32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QualityMetrics {
    pub mentor: Address,
    pub avg_score: u32,
    pub total_sessions: u32,
    pub flagged_count: u32,
    pub last_assessed_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QualityCheckResult {
    pub passed: bool,
    pub score: u32,
    pub reason: QualityFailReason,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum QualityFailReason {
    None,
    BelowThreshold,
    InsufficientSessions,
    CriteriaMismatch,
}

/// Compute a rolling average quality score from individual assessment scores.
pub fn compute_average_score(scores: &[u32]) -> u32 {
    if scores.is_empty() {
        return 0;
    }
    let total: u64 = scores.iter().map(|&s| s as u64).sum();
    (total / scores.len() as u64) as u32
}

/// Check whether a quality score passes the threshold.
pub fn passes_quality_check(score: u32) -> bool {
    score >= QUALITY_RISK_THRESHOLD_BPS
}

/// Build a QualityMetrics record from raw assessment data.
pub fn build_quality_metrics(
    mentor: Address,
    assessments: &[QualityAssessment],
    now: u64,
) -> QualityMetrics {
    let scores: Vec<u32> = assessments.iter().map(|a| a.score).collect();
    let flagged = scores.iter().filter(|&&s| !passes_quality_check(s)).count() as u32;
    QualityMetrics {
        mentor,
        avg_score: compute_average_score(&scores),
        total_sessions: scores.len() as u32,
        flagged_count: flagged,
        last_assessed_at: now,
    }
}

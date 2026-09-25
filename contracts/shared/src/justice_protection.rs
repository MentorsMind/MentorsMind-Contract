//! Dispute-resolution integrity protection primitives.
//!
//! Mentor groups can coordinate dispute strategies: filing disputes against
//! the same counterparties in tight clusters, reusing evidence content
//! across unrelated cases, or steering arbitration toward a systematically
//! one-sided arbitrator. These helpers give contracts a deterministic,
//! storage-agnostic way to score dispute/evidence/arbitration patterns and
//! decide when to intervene. Contracts own the storage of raw dispute
//! history; these functions are pure scoring/decision logic over data the
//! caller already has on hand.

use soroban_sdk::{contracttype, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Dispute-related events (opens, evidence submissions) between the same
/// actor set within this window are treated as tightly clustered
/// (characteristic of coordinated dispute filing).
pub const DISPUTE_COORDINATION_WINDOW_SECS: u64 = 3_600;

/// Risk score (0-100) at or above which dispute independence is doubted.
pub const DISPUTE_INDEPENDENCE_RISK_THRESHOLD: u32 = 60;

/// Evidence submissions within this window that also share content are
/// treated as suspiciously rehearsed rather than independently produced.
pub const EVIDENCE_DUPLICATE_WINDOW_SECS: u64 = 1_800;

/// Risk score at or above which evidence authenticity is doubted.
pub const EVIDENCE_TAMPER_RISK_THRESHOLD: u32 = 60;

/// Minimum rulings an arbitrator must have on record before a one-sided
/// ratio is treated as statistically meaningful bias.
pub const ARBITRATION_MIN_RULINGS_FOR_BIAS: u32 = 3;

/// One-sided ruling ratio (basis points) at or above which an arbitrator's
/// history is flagged as biased, given `ARBITRATION_MIN_RULINGS_FOR_BIAS`.
pub const ARBITRATION_BIAS_RATIO_BPS_THRESHOLD: u32 = 8_000; // 80%

/// Risk score at or above which arbitration is considered unfair.
pub const ARBITRATION_BIAS_RISK_THRESHOLD: u32 = 60;

/// Combined risk score at or above which justice protection auto-intervenes.
pub const JUSTICE_INTERVENTION_THRESHOLD: u32 = 65;

/// Default cooldown before an intervened dispute flow is eligible for
/// automatic restoration.
pub const JUSTICE_RESTORATION_COOLDOWN_SECS: u64 = 14 * 24 * 3_600;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of scoring a dispute for independence from coordinated filing.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DisputeIndependenceFlag {
    pub independent: bool,
    pub risk_score: u32,
    pub shared_actor_count: u32,
    pub clustered_timing_count: u32,
}

/// Authenticity assessment for a dispute's evidence submissions.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct EvidenceAuthenticity {
    pub authentic: bool,
    pub tampering_risk_score: u32,
    pub duplicate_submission_count: u32,
    pub suspicious_timing_count: u32,
}

/// Fairness assessment for an arbitrator's ruling history.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ArbitrationBiasFlag {
    pub fair: bool,
    pub bias_risk_score: u32,
    pub one_sided_ratio_bps: u32,
    pub ruling_count: u32,
}

/// Automatic justice-protection intervention outcome.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JusticeInterventionRecord {
    pub intervene: bool,
    pub combined_risk_score: u32,
    pub reason: Symbol,
    pub restoration_eligible_at: u64,
}

// ---------------------------------------------------------------------------
// Dispute independence
// ---------------------------------------------------------------------------

/// Detect whether a dispute was filed independently or shows signs of
/// coordination. `shared_timestamps` are dispute-open/evidence timestamps
/// involving an overlapping actor set (e.g. the same mentor repeatedly
/// disputing the same learner, or vice versa); `shared_actor_count` counts
/// how many other open disputes share an actor with this one.
pub fn ensure_dispute_independence(
    shared_timestamps: &Vec<u64>,
    shared_actor_count: u32,
) -> DisputeIndependenceFlag {
    let count = shared_timestamps.len();
    let mut clustered = 0u32;

    if count >= 2 {
        for i in 1..count {
            let prev = shared_timestamps.get(i - 1).unwrap_or(0);
            let cur = shared_timestamps.get(i).unwrap_or(prev);
            if cur.saturating_sub(prev) < DISPUTE_COORDINATION_WINDOW_SECS {
                clustered = clustered.saturating_add(1);
            }
        }
    }

    let mut risk = 0u32;
    if count >= 3 {
        risk = risk.saturating_add(30);
    }
    if clustered >= 2 {
        risk = risk.saturating_add(35);
    }
    if shared_actor_count >= 2 {
        risk = risk.saturating_add(35);
    }
    risk = risk.min(100);

    DisputeIndependenceFlag {
        independent: risk < DISPUTE_INDEPENDENCE_RISK_THRESHOLD,
        risk_score: risk,
        shared_actor_count,
        clustered_timing_count: clustered,
    }
}

// ---------------------------------------------------------------------------
// Evidence authenticity
// ---------------------------------------------------------------------------

/// Validate the authenticity of a dispute's evidence submissions: detect
/// content reuse across unrelated disputes (`duplicate_hash_count`) and
/// clustered submission timing consistent with fabricated or rehearsed
/// evidence.
pub fn validate_evidence_authenticity(
    submission_timestamps: &Vec<u64>,
    duplicate_hash_count: u32,
) -> EvidenceAuthenticity {
    let total = submission_timestamps.len();
    let mut suspicious_timing = 0u32;

    if total >= 2 {
        for i in 1..total {
            let prev = submission_timestamps.get(i - 1).unwrap_or(0);
            let cur = submission_timestamps.get(i).unwrap_or(prev);
            if cur.saturating_sub(prev) < EVIDENCE_DUPLICATE_WINDOW_SECS {
                suspicious_timing = suspicious_timing.saturating_add(1);
            }
        }
    }

    let mut risk = 0u32;
    if duplicate_hash_count >= 1 {
        risk = risk.saturating_add(50);
    }
    if suspicious_timing >= 2 {
        risk = risk.saturating_add(30);
    } else if suspicious_timing >= 1 {
        risk = risk.saturating_add(10);
    }
    risk = risk.min(100);

    EvidenceAuthenticity {
        authentic: risk < EVIDENCE_TAMPER_RISK_THRESHOLD,
        tampering_risk_score: risk,
        duplicate_submission_count: duplicate_hash_count,
        suspicious_timing_count: suspicious_timing,
    }
}

// ---------------------------------------------------------------------------
// Arbitration fairness
// ---------------------------------------------------------------------------

/// Assess an arbitrator's recent ruling history for systematic bias toward
/// one party. `favor_history` is a rolling window of recent rulings for this
/// arbitrator (`true` = ruled for mentor/first party, `false` = ruled for
/// learner/second party).
pub fn protect_arbitration_fairness(favor_history: &Vec<bool>) -> ArbitrationBiasFlag {
    let total = favor_history.len();
    if total == 0 {
        return ArbitrationBiasFlag {
            fair: true,
            bias_risk_score: 0,
            one_sided_ratio_bps: 0,
            ruling_count: 0,
        };
    }

    let mut favor_count = 0u32;
    for i in 0..total {
        if favor_history.get(i).unwrap_or(false) {
            favor_count = favor_count.saturating_add(1);
        }
    }
    let against_count = total.saturating_sub(favor_count);
    let dominant = favor_count.max(against_count);
    let ratio_bps = (dominant.saturating_mul(10_000)) / total.max(1);

    let mut risk = 0u32;
    if total >= ARBITRATION_MIN_RULINGS_FOR_BIAS && ratio_bps >= ARBITRATION_BIAS_RATIO_BPS_THRESHOLD {
        risk = risk.saturating_add(70);
    } else if ratio_bps >= 7_000 {
        risk = risk.saturating_add(30);
    }
    risk = risk.min(100);

    ArbitrationBiasFlag {
        fair: risk < ARBITRATION_BIAS_RISK_THRESHOLD,
        bias_risk_score: risk,
        one_sided_ratio_bps: ratio_bps,
        ruling_count: total,
    }
}

// ---------------------------------------------------------------------------
// Automatic intervention & restoration
// ---------------------------------------------------------------------------

/// Combine dispute-independence, evidence-authenticity, and
/// arbitration-fairness signals into a single automatic justice-protection
/// intervention decision. `restoration_cooldown_secs` controls how long an
/// intervened dispute flow must wait before fair resolution resumes.
pub fn compute_justice_intervention(
    env: &Env,
    independence: DisputeIndependenceFlag,
    evidence: EvidenceAuthenticity,
    bias: ArbitrationBiasFlag,
    restoration_cooldown_secs: u64,
) -> JusticeInterventionRecord {
    let combined = independence
        .risk_score
        .saturating_add(evidence.tampering_risk_score)
        .saturating_add(bias.bias_risk_score)
        / 3;
    let combined = combined.min(100);

    let (intervene, reason) = if !independence.independent {
        (true, Symbol::new(env, "dispute_coordination"))
    } else if !evidence.authentic {
        (true, Symbol::new(env, "evidence_tampering"))
    } else if !bias.fair {
        (true, Symbol::new(env, "arbitration_bias"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    let now = env.ledger().timestamp();
    JusticeInterventionRecord {
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

/// Whether a previously-intervened dispute flow is now eligible to have fair
/// resolution automatically restored.
pub fn is_justice_restoration_eligible(record: &JusticeInterventionRecord, now: u64) -> bool {
    !record.intervene || now >= record.restoration_eligible_at
}

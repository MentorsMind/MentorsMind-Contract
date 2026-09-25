//! Community dynamics protection primitives.
//!
//! Mentor groups can coordinate to manipulate community perception: tight
//! clusters of accounts that only ever interact with each other, referral
//! chains grown from a handful of sources, or endorsement bursts from the
//! same small circle. These helpers give contracts a deterministic,
//! storage-agnostic way to score interaction patterns and decide when to
//! restrict access or flag a group for review. Contracts own the storage of
//! raw interaction history; these functions are pure scoring/decision logic
//! over data the caller already has on hand.

use soroban_sdk::{contracttype, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum number of interactions between the same counterparties before a
/// pair is considered for coordination scoring.
pub const COORDINATION_MIN_INTERACTIONS: u32 = 3;

/// Interactions between the same pair within this window are treated as
/// tightly clustered (characteristic of scripted/coordinated activity).
pub const COORDINATION_TIGHT_WINDOW_SECS: u64 = 3_600;

/// Risk score (0-100) at or above which a coordination flag is "suspicious".
pub const COORDINATION_RISK_THRESHOLD: u32 = 60;

/// Below this ratio of distinct counterparties to total interactions
/// (in basis points), a community's growth is considered concentrated
/// rather than organic.
pub const NETWORK_DISTINCT_SOURCE_MIN_BPS: u32 = 2_000; // 20%

/// Growth rate (new members per day) above which network expansion is
/// scrutinized for artificial inflation.
pub const NETWORK_SUSPICIOUS_GROWTH_PER_DAY: u32 = 25;

/// Endorsements landing within this window of each other are treated as a
/// burst for social-proof gaming detection.
pub const SOCIAL_PROOF_BURST_WINDOW_SECS: u64 = 1_800;

/// Minimum distinct endorsers required, relative to total endorsements
/// (basis points), for a social-proof signal to be considered genuine.
pub const SOCIAL_PROOF_MIN_DISTINCT_BPS: u32 = 4_000; // 40%

/// Risk score at or above which community protection auto-intervenes.
pub const COMMUNITY_INTERVENTION_THRESHOLD: u32 = 70;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of scoring a cluster of accounts for coordinated behavior.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct CoordinationFlag {
    pub suspicious: bool,
    pub risk_score: u32,
    pub repeated_pair_count: u32,
    pub clustered_timing_count: u32,
}

/// Authenticity assessment for referral/network growth.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct NetworkEffectScore {
    pub authentic: bool,
    pub influence_score: u32,
    pub artificial_growth_flag: bool,
    pub distinct_source_bps: u32,
}

/// Genuineness assessment for a batch of social-proof signals
/// (endorsements, reviews, follows, etc.) directed at one subject.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SocialProofRecord {
    pub genuine: bool,
    pub gaming_risk_score: u32,
    pub distinct_endorser_bps: u32,
    pub burst_count: u32,
}

/// Decision on whether an account should be granted fair community access.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FairAccessDecision {
    pub access_granted: bool,
    pub restriction_reason: Option<Symbol>,
    pub review_required: bool,
}

/// Automatic community-protection intervention outcome.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CommunityInterventionRecord {
    pub intervene: bool,
    pub combined_risk_score: u32,
    pub reason: Symbol,
    pub restoration_eligible_at: u64,
}

// ---------------------------------------------------------------------------
// Coordination detection
// ---------------------------------------------------------------------------

/// Detect coordination between two counterparties from their shared
/// interaction timestamps. `timestamps` must belong to interactions between
/// the same two accounts (e.g. a mentor and a recurring learner/reviewer).
pub fn detect_coordination(timestamps: &Vec<u64>) -> CoordinationFlag {
    let count = timestamps.len();
    let mut clustered = 0u32;
    let mut risk = 0u32;

    if count >= 2 {
        for i in 1..count {
            let prev = timestamps.get(i - 1).unwrap_or(0);
            let cur = timestamps.get(i).unwrap_or(prev);
            if cur.saturating_sub(prev) < COORDINATION_TIGHT_WINDOW_SECS {
                clustered = clustered.saturating_add(1);
            }
        }
    }

    if count >= COORDINATION_MIN_INTERACTIONS {
        risk = risk.saturating_add(30);
    }
    if clustered >= 2 {
        risk = risk.saturating_add(40);
    }
    if count >= COORDINATION_MIN_INTERACTIONS.saturating_mul(2) {
        risk = risk.saturating_add(20);
    }
    risk = risk.min(100);

    CoordinationFlag {
        suspicious: risk >= COORDINATION_RISK_THRESHOLD,
        risk_score: risk,
        repeated_pair_count: count,
        clustered_timing_count: clustered,
    }
}

/// Scan a group of accounts for a coordination ring: a small clique that
/// interacts almost exclusively with itself. `distinct_counterparties` is
/// the number of unique accounts each member has interacted with *outside*
/// the group; `group_size` is the size of the suspected clique.
pub fn detect_coordination_ring(group_size: u32, distinct_counterparties: u32) -> CoordinationFlag {
    if group_size < 2 {
        return CoordinationFlag {
            suspicious: false,
            risk_score: 0,
            repeated_pair_count: 0,
            clustered_timing_count: 0,
        };
    }

    // Ring members that barely interact outside the group are suspicious;
    // ones with rich external activity are not.
    let external_ratio_bps = if group_size == 0 {
        10_000
    } else {
        (distinct_counterparties.saturating_mul(10_000)) / group_size.max(1)
    };

    let mut risk = 0u32;
    if external_ratio_bps < 1_000 {
        risk = risk.saturating_add(60);
    } else if external_ratio_bps < 3_000 {
        risk = risk.saturating_add(30);
    }
    if group_size >= 3 {
        risk = risk.saturating_add(20);
    }
    risk = risk.min(100);

    CoordinationFlag {
        suspicious: risk >= COORDINATION_RISK_THRESHOLD,
        risk_score: risk,
        repeated_pair_count: group_size,
        clustered_timing_count: 0,
    }
}

// ---------------------------------------------------------------------------
// Network effect validation
// ---------------------------------------------------------------------------

/// Validate the authenticity of network growth (referrals, follows, joins).
/// `new_members_per_day` is the observed growth rate; `distinct_sources`
/// counts unique referrers/inviters behind `total_new_members`.
pub fn validate_network_authenticity(
    new_members_per_day: u32,
    total_new_members: u32,
    distinct_sources: u32,
) -> NetworkEffectScore {
    let distinct_source_bps = if total_new_members == 0 {
        10_000
    } else {
        (distinct_sources.saturating_mul(10_000)) / total_new_members
    };

    let mut artificial_growth_flag = false;
    let mut influence_score = 100u32;

    if new_members_per_day > NETWORK_SUSPICIOUS_GROWTH_PER_DAY
        && distinct_source_bps < NETWORK_DISTINCT_SOURCE_MIN_BPS
    {
        artificial_growth_flag = true;
        influence_score = influence_score.saturating_sub(60);
    } else if distinct_source_bps < NETWORK_DISTINCT_SOURCE_MIN_BPS {
        influence_score = influence_score.saturating_sub(30);
    }

    NetworkEffectScore {
        authentic: !artificial_growth_flag,
        influence_score,
        artificial_growth_flag,
        distinct_source_bps,
    }
}

// ---------------------------------------------------------------------------
// Social proof protection
// ---------------------------------------------------------------------------

/// Verify the genuineness of social-proof signals (endorsements, reviews,
/// upvotes) directed at a single subject.
pub fn verify_social_proof(
    signal_timestamps: &Vec<u64>,
    distinct_endorsers: u32,
) -> SocialProofRecord {
    let total = signal_timestamps.len();
    let distinct_endorser_bps = if total == 0 {
        10_000
    } else {
        (distinct_endorsers.saturating_mul(10_000)) / total
    };

    let mut burst_count = 0u32;
    if total >= 2 {
        for i in 1..total {
            let prev = signal_timestamps.get(i - 1).unwrap_or(0);
            let cur = signal_timestamps.get(i).unwrap_or(prev);
            if cur.saturating_sub(prev) < SOCIAL_PROOF_BURST_WINDOW_SECS {
                burst_count = burst_count.saturating_add(1);
            }
        }
    }

    let mut gaming_risk = 0u32;
    if distinct_endorser_bps < SOCIAL_PROOF_MIN_DISTINCT_BPS {
        gaming_risk = gaming_risk.saturating_add(50);
    }
    if burst_count >= 3 {
        gaming_risk = gaming_risk.saturating_add(40);
    } else if burst_count >= 1 {
        gaming_risk = gaming_risk.saturating_add(15);
    }
    gaming_risk = gaming_risk.min(100);

    SocialProofRecord {
        genuine: gaming_risk < COORDINATION_RISK_THRESHOLD,
        gaming_risk_score: gaming_risk,
        distinct_endorser_bps,
        burst_count,
    }
}

// ---------------------------------------------------------------------------
// Fair community access
// ---------------------------------------------------------------------------

/// Decide whether an account should retain full community access given its
/// current coordination and social-proof risk signals.
pub fn evaluate_fair_access(
    env: &soroban_sdk::Env,
    coordination: CoordinationFlag,
    social_proof: SocialProofRecord,
) -> FairAccessDecision {
    if coordination.risk_score >= COMMUNITY_INTERVENTION_THRESHOLD {
        return FairAccessDecision {
            access_granted: false,
            restriction_reason: Some(Symbol::new(env, "coordination_detected")),
            review_required: true,
        };
    }
    if social_proof.gaming_risk_score >= COMMUNITY_INTERVENTION_THRESHOLD {
        return FairAccessDecision {
            access_granted: false,
            restriction_reason: Some(Symbol::new(env, "social_proof_gaming")),
            review_required: true,
        };
    }
    if coordination.suspicious || !social_proof.genuine {
        return FairAccessDecision {
            access_granted: true,
            restriction_reason: Some(Symbol::new(env, "monitored")),
            review_required: true,
        };
    }
    FairAccessDecision {
        access_granted: true,
        restriction_reason: None,
        review_required: false,
    }
}

// ---------------------------------------------------------------------------
// Automatic intervention & restoration
// ---------------------------------------------------------------------------

/// Combine coordination, network, and social-proof signals into a single
/// automatic-intervention decision. `restoration_cooldown_secs` controls how
/// long an intervened account must wait before fair participation resumes.
pub fn compute_community_intervention(
    env: &soroban_sdk::Env,
    coordination: CoordinationFlag,
    network: NetworkEffectScore,
    social_proof: SocialProofRecord,
    restoration_cooldown_secs: u64,
) -> CommunityInterventionRecord {
    let combined = coordination
        .risk_score
        .saturating_add(social_proof.gaming_risk_score)
        .saturating_add(if network.artificial_growth_flag { 40 } else { 0 })
        / 2;
    let combined = combined.min(100);

    let (intervene, reason) = if coordination.risk_score >= COMMUNITY_INTERVENTION_THRESHOLD {
        (true, Symbol::new(env, "coordination_ring"))
    } else if social_proof.gaming_risk_score >= COMMUNITY_INTERVENTION_THRESHOLD {
        (true, Symbol::new(env, "social_proof_gaming"))
    } else if network.artificial_growth_flag {
        (true, Symbol::new(env, "artificial_network_growth"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    let now = env.ledger().timestamp();
    CommunityInterventionRecord {
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

/// Whether a previously-intervened account is now eligible to have fair
/// participation automatically restored.
pub fn is_restoration_eligible(record: &CommunityInterventionRecord, now: u64) -> bool {
    !record.intervene || now >= record.restoration_eligible_at
}

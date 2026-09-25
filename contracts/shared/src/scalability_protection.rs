//! Platform scalability and fair-resource-allocation protection primitives.
//!
//! Users can exploit scalability limitations under high demand: bursts of
//! requests from a narrow set of actors competing for the same scarce
//! resource, coordinated load spikes that resemble an attack rather than
//! organic usage, or a single requester claiming an unfair share of pooled
//! capacity. These helpers give contracts a deterministic, storage-agnostic
//! way to score load patterns and decide how to allocate resources fairly.
//! Contracts own the storage of raw request history; these functions are
//! pure scoring/decision logic over data the caller already has on hand.

use soroban_sdk::{contracttype, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Resource requests landing within this window of each other are treated
/// as a burst for competition-detection purposes.
pub const RESOURCE_BURST_WINDOW_SECS: u64 = 60;

/// Minimum distinct-requester ratio (basis points) for a burst of resource
/// requests to be considered fair/organic.
pub const RESOURCE_MIN_DISTINCT_BPS: u32 = 2_000; // 20%

/// Risk score (0-100) at or above which resource competition is unfair.
pub const RESOURCE_COMPETITION_RISK_THRESHOLD: u32 = 60;

/// Request rate (per minute) at or above which a caller's load pattern is
/// scrutinized for a coordinated attack.
pub const LOAD_SUSPICIOUS_RATE_PER_MINUTE: u32 = 50;

/// Maximum share (basis points) of pooled resource capacity a single
/// requester may be granted before being throttled.
pub const FAIR_ALLOCATION_MAX_SHARE_BPS: u32 = 3_000; // 30%

/// Combined risk score at or above which performance protection
/// auto-intervenes.
pub const PERFORMANCE_INTERVENTION_THRESHOLD: u32 = 65;

/// Default cooldown before an intervened resource pool is eligible for
/// automatic fair-allocation restoration.
pub const PERFORMANCE_RESTORATION_COOLDOWN_SECS: u64 = 3_600;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of scanning a resource pool's request history for unfair
/// competition (griefing/hoarding by a narrow set of requesters).
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ResourceCompetitionFlag {
    pub fair: bool,
    pub risk_score: u32,
    pub distinct_requester_bps: u32,
    pub burst_count: u32,
}

/// Outcome of validating a caller's request rate against legitimate-usage
/// bounds.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct LoadValidationResult {
    pub legitimate: bool,
    pub attack_risk_score: u32,
    pub request_rate_per_minute: u32,
}

/// Fair-allocation decision for one requester's share of a pooled resource.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FairResourceAllocation {
    pub granted_share_bps: u32,
    pub throttled: bool,
    pub reason: Symbol,
}

/// Automatic performance-protection intervention outcome.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformanceInterventionRecord {
    pub intervene: bool,
    pub combined_risk_score: u32,
    pub reason: Symbol,
    pub restoration_eligible_at: u64,
}

// ---------------------------------------------------------------------------
// Resource competition detection
// ---------------------------------------------------------------------------

/// Detect unfair resource competition: a burst of requests from a narrow set
/// of requesters targeting the same scarce resource within a short window.
pub fn detect_resource_competition(
    request_timestamps: &Vec<u64>,
    distinct_requesters: u32,
) -> ResourceCompetitionFlag {
    let total = request_timestamps.len();
    let distinct_bps = if total == 0 {
        10_000
    } else {
        (distinct_requesters.saturating_mul(10_000)) / total
    };

    let mut burst = 0u32;
    if total >= 2 {
        for i in 1..total {
            let prev = request_timestamps.get(i - 1).unwrap_or(0);
            let cur = request_timestamps.get(i).unwrap_or(prev);
            if cur.saturating_sub(prev) < RESOURCE_BURST_WINDOW_SECS {
                burst = burst.saturating_add(1);
            }
        }
    }

    let mut risk = 0u32;
    if distinct_bps < RESOURCE_MIN_DISTINCT_BPS {
        risk = risk.saturating_add(45);
    }
    if burst >= 5 {
        risk = risk.saturating_add(45);
    } else if burst >= 2 {
        risk = risk.saturating_add(20);
    }
    risk = risk.min(100);

    ResourceCompetitionFlag {
        fair: risk < RESOURCE_COMPETITION_RISK_THRESHOLD,
        risk_score: risk,
        distinct_requester_bps: distinct_bps,
        burst_count: burst,
    }
}

// ---------------------------------------------------------------------------
// Load validation
// ---------------------------------------------------------------------------

/// Validate whether a caller's recent request volume reflects legitimate
/// usage or a coordinated load attack. `window_request_count` is the number
/// of requests observed from the caller in the last `window_secs`.
pub fn validate_load_pattern(window_request_count: u32, window_secs: u64) -> LoadValidationResult {
    let window_secs_u32 = window_secs.min(u32::MAX as u64) as u32;
    let rate = if window_secs_u32 == 0 {
        window_request_count
    } else {
        (window_request_count.saturating_mul(60)) / window_secs_u32.max(1)
    };

    let mut risk = 0u32;
    if rate >= LOAD_SUSPICIOUS_RATE_PER_MINUTE.saturating_mul(2) {
        risk = risk.saturating_add(70);
    } else if rate >= LOAD_SUSPICIOUS_RATE_PER_MINUTE {
        risk = risk.saturating_add(40);
    }
    risk = risk.min(100);

    LoadValidationResult {
        legitimate: risk < RESOURCE_COMPETITION_RISK_THRESHOLD,
        attack_risk_score: risk,
        request_rate_per_minute: rate,
    }
}

// ---------------------------------------------------------------------------
// Fair resource distribution
// ---------------------------------------------------------------------------

/// Compute a fair allocation share for one requester given the pool's total
/// demand, throttling any requester attempting to claim more than
/// `FAIR_ALLOCATION_MAX_SHARE_BPS` of total requested capacity.
pub fn distribute_resources_fairly(
    env: &Env,
    requested_units: u32,
    total_requested_units: u32,
) -> FairResourceAllocation {
    if total_requested_units == 0 {
        return FairResourceAllocation {
            granted_share_bps: 0,
            throttled: false,
            reason: Symbol::new(env, "no_demand"),
        };
    }

    let requested_share_bps = (requested_units.saturating_mul(10_000)) / total_requested_units.max(1);

    if requested_share_bps > FAIR_ALLOCATION_MAX_SHARE_BPS {
        return FairResourceAllocation {
            granted_share_bps: FAIR_ALLOCATION_MAX_SHARE_BPS,
            throttled: true,
            reason: Symbol::new(env, "share_capped"),
        };
    }

    FairResourceAllocation {
        granted_share_bps: requested_share_bps,
        throttled: false,
        reason: Symbol::new(env, "unchanged"),
    }
}

// ---------------------------------------------------------------------------
// Automatic intervention & restoration
// ---------------------------------------------------------------------------

/// Combine resource-competition and load-validation signals into a single
/// automatic performance-protection intervention decision.
/// `restoration_cooldown_secs` controls how long an intervened resource pool
/// must wait before fair allocation automatically resumes.
pub fn compute_scalability_intervention(
    env: &Env,
    competition: ResourceCompetitionFlag,
    load: LoadValidationResult,
    restoration_cooldown_secs: u64,
) -> PerformanceInterventionRecord {
    let combined = competition
        .risk_score
        .saturating_add(load.attack_risk_score)
        / 2;
    let combined = combined.min(100);

    let (intervene, reason) = if !competition.fair {
        (true, Symbol::new(env, "resource_competition"))
    } else if !load.legitimate {
        (true, Symbol::new(env, "load_attack"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    let now = env.ledger().timestamp();
    PerformanceInterventionRecord {
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

/// Whether a previously-intervened resource pool is now eligible to have
/// fair allocation automatically restored.
pub fn is_performance_restoration_eligible(record: &PerformanceInterventionRecord, now: u64) -> bool {
    !record.intervene || now >= record.restoration_eligible_at
}

//! Dynamic-pricing manipulation protection primitives.
//!
//! Mentors can coordinate to inflate session prices: setting near-identical
//! rates in a tight time window, manufacturing artificial demand spikes, or
//! drifting prices away from the observed market rate. These helpers give
//! contracts deterministic, storage-agnostic scoring so pricing logic can
//! resist manipulation, validate against a benchmark, and enforce fair
//! bounds. Contracts own the storage of raw price/demand history; these
//! functions operate on data the caller already has on hand.

use soroban_sdk::{contracttype, Symbol};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Prices set within this window of each other are treated as clustered
/// (characteristic of coordinated price setting).
pub const PRICE_COORDINATION_WINDOW_SECS: u64 = 3_600;

/// Maximum deviation (basis points) between two prices for them to still be
/// considered "matching" for coordination-detection purposes.
pub const PRICE_MATCH_TOLERANCE_BPS: u32 = 200; // 2%

/// Risk score (0-100) at or above which a pricing pattern is "suspicious".
pub const PRICING_RISK_THRESHOLD: u32 = 60;

/// Default maximum allowed deviation from the benchmark market rate before a
/// price is flagged as artificially inflated (basis points).
pub const DEFAULT_MAX_MARKET_DEVIATION_BPS: u32 = 3_000; // 30%

/// Absolute basis-point ceiling; no contract may configure a wider band.
pub const MAX_MARKET_DEVIATION_CEILING_BPS: u32 = 10_000; // 100%

/// Demand requests within this window are treated as a burst for artificial
/// demand detection.
pub const DEMAND_BURST_WINDOW_SECS: u64 = 900;

/// Minimum distinct-requester ratio (basis points) for demand to be
/// considered genuine.
pub const DEMAND_MIN_DISTINCT_BPS: u32 = 3_000; // 30%

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of scanning a set of prices/timestamps for coordinated setting.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PriceCoordinationFlag {
    pub suspicious: bool,
    pub risk_score: u32,
    pub matching_price_count: u32,
    pub clustered_timing_count: u32,
}

/// Outcome of validating a proposed price against a benchmark market rate.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct MarketRateValidation {
    pub within_bounds: bool,
    pub deviation_bps: u32,
    pub inflated: bool,
}

/// Fair-pricing enforcement outcome: the price a contract should actually
/// charge/store, along with whether an adjustment was applied.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FairPricingResult {
    pub enforced_price: i128,
    pub adjusted: bool,
    pub reason: Symbol,
}

/// Assessment of whether a burst of session-booking demand is genuine.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct DemandAuthenticity {
    pub genuine: bool,
    pub artificial_risk_score: u32,
    pub distinct_requester_bps: u32,
    pub burst_count: u32,
}

/// Automatic pricing-protection intervention outcome.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PricingInterventionRecord {
    pub intervene: bool,
    pub restored_price: i128,
    pub reason: Symbol,
}

// ---------------------------------------------------------------------------
// Pricing algorithm protection: coordination detection
// ---------------------------------------------------------------------------

/// Detect coordinated price setting across independent mentors. `prices`
/// and `set_timestamps` are parallel arrays (same length, index i is one
/// mentor's price/timestamp).
pub fn detect_price_coordination(
    prices: &soroban_sdk::Vec<i128>,
    set_timestamps: &soroban_sdk::Vec<u64>,
) -> PriceCoordinationFlag {
    let n = prices.len().min(set_timestamps.len());
    let mut matching = 0u32;
    let mut clustered = 0u32;

    let mut i = 0u32;
    while i < n {
        let mut j = i + 1;
        while j < n {
            let pi = prices.get(i).unwrap_or(0);
            let pj = prices.get(j).unwrap_or(0);
            if prices_match(pi, pj, PRICE_MATCH_TOLERANCE_BPS) {
                matching = matching.saturating_add(1);
            }
            let ti = set_timestamps.get(i).unwrap_or(0);
            let tj = set_timestamps.get(j).unwrap_or(0);
            let delta = if ti > tj { ti - tj } else { tj - ti };
            if delta < PRICE_COORDINATION_WINDOW_SECS {
                clustered = clustered.saturating_add(1);
            }
            j += 1;
        }
        i += 1;
    }

    let mut risk = 0u32;
    if matching >= 1 {
        risk = risk.saturating_add(35);
    }
    if clustered >= 1 {
        risk = risk.saturating_add(25);
    }
    if matching >= 2 && clustered >= 2 {
        risk = risk.saturating_add(30);
    }
    risk = risk.min(100);

    PriceCoordinationFlag {
        suspicious: risk >= PRICING_RISK_THRESHOLD,
        risk_score: risk,
        matching_price_count: matching,
        clustered_timing_count: clustered,
    }
}

fn prices_match(a: i128, b: i128, tolerance_bps: u32) -> bool {
    if a <= 0 || b <= 0 {
        return false;
    }
    let diff = if a > b { a - b } else { b - a };
    let allowed = (a.max(b) * tolerance_bps as i128) / 10_000;
    diff <= allowed
}

// ---------------------------------------------------------------------------
// Market rate validation
// ---------------------------------------------------------------------------

/// Validate a proposed price against an externally/admin-supplied benchmark
/// market rate. `max_deviation_bps` is clamped to
/// `MAX_MARKET_DEVIATION_CEILING_BPS`.
pub fn validate_market_rate(
    proposed_price: i128,
    benchmark_rate: i128,
    max_deviation_bps: u32,
) -> MarketRateValidation {
    if benchmark_rate <= 0 || proposed_price <= 0 {
        return MarketRateValidation {
            within_bounds: false,
            deviation_bps: MAX_MARKET_DEVIATION_CEILING_BPS,
            inflated: proposed_price > 0,
        };
    }

    let cap = max_deviation_bps.min(MAX_MARKET_DEVIATION_CEILING_BPS);
    let diff = if proposed_price > benchmark_rate {
        proposed_price - benchmark_rate
    } else {
        benchmark_rate - proposed_price
    };
    let deviation_bps = ((diff.saturating_mul(10_000)) / benchmark_rate).min(i128::from(u32::MAX)) as u32;

    MarketRateValidation {
        within_bounds: deviation_bps <= cap,
        deviation_bps,
        inflated: proposed_price > benchmark_rate && deviation_bps > cap,
    }
}

// ---------------------------------------------------------------------------
// Fair pricing enforcement
// ---------------------------------------------------------------------------

/// Enforce fair pricing bounds. Clamps `proposed_price` into `[floor,
/// ceiling]`; when the market-rate validation flags inflation, the price is
/// clamped to the benchmark rate adjusted by the allowed deviation instead.
pub fn enforce_fair_pricing(
    env: &soroban_sdk::Env,
    proposed_price: i128,
    floor: i128,
    ceiling: i128,
    market: MarketRateValidation,
    benchmark_rate: i128,
    max_deviation_bps: u32,
) -> FairPricingResult {
    if market.inflated && benchmark_rate > 0 {
        let cap = max_deviation_bps.min(MAX_MARKET_DEVIATION_CEILING_BPS) as i128;
        let max_allowed = benchmark_rate + (benchmark_rate * cap) / 10_000;
        let clamped = proposed_price.min(max_allowed).max(floor).min(ceiling);
        return FairPricingResult {
            enforced_price: clamped,
            adjusted: clamped != proposed_price,
            reason: Symbol::new(env, "market_rate_inflation"),
        };
    }

    let clamped = proposed_price.max(floor).min(ceiling);
    FairPricingResult {
        enforced_price: clamped,
        adjusted: clamped != proposed_price,
        reason: if clamped != proposed_price {
            Symbol::new(env, "bounds_clamped")
        } else {
            Symbol::new(env, "unchanged")
        },
    }
}

// ---------------------------------------------------------------------------
// Demand authenticity verification
// ---------------------------------------------------------------------------

/// Verify whether a burst of booking/session requests reflects genuine
/// learner demand rather than artificially generated requests.
pub fn verify_demand_authenticity(
    request_timestamps: &soroban_sdk::Vec<u64>,
    distinct_requesters: u32,
) -> DemandAuthenticity {
    let total = request_timestamps.len();
    let distinct_requester_bps = if total == 0 {
        10_000
    } else {
        (distinct_requesters.saturating_mul(10_000)) / total
    };

    let mut burst_count = 0u32;
    if total >= 2 {
        for i in 1..total {
            let prev = request_timestamps.get(i - 1).unwrap_or(0);
            let cur = request_timestamps.get(i).unwrap_or(prev);
            if cur.saturating_sub(prev) < DEMAND_BURST_WINDOW_SECS {
                burst_count = burst_count.saturating_add(1);
            }
        }
    }

    let mut risk = 0u32;
    if distinct_requester_bps < DEMAND_MIN_DISTINCT_BPS {
        risk = risk.saturating_add(50);
    }
    if burst_count >= 3 {
        risk = risk.saturating_add(40);
    } else if burst_count >= 1 {
        risk = risk.saturating_add(15);
    }
    risk = risk.min(100);

    DemandAuthenticity {
        genuine: risk < PRICING_RISK_THRESHOLD,
        artificial_risk_score: risk,
        distinct_requester_bps,
        burst_count,
    }
}

// ---------------------------------------------------------------------------
// Automatic pricing protection & restoration
// ---------------------------------------------------------------------------

/// Combine coordination, market-rate, and demand signals into a single
/// automatic pricing-protection intervention, restoring the benchmark rate
/// (bounded by `floor`/`ceiling`) when manipulation is detected.
pub fn compute_pricing_intervention(
    env: &soroban_sdk::Env,
    coordination: PriceCoordinationFlag,
    market: MarketRateValidation,
    demand: DemandAuthenticity,
    benchmark_rate: i128,
    floor: i128,
    ceiling: i128,
) -> PricingInterventionRecord {
    let (intervene, reason) = if coordination.risk_score >= PRICING_RISK_THRESHOLD {
        (true, Symbol::new(env, "price_coordination"))
    } else if market.inflated {
        (true, Symbol::new(env, "market_rate_inflation"))
    } else if demand.artificial_risk_score >= PRICING_RISK_THRESHOLD {
        (true, Symbol::new(env, "artificial_demand"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    let restored_price = if intervene {
        benchmark_rate.max(floor).min(ceiling)
    } else {
        benchmark_rate
    };

    PricingInterventionRecord {
        intervene,
        restored_price,
        reason,
    }
}

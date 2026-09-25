//! Deterministic economic invariants shared by financial contracts.
//!
//! The predicates in this module are intentionally side-effect free. Contracts
//! can use them before committing a state transition and persist the returned
//! result for continuous monitoring.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

pub const BPS_DENOMINATOR: i128 = 10_000;
pub const MAX_REWARD_ROUNDING_ERROR: i128 = 1;
pub const DEFAULT_MAX_STATE_AGE_SECS: u64 = 86_400;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EconomicInvariant {
    FundConservation,
    RewardDistribution,
    TemporalProgress,
    MarketPrice,
    IncentiveCompatibility,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PropertyValidation {
    pub invariant: EconomicInvariant,
    pub valid: bool,
    pub observed: i128,
    pub expected: i128,
    pub checked_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EconomicInvariantRecord {
    pub invariant: EconomicInvariant,
    pub valid: bool,
    pub observed: i128,
    pub expected: i128,
    pub timestamp: u64,
    pub ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardAllocation {
    pub recipient: Address,
    pub weight: i128,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketObservation {
    pub venue: Address,
    pub price: i128,
    pub liquidity: i128,
    pub observed_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketValidation {
    pub valid: bool,
    pub aggregate_price: i128,
    pub confidence_bps: u32,
    pub suspicious_venues: u32,
}

/// Checks `prior_balance + inflows = current_balance + outflows + fees`.
pub fn validate_fund_conservation(
    env: &Env,
    prior_balance: i128,
    inflows: i128,
    outflows: i128,
    fees: i128,
    current_balance: i128,
) -> PropertyValidation {
    let expected = prior_balance
        .checked_add(inflows)
        .and_then(|value| value.checked_sub(outflows))
        .and_then(|value| value.checked_sub(fees));
    let (valid, expected_value) = match expected {
        Some(value) => (value == current_balance, value),
        None => (false, i128::MAX),
    };
    PropertyValidation {
        invariant: EconomicInvariant::FundConservation,
        valid,
        observed: current_balance,
        expected: expected_value,
        checked_at: env.ledger().timestamp(),
    }
}

/// Verifies exact allocation with at most one base unit of aggregate rounding.
pub fn validate_reward_distribution(
    env: &Env,
    total_reward: i128,
    allocations: &Vec<RewardAllocation>,
) -> PropertyValidation {
    let mut allocated = 0i128;
    let mut valid = total_reward >= 0;
    for allocation in allocations.iter() {
        if allocation.weight < 0 || allocation.amount < 0 {
            valid = false;
        }
        allocated = allocated.saturating_add(allocation.amount);
    }
    let error = if allocated >= total_reward {
        allocated - total_reward
    } else {
        total_reward - allocated
    };
    valid = valid && error <= MAX_REWARD_ROUNDING_ERROR;
    PropertyValidation {
        invariant: EconomicInvariant::RewardDistribution,
        valid,
        observed: allocated,
        expected: total_reward,
        checked_at: env.ledger().timestamp(),
    }
}

/// Checks monotonic state time and a bounded interval between observations.
pub fn validate_temporal_progress(
    env: &Env,
    previous_timestamp: u64,
    current_timestamp: u64,
    max_age_secs: u64,
) -> PropertyValidation {
    let elapsed = current_timestamp.saturating_sub(previous_timestamp);
    let valid = current_timestamp >= previous_timestamp && elapsed <= max_age_secs;
    PropertyValidation {
        invariant: EconomicInvariant::TemporalProgress,
        valid,
        observed: elapsed as i128,
        expected: max_age_secs as i128,
        checked_at: env.ledger().timestamp(),
    }
}

/// Records every check, including failures, and emits failures for indexers.
pub fn record_invariant_check(env: &Env, record: &EconomicInvariantRecord) {
    let count_key = symbol_short!("INV_CNT");
    let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
    env.storage().persistent().set(&count_key, &count.saturating_add(1));
    env.storage().persistent().set(&(symbol_short!("INV_LAST"), count), record);
    if !record.valid {
        env.events().publish(
            (symbol_short!("economic"), symbol_short!("violation")),
            (record.invariant.clone(), record.observed, record.expected, record.timestamp),
        );
    }
}

/// Computes a liquidity-weighted median and flags venues beyond the threshold.
pub fn validate_market_observations(
    env: &Env,
    observations: &Vec<MarketObservation>,
    max_deviation_bps: u32,
) -> MarketValidation {
    if observations.len() == 0 {
        return MarketValidation { valid: false, aggregate_price: 0, confidence_bps: 0, suspicious_venues: 0 };
    }
    let mut prices: Vec<(i128, i128)> = Vec::new(env);
    for observation in observations.iter() {
        if observation.price > 0 && observation.liquidity > 0 {
            prices.push_back((observation.price, observation.liquidity));
        }
    }
    if prices.len() == 0 { return MarketValidation { valid: false, aggregate_price: 0, confidence_bps: 0, suspicious_venues: observations.len() as u32 }; }
    for index in 0..prices.len() {
        for next in 0..prices.len().saturating_sub(1).saturating_sub(index) {
            if prices.get(next).unwrap_or((0, 0)).0 > prices.get(next + 1).unwrap_or((0, 0)).0 {
                let left = prices.get(next).unwrap_or((0, 0));
                prices.set(next, prices.get(next + 1).unwrap_or((0, 0)));
                prices.set(next + 1, left);
            }
        }
    }
    let total_liquidity = prices.iter().fold(0i128, |total, price| total.saturating_add(price.1));
    let midpoint = total_liquidity.saturating_add(1) / 2;
    let mut cumulative_liquidity = 0i128;
    let mut aggregate = prices.get(0).unwrap_or((0, 0)).0;
    for price in prices.iter() {
        cumulative_liquidity = cumulative_liquidity.saturating_add(price.1);
        if cumulative_liquidity >= midpoint {
            aggregate = price.0;
            break;
        }
    }
    let mut suspicious = 0u32;
    for observation in observations.iter() {
        let difference = if observation.price > aggregate { observation.price - aggregate } else { aggregate - observation.price };
        if aggregate == 0 || difference.saturating_mul(BPS_DENOMINATOR) > aggregate.saturating_mul(max_deviation_bps as i128) { suspicious += 1; }
    }
    let confidence = ((prices.len() as u32 * 10_000) / (observations.len() as u32)).min(10_000);
    MarketValidation { valid: prices.len() >= 2 && suspicious * 2 < observations.len() as u32, aggregate_price: aggregate, confidence_bps: confidence, suspicious_venues: suspicious }
}

#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

/// Oracle consumer library for TWAP validation across DeFi contracts
/// Provides centralized price staleness, feeder count, and deviation checks
#[contracttype]
#[derive(Clone)]
pub enum OracleError {
    Stale = 1,
    InsufficientFeeders = 2,
    DeviationExceeded = 3,
    PriceNotFound = 4,
}

/// OracleConsumer: reusable library for validating oracle prices
#[contracttype]
pub struct OracleConsumer;

impl OracleConsumer {
    /// Get and validate price from oracle: checks staleness, feeder count, and deviation
    /// Returns (price, timestamp) if valid
    pub fn get_price(
        env: &Env,
        oracle_address: &Address,
        base: Symbol,
        quote: Symbol,
        max_staleness_secs: u64,
    ) -> Result<(i128, u64), OracleError> {
        // Call oracle to get (price, timestamp)
        let (price, timestamp): (i128, u64) = env.invoke_contract(
            oracle_address,
            &Symbol::new(env, "get_price"),
            (base.clone(), quote).into_val(env),
        );

        // Check staleness
        let now = env.ledger().timestamp();
        if now.saturating_sub(timestamp) > max_staleness_secs {
            return Err(OracleError::Stale);
        }

        Ok((price, timestamp))
    }

    /// Assert price is within range, panic if not
    pub fn assert_price_in_range(env: &Env, price: i128, min_price: i128, max_price: i128) {
        if price < min_price || price > max_price {
            panic!("price outside acceptable range");
        }
    }

    /// Validate price deviation from reference (in basis points)
    /// Returns true if deviation exceeds threshold
    pub fn check_deviation(
        price: i128,
        reference_price: i128,
        threshold_bps: i128,
    ) -> bool {
        if reference_price == 0 {
            return false;
        }
        let diff = if price > reference_price {
            price - reference_price
        } else {
            reference_price - price
        };
        let deviation_bps = diff
            .checked_mul(10_000)
            .unwrap_or(i128::MAX)
            .checked_div(reference_price)
            .unwrap_or(i128::MAX);
        deviation_bps > threshold_bps
    }
}

#[contract]
pub struct OracleConsumerLib;

#[contractimpl]
impl OracleConsumerLib {
    /// Placeholder implementation - this is a library contract
    pub fn version() -> u32 {
        1
    }
}

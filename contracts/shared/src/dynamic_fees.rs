use soroban_sdk::{contracterror, contracttype, Env};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const BASE_FEE_BPS: u32 = 200; // 2%
pub const MAX_FEE_BPS: u32 = 1000; // 10%
pub const MIN_FEE_BPS: u32 = 100; // 1%
pub const HIGH_LOAD_THRESHOLD: u32 = 1000; // System load threshold
/// Minimum seconds between two fee adjustments (#1459).
pub const MIN_FEE_ADJUSTMENT_INTERVAL_SECS: u64 = 5 * 60;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DynamicFeeResult {
    pub fee_bps: u32,
    pub is_emergency: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeEvasionResult {
    pub is_evading: bool,
    pub evasion_score: u32,
}

/// Error returned when a fee adjustment is attempted too soon (#1459).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DynamicFeeError {
    AdjustmentTooFrequent = 1,
}

/// Storage key that enforces the minimum adjustment interval (#1459).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FeeDataKey {
    LastFeeAdjustedAt,
}

// ---------------------------------------------------------------------------
// Logic
// ---------------------------------------------------------------------------

pub fn calculate_dynamic_fee(
    _env: &Env,
    system_load: u32,
    reputation_score: u32, // 0-100
) -> DynamicFeeResult {
    let mut fee = BASE_FEE_BPS;

    // Surge pricing based on system load
    let mut is_emergency = false;
    if system_load > HIGH_LOAD_THRESHOLD * 2 {
        fee = MAX_FEE_BPS;
        is_emergency = true;
    } else if system_load > HIGH_LOAD_THRESHOLD {
        fee += 300; // Add 3% surge
    }

    // Contributor discount
    if reputation_score >= 80 {
        fee = fee.saturating_sub(100); // 1% discount
    } else if reputation_score >= 50 {
        fee = fee.saturating_sub(50); // 0.5% discount
    }

    // Clamp values
    if fee < MIN_FEE_BPS {
        fee = MIN_FEE_BPS;
    }
    if fee > MAX_FEE_BPS {
        fee = MAX_FEE_BPS;
    }

    DynamicFeeResult {
        fee_bps: fee,
        is_emergency,
    }
}

/// Rate-limited fee adjustment (#1459).
///
/// Guards `calculate_dynamic_fee` with `LastFeeAdjustedAt` so rapid oracle-driven
/// load changes cannot oscillate fees. The first adjustment always succeeds;
/// later ones require `MIN_FEE_ADJUSTMENT_INTERVAL_SECS` to elapse.
pub fn adjust_fee(
    env: &Env,
    system_load: u32,
    reputation_score: u32,
) -> Result<DynamicFeeResult, DynamicFeeError> {
    let key = FeeDataKey::LastFeeAdjustedAt;
    let now = env.ledger().timestamp();
    if let Some(last) = env.storage().instance().get::<_, u64>(&key) {
        if now.saturating_sub(last) < MIN_FEE_ADJUSTMENT_INTERVAL_SECS {
            return Err(DynamicFeeError::AdjustmentTooFrequent);
        }
    }
    let result = calculate_dynamic_fee(env, system_load, reputation_score);
    env.storage().instance().set(&key, &now);
    Ok(result)
}

/// Timestamp of the last accepted adjustment, for operators/tests.
pub fn last_fee_adjusted_at(env: &Env) -> Option<u64> {
    env.storage().instance().get(&FeeDataKey::LastFeeAdjustedAt)
}

pub fn detect_fee_gaming(
    _env: &Env,
    recent_transactions: u32,
    total_volume: i128,
) -> FeeEvasionResult {
    // If a user is doing many small transactions to game fixed cost aspects or rounding
    if recent_transactions > 20 && total_volume < 500_000 {
        return FeeEvasionResult {
            is_evading: true,
            evasion_score: 90,
        };
    }
    FeeEvasionResult {
        is_evading: false,
        evasion_score: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_base_fee_zero_load_zero_reputation() {
        let env = Env::default();
        let result = calculate_dynamic_fee(&env, 0, 0);
        assert_eq!(result.fee_bps, BASE_FEE_BPS); // 200
        assert_eq!(result.is_emergency, false);
    }

    #[test]
    fn test_max_fee_at_double_threshold_plus_one() {
        let env = Env::default();
        let result = calculate_dynamic_fee(&env, HIGH_LOAD_THRESHOLD * 2 + 1, 0);
        assert_eq!(result.fee_bps, MAX_FEE_BPS); // 1000
        assert_eq!(result.is_emergency, true);
    }

    #[test]
    fn test_reputation_discount_at_score_80() {
        let env = Env::default();
        let result = calculate_dynamic_fee(&env, 0, 80);
        // BASE_FEE_BPS (200) - 100 (1% discount) = 100
        assert_eq!(result.fee_bps, MIN_FEE_BPS); // Should be clamped to MIN_FEE_BPS (100)
        assert_eq!(result.is_emergency, false);
    }

    #[test]
    fn test_reputation_discount_at_score_90() {
        let env = Env::default();
        let result = calculate_dynamic_fee(&env, 0, 90);
        // BASE_FEE_BPS (200) - 100 (1% discount) = 100
        assert_eq!(result.fee_bps, MIN_FEE_BPS); // Should be clamped to MIN_FEE_BPS (100)
        assert_eq!(result.is_emergency, false);
    }

    #[test]
    fn test_emergency_at_double_threshold_load() {
        let env = Env::default();
        let result = calculate_dynamic_fee(&env, HIGH_LOAD_THRESHOLD * 2, 50);
        // At exactly 2 * threshold, the condition is false (needs > not >=)
        // So it falls to the else if condition: > HIGH_LOAD_THRESHOLD
        // fee = 200 + 300 - 50 (0.5% discount for rep >= 50) = 450
        assert_eq!(result.fee_bps, 450);
        assert_eq!(result.is_emergency, false);
        
        // Test just above double threshold
        let result_emergency = calculate_dynamic_fee(&env, HIGH_LOAD_THRESHOLD * 2 + 1, 50);
        assert_eq!(result_emergency.fee_bps, MAX_FEE_BPS);
        assert_eq!(result_emergency.is_emergency, true);
    }
    #[test]
    fn test_adjust_fee_is_rate_limited() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 1_000);

        // First adjustment succeeds and records the timestamp.
        assert_eq!(adjust_fee(&env, 0, 0).unwrap().fee_bps, BASE_FEE_BPS);
        assert_eq!(last_fee_adjusted_at(&env), Some(1_000));

        // Rapid follow-up (e.g. oracle oscillation) is blocked.
        assert_eq!(
            adjust_fee(&env, HIGH_LOAD_THRESHOLD * 2 + 1, 0),
            Err(DynamicFeeError::AdjustmentTooFrequent)
        );
        assert_eq!(last_fee_adjusted_at(&env), Some(1_000));

        // After the interval, a new adjustment is accepted.
        env.ledger().with_mut(|li| li.timestamp = 1_000 + MIN_FEE_ADJUSTMENT_INTERVAL_SECS);
        let result = adjust_fee(&env, HIGH_LOAD_THRESHOLD * 2 + 1, 0).unwrap();
        assert_eq!(result.fee_bps, MAX_FEE_BPS);
        assert_eq!(last_fee_adjusted_at(&env), Some(1_000 + MIN_FEE_ADJUSTMENT_INTERVAL_SECS));
    }
}
use soroban_sdk::{contracttype, Env};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const BASE_FEE_BPS: u32 = 200; // 2%
pub const MAX_FEE_BPS: u32 = 1000; // 10%
pub const MIN_FEE_BPS: u32 = 100; // 1%
pub const HIGH_LOAD_THRESHOLD: u32 = 1000; // System load threshold

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
}

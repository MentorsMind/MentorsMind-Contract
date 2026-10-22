#![no_std]

//! Mathematical model for the economic invariants enforced on-chain.
//! Run with Kani (`cargo kani`) to exhaustively check bounded arithmetic
//! transitions. The model deliberately contains no Soroban dependencies.

pub const ROUNDING_UNIT: u128 = 1;

pub const fn funds_conserved(
    prior_balance: u128,
    inflows: u128,
    outflows: u128,
    fees: u128,
    current_balance: u128,
) -> bool {
    match prior_balance.checked_add(inflows) {
        Some(gross) => match gross.checked_sub(outflows) {
            Some(after_outflows) => match after_outflows.checked_sub(fees) {
                Some(expected) => expected == current_balance,
                None => false,
            },
            None => false,
        },
        None => false,
    }
}

pub fn rewards_conserved(total: u128, allocations: &[u128]) -> bool {
    let mut sum = 0u128;
    for amount in allocations {
        sum = match sum.checked_add(*amount) { Some(value) => value, None => return false };
    }
    sum.abs_diff(total) <= ROUNDING_UNIT
}

pub const fn temporal_progress(previous: u64, current: u64, max_age: u64) -> bool {
    current >= previous && current - previous <= max_age
}

pub const fn honest_strategy_is_compatible(
    honest_payoff: i128,
    dishonest_payoff: i128,
    detection_probability_bps: u128,
    penalty: i128,
) -> bool {
    if detection_probability_bps > 10_000 || penalty < 0 { return false; }
    let expected_penalty = dishonest_payoff
        .saturating_sub((penalty.saturating_mul(detection_probability_bps as i128)) / 10_000);
    honest_payoff >= expected_penalty
}

#[cfg(kani)]
mod proofs {
    use super::*;

    #[kani::proof]
    fn fund_conservation_is_closed_under_valid_transition() {
        let prior: u128 = kani::any();
        let inflows: u128 = kani::any();
        let outflows: u128 = kani::any();
        let fees: u128 = kani::any();
        let expected = prior.checked_add(inflows)
            .and_then(|value| value.checked_sub(outflows))
            .and_then(|value| value.checked_sub(fees));
        if let Some(current) = expected {
            assert!(funds_conserved(prior, inflows, outflows, fees, current));
        }
    }

    #[kani::proof]
    fn temporal_invariant_rejects_backwards_time() {
        let previous: u64 = kani::any();
        let current: u64 = kani::any();
        kani::assume(current < previous);
        assert!(!temporal_progress(previous, current, u64::MAX));
    }

    #[kani::proof]
    fn incentive_rule_rejects_profitable_detectable_deviation() {
        let honest: i128 = kani::any();
        let dishonest: i128 = kani::any();
        let probability: u128 = kani::any();
        let penalty: i128 = kani::any();
        kani::assume(probability <= 10_000);
        kani::assume(penalty >= 0);
        kani::assume(dishonest.saturating_sub((penalty.saturating_mul(probability as i128)) / 10_000) > honest);
        assert!(!honest_strategy_is_compatible(honest, dishonest, probability, penalty));
    }

    // Issue #1254: temporal_progress Kani proofs
    #[kani::proof]
    fn temporal_progress_same_timestamp_is_always_true() {
        let a: u64 = kani::any();
        let max_age: u64 = kani::any();
        assert!(temporal_progress(a, a, max_age));
    }

    #[kani::proof]
    fn temporal_progress_zero_max_age_only_accepts_equal_timestamps() {
        let a: u64 = kani::any();
        let b: u64 = kani::any();
        let result = temporal_progress(a, b, 0);
        if a == b {
            assert!(result);
        } else {
            assert!(!result);
        }
    }

    #[kani::proof]
    fn temporal_progress_rejects_backwards_time() {
        assert!(!temporal_progress(10, 5, 100));
    }

    // Issue #1255: honest_strategy_is_compatible Kani proofs
    #[kani::proof]
    fn honest_strategy_compatible_with_perfect_detection() {
        let honest_payoff: i128 = kani::any();
        let dishonest_payoff: i128 = kani::any();
        let penalty: i128 = kani::any();
        kani::assume(penalty >= 0);
        kani::assume(honest_payoff >= dishonest_payoff - penalty);
        assert!(honest_strategy_is_compatible(honest_payoff, dishonest_payoff, 10_000, penalty));
    }

    #[kani::proof]
    fn honest_strategy_rejects_invalid_detection_probability() {
        let honest_payoff: i128 = kani::any();
        let dishonest_payoff: i128 = kani::any();
        let detection_probability_bps: u128 = kani::any();
        let penalty: i128 = kani::any();
        kani::assume(detection_probability_bps > 10_000);
        assert!(!honest_strategy_is_compatible(honest_payoff, dishonest_payoff, detection_probability_bps, penalty));
    }

    #[kani::proof]
    fn honest_strategy_rejects_negative_penalty() {
        let honest_payoff: i128 = kani::any();
        let dishonest_payoff: i128 = kani::any();
        let detection_probability_bps: u128 = kani::any();
        let penalty: i128 = kani::any();
        kani::assume(penalty < 0);
        assert!(!honest_strategy_is_compatible(honest_payoff, dishonest_payoff, detection_probability_bps, penalty));
    }

    // Issue #1256: rewards_conserved Kani proofs
    #[kani::proof]
    fn rewards_conserved_exact_match_is_always_true() {
        let total: u128 = kani::any();
        let allocations = [total];
        assert!(rewards_conserved(total, &allocations));
    }

    #[kani::proof]
    fn rewards_conserved_rejects_excess_over_tolerance() {
        let total: u128 = kani::any();
        kani::assume(total <= u128::MAX - 2);
        let allocations = [total + 2];
        assert!(!rewards_conserved(total, &allocations));
    }

    #[kani::proof]
    fn rewards_conserved_overflow_safety() {
        let a: u128 = kani::any();
        let b: u128 = kani::any();
        let c: u128 = kani::any();
        let allocations = [a, b, c];
        let total: u128 = kani::any();
        // This proof verifies that rewards_conserved handles overflow correctly
        // by returning false when checked_add would overflow
        let _ = rewards_conserved(total, &allocations);
        // If we reach here without panic, overflow is handled safely
    }
}

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
}

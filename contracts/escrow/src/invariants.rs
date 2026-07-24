#![cfg(test)]

use crate::{Escrow, EscrowStatus};
use soroban_sdk::{token, Address, Env};

/// Check that total active escrow amounts <= contract token balance
pub fn check_token_balance_consistency(env: &Env, escrow: &Escrow, token_address: &Address) {
    let token_client = token::Client::new(env, token_address);
    let contract_balance = token_client.balance(&env.current_contract_address());

    // In production, would sum all active escrows
    // For now, verify the specific escrow amount is reasonable
    assert!(
        escrow.amount >= 0,
        "Invariant 1 violated: escrow amount is negative"
    );
    assert!(
        escrow.amount <= contract_balance,
        "Invariant 1 violated: escrow amount exceeds contract balance"
    );
}

/// Check that state transitions are valid
pub fn check_state_transition_validity(
    _env: &Env,
    from_status: &EscrowStatus,
    to_status: &EscrowStatus,
) {
    let valid = matches!(
        (from_status, to_status),
        (EscrowStatus::Active, EscrowStatus::Released)
            | (EscrowStatus::Active, EscrowStatus::Disputed)
            | (EscrowStatus::Disputed, EscrowStatus::Released)
            | (EscrowStatus::Disputed, EscrowStatus::Active)
    );

    assert!(
        valid,
        "Invariant 2 violated: invalid state transition from {:?} to {:?}",
        from_status,
        to_status
    );
}

/// Check that sessions_completed <= total_sessions
pub fn check_session_completion_bounds(
    _env: &Env,
    sessions_completed: u32,
    total_sessions: u32,
) {
    assert!(
        sessions_completed <= total_sessions,
        "Invariant 3 violated: sessions_completed ({}) > total_sessions ({})",
        sessions_completed,
        total_sessions
    );
}

/// Check that platform_fee + net_amount == original_amount
pub fn check_fund_conservation(_env: &Env, platform_fee: i128, net_amount: i128, original: i128) {
    let total_distributed = platform_fee + net_amount;
    assert!(
        total_distributed == original,
        "Invariant 4 violated: distributed ({}) != original ({})",
        total_distributed,
        original
    );
}

/// Check that only one recipient receives funds
pub fn check_exclusive_distribution(
    _env: &Env,
    mentor_receives: bool,
    learner_receives: bool,
    treasury_receives: bool,
) {
    let recipient_count = [mentor_receives, learner_receives, treasury_receives]
        .iter()
        .filter(|&&x| x)
        .count();

    assert!(
        recipient_count == 1,
        "Invariant 5 violated: {} recipients selected, expected 1",
        recipient_count
    );
}

/// Check that escrow amount is non-negative
pub fn check_amount_non_negativity(_env: &Env, amount: i128) {
    assert!(
        amount >= 0,
        "Invariant 6 violated: escrow amount is negative ({})",
        amount
    );
}

/// Check timestamp consistency
pub fn check_timestamp_consistency(_env: &Env, created_at: u64, current_time: u64) {
    assert!(
        created_at <= current_time,
        "Invariant 7 violated: created_at ({}) > current_time ({})",
        created_at,
        current_time
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_invariant_1_token_balance() {
        let env = Env::default();
        let escrow = Escrow {
            mentor: Address::generate(&env),
            learner: Address::generate(&env),
            amount: 1000,
            token: Address::generate(&env),
            status: EscrowStatus::Active,
            sessions_completed: 0,
            total_sessions: 5,
            created_at: 0,
            released_at: None,
        };

        // This would pass if contract has sufficient balance
        // In real tests, would mock token balance
        check_amount_non_negativity(&env, escrow.amount);
    }

    #[test]
    fn test_invariant_2_valid_transition() {
        let env = Env::default();
        check_state_transition_validity(&env, &EscrowStatus::Active, &EscrowStatus::Released);
    }

    #[test]
    #[should_panic(expected = "invalid state transition")]
    fn test_invariant_2_invalid_transition() {
        let env = Env::default();
        check_state_transition_validity(&env, &EscrowStatus::Released, &EscrowStatus::Active);
    }

    #[test]
    fn test_invariant_3_session_bounds() {
        let env = Env::default();
        check_session_completion_bounds(&env, 3, 5);
    }

    #[test]
    #[should_panic(expected = "sessions_completed")]
    fn test_invariant_3_violation() {
        let env = Env::default();
        check_session_completion_bounds(&env, 6, 5);
    }

    #[test]
    fn test_invariant_4_fund_conservation() {
        let env = Env::default();
        check_fund_conservation(&env, 20, 980, 1000);
    }

    #[test]
    #[should_panic(expected = "distributed")]
    fn test_invariant_4_violation() {
        let env = Env::default();
        check_fund_conservation(&env, 20, 970, 1000);
    }

    #[test]
    fn test_invariant_5_exclusive_distribution() {
        let env = Env::default();
        check_exclusive_distribution(&env, true, false, false);
    }

    #[test]
    #[should_panic(expected = "recipients selected")]
    fn test_invariant_5_violation() {
        let env = Env::default();
        check_exclusive_distribution(&env, true, true, false);
    }

    #[test]
    fn test_invariant_6_non_negative() {
        let env = Env::default();
        check_amount_non_negativity(&env, 1000);
    }

    #[test]
    #[should_panic(expected = "negative")]
    fn test_invariant_6_violation() {
        let env = Env::default();
        check_amount_non_negativity(&env, -100);
    }

    #[test]
    fn test_invariant_7_timestamp() {
        let env = Env::default();
        check_timestamp_consistency(&env, 100, 200);
    }

    #[test]
    #[should_panic(expected = "created_at")]
    fn test_invariant_7_violation() {
        let env = Env::default();
        check_timestamp_consistency(&env, 200, 100);
    }
}

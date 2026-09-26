use mentorminds_escrow::EscrowStatus;
use mentorminds_governance::ProposalStatus;
use shared::StateMachine;
use soroban_sdk::Env;

#[test]
fn test_escrow_state_machine_transitions() {
    let env = Env::default();
    let states = [
        EscrowStatus::Pending,
        EscrowStatus::Active,
        EscrowStatus::Released,
        EscrowStatus::Disputed,
        EscrowStatus::Refunded,
        EscrowStatus::Resolved,
    ];

    for from in states.iter() {
        for to in states.iter() {
            let is_valid = EscrowStatus::is_valid_transition(&env, from, to);
            let expected_valid = matches!(
                (from, to),
                (EscrowStatus::Pending,  EscrowStatus::Active)
                    | (EscrowStatus::Active,   EscrowStatus::Released)
                    | (EscrowStatus::Active,   EscrowStatus::Disputed)
                    | (EscrowStatus::Active,   EscrowStatus::Refunded)
                    | (EscrowStatus::Disputed, EscrowStatus::Resolved)
                    | (EscrowStatus::Disputed, EscrowStatus::Refunded)
                    | (EscrowStatus::Pending,  EscrowStatus::Refunded)
            );
            assert_eq!(
                is_valid, expected_valid,
                "Escrow transition validation failed from {:?} to {:?}",
                from, to
            );
        }
    }
}

#[test]
fn test_governance_state_machine_transitions() {
    let env = Env::default();
    let states = [
        ProposalStatus::Active,
        ProposalStatus::Passed,
        ProposalStatus::Failed,
        ProposalStatus::Executed,
        ProposalStatus::Cancelled,
    ];

    for from in states.iter() {
        for to in states.iter() {
            let is_valid = ProposalStatus::is_valid_transition(&env, from, to);
            let expected_valid = matches!(
                (from, to),
                (ProposalStatus::Active, ProposalStatus::Passed)
                    | (ProposalStatus::Active, ProposalStatus::Failed)
                    | (ProposalStatus::Active, ProposalStatus::Cancelled)
                    | (ProposalStatus::Passed, ProposalStatus::Executed)
            );
            assert_eq!(
                is_valid, expected_valid,
                "Governance transition validation failed from {:?} to {:?}",
                from, to
            );
        }
    }
}

#[test]
fn test_subscription_state_machine_transitions() {
    use shared::state_machine::SubscriptionStatus;

    let env = Env::default();

    // All 6 states — the loop below checks every one of the 6×6 = 36 pairs.
    let states = [
        SubscriptionStatus::Trial,
        SubscriptionStatus::Active,
        SubscriptionStatus::GracePeriod,
        SubscriptionStatus::Paused,
        SubscriptionStatus::Cancelled,
        SubscriptionStatus::Expired,
    ];

    // ── Exhaustive 6×6 matrix ─────────────────────────────────────────────
    // Only the 9 transitions listed in `matches!` are valid; every other
    // (from, to) pair must return false.
    for from in states.iter() {
        for to in states.iter() {
            let is_valid = SubscriptionStatus::is_valid_transition(&env, from, to);
            let expected_valid = matches!(
                (from, to),
                // Trial can activate or be cancelled before it starts.
                (SubscriptionStatus::Trial,       SubscriptionStatus::Active)
                | (SubscriptionStatus::Trial,       SubscriptionStatus::Cancelled)
                // Active subscription moves to grace on missed payment,
                // can be paused voluntarily, or cancelled outright.
                | (SubscriptionStatus::Active,      SubscriptionStatus::GracePeriod)
                | (SubscriptionStatus::Active,      SubscriptionStatus::Paused)
                | (SubscriptionStatus::Active,      SubscriptionStatus::Cancelled)
                // GracePeriod → Active: renewal during grace period restores
                // the subscription; GracePeriod → Expired: grace window closes.
                | (SubscriptionStatus::GracePeriod, SubscriptionStatus::Active)
                | (SubscriptionStatus::GracePeriod, SubscriptionStatus::Expired)
                // Paused subscription can be resumed or cancelled.
                // Paused → Cancelled is valid; Paused → Expired is not.
                | (SubscriptionStatus::Paused,      SubscriptionStatus::Active)
                | (SubscriptionStatus::Paused,      SubscriptionStatus::Cancelled)
            );
            assert_eq!(
                is_valid, expected_valid,
                "Subscription transition {:?} → {:?}: expected {}, got {}",
                from, to, expected_valid, is_valid
            );
        }
    }

    // ── Explicit spot-checks for the acceptance-criteria cases ────────────

    // GracePeriod → Active (renewal during grace) must be valid.
    assert!(
        SubscriptionStatus::is_valid_transition(
            &env,
            &SubscriptionStatus::GracePeriod,
            &SubscriptionStatus::Active
        ),
        "GracePeriod → Active (renewal during grace) must be a valid transition"
    );

    // Cancelled is a terminal state: every transition out of it is invalid.
    for to in states.iter() {
        assert!(
            !SubscriptionStatus::is_valid_transition(&env, &SubscriptionStatus::Cancelled, to),
            "Cancelled → {:?} must be invalid (Cancelled is a terminal state)",
            to
        );
    }

    // Expired is also a terminal state: every transition out of it is invalid.
    for to in states.iter() {
        assert!(
            !SubscriptionStatus::is_valid_transition(&env, &SubscriptionStatus::Expired, to),
            "Expired → {:?} must be invalid (Expired is a terminal state)",
            to
        );
    }

    // Paused → Cancelled is valid (explicit per-spec check).
    assert!(
        SubscriptionStatus::is_valid_transition(
            &env,
            &SubscriptionStatus::Paused,
            &SubscriptionStatus::Cancelled
        ),
        "Paused → Cancelled must be a valid transition"
    );

    // Paused → Expired is NOT valid (paused subscriptions don't expire directly).
    assert!(
        !SubscriptionStatus::is_valid_transition(
            &env,
            &SubscriptionStatus::Paused,
            &SubscriptionStatus::Expired
        ),
        "Paused → Expired must be invalid"
    );
}

#[test]
fn test_loan_state_machine_transitions() {
    let env = Env::default();
    let states = [
        shared::state_machine::LoanStatus::Pending,
        shared::state_machine::LoanStatus::Active,
        shared::state_machine::LoanStatus::Repaid,
        shared::state_machine::LoanStatus::Defaulted,
        shared::state_machine::LoanStatus::Cancelled,
    ];

    for from in states.iter() {
        for to in states.iter() {
            let is_valid = shared::state_machine::LoanStatus::is_valid_transition(&env, from, to);
            let expected_valid = matches!(
                (from, to),
                (
                    shared::state_machine::LoanStatus::Pending,
                    shared::state_machine::LoanStatus::Active,
                ) | (
                    shared::state_machine::LoanStatus::Pending,
                    shared::state_machine::LoanStatus::Cancelled,
                ) | (
                    shared::state_machine::LoanStatus::Active,
                    shared::state_machine::LoanStatus::Repaid,
                ) | (
                    shared::state_machine::LoanStatus::Active,
                    shared::state_machine::LoanStatus::Defaulted,
                )
            );
            assert_eq!(
                is_valid, expected_valid,
                "Loan transition validation failed from {:?} to {:?}",
                from, to
            );
        }
    }
}

#[test]
fn test_isa_state_machine_transitions() {
    let env = Env::default();
    let states = [
        shared::state_machine::ISAStatus::Pending,
        shared::state_machine::ISAStatus::StudyPeriod,
        shared::state_machine::ISAStatus::GracePeriod,
        shared::state_machine::ISAStatus::Repayment,
        shared::state_machine::ISAStatus::Completed,
        shared::state_machine::ISAStatus::Defaulted,
    ];

    for from in states.iter() {
        for to in states.iter() {
            let is_valid = shared::state_machine::ISAStatus::is_valid_transition(&env, from, to);
            let expected_valid = matches!(
                (from, to),
                (
                    shared::state_machine::ISAStatus::Pending,
                    shared::state_machine::ISAStatus::StudyPeriod,
                ) | (
                    shared::state_machine::ISAStatus::StudyPeriod,
                    shared::state_machine::ISAStatus::GracePeriod,
                ) | (
                    shared::state_machine::ISAStatus::GracePeriod,
                    shared::state_machine::ISAStatus::Repayment,
                ) | (
                    shared::state_machine::ISAStatus::Repayment,
                    shared::state_machine::ISAStatus::Completed,
                ) | (
                    shared::state_machine::ISAStatus::Repayment,
                    shared::state_machine::ISAStatus::Defaulted,
                )
            );
            assert_eq!(
                is_valid, expected_valid,
                "ISA transition validation failed from {:?} to {:?}",
                from, to
            );
        }
    }
}

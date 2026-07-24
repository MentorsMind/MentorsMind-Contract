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
    let env = Env::default();
    let states = [
        shared::state_machine::SubscriptionStatus::Trial,
        shared::state_machine::SubscriptionStatus::Active,
        shared::state_machine::SubscriptionStatus::GracePeriod,
        shared::state_machine::SubscriptionStatus::Paused,
        shared::state_machine::SubscriptionStatus::Cancelled,
        shared::state_machine::SubscriptionStatus::Expired,
    ];

    for from in states.iter() {
        for to in states.iter() {
            let is_valid =
                shared::state_machine::SubscriptionStatus::is_valid_transition(&env, from, to);
            let expected_valid = matches!(
                (from, to),
                (
                    shared::state_machine::SubscriptionStatus::Trial,
                    shared::state_machine::SubscriptionStatus::Active,
                ) | (
                    shared::state_machine::SubscriptionStatus::Trial,
                    shared::state_machine::SubscriptionStatus::Cancelled,
                ) | (
                    shared::state_machine::SubscriptionStatus::Active,
                    shared::state_machine::SubscriptionStatus::GracePeriod,
                ) | (
                    shared::state_machine::SubscriptionStatus::Active,
                    shared::state_machine::SubscriptionStatus::Paused,
                ) | (
                    shared::state_machine::SubscriptionStatus::Active,
                    shared::state_machine::SubscriptionStatus::Cancelled,
                ) | (
                    shared::state_machine::SubscriptionStatus::GracePeriod,
                    shared::state_machine::SubscriptionStatus::Active,
                ) | (
                    shared::state_machine::SubscriptionStatus::GracePeriod,
                    shared::state_machine::SubscriptionStatus::Expired,
                ) | (
                    shared::state_machine::SubscriptionStatus::Paused,
                    shared::state_machine::SubscriptionStatus::Active,
                ) | (
                    shared::state_machine::SubscriptionStatus::Paused,
                    shared::state_machine::SubscriptionStatus::Cancelled,
                )
            );
            assert_eq!(
                is_valid, expected_valid,
                "Subscription transition validation failed from {:?} to {:?}",
                from, to
            );
        }
    }
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

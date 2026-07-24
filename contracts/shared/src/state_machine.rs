use soroban_sdk::{contracttype, Env};

pub trait StateMachine {
    type State;

    /// Checks if a transition from `from` to `to` is valid.
    fn is_valid_transition(env: &Env, from: &Self::State, to: &Self::State) -> bool;
}

// ---------------------------------------------------------------------------
// EscrowStatus state machine
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EscrowStatus {
    /// Escrow created but funds not yet deposited (pre-funding state).
    Pending,
    /// Funds locked; session in progress.
    Active,
    /// Funds released to mentor.
    Released,
    /// Participant raised a dispute; funds frozen.
    Disputed,
    /// Admin refunded funds to learner.
    Refunded,
    /// Dispute resolved by admin arbitration.
    Resolved,
}

impl StateMachine for EscrowStatus {
    type State = Self;

    fn is_valid_transition(_env: &Env, from: &Self::State, to: &Self::State) -> bool {
        matches!(
            (from, to),
            // Funding path
            (EscrowStatus::Pending,  EscrowStatus::Active)
            // Normal release
            | (EscrowStatus::Active,   EscrowStatus::Released)
            // Dispute flow
            | (EscrowStatus::Active,   EscrowStatus::Disputed)
            | (EscrowStatus::Disputed, EscrowStatus::Resolved)
            | (EscrowStatus::Disputed, EscrowStatus::Refunded)
            // Admin refund from active
            | (EscrowStatus::Active,   EscrowStatus::Refunded)
            // Pending cancellation before funding
            | (EscrowStatus::Pending,  EscrowStatus::Refunded)
        )
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionStatus {
    Trial,
    Active,
    GracePeriod,
    Paused,
    Cancelled,
    Expired,
}

impl StateMachine for SubscriptionStatus {
    type State = Self;
    fn is_valid_transition(_env: &Env, from: &Self::State, to: &Self::State) -> bool {
        matches!(
            (from, to),
            (SubscriptionStatus::Trial, SubscriptionStatus::Active)
                | (SubscriptionStatus::Trial, SubscriptionStatus::Cancelled)
                | (SubscriptionStatus::Active, SubscriptionStatus::GracePeriod)
                | (SubscriptionStatus::Active, SubscriptionStatus::Paused)
                | (SubscriptionStatus::Active, SubscriptionStatus::Cancelled)
                | (SubscriptionStatus::GracePeriod, SubscriptionStatus::Active)
                | (SubscriptionStatus::GracePeriod, SubscriptionStatus::Expired)
                | (SubscriptionStatus::Paused, SubscriptionStatus::Active)
                | (SubscriptionStatus::Paused, SubscriptionStatus::Cancelled)
        )
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LoanStatus {
    Pending,
    Active,
    Repaid,
    Defaulted,
    Cancelled,
}

impl StateMachine for LoanStatus {
    type State = Self;
    fn is_valid_transition(_env: &Env, from: &Self::State, to: &Self::State) -> bool {
        matches!(
            (from, to),
            (LoanStatus::Pending, LoanStatus::Active)
                | (LoanStatus::Pending, LoanStatus::Cancelled)
                | (LoanStatus::Active, LoanStatus::Repaid)
                | (LoanStatus::Active, LoanStatus::Defaulted)
        )
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ISAStatus {
    Pending,
    StudyPeriod,
    GracePeriod,
    Repayment,
    Completed,
    Defaulted,
}

impl StateMachine for ISAStatus {
    type State = Self;
    fn is_valid_transition(_env: &Env, from: &Self::State, to: &Self::State) -> bool {
        matches!(
            (from, to),
            (ISAStatus::Pending, ISAStatus::StudyPeriod)
                | (ISAStatus::StudyPeriod, ISAStatus::GracePeriod)
                | (ISAStatus::GracePeriod, ISAStatus::Repayment)
                | (ISAStatus::Repayment, ISAStatus::Completed)
                | (ISAStatus::Repayment, ISAStatus::Defaulted)
        )
    }
}

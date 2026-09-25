use soroban_sdk::{contracttype, Env, Symbol, BytesN};

pub trait StateMachine {
    type State: Clone;

    /// Checks if a transition from `from` to `to` is valid.
    fn is_valid_transition(env: &Env, from: &Self::State, to: &Self::State) -> bool;

    /// Transitions from `from` to `to`, panicking on invalid transitions.
    fn transition(env: &Env, from: &Self::State, to: &Self::State) -> Self::State {
        if !Self::is_valid_transition(env, from, to) {
            panic!("Invalid state transition");
        }
        to.clone()
    }

    /// Atomic transition with validation checkpoints (enhanced for safety)
    /// Default implementation; contracts can override for custom validation
    fn atomic_transition(
        env: &Env,
        from: &Self::State,
        to: &Self::State,
        _pre_checks: bool,
        _post_checks: bool,
    ) -> Result<Self::State, &'static str> {
        // Pre-condition: current state must be valid
        if !Self::is_valid_transition(env, from, to) {
            return Err("Invalid state transition");
        }

        // Perform transition
        let new_state = to.clone();

        // Post-condition: new state should be reachable
        // (In base implementation, this is already validated above)

        Ok(new_state)
    }
}

/// Atomic state transition control structure
#[contracttype]
#[derive(Clone, Debug)]
pub struct AtomicTransitionControl {
    /// Unique ID for this transition
    pub control_id: BytesN<32>,
    /// Entity being transitioned
    pub entity_id: u64,
    /// Lock is held by this address
    pub lock_holder: Symbol,
    /// Current checkpoint index
    pub checkpoint_index: u32,
    /// Total checkpoints required
    pub total_checkpoints: u32,
    /// Transition timed out at this timestamp
    pub timeout_at: u64,
}

// ---------------------------------------------------------------------------
// EscrowStatus state machine
// ---------------------------------------------------------------------------

pub use crate::escrow::EscrowStatus;

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

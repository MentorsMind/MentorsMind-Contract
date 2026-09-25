#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Steps a mentor must complete before onboarding is finished.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum OnboardingStep {
    Verified,
    Bonded,
    FirstSessionCompleted,
}

/// Tracks which onboarding steps a mentor has completed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnboardingStatus {
    pub is_verified: bool,
    pub is_bonded: bool,
    pub has_completed_first_session: bool,
    pub onboarding_complete: bool,
}

/// Rent/health info returned by queries.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OnboardingInfo {
    pub mentor: Address,
    pub status: OnboardingStatus,
    pub escrow_extended: bool,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    /// OnboardingStatus per mentor.
    MentorOnboardingStatus(Address),
    /// Whether a mentor's first escrow uses extended delay.
    FirstEscrowExtended(Address),
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

const EVT_STEP: Symbol = symbol_short!("STEP");
const EVT_DONE: Symbol = symbol_short!("DONE");

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// 7-day extended auto-release delay for first-session escrow (in seconds).
pub const EXTENDED_AUTO_RELEASE_SECS: u64 = 7 * 24 * 60 * 60;

/// 30-day refund deadline for incomplete onboarding (in seconds).
pub const ONBOARDING_DEADLINE_SECS: u64 = 30 * 24 * 60 * 60;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct OnboardingEscrow;

#[contractimpl]
impl OnboardingEscrow {
    /// Initialise a mentor's onboarding status. Called once when the mentor
    /// first registers or when the factory creates their first escrow.
    pub fn init_mentor(env: Env, mentor: Address) {
        mentor.require_auth();
        let key = DataKey::MentorOnboardingStatus(mentor.clone());
        if env.storage().persistent().has(&key) {
            return; // already initialised
        }
        let status = OnboardingStatus {
            is_verified: false,
            is_bonded: false,
            has_completed_first_session: false,
            onboarding_complete: false,
        };
        env.storage().persistent().set(&key, &status);
        env.storage()
            .persistent()
            .set(&DataKey::FirstEscrowExtended(mentor), &true);
    }

    /// Mark a step as completed. Callable by the relevant subsystem contract
    /// (verification, performance_bond, or escrow release).
    pub fn complete_onboarding_step(env: Env, mentor: Address, step: OnboardingStep) {
        mentor.require_auth();
        let key = DataKey::MentorOnboardingStatus(mentor.clone());
        let mut status: OnboardingStatus = env
            .storage()
            .persistent()
            .get(&key)
            .expect("mentor not initialised");

        if status.onboarding_complete {
            return; // nothing to do
        }

        match step {
            OnboardingStep::Verified => status.is_verified = true,
            OnboardingStep::Bonded => status.is_bonded = true,
            OnboardingStep::FirstSessionCompleted => {
                status.has_completed_first_session = true
            }
        }

        let remaining = Self::remaining_steps(&env, &status);
        let all_done = remaining.is_empty();

        if all_done {
            status.onboarding_complete = true;
        }

        env.storage().persistent().set(&key, &status);

        // Event: step completed
        env.events().publish(
            (EVT_STEP, mentor.clone()),
            (step.clone(), remaining.clone()),
        );

        // Event: onboarding complete
        if all_done {
            env.events().publish((EVT_DONE, mentor), ());
        }
    }

    /// Returns the current onboarding status for a mentor.
    pub fn get_onboarding_status(env: Env, mentor: Address) -> OnboardingStatus {
        env.storage()
            .persistent()
            .get(&DataKey::MentorOnboardingStatus(mentor))
            .expect("mentor not initialised")
    }

    /// Whether the mentor's first escrow should use the extended delay.
    pub fn is_first_escrow_extended(env: Env, mentor: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::FirstEscrowExtended(mentor))
            .unwrap_or(false)
    }

    /// Returns true if onboarding is complete (all steps done).
    pub fn is_onboarding_complete(env: Env, mentor: Address) -> bool {
        let status = Self::get_onboarding_status(env, mentor);
        status.onboarding_complete
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn remaining_steps(env: &Env, status: &OnboardingStatus) -> soroban_sdk::Vec<OnboardingStep> {
        let mut steps = soroban_sdk::Vec::new(env);
        if !status.is_verified {
            steps.push_back(OnboardingStep::Verified);
        }
        if !status.is_bonded {
            steps.push_back(OnboardingStep::Bonded);
        }
        if !status.has_completed_first_session {
            steps.push_back(OnboardingStep::FirstSessionCompleted);
        }
        steps
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(OnboardingEscrow, ());
        let mentor = Address::generate(&env);
        (env, contract_id, mentor)
    }

    #[test]
    fn test_init_mentor() {
        let (env, _cid, mentor) = setup();
        let client = OnboardingEscrowClient::new(&env, &_cid);
        client.init_mentor(&mentor);
        let status = client.get_onboarding_status(&mentor);
        assert!(!status.is_verified);
        assert!(!status.is_bonded);
        assert!(!status.has_completed_first_session);
        assert!(!status.onboarding_complete);
    }

    #[test]
    fn test_first_escrow_extended_default() {
        let (env, _cid, mentor) = setup();
        let client = OnboardingEscrowClient::new(&env, &_cid);
        client.init_mentor(&mentor);
        assert!(client.is_first_escrow_extended(&mentor));
    }

    #[test]
    fn test_complete_verified_step() {
        let (env, _cid, mentor) = setup();
        let client = OnboardingEscrowClient::new(&env, &_cid);
        client.init_mentor(&mentor);
        client.complete_onboarding_step(&mentor, &OnboardingStep::Verified);
        let status = client.get_onboarding_status(&mentor);
        assert!(status.is_verified);
        assert!(!status.onboarding_complete);
    }

    #[test]
    fn test_complete_all_steps() {
        let (env, _cid, mentor) = setup();
        let client = OnboardingEscrowClient::new(&env, &_cid);
        client.init_mentor(&mentor);
        client.complete_onboarding_step(&mentor, &OnboardingStep::Verified);
        client.complete_onboarding_step(&mentor, &OnboardingStep::Bonded);
        client.complete_onboarding_step(
            &mentor,
            &OnboardingStep::FirstSessionCompleted,
        );
        let status = client.get_onboarding_status(&mentor);
        assert!(status.onboarding_complete);
        assert!(client.is_onboarding_complete(&mentor));
    }

    #[test]
    fn test_step_completed_after_onboarding_done_is_noop() {
        let (env, _cid, mentor) = setup();
        let client = OnboardingEscrowClient::new(&env, &_cid);
        client.init_mentor(&mentor);
        client.complete_onboarding_step(&mentor, &OnboardingStep::Verified);
        client.complete_onboarding_step(&mentor, &OnboardingStep::Bonded);
        client.complete_onboarding_step(
            &mentor,
            &OnboardingStep::FirstSessionCompleted,
        );
        // Call again — should not panic
        client.complete_onboarding_step(&mentor, &OnboardingStep::Verified);
        assert!(client.is_onboarding_complete(&mentor));
    }

    #[test]
    fn test_onboarding_complete_all_steps_done() {
        let (env, _cid, mentor) = setup();
        let client = OnboardingEscrowClient::new(&env, &_cid);
        client.init_mentor(&mentor);
        assert!(!client.is_onboarding_complete(&mentor));
        client.complete_onboarding_step(&mentor, &OnboardingStep::Verified);
        assert!(!client.is_onboarding_complete(&mentor));
        client.complete_onboarding_step(&mentor, &OnboardingStep::Bonded);
        assert!(!client.is_onboarding_complete(&mentor));
        client.complete_onboarding_step(
            &mentor,
            &OnboardingStep::FirstSessionCompleted,
        );
        assert!(client.is_onboarding_complete(&mentor));
    }
}

#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol};

const EXPIRY_GRACE_SECS: u64 = 48 * 60 * 60;
const DISPUTE_WINDOW_SECS: u64 = 48 * 60 * 60;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionState {
    Pending,
    Completed,
    Disputed,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSession {
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub escrow_id: u64,
    pub end_time: u64,
    pub confirmation_deadline: u64,
    pub dispute_deadline: u64,
    pub mentor_confirmed: bool,
    pub learner_confirmed: bool,
    pub completed_at: u64,
    pub state: SessionState,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Session(Symbol),
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    SessionAlreadyExists = 2,
    SessionNotFound = 3,
    Unauthorized = 4,
    SessionDisputed = 5,
    SessionExpired = 6,
    DeadlineNotPassed = 7,
    NoConfirmations = 8,
}

#[contract]
pub struct SessionOracleContract;

#[contractimpl]
impl SessionOracleContract {
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        Ok(())
    }

    pub fn register_session(
        env: Env,
        admin: Address,
        session_id: Symbol,
        mentor: Address,
        learner: Address,
        escrow_id: u64,
        end_time: u64,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        let key = DataKey::Session(session_id.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::SessionAlreadyExists);
        }

        let confirmation_deadline = end_time.saturating_add(EXPIRY_GRACE_SECS);
        let dispute_deadline = confirmation_deadline.saturating_add(DISPUTE_WINDOW_SECS);
        let record = OracleSession {
            session_id: session_id.clone(),
            mentor,
            learner,
            escrow_id,
            end_time,
            confirmation_deadline,
            dispute_deadline,
            mentor_confirmed: false,
            learner_confirmed: false,
            completed_at: 0,
            state: SessionState::Pending,
        };
        env.storage().persistent().set(&key, &record);
        env.events().publish(
            (Symbol::new(&env, "session_registered"), session_id),
            escrow_id,
        );
        Ok(())
    }

    pub fn confirm_completion(
        env: Env,
        session_id: Symbol,
        participant: Address,
    ) -> Result<OracleSession, Error> {
        participant.require_auth();
        let key = DataKey::Session(session_id.clone());
        let mut record: OracleSession = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::SessionNotFound)?;

        if record.state == SessionState::Disputed {
            return Err(Error::SessionDisputed);
        }
        if participant != record.mentor && participant != record.learner {
            return Err(Error::Unauthorized);
        }

        if participant == record.mentor {
            record.mentor_confirmed = true;
        }
        if participant == record.learner {
            record.learner_confirmed = true;
        }

        if record.mentor_confirmed && record.learner_confirmed {
            record.state = SessionState::Completed;
            record.completed_at = env.ledger().timestamp();
            env.events().publish(
                (
                    Symbol::new(&env, "escrow_release_ready"),
                    session_id.clone(),
                ),
                record.escrow_id,
            );
        }

        env.storage().persistent().set(&key, &record);
        env.events().publish(
            (Symbol::new(&env, "session_confirmed"), session_id),
            participant,
        );
        Ok(record)
    }

    pub fn raise_dispute(env: Env, session_id: Symbol, participant: Address) -> Result<(), Error> {
        participant.require_auth();
        let key = DataKey::Session(session_id.clone());
        let mut record: OracleSession = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::SessionNotFound)?;
        if participant != record.mentor && participant != record.learner {
            return Err(Error::Unauthorized);
        }
        record.state = SessionState::Disputed;
        env.storage().persistent().set(&key, &record);
        env.events().publish(
            (Symbol::new(&env, "session_disputed"), session_id),
            participant,
        );
        Ok(())
    }
    /// Trigger timeout release: if at least one confirmation after confirmation_deadline,
    /// auto-release escrow. If zero confirmations after dispute_deadline, auto-dispute.
    pub fn trigger_timeout_release(env: Env, session_id: Symbol) -> Result<(), Error> {
        let key = DataKey::Session(session_id.clone());
        let mut record: OracleSession = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::SessionNotFound)?;

        let now = env.ledger().timestamp();
        if now <= record.confirmation_deadline {
            return Err(Error::DeadlineNotPassed);
        }

        let confirmed_by = if record.mentor_confirmed {
            record.mentor.clone()
        } else if record.learner_confirmed {
            record.learner.clone()
        } else {
            // zero confirmations
            if now > record.dispute_deadline {
                // auto-dispute
                record.state = SessionState::Disputed;
                env.storage().persistent().set(&key, &record);
                env.events().publish(
                    (Symbol::new(&env, "session_disputed"), session_id.clone()),
                    env.invoker(),
                );
                return Ok(());
            } else {
                return Err(Error::NoConfirmations);
            }
        };

        // At least one confirmation, auto-release
        record.state = SessionState::Completed;
        record.completed_at = now;
        env.storage().persistent().set(&key, &record);
        env.events().publish(
            (
                Symbol::new(&env, "escrow_release_ready"),
                session_id.clone(),
            ),
            record.escrow_id,
        );
        env.events().publish(
            (
                Symbol::new(&env, "ConfirmationTimeout"),
                session_id.clone(),
            ),
            (confirmed_by, true),
        );
        Ok(())
    }

    pub fn get_session(env: Env, session_id: Symbol) -> Result<OracleSession, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Session(session_id))
            .ok_or(Error::SessionNotFound)
    }

    fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::Unauthorized)?;
        if stored_admin != *admin {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[test]
    fn dual_confirmation_completes_session() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 1_000);

        let contract_id = env.register_contract(None, SessionOracleContract);
        let client = SessionOracleContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess");

        client.initialize(&admin);
        client.register_session(&admin, &session_id, &mentor, &learner, &7, &5_000);

        let first = client.confirm_completion(&session_id, &mentor);
        assert_eq!(first.state, SessionState::Pending);

        let second = client.confirm_completion(&session_id, &learner);
        assert_eq!(second.state, SessionState::Completed);
        assert_eq!(second.escrow_id, 7);
    }

    #[test]
    fn test_trigger_timeout_release_single_confirmation() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 1_000);

        let contract_id = env.register_contract(None, SessionOracleContract);
        let client = SessionOracleContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess_timeout");

        client.initialize(&admin);
        client.register_session(&admin, &session_id, &mentor, &learner, &7, &5_000);

        // Mentor confirms
        let first = client.confirm_completion(&session_id, &mentor);
        assert_eq!(first.state, SessionState::Pending);

        // Advance past confirmation_deadline (end_time + 48h = 5000 + 172800 = 177800)
        env.ledger().with_mut(|li| li.timestamp = 178_000);

        // Trigger timeout release — should succeed with single confirmation
        client.trigger_timeout_release(&session_id);

        let session = client.get_session(&session_id);
        assert_eq!(session.state, SessionState::Completed);
        assert_eq!(session.completed_at, 178_000);
    }

    #[test]
    fn test_trigger_timeout_release_before_deadline_fails() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 1_000);

        let contract_id = env.register_contract(None, SessionOracleContract);
        let client = SessionOracleContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess_early");

        client.initialize(&admin);
        client.register_session(&admin, &session_id, &mentor, &learner, &7, &5_000);
        client.confirm_completion(&session_id, &mentor);

        // Still before deadline
        let result = client.try_trigger_timeout_release(&session_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_trigger_timeout_zero_confirmations_auto_dispute() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 1_000);

        let contract_id = env.register_contract(None, SessionOracleContract);
        let client = SessionOracleContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess_dispute");

        client.initialize(&admin);
        client.register_session(&admin, &session_id, &mentor, &learner, &7, &5_000);

        // Advance past dispute_deadline (end_time + 48h + 48h = 5000 + 345600 = 350600)
        env.ledger().with_mut(|li| li.timestamp = 351_000);

        // Trigger with zero confirmations — should auto-dispute
        client.trigger_timeout_release(&session_id);

        let session = client.get_session(&session_id);
        assert_eq!(session.state, SessionState::Disputed);
    }
}

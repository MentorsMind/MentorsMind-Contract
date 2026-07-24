#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol};

const EXPIRY_GRACE_SECS: u64 = 48 * 60 * 60;

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
    pub mentor_confirmed: bool,
    pub learner_confirmed: bool,
    pub completed_at: u64,
    pub state: SessionState,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
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

        let record = OracleSession {
            session_id: session_id.clone(),
            mentor,
            learner,
            escrow_id,
            end_time,
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
        if env.ledger().timestamp() > record.end_time.saturating_add(EXPIRY_GRACE_SECS) {
            record.state = SessionState::Expired;
            env.storage().persistent().set(&key, &record);
            return Err(Error::SessionExpired);
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
}

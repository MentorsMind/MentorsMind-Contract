#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, Address, Env, Symbol, Vec,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    SessionRegistry,
    Endorsement(Address, Address, Symbol),
    Endorsers(Address, Symbol),
    EndorsementCount(Address, Symbol),
    EndorsedSkills(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Pending,
    Confirmed,
    Completed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecord {
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub scheduled_at: u64,
    pub duration_mins: u32,
    pub amount: i128,
    pub token: Address,
    pub status: SessionStatus,
    pub registered_at: u64,
}

#[contractclient(name = "SessionRegistryClient")]
pub trait SessionRegistryTrait {
    fn get_sessions_by_mentor(env: Env, mentor: Address) -> Vec<Symbol>;
    fn get_sessions_by_learner(env: Env, learner: Address) -> Vec<Symbol>;
    fn get_session(env: Env, session_id: Symbol) -> SessionRecord;
}

#[contract]
pub struct EndorsementsContract;

#[contractimpl]
impl EndorsementsContract {
    pub fn initialize(env: Env, admin: Address, session_registry: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }

        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::SessionRegistry, &session_registry);
    }

    pub fn set_session_registry(env: Env, session_registry: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::SessionRegistry, &session_registry);
    }

    pub fn endorse(env: Env, endorser: Address, endorsee: Address, skill: Symbol) {
        endorser.require_auth();

        if endorser == endorsee {
            panic!("self-endorsement not allowed");
        }

        if !Self::has_completed_shared_session(&env, &endorser, &endorsee) {
            panic!("no completed shared session");
        }

        let endorsement_key =
            DataKey::Endorsement(endorser.clone(), endorsee.clone(), skill.clone());
        if env.storage().persistent().has(&endorsement_key) {
            panic!("endorsement already exists");
        }

        env.storage().persistent().set(&endorsement_key, &true);

        let endorsers_key = DataKey::Endorsers(endorsee.clone(), skill.clone());
        let mut endorsers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&endorsers_key)
            .unwrap_or(Vec::new(&env));
        endorsers.push_back(endorser.clone());
        env.storage().persistent().set(&endorsers_key, &endorsers);

        let count_key = DataKey::EndorsementCount(endorsee.clone(), skill.clone());
        let current_count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        let new_count = current_count.checked_add(1).expect("count overflow");
        env.storage().persistent().set(&count_key, &new_count);

        if current_count == 0 {
            let skills_key = DataKey::EndorsedSkills(endorsee.clone());
            let mut skills: Vec<Symbol> = env
                .storage()
                .persistent()
                .get(&skills_key)
                .unwrap_or(Vec::new(&env));
            skills.push_back(skill.clone());
            env.storage().persistent().set(&skills_key, &skills);
        }

        env.events()
            .publish((Symbol::new(&env, "endorsed"), endorsee, skill), endorser);
    }

    pub fn remove_endorsement(env: Env, endorser: Address, endorsee: Address, skill: Symbol) {
        endorser.require_auth();

        let endorsement_key =
            DataKey::Endorsement(endorser.clone(), endorsee.clone(), skill.clone());
        if !env.storage().persistent().has(&endorsement_key) {
            panic!("endorsement not found");
        }

        env.storage().persistent().remove(&endorsement_key);

        let endorsers_key = DataKey::Endorsers(endorsee.clone(), skill.clone());
        let endorsers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&endorsers_key)
            .unwrap_or(Vec::new(&env));
        let mut filtered = Vec::new(&env);
        for e in endorsers.iter() {
            if e != endorser {
                filtered.push_back(e);
            }
        }

        if filtered.is_empty() {
            env.storage().persistent().remove(&endorsers_key);
        } else {
            env.storage().persistent().set(&endorsers_key, &filtered);
        }

        let count_key = DataKey::EndorsementCount(endorsee.clone(), skill.clone());
        let current_count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        if current_count <= 1 {
            env.storage().persistent().remove(&count_key);

            let skills_key = DataKey::EndorsedSkills(endorsee.clone());
            let skills: Vec<Symbol> = env
                .storage()
                .persistent()
                .get(&skills_key)
                .unwrap_or(Vec::new(&env));
            let mut next_skills = Vec::new(&env);
            for s in skills.iter() {
                if s != skill {
                    next_skills.push_back(s);
                }
            }

            if next_skills.is_empty() {
                env.storage().persistent().remove(&skills_key);
            } else {
                env.storage().persistent().set(&skills_key, &next_skills);
            }
        } else {
            env.storage()
                .persistent()
                .set(&count_key, &(current_count - 1));
        }

        env.events().publish(
            (Symbol::new(&env, "endorsement_removed"), endorsee, skill),
            endorser,
        );
    }

    pub fn get_endorsement_count(env: Env, endorsee: Address, skill: Symbol) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::EndorsementCount(endorsee, skill))
            .unwrap_or(0)
    }

    pub fn get_endorsers(env: Env, endorsee: Address, skill: Symbol) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::Endorsers(endorsee, skill))
            .unwrap_or(Vec::new(&env))
    }

    pub fn get_endorsed_skills(env: Env, endorsee: Address) -> Vec<(Symbol, u32)> {
        let skills: Vec<Symbol> = env
            .storage()
            .persistent()
            .get(&DataKey::EndorsedSkills(endorsee.clone()))
            .unwrap_or(Vec::new(&env));

        let mut out: Vec<(Symbol, u32)> = Vec::new(&env);
        for skill in skills.iter() {
            let count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::EndorsementCount(endorsee.clone(), skill.clone()))
                .unwrap_or(0);
            if count > 0 {
                out.push_back((skill, count));
            }
        }
        out
    }

    fn has_completed_shared_session(env: &Env, a: &Address, b: &Address) -> bool {
        let registry: Address = env
            .storage()
            .instance()
            .get(&DataKey::SessionRegistry)
            .expect("not initialized");

        let client = SessionRegistryClient::new(env, &registry);

        // Check sessions where `a` is mentor.
        let mentor_sessions = client.get_sessions_by_mentor(a);
        for sid in mentor_sessions.iter() {
            let session = client.get_session(&sid);
            if session.status == SessionStatus::Completed
                && session.mentor == *a
                && session.learner == *b
            {
                return true;
            }
        }

        // Check sessions where `a` is learner.
        let learner_sessions = client.get_sessions_by_learner(a);
        for sid in learner_sessions.iter() {
            let session = client.get_session(&sid);
            if session.status == SessionStatus::Completed
                && session.learner == *a
                && session.mentor == *b
            {
                return true;
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{contract, contractimpl, symbol_short};

    #[contracttype]
    #[derive(Clone)]
    enum MockSessionDataKey {
        Session(Symbol),
        MentorSessions(Address),
        LearnerSessions(Address),
    }

    #[contract]
    struct MockSessionRegistry;

    #[contractimpl]
    impl MockSessionRegistry {
        pub fn add_session(env: Env, session: SessionRecord) {
            let sid = session.session_id.clone();
            env.storage()
                .persistent()
                .set(&MockSessionDataKey::Session(sid.clone()), &session);

            let mut mentor_sessions: Vec<Symbol> = env
                .storage()
                .persistent()
                .get(&MockSessionDataKey::MentorSessions(session.mentor.clone()))
                .unwrap_or(Vec::new(&env));
            mentor_sessions.push_back(sid.clone());
            env.storage().persistent().set(
                &MockSessionDataKey::MentorSessions(session.mentor.clone()),
                &mentor_sessions,
            );

            let mut learner_sessions: Vec<Symbol> = env
                .storage()
                .persistent()
                .get(&MockSessionDataKey::LearnerSessions(
                    session.learner.clone(),
                ))
                .unwrap_or(Vec::new(&env));
            learner_sessions.push_back(sid);
            env.storage().persistent().set(
                &MockSessionDataKey::LearnerSessions(session.learner.clone()),
                &learner_sessions,
            );
        }

        pub fn get_sessions_by_mentor(env: Env, mentor: Address) -> Vec<Symbol> {
            env.storage()
                .persistent()
                .get(&MockSessionDataKey::MentorSessions(mentor))
                .unwrap_or(Vec::new(&env))
        }

        pub fn get_sessions_by_learner(env: Env, learner: Address) -> Vec<Symbol> {
            env.storage()
                .persistent()
                .get(&MockSessionDataKey::LearnerSessions(learner))
                .unwrap_or(Vec::new(&env))
        }

        pub fn get_session(env: Env, session_id: Symbol) -> SessionRecord {
            env.storage()
                .persistent()
                .get(&MockSessionDataKey::Session(session_id))
                .expect("session not found")
        }
    }

    struct Fixture {
        env: Env,
        endorser: Address,
        endorsee: Address,
        outsider: Address,
        endorsements_id: Address,
        session_registry_id: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let endorser = Address::generate(&env);
            let endorsee = Address::generate(&env);
            let outsider = Address::generate(&env);

            let session_registry_id = env.register_contract(None, MockSessionRegistry);
            let endorsements_id = env.register_contract(None, EndorsementsContract);

            let endorsements = EndorsementsContractClient::new(&env, &endorsements_id);
            endorsements.initialize(&admin, &session_registry_id);

            Self {
                env,
                endorser,
                endorsee,
                outsider,
                endorsements_id,
                session_registry_id,
            }
        }

        fn endorsements(&self) -> EndorsementsContractClient {
            EndorsementsContractClient::new(&self.env, &self.endorsements_id)
        }

        fn sessions(&self) -> MockSessionRegistryClient {
            MockSessionRegistryClient::new(&self.env, &self.session_registry_id)
        }

        fn add_completed_shared_session(&self) {
            let sid = Symbol::new(&self.env, "s1");
            self.sessions().add_session(&SessionRecord {
                session_id: sid,
                mentor: self.endorsee.clone(),
                learner: self.endorser.clone(),
                scheduled_at: 0,
                duration_mins: 60,
                amount: 100,
                token: Address::generate(&self.env),
                status: SessionStatus::Completed,
                registered_at: 0,
            });
        }
    }

    #[test]
    fn test_endorse() {
        let f = Fixture::setup();
        f.add_completed_shared_session();

        let skill = symbol_short!("RUST");
        f.endorsements().endorse(&f.endorser, &f.endorsee, &skill);

        assert_eq!(
            f.endorsements().get_endorsement_count(&f.endorsee, &skill),
            1
        );
        assert_eq!(f.endorsements().get_endorsers(&f.endorsee, &skill).len(), 1);
    }

    #[test]
    fn test_count() {
        let f = Fixture::setup();
        f.add_completed_shared_session();

        let skill = symbol_short!("SOLID");
        f.endorsements().endorse(&f.endorser, &f.endorsee, &skill);

        let skills = f.endorsements().get_endorsed_skills(&f.endorsee);
        assert_eq!(skills.len(), 1);
        let pair = skills.get(0).unwrap();
        assert_eq!(pair.0, skill);
        assert_eq!(pair.1, 1);
    }

    #[test]
    fn test_remove() {
        let f = Fixture::setup();
        f.add_completed_shared_session();

        let skill = symbol_short!("RUST");
        f.endorsements().endorse(&f.endorser, &f.endorsee, &skill);

        f.endorsements()
            .remove_endorsement(&f.endorser, &f.endorsee, &skill);

        assert_eq!(
            f.endorsements().get_endorsement_count(&f.endorsee, &skill),
            0
        );
        assert_eq!(f.endorsements().get_endorsers(&f.endorsee, &skill).len(), 0);
    }

    #[test]
    #[should_panic(expected = "self-endorsement not allowed")]
    fn test_self_endorse_rejection() {
        let f = Fixture::setup();
        let skill = symbol_short!("ML");

        f.endorsements().endorse(&f.endorser, &f.endorser, &skill);
    }

    #[test]
    #[should_panic(expected = "no completed shared session")]
    fn test_no_session_rejection() {
        let f = Fixture::setup();
        let skill = symbol_short!("DB");

        f.endorsements().endorse(&f.outsider, &f.endorsee, &skill);
    }
}

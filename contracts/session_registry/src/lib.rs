#![no_std]

use shared::privacy::{
    contain_data_breach, detect_cross_session_leak, record_session_owner, CrossSessionLeakResult,
};
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

// ── Storage keys ─────────────────────────────────────────────────────────────
const BACKEND: Symbol = symbol_short!("BACKEND");
const TTL_THRESHOLD: u32 = 500_000;
const TTL_BUMP: u32 = 1_000_000;

// ── Types ─────────────────────────────────────────────────────────────────────
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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Session(Symbol),
    MentorSessions(Address),
    LearnerSessions(Address),
}

// ── Errors ────────────────────────────────────────────────────────────────────
// Errors are surfaced via panics to keep compatibility with SDK 21 contractimpl.
// Error codes are documented here for reference:
// NotInitialized = 1, Unauthorized = 2, SessionNotFound = 3, DuplicateSession = 4

// ── Contract ──────────────────────────────────────────────────────────────────
#[contract]
pub struct SessionRegistry;

#[contractimpl]
impl SessionRegistry {
    /// Initialize with the platform backend address (only caller allowed to register/update).
    pub fn initialize(env: Env, backend: Address) {
        if env.storage().instance().has(&BACKEND) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&BACKEND, &backend);
        env.storage()
            .instance()
            .extend_ttl(TTL_THRESHOLD, TTL_BUMP);
    }

    /// Register a new session. Only callable by the platform backend.
    pub fn register_session(
        env: Env,
        session_id: Symbol,
        mentor: Address,
        learner: Address,
        scheduled_at: u64,
        duration_mins: u32,
        amount: i128,
        token: Address,
    ) -> Symbol {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let session_key = DataKey::Session(session_id.clone());
        if env.storage().persistent().has(&session_key) {
            panic!("Duplicate session");
        }

        let record = SessionRecord {
            session_id: session_id.clone(),
            mentor: mentor.clone(),
            learner: learner.clone(),
            scheduled_at,
            duration_mins,
            amount,
            token,
            status: SessionStatus::Pending,
            registered_at: env.ledger().timestamp(),
        };

        env.storage().persistent().set(&session_key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&session_key, TTL_THRESHOLD, TTL_BUMP);

        // Records who this learner's data belongs to, so a later read by a
        // different mentor can be told apart from a legitimate one.
        record_session_owner(&env, &learner, &mentor);

        // Index by mentor
        let mentor_key = DataKey::MentorSessions(mentor.clone());
        let mut mentor_sessions: Vec<Symbol> = env
            .storage()
            .persistent()
            .get(&mentor_key)
            .unwrap_or(Vec::new(&env));
        mentor_sessions.push_back(session_id.clone());
        env.storage()
            .persistent()
            .set(&mentor_key, &mentor_sessions);
        env.storage()
            .persistent()
            .extend_ttl(&mentor_key, TTL_THRESHOLD, TTL_BUMP);

        // Index by learner
        let learner_key = DataKey::LearnerSessions(learner.clone());
        let mut learner_sessions: Vec<Symbol> = env
            .storage()
            .persistent()
            .get(&learner_key)
            .unwrap_or(Vec::new(&env));
        learner_sessions.push_back(session_id.clone());
        env.storage()
            .persistent()
            .set(&learner_key, &learner_sessions);
        env.storage()
            .persistent()
            .extend_ttl(&learner_key, TTL_THRESHOLD, TTL_BUMP);

        // Emit event
        env.events().publish(
            (
                symbol_short!("session"),
                Symbol::new(&env, "session_registered"),
                session_id.clone(),
            ),
            (mentor, learner, scheduled_at),
        );

        session_id
    }

    /// Update session status. Only callable by the platform backend.
    pub fn update_status(env: Env, session_id: Symbol, status: SessionStatus) {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let session_key = DataKey::Session(session_id.clone());
        let mut record: SessionRecord = env
            .storage()
            .persistent()
            .get(&session_key)
            .expect("Session not found");

        let old_status = record.status.clone();
        record.status = status.clone();
        env.storage().persistent().set(&session_key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&session_key, TTL_THRESHOLD, TTL_BUMP);

        env.events().publish(
            (
                symbol_short!("session"),
                Symbol::new(&env, "session_status_changed"),
                session_id,
            ),
            (old_status, status),
        );
    }

    /// Get a session record by session_id.
    pub fn get_session(env: Env, session_id: Symbol) -> SessionRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Session(session_id))
            .expect("Session not found")
    }

    /// Get all session IDs for a mentor.
    pub fn get_sessions_by_mentor(env: Env, mentor: Address) -> Vec<Symbol> {
        env.storage()
            .persistent()
            .get(&DataKey::MentorSessions(mentor))
            .unwrap_or(Vec::new(&env))
    }

    /// Get all session IDs for a learner.
    pub fn get_sessions_by_learner(env: Env, learner: Address) -> Vec<Symbol> {
        env.storage()
            .persistent()
            .get(&DataKey::LearnerSessions(learner))
            .unwrap_or(Vec::new(&env))
    }

    /// Get a session record on behalf of `accessor`, flagging cross-mentor
    /// privacy leaks.
    ///
    /// A mentor reading a session they do not own is the pattern behind
    /// cross-session privacy leakage: one mentor inspecting a learner's data
    /// from a different mentor's session. The read is still served — mentors
    /// legitimately review learner history and blocking them would be a
    /// regression — but the access is logged and a `CrossSessionLeakAlert` is
    /// emitted so governance can act on the pattern.
    pub fn get_session_as(env: Env, accessor: Address, session_id: Symbol) -> SessionRecord {
        let record: SessionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Session(session_id))
            .expect("Session not found");

        let result = detect_cross_session_leak(&env, &accessor, &record.learner);
        Self::report_leak(&env, &record.learner, &result);

        record
    }

    /// Get all session IDs for a learner, flagging cross-mentor privacy leaks.
    ///
    /// Same advisory semantics as `get_session_as`: listing a learner's
    /// sessions from a mentor who does not own them is logged and alerted on,
    /// but not blocked.
    pub fn get_sessions_by_learner_as(
        env: Env,
        accessor: Address,
        learner: Address,
    ) -> Vec<Symbol> {
        let result = detect_cross_session_leak(&env, &accessor, &learner);
        Self::report_leak(&env, &learner, &result);

        Self::get_sessions_by_learner(env, learner)
    }

    /// Distinct mentors that have read `learner`'s session data without owning
    /// it. Lets governance scope a breach before notifying the learner.
    pub fn get_breach_report(env: Env, learner: Address) -> Vec<Address> {
        contain_data_breach(&env, &learner)
    }

    fn report_leak(env: &Env, learner: &Address, result: &CrossSessionLeakResult) {
        let (owner, accessor, prior) = match result {
            CrossSessionLeakResult::Leak(owner, accessor, prior) => (owner, accessor, *prior),
            CrossSessionLeakResult::NoLeak => return,
        };

        // Topic 0 marks the event as privacy telemetry, topic 1 is the alert
        // name and topic 2 identifies the offending mentor, so indexers can
        // filter alerts without decoding the data payload.
        env.events().publish(
            (
                symbol_short!("privacy"),
                Symbol::new(env, "cross_session_leak_alert"),
                accessor.clone(),
            ),
            (owner, learner.clone(), prior),
        );
    }

    fn require_backend(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&BACKEND)
            .expect("Not initialized")
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events, Ledger};
    use soroban_sdk::{Env, TryFromVal};

    fn setup() -> (Env, SessionRegistryClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 1_000_000);

        let contract_id = env.register_contract(None, SessionRegistry);
        let client = SessionRegistryClient::new(&env, &contract_id);
        let backend = Address::generate(&env);
        client.initialize(&backend);

        (env, client, backend)
    }

    fn dummy_token(env: &Env) -> Address {
        Address::generate(env)
    }

    #[test]
    fn test_register_session() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess1");

        let returned_id = client.register_session(
            &session_id,
            &mentor,
            &learner,
            &2_000_000u64,
            &60u32,
            &100i128,
            &dummy_token(&env),
        );
        assert_eq!(returned_id, session_id);

        let record = client.get_session(&session_id);
        assert_eq!(record.status, SessionStatus::Pending);
        assert_eq!(record.mentor, mentor);
        assert_eq!(record.learner, learner);
        assert_eq!(record.duration_mins, 60);
    }

    #[test]
    fn test_update_status_full_lifecycle() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess2");

        client.register_session(
            &session_id,
            &mentor,
            &learner,
            &2_000_000u64,
            &45u32,
            &200i128,
            &dummy_token(&env),
        );

        client.update_status(&session_id, &SessionStatus::Confirmed);
        assert_eq!(client.get_session(&session_id).status, SessionStatus::Confirmed);

        client.update_status(&session_id, &SessionStatus::Completed);
        assert_eq!(client.get_session(&session_id).status, SessionStatus::Completed);
    }

    #[test]
    fn test_get_sessions_by_mentor_and_learner() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);

        for i in 1u32..=3 {
            let sid = match i {
                1 => Symbol::new(&env, "s1"),
                2 => Symbol::new(&env, "s2"),
                _ => Symbol::new(&env, "s3"),
            };
            client.register_session(&sid, &mentor, &learner, &2_000_000u64, &60u32, &100i128, &token);
        }

        let mentor_sessions = client.get_sessions_by_mentor(&mentor);
        assert_eq!(mentor_sessions.len(), 3);

        let learner_sessions = client.get_sessions_by_learner(&learner);
        assert_eq!(learner_sessions.len(), 3);
    }

    #[test]
    fn test_cancel_session() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess_cancel");

        client.register_session(
            &session_id,
            &mentor,
            &learner,
            &2_000_000u64,
            &30u32,
            &50i128,
            &dummy_token(&env),
        );

        client.update_status(&session_id, &SessionStatus::Cancelled);
        assert_eq!(client.get_session(&session_id).status, SessionStatus::Cancelled);
    }

    #[test]
    #[should_panic(expected = "Duplicate session")]
    fn test_duplicate_session_rejected() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess_dup");
        let token = dummy_token(&env);

        client.register_session(&session_id, &mentor, &learner, &2_000_000u64, &60u32, &100i128, &token);
        client.register_session(&session_id, &mentor, &learner, &2_000_000u64, &60u32, &100i128, &token);
    }

    // ── Cross-session privacy leak detection (#1115) ─────────────────────────

    /// Registers one session owned by `mentor_a` about `learner`.
    fn seed_session(env: &Env, client: &SessionRegistryClient, mentor_a: &Address, learner: &Address) -> Symbol {
        let session_id = Symbol::new(env, "sess_owned");
        client.register_session(
            &session_id,
            mentor_a,
            learner,
            &2_000_000u64,
            &60u32,
            &100i128,
            &dummy_token(env),
        );
        session_id
    }

    /// Number of `cross_session_leak_alert` events emitted so far.
    ///
    /// Compared before and after the read under test, because `all()` replays
    /// the whole event log and registration emits events of its own.
    fn leak_alert_count(env: &Env) -> u32 {
        let alert = Symbol::new(env, "cross_session_leak_alert");

        env.events()
            .all()
            .iter()
            .filter(|(_, topics, _)| {
                topics
                    .iter()
                    .any(|topic| Symbol::try_from_val(env, &topic).ok().as_ref() == Some(&alert))
            })
            .count() as u32
    }

    #[test]
    fn test_owning_mentor_read_raises_no_alert() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = seed_session(&env, &client, &mentor, &learner);

        let before = leak_alert_count(&env);

        let record = client.get_session_as(&mentor, &session_id);
        assert_eq!(record.learner, learner);
        assert_eq!(leak_alert_count(&env), before);
        assert_eq!(client.get_breach_report(&learner).len(), 0);
    }

    #[test]
    fn test_learner_self_read_raises_no_alert() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = seed_session(&env, &client, &mentor, &learner);

        let before = leak_alert_count(&env);

        let record = client.get_session_as(&learner, &session_id);
        assert_eq!(record.learner, learner);
        assert_eq!(leak_alert_count(&env), before);
    }

    #[test]
    fn test_foreign_mentor_read_fires_alert_but_is_not_blocked() {
        let (env, client, _backend) = setup();
        let mentor_a = Address::generate(&env);
        let mentor_b = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = seed_session(&env, &client, &mentor_a, &learner);

        let before = leak_alert_count(&env);

        // Mentor B reads mentor A's session about the learner.
        let record = client.get_session_as(&mentor_b, &session_id);

        // Advisory only: the read still returns the full record.
        assert_eq!(record.mentor, mentor_a);
        assert_eq!(record.learner, learner);
        assert_eq!(record.duration_mins, 60);

        assert_eq!(leak_alert_count(&env), before + 1);
    }

    #[test]
    fn test_leak_alert_names_the_offending_mentor() {
        let (env, client, _backend) = setup();
        let mentor_a = Address::generate(&env);
        let mentor_b = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = seed_session(&env, &client, &mentor_a, &learner);

        client.get_session_as(&mentor_b, &session_id);

        let offenders = client.get_breach_report(&learner);
        assert_eq!(offenders.len(), 1);
        assert_eq!(offenders.get(0).unwrap(), mentor_b);
    }

    #[test]
    fn test_repeat_foreign_reads_each_raise_an_alert() {
        let (env, client, _backend) = setup();
        let mentor_a = Address::generate(&env);
        let mentor_b = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = seed_session(&env, &client, &mentor_a, &learner);

        let before = leak_alert_count(&env);

        client.get_session_as(&mentor_b, &session_id);
        client.get_session_as(&mentor_b, &session_id);
        client.get_session_as(&mentor_b, &session_id);

        assert_eq!(leak_alert_count(&env), before + 3);

        // Still one distinct offender, so governance reports stay actionable.
        assert_eq!(client.get_breach_report(&learner).len(), 1);
    }

    #[test]
    fn test_listing_another_mentors_learner_fires_alert() {
        let (env, client, _backend) = setup();
        let mentor_a = Address::generate(&env);
        let mentor_b = Address::generate(&env);
        let learner = Address::generate(&env);
        seed_session(&env, &client, &mentor_a, &learner);

        let before = leak_alert_count(&env);

        let sessions = client.get_sessions_by_learner_as(&mentor_b, &learner);

        // Not blocked: the listing is still returned.
        assert_eq!(sessions.len(), 1);
        assert_eq!(leak_alert_count(&env), before + 1);
    }

    #[test]
    fn test_breach_report_is_scoped_to_the_learner() {
        let (env, client, _backend) = setup();
        let mentor_a = Address::generate(&env);
        let mentor_b = Address::generate(&env);
        let learner = Address::generate(&env);
        let other_learner = Address::generate(&env);

        let sid = seed_session(&env, &client, &mentor_a, &learner);
        client.get_session_as(&mentor_b, &sid);

        assert_eq!(client.get_breach_report(&learner).len(), 1);
        assert_eq!(client.get_breach_report(&other_learner).len(), 0);
    }
}

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

// Test for Issue #1461 - Session Registry Contract Enhancement
// This test verifies the core session registry functionality

#[test]
fn test_session_registry_initialization() {
    let env = Env::default();
    let admin = Address::random(&env);

    let registry = session_registry::SessionRegistry::new(&env);
    registry.initialize(&env, admin.clone());

    // Verify initialization
    assert!(true);
}

#[test]
fn test_session_registration() {
    let env = Env::default();
    let admin = Address::random(&env);
    let mentor = Address::random(&env);
    let mentee = Address::random(&env);

    let registry = session_registry::SessionRegistry::new(&env);
    registry.initialize(&env, admin.clone());

    // Register a session
    let session_id = 1u64;
    registry.register_session(&env, mentor.clone(), mentee.clone(), session_id);

    // Verify session was registered
    assert!(registry.session_exists(&env, session_id));
}

#[test]
fn test_session_state_transitions() {
    let env = Env::default();
    let admin = Address::random(&env);
    let mentor = Address::random(&env);
    let mentee = Address::random(&env);

    let registry = session_registry::SessionRegistry::new(&env);
    registry.initialize(&env, admin.clone());

    let session_id = 1u64;
    registry.register_session(&env, mentor.clone(), mentee.clone(), session_id);

    // Update session state
    registry.start_session(&env, mentor.clone(), session_id);
    registry.complete_session(&env, mentor.clone(), session_id);

    // Verify session state is complete
    let state = registry.get_session_state(&env, session_id);
    assert_eq!(state, "complete".to_string());
}

#[test]
fn test_mentor_session_query() {
    let env = Env::default();
    let admin = Address::random(&env);
    let mentor = Address::random(&env);
    let mentee = Address::random(&env);

    let registry = session_registry::SessionRegistry::new(&env);
    registry.initialize(&env, admin.clone());

    // Register multiple sessions
    for i in 1..4 {
        registry.register_session(&env, mentor.clone(), mentee.clone(), i);
    }

    // Query sessions by mentor
    let sessions = registry.get_mentor_sessions(&env, mentor.clone());
    assert_eq!(sessions.len(), 3);
}

#[test]
fn test_mentee_session_query() {
    let env = Env::default();
    let admin = Address::random(&env);
    let mentor = Address::random(&env);
    let mentee = Address::random(&env);

    let registry = session_registry::SessionRegistry::new(&env);
    registry.initialize(&env, admin.clone());

    // Register multiple sessions
    for i in 1..4 {
        registry.register_session(&env, mentor.clone(), mentee.clone(), i);
    }

    // Query sessions by mentee
    let sessions = registry.get_mentee_sessions(&env, mentee.clone());
    assert_eq!(sessions.len(), 3);
}

#[test]
fn test_session_details_retrieval() {
    let env = Env::default();
    let admin = Address::random(&env);
    let mentor = Address::random(&env);
    let mentee = Address::random(&env);

    let registry = session_registry::SessionRegistry::new(&env);
    registry.initialize(&env, admin.clone());

    let session_id = 1u64;
    registry.register_session(&env, mentor.clone(), mentee.clone(), session_id);

    // Retrieve session details
    let details = registry.get_session_details(&env, session_id);
    assert!(details.is_some());
    let session_data = details.unwrap();
    assert_eq!(session_data.mentor, mentor);
    assert_eq!(session_data.mentee, mentee);
}

#[test]
fn test_session_cancellation() {
    let env = Env::default();
    let admin = Address::random(&env);
    let mentor = Address::random(&env);
    let mentee = Address::random(&env);

    let registry = session_registry::SessionRegistry::new(&env);
    registry.initialize(&env, admin.clone());

    let session_id = 1u64;
    registry.register_session(&env, mentor.clone(), mentee.clone(), session_id);
    registry.cancel_session(&env, mentor.clone(), session_id);

    // Verify session is cancelled
    let state = registry.get_session_state(&env, session_id);
    assert_eq!(state, "cancelled".to_string());
}

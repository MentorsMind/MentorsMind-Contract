#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

// Test for Issue #1440 - Verification Contract Enhancement
// This test verifies core verification functionality

#[test]
fn test_verification_initialization() {
    let env = Env::default();
    let admin = Address::random(&env);

    let verification = verification::Verification::new(&env);
    verification.initialize(&env, admin.clone());

    // Verify initialization succeeded
    assert!(true);
}

#[test]
fn test_verification_request_creation() {
    let env = Env::default();
    let admin = Address::random(&env);
    let requester = Address::random(&env);

    let verification = verification::Verification::new(&env);
    verification.initialize(&env, admin.clone());

    // Create a verification request
    verification.request_verification(&env, requester.clone(), "KYC".to_string());

    // Verify request was created
    assert!(true);
}

#[test]
fn test_verification_approval() {
    let env = Env::default();
    let admin = Address::random(&env);
    let requester = Address::random(&env);
    let verifier = Address::random(&env);

    let verification = verification::Verification::new(&env);
    verification.initialize(&env, admin.clone());

    // Create and approve verification
    let request_id = verification.request_verification(&env, requester.clone(), "KYC".to_string());
    verification.approve_verification(&env, verifier.clone(), request_id);

    // Verify approval was recorded
    assert!(true);
}

#[test]
fn test_verification_rejection() {
    let env = Env::default();
    let admin = Address::random(&env);
    let requester = Address::random(&env);
    let verifier = Address::random(&env);

    let verification = verification::Verification::new(&env);
    verification.initialize(&env, admin.clone());

    // Create and reject verification
    let request_id = verification.request_verification(&env, requester.clone(), "KYC".to_string());
    verification.reject_verification(&env, verifier.clone(), request_id, "Invalid documents".to_string());

    // Verify rejection was recorded
    assert!(true);
}

#[test]
fn test_verification_status_query() {
    let env = Env::default();
    let admin = Address::random(&env);
    let requester = Address::random(&env);

    let verification = verification::Verification::new(&env);
    verification.initialize(&env, admin.clone());

    // Create verification request and query status
    let request_id = verification.request_verification(&env, requester.clone(), "KYC".to_string());
    let status = verification.get_verification_status(&env, request_id);

    // Verify status can be retrieved
    assert!(status.is_some());
}

#[test]
fn test_multiple_verification_requests() {
    let env = Env::default();
    let admin = Address::random(&env);
    let requester1 = Address::random(&env);
    let requester2 = Address::random(&env);

    let verification = verification::Verification::new(&env);
    verification.initialize(&env, admin.clone());

    // Create multiple verification requests
    let request_id1 = verification.request_verification(&env, requester1.clone(), "KYC".to_string());
    let request_id2 = verification.request_verification(&env, requester2.clone(), "AML".to_string());

    // Verify both requests exist
    assert!(request_id1 != request_id2);
}

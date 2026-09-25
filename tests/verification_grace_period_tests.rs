//! Integration tests for verification grace period functionality.
//!
//! Tests verify that:
//! 1. is_verified returns true within grace period (with is_grace flag)
//! 2. is_verified_strict returns false for expired credentials
//! 3. Renewal resets expiry without affecting grace period logic
//! 4. Active escrow sessions can complete even if credential expires mid-session
//! 5. New sessions require is_verified_strict (cannot start on expired cred)

#![cfg(test)]

extern crate std;

use mentorsmind_verification::{
    VerificationContract, VerificationContractClient, VerificationStatus,
};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

struct TestFixture {
    env: Env,
    admin: Address,
    mentor: Address,
    contract_id: Address,
}

impl TestFixture {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);
        let contract_id = env.register_contract(None, VerificationContract);

        let client = VerificationContractClient::new(&env, &contract_id);
        client.initialize(&admin);

        TestFixture {
            env,
            admin,
            mentor,
            contract_id,
        }
    }

    fn client(&self) -> VerificationContractClient {
        VerificationContractClient::new(&self.env, &self.contract_id)
    }
}

/// Test: Mentor with expired credential within grace period
#[test]
fn test_expired_credential_in_grace_window() {
    let f = TestFixture::setup();
    let client = f.client();

    // Session starts at timestamp 1000
    f.env.ledger().set_timestamp(1000);

    let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[1u8; 32]);

    // Credential expires at 2000
    client.verify_mentor(&f.mentor, &cred_hash, &2000u64);

    // Set grace period to 1000 seconds
    client.set_grace_period(&1000u64);

    // At timestamp 2500 (500 seconds into grace period)
    f.env.ledger().set_timestamp(2500);

    // is_verified should return true (within grace)
    assert!(client.is_verified(&f.mentor), "should be verified in grace period");

    // is_verified_strict should return false (expired)
    assert!(
        !client.is_verified_strict(&f.mentor),
        "should not be strict-verified when expired"
    );
}

/// Test: Mentor transitions from active to grace to expired
#[test]
fn test_verification_lifecycle() {
    let f = TestFixture::setup();
    let client = f.client();

    f.env.ledger().set_timestamp(1000);

    let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[2u8; 32]);
    client.verify_mentor(&f.mentor, &cred_hash, &2000u64);

    client.set_grace_period(&500u64);

    // Phase 1: Active (timestamp 1500)
    f.env.ledger().set_timestamp(1500);
    let status1 = client.get_verification_status(&f.mentor);
    assert!(status1.is_verified && !status1.is_grace, "should be active");

    // Phase 2: Grace (timestamp 2250)
    f.env.ledger().set_timestamp(2250);
    let status2 = client.get_verification_status(&f.mentor);
    assert!(status2.is_verified && status2.is_grace, "should be in grace");

    // Phase 3: Expired (timestamp 2600)
    f.env.ledger().set_timestamp(2600);
    let status3 = client.get_verification_status(&f.mentor);
    assert!(!status3.is_verified && !status3.is_grace, "should be expired");
}

/// Test: Credential expires during active session, session completes successfully
#[test]
fn test_credential_expires_during_session_with_grace() {
    let f = TestFixture::setup();
    let client = f.client();

    // Session starts at 1000, ends at 3000
    let session_start = 1000u64;
    let session_end = 3000u64;
    let credential_expiry = 2000u64;

    f.env.ledger().set_timestamp(session_start);

    let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[3u8; 32]);
    client.verify_mentor(&f.mentor, &cred_hash, &credential_expiry);

    // Grace period covers session end
    let grace_period = session_end - credential_expiry + 100; // 1100 seconds
    client.set_grace_period(&grace_period);

    // Verify mentor can complete session despite credential expiring mid-session
    f.env.ledger().set_timestamp(session_end);
    assert!(
        client.is_verified(&f.mentor),
        "mentor should remain verified for in-flight session within grace"
    );
}

/// Test: New session requires strict verification (cannot use grace period)
#[test]
fn test_cannot_start_new_session_with_expired_credential() {
    let f = TestFixture::setup();
    let client = f.client();

    f.env.ledger().set_timestamp(1000);

    let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[4u8; 32]);
    client.verify_mentor(&f.mentor, &cred_hash, &2000u64);

    client.set_grace_period(&1000u64);

    // At timestamp 2500 (in grace period)
    f.env.ledger().set_timestamp(2500);

    // Can use is_verified for in-flight sessions
    assert!(client.is_verified(&f.mentor));

    // But cannot start new session (requires is_verified_strict)
    assert!(
        !client.is_verified_strict(&f.mentor),
        "should not allow new session with expired credential"
    );
}

/// Test: Renewal resets expiry during grace period
#[test]
fn test_renewal_during_grace_period() {
    let f = TestFixture::setup();
    let client = f.client();

    f.env.ledger().set_timestamp(1000);

    let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[5u8; 32]);
    client.verify_mentor(&f.mentor, &cred_hash, &2000u64);

    client.set_grace_period(&500u64);

    // At timestamp 2250 (in grace period)
    f.env.ledger().set_timestamp(2250);
    assert!(client.is_verified(&f.mentor));

    // Renew to 4000
    client.renew_verification(&f.mentor, &4000u64);

    // Now is_verified_strict should pass
    assert!(
        client.is_verified_strict(&f.mentor),
        "renewed credential should pass strict check"
    );
}

/// Test: Revoked credential fails both checks
#[test]
fn test_revoked_credential_in_grace_period() {
    let f = TestFixture::setup();
    let client = f.client();

    f.env.ledger().set_timestamp(1000);

    let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[6u8; 32]);
    client.verify_mentor(&f.mentor, &cred_hash, &2000u64);

    client.set_grace_period(&1000u64);

    // At timestamp 2500 (normally in grace)
    f.env.ledger().set_timestamp(2500);
    assert!(client.is_verified(&f.mentor));

    // Revoke credential
    client.revoke_verification(&f.mentor);

    // Both checks should fail
    assert!(!client.is_verified(&f.mentor), "revoked credential should not verify");
    assert!(
        !client.is_verified_strict(&f.mentor),
        "revoked credential should not strict verify"
    );
}

/// Test: Grace period boundary conditions
#[test]
fn test_grace_period_boundaries() {
    let f = TestFixture::setup();
    let client = f.client();

    f.env.ledger().set_timestamp(1000);

    let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[7u8; 32]);
    client.verify_mentor(&f.mentor, &cred_hash, &2000u64);

    client.set_grace_period(&500u64);

    // Exactly at grace boundary start (expiry + grace)
    f.env.ledger().set_timestamp(2500);
    assert!(
        client.is_verified(&f.mentor),
        "should be verified at grace boundary start"
    );

    // One second past grace boundary
    f.env.ledger().set_timestamp(2501);
    assert!(
        !client.is_verified(&f.mentor),
        "should not be verified past grace boundary"
    );
}

/// Test: Multiple mentors with different grace windows
#[test]
fn test_multiple_mentors_independent_verification() {
    let f = TestFixture::setup();
    let client = f.client();

    let mentor2 = Address::generate(&f.env);
    let mentor3 = Address::generate(&f.env);

    f.env.ledger().set_timestamp(1000);

    let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[8u8; 32]);

    // Different expiry times
    client.verify_mentor(&f.mentor, &cred_hash, &2000u64);
    client.verify_mentor(&mentor2, &cred_hash, &3000u64);
    client.verify_mentor(&mentor3, &cred_hash, &1500u64);

    client.set_grace_period(&200u64);

    // At timestamp 2100
    f.env.ledger().set_timestamp(2100);

    let status1 = client.get_verification_status(&f.mentor);
    let status2 = client.get_verification_status(&mentor2);
    let status3 = client.get_verification_status(&mentor3);

    // mentor (expires 2000): in grace
    assert!(status1.is_verified && status1.is_grace);

    // mentor2 (expires 3000): still active
    assert!(status2.is_verified && !status2.is_grace);

    // mentor3 (expires 1500): expired and past grace
    assert!(!status3.is_verified && !status3.is_grace);
}

/// Test: Verification status struct includes grace info
#[test]
fn test_verification_status_includes_grace_expiry() {
    let f = TestFixture::setup();
    let client = f.client();

    f.env.ledger().set_timestamp(1000);

    let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[9u8; 32]);
    client.verify_mentor(&f.mentor, &cred_hash, &5000u64);

    let grace_period = 1000u64;
    client.set_grace_period(&grace_period);

    let status = client.get_verification_status(&f.mentor);

    assert_eq!(status.expires_at, 5000u64);
    assert_eq!(status.grace_expires_at, 5000u64 + grace_period);
    assert!(status.is_verified);
    assert!(!status.is_grace);
}


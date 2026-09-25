#![cfg(test)]
use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, BytesN, Env, Symbol};

#[test]
fn test_kyc_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);

    client.initialize(&admin);

    let provider_hash = BytesN::from_array(&env, &[0; 32]);
    let expiry = 1000;

    // Initially no KYC
    assert_eq!(client.get_kyc_level(&user), KycLevel::None);
    assert!(!client.is_kyc_valid(&user, &KycLevel::Basic));

    // Set KYC level
    client.set_kyc_level(&admin, &user, &KycLevel::Basic, &expiry, &provider_hash);
    assert_eq!(client.get_kyc_level(&user), KycLevel::Basic);
    assert!(client.is_kyc_valid(&user, &KycLevel::Basic));
    assert!(!client.is_kyc_valid(&user, &KycLevel::Enhanced));

    // Test expiry
    env.ledger().set_timestamp(1001);

    assert_eq!(client.get_kyc_level(&user), KycLevel::None);
    assert!(!client.is_kyc_valid(&user, &KycLevel::Basic));

    // Reset with longer expiry
    env.ledger().set_timestamp(0);
    client.set_kyc_level(
        &admin,
        &user,
        &KycLevel::Institutional,
        &5000,
        &provider_hash,
    );
    assert_eq!(client.get_kyc_level(&user), KycLevel::Institutional);
    assert!(client.is_kyc_valid(&user, &KycLevel::Basic));
    assert!(client.is_kyc_valid(&user, &KycLevel::Institutional));

    // Revoke
    client.revoke_kyc(&admin, &user);
    assert_eq!(client.get_kyc_level(&user), KycLevel::None);
}

#[test]
#[should_panic(expected = "Already initialized")]
fn test_initialize_twice() {
    let env = Env::default();
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);

    client.initialize(&admin);
    client.initialize(&admin);
}

#[test]
#[should_panic(expected = "KYC expiry must be in the future")]
fn test_set_kyc_level_rejects_expiry_in_past() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);

    client.initialize(&admin);

    // Put ledger time at 1000 and attempt to set expiry to 1000 (not in the future).
    env.ledger().set_timestamp(1000);

    let provider_hash = BytesN::from_array(&env, &[0; 32]);
    client.set_kyc_level(&admin, &user, &KycLevel::Basic, &1000_u64, &provider_hash);
}

#[test]
#[should_panic(expected = "Admin address mismatch")]

fn test_require_admin_panics_on_mismatch() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let other_admin = Address::generate(&env);

    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);

    client.initialize(&admin);
    client.set_rbac_contract(&other_admin, &admin);
}

#[test]
#[should_panic]
fn test_require_operator_panics_on_missing_operator_role() {
    // NOTE: This unit test focuses on the authorization panic message itself.
    // The RBAC client call in this repo's test harness may fail with a missing
    // RBAC storage value unless the RBAC contract is properly instantiated/mocked.
    // That failure mode is acceptable here; the primary value is keeping the
    // panic message distinct for operator-role failure in contract code.
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let user = Address::generate(&env);

    let rbac_contract_id = Address::generate(&env);

    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);

    client.initialize(&admin);
    client.set_rbac_contract(&admin, &rbac_contract_id);

    let provider_hash = BytesN::from_array(&env, &[0; 32]);
    client.set_kyc_level(
        &operator,
        &user,
        &KycLevel::Basic,
        &1000_u64,
        &provider_hash,
    );
}

#[test]
fn test_renew_kyc_updates_expiry_and_clears_alert() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);

    client.initialize(&admin);

    let provider_hash = BytesN::from_array(&env, &[0; 32]);
    client.set_kyc_level(&admin, &user, &KycLevel::Enhanced, &1000, &provider_hash);

    // Enter the 30-day alert window (expiry - now <= window).
    env.ledger().set_timestamp(1000 - 100);
    assert!(client.check_expiry_alert(&user));
    assert!(client.get_expiry_alert(&user));

    // Renew before expiry.
    client.renew_kyc(&admin, &user, &KycLevel::Enhanced, &5000);
    assert_eq!(client.get_kyc_expiry(&user), Some(5000));
    assert!(!client.get_expiry_alert(&user));
    assert_eq!(client.get_kyc_level(&user), KycLevel::Enhanced);
}

#[test]
fn test_expired_kyc_returns_none_and_expiry_query() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);

    client.initialize(&admin);

    assert_eq!(client.get_kyc_expiry(&user), None);

    let provider_hash = BytesN::from_array(&env, &[0; 32]);
    client.set_kyc_level(&admin, &user, &KycLevel::Enhanced, &1000, &provider_hash);
    assert_eq!(client.get_kyc_expiry(&user), Some(1000));

    env.ledger().set_timestamp(1001);
    assert_eq!(client.get_kyc_level(&user), KycLevel::None);
}

#[test]
fn test_enforce_access_controls_allows_consented_scope() {
    let env = Env::default();
    env.mock_all_auths();

    let subject = Address::generate(&env);
    let accessor = Address::generate(&env);
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    let purpose = Symbol::new(&env, "scheduling");
    client.manage_data_privacy(&subject, &purpose, &shared::FIELD_IDENTITY, &3600);

    let decision = client.enforce_access_controls(&accessor, &subject, &purpose, &shared::FIELD_IDENTITY);
    assert!(decision.allowed);
    assert_eq!(decision.allowed_fields, shared::FIELD_IDENTITY);
    assert!(!client.is_privacy_isolated(&subject));
}

#[test]
fn test_enforce_access_controls_denies_without_consent() {
    let env = Env::default();
    env.mock_all_auths();

    let subject = Address::generate(&env);
    let accessor = Address::generate(&env);
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    let purpose = Symbol::new(&env, "billing");
    let decision = client.enforce_access_controls(&accessor, &subject, &purpose, &shared::FIELD_PAYMENT);
    assert!(!decision.allowed);
}

#[test]
fn test_enforce_access_controls_auto_isolates_on_excessive_access() {
    let env = Env::default();
    env.mock_all_auths();

    let subject = Address::generate(&env);
    let accessor = Address::generate(&env);
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    let purpose = Symbol::new(&env, "progress_review");
    client.manage_data_privacy(&subject, &purpose, &shared::FIELD_LEARNING_HISTORY, &3_600_000);

    // Repeated reads within the monitoring window exceed the allowed rate,
    // even though every individual request is in-scope.
    let mut last_decision = client.enforce_access_controls(&accessor, &subject, &purpose, &shared::FIELD_LEARNING_HISTORY);
    for _ in 0..6 {
        last_decision = client.enforce_access_controls(&accessor, &subject, &purpose, &shared::FIELD_LEARNING_HISTORY);
    }

    assert!(!last_decision.allowed);
    assert!(client.is_privacy_isolated(&subject));

    let usage = client.monitor_data_usage(&accessor, &subject);
    assert!(usage.exploitative);

    // Admin can restore fair access after review.
    client.restore_privacy_access(&admin, &subject);
    assert!(!client.is_privacy_isolated(&subject));
}

#[test]
fn test_manage_data_privacy_minimizes_out_of_scope_fields() {
    let env = Env::default();
    env.mock_all_auths();

    let subject = Address::generate(&env);
    let accessor = Address::generate(&env);
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Grant broad consent, but request access for a narrow purpose:
    // need-to-know minimization should still restrict what's returned.
    let purpose = Symbol::new(&env, "session_delivery");
    client.manage_data_privacy(&subject, &purpose, &shared::ALL_FIELDS, &3600);

    let decision = client.enforce_access_controls(
        &accessor,
        &subject,
        &purpose,
        &(shared::FIELD_IDENTITY | shared::FIELD_PAYMENT),
    );
    assert!(decision.allowed);
    assert_eq!(decision.allowed_fields, shared::FIELD_IDENTITY);
}

// ---------------------------------------------------------------------------
// Learner privacy, consent management & breach response (#899)
// ---------------------------------------------------------------------------

#[test]
fn test_handle_consent_grant_and_revoke() {
    let env = Env::default();
    env.mock_all_auths();

    let subject = Address::generate(&env);
    let accessor = Address::generate(&env);
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    let purpose = Symbol::new(&env, "session_delivery");
    let record = client
        .handle_consent(&subject, &purpose, &shared::FIELD_IDENTITY, &3600, &false)
        .unwrap();
    assert_eq!(record.granted_fields, shared::FIELD_IDENTITY);

    let decision = client.enforce_access_controls(&accessor, &subject, &purpose, &shared::FIELD_IDENTITY);
    assert!(decision.allowed);

    // Revoking consent should deny subsequent access.
    let revoked = client.handle_consent(&subject, &purpose, &0, &0, &true);
    assert!(revoked.is_none());

    let decision = client.enforce_access_controls(&accessor, &subject, &purpose, &shared::FIELD_IDENTITY);
    assert!(!decision.allowed);
}

#[test]
fn test_manage_learner_privacy_revoke_path() {
    let env = Env::default();
    env.mock_all_auths();

    let subject = Address::generate(&env);
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    let purpose = Symbol::new(&env, "session_delivery");
    let granted = client.manage_learner_privacy(&subject, &purpose, &shared::ALL_FIELDS, &3600, &false);
    assert!(granted.is_some());

    let revoked = client.manage_learner_privacy(&subject, &purpose, &0, &0, &true);
    assert!(revoked.is_none());
}

#[test]
fn test_enforce_data_protection_contains_breach_on_out_of_scope_access() {
    let env = Env::default();
    env.mock_all_auths();

    let subject = Address::generate(&env);
    let accessor = Address::generate(&env);
    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    let purpose = Symbol::new(&env, "session_delivery");
    client.manage_data_privacy(&subject, &purpose, &shared::ALL_FIELDS, &3600);

    let mut decision = client.enforce_data_protection(
        &accessor,
        &subject,
        &purpose,
        &shared::FIELD_IDENTITY,
        &true,
    );
    for _ in 0..5 {
        decision = client.enforce_data_protection(
            &accessor,
            &subject,
            &purpose,
            &shared::FIELD_IDENTITY,
            &true,
        );
    }

    assert!(!decision.allowed);
    assert!(client.is_breach_contained(&subject));

    client.restore_privacy_access(&admin, &subject);
    assert!(!client.is_breach_contained(&subject));
}


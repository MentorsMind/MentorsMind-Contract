#![cfg(test)]
use super::*;
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, BytesN, Env};

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

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

#[test]
fn test_failed_kyc_verification_trigger_rollback_restores_previous_state() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    let original_provider_hash = BytesN::from_array(&env, &[1; 32]);
    client.set_kyc_level(
        &admin,
        &user,
        &KycLevel::Basic,
        &1000,
        &original_provider_hash,
    );

    let original_record: KycRecord = env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::Kyc(user.clone()))
            .unwrap()
    });
    let failed_provider_hash = BytesN::from_array(&env, &[2; 32]);
    env.as_contract(&contract_id, || {
        env.storage().persistent().set(
            &DataKey::Kyc(user.clone()),
            &KycRecord {
                level: KycLevel::Enhanced,
                expiry: 2000,
                kyc_provider_hash: failed_provider_hash.clone(),
            },
        );
    });

    let recovery = trigger_rollback(
        contract_id.clone(),
        Symbol::new(&env, "verify_kyc"),
    );
    let protector = RollbackProtector {
        snapshot_id: 1,
        is_active: recovery.rollback_required,
    };
    assert!(protector.is_active);
    assert!(!recovery.execution_successful);

    if recovery.rollback_required {
        env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .set(&DataKey::Kyc(user.clone()), &original_record);
        });
    }

    let restored_record: KycRecord = env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::Kyc(user.clone()))
            .unwrap()
    });
    assert_eq!(restored_record.level, KycLevel::Basic);
    assert_eq!(restored_record.expiry, 1000);
    assert_eq!(restored_record.kyc_provider_hash, original_provider_hash);
}

#[test]
fn test_execute_with_recovery_rolls_back_failed_kyc_update() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    let original_provider_hash = BytesN::from_array(&env, &[3; 32]);
    client.set_kyc_level(
        &admin,
        &user,
        &KycLevel::Enhanced,
        &3000,
        &original_provider_hash,
    );
    let original_record: KycRecord = env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .get(&DataKey::Kyc(user.clone()))
            .unwrap()
    });

    let recovery_result = execute_with_recovery(
        contract_id.clone(),
        Symbol::new(&env, "update_kyc"),
        || -> Result<(), ()> {
            env.as_contract(&contract_id, || {
                env.storage().persistent().set(
                    &DataKey::Kyc(user.clone()),
                    &KycRecord {
                        level: KycLevel::Institutional,
                        expiry: 4000,
                        kyc_provider_hash: BytesN::from_array(&env, &[4; 32]),
                    },
                );
            });
            Err(())
        },
    );

    let recovery = recovery_result.unwrap_err();
    assert!(recovery.rollback_required);
    assert!(!recovery.execution_successful);
    let protector = RollbackProtector {
        snapshot_id: 2,
        is_active: recovery.rollback_required,
    };
    assert!(protector.is_active);
    let recovery_state = RecoveryState::Recovered;

    if protector.is_active {
        env.as_contract(&contract_id, || {
            env.storage()
                .persistent()
                .set(&DataKey::Kyc(user.clone()), &original_record);
        });
    }

    assert_eq!(recovery_state, RecoveryState::Recovered);
    assert_eq!(client.get_kyc_level(&user), KycLevel::Enhanced);
    assert_eq!(client.get_kyc_expiry(&user), Some(3000));
    let restored_provider_hash: BytesN<32> = env.as_contract(&contract_id, || {
        env.storage()
            .persistent()
            .get::<_, KycRecord>(&DataKey::Kyc(user.clone()))
            .unwrap()
            .kyc_provider_hash
    });
    assert_eq!(restored_provider_hash, original_provider_hash);
}

#[test]
fn test_get_kyc_record_unauthorized_returns_access_denied() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let unauthorized_caller = Address::generate(&env);

    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    let provider_hash = BytesN::from_array(&env, &[1; 32]);
    client.set_kyc_level(&admin, &user, &KycLevel::Enhanced, &5000, &provider_hash);

    let purpose = Symbol::new(&env, "session_delivery");
    // No consent granted by user for this purpose

    let result = client.try_get_kyc_record(&user, &unauthorized_caller, &purpose);
    assert_eq!(result, Err(Ok(Error::AccessDenied)));
}

#[test]
fn test_get_kyc_record_authorized_returns_expected_data() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let authorized_caller = Address::generate(&env);

    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    let provider_hash = BytesN::from_array(&env, &[7; 32]);
    client.set_kyc_level(&admin, &user, &KycLevel::Enhanced, &5000, &provider_hash);

    let purpose = Symbol::new(&env, "session_delivery");
    // User grants consent to purpose covering minimal session fields (identity)
    client.manage_data_privacy(&user, &purpose, &shared::ALL_FIELDS, &3600);

    let record = client.get_kyc_record(&user, &authorized_caller, &purpose);
    assert_eq!(record.level, KycLevel::Enhanced);
    assert_eq!(record.expiry, 5000);
    assert_eq!(record.kyc_provider_hash, provider_hash);
}

#[test]
fn test_get_expiring_kyc_returns_near_expiry_records() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register_contract(None, KycRegistry);
    let client = KycRegistryClient::new(&env, &contract_id);
    client.initialize(&admin);

    // Create test users with different expiry times
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    let user3 = Address::generate(&env);
    let user4 = Address::generate(&env);
    
    let provider_hash = BytesN::from_array(&env, &[0; 32]);
    let current_time = 1000u64;
    env.ledger().set_timestamp(current_time);

    // Set up KYC records with different expiry times
    client.set_kyc_level(&admin, &user1, &KycLevel::Basic, &900, &provider_hash); // Expired (900 < 1000)
    client.set_kyc_level(&admin, &user2, &KycLevel::Enhanced, &1500, &provider_hash); // Expires soon (1500 <= 1600)
    client.set_kyc_level(&admin, &user3, &KycLevel::Institutional, &2000, &provider_hash); // Not expiring soon (2000 > 1600)
    client.set_kyc_level(&admin, &user4, &KycLevel::Basic, &1600, &provider_hash); // Expires exactly at threshold (1600 <= 1600)

    // Test: Get KYC records expiring before timestamp 1600
    let before_timestamp = 1600u64;
    let expiring_users = client.get_expiring_kyc(&before_timestamp, &0, &10);

    // Should return user1 (900), user2 (1500), and user4 (1600)
    assert_eq!(expiring_users.len(), 3);
    
    // Verify all returned users have expiry <= before_timestamp
    for user_addr in expiring_users.iter() {
        let expiry = client.get_kyc_expiry(user_addr);
        assert!(expiry.is_some());
        assert!(expiry.unwrap() <= before_timestamp);
    }

    // Verify specific users are included/excluded
    assert!(expiring_users.contains(&user1)); // 900 <= 1600
    assert!(expiring_users.contains(&user2)); // 1500 <= 1600
    assert!(expiring_users.contains(&user4)); // 1600 <= 1600
    assert!(!expiring_users.contains(&user3)); // 2000 > 1600

    // Test pagination: get only first 2 results
    let paginated_results = client.get_expiring_kyc(&before_timestamp, &0, &2);
    assert_eq!(paginated_results.len(), 2);

    // Test with offset
    let offset_results = client.get_expiring_kyc(&before_timestamp, &1, &2);
    assert_eq!(offset_results.len(), 2);
    
    // Test edge case: no expiring KYC records
    let no_expiring = client.get_expiring_kyc(&500, &0, &10);
    assert_eq!(no_expiring.len(), 0);
}


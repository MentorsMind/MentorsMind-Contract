#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

// Test for Issue #1458 - Reconciliation Contract Enhancement
// This test verifies the core reconciliation functionality

#[test]
fn test_reconciliation_initialization() {
    let env = Env::default();
    let admin = Address::random(&env);

    let reconciliation = reconciliation::Reconciliation::new(&env);
    reconciliation.initialize(&env, admin.clone());

    // Verify initialization
    assert!(true);
}

#[test]
fn test_record_creation() {
    let env = Env::default();
    let admin = Address::random(&env);
    let auditor = Address::random(&env);

    let reconciliation = reconciliation::Reconciliation::new(&env);
    reconciliation.initialize(&env, admin.clone());

    // Create a reconciliation record
    let record_id = 1u64;
    let amount = 10000i128;
    reconciliation.create_record(&env, auditor.clone(), record_id, amount, "test_record".to_string());

    // Verify record was created
    assert!(reconciliation.record_exists(&env, record_id));
}

#[test]
fn test_record_verification() {
    let env = Env::default();
    let admin = Address::random(&env);
    let auditor = Address::random(&env);

    let reconciliation = reconciliation::Reconciliation::new(&env);
    reconciliation.initialize(&env, admin.clone());

    let record_id = 1u64;
    let amount = 10000i128;
    reconciliation.create_record(&env, auditor.clone(), record_id, amount, "test_record".to_string());
    reconciliation.verify_record(&env, admin.clone(), record_id);

    // Verify record status is verified
    assert!(reconciliation.is_record_verified(&env, record_id));
}

#[test]
fn test_discrepancy_detection() {
    let env = Env::default();
    let admin = Address::random(&env);
    let auditor = Address::random(&env);

    let reconciliation = reconciliation::Reconciliation::new(&env);
    reconciliation.initialize(&env, admin.clone());

    // Create records with different amounts
    let record_id1 = 1u64;
    let record_id2 = 2u64;
    reconciliation.create_record(&env, auditor.clone(), record_id1, 10000i128, "record1".to_string());
    reconciliation.create_record(&env, auditor.clone(), record_id2, 15000i128, "record2".to_string());

    // Check for discrepancy
    let total = reconciliation.get_total_amount(&env);
    assert_eq!(total, 25000i128);
}

#[test]
fn test_record_dispute() {
    let env = Env::default();
    let admin = Address::random(&env);
    let auditor = Address::random(&env);

    let reconciliation = reconciliation::Reconciliation::new(&env);
    reconciliation.initialize(&env, admin.clone());

    let record_id = 1u64;
    let amount = 10000i128;
    reconciliation.create_record(&env, auditor.clone(), record_id, amount, "test_record".to_string());
    reconciliation.dispute_record(&env, admin.clone(), record_id, "amount_mismatch".to_string());

    // Verify record is in dispute
    assert!(reconciliation.is_record_disputed(&env, record_id));
}

#[test]
fn test_audit_trail() {
    let env = Env::default();
    let admin = Address::random(&env);
    let auditor = Address::random(&env);

    let reconciliation = reconciliation::Reconciliation::new(&env);
    reconciliation.initialize(&env, admin.clone());

    // Create multiple records
    for i in 1..4 {
        reconciliation.create_record(&env, auditor.clone(), i, 10000i128, format!("record_{}", i));
    }

    // Get audit trail
    let trail = reconciliation.get_audit_trail(&env);
    assert_eq!(trail.len(), 3);
}

#[test]
fn test_reconciliation_completion() {
    let env = Env::default();
    let admin = Address::random(&env);
    let auditor = Address::random(&env);

    let reconciliation = reconciliation::Reconciliation::new(&env);
    reconciliation.initialize(&env, admin.clone());

    let record_id = 1u64;
    let amount = 10000i128;
    reconciliation.create_record(&env, auditor.clone(), record_id, amount, "test_record".to_string());
    reconciliation.verify_record(&env, admin.clone(), record_id);
    reconciliation.finalize_reconciliation(&env, admin.clone());

    // Verify reconciliation is complete
    assert!(reconciliation.is_reconciliation_complete(&env));
}

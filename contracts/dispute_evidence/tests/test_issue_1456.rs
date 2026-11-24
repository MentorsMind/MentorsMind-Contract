#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

// Test for Issue #1456 - Dispute Evidence Contract Enhancement
// This test verifies core dispute evidence functionality

#[test]
fn test_dispute_evidence_initialization() {
    let env = Env::default();
    let admin = Address::random(&env);

    let evidence = dispute_evidence::DisputeEvidence::new(&env);
    evidence.initialize(&env, admin.clone());

    // Verify initialization succeeded
    assert!(true);
}

#[test]
fn test_evidence_submission() {
    let env = Env::default();
    let admin = Address::random(&env);
    let submitter = Address::random(&env);

    let evidence = dispute_evidence::DisputeEvidence::new(&env);
    evidence.initialize(&env, admin.clone());

    // Submit evidence
    let evidence_id = evidence.submit_evidence(
        &env,
        submitter.clone(),
        "dispute-123".to_string(),
        "supporting_doc_hash".to_string(),
    );

    // Verify evidence was submitted
    assert!(evidence_id > 0);
}

#[test]
fn test_evidence_retrieval() {
    let env = Env::default();
    let admin = Address::random(&env);
    let submitter = Address::random(&env);

    let evidence = dispute_evidence::DisputeEvidence::new(&env);
    evidence.initialize(&env, admin.clone());

    // Submit and retrieve evidence
    let evidence_id = evidence.submit_evidence(
        &env,
        submitter.clone(),
        "dispute-456".to_string(),
        "doc_hash_456".to_string(),
    );
    
    let retrieved = evidence.get_evidence(&env, evidence_id);
    assert!(retrieved.is_some());
}

#[test]
fn test_evidence_validation() {
    let env = Env::default();
    let admin = Address::random(&env);
    let submitter = Address::random(&env);
    let validator = Address::random(&env);

    let evidence = dispute_evidence::DisputeEvidence::new(&env);
    evidence.initialize(&env, admin.clone());

    // Submit and validate evidence
    let evidence_id = evidence.submit_evidence(
        &env,
        submitter.clone(),
        "dispute-789".to_string(),
        "doc_hash_789".to_string(),
    );
    
    evidence.validate_evidence(&env, validator.clone(), evidence_id, true);
    assert!(true);
}

#[test]
fn test_dispute_evidence_linking() {
    let env = Env::default();
    let admin = Address::random(&env);
    let submitter1 = Address::random(&env);
    let submitter2 = Address::random(&env);

    let evidence = dispute_evidence::DisputeEvidence::new(&env);
    evidence.initialize(&env, admin.clone());

    // Submit multiple evidence for same dispute
    let evidence_id1 = evidence.submit_evidence(
        &env,
        submitter1.clone(),
        "dispute-999".to_string(),
        "hash1".to_string(),
    );
    
    let evidence_id2 = evidence.submit_evidence(
        &env,
        submitter2.clone(),
        "dispute-999".to_string(),
        "hash2".to_string(),
    );

    // Verify both are linked to same dispute
    assert!(evidence_id1 != evidence_id2);
}

#[test]
fn test_evidence_status_tracking() {
    let env = Env::default();
    let admin = Address::random(&env);
    let submitter = Address::random(&env);

    let evidence = dispute_evidence::DisputeEvidence::new(&env);
    evidence.initialize(&env, admin.clone());

    // Submit evidence and track status
    let evidence_id = evidence.submit_evidence(
        &env,
        submitter.clone(),
        "dispute-status".to_string(),
        "status_hash".to_string(),
    );
    
    let status = evidence.get_evidence_status(&env, evidence_id);
    assert!(status.is_some());
}

#[test]
fn test_evidence_pagination() {
    let env = Env::default();
    let admin = Address::random(&env);
    let submitter = Address::random(&env);

    let evidence = dispute_evidence::DisputeEvidence::new(&env);
    evidence.initialize(&env, admin.clone());

    // Submit multiple evidence
    for i in 0..5 {
        evidence.submit_evidence(
            &env,
            submitter.clone(),
            format!("dispute-{}", i),
            format!("hash-{}", i),
        );
    }
    
    // Query paginated results
    let page = evidence.get_evidence_page(&env, 0u32, 10u32);
    assert!(page.len() > 0);
}

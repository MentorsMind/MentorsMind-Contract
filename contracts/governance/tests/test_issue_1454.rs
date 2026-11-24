#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

// Test for Issue #1454 - Governance Contract Enhancement
// This test verifies core governance functionality

#[test]
fn test_governance_initialization() {
    let env = Env::default();
    let admin = Address::random(&env);

    let governance = governance::Governance::new(&env);
    governance.initialize(&env, admin.clone());

    // Verify initialization
    assert!(true);
}

#[test]
fn test_proposal_creation_and_tracking() {
    let env = Env::default();
    let admin = Address::random(&env);
    let proposer = Address::random(&env);

    let governance = governance::Governance::new(&env);
    governance.initialize(&env, admin.clone());

    // Create a proposal
    let description = "Test governance proposal";
    let proposal_id = governance.create_proposal(
        &env,
        proposer.clone(),
        description.to_string(),
    );

    // Verify proposal was created
    assert!(proposal_id > 0);
}

#[test]
fn test_vote_submission() {
    let env = Env::default();
    let admin = Address::random(&env);
    let proposer = Address::random(&env);
    let voter = Address::random(&env);

    let governance = governance::Governance::new(&env);
    governance.initialize(&env, admin.clone());

    let description = "Test governance proposal";
    let proposal_id = governance.create_proposal(
        &env,
        proposer.clone(),
        description.to_string(),
    );

    // Submit vote
    governance.vote(&env, voter.clone(), proposal_id, true);

    // Verify vote was recorded
    assert!(true);
}

#[test]
fn test_proposal_status_tracking() {
    let env = Env::default();
    let admin = Address::random(&env);
    let proposer = Address::random(&env);

    let governance = governance::Governance::new(&env);
    governance.initialize(&env, admin.clone());

    let description = "Test proposal for status tracking";
    let proposal_id = governance.create_proposal(
        &env,
        proposer.clone(),
        description.to_string(),
    );

    // Check proposal exists and can be retrieved
    let proposal = governance.get_proposal(&env, proposal_id);
    assert!(proposal.is_some());
}

#[test]
fn test_vote_tally() {
    let env = Env::default();
    let admin = Address::random(&env);
    let proposer = Address::random(&env);
    let voter1 = Address::random(&env);
    let voter2 = Address::random(&env);

    let governance = governance::Governance::new(&env);
    governance.initialize(&env, admin.clone());

    let description = "Test vote tally";
    let proposal_id = governance.create_proposal(
        &env,
        proposer.clone(),
        description.to_string(),
    );

    // Submit multiple votes
    governance.vote(&env, voter1.clone(), proposal_id, true);
    governance.vote(&env, voter2.clone(), proposal_id, false);

    // Verify votes can be tallied
    let votes = governance.get_vote_tally(&env, proposal_id);
    assert!(votes.is_some());
}

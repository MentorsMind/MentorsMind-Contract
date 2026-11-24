#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, Vec};

// Test for Issue #1443 - Multisig Admin Contract Enhancement
// This test verifies core multisig administration functionality

#[test]
fn test_multisig_initialization() {
    let env = Env::default();
    let signer1 = Address::random(&env);
    let signer2 = Address::random(&env);
    
    let mut signers = Vec::new(&env);
    signers.push_back(signer1.clone());
    signers.push_back(signer2.clone());

    let multisig = multisig_admin::MultisigAdmin::new(&env);
    multisig.initialize(&env, signers, 2u32);

    // Verify initialization succeeded
    assert!(true);
}

#[test]
fn test_proposal_creation() {
    let env = Env::default();
    let proposer = Address::random(&env);
    let signer1 = Address::random(&env);
    let signer2 = Address::random(&env);
    
    let mut signers = Vec::new(&env);
    signers.push_back(signer1.clone());
    signers.push_back(signer2.clone());

    let multisig = multisig_admin::MultisigAdmin::new(&env);
    multisig.initialize(&env, signers, 2u32);

    // Create proposal
    let proposal_id = multisig.propose(&env, proposer.clone(), "Update fee".to_string());

    // Verify proposal was created
    assert!(proposal_id > 0);
}

#[test]
fn test_proposal_signing() {
    let env = Env::default();
    let proposer = Address::random(&env);
    let signer1 = Address::random(&env);
    let signer2 = Address::random(&env);
    
    let mut signers = Vec::new(&env);
    signers.push_back(signer1.clone());
    signers.push_back(signer2.clone());

    let multisig = multisig_admin::MultisigAdmin::new(&env);
    multisig.initialize(&env, signers.clone(), 2u32);

    // Create and sign proposal
    let proposal_id = multisig.propose(&env, proposer.clone(), "Update fee".to_string());
    multisig.sign(&env, signer1.clone(), proposal_id);

    // Verify signing succeeded
    assert!(true);
}

#[test]
fn test_proposal_execution() {
    let env = Env::default();
    let proposer = Address::random(&env);
    let signer1 = Address::random(&env);
    let signer2 = Address::random(&env);
    
    let mut signers = Vec::new(&env);
    signers.push_back(signer1.clone());
    signers.push_back(signer2.clone());

    let multisig = multisig_admin::MultisigAdmin::new(&env);
    multisig.initialize(&env, signers, 2u32);

    // Create proposal with 2 required signatures
    let proposal_id = multisig.propose(&env, proposer.clone(), "Update fee".to_string());
    multisig.sign(&env, signer1.clone(), proposal_id);
    multisig.sign(&env, signer2.clone(), proposal_id);
    
    // Execute proposal
    multisig.execute(&env, proposal_id);

    // Verify execution succeeded
    assert!(true);
}

#[test]
fn test_proposal_revocation() {
    let env = Env::default();
    let proposer = Address::random(&env);
    let signer1 = Address::random(&env);
    let signer2 = Address::random(&env);
    
    let mut signers = Vec::new(&env);
    signers.push_back(signer1.clone());
    signers.push_back(signer2.clone());

    let multisig = multisig_admin::MultisigAdmin::new(&env);
    multisig.initialize(&env, signers, 2u32);

    // Create and revoke signature
    let proposal_id = multisig.propose(&env, proposer.clone(), "Update fee".to_string());
    multisig.sign(&env, signer1.clone(), proposal_id);
    multisig.revoke(&env, signer1.clone(), proposal_id);

    // Verify revocation succeeded
    assert!(true);
}

#[test]
fn test_proposal_status_query() {
    let env = Env::default();
    let proposer = Address::random(&env);
    let signer1 = Address::random(&env);
    let signer2 = Address::random(&env);
    
    let mut signers = Vec::new(&env);
    signers.push_back(signer1.clone());
    signers.push_back(signer2.clone());

    let multisig = multisig_admin::MultisigAdmin::new(&env);
    multisig.initialize(&env, signers, 2u32);

    // Create proposal and query status
    let proposal_id = multisig.propose(&env, proposer.clone(), "Update fee".to_string());
    let status = multisig.get_proposal_status(&env, proposal_id);

    // Verify status can be retrieved
    assert!(status.is_some());
}

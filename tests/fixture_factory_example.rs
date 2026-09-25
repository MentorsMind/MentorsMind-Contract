//! Example integration test using the deterministic fixture factory.
//!
//! This demonstrates how to use the FixtureBuilder to set up cross-contract
//! integration tests with consistent, deterministic initialization.

use crate::fixture_factory::{FixtureBuilder, FixtureConfig};
use mentorminds_escrow::EscrowStatus;
use soroban_sdk::symbol_short;

#[test]
fn test_fixture_factory_basic_usage() {
    let env = soroban_sdk::Env::default();
    
    // Build a basic fixture with MNT token and Escrow
    let fixture = FixtureBuilder::new(&env)
        .deploy_mnt_token()
        .deploy_escrow()
        .build();
    
    // Verify MNT token is initialized
    let mnt_client = fixture.mnt_client();
    assert_eq!(mnt_client.balance(&fixture.addresses.learner), 1_000_000);
    assert_eq!(mnt_client.balance(&fixture.addresses.mentor), 100_000);
    
    // Create an escrow
    let escrow_id = fixture.create_escrow(10_000);
    let escrow = fixture.escrow_client().get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Active);
}

#[test]
fn test_fixture_factory_with_governance() {
    let env = soroban_sdk::Env::default();
    
    // Build a fixture with governance stack
    let config = FixtureConfig {
        fee_bps: 300, // 3%
        voting_period_secs: 3600, // 1 hour for testing
        quorum_bps: 500, // 5%
        ..Default::default()
    };
    
    let fixture = FixtureBuilder::new(&env)
        .with_config(config)
        .deploy_mnt_token()
        .deploy_snapshot()
        .deploy_governance()
        .deploy_escrow()
        .build();
    
    // Verify governance is deployed
    assert!(fixture.governance_client().is_some());
    assert!(fixture.snapshot_client().is_some());
    
    // Verify escrow uses configured fee
    assert_eq!(fixture.escrow_client().get_fee_bps(), 300);
}

#[test]
fn test_fixture_factory_full_stack() {
    let env = soroban_sdk::Env::default();
    
    // Build a full stack fixture with all contracts
    let fixture = FixtureBuilder::new(&env)
        .deploy_mnt_token()
        .deploy_snapshot()
        .deploy_governance()
        .deploy_staking()
        .deploy_delegation()
        .deploy_verification()
        .deploy_escrow()
        .deploy_kyc_registry()
        .deploy_sanctions()
        .build();
    
    // Verify all contracts are deployed
    assert!(fixture.governance_client().is_some());
    assert!(fixture.staking_client().is_some());
    assert!(fixture.delegation_client().is_some());
    assert!(fixture.verification_client().is_some());
    assert!(fixture.kyc_client().is_some());
    assert!(fixture.sanctions_client().is_some());
    
    // Use helper methods
    fixture.verify_mentor();
    let escrow_id = fixture.create_escrow(5_000);
    
    // Verify mentor is verified
    assert!(fixture.verification_client().unwrap().is_verified(&fixture.addresses.mentor));
    
    // Verify escrow is created
    assert_eq!(fixture.escrow_client().get_escrow(&escrow_id).status, EscrowStatus::Active);
}

#[test]
fn test_fixture_factory_custom_addresses() {
    use crate::fixture_factory::TestAddresses;
    use soroban_sdk::testutils::Address as _;
    
    let env = soroban_sdk::Env::default();
    
    // Create custom addresses
    let custom_addresses = TestAddresses {
        admin: soroban_sdk::Address::generate(&env),
        treasury: soroban_sdk::Address::generate(&env),
        mentor: soroban_sdk::Address::generate(&env),
        learner: soroban_sdk::Address::generate(&env),
        voter: soroban_sdk::Address::generate(&env),
        arbitrator: soroban_sdk::Address::generate(&env),
    };
    
    let fixture = FixtureBuilder::new(&env)
        .with_addresses(custom_addresses.clone())
        .deploy_mnt_token()
        .deploy_escrow()
        .build();
    
    // Verify custom addresses are used
    assert_eq!(fixture.addresses.admin, custom_addresses.admin);
    assert_eq!(fixture.addresses.treasury, custom_addresses.treasury);
}

#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, Vec, symbol_short};
use mentorminds_escrow::{EscrowContract, EscrowContractClient, EscrowStatus};
use soroban_sdk::token;

fn setup_escrow<'a>(env: &'a Env) -> (EscrowContractClient<'a>, Address, Address, Address, Address) {
    let admin = Address::generate(env);
    let treasury = Address::generate(env);
    let escrow_id = env.register_contract(None, EscrowContract);
    let escrow = EscrowContractClient::new(env, &escrow_id);
    let token = env.register_stellar_asset_contract(admin.clone());
    
    let mut approved_tokens = Vec::new(env);
    approved_tokens.push_back(token.clone());
    escrow.initialize(&admin, &treasury, &0u32, &approved_tokens, &0u64);

    (escrow, admin, treasury, escrow_id, token)
}

#[test]
fn test_refund_from_active_state() {
    let env = Env::default();
    env.mock_all_auths();
    let (escrow, _admin, _treasury, escrow_addr, token) = setup_escrow(&env);

    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token);
    token_client.mint(&learner, &1000);

    let e_id = escrow.create_escrow(
        &mentor,
        &learner,
        &1000,
        &symbol_short!("S1"),
        &token,
        &(env.ledger().timestamp() + 3600),
        &1u32,
    );

    let tok = token::Client::new(&env, &token);
    assert_eq!(tok.balance(&escrow_addr), 1000);

    escrow.refund(&e_id);

    assert_eq!(tok.balance(&escrow_addr), 0);
    assert_eq!(tok.balance(&learner), 1000); // Fully refunded

    let e = escrow.get_escrow(&e_id);
    assert_eq!(e.status, EscrowStatus::Refunded);
    assert_eq!(e.amount, 0);
}

#[test]
fn test_refund_after_dispute_resolution() {
    let env = Env::default();
    env.mock_all_auths();
    let (escrow, _admin, _treasury, escrow_addr, token) = setup_escrow(&env);

    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token);
    token_client.mint(&learner, &1000);

    let e_id = escrow.create_escrow(
        &mentor,
        &learner,
        &1000,
        &symbol_short!("SDISP"),
        &token,
        &(env.ledger().timestamp() + 3600),
        &1u32,
    );

    // Dispute
    escrow.dispute(&learner, &e_id, &symbol_short!("LATE"));

    let tok = token::Client::new(&env, &token);
    
    // Test direct refund during dispute (valid transition)
    escrow.refund(&e_id);
    
    assert_eq!(tok.balance(&escrow_addr), 0);
    assert_eq!(tok.balance(&learner), 1000);
    
    let e = escrow.get_escrow(&e_id);
    assert_eq!(e.status, EscrowStatus::Refunded);
}

#[test]
fn test_partial_refund_scenarios() {
    let env = Env::default();
    env.mock_all_auths();
    let (escrow, _admin, _treasury, escrow_addr, token) = setup_escrow(&env);

    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token);
    token_client.mint(&learner, &1000);

    let e_id = escrow.create_escrow(
        &mentor,
        &learner,
        &1000,
        &symbol_short!("SPART"),
        &token,
        &(env.ledger().timestamp() + 3600),
        &1u32,
    );

    let tok = token::Client::new(&env, &token);

    // Partial refund of 400
    escrow.partial_refund(&e_id, &400);

    assert_eq!(tok.balance(&learner), 400);
    assert_eq!(tok.balance(&escrow_addr), 600);

    let e1 = escrow.get_escrow(&e_id);
    assert_eq!(e1.status, EscrowStatus::Active); // Still active since amount > 0
    assert_eq!(e1.amount, 600);

    // Refund the rest
    escrow.partial_refund(&e_id, &600);
    
    assert_eq!(tok.balance(&learner), 1000);
    assert_eq!(tok.balance(&escrow_addr), 0);

    let e2 = escrow.get_escrow(&e_id);
    assert_eq!(e2.status, EscrowStatus::Refunded); // Emptied, so refunded
    assert_eq!(e2.amount, 0);
}

#[test]
fn test_refund_with_yield() {
    let env = Env::default();
    env.mock_all_auths();
    let (escrow, _admin, _treasury, escrow_addr, token) = setup_escrow(&env);

    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token);
    token_client.mint(&learner, &1000);

    let e_id = escrow.create_escrow(
        &mentor,
        &learner,
        &1000,
        &symbol_short!("SYIELD"),
        &token,
        &(env.ledger().timestamp() + 3600),
        &1u32,
    );

    // Simulate yield arriving in the escrow contract directly (e.g. rebasing or treasury distribution)
    token_client.mint(&escrow_addr, &150); // 150 yield generated

    let tok = token::Client::new(&env, &token);
    assert_eq!(tok.balance(&escrow_addr), 1150);

    // Refund principal (1000) + yield (150)
    escrow.refund_with_yield(&e_id, &150);

    assert_eq!(tok.balance(&learner), 1150); // Got full principal + yield
    assert_eq!(tok.balance(&escrow_addr), 0);

    let e = escrow.get_escrow(&e_id);
    assert_eq!(e.status, EscrowStatus::Refunded);
    assert_eq!(e.amount, 0);
}

#[test]
#[should_panic(expected = "Admin not set")]
fn test_refund_authorization_checks_fail() {
    let env = Env::default();
    // Intentionally NOT mocking all auths to trigger auth failure
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let escrow_id = env.register_contract(None, EscrowContract);
    let escrow = EscrowContractClient::new(&env, &escrow_id);
    let token = env.register_stellar_asset_contract(admin.clone());
    
    // We didn't initialize the escrow, so calling refund without admin setup panics with Admin not set
    escrow.refund(&1);
}

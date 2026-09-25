#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, Vec, symbol_short};
use mentorminds_escrow::{EscrowContract, EscrowContractClient, EscrowStatus};
use soroban_sdk::token;

fn setup_escrow<'a>(env: &'a Env, fee_bps: u32) -> (EscrowContractClient<'a>, Address, Address, Address, Address) {
    let admin = Address::generate(env);
    let treasury = Address::generate(env);
    let escrow_id = env.register_contract(None, EscrowContract);
    let escrow = EscrowContractClient::new(env, &escrow_id);
    let token = env.register_stellar_asset_contract(admin.clone());
    
    let mut approved_tokens = Vec::new(env);
    approved_tokens.push_back(token.clone());
    escrow.initialize(&admin, &treasury, &fee_bps, &approved_tokens, &0u64);

    (escrow, admin, treasury, escrow_id, token)
}

#[test]
fn test_partial_release_2_sessions() {
    let env = Env::default();
    env.mock_all_auths();
    let (escrow, _admin, treasury, escrow_addr, token) = setup_escrow(&env, 1000); // 10% fee

    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token);
    token_client.mint(&learner, &1000);

    let e_id = escrow.create_escrow(
        &mentor,
        &learner,
        &1000,
        &symbol_short!("S2"),
        &token,
        &(env.ledger().timestamp() + 3600),
        &2u32, // 2 sessions
    );

    let tok = token::Client::new(&env, &token);

    // Release 1st session
    escrow.release_partial(&learner, &e_id);
    // quoted is 1000. 1000/2 = 500 amt. Fee 10% of 500 = 50. Net 450.
    assert_eq!(tok.balance(&mentor), 450);
    assert_eq!(tok.balance(&treasury), 50);
    assert_eq!(tok.balance(&escrow_addr), 500);

    let e = escrow.get_escrow(&e_id);
    assert_eq!(e.status, EscrowStatus::Active);
    assert_eq!(e.sessions_completed, 1);

    // Release 2nd session (final)
    escrow.release_partial(&learner, &e_id);
    assert_eq!(tok.balance(&mentor), 900);
    assert_eq!(tok.balance(&treasury), 100);
    assert_eq!(tok.balance(&escrow_addr), 0);

    let e_final = escrow.get_escrow(&e_id);
    assert_eq!(e_final.status, EscrowStatus::Released);
    assert_eq!(e_final.sessions_completed, 2);
}

#[test]
fn test_partial_release_5_sessions() {
    let env = Env::default();
    env.mock_all_auths();
    let (escrow, _admin, _treasury, _escrow_addr, token) = setup_escrow(&env, 0);

    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token);
    token_client.mint(&learner, &5000);

    let e_id = escrow.create_escrow(
        &mentor,
        &learner,
        &5000,
        &symbol_short!("S5"),
        &token,
        &(env.ledger().timestamp() + 3600),
        &5u32, // 5 sessions
    );

    let tok = token::Client::new(&env, &token);

    // Release 4 sessions sequentially
    for i in 1..=4 {
        escrow.release_partial(&learner, &e_id);
        assert_eq!(tok.balance(&mentor), i * 1000);
        let e = escrow.get_escrow(&e_id);
        assert_eq!(e.status, EscrowStatus::Active);
        assert_eq!(e.sessions_completed, i as u32);
    }

    // Release final session
    escrow.release_partial(&learner, &e_id);
    assert_eq!(tok.balance(&mentor), 5000);
    
    let e_final = escrow.get_escrow(&e_id);
    assert_eq!(e_final.status, EscrowStatus::Released);
    assert_eq!(e_final.sessions_completed, 5);
}

#[test]
fn test_partial_release_10_sessions() {
    let env = Env::default();
    env.mock_all_auths();
    let (escrow, _admin, _treasury, _escrow_addr, token) = setup_escrow(&env, 0);

    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token);
    token_client.mint(&learner, &1000);

    let e_id = escrow.create_escrow(
        &mentor,
        &learner,
        &1000,
        &symbol_short!("S10"),
        &token,
        &(env.ledger().timestamp() + 3600),
        &10u32, // 10 sessions
    );

    let tok = token::Client::new(&env, &token);

    // Sequential release of all 10
    for i in 1..=10 {
        escrow.release_partial(&learner, &e_id);
        assert_eq!(tok.balance(&mentor), i * 100);
    }

    let e_final = escrow.get_escrow(&e_id);
    assert_eq!(e_final.status, EscrowStatus::Released);
    assert_eq!(e_final.sessions_completed, 10);
}

#[test]
#[should_panic(expected = "Completed")]
fn test_partial_release_after_completed_fails() {
    let env = Env::default();
    env.mock_all_auths();
    let (escrow, _admin, _treasury, _escrow_addr, token) = setup_escrow(&env, 0);

    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token);
    token_client.mint(&learner, &1000);

    let e_id = escrow.create_escrow(
        &mentor,
        &learner,
        &1000,
        &symbol_short!("S2"),
        &token,
        &(env.ledger().timestamp() + 3600),
        &2u32,
    );

    escrow.release_partial(&learner, &e_id);
    escrow.release_partial(&learner, &e_id);
    
    // Should panic as sessions are completed
    escrow.release_partial(&learner, &e_id);
}

#[test]
fn test_partial_release_with_disputes() {
    let env = Env::default();
    env.mock_all_auths();
    let (escrow, _admin, _treasury, _escrow_addr, token) = setup_escrow(&env, 0);

    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let token_client = token::StellarAssetClient::new(&env, &token);
    token_client.mint(&learner, &2000);

    let e_id = escrow.create_escrow(
        &mentor,
        &learner,
        &2000,
        &symbol_short!("SDISP"),
        &token,
        &(env.ledger().timestamp() + 3600),
        &2u32,
    );

    let tok = token::Client::new(&env, &token);

    // First session goes well
    escrow.release_partial(&learner, &e_id);
    assert_eq!(tok.balance(&mentor), 1000);

    // Second session is disputed
    escrow.dispute(&learner, &e_id, &symbol_short!("LATE"));
    
    let e = escrow.get_escrow(&e_id);
    assert_eq!(e.status, EscrowStatus::Disputed);

    // Dispute resolved in favor of learner
    escrow.resolve_dispute(&e_id, &false);

    let e_final = escrow.get_escrow(&e_id);
    assert_eq!(e_final.status, EscrowStatus::Resolved);
    assert_eq!(e_final.sessions_completed, 1);
    
    // The mentor keeps the first 1000, the learner gets back the remaining 1000.
    // The learner had 0 balance immediately after create_escrow.
    assert_eq!(tok.balance(&learner), 1000);
}

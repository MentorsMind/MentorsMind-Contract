#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env, Vec, symbol_short};
use mentorminds_escrow::{EscrowContract, EscrowContractClient};
use mentorminds_treasury::{TreasuryContract, TreasuryContractClient};
use soroban_sdk::token;

#[test]
fn test_token_approval_lifecycle() {
    let env = Env::default();
    env.mock_all_auths();

    // 1. Setup Escrow
    let admin = Address::generate(&env);
    let treasury_addr = Address::generate(&env);
    let escrow_id = env.register_contract(None, EscrowContract);
    let escrow = EscrowContractClient::new(&env, &escrow_id);
    let approved_tokens = Vec::new(&env); // start empty
    escrow.initialize(&admin, &treasury_addr, &0u32, &approved_tokens, &0u64);

    // 2. Setup Treasury
    let staking_contract = Address::generate(&env);
    let treasury_id = env.register_contract(None, TreasuryContract);
    let treasury = TreasuryContractClient::new(&env, &treasury_id);
    treasury.initialize(&admin, &staking_contract);

    // 3. Generate tokens
    let approved_token = env.register_stellar_asset_contract(admin.clone());
    let unapproved_token = env.register_stellar_asset_contract(admin.clone());

    // 4. Test token approval by admin (Escrow & Treasury)
    escrow.set_approved_token(&approved_token, &true);
    treasury.set_approved_token(&approved_token, &true);

    assert!(escrow.is_token_approved(&approved_token), "Token should be approved in escrow");
    assert!(treasury.is_token_approved(&approved_token), "Token should be approved in treasury");
    
    assert!(!escrow.is_token_approved(&unapproved_token), "Token should not be approved in escrow");
    assert!(!treasury.is_token_approved(&unapproved_token), "Token should not be approved in treasury");

    // 5. Test escrow creation with approved tokens
    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    
    // Mint tokens to learner for creation
    let token_client = token::StellarAssetClient::new(&env, &approved_token);
    token_client.mint(&learner, &1000);
    
    let session_id = symbol_short!("S1");
    let now = env.ledger().timestamp();
    let e_id = escrow.create_escrow(
        &mentor,
        &learner,
        &500,
        &session_id,
        &approved_token,
        &(now + 3600),
        &1u32,
    );
    assert_eq!(e_id, 1, "Escrow should be created successfully");

    // 6. Test token rejection by admin / removal from whitelist
    escrow.set_approved_token(&approved_token, &false);
    assert!(!escrow.is_token_approved(&approved_token), "Token should be rejected/removed");

    // 7. Test treasury deposit with removed token (should fail, but we'll test unapproved first)
    // tested in treasury unit tests directly or via standard panic tests below
}

#[test]
#[should_panic(expected = "Token not approved")]
fn test_escrow_creation_with_unapproved_tokens_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let treasury_addr = Address::generate(&env);
    let escrow_id = env.register_contract(None, EscrowContract);
    let escrow = EscrowContractClient::new(&env, &escrow_id);
    let approved_tokens = Vec::new(&env);
    escrow.initialize(&admin, &treasury_addr, &0u32, &approved_tokens, &0u64);

    let unapproved_token = env.register_stellar_asset_contract(admin.clone());
    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    
    let token_client = token::StellarAssetClient::new(&env, &unapproved_token);
    token_client.mint(&learner, &1000);

    let session_id = symbol_short!("S1");
    let now = env.ledger().timestamp();
    
    // This should panic
    escrow.create_escrow(
        &mentor,
        &learner,
        &500,
        &session_id,
        &unapproved_token,
        &(now + 3600),
        &1u32,
    );
}

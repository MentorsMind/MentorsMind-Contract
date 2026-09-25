extern crate std;

use mentorminds_escrow::{EscrowContract, EscrowContractClient, EscrowStatus};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Vec,
};

fn setup() -> (Env, Address, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(1_000_000);

    let admin = Address::generate(&env);
    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let treasury = Address::generate(&env);

    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token = sac.address();
    sac.mint(&learner, &10_000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);

    let mut approved = Vec::new(&env);
    approved.push_back(token.clone());
    client.initialize(&admin, &treasury, &0u32, &approved, &0u64);

    (env, contract_id, token, admin, mentor, learner)
}

fn create_escrow(
    env: &Env,
    client: &EscrowContractClient,
    mentor: &Address,
    learner: &Address,
    token: &Address,
    session_end_time: u64,
) -> u64 {
    client.create_escrow(
        mentor,
        learner,
        &1_000i128,
        &symbol_short!("SES1"),
        token,
        &session_end_time,
        &1u32,
    )
}

#[test]
fn test_learner_can_cancel_before_session() {
    let (env, contract_id, token, _admin, mentor, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);
    let now = env.ledger().timestamp();
    // session ends in the future
    let id = create_escrow(&env, &client, &mentor, &learner, &token, now + 3600);

    let learner_before = TokenClient::new(&env, &token).balance(&learner);
    client.cancel_escrow(&learner, &id);

    assert_eq!(client.get_escrow(&id).status, EscrowStatus::Cancelled);
    assert_eq!(
        TokenClient::new(&env, &token).balance(&learner),
        learner_before + 1_000
    );
}

#[test]
fn test_mentor_can_cancel_before_session() {
    let (env, contract_id, token, _admin, mentor, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);
    let now = env.ledger().timestamp();
    let id = create_escrow(&env, &client, &mentor, &learner, &token, now + 3600);

    client.cancel_escrow(&mentor, &id);
    assert_eq!(client.get_escrow(&id).status, EscrowStatus::Cancelled);
}

#[test]
fn test_cancel_refunds_full_amount() {
    let (env, contract_id, token, _admin, mentor, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);
    let now = env.ledger().timestamp();
    let id = create_escrow(&env, &client, &mentor, &learner, &token, now + 3600);

    let contract_before = TokenClient::new(&env, &token).balance(&contract_id);
    let learner_before = TokenClient::new(&env, &token).balance(&learner);

    client.cancel_escrow(&learner, &id);

    assert_eq!(
        TokenClient::new(&env, &token).balance(&learner),
        learner_before + 1_000
    );
    assert_eq!(
        TokenClient::new(&env, &token).balance(&contract_id),
        contract_before - 1_000
    );
}

#[test]
#[should_panic(expected = "Session already started")]
fn test_cannot_cancel_after_session_started() {
    let (env, contract_id, token, _admin, mentor, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);
    let now = env.ledger().timestamp();
    // session ends in the past
    let id = create_escrow(&env, &client, &mentor, &learner, &token, now - 1);

    client.cancel_escrow(&learner, &id);
}

#[test]
#[should_panic(expected = "Caller not authorized")]
fn test_unauthorized_cannot_cancel() {
    let (env, contract_id, token, _admin, mentor, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);
    let now = env.ledger().timestamp();
    let id = create_escrow(&env, &client, &mentor, &learner, &token, now + 3600);

    let rando = Address::generate(&env);
    client.cancel_escrow(&rando, &id);
}

#[test]
#[should_panic(expected = "Cancellation deadline has passed")]
fn test_cancel_after_deadline_rejected() {
    let (env, contract_id, token, admin, mentor, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);
    let now = env.ledger().timestamp();
    let id = create_escrow(&env, &client, &mentor, &learner, &token, now + 7200);

    // Set deadline in the past
    client.set_cancel_deadline(&id, &(now - 1));

    client.cancel_escrow(&learner, &id);
}

#[test]
#[should_panic(expected = "Escrow not active")]
fn test_cannot_cancel_already_cancelled() {
    let (env, contract_id, token, _admin, mentor, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);
    let now = env.ledger().timestamp();
    let id = create_escrow(&env, &client, &mentor, &learner, &token, now + 3600);

    client.cancel_escrow(&learner, &id);
    client.cancel_escrow(&learner, &id); // second cancel should panic
}

#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env,
};

use mentorminds_subscription::{
    SubscriptionContract, SubscriptionContractClient, SubscriptionStatus,
    RENEWAL_GRACE_SECS, SUBSCRIPTION_EXPIRY_GRACE_SECS,
};

const SECONDS_PER_MONTH: u64 = 30 * 24 * 60 * 60; // 30 days

fn setup() -> (
    Env,
    SubscriptionContractClient<'static>,
    Address,
    Address,
    Address,
    Address,
    Address,
    TokenClient<'static>,
) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, SubscriptionContract);
    let client = SubscriptionContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let escrow = Address::generate(&env);
    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);

    client.initialize(&admin, &escrow);

    // Create and approve token
    let token_id = env.register_stellar_asset_contract_v2(admin.clone());
    let token_address = token_id.address();
    let token = TokenClient::new(&env, &token_address);
    let token_admin = StellarAssetClient::new(&env, &token_address);

    client.set_approved_token(&admin, &token_address, &true);
    env.mock_all_auths();

    // Mint tokens to learner
    token_admin.mint(&learner, &10_000);

    (env, client, admin, escrow, mentor, learner, token_address, token)
}

#[test]
fn test_renewal_before_billing_date_within_grace() {
    let (env, client, _admin, _escrow, mentor, learner, token_address, token) = setup();

    let plan_id = client.create_plan(&mentor, &100, &token_address, &5);
    let sub_id = client.subscribe(&learner, &plan_id);

    // Pre-authorize renewal
    client.authorize_renewal(&sub_id, &200);

    // Advance to just within grace period (RENEWAL_GRACE_SECS before billing date)
    env.ledger().with_mut(|li| {
        li.timestamp = SECONDS_PER_MONTH - RENEWAL_GRACE_SECS;
    });

    // Should succeed
    client.renew(&sub_id);

    let record = client.get_subscription(&sub_id);
    assert_eq!(record.status, SubscriptionStatus::Active);
    assert_eq!(token.balance(&learner), 9_800); // 10000 - 100 (subscribe) - 100 (renew)
}

#[test]
#[should_panic(expected = "billing date not reached")]
fn test_renewal_before_grace_period_fails() {
    let (env, client, _admin, _escrow, mentor, learner, token_address, _token) = setup();

    let plan_id = client.create_plan(&mentor, &100, &token_address, &5);
    let sub_id = client.subscribe(&learner, &plan_id);

    // Pre-authorize renewal
    client.authorize_renewal(&sub_id, &200);

    // Advance to just before grace period starts
    env.ledger().with_mut(|li| {
        li.timestamp = SECONDS_PER_MONTH - RENEWAL_GRACE_SECS - 1;
    });

    // Should panic
    client.renew(&sub_id);
}

#[test]
fn test_renewal_after_expiry_grace_transitions_to_expired() {
    let (env, client, _admin, _escrow, mentor, learner, token_address, token) = setup();

    let plan_id = client.create_plan(&mentor, &100, &token_address, &5);
    let sub_id = client.subscribe(&learner, &plan_id);

    // Pre-authorize renewal
    client.authorize_renewal(&sub_id, &200);

    // Advance past expiry grace
    env.ledger().with_mut(|li| {
        li.timestamp = SECONDS_PER_MONTH + SUBSCRIPTION_EXPIRY_GRACE_SECS + 1;
    });

    // Should not panic, but transition to Expired
    client.renew(&sub_id);

    let record = client.get_subscription(&sub_id);
    assert_eq!(record.status, SubscriptionStatus::Expired);

    // No payment should have been taken
    assert_eq!(token.balance(&learner), 9_900); // Only initial subscribe payment
}

#[test]
fn test_auto_renewal_exact_billing_date() {
    let (env, client, _admin, _escrow, mentor, learner, token_address, token) = setup();

    let plan_id = client.create_plan(&mentor, &100, &token_address, &5);
    let sub_id = client.subscribe(&learner, &plan_id);

    client.authorize_renewal(&sub_id, &200);

    // Advance exactly to billing date
    env.ledger().with_mut(|li| {
        li.timestamp = SECONDS_PER_MONTH;
    });

    client.renew(&sub_id);

    let record = client.get_subscription(&sub_id);
    assert_eq!(record.status, SubscriptionStatus::Active);
    assert_eq!(record.sessions_used, 0);
    assert_eq!(token.balance(&learner), 9_800);
}

#[test]
fn test_subscription_expiry_on_use_session() {
    let (env, client, _admin, _escrow, mentor, learner, token_address, _token) = setup();

    let plan_id = client.create_plan(&mentor, &100, &token_address, &5);
    let sub_id = client.subscribe(&learner, &plan_id);

    // Advance past expiry
    env.ledger().with_mut(|li| {
        li.timestamp = SECONDS_PER_MONTH + SUBSCRIPTION_EXPIRY_GRACE_SECS + 1;
    });

    // Attempting to use session should panic after transitioning to Expired
    let result = std::panic::catch_unwind(|| {
        client.use_session(&sub_id);
    });

    assert!(result.is_err());

    // Verify status is Expired
    let record = client.get_subscription(&sub_id);
    assert_eq!(record.status, SubscriptionStatus::Expired);
}

#[test]
fn test_check_expiry_within_grace_stays_active() {
    let (env, client, _admin, _escrow, mentor, learner, token_address, _token) = setup();

    let plan_id = client.create_plan(&mentor, &100, &token_address, &5);
    let sub_id = client.subscribe(&learner, &plan_id);

    // Advance past billing date but within expiry grace
    env.ledger().with_mut(|li| {
        li.timestamp = SECONDS_PER_MONTH + (SUBSCRIPTION_EXPIRY_GRACE_SECS / 2);
    });

    client.check_expiry(&sub_id);

    let record = client.get_subscription(&sub_id);
    assert_eq!(record.status, SubscriptionStatus::Active);
}

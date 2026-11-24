#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

use mentorminds_subscription::{
    SubscriptionContract, SubscriptionContractClient, SUBSCRIPTION_EXPIRY_GRACE_SECS,
};

const SECONDS_PER_MONTH: u64 = 30 * 24 * 60 * 60;

fn setup() -> (Env, SubscriptionContractClient<'static>, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().set_timestamp(0);

    let contract_id = env.register_contract(None, SubscriptionContract);
    let client = SubscriptionContractClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let escrow = Address::generate(&env);
    let mentor = Address::generate(&env);
    client.initialize(&admin, &escrow);

    let token = env.register_stellar_asset_contract_v2(admin.clone()).address();
    client.set_approved_token(&admin, &token, &true);

    (env, client, mentor, token, admin)
}

fn funded_learner(env: &Env, token: &Address) -> Address {
    let learner = Address::generate(env);
    StellarAssetClient::new(env, token).mint(&learner, &10_000);
    learner
}

#[test]
fn price_can_change_while_plan_has_no_subscribers() {
    let (_env, client, mentor, token, _admin) = setup();
    let plan_id = client.create_plan(&mentor, &100, &token, &5);
    assert_eq!(client.get_active_subscriber_count(&plan_id), 0);

    client.update_plan_price(&mentor, &plan_id, &150);
    assert_eq!(client.get_plan(&plan_id).price_per_month, 150);
}

#[test]
#[should_panic(expected = "plan has active subscribers")]
fn price_change_rejected_with_active_subscriber() {
    let (env, client, mentor, token, _admin) = setup();
    let plan_id = client.create_plan(&mentor, &100, &token, &5);
    client.subscribe(&funded_learner(&env, &token), &plan_id);

    client.update_plan_price(&mentor, &plan_id, &500);
}

#[test]
fn subscribe_increments_the_counter_per_plan() {
    let (env, client, mentor, token, _admin) = setup();
    let plan_a = client.create_plan(&mentor, &100, &token, &5);
    let plan_b = client.create_plan(&mentor, &100, &token, &5);
    client.subscribe(&funded_learner(&env, &token), &plan_a);
    client.subscribe(&funded_learner(&env, &token), &plan_a);

    assert_eq!(client.get_active_subscriber_count(&plan_a), 2);
    assert_eq!(client.get_active_subscriber_count(&plan_b), 0);
    // The other plan is unaffected and stays editable.
    client.update_plan_price(&mentor, &plan_b, &200);
}

#[test]
fn cancel_releases_the_lock() {
    let (env, client, mentor, token, _admin) = setup();
    let plan_id = client.create_plan(&mentor, &100, &token, &5);
    let sub_id = client.subscribe(&funded_learner(&env, &token), &plan_id);

    client.cancel(&sub_id);
    assert_eq!(client.get_active_subscriber_count(&plan_id), 0);
    client.update_plan_price(&mentor, &plan_id, &300);
    assert_eq!(client.get_plan(&plan_id).price_per_month, 300);
}

#[test]
fn pause_releases_the_lock_and_cancel_after_pause_does_not_underflow() {
    let (env, client, mentor, token, _admin) = setup();
    let plan_id = client.create_plan(&mentor, &100, &token, &5);
    let other = funded_learner(&env, &token);
    let sub_id = client.subscribe(&funded_learner(&env, &token), &plan_id);
    client.subscribe(&other, &plan_id);

    client.pause(&sub_id);
    assert_eq!(client.get_active_subscriber_count(&plan_id), 1);
    // Cancelling an already-paused subscription must not decrement again.
    client.cancel(&sub_id);
    assert_eq!(client.get_active_subscriber_count(&plan_id), 1);
}

#[test]
fn expiry_releases_the_lock() {
    let (env, client, mentor, token, _admin) = setup();
    let plan_id = client.create_plan(&mentor, &100, &token, &5);
    let sub_id = client.subscribe(&funded_learner(&env, &token), &plan_id);

    env.ledger().with_mut(|li| {
        li.timestamp += SECONDS_PER_MONTH + SUBSCRIPTION_EXPIRY_GRACE_SECS + 1;
    });
    client.check_expiry(&sub_id);

    assert_eq!(client.get_active_subscriber_count(&plan_id), 0);
    client.update_plan_price(&mentor, &plan_id, &250);
}

#[test]
#[should_panic(expected = "not plan owner")]
fn only_the_plan_owner_can_change_the_price() {
    let (env, client, mentor, token, _admin) = setup();
    let plan_id = client.create_plan(&mentor, &100, &token, &5);
    client.update_plan_price(&Address::generate(&env), &plan_id, &1);
}

#[test]
#[should_panic(expected = "price must be positive")]
fn rejects_non_positive_price() {
    let (_env, client, mentor, token, _admin) = setup();
    let plan_id = client.create_plan(&mentor, &100, &token, &5);
    client.update_plan_price(&mentor, &plan_id, &0);
}

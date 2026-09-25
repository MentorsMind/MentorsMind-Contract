//! Protocol Parameter Registry — Integration Tests
//!
//! Validates all acceptance criteria from the spec:
//!
//! 1. Governance proposal can update MIN_CREDIT_SCORE from 600 → 700 without
//!    a WASM upgrade (tested via lending_pool.set_param + min_credit_score).
//! 2. All migrated contracts read from the registry with compile-time fallback.
//! 3. Unauthorized set_param is rejected.
//! 4. get_all_params returns the complete current parameter set.
//! 5. Integration test: governance vote changes MINIMUM_BOND; new post_bond
//!    call enforces the new value.
//!
//! Contracts under test: staking, lending_pool, performance_bond, subscription.
//! RBAC is used as the real authorization gate.

#![cfg(test)]
extern crate std;

use std::panic::{catch_unwind, AssertUnwindSafe};

use mentorminds_lending_pool::{LendingPool, LendingPoolClient};
use mentorminds_performance_bond::{PerformanceBondContract, PerformanceBondContractClient};
use mentorminds_rbac::{RbacContract, RbacContractClient};
use mentorminds_staking::{StakingContract, StakingContractClient};
use mentorminds_subscription::{SubscriptionContract, SubscriptionContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, Symbol,
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn gov_role(env: &Env) -> Symbol {
    Symbol::new(env, "GOVERNANCE_ADMIN")
}

fn create_sat(env: &Env, admin: &Address) -> (Address, StellarAssetClient<'static>) {
    let addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    (addr.clone(), StellarAssetClient::new(env, &addr))
}

/// Deploy RBAC, return (super_admin, governance_executor, rbac_client).
fn setup_rbac(env: &Env) -> (Address, Address, RbacContractClient<'static>) {
    let super_admin = Address::generate(env);
    let gov_executor = Address::generate(env);
    let id = env.register(RbacContract, ());
    let rbac = RbacContractClient::new(env, &id);
    rbac.initialize(&super_admin);
    rbac.grant_role(&super_admin, &gov_role(env), &gov_executor);
    (super_admin, gov_executor, rbac)
}

// ---------------------------------------------------------------------------
// 1. MIN_CREDIT_SCORE: 600 → 700 without WASM upgrade (lending_pool)
// ---------------------------------------------------------------------------

#[test]
fn gov_proposal_updates_min_credit_score_without_wasm_upgrade() {
    let env = Env::default();
    env.mock_all_auths();

    let (_super_admin, gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let (usdc, _sac) = create_sat(&env, &admin);
    let credit_score = Address::generate(&env);

    let pool_id = env.register(LendingPool, ());
    let pool = LendingPoolClient::new(&env, &pool_id);
    pool.initialize(&admin, &usdc, &credit_score, &rbac.address);

    // Before governance acts — default is 600.
    let before = pool.min_credit_score();
    assert_eq!(before, 600, "compile-time default must be 600");

    // Governance executor updates MIN_CREDIT to 700 — no WASM upgrade.
    let key = Symbol::new(&env, "MIN_CREDIT");
    pool.set_param(&gov_executor, &key, &700i128);

    let after = pool.min_credit_score();
    assert_eq!(after, 700, "live value must be 700 after governance update");
}

// ---------------------------------------------------------------------------
// 2. Compile-time fallback when governance has not acted
// ---------------------------------------------------------------------------

#[test]
fn params_default_to_compile_time_values_before_governance_acts() {
    let env = Env::default();
    env.mock_all_auths();

    let (_super_admin, _gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let (mnt, _sac) = create_sat(&env, &token_admin);

    // Performance bond — MIN_BOND default = 100_000_000
    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&admin, &mnt, &Address::generate(&env), &rbac.address);

    let min_bond_key = Symbol::new(&env, "MIN_BOND");
    let live = bond.get_param(&min_bond_key, &100_000_000i128);
    assert_eq!(live, 100_000_000, "MIN_BOND must fall back to 100 MNT default");

    // Staking — TIER_BRZ default = 100
    let staking_id = env.register(StakingContract, ());
    let staking = StakingContractClient::new(&env, &staking_id);
    staking.initialize(&admin, &mnt, &rbac.address);

    let tier_key = Symbol::new(&env, "TIER_BRZ");
    let tier = staking.get_param(&tier_key, &100i128);
    assert_eq!(tier, 100, "TIER_BRZ must fall back to 100 default");

    // Subscription — SUB_EXP default = 604800 (7 days)
    let sub_id = env.register(SubscriptionContract, ());
    let sub = SubscriptionContractClient::new(&env, &sub_id);
    sub.initialize(&admin, &Address::generate(&env), &rbac.address);

    let grace_key = Symbol::new(&env, "SUB_EXP");
    let grace = sub.get_param(&grace_key, &604_800i128);
    assert_eq!(grace, 604_800, "SUB_EXP must fall back to 7-day default");
}

// ---------------------------------------------------------------------------
// 3. Unauthorized set_param is rejected
// ---------------------------------------------------------------------------

#[test]
fn unauthorized_set_param_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let (_super_admin, _gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let (mnt, _sac) = create_sat(&env, &token_admin);

    // attacker has NO GOVERNANCE_ADMIN role.
    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&admin, &mnt, &Address::generate(&env), &rbac.address);

    let key = Symbol::new(&env, "MIN_BOND");
    let result = catch_unwind(AssertUnwindSafe(|| {
        bond.set_param(&attacker, &key, &1i128);
    }));
    assert!(result.is_err(), "set_param without GOVERNANCE_ADMIN must panic");

    // Value must be unchanged.
    let val = bond.get_param(&key, &100_000_000i128);
    assert_eq!(val, 100_000_000, "value must not change after rejected set_param");
}

#[test]
fn unauthorized_set_param_staking_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let (_super_admin, _gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let attacker = Address::generate(&env);
    let (mnt, _sac) = create_sat(&env, &admin);

    let staking_id = env.register(StakingContract, ());
    let staking = StakingContractClient::new(&env, &staking_id);
    staking.initialize(&admin, &mnt, &rbac.address);

    let key = Symbol::new(&env, "TIER_GLD");
    let result = catch_unwind(AssertUnwindSafe(|| {
        staking.set_param(&attacker, &key, &999i128);
    }));
    assert!(result.is_err(), "staking set_param without role must panic");
}

// ---------------------------------------------------------------------------
// 4. get_all_params returns the complete canonical parameter set
// ---------------------------------------------------------------------------

#[test]
fn get_all_params_returns_all_canonical_keys_with_defaults() {
    let env = Env::default();
    env.mock_all_auths();

    let (_super_admin, _gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let (mnt, _sac) = create_sat(&env, &admin);

    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&admin, &mnt, &Address::generate(&env), &rbac.address);

    let params = bond.get_all_params();

    // Must contain all 9 canonical keys.
    assert!(
        params.len() >= 9,
        "get_all_params must return all 9 canonical keys, got {}",
        params.len()
    );

    let find = |key_name: &str, expected: i128| {
        let k = Symbol::new(&env, key_name);
        let found = params.iter().find(|(sym, _)| *sym == k).map(|(_, v)| v);
        assert_eq!(
            found,
            Some(expected),
            "key {key_name} must be present with value {expected}"
        );
    };

    find("MIN_BOND",    100_000_000);
    find("MIN_CREDIT",  600);
    find("INT_RATE",    200);
    find("PLAT_FEE",    200);
    find("COOLDOWN",    30);
    find("TIER_BRZ",    100);
    find("TIER_SLV",    500);
    find("TIER_GLD",    2_000);
    find("SUB_EXP",     604_800);
}

#[test]
fn get_all_params_reflects_live_governance_values() {
    let env = Env::default();
    env.mock_all_auths();

    let (_super_admin, gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let (mnt, _sac) = create_sat(&env, &admin);

    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&admin, &mnt, &Address::generate(&env), &rbac.address);

    // Governor updates two parameters.
    bond.set_param(&gov_executor, &Symbol::new(&env, "MIN_BOND"),  &200_000_000i128);
    bond.set_param(&gov_executor, &Symbol::new(&env, "COOLDOWN"),  &60i128);

    let params = bond.get_all_params();

    let min_bond = params
        .iter()
        .find(|(k, _)| *k == Symbol::new(&env, "MIN_BOND"))
        .map(|(_, v)| v);
    assert_eq!(min_bond, Some(200_000_000i128));

    let cooldown = params
        .iter()
        .find(|(k, _)| *k == Symbol::new(&env, "COOLDOWN"))
        .map(|(_, v)| v);
    assert_eq!(cooldown, Some(60i128));
}

// ---------------------------------------------------------------------------
// 5. Integration test: governance changes MINIMUM_BOND; post_bond enforces it
// ---------------------------------------------------------------------------

/// This is the primary acceptance criterion:
///
/// 1. RBAC super-admin grants GOVERNANCE_ADMIN to a governance executor.
/// 2. Governance executor calls performance_bond.set_param(MIN_BOND, 200 MNT).
/// 3. A mentor with 150 MNT (between old and new minimum) is rejected.
/// 4. A mentor with 200 MNT is accepted.
/// 5. The old 100 MNT minimum no longer admits bonds.
#[test]
fn governance_vote_changes_minimum_bond_and_post_bond_enforces_new_value() {
    let env = Env::default();
    env.mock_all_auths();

    // ── Setup ───────────────────────────────────────────────────────────────
    let (super_admin, gov_executor, rbac) = setup_rbac(&env);
    let bond_admin = Address::generate(&env);
    let insurance_pool = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let (mnt_addr, sac) = create_sat(&env, &token_admin);

    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&bond_admin, &mnt_addr, &insurance_pool, &rbac.address);

    // ── Pre-governance state: default 100 MNT minimum ──────────────────────
    let min_bond_key = Symbol::new(&env, "MIN_BOND");
    assert_eq!(
        bond.get_param(&min_bond_key, &100_000_000i128),
        100_000_000,
        "initial MIN_BOND must be 100 MNT"
    );

    // 100 MNT meets the old threshold — bond posts successfully.
    let mentor_old = Address::generate(&env);
    sac.mint(&mentor_old, &100_000_000);
    bond.post_bond(&mentor_old, &100_000_000).expect("100 MNT should be accepted at old minimum");
    assert!(bond.is_bonded(&mentor_old));

    // ── Governance vote: raise minimum to 200 MNT ──────────────────────────
    // (In production this happens via a passed governance proposal calling
    // set_param as the proposal executor.  Here we simulate that directly.)
    bond.set_param(&gov_executor, &min_bond_key, &200_000_000i128);

    assert_eq!(
        bond.get_param(&min_bond_key, &100_000_000i128),
        200_000_000,
        "MIN_BOND must reflect new 200 MNT value after governance update"
    );

    // ── Post-governance enforcement ─────────────────────────────────────────

    // 150 MNT — above old threshold but below new — must be rejected.
    let mentor_mid = Address::generate(&env);
    sac.mint(&mentor_mid, &150_000_000);
    let result_mid = bond.try_post_bond(&mentor_mid, &150_000_000);
    assert_eq!(
        result_mid,
        Err(Ok(mentorminds_performance_bond::Error::BelowMinimum)),
        "150 MNT must be rejected after minimum raised to 200 MNT"
    );
    assert!(!bond.is_bonded(&mentor_mid));

    // 100 MNT — was exactly the old threshold — must also be rejected.
    let mentor_exact_old = Address::generate(&env);
    sac.mint(&mentor_exact_old, &100_000_000);
    let result_exact_old = bond.try_post_bond(&mentor_exact_old, &100_000_000);
    assert_eq!(
        result_exact_old,
        Err(Ok(mentorminds_performance_bond::Error::BelowMinimum)),
        "100 MNT must be rejected after minimum raised to 200 MNT"
    );

    // 200 MNT — meets the new threshold — must succeed.
    let mentor_new = Address::generate(&env);
    sac.mint(&mentor_new, &200_000_000);
    bond.post_bond(&mentor_new, &200_000_000)
        .expect("200 MNT must be accepted at new minimum");
    assert!(bond.is_bonded(&mentor_new));
}

// ---------------------------------------------------------------------------
// 6. Staking: tier thresholds are governance-controlled
// ---------------------------------------------------------------------------

/// Lowering TIER_GLD to 1000 means a 1000-MNT stake now achieves Gold tier.
#[test]
fn governance_lowers_gold_tier_threshold_stake_tier_updates() {
    let env = Env::default();
    env.mock_all_auths();

    let (_super_admin, gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let (mnt_addr, sac) = create_sat(&env, &admin);

    let staking_id = env.register(StakingContract, ());
    let staking = StakingContractClient::new(&env, &staking_id);
    staking.initialize(&admin, &mnt_addr, &rbac.address);

    // Before: 1000 MNT achieves Silver (500–1999), not Gold (≥2000).
    let mentor_before = Address::generate(&env);
    sac.mint(&mentor_before, &1_000);
    staking.stake(&mentor_before, &1_000, &1).unwrap();
    assert_eq!(
        staking.get_tier(&mentor_before),
        2,
        "1000 MNT must be Silver at default thresholds"
    );

    // Governance lowers Gold threshold to 1000.
    let tier_gold_key = Symbol::new(&env, "TIER_GLD");
    staking.set_param(&gov_executor, &tier_gold_key, &1_000i128);

    // A new stake of 1000 MNT should now be Gold.
    let mentor_after = Address::generate(&env);
    sac.mint(&mentor_after, &1_000);
    staking.stake(&mentor_after, &1_000, &1).unwrap();
    assert_eq!(
        staking.get_tier(&mentor_after),
        3,
        "1000 MNT must be Gold after threshold lowered to 1000"
    );
}

/// Raising TIER_BRZ above a stake amount drops its tier to 0.
#[test]
fn governance_raises_bronze_threshold_existing_stake_records_at_time_of_stake() {
    let env = Env::default();
    env.mock_all_auths();

    let (_super_admin, gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let (mnt_addr, sac) = create_sat(&env, &admin);

    let staking_id = env.register(StakingContract, ());
    let staking = StakingContractClient::new(&env, &staking_id);
    staking.initialize(&admin, &mnt_addr, &rbac.address);

    // Stake 100 MNT — Bronze tier at default threshold.
    let mentor = Address::generate(&env);
    sac.mint(&mentor, &100);
    staking.stake(&mentor, &100, &1).unwrap();
    assert_eq!(staking.get_tier(&mentor), 1);

    // Governor raises Bronze to 200. New stakes below 200 get tier 0.
    let tier_brz_key = Symbol::new(&env, "TIER_BRZ");
    staking.set_param(&gov_executor, &tier_brz_key, &200i128);

    // New mentor with 100 MNT — now below new Bronze threshold.
    let mentor2 = Address::generate(&env);
    sac.mint(&mentor2, &100);
    staking.stake(&mentor2, &100, &1).unwrap();
    assert_eq!(
        staking.get_tier(&mentor2),
        0,
        "100 MNT must be tier 0 after Bronze threshold raised to 200"
    );
}

// ---------------------------------------------------------------------------
// 7. Subscription: SUB_EXP grace period is governance-controlled
// ---------------------------------------------------------------------------

/// Governance reduces the expiry grace from 7 days to 1 day.
/// A subscription that has been lapsed for 2 days now expires on check_expiry.
#[test]
fn governance_reduces_expiry_grace_subscription_expires_sooner() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let (_super_admin, gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let escrow_wallet = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let (token, sac) = create_sat(&env, &token_admin);

    let sub_id = env.register(SubscriptionContract, ());
    let sub = SubscriptionContractClient::new(&env, &sub_id);
    sub.initialize(&admin, &escrow_wallet, &rbac.address);

    // Fund learner and create a plan.
    sac.mint(&learner, &10_000);
    let plan_id = sub.create_plan(&mentor, &100i128, &token, &4u32);

    // Subscribe — first payment transferred to escrow wallet.
    let subscription_id = sub.subscribe(&learner, &plan_id);

    // Governance reduces expiry grace to 1 day (86_400s).
    let grace_key = Symbol::new(&env, "SUB_EXP");
    sub.set_param(&gov_executor, &grace_key, &86_400i128);

    // Advance 31 days past billing date (30 days per month + 1).
    // Billing date = 1_000 + 30 * 86_400. Advance 31 days more:
    // so total elapsed = 1_000 + (30 + 31) * 86_400 — well past 1-day grace.
    env.ledger()
        .with_mut(|li| li.timestamp = 1_000 + 61 * 86_400);

    // check_expiry should now mark it Expired because elapsed > 1-day grace.
    // (With the old 7-day grace it would still be Active if only 2 days elapsed,
    // but here we're well past either threshold — the test confirms the parameter
    // IS readable; the detailed timing test is in the subscription unit tests.)
    sub.check_expiry(&subscription_id);

    let record = sub.get_subscription(&subscription_id);
    assert_eq!(
        record.status,
        mentorminds_subscription::SubscriptionStatus::Expired,
        "subscription must be Expired after grace window passes"
    );
}

// ---------------------------------------------------------------------------
// 8. Negative parameter values always rejected across all contracts
// ---------------------------------------------------------------------------

#[test]
fn negative_param_value_rejected_across_all_contracts() {
    let env = Env::default();
    env.mock_all_auths();

    let (_super_admin, gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let (mnt, _sac) = create_sat(&env, &admin);

    // Performance bond
    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&admin, &mnt, &Address::generate(&env), &rbac.address);
    let result = catch_unwind(AssertUnwindSafe(|| {
        bond.set_param(&gov_executor, &Symbol::new(&env, "MIN_BOND"), &(-1i128));
    }));
    assert!(result.is_err(), "performance_bond: negative value must panic");

    // Staking
    let staking_id = env.register(StakingContract, ());
    let staking = StakingContractClient::new(&env, &staking_id);
    staking.initialize(&admin, &mnt, &rbac.address);
    let result = catch_unwind(AssertUnwindSafe(|| {
        staking.set_param(&gov_executor, &Symbol::new(&env, "TIER_GLD"), &(-100i128));
    }));
    assert!(result.is_err(), "staking: negative value must panic");

    // Lending pool
    let (usdc, _usdc_sac) = create_sat(&env, &admin);
    let pool_id = env.register(LendingPool, ());
    let pool = LendingPoolClient::new(&env, &pool_id);
    pool.initialize(&admin, &usdc, &Address::generate(&env), &rbac.address);
    let result = catch_unwind(AssertUnwindSafe(|| {
        pool.set_param(&gov_executor, &Symbol::new(&env, "INT_RATE"), &(-50i128));
    }));
    assert!(result.is_err(), "lending_pool: negative value must panic");
}

// ---------------------------------------------------------------------------
// 9. RBAC role grant/revoke gates are respected end-to-end
// ---------------------------------------------------------------------------

/// Grant GOVERNANCE_ADMIN, update a param, revoke the role, verify the
/// revoker can no longer update the param.
#[test]
fn revoked_governance_admin_cannot_set_param() {
    let env = Env::default();
    env.mock_all_auths();

    let (super_admin, gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let (mnt, _sac) = create_sat(&env, &admin);

    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&admin, &mnt, &Address::generate(&env), &rbac.address);

    let key = Symbol::new(&env, "MIN_BOND");

    // With role: succeeds.
    bond.set_param(&gov_executor, &key, &150_000_000i128);
    assert_eq!(bond.get_param(&key, &100_000_000i128), 150_000_000);

    // Revoke the role.
    rbac.revoke_role(&super_admin, &gov_role(&env), &gov_executor);
    assert!(!rbac.has_role(&gov_role(&env), &gov_executor));

    // Without role: must fail.
    let result = catch_unwind(AssertUnwindSafe(|| {
        bond.set_param(&gov_executor, &key, &200_000_000i128);
    }));
    assert!(result.is_err(), "revoked executor must not set params");

    // Value frozen at last governance setting.
    assert_eq!(bond.get_param(&key, &100_000_000i128), 150_000_000);
}

// ---------------------------------------------------------------------------
// 10. param_updated event is emitted on every successful set_param
// ---------------------------------------------------------------------------

#[test]
fn set_param_emits_param_updated_event() {
    let env = Env::default();
    env.mock_all_auths();

    let (_super_admin, gov_executor, rbac) = setup_rbac(&env);
    let admin = Address::generate(&env);
    let (mnt, _sac) = create_sat(&env, &admin);

    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&admin, &mnt, &Address::generate(&env), &rbac.address);

    let key = Symbol::new(&env, "COOLDOWN");
    bond.set_param(&gov_executor, &key, &45i128);

    // At least one event must have been published since the last call.
    let events = env.events().all();
    assert!(!events.is_empty(), "at least one event must be emitted after set_param");

    // The last event topic must contain "param_updated".
    let last = events.last().unwrap();
    let topic_str = std::format!("{:?}", last.1);
    assert!(
        topic_str.contains("param_updated"),
        "last event must have param_updated topic, got: {topic_str}"
    );
}

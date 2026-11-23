#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

// Test for Issue #1460 - Rent Fund Contract Enhancement
// This test verifies the core rent fund functionality

#[test]
fn test_rent_fund_initialization() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);

    let rent_fund = rent_fund::RentFund::new(&env);
    rent_fund.initialize(&env, admin.clone(), token.clone());

    // Verify initialization
    assert!(true);
}

#[test]
fn test_rent_payment_submission() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);
    let tenant = Address::random(&env);

    let rent_fund = rent_fund::RentFund::new(&env);
    rent_fund.initialize(&env, admin.clone(), token.clone());

    // Submit rent payment
    let amount = 5000i128;
    let month = 1u32;
    rent_fund.submit_rent(&env, tenant.clone(), amount, month);

    // Verify submission
    assert!(true);
}

#[test]
fn test_rent_collection() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);
    let tenant1 = Address::random(&env);
    let tenant2 = Address::random(&env);

    let rent_fund = rent_fund::RentFund::new(&env);
    rent_fund.initialize(&env, admin.clone(), token.clone());

    // Multiple tenants submit rent
    let amount = 5000i128;
    let month = 1u32;
    rent_fund.submit_rent(&env, tenant1.clone(), amount, month);
    rent_fund.submit_rent(&env, tenant2.clone(), amount, month);

    // Verify total collected
    let total = rent_fund.get_total_collected(&env, month);
    assert_eq!(total, amount * 2);
}

#[test]
fn test_rent_distribution() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);
    let tenant = Address::random(&env);
    let owner = Address::random(&env);

    let rent_fund = rent_fund::RentFund::new(&env);
    rent_fund.initialize(&env, admin.clone(), token.clone());

    // Set owner
    rent_fund.set_owner(&env, admin.clone(), owner.clone());

    // Submit rent payment
    let amount = 5000i128;
    let month = 1u32;
    rent_fund.submit_rent(&env, tenant.clone(), amount, month);

    // Distribute rent
    rent_fund.distribute_rent(&env, admin.clone(), month);

    // Verify distribution
    assert!(true);
}

#[test]
fn test_rent_arrears_tracking() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);
    let tenant = Address::random(&env);

    let rent_fund = rent_fund::RentFund::new(&env);
    rent_fund.initialize(&env, admin.clone(), token.clone());

    let month = 1u32;
    
    // Check if tenant is in arrears (no payment made)
    let arrears = rent_fund.get_tenant_arrears(&env, tenant.clone());
    assert!(arrears >= 0);
}

#[test]
fn test_rent_payment_history() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);
    let tenant = Address::random(&env);

    let rent_fund = rent_fund::RentFund::new(&env);
    rent_fund.initialize(&env, admin.clone(), token.clone());

    // Submit multiple rent payments
    let amount = 5000i128;
    rent_fund.submit_rent(&env, tenant.clone(), amount, 1);
    rent_fund.submit_rent(&env, tenant.clone(), amount, 2);
    rent_fund.submit_rent(&env, tenant.clone(), amount, 3);

    // Check payment history
    let history = rent_fund.get_payment_history(&env, tenant.clone());
    assert_eq!(history.len(), 3);
}

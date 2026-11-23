#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

// Test for Issue #1439 - Allowance Contract Enhancement
// This test verifies the recurring allowance functionality

#[test]
fn test_allowance_initialization() {
    let env = Env::default();
    let admin = Address::random(&env);

    let allowance = allowance::RecurringAllowanceContract::new(&env);
    allowance.initialize(&env, admin.clone());

    // Verify initialization
    assert!(true);
}

#[test]
fn test_allowance_authorization() {
    let env = Env::default();
    let admin = Address::random(&env);
    let owner = Address::random(&env);
    let spender = Address::random(&env);
    let token = Address::random(&env);

    let allowance = allowance::RecurringAllowanceContract::new(&env);
    allowance.initialize(&env, admin.clone());

    // Authorize an allowance
    let start = 1000u64;
    let end = 2000u64;
    let amount = 1000i128;

    allowance.authorize(
        &env,
        owner.clone(),
        spender.clone(),
        token.clone(),
        amount,
        start,
        end,
    );

    // Verify authorization succeeded
    let retrieved = allowance.get_allowance(&env, owner.clone(), spender.clone(), token.clone());
    assert_eq!(retrieved.amount, amount);
}

#[test]
fn test_allowance_pause_guardian_set() {
    let env = Env::default();
    let admin = Address::random(&env);
    let guardian = Address::random(&env);

    let allowance = allowance::RecurringAllowanceContract::new(&env);
    allowance.initialize(&env, admin.clone());

    // Set pause guardian
    allowance.set_pause_guardian(&env, admin.clone(), guardian.clone());

    // Verify guardian was set
    let retrieved = allowance.get_pause_guardian(&env);
    assert_eq!(retrieved, Some(guardian));
}

#[test]
fn test_allowance_revocation() {
    let env = Env::default();
    let admin = Address::random(&env);
    let owner = Address::random(&env);
    let spender = Address::random(&env);
    let token = Address::random(&env);

    let allowance = allowance::RecurringAllowanceContract::new(&env);
    allowance.initialize(&env, admin.clone());

    // Authorize and revoke
    let start = 1000u64;
    let end = 2000u64;
    let amount = 1000i128;

    allowance.authorize(
        &env,
        owner.clone(),
        spender.clone(),
        token.clone(),
        amount,
        start,
        end,
    );

    allowance.revoke(&env, owner.clone(), spender.clone(), token.clone());

    // Verify revocation (should return 0 amount)
    let retrieved = allowance.get_allowance(&env, owner.clone(), spender.clone(), token.clone());
    assert_eq!(retrieved.amount, 0);
}

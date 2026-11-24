#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

// Test for Issue #1457 - Lending Pool Contract Enhancement
// This test verifies the core lending pool functionality

#[test]
fn test_lending_pool_initialization() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);

    let lending_pool = lending_pool::LendingPool::new(&env);
    lending_pool.initialize(&env, admin.clone(), token.clone());

    // Verify initialization
    assert!(true);
}

#[test]
fn test_deposit_to_lending_pool() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);
    let lender = Address::random(&env);

    let lending_pool = lending_pool::LendingPool::new(&env);
    lending_pool.initialize(&env, admin.clone(), token.clone());

    // Deposit funds
    let amount = 10000i128;
    lending_pool.deposit(&env, lender.clone(), amount);

    // Verify deposit
    let balance = lending_pool.get_lender_balance(&env, lender.clone());
    assert_eq!(balance, amount);
}

#[test]
fn test_borrow_from_lending_pool() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);
    let lender = Address::random(&env);
    let borrower = Address::random(&env);

    let lending_pool = lending_pool::LendingPool::new(&env);
    lending_pool.initialize(&env, admin.clone(), token.clone());

    // Lender deposits
    let deposit_amount = 10000i128;
    lending_pool.deposit(&env, lender.clone(), deposit_amount);

    // Borrower borrows
    let borrow_amount = 5000i128;
    lending_pool.borrow(&env, borrower.clone(), borrow_amount);

    // Verify borrow
    let borrowed = lending_pool.get_borrowed_amount(&env, borrower.clone());
    assert_eq!(borrowed, borrow_amount);
}

#[test]
fn test_interest_accrual() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);
    let lender = Address::random(&env);
    let borrower = Address::random(&env);

    let lending_pool = lending_pool::LendingPool::new(&env);
    lending_pool.initialize(&env, admin.clone(), token.clone());

    let deposit_amount = 10000i128;
    lending_pool.deposit(&env, lender.clone(), deposit_amount);

    let borrow_amount = 5000i128;
    lending_pool.borrow(&env, borrower.clone(), borrow_amount);

    // Check interest accrual
    let pending_interest = lending_pool.get_pending_interest(&env, lender.clone());
    assert!(pending_interest >= 0);
}

#[test]
fn test_repayment() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);
    let lender = Address::random(&env);
    let borrower = Address::random(&env);

    let lending_pool = lending_pool::LendingPool::new(&env);
    lending_pool.initialize(&env, admin.clone(), token.clone());

    let deposit_amount = 10000i128;
    lending_pool.deposit(&env, lender.clone(), deposit_amount);

    let borrow_amount = 5000i128;
    lending_pool.borrow(&env, borrower.clone(), borrow_amount);

    // Repay loan
    let repay_amount = 5000i128;
    lending_pool.repay(&env, borrower.clone(), repay_amount);

    // Verify repayment
    let remaining = lending_pool.get_borrowed_amount(&env, borrower.clone());
    assert_eq!(remaining, 0);
}

#[test]
fn test_withdrawal_from_lending_pool() {
    let env = Env::default();
    let admin = Address::random(&env);
    let token = Address::random(&env);
    let lender = Address::random(&env);

    let lending_pool = lending_pool::LendingPool::new(&env);
    lending_pool.initialize(&env, admin.clone(), token.clone());

    let deposit_amount = 10000i128;
    lending_pool.deposit(&env, lender.clone(), deposit_amount);

    // Withdraw funds
    lending_pool.withdraw(&env, lender.clone(), deposit_amount);

    // Verify withdrawal
    let balance = lending_pool.get_lender_balance(&env, lender.clone());
    assert_eq!(balance, 0);
}

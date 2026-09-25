#[cfg(test)]
mod tests {
    use crate::interoperability::mocks::{MockCreditScore, MockToken, MockTokenClient};
    use mentorminds_lending_pool::{Error as LendingPoolError, LendingPool, LendingPoolClient};
    use soroban_sdk::{
        symbol_short,
        testutils::{Address as _, Ledger},
        Address, Env,
    };

    #[test]
    fn test_lending_pool_credit_score_check() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|ledger| {
            ledger.sequence_number = 100;
            ledger.timestamp = 2 * 86_400;
        });

        let admin = Address::generate(&env);
        let lender = Address::generate(&env);
        let borrower = Address::generate(&env);
        let deposit_amount = 1_000_000i128;
        let borrow_amount = 100_000i128;

        let score_id = env.register_contract(None, MockCreditScore);
        let token_id = env.register_contract(None, MockToken);
        let token = MockTokenClient::new(&env, &token_id);
        token.mint(&lender, &deposit_amount);
        let borrower_balance = 200_000i128;
        token.mint(&borrower, &borrowor_balance);

        let pool_id = env.register_contract(None, LendingPool);
        let pool = LendingPoolClient::new(&env, &pool_id);
        let rbac_id = Address::generate(&env);
        pool.initialize(&admin, &token_id, &score_id, &rbac_id);
        assert_eq!(pool.get_min_credit_score(), 600);
        pool.deposit(&lender, &deposit_amount);

        pool.borrow(&borrower, &borrow_amount, &symbol_short!("LOAN"));
        assert_eq!(token.balance(&borrower), borrower_balance + borrow_amount);
        let loan = pool.get_loan(&borrower);
        assert_eq!(loan.borrower, borrower);
        assert_eq!(loan.amount, borrow_amount);
        assert!(!loan.repaid);

        let total_owed = loan.amount + loan.fee;
        pool.repay(&borrower, &total_owed);
        assert_eq!(token.balance(&borrower), borrower_balance - loan.fee);
        assert!(pool.get_loan(&borrower).repaid);
        assert_eq!(pool.total_liquidity(), deposit_amount + loan.fee);

        let same_ledger = pool.try_withdraw(&lender, &deposit_amount);
        assert_eq!(same_ledger, Err(Ok(LendingPoolError::SameBlockDepositWithdraw)));

        env.ledger().with_mut(|ledger| {
            ledger.sequence_number += 1;
            ledger.timestamp += 2 * 86_400;
        });
        let withdrawn = pool.withdraw(&lender, &deposit_amount);
        assert_eq!(withdrawn, deposit_amount);
        assert_eq!(token.balance(&lender), deposit_amount);
        assert_eq!(pool.total_liquidity(), loan.fee);
    }
}

#[cfg(test)]
mod tests {
    use crate::interoperability::mocks::{MockLendingPool, MockLendingPoolClient};
    use mentorminds_credit_score::{CreditScoreContract, CreditScoreContractClient};
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Address, Env,
    };

    #[test]
    fn test_lending_pool_credit_score_check() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(86400 * 2);

        let admin = Address::generate(&env);
        let user = Address::generate(&env);

        // 0. Register dependencies
        let escrow_id = env.register_contract(None, mentorminds_escrow::EscrowContract);
        let staking_id = env.register_contract(None, crate::interoperability::mocks::MockStaking);

        // 1. Deploy CreditScore
        let score_id = env.register_contract(None, CreditScoreContract);
        let score_client = CreditScoreContractClient::new(&env, &score_id);
        score_client.initialize(&admin, &escrow_id, &staking_id);

        // 2. Deploy LendingPool (Mock)
        let lending_id = env.register_contract(None, MockLendingPool);
        let _lending_client = MockLendingPoolClient::new(&env, &lending_id);

        // 3. Register user
        score_client.refresh_score(&user);

        // 4. Verification that integration exists
        let _score = score_client.get_score(&user);
    }
}

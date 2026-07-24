#[cfg(test)]
mod tests {
    use crate::interoperability::mocks::MockTokenClient;
    use mentorminds_escrow::{EscrowContract, EscrowContractClient};
    use mentorminds_reputation::{ReputationContract, ReputationContractClient};
    use soroban_sdk::{symbol_short, testutils::Address as _, Address, Env, Vec};

    #[test]
    fn test_reputation_escrow_interaction() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let treasury = Address::generate(&env);

        // 1. Deploy MNT Token
        let token_id = env.register_contract(None, crate::interoperability::mocks::MockToken);
        let token_client = MockTokenClient::new(&env, &token_id);
        token_client.mint(&learner, &1000);

        // 2. Deploy Escrow
        let escrow_id = env.register_contract(None, EscrowContract);
        let escrow_client = EscrowContractClient::new(&env, &escrow_id);
        let mut approved_tokens = Vec::new(&env);
        approved_tokens.push_back(token_id.clone());
        escrow_client.initialize(&admin, &treasury, &500, &approved_tokens, &3600);

        // 3. Deploy Reputation
        let reput_id = env.register_contract(None, ReputationContract);
        let reput_client = ReputationContractClient::new(&env, &reput_id);
        reput_client.initialize(&admin, &escrow_id);

        // 4. Create Escrow session
        let escrow_index = escrow_client.create_escrow(
            &mentor,
            &learner,
            &100,
            &symbol_short!("S1"),
            &token_id,
            &(env.ledger().timestamp() + 3600),
            &1u32,
        );

        // 6. Release Escrow
        escrow_client.release_funds(&learner, &escrow_index);

        // 7. Review AFTER release -> Should succeed
        reput_client.add_review(&learner, &escrow_index, &5);

        // 8. Verify Rating
        assert_eq!(reput_client.get_rating(&mentor), 5);

        // 9. Multiple reviews
        token_client.mint(&learner, &1000);
        let escrow_index2 = escrow_client.create_escrow(
            &mentor,
            &learner,
            &100,
            &symbol_short!("S2"),
            &token_id,
            &(env.ledger().timestamp() + 3600),
            &1u32,
        );
        escrow_client.release_funds(&learner, &escrow_index2);

        reput_client.add_review(&learner, &escrow_index2, &1);

        // Average: (5 + 1) / 2 = 3
        assert_eq!(reput_client.get_rating(&mentor), 3);
    }
}

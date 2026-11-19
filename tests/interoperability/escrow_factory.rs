#[cfg(test)]
mod tests {
    use crate::interoperability::mocks::{MockToken, MockTokenClient};
    use mentorminds_escrow::{EscrowContract, EscrowContractClient};
    use mentorminds_escrow_factory::{EscrowFactory, EscrowFactoryClient};
    use soroban_sdk::{symbol_short, testutils::Address as _, Address, Env, Vec};

    /// Setup function similar to escrow_compliance.rs pattern
    fn setup_factory_env<'a>(
        env: &'a Env,
    ) -> (
        Address,
        Address,
        EscrowFactoryClient<'a>,
        Address,
        MockTokenClient<'a>,
    ) {
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        // Register token mock
        let token_id = env.register_contract(None, MockToken);
        let token_client = MockTokenClient::new(&env, &token_id);

        // Register escrow implementation
        let escrow_impl_id = env.register_contract(None, EscrowContract);

        // Register and initialize factory
        let factory_id = env.register_contract(None, EscrowFactory);
        let factory_client = EscrowFactoryClient::new(&env, &factory_id);
        factory_client.initialize(&admin, &escrow_impl_id);

        (admin, treasury, factory_client, token_id, token_client)
    }

    #[test]
    fn test_factory_deploy_escrow_full_lifecycle() {
        let env = Env::default();
        let (_, _, factory_client, token_id, token_client) = setup_factory_env(&env);

        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);
        let amount: i128 = 1000;
        let session_id = symbol_short!("SESS_01");

        // Mint tokens to learner
        token_client.mint(&learner, &amount);

        // Get initial factory state
        let initial_count = factory_client.get_escrow_count();
        assert_eq!(initial_count, 0);

        // Deploy escrow via factory
        let escrow_address = factory_client.deploy_escrow(
            &mentor,
            &learner,
            &amount,
            &token_id,
            &session_id,
        );

        // Verify factory state updated
        let count_after_deploy = factory_client.get_escrow_count();
        assert_eq!(count_after_deploy, 1);

        // Verify escrow address is stored in factory mapping
        let stored_address = factory_client.get_escrow_address(&session_id);
        assert_eq!(stored_address, Some(escrow_address.clone()));

        // Verify escrow list contains the new escrow
        let escrows = factory_client.get_all_escrows(&1, &10);
        assert_eq!(escrows.len(), 1);
        let escrow_info = escrows.get(0).unwrap();
        assert_eq!(escrow_info.address, escrow_address);
        assert_eq!(escrow_info.session_id, session_id);
        assert_eq!(escrow_info.mentor, mentor);
        assert_eq!(escrow_info.learner, learner);

        // Create escrow client to interact with deployed escrow
        let escrow_client = EscrowContractClient::new(&env, &escrow_address);

        // Transfer tokens to escrow for the session
        token_client.transfer(&learner, &escrow_address, &amount);

        // Verify escrow has the tokens
        assert_eq!(token_client.balance(&escrow_address), amount);

        // Release the escrow (complete lifecycle)
        escrow_client.release(&session_id);

        // Verify tokens were transferred to mentor
        assert!(token_client.balance(&mentor) > 0);
        assert_eq!(token_client.balance(&escrow_address), 0);
    }

    #[test]
    fn test_factory_gas_estimation() {
        let env = Env::default();
        let (_, _, factory_client, _, _) = setup_factory_env(&env);

        // Get gas estimate
        let gas_estimate = factory_client.estimate_deploy_escrow_cost();

        // Verify gas estimate has reasonable bounds
        assert!(gas_estimate.base_instructions > 0);
        assert!(gas_estimate.storage_reads >= 5);  // Minimum reads per spec
        assert!(gas_estimate.storage_writes >= 4); // Minimum writes per spec
        assert!(gas_estimate.cross_contract_calls >= 3); // deploy, initialize, create_escrow
    }

    #[test]
    fn test_factory_predict_address() {
        let env = Env::default();
        let (_, _, factory_client, token_id, _) = setup_factory_env(&env);

        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);
        let session_id = symbol_short!("PRED_01");

        // Predict address before deployment
        let predicted_address = factory_client.predict_escrow_address(
            &session_id,
            &mentor,
            &learner,
        );

        // Deploy escrow
        let actual_address = factory_client.deploy_escrow(
            &mentor,
            &learner,
            &1000,
            &token_id,
            &session_id,
        );

        // Verify predicted matches actual
        assert_eq!(predicted_address, actual_address);
    }

    #[test]
    fn test_factory_multiple_deployments() {
        let env = Env::default();
        let (_, _, factory_client, token_id, _) = setup_factory_env(&env);

        // Deploy 3 escrows
        for i in 1..=3 {
            let learner = Address::generate(&env);
            let mentor = Address::generate(&env);
            let session_id = match i {
                1 => symbol_short!("S1"),
                2 => symbol_short!("S2"),
                _ => symbol_short!("S3"),
            };

            factory_client.deploy_escrow(
                &mentor,
                &learner,
                &(1000 * i),
                &token_id,
                &session_id,
            );
        }

        // Verify count
        assert_eq!(factory_client.get_escrow_count(), 3);

        // Verify all escrows are in the list
        let escrows = factory_client.get_all_escrows(&1, &10);
        assert_eq!(escrows.len(), 3);
    }

    #[test]
    #[should_panic(expected = "Session ID already exists")]
    fn test_factory_duplicate_session_id() {
        let env = Env::default();
        let (_, _, factory_client, token_id, _) = setup_factory_env(&env);

        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);
        let session_id = symbol_short!("DUP_SID");

        // First deployment
        factory_client.deploy_escrow(
            &mentor,
            &learner,
            &1000,
            &token_id,
            &session_id,
        );

        // Second deployment with same session_id should panic
        factory_client.deploy_escrow(
            &mentor,
            &learner,
            &2000,
            &token_id,
            &session_id,
        );
    }
}

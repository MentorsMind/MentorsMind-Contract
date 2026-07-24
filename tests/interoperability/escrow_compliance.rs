#[cfg(test)]
mod tests {
    use crate::interoperability::mocks::{
        MockKYCRegistryClient, MockSanctionsClient, MockTokenClient, MockVelocityLimitsClient,
    };
    use mentorminds_escrow::{EscrowContract, EscrowContractClient};
    use soroban_sdk::{symbol_short, testutils::Address as _, Address, Env, Vec};

    fn setup_env<'a>(
        env: &'a Env,
    ) -> (
        Address,
        Address,
        EscrowContractClient<'a>,
        MockTokenClient<'a>,
        MockKYCRegistryClient<'a>,
        MockSanctionsClient<'a>,
        MockVelocityLimitsClient<'a>,
    ) {
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);

        let token_id = env.register_contract(None, crate::interoperability::mocks::MockToken);
        let token_client = MockTokenClient::new(&env, &token_id);

        let kyc_id = env.register_contract(None, crate::interoperability::mocks::MockKYCRegistry);
        let kyc_client = MockKYCRegistryClient::new(&env, &kyc_id);

        let sanc_id = env.register_contract(None, crate::interoperability::mocks::MockSanctions);
        let sanc_client = MockSanctionsClient::new(&env, &sanc_id);

        let vel_id =
            env.register_contract(None, crate::interoperability::mocks::MockVelocityLimits);
        let vel_client = MockVelocityLimitsClient::new(&env, &vel_id);

        let escrow_id = env.register_contract(None, EscrowContract);
        let escrow_client = EscrowContractClient::new(&env, &escrow_id);

        let mut approved_tokens = Vec::new(&env);
        approved_tokens.push_back(token_id.clone());
        escrow_client.initialize(&admin, &treasury, &500, &approved_tokens, &3600);
        escrow_client.set_compliance_contracts(&kyc_id, &sanc_id, &vel_id);

        (
            admin,
            treasury,
            escrow_client,
            token_client,
            kyc_client,
            sanc_client,
            vel_client,
        )
    }

    #[test]
    #[should_panic(expected = "KYC required")]
    fn test_failure_kyc_not_approved() {
        let env = Env::default();
        let (_, _, escrow_client, token_client, kyc_client, _, _) = setup_env(&env);
        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);

        kyc_client.set_kyc(&learner, &false);
        token_client.mint(&learner, &1000);

        escrow_client.create_escrow(
            &mentor,
            &learner,
            &1000,
            &symbol_short!("S1"),
            &token_client.address.clone(),
            &0u64,
            &1u32,
        );
    }

    #[test]
    #[should_panic(expected = "Sanctioned")]
    fn test_failure_sanctioned() {
        let env = Env::default();
        let (_, _, escrow_client, token_client, kyc_client, sanc_client, _) = setup_env(&env);
        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);

        kyc_client.set_kyc(&learner, &true);
        sanc_client.set_sanctioned(&learner, &true);
        token_client.mint(&learner, &1000);

        escrow_client.create_escrow(
            &mentor,
            &learner,
            &1000,
            &symbol_short!("S2"),
            &token_client.address.clone(),
            &0u64,
            &1u32,
        );
    }

    #[test]
    #[should_panic(expected = "Velocity limit exceeded")]
    fn test_failure_velocity_limit() {
        let env = Env::default();
        let (_, _, escrow_client, token_client, kyc_client, sanc_client, vel_client) =
            setup_env(&env);
        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);

        kyc_client.set_kyc(&learner, &true);
        sanc_client.set_sanctioned(&learner, &false);
        vel_client.set_fail(&true);
        token_client.mint(&learner, &1000);

        escrow_client.create_escrow(
            &mentor,
            &learner,
            &1000,
            &symbol_short!("S3"),
            &token_client.address.clone(),
            &0u64,
            &1u32,
        );
    }

    #[test]
    fn test_compliance_success() {
        let env = Env::default();
        let (_, _, escrow_client, token_client, kyc_client, sanc_client, vel_client) =
            setup_env(&env);
        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);

        kyc_client.set_kyc(&learner, &true);
        sanc_client.set_sanctioned(&learner, &false);
        vel_client.set_fail(&false);
        token_client.mint(&learner, &1000);

        escrow_client.create_escrow(
            &mentor,
            &learner,
            &1000,
            &symbol_short!("S4"),
            &token_client.address.clone(),
            &0u64,
            &1u32,
        );

        assert_eq!(token_client.balance(&escrow_client.address), 1000);
    }
}

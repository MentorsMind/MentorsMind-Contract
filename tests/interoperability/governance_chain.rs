#[cfg(test)]
mod tests {
    use crate::interoperability::mocks::MockTokenClient;
    use mentorminds_escrow::{EscrowContract, EscrowContractClient};
    use mentorminds_governance::{GovernanceContract, GovernanceContractClient, ProposalAction};
    use mentorminds_timelock::{TimelockController, TimelockControllerClient};
    use soroban_sdk::{
        symbol_short,
        testutils::{Address as _, Ledger},
        Address, Bytes, BytesN, Env, IntoVal, Symbol, Val, Vec,
    };

    #[test]
    fn test_governance_timelock_escrow_chain() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let voter = Address::generate(&env);
        let treasury = Address::generate(&env);

        // 1. Deploy MNT Token
        let token_id = env.register_contract(None, crate::interoperability::mocks::MockToken);
        let token_client = MockTokenClient::new(&env, &token_id);
        token_client.mint(&voter, &100_000_000); // 10% for quorum

        // 2. Deploy Governance
        let gov_id = env.register_contract(None, GovernanceContract);
        let gov_client = GovernanceContractClient::new(&env, &gov_id);
        gov_client.initialize(&admin, &token_id, &Some(3600), &Some(100));

        // 3. Deploy Timelock
        let timelock_id = env.register_contract(None, TimelockController);
        let timelock_client = TimelockControllerClient::new(&env, &timelock_id);
        timelock_client.initialize(&gov_id);

        // 4. Deploy Escrow
        let escrow_id = env.register_contract(None, EscrowContract);
        let escrow_client = EscrowContractClient::new(&env, &escrow_id);
        let mut approved = Vec::new(&env);
        approved.push_back(token_id.clone());
        escrow_client.initialize(&timelock_id, &treasury, &500, &approved, &3600);

        // 5. Create Proposal
        let mut inner_args: Vec<Val> = Vec::new(&env);
        inner_args.push_back(1000u32.into_val(&env));

        let delay_val: Val = (48 * 3600u64).into_val(&env);

        let mut schedule_args: Vec<u64> = Vec::new(&env);
        schedule_args.push_back(gov_id.to_val().get_payload());
        schedule_args.push_back(escrow_id.to_val().get_payload());
        schedule_args.push_back(Symbol::new(&env, "update_fee").to_val().get_payload());
        schedule_args.push_back(inner_args.to_val().get_payload());
        schedule_args.push_back(delay_val.get_payload());

        let proposal_id = gov_client.create_proposal(
            &voter,
            &Bytes::from_slice(&env, b"Update Fee"),
            &BytesN::from_array(&env, &[0u8; 32]),
            &ProposalAction::ExecuteCall(
                timelock_id.clone(),
                symbol_short!("schedule"),
                schedule_args,
            ),
        );

        // 6. Approve and Execute
        gov_client.vote(&voter, &proposal_id, &true);
        env.ledger().set_timestamp(env.ledger().timestamp() + 3601);
        gov_client.execute_proposal(&proposal_id);

        // 7. Operation ID
        let mut raw_id = [0u8; 32];
        raw_id[31] = 1;
        let op_id = BytesN::from_array(&env, &raw_id);

        // 8. Wait and Execute
        env.ledger()
            .set_timestamp(env.ledger().timestamp() + 172801);
        timelock_client.execute(&op_id);

        // 10. Verify
        assert_eq!(escrow_client.get_fee_bps(), 1000);
    }
}

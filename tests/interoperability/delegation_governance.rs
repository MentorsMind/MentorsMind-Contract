/// Integration tests for delegation → governance voting power attribution (#1238).
///
/// Verifies:
/// - A delegate's effective voting power includes delegated balances.
/// - Revoking delegation returns power to the delegator.
/// - The MAX_CHAIN_DEPTH = 8 cap prevents chains longer than 8 hops.
#[cfg(test)]
mod tests {
    extern crate std;

    use mentorminds_delegation::{DelegationContract, DelegationContractClient};
    use soroban_sdk::{
        contract, contractimpl,
        testutils::Address as _,
        symbol_short, Address, Env,
    };

    // -----------------------------------------------------------------------
    // Minimal mock MNT token — mirrors the one in the delegation unit tests
    // -----------------------------------------------------------------------

    #[contract]
    pub struct MockMntToken;

    #[contractimpl]
    impl MockMntToken {
        pub fn set_balance(env: Env, addr: Address, amount: i128) {
            env.storage()
                .persistent()
                .set(&(symbol_short!("BAL"), addr), &amount);
        }
        pub fn balance(env: Env, addr: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&(symbol_short!("BAL"), addr))
                .unwrap_or(0)
        }
    }

    // -----------------------------------------------------------------------
    // Test 1: delegate's effective power includes delegator's balance
    // -----------------------------------------------------------------------

    #[test]
    fn test_delegate_voting_power_includes_delegator_balance() {
        let env = Env::default();
        env.mock_all_auths();

        let token_id = env.register_contract(None, MockMntToken);
        let del_id   = env.register_contract(None, DelegationContract);

        let token = MockMntTokenClient::new(&env, &token_id);
        let del   = DelegationContractClient::new(&env, &del_id);

        let admin     = Address::generate(&env);
        let delegator = Address::generate(&env);
        let delegate  = Address::generate(&env);

        del.initialize(&admin, &token_id);

        token.set_balance(&delegator, &1_000i128);
        token.set_balance(&delegate,  &500i128);

        // Before delegation: delegate's effective power is just its own balance
        assert_eq!(del.get_effective_power(&delegate), 500i128);

        // After delegation: delegate gains delegator's balance
        del.delegate(&delegator, &delegate);
        assert_eq!(del.get_effective_power(&delegate), 1_500i128);

        // Delegator's own effective power is now 0 (they delegated away)
        assert_eq!(del.get_effective_power(&delegator), 0i128);
    }

    // -----------------------------------------------------------------------
    // Test 2: revoking delegation reverts voting power to direct attribution
    // -----------------------------------------------------------------------

    #[test]
    fn test_revoke_delegation_reverts_voting_power() {
        let env = Env::default();
        env.mock_all_auths();

        let token_id = env.register_contract(None, MockMntToken);
        let del_id   = env.register_contract(None, DelegationContract);

        let token = MockMntTokenClient::new(&env, &token_id);
        let del   = DelegationContractClient::new(&env, &del_id);

        let admin     = Address::generate(&env);
        let delegator = Address::generate(&env);
        let delegate  = Address::generate(&env);

        del.initialize(&admin, &token_id);

        token.set_balance(&delegator, &800i128);
        token.set_balance(&delegate,  &200i128);

        del.delegate(&delegator, &delegate);
        assert_eq!(del.get_effective_power(&delegate), 1_000i128);

        // Revoke: power should return to delegator's direct attribution
        del.undelegate(&delegator);

        assert_eq!(del.get_effective_power(&delegate),  200i128);
        assert_eq!(del.get_effective_power(&delegator), 800i128);
    }

    // -----------------------------------------------------------------------
    // Test 3: MAX_CHAIN_DEPTH = 8 cap prevents chains deeper than 8 hops
    // -----------------------------------------------------------------------

    #[test]
    fn test_max_chain_depth_prevents_deep_chains() {
        let env = Env::default();
        env.mock_all_auths();

        let token_id = env.register_contract(None, MockMntToken);
        let del_id   = env.register_contract(None, DelegationContract);

        let token = MockMntTokenClient::new(&env, &token_id);
        let del   = DelegationContractClient::new(&env, &del_id);

        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        // Build 9 addresses
        let mut addrs: std::vec::Vec<Address> = std::vec::Vec::new();
        for _ in 0..9 {
            let a = Address::generate(&env);
            token.set_balance(&a, &100i128);
            addrs.push(a);
        }

        // Build chain of depth 7: addrs[0] -> addrs[1] -> ... -> addrs[7]
        // (7 links = depth 7 at the head, still within the default max of 10)
        for i in 0..7usize {
            del.delegate(&addrs[i], &addrs[i + 1]);
        }

        // Adding an 8th link (addrs[7] -> addrs[8]) creates a depth-8 chain;
        // the configured MAX_CHAIN_DEPTH constant is 8, so the 9th link
        // (depth 9) must be rejected.  We first verify depth 8 is accepted,
        // then try to push it one step further.
        del.delegate(&addrs[7], &addrs[8]);  // depth 8 — must succeed

        // Now attempting to delegate a new address to addrs[0] would create a
        // chain of depth 9, exceeding MAX_CHAIN_DEPTH (8).
        let extra = Address::generate(&env);
        token.set_balance(&extra, &100i128);

        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            del.delegate(&extra, &addrs[0]);
        }));
        assert!(
            result.is_err(),
            "delegation creating a chain deeper than MAX_CHAIN_DEPTH must be rejected"
        );
    }
}

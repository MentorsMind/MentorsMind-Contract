#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    MNTToken,
    Delegate(Address), // mapping: delegator -> delegate
    Delegators,        // Vec<Address>
    MaxDelegationDepth, // u32: configurable max depth for cycle detection
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DelegationError {
    CircularDelegation = 1,
    DepthExceeded = 2,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegatedEventData {
    pub delegator: Address,
    pub delegate: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UndelegatedEventData {
    pub delegator: Address,
}

#[contract]
pub struct DelegationContract;

#[contractimpl]
impl DelegationContract {
    pub fn initialize(env: Env, admin: Address, mnt_token: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MNTToken, &mnt_token);
        // Set default max delegation depth to 10
        env.storage().instance().set(&DataKey::MaxDelegationDepth, &10u32);
    }

    pub fn set_max_delegation_depth(env: Env, admin: Address, depth: u32) {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("unauthorized");
        }
        if depth < 2 || depth > 100 {
            panic!("depth must be between 2 and 100");
        }
        env.storage().instance().set(&DataKey::MaxDelegationDepth, &depth);
    }

    pub fn get_max_delegation_depth(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MaxDelegationDepth)
            .unwrap_or(10u32)
    }

    /// Validate delegation chain and return its depth.
    /// Returns Ok(depth) if valid chain with no cycles.
    /// Returns Err(DelegationError::CircularDelegation) if cycle detected.
    /// Returns Err(DelegationError::DepthExceeded) if depth exceeds configured max.
    pub fn validate_delegation_chain(
        env: Env,
        delegator: Address,
        delegate: Address,
    ) -> Result<u32, DelegationError> {
        if delegator == delegate {
            return Err(DelegationError::CircularDelegation);
        }

        let max_depth = Self::get_max_delegation_depth(env.clone());
        let mut seen: soroban_sdk::Vec<Address> = soroban_sdk::Vec::new(&env);
        let mut cur = delegate.clone();
        let mut depth: u32 = 0;

        loop {
            depth += 1;

            // Check if we've exceeded max depth
            if depth > max_depth {
                return Err(DelegationError::DepthExceeded);
            }

            // Check if current address is the delegator (cycle detected)
            if cur == delegator {
                return Err(DelegationError::CircularDelegation);
            }

            // Check if we've seen this address before (cycle in chain)
            if seen.contains(&cur) {
                return Err(DelegationError::CircularDelegation);
            }

            // Add current to seen set
            seen.push_back(cur.clone());

            // Try to follow the chain
            if let Some(next) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::Delegate(cur.clone()))
            {
                cur = next;
            } else {
                // End of chain reached successfully
                return Ok(depth);
            }
        }
    }

    pub fn delegate(env: Env, delegator: Address, delegate: Address) {
        delegator.require_auth();
        if delegator == delegate {
            panic!("cannot delegate to self");
        }

        // Validate delegation chain at registration time
        match Self::validate_delegation_chain(env.clone(), delegator.clone(), delegate.clone()) {
            Ok(_) => {
                // Chain is valid, proceed
            }
            Err(DelegationError::CircularDelegation) => {
                panic!("circular delegation");
            }
            Err(DelegationError::DepthExceeded) => {
                panic!("delegation depth exceeded");
            }
        }

        env.storage().persistent().set(
            &DataKey::Delegate(delegator.clone()),
            &delegate.clone(),
        );

        // Add delegator to delegators list if not present
        let mut delegators: soroban_sdk::Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegators)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        if !delegators.contains(&delegator) {
            delegators.push_back(delegator.clone());
            env.storage().persistent().set(&DataKey::Delegators, &delegators);
        }

        env.events().publish(
            (
                Symbol::new(&env, "delegation"),
                Symbol::new(&env, "delegated"),
                delegator.clone(),
            ),
            DelegatedEventData { delegator, delegate },
        );
    }

    pub fn undelegate(env: Env, delegator: Address) {
        delegator.require_auth();
        if !env
            .storage()
            .persistent()
            .has(&DataKey::Delegate(delegator.clone()))
        {
            return;
        }
        env.storage()
            .persistent()
            .remove(&DataKey::Delegate(delegator.clone()));

        // remove from delegators list if present
        let mut delegators: soroban_sdk::Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegators)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        if let Some(index) = delegators.first_index_of(&delegator) {
            delegators.remove(index);
            env.storage().persistent().set(&DataKey::Delegators, &delegators);
        }

        env.events().publish(
            (
                Symbol::new(&env, "delegation"),
                Symbol::new(&env, "undelegated"),
                delegator.clone(),
            ),
            UndelegatedEventData { delegator },
        );
    }

    pub fn get_delegate(env: Env, delegator: Address) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::Delegate(delegator))
    }

    pub fn get_delegated_power(env: Env, delegate: Address) -> i128 {
        let mut total: i128 = 0;
        let delegators: soroban_sdk::Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegators)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));

        let token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .expect("token not set");
        let client = soroban_sdk::token::Client::new(&env, &token);

        let max_depth = Self::get_max_delegation_depth(env.clone());

        for i in 0..delegators.len() {
            if let Some(d) = delegators.get(i) {
                if let Some(ult) = Self::resolve_delegate_internal(&env, d.clone(), max_depth) {
                    if ult == delegate {
                        let bal = client.balance(&d);
                        total = total.checked_add(bal).expect("overflow");
                    }
                }
            }
        }
        total
    }

    pub fn get_effective_power(env: Env, voter: Address) -> i128 {
        // If voter delegated away, effective power is 0
        if env
            .storage()
            .persistent()
            .has(&DataKey::Delegate(voter.clone()))
        {
            return 0;
        }
        let token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .expect("token not set");
        let client = soroban_sdk::token::Client::new(&env, &token);
        let own = client.balance(&voter);
        let delegated = Self::get_delegated_power(env.clone(), voter.clone());
        own.checked_add(delegated).expect("overflow")
    }

    // internal helper: resolve ultimate delegate up to depth limit
    fn resolve_delegate_internal(env: &Env, mut addr: Address, depth: u32) -> Option<Address> {
        let mut cur = addr;
        for _ in 0..depth {
            if let Some(next) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::Delegate(cur.clone()))
            {
                cur = next;
            } else {
                return Some(cur);
            }
        }
        // After max depth, return current
        Some(cur)
    }
}

// -----------------------
// Tests
// -----------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Env, IntoVal};

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

    #[test]
    fn test_delegate_and_undelegate() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);

        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let a = Address::generate(&env);
        let b = Address::generate(&env);
        token.set_balance(&a, &100i128);

        del.delegate(&a, &b);
        let got = del.get_delegate(&a);
        assert!(got.is_some());
        assert_eq!(got.unwrap(), b.clone());

        // delegated power should include a's balance for b
        assert_eq!(del.get_delegated_power(&b), 100i128);

        del.undelegate(&a);
        assert!(del.get_delegate(&a).is_none());
        assert_eq!(del.get_delegated_power(&b), 0i128);
    }

    #[test]
    #[should_panic(expected = "circular delegation")]
    fn test_circular_depth_2() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);

        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let a = Address::generate(&env);
        let b = Address::generate(&env);

        token.set_balance(&a, &10i128);
        token.set_balance(&b, &20i128);

        del.delegate(&a, &b);
        // this should panic due to circular detection
        del.delegate(&b, &a);
    }

    #[test]
    #[should_panic(expected = "circular delegation")]
    fn test_circular_depth_4() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);

        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let c = Address::generate(&env);
        let d = Address::generate(&env);

        token.set_balance(&a, &10i128);
        token.set_balance(&b, &20i128);
        token.set_balance(&c, &15i128);
        token.set_balance(&d, &25i128);

        // Create chain: a→b→c→d
        del.delegate(&a, &b);
        del.delegate(&b, &c);
        del.delegate(&c, &d);
        // Try to close cycle: d→a (would create a→b→c→d→a)
        del.delegate(&d, &a);
    }

    #[test]
    #[should_panic(expected = "circular delegation")]
    fn test_circular_depth_5() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);

        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let c = Address::generate(&env);
        let d = Address::generate(&env);
        let e = Address::generate(&env);

        // Setup: a→b→c→d→e
        del.delegate(&a, &b);
        del.delegate(&b, &c);
        del.delegate(&c, &d);
        del.delegate(&d, &e);
        // Try: e→a (creates cycle of length 5)
        del.delegate(&e, &a);
    }

    #[test]
    fn test_chain_delegation_and_effective_power() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);

        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let c = Address::generate(&env);

        token.set_balance(&a, &10i128);
        token.set_balance(&b, &20i128);
        token.set_balance(&c, &30i128);

        del.delegate(&a, &b);
        del.delegate(&b, &c);

        let pow_c = del.get_effective_power(&c);
        // c has own 30 + b(20) + a(10) = 60
        assert_eq!(pow_c, 60i128);

        // a delegated away -> effective power 0
        assert_eq!(del.get_effective_power(&a), 0i128);
    }

    #[test]
    fn test_validate_chain_depth() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);

        let del = DelegationContractClient::new(&env, &del_id);
        let _token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let c = Address::generate(&env);

        // Chain: a→b→c (depth 2)
        del.delegate(&a, &b);
        del.delegate(&b, &c);

        // Validate chain from different starting points
        let result = del.try_validate_delegation_chain(&a, &b);
        assert!(result.is_ok());

        let result = del.try_validate_delegation_chain(&b, &c);
        assert!(result.is_ok());
    }

    #[test]
    fn test_max_delegation_depth_configurable() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);

        let del = DelegationContractClient::new(&env, &del_id);
        let _token = MockMntTokenClient::new(&env, &token_id);

        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        // Default depth should be 10
        assert_eq!(del.get_max_delegation_depth(), 10u32);

        // Set to custom value
        del.set_max_delegation_depth(&admin, &20u32);
        assert_eq!(del.get_max_delegation_depth(), 20u32);
    }
}

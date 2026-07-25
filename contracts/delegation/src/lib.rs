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
    SnapshotContract,
    DelegationSnapshotPower(u32, Address), // (snapshot_id, delegate) -> i128
    DelegationSnapshotMapping(u32, Address), // (snapshot_id, delegator) -> Address
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

    pub fn set_snapshot_contract(env: Env, admin: Address, snapshot_contract: Address) {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("unauthorized");
        }
        env.storage().instance().set(&DataKey::SnapshotContract, &snapshot_contract);
    }

    /// Snapshots the current delegation state for a given snapshot_id.
    /// Iterates all delegators, resolves ultimate delegates, and records:
    /// - DelegationSnapshotPower: total delegated power each delegate received
    /// - DelegationSnapshotMapping: who delegated to whom
    pub fn snapshot_delegations(env: Env, snapshot_id: u32, snapshot_contract: Address) {
        let delegators: soroban_sdk::Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegators)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));
        let max_depth = Self::get_max_delegation_depth(env.clone());

        for i in 0..delegators.len() {
            if let Some(delegator) = delegators.get(i) {
                // Record delegation mapping for this snapshot
                if let Some(delegate) = env
                    .storage()
                    .persistent()
                    .get::<_, Address>(&DataKey::Delegate(delegator.clone()))
                {
                    env.storage().persistent().set(
                        &DataKey::DelegationSnapshotMapping(snapshot_id, delegator.clone()),
                        &delegate,
                    );

                    // Resolve ultimate delegate
                    if let Some(ult) = Self::resolve_delegate_internal(&env, delegator.clone(), max_depth) {
                        // Get delegator's staked balance at snapshot time
                        let bal: i128 = env.invoke_contract(
                            &snapshot_contract,
                            &Symbol::new(&env, "get_voting_power"),
                            (snapshot_id, delegator.clone()).into_val(&env),
                        );
                        if bal > 0 {
                            let key = DataKey::DelegationSnapshotPower(snapshot_id, ult.clone());
                            let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
                            env.storage().persistent().set(
                                &key,
                                &current.checked_add(bal).expect("overflow"),
                            );
                        }
                    }
                }
            }
        }
    }

    /// Returns the delegated voting power a delegate had at snapshot time.
    /// If the voter had delegated away at snapshot time, returns 0.
    pub fn get_delegated_power_at_snapshot(env: Env, delegate: Address, snapshot_id: u32) -> i128 {
        // If this address delegated away at snapshot time, they get 0 delegated power
        if env
            .storage()
            .persistent()
            .has(&DataKey::DelegationSnapshotMapping(snapshot_id, delegate.clone()))
        {
            return 0;
        }
        env.storage()
            .persistent()
            .get(&DataKey::DelegationSnapshotPower(snapshot_id, delegate))
            .unwrap_or(0)
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

    #[contract]
    pub struct MockSnapshotForDel;

    #[contractimpl]
    impl MockSnapshotForDel {
        pub fn set_voting_power(env: Env, snapshot_id: u32, voter: Address, amount: i128) {
            env.storage().persistent().set(&(symbol_short!("SNAP"), snapshot_id, voter), &amount);
        }
        pub fn get_voting_power(env: Env, snapshot_id: u32, voter: Address) -> i128 {
            env.storage().persistent().get(&(symbol_short!("SNAP"), snapshot_id, voter)).unwrap_or(0)
        }
    }

    #[test]
    fn test_delegation_snapshot_and_get_delegated_power_at_snapshot() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snap_id = env.register_contract(None, MockSnapshotForDel);

        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let snap = MockSnapshotForDelClient::new(&env, &snap_id);

        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let alice = Address::generate(&env); // delegator
        let bob = Address::generate(&env);   // delegate
        let charlie = Address::generate(&env); // independent voter

        token.set_balance(&alice, &100i128);
        token.set_balance(&bob, &50i128);
        token.set_balance(&charlie, &30i128);

        // Alice delegates to Bob
        del.delegate(&alice, &bob);

        // Setup snapshot: alice has 100 staked, bob has 50, charlie has 30
        snap.set_voting_power(&1, &alice, &100i128);
        snap.set_voting_power(&1, &bob, &50i128);
        snap.set_voting_power(&1, &charlie, &30i128);

        // Snapshot delegation state
        del.snapshot_delegations(&1, &snap_id);

        // Bob should have 100 delegated power at snapshot 1 (from Alice)
        assert_eq!(del.get_delegated_power_at_snapshot(&bob, &1), 100i128);
        // Alice delegated away, so her delegated power is 0
        assert_eq!(del.get_delegated_power_at_snapshot(&alice, &1), 0i128);
        // Charlie has no one delegating to him
        assert_eq!(del.get_delegated_power_at_snapshot(&charlie, &1), 0i128);

        // After snapshot, Alice undelegates
        del.undelegate(&alice);

        // Snapshot 1 should still show old state
        assert_eq!(del.get_delegated_power_at_snapshot(&bob, &1), 100i128);
        assert_eq!(del.get_delegated_power_at_snapshot(&alice, &1), 0i128);

        // New snapshot 2 with updated state
        del.snapshot_delegations(&2, &snap_id);
        assert_eq!(del.get_delegated_power_at_snapshot(&bob, &2), 0i128);
    }

    #[test]
    fn test_delegation_snapshot_chain() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snap_id = env.register_contract(None, MockSnapshotForDel);

        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let snap = MockSnapshotForDelClient::new(&env, &snap_id);

        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let a = Address::generate(&env);
        let b = Address::generate(&env);
        let c = Address::generate(&env);

        token.set_balance(&a, &10i128);
        token.set_balance(&b, &20i128);
        token.set_balance(&c, &30i128);

        // Chain: a→b→c
        del.delegate(&a, &b);
        del.delegate(&b, &c);

        // Setup snapshot balances
        snap.set_voting_power(&1, &a, &10i128);
        snap.set_voting_power(&1, &b, &20i128);
        snap.set_voting_power(&1, &c, &30i128);

        del.snapshot_delegations(&1, &snap_id);

        // c receives a(10) + b(20) = 30 delegated
        assert_eq!(del.get_delegated_power_at_snapshot(&c, &1), 30i128);
        // b delegated away, gets 0
        assert_eq!(del.get_delegated_power_at_snapshot(&b, &1), 0i128);
        // a delegated away, gets 0
        assert_eq!(del.get_delegated_power_at_snapshot(&a, &1), 0i128);
    }

    #[test]
    fn test_set_snapshot_contract() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);

        let del = DelegationContractClient::new(&env, &del_id);
        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let snap_addr = Address::generate(&env);
        del.set_snapshot_contract(&admin, &snap_addr);
    }

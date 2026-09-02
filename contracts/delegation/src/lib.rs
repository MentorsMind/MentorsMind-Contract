#![no_std]

use shared::events::{
    emit_delegation_event, evt_del_delegated, evt_del_suspended, evt_del_undelegated,
};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol,
};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    MNTToken,
    SnapshotContract,
    Delegate(Address),            // mapping: delegator -> delegate
    Delegators,                   // Vec<Address>
    MaxDelegationDepth,           // u32: configurable max depth for cycle detection
    DelegatedPowerCache(Address), // eager cache: ultimate delegate -> sum of delegator balances
    /// Eager subtree weight: `SubtreeWeight(X)` = own balance (if X is a
    /// delegator) + the subtree weight of everyone whose delegate link
    /// points directly at X. This lets a re-delegation move an entire
    /// subtree of followers to a new ultimate target in O(1), instead of
    /// having to walk every follower.
    SubtreeWeight(Address),
    /// Historical delegation snapshot: (snapshot_id, delegator) -> delegate address at that snapshot
    DelegationAtSnapshot(u32, Address),
    /// Snapshot-time delegated power cache: (snapshot_id, delegate) -> total delegated power
    DelegationSnapshot(u32, Address),
    /// Admin-configured concentration cap. Defaults to 10_000 for backward
    /// compatibility; governance can lower it to enforce anti-capture policy.
    MaxDelegatedPowerBps,
    /// Emergency switch that forces direct voting fallback by blocking new
    /// delegation writes while preserving existing read-only snapshots.
    DelegationSuspended,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum DelegationError {
    CircularDelegation = 1,
    DepthExceeded = 2,
    ConcentrationExceeded = 3,
    DelegationSuspended = 4,
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
        env.storage()
            .instance()
            .set(&DataKey::MaxDelegationDepth, &10u32);
        env.storage().instance().set(
            &DataKey::MaxDelegatedPowerBps,
            &shared::DEFAULT_DELEGATION_CAP_BPS,
        );
        env.storage()
            .instance()
            .set(&DataKey::DelegationSuspended, &false);
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
        env.storage()
            .instance()
            .set(&DataKey::MaxDelegationDepth, &depth);
    }

    pub fn get_max_delegation_depth(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MaxDelegationDepth)
            .unwrap_or(10u32)
    }

    pub fn set_delegation_power_cap_bps(env: Env, admin: Address, cap_bps: u32) {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("unauthorized");
        }
        if cap_bps == 0 || cap_bps > 10_000 {
            panic!("cap must be between 1 and 10000 bps");
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxDelegatedPowerBps, &cap_bps);
    }

    pub fn get_delegation_power_cap_bps(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MaxDelegatedPowerBps)
            .unwrap_or(shared::DEFAULT_DELEGATION_CAP_BPS)
    }

    pub fn set_delegation_suspended(env: Env, admin: Address, suspended: bool) {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("unauthorized");
        }
        env.storage()
            .instance()
            .set(&DataKey::DelegationSuspended, &suspended);
        emit_delegation_event(&env, evt_del_suspended(&env), suspended);
    }

    pub fn is_delegation_suspended(env: Env) -> bool {
        env.storage()
            .instance()
            .get(&DataKey::DelegationSuspended)
            .unwrap_or(false)
    }

    pub fn assess_delegate_concentration(
        env: Env,
        delegate: Address,
    ) -> shared::DelegationConcentrationReport {
        let total = Self::total_delegated_balance(&env);
        let delegate_power = Self::get_delegated_power(env.clone(), delegate);
        let cap = Self::get_delegation_power_cap_bps(env);
        shared::assess_delegation_concentration(total, delegate_power, cap)
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
        if Self::is_delegation_suspended(env.clone()) {
            panic!("delegation suspended");
        }
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

        let max_depth = Self::get_max_delegation_depth(env.clone());

        // `delegator`'s own balance plus everyone already delegating through
        // it (its subtree) is what needs to move to the new chain. Moving
        // this single stored number is O(depth), not O(followers) — a
        // re-delegation never needs to touch each individual follower.
        let weight = Self::get_subtree_weight(&env, &delegator);

        let previous_delegate: Option<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegate(delegator.clone()));

        if let Some(prev) = previous_delegate {
            Self::propagate_weight_change(&env, &prev, -weight, max_depth);
        }

        // Add delegator to delegators list if not present
        let mut delegators: soroban_sdk::Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegators)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));

        let cap = Self::get_delegation_power_cap_bps(env.clone());
        if cap < shared::DEFAULT_DELEGATION_CAP_BPS {
            let total_after = Self::total_delegated_balance_from(&env, &delegators, &delegator);
            let current_delegate_power = Self::get_delegated_power(env.clone(), delegate.clone());
            let projected = current_delegate_power
                .checked_add(weight)
                .expect("overflow");
            let report = shared::assess_delegation_concentration(total_after, projected, cap);
            if report.cap_exceeded {
                if let Some(prev) = env
                    .storage()
                    .persistent()
                    .get::<_, Address>(&DataKey::Delegate(delegator.clone()))
                {
                    Self::propagate_weight_change(&env, &prev, weight, max_depth);
                }
                panic!("delegation concentration cap exceeded");
            }
        }

        env.storage()
            .persistent()
            .set(&DataKey::Delegate(delegator.clone()), &delegate.clone());

        if !delegators.contains(&delegator) {
            delegators.push_back(delegator.clone());
            env.storage()
                .persistent()
                .set(&DataKey::Delegators, &delegators);
        }

        Self::propagate_weight_change(&env, &delegate, weight, max_depth);

        emit_delegation_event(
            &env,
            evt_del_delegated(&env),
            DelegatedEventData {
                delegator,
                delegate,
            },
        );
    }

    pub fn undelegate(env: Env, delegator: Address) {
        delegator.require_auth();
        let existing: Option<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegate(delegator.clone()));
        let delegate = match existing {
            Some(d) => d,
            None => return,
        };

        let max_depth = Self::get_max_delegation_depth(env.clone());
        let weight = Self::get_subtree_weight(&env, &delegator);

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
            env.storage()
                .persistent()
                .set(&DataKey::Delegators, &delegators);
        }

        Self::propagate_weight_change(&env, &delegate, -weight, max_depth);

        emit_delegation_event(
            &env,
            evt_del_undelegated(&env),
            UndelegatedEventData { delegator },
        );
    }

    pub fn get_delegate(env: Env, delegator: Address) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::Delegate(delegator))
    }

    /// O(1) read of cached delegated power for `delegate`.
    pub fn get_delegated_power(env: Env, delegate: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::DelegatedPowerCache(delegate))
            .unwrap_or(0)
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
        let own = Self::token_balance(&env, &voter);
        let delegated = Self::get_delegated_power(env.clone(), voter.clone());
        own.checked_add(delegated).expect("overflow")
    }

    /// Callable by the MNT token contract on balance change (transfer hook) to
    /// keep the eager delegated-power cache consistent with actual balances.
    /// `old_balance`/`new_balance` describe `delegator`'s balance before/after
    /// the change; the delta is propagated up `delegator`'s chain, updating
    /// every ancestor's subtree weight and the ultimate delegate's cache.
    pub fn invalidate_power_cache(
        env: Env,
        delegator: Address,
        old_balance: i128,
        new_balance: i128,
    ) {
        let token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .expect("token not set");
        token.require_auth();

        let delta = new_balance.checked_sub(old_balance).expect("overflow");
        if delta == 0 {
            return;
        }

        // The delegator's own subtree weight includes its own balance, so it
        // must be adjusted too, not just the ancestors above it.
        Self::adjust_subtree_weight(&env, &delegator, delta);

        if let Some(delegate) = env
            .storage()
            .persistent()
            .get::<_, Address>(&DataKey::Delegate(delegator))
        {
            let max_depth = Self::get_max_delegation_depth(env.clone());
            Self::propagate_weight_change(&env, &delegate, delta, max_depth);
        }
    }

    /// Snapshot current delegation state at a specific proposal snapshot.
    /// Stores (snapshot_id, delegator) -> delegate for all current delegations.
    /// Called by snapshot contract at proposal creation time.
    /// TTL: delegation snapshot entries expire after 90 days.
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
        env.storage()
            .instance()
            .set(&DataKey::SnapshotContract, &snapshot_contract);
    }

    pub fn snapshot_delegations(env: Env, snapshot_id: u32) {
        let delegators: soroban_sdk::Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegators)
            .unwrap_or_else(|| soroban_sdk::Vec::new(&env));

        let snapshot_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::SnapshotContract)
            .expect("snapshot contract not set");

        let ninety_days_ledgers = 90 * 24 * 60 * 60 / 5; // ~5s per ledger

        for delegator in delegators.iter() {
            if let Some(delegate) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::Delegate(delegator.clone()))
            {
                let key = DataKey::DelegationAtSnapshot(snapshot_id, delegator.clone());
                env.storage().persistent().set(&key, &delegate);
                // Auto-expire: extend TTL for 90 days
                env.storage().persistent().extend_ttl(
                    &key,
                    ninety_days_ledgers,
                    ninety_days_ledgers,
                );

                // Get delegator's staked balance at snapshot from snapshot contract
                let balance: i128 = env.invoke_contract(
                    &snapshot_contract,
                    &Symbol::new(&env, "get_snapshot_balance"),
                    (snapshot_id, delegator.clone()).into_val(&env),
                );

                // Accumulate delegated power for the delegate at this snapshot
                if balance > 0 {
                    let power_key = DataKey::DelegationSnapshot(snapshot_id, delegate.clone());
                    let current: i128 = env.storage().persistent().get(&power_key).unwrap_or(0);
                    env.storage()
                        .persistent()
                        .set(&power_key, &current.checked_add(balance).expect("overflow"));
                    env.storage().persistent().extend_ttl(
                        &power_key,
                        ninety_days_ledgers,
                        ninety_days_ledgers,
                    );
                }
            }
        }
    }

    /// Get the delegate of a delegator at a specific snapshot.
    /// Returns None if delegator had no delegation at snapshot time.
    pub fn get_delegation_at_snapshot(
        env: Env,
        snapshot_id: u32,
        delegator: Address,
    ) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::DelegationAtSnapshot(snapshot_id, delegator))
    }

    /// Get the total delegated power received by `delegate` at a specific snapshot.
    /// Returns the sum of staked balances (at snapshot time) of all delegators
    /// whose delegate (at snapshot time) is the given address.
    pub fn get_delegated_power_at_snapshot(env: Env, delegate: Address, snapshot_id: u32) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::DelegationSnapshot(snapshot_id, delegate))
            .unwrap_or(0)
    }

    /// Subtree weight of `addr`: its own token balance plus the subtree
    /// weight of everyone whose delegate link points directly at it.
    fn get_subtree_weight(env: &Env, addr: &Address) -> i128 {
        let own_balance = Self::token_balance(env, addr);
        let followers: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::SubtreeWeight(addr.clone()))
            .unwrap_or(0);
        own_balance.checked_add(followers).expect("overflow")
    }

    fn adjust_subtree_weight(env: &Env, addr: &Address, delta: i128) {
        if delta == 0 {
            return;
        }
        let key = DataKey::SubtreeWeight(addr.clone());
        let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        let updated = current.checked_add(delta).expect("overflow");
        env.storage().persistent().set(&key, &updated);
    }

    fn adjust_power_cache(env: &Env, delegate: &Address, delta: i128) {
        if delta == 0 {
            return;
        }
        let key = DataKey::DelegatedPowerCache(delegate.clone());
        let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        let updated = current.checked_add(delta).expect("overflow");
        env.storage().persistent().set(&key, &updated);
    }

    /// Walk the delegation chain starting at `start`, applying `delta` to
    /// each ancestor's `SubtreeWeight` (so their own subtree total stays
    /// correct if they are later re-delegated) and to the ultimate
    /// delegate's `DelegatedPowerCache`. Bounded by `max_depth` — never
    /// proportional to the number of delegators in the system.
    fn propagate_weight_change(env: &Env, start: &Address, delta: i128, max_depth: u32) {
        if delta == 0 {
            return;
        }
        let mut cur = start.clone();
        for _ in 0..max_depth {
            Self::adjust_subtree_weight(env, &cur, delta);
            match env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::Delegate(cur.clone()))
            {
                Some(next) => cur = next,
                None => {
                    // `cur` is the ultimate delegate: also update its
                    // publicly-read cache.
                    Self::adjust_power_cache(env, &cur, delta);
                    return;
                }
            }
        }
        // Depth exhausted without finding an end (shouldn't happen given
        // validate_delegation_chain enforces max_depth at write time) —
        // treat `cur` as the ultimate as a defensive fallback.
        Self::adjust_power_cache(env, &cur, delta);
    }

    fn token_balance(env: &Env, addr: &Address) -> i128 {
        let token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .expect("token not set");
        let client = soroban_sdk::token::Client::new(env, &token);
        client.balance(addr)
    }

    fn total_delegated_balance(env: &Env) -> i128 {
        let delegators: soroban_sdk::Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegators)
            .unwrap_or_else(|| soroban_sdk::Vec::new(env));
        let mut total = 0i128;
        for delegator in delegators.iter() {
            total = total
                .checked_add(Self::token_balance(env, &delegator))
                .expect("overflow");
        }
        total
    }

    fn total_delegated_balance_from(
        env: &Env,
        delegators: &soroban_sdk::Vec<Address>,
        pending: &Address,
    ) -> i128 {
        let mut total = 0i128;
        for delegator in delegators.iter() {
            total = total
                .checked_add(Self::token_balance(env, &delegator))
                .expect("overflow");
        }
        if !delegators.contains(pending) {
            total = total
                .checked_add(Self::token_balance(env, pending))
                .expect("overflow");
        }
        total
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

    // -----------------------
    // Eager cache correctness tests (#658)
    // -----------------------

    #[test]
    fn test_get_delegated_power_is_o1_cache_read() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);
        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let target = Address::generate(&env);
        let mut expected_total: i128 = 0;
        for i in 0..20 {
            let d = Address::generate(&env);
            let bal = 10i128 + i as i128;
            token.set_balance(&d, &bal);
            del.delegate(&d, &target);
            expected_total += bal;
        }

        env.budget().reset_unlimited();
        let power = del.get_delegated_power(&target);
        assert_eq!(power, expected_total);
        // Cache read must not scale with delegator count: cheap enough to assert
        // instruction count stays well below what a linear scan of 20
        // cross-contract calls would cost.
        let cpu = env.budget().cpu_instruction_cost();
        assert!(cpu < 200_000, "cache read too expensive: {}", cpu);
    }

    #[test]
    fn test_cache_consistent_after_redelegate() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);
        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let a = Address::generate(&env);
        let target1 = Address::generate(&env);
        let target2 = Address::generate(&env);
        token.set_balance(&a, &50i128);

        del.delegate(&a, &target1);
        assert_eq!(del.get_delegated_power(&target1), 50i128);
        assert_eq!(del.get_delegated_power(&target2), 0i128);

        // Re-delegate directly to a new target: cache must move, not double-count.
        del.delegate(&a, &target2);
        assert_eq!(del.get_delegated_power(&target1), 0i128);
        assert_eq!(del.get_delegated_power(&target2), 50i128);
    }

    #[test]
    fn test_cache_consistent_after_balance_change_hook() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);
        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let a = Address::generate(&env);
        let target = Address::generate(&env);
        token.set_balance(&a, &50i128);
        del.delegate(&a, &target);
        assert_eq!(del.get_delegated_power(&target), 50i128);

        // Simulate a token transfer hook: balance increases from 50 to 120.
        token.set_balance(&a, &120i128);
        del.invalidate_power_cache(&a, &50i128, &120i128);
        assert_eq!(del.get_delegated_power(&target), 120i128);

        // Balance decreases from 120 to 30.
        token.set_balance(&a, &30i128);
        del.invalidate_power_cache(&a, &120i128, &30i128);
        assert_eq!(del.get_delegated_power(&target), 30i128);
    }

    #[test]
    fn test_property_cache_equals_sum_of_delegator_balances() {
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);
        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        let target = Address::generate(&env);
        let mut delegators: std::vec::Vec<Address> = std::vec::Vec::new();
        let mut expected: i128 = 0;

        // delegate a set of accounts to target
        for i in 0..10 {
            let d = Address::generate(&env);
            let bal = (i as i128 + 1) * 7;
            token.set_balance(&d, &bal);
            del.delegate(&d, &target);
            expected += bal;
            delegators.push(d);
        }
        assert_eq!(del.get_delegated_power(&target), expected);

        // undelegate a couple, changing the sum
        del.undelegate(&delegators[2]);
        expected -= 3 * 7; // i=2 -> bal = 21
        assert_eq!(del.get_delegated_power(&target), expected);

        del.undelegate(&delegators[5]);
        expected -= 6 * 7; // i=5 -> bal = 42
        assert_eq!(del.get_delegated_power(&target), expected);

        // balance change via hook for a still-delegating account
        let d0 = delegators[0].clone();
        let old_bal = 7i128;
        let new_bal = 100i128;
        token.set_balance(&d0, &new_bal);
        del.invalidate_power_cache(&d0, &old_bal, &new_bal);
        expected += new_bal - old_bal;
        assert_eq!(del.get_delegated_power(&target), expected);
    }

    #[test]
    fn test_intermediate_redelegation_moves_entire_subtree() {
        // a -> b -> c, then b re-delegates to d. a's balance (which was
        // cached under b's original ultimate, c) must move to d along with
        // b's own balance — not get left stranded on c.
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

        del.delegate(&a, &b);
        del.delegate(&b, &c);
        assert_eq!(del.get_delegated_power(&c), 30i128); // a(10) + b(20)

        // b re-delegates to d; a's 10 must follow, not stay stuck on c.
        del.delegate(&b, &d);
        assert_eq!(del.get_delegated_power(&c), 0i128);
        assert_eq!(del.get_delegated_power(&d), 30i128);
    }

    #[test]
    fn test_deep_chain_redelegation_moves_whole_subtree() {
        // a -> b -> c -> e (ultimate). Then c re-delegates to f.
        // Both a's and b's balances must move to f.
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
        let e = Address::generate(&env);
        let f = Address::generate(&env);

        token.set_balance(&a, &5i128);
        token.set_balance(&b, &7i128);
        token.set_balance(&c, &11i128);

        del.delegate(&a, &b);
        del.delegate(&b, &c);
        del.delegate(&c, &e);
        assert_eq!(del.get_delegated_power(&e), 23i128); // 5+7+11

        del.delegate(&c, &f);
        assert_eq!(del.get_delegated_power(&e), 0i128);
        assert_eq!(del.get_delegated_power(&f), 23i128);

        // a and b's individual undelegate/re-delegate still work correctly
        // after the subtree moved.
        del.undelegate(&a);
        assert_eq!(del.get_delegated_power(&f), 18i128); // 23 - 5
    }

    #[test]
    fn test_get_delegated_power_o1_with_500_delegators() {
        // Directly exercises the #658 acceptance criterion: reading
        // delegated power must not scale with delegator count, unlike the
        // original lazy implementation which made one cross-contract
        // token::Client::balance() call per delegator.
        let env = Env::default();
        env.mock_all_auths();

        let del_id = env.register_contract(None, DelegationContract);
        let token_id = env.register_contract(None, MockMntToken);
        let del = DelegationContractClient::new(&env, &del_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let admin = Address::generate(&env);
        del.initialize(&admin, &token_id);

        env.budget().reset_unlimited();
        let target = Address::generate(&env);
        let mut expected: i128 = 0;
        for _ in 0..500 {
            let d = Address::generate(&env);
            token.set_balance(&d, &1i128);
            del.delegate(&d, &target);
            expected += 1;
        }

        let before = env.budget().cpu_instruction_cost();
        let power = del.get_delegated_power(&target);
        let after = env.budget().cpu_instruction_cost();
        assert_eq!(power, expected);
        let incremental_cpu = after - before;
        // A single storage read; must be orders of magnitude below what 500
        // cross-contract balance() calls would cost (each balance() call
        // alone typically costs well over 1M instructions under the
        // Soroban cost model once cross-contract invocation overhead is
        // included, so 500 of them would exceed the entire ledger budget).
        assert!(
            incremental_cpu < 500_000,
            "get_delegated_power cost scaled with delegator count: {}",
            incremental_cpu
        );
    }
}

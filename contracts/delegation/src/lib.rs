#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec,
};

/// Instance storage: frequently read config.
const ADMIN: Symbol = symbol_short!("ADMIN");
const SNAPSHOT: Symbol = symbol_short!("SNAPSHOT");

/// Maximum number of delegation hops resolved when attributing weight.
///
/// Chains longer than this are capped rather than followed, so a cycle that
/// somehow reaches storage cannot make weight resolution loop forever or blow
/// the transaction's budget.
pub const MAX_CHAIN_DEPTH: u32 = 8;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Outgoing delegation: delegator -> delegate.
    Delegation(Address),
    /// Reverse index: delegate -> the delegators pointing at it.
    Delegators(Address),
}

/// Governance snapshot contract this registry reads base weight from.
#[contractclient(name = "SnapshotClient")]
pub trait SnapshotTrait {
    fn get_voting_power(env: Env, proposal_id: u32, voter: Address) -> i128;
}

#[contract]
pub struct DelegationContract;

#[contractimpl]
impl DelegationContract {
    /// One-time setup. `snapshot_contract` is the governance snapshot
    /// registry the delegation weights are resolved against.
    pub fn initialize(env: Env, admin: Address, snapshot_contract: Address) {
        if env.storage().instance().has(&ADMIN) {
            panic!("already initialized");
        }

        env.storage().instance().set(&ADMIN, &admin);
        env.storage().instance().set(&SNAPSHOT, &snapshot_contract);
    }

    /// Delegates all of the caller's governance weight to `delegate`.
    ///
    /// Rejects self-delegation and any delegation that would close a cycle,
    /// detected by walking the delegate's existing chain up to
    /// `MAX_CHAIN_DEPTH` hops. Re-delegating moves the weight rather than
    /// duplicating it.
    pub fn delegate(env: Env, delegator: Address, delegate: Address) {
        Self::require_initialized(&env);
        delegator.require_auth();

        if delegator == delegate {
            panic!("cannot delegate to self");
        }

        Self::require_acyclic(&env, &delegator, &delegate);

        // Drop any previous delegation first so weight is never counted twice.
        Self::unlink(&env, &delegator);

        env.storage()
            .persistent()
            .set(&DataKey::Delegation(delegator.clone()), &delegate);

        let mut delegators: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegators(delegate.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        if !delegators.contains(&delegator) {
            delegators.push_back(delegator.clone());
            env.storage()
                .persistent()
                .set(&DataKey::Delegators(delegate.clone()), &delegators);
        }

        env.events()
            .publish((Symbol::new(&env, "delegated"), delegator), delegate);
    }

    /// Cancels the caller's outgoing delegation, returning its weight to it.
    pub fn revoke(env: Env, delegator: Address) {
        Self::require_initialized(&env);
        delegator.require_auth();

        if Self::get_delegate(env.clone(), delegator.clone()).is_none() {
            panic!("no delegation to revoke");
        }

        Self::unlink(&env, &delegator);

        env.events()
            .publish((Symbol::new(&env, "revoked"), delegator), ());
    }

    /// Who `delegator` delegated to, if anyone.
    pub fn get_delegate(env: Env, delegator: Address) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::Delegation(delegator))
    }

    /// Everyone currently delegating to `delegate`.
    pub fn get_delegators(env: Env, delegate: Address) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::Delegators(delegate))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// The holder's own weight as recorded by the governance snapshot
    /// contract, ignoring any delegation.
    pub fn get_base_weight(env: Env, proposal_id: u32, holder: Address) -> i128 {
        Self::require_initialized(&env);

        let snapshot: Address = env
            .storage()
            .instance()
            .get(&SNAPSHOT)
            .expect("snapshot not set");

        SnapshotClient::new(&env, &snapshot).get_voting_power(&proposal_id, &holder)
    }

    /// Weight `holder` may vote with on `proposal_id`.
    ///
    /// Weight flows to the end of the chain, so a delegator that has delegated
    /// votes with zero and the final delegate votes with its own weight plus
    /// everything routed to it. Chains resolve to at most `MAX_CHAIN_DEPTH`
    /// hops; anything deeper is dropped rather than followed.
    pub fn get_voting_weight(env: Env, proposal_id: u32, holder: Address) -> i128 {
        Self::require_initialized(&env);

        if Self::get_delegate(env.clone(), holder.clone()).is_some() {
            return 0;
        }

        Self::get_base_weight(env.clone(), proposal_id, holder.clone())
            .checked_add(Self::resolve(env.clone(), proposal_id, &holder, 0))
            .unwrap_or_else(|| panic!("delegated weight overflow"))
    }

    /// Walks the delegation tree below `holder`, adding the weight each
    /// delegator contributed. `holder`'s own weight is excluded because the
    /// caller already counted it.
    fn resolve(env: Env, proposal_id: u32, holder: &Address, depth: u32) -> i128 {
        if depth >= MAX_CHAIN_DEPTH {
            return 0;
        }

        let mut total: i128 = 0;

        for delegator in Self::get_delegators(env.clone(), holder.clone()).iter() {
            let contributed = Self::get_base_weight(env.clone(), proposal_id, delegator.clone())
                .checked_add(Self::resolve(
                    env.clone(),
                    proposal_id,
                    &delegator,
                    depth + 1,
                ))
                .unwrap_or_else(|| panic!("delegated weight overflow"));

            total = total
                .checked_add(contributed)
                .unwrap_or_else(|| panic!("delegated weight overflow"));
        }

        total
    }

    /// Rejects a delegation that would close a cycle, and caps chains that
    /// already sit at the depth limit.
    fn require_acyclic(env: &Env, delegator: &Address, delegate: &Address) {
        let mut current = delegate.clone();

        for _ in 0..MAX_CHAIN_DEPTH {
            if &current == delegator {
                panic!("circular delegation");
            }

            match Self::get_delegate(env.clone(), current.clone()) {
                Some(next) => current = next,
                None => return,
            }
        }

        panic!("delegation chain too deep");
    }

    /// Removes the delegator's outgoing delegation from both indexes.
    fn unlink(env: &Env, delegator: &Address) {
        let delegate = match Self::get_delegate(env.clone(), delegator.clone()) {
            Some(d) => d,
            None => return,
        };

        env.storage()
            .persistent()
            .remove(&DataKey::Delegation(delegator.clone()));

        let delegators: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::Delegators(delegate.clone()))
            .unwrap_or_else(|| Vec::new(env));

        let mut remaining = Vec::new(env);
        for d in delegators.iter() {
            if &d != delegator {
                remaining.push_back(d);
            }
        }

        if remaining.is_empty() {
            env.storage()
                .persistent()
                .remove(&DataKey::Delegators(delegate));
        } else {
            env.storage()
                .persistent()
                .set(&DataKey::Delegators(delegate), &remaining);
        }
    }

    fn require_initialized(env: &Env) {
        if !env.storage().instance().has(&ADMIN) {
            panic!("not initialized");
        }
    }
}

#[cfg(test)]
mod tests;

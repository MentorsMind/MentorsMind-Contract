#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundleNFT {
    pub token_id: u64,
    pub owner: Address,
    pub mentor: Address,
    pub sessions_total: u32,
    pub sessions_remaining: u32,
    pub expiry: u64,
    pub transferable: bool,
}

#[contracttype]
pub enum DataKey {
    TokenIdCounter,
    Bundle(u64),           // token_id -> BundleNFT
    OwnerBundles(Address), // owner -> Vec<u64>
}

#[contract]
pub struct SessionBundleNFT;

#[contractimpl]
impl SessionBundleNFT {
    /// Mint a new session bundle NFT.
    ///
    /// Auth: learner must authorize the call.
    ///
    /// Returns: token ID of the minted bundle.
    pub fn mint_bundle(
        env: Env,
        learner: Address,
        mentor: Address,
        sessions: u32,
        expiry: u64,
    ) -> u64 {
        learner.require_auth();

        let mut token_id: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::TokenIdCounter)
            .unwrap_or(0);
        token_id += 1;
        env.storage()
            .persistent()
            .set(&DataKey::TokenIdCounter, &token_id);

        let bundle = BundleNFT {
            token_id,
            owner: learner.clone(),
            mentor: mentor.clone(),
            sessions_total: sessions,
            sessions_remaining: sessions,
            expiry,
            transferable: true, // Mark as transferable by default as per requirement
        };

        env.storage()
            .persistent()
            .set(&DataKey::Bundle(token_id), &bundle);

        let mut owner_bundles: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerBundles(learner.clone()))
            .unwrap_or(Vec::new(&env));
        owner_bundles.push_back(token_id);
        env.storage()
            .persistent()
            .set(&DataKey::OwnerBundles(learner.clone()), &owner_bundles);

        // Emit minted event
        env.events().publish(
            (symbol_short!("bundle"), symbol_short!("minted"), token_id),
            (learner, mentor, sessions, expiry),
        );

        token_id
    }

    /// Transfer an NFT bundle to another learner.
    ///
    /// Auth: from address must authorize.
    /// Only works if transferable is true.
    pub fn transfer(env: Env, from: Address, to: Address, token_id: u64) {
        from.require_auth();

        let mut bundle: BundleNFT = env
            .storage()
            .persistent()
            .get(&DataKey::Bundle(token_id))
            .expect("Bundle not found");

        if bundle.owner != from {
            panic!("Not the owner");
        }

        if !bundle.transferable {
            panic!("Not transferable");
        }

        // Remove from old owner's list
        let from_bundles: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerBundles(from.clone()))
            .expect("Owner list not found");
        let mut new_from_bundles = Vec::new(&env);
        for id in from_bundles.iter() {
            if id != token_id {
                new_from_bundles.push_back(id);
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::OwnerBundles(from.clone()), &new_from_bundles);

        // Add to new owner's list
        let mut to_bundles: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerBundles(to.clone()))
            .unwrap_or(Vec::new(&env));
        to_bundles.push_back(token_id);
        env.storage()
            .persistent()
            .set(&DataKey::OwnerBundles(to.clone()), &to_bundles);

        // Update bundle owner
        bundle.owner = to.clone();
        env.storage()
            .persistent()
            .set(&DataKey::Bundle(token_id), &bundle);

        // Emit transferred event
        env.events().publish(
            (symbol_short!("bundle"), symbol_short!("transfd"), token_id),
            (from, to),
        );
    }

    /// Redeem a session from the bundle.
    ///
    /// Auth: holder must authorize.
    /// Decrements sessions_remaining and emits a registry event.
    pub fn redeem(env: Env, holder: Address, token_id: u64) {
        holder.require_auth();

        let mut bundle: BundleNFT = env
            .storage()
            .persistent()
            .get(&DataKey::Bundle(token_id))
            .expect("Bundle not found");

        if bundle.owner != holder {
            panic!("Not the owner");
        }

        if env.ledger().timestamp() > bundle.expiry {
            panic!("Expired");
        }

        if bundle.sessions_remaining == 0 {
            panic!("No sessions remaining");
        }

        bundle.sessions_remaining -= 1;
        env.storage()
            .persistent()
            .set(&DataKey::Bundle(token_id), &bundle);

        // Emit redeemed event
        env.events().publish(
            (symbol_short!("bundle"), symbol_short!("redeemd"), token_id),
            (holder.clone(), bundle.sessions_remaining),
        );

        // Create session in registry (conceptual, emitted as event)
        env.events().publish(
            (
                symbol_short!("registry"),
                symbol_short!("session"),
                bundle.mentor.clone(),
            ),
            (holder, env.ledger().timestamp()),
        );
    }

    /// Burn the bundle when sessions are exhausted or it has expired.
    ///
    /// Auth: holder must authorize.
    pub fn burn(env: Env, holder: Address, token_id: u64) {
        holder.require_auth();

        let bundle: BundleNFT = env
            .storage()
            .persistent()
            .get(&DataKey::Bundle(token_id))
            .expect("Bundle not found");

        if bundle.owner != holder {
            panic!("Not the owner");
        }

        let is_expired = env.ledger().timestamp() > bundle.expiry;
        let is_empty = bundle.sessions_remaining == 0;

        if !is_expired && !is_empty {
            panic!("Cannot burn: neither expired nor empty");
        }

        // Remove from owner's list
        let owner_bundles_res: Option<Vec<u64>> = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerBundles(holder.clone()));
        if let Some(owner_bundles) = owner_bundles_res {
            let mut new_owner_bundles = Vec::new(&env);
            for id in owner_bundles.iter() {
                if id != token_id {
                    new_owner_bundles.push_back(id);
                }
            }
            env.storage()
                .persistent()
                .set(&DataKey::OwnerBundles(holder.clone()), &new_owner_bundles);
        }

        // Delete bundle
        env.storage()
            .persistent()
            .remove(&DataKey::Bundle(token_id));

        // Emit burned event
        env.events().publish(
            (symbol_short!("bundle"), symbol_short!("burned"), token_id),
            holder,
        );
    }

    /// Get bundle details by token ID.
    pub fn get_bundle(env: Env, token_id: u64) -> BundleNFT {
        env.storage()
            .persistent()
            .get(&DataKey::Bundle(token_id))
            .expect("Bundle not found")
    }

    /// Get all bundles owned by a specific address.
    pub fn get_bundles_by_owner(env: Env, owner: Address) -> Vec<BundleNFT> {
        let owner_bundles_ids: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerBundles(owner))
            .unwrap_or(Vec::new(&env));

        let mut bundles = Vec::new(&env);
        for id in owner_bundles_ids.iter() {
            if let Some(bundle) = env
                .storage()
                .persistent()
                .get::<DataKey, BundleNFT>(&DataKey::Bundle(id))
            {
                bundles.push_back(bundle);
            }
        }
        bundles
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[test]
    fn test_mint_and_get() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SessionBundleNFT);
        let client = SessionBundleNFTClient::new(&env, &contract_id);

        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);
        let sessions = 10;
        let expiry = 1000;

        let token_id = client.mint_bundle(&learner, &mentor, &sessions, &expiry);
        assert_eq!(token_id, 1);

        let bundle = client.get_bundle(&token_id);
        assert_eq!(bundle.owner, learner);
        assert_eq!(bundle.mentor, mentor);
        assert_eq!(bundle.sessions_total, sessions);
        assert_eq!(bundle.sessions_remaining, sessions);
        assert_eq!(bundle.expiry, expiry);
        assert_eq!(bundle.transferable, true);

        let learner_bundles = client.get_bundles_by_owner(&learner);
        assert_eq!(learner_bundles.len(), 1);
        assert_eq!(learner_bundles.get(0).unwrap().token_id, token_id);
    }

    #[test]
    fn test_redeem_3_times() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SessionBundleNFT);
        let client = SessionBundleNFTClient::new(&env, &contract_id);

        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);
        let sessions = 10;
        let expiry = 1000;

        let token_id = client.mint_bundle(&learner, &mentor, &sessions, &expiry);

        client.redeem(&learner, &token_id);
        client.redeem(&learner, &token_id);
        client.redeem(&learner, &token_id);

        let bundle = client.get_bundle(&token_id);
        assert_eq!(bundle.sessions_remaining, 7);
    }

    #[test]
    fn test_transfer() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SessionBundleNFT);
        let client = SessionBundleNFTClient::new(&env, &contract_id);

        let learner1 = Address::generate(&env);
        let learner2 = Address::generate(&env);
        let mentor = Address::generate(&env);

        let token_id = client.mint_bundle(&learner1, &mentor, &5, &1000);

        client.transfer(&learner1, &learner2, &token_id);

        let bundle = client.get_bundle(&token_id);
        assert_eq!(bundle.owner, learner2);

        assert_eq!(client.get_bundles_by_owner(&learner1).len(), 0);
        assert_eq!(client.get_bundles_by_owner(&learner2).len(), 1);
    }

    #[test]
    #[should_panic(expected = "Expired")]
    fn test_expiry_enforcement() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SessionBundleNFT);
        let client = SessionBundleNFTClient::new(&env, &contract_id);

        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);
        let sessions = 10;
        let expiry = 100;

        let token_id = client.mint_bundle(&learner, &mentor, &sessions, &expiry);

        env.ledger().with_mut(|li| {
            li.timestamp = 101;
        });

        client.redeem(&learner, &token_id);
    }

    #[test]
    fn test_burn_when_empty() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SessionBundleNFT);
        let client = SessionBundleNFTClient::new(&env, &contract_id);

        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);

        let token_id = client.mint_bundle(&learner, &mentor, &1, &1000);
        client.redeem(&learner, &token_id);

        client.burn(&learner, &token_id);

        assert_eq!(client.get_bundles_by_owner(&learner).len(), 0);
        let res = env
            .storage()
            .persistent()
            .get::<DataKey, BundleNFT>(&DataKey::Bundle(token_id));
        assert!(res.is_none());
    }

    #[test]
    #[should_panic(expected = "Cannot burn: neither expired nor empty")]
    fn test_burn_fails_if_not_empty_nor_expired() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SessionBundleNFT);
        let client = SessionBundleNFTClient::new(&env, &contract_id);

        let learner = Address::generate(&env);
        let mentor = Address::generate(&env);

        let token_id = client.mint_bundle(&learner, &mentor, &5, &1000);
        client.burn(&learner, &token_id);
    }
}

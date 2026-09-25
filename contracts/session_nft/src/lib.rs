#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, vec, xdr::ToXdr, Address, BytesN, Env,
    IntoVal, String, Symbol, Vec,
};

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
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NFTMintedEvent {
    pub nft_id: u64,
    pub learner: Address,
    pub session_count: u32,
    pub session_ids_hash: BytesN<32>,
}

/// Canonical on-chain NFT metadata for a minted session bundle (#759).
///
/// `metadata_hash` is a sha256 seal over the XDR-serialized
/// `(name, skill, mentor, learner, completed_at, rating_snapshot)` tuple,
/// letting any party later recompute and compare it against the stored
/// hash via [`SessionBundleNFT::verify_metadata_integrity`] to detect
/// tampering.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NftMetadata {
    pub name: String,
    pub skill: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub completed_at: u64,
    pub rating_snapshot: u32,
    pub sessions_count: u32,
    pub metadata_hash: BytesN<32>,
}

#[contracttype]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    TokenIdCounter,
    Bundle(u64),           // token_id -> BundleNFT
    OwnerBundles(Address), // owner -> Vec<u64>
    BundledSessions(u64),  // token_id -> Vec<Symbol>
    Metadata(u64),          // token_id -> NftMetadata
}

/// Approximate Stellar ledger close time, used to convert the "sessions_count
/// * 30 days" TTL requirement into a ledger-count `extend_ttl` bump.
const SECONDS_PER_LEDGER: u32 = 5;
const LEDGERS_PER_30_DAYS: u32 = (30 * 24 * 60 * 60) / SECONDS_PER_LEDGER;

#[contract]
pub struct SessionBundleNFT;

#[contractimpl]
impl SessionBundleNFT {
    pub fn mint_bundle(
        env: Env,
        learner: Address,
        session_registry: Address,
        reputation: Address,
        session_ids: Vec<Symbol>,
        session_ids_hash: BytesN<32>,
        expiry: u64,
        name: String,
        skill: Symbol,
    ) -> u64 {
        learner.require_auth();

        let count = session_ids.len();
        if count == 0 {
            panic!("No sessions provided");
        }

        // Cross-verify each session and extract mentor
        let mut mentor: Option<Address> = None;
        for i in 0..count {
            let sid = session_ids.get(i).unwrap();
            let record: mentorminds_session_registry::SessionRecord = env.invoke_contract(
                &session_registry,
                &Symbol::new(&env, "get_session"),
                (sid,).into_val(&env),
            );

            if record.status != mentorminds_session_registry::SessionStatus::Completed {
                panic!("Session not completed");
            }
            if record.learner != learner {
                panic!("Session learner mismatch");
            }
            match &mentor {
                Some(m) if *m != record.mentor => panic!("Sessions have different mentors"),
                _ => mentor = Some(record.mentor),
            }
        }

        let mentor = mentor.expect("mentor should be set");

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
            sessions_total: count,
            sessions_remaining: count,
            expiry,
            transferable: true,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Bundle(token_id), &bundle);

        // Store bundled session IDs on-chain
        env.storage()
            .persistent()
            .set(&DataKey::BundledSessions(token_id), &session_ids);

        // Populate and seal canonical NFT metadata (#759).
        let completed_at = env.ledger().timestamp();
        let (rating_x100, _review_count): (u64, u64) = env.invoke_contract(
            &reputation,
            &Symbol::new(&env, "get_mentor_rating"),
            (mentor.clone(),).into_val(&env),
        );
        let rating_snapshot = rating_x100 as u32;
        let metadata_hash = Self::compute_metadata_hash(
            &env,
            &name,
            &skill,
            &mentor,
            &learner,
            completed_at,
            rating_snapshot,
        );
        let metadata = NftMetadata {
            name,
            skill,
            mentor: mentor.clone(),
            learner: learner.clone(),
            completed_at,
            rating_snapshot,
            sessions_count: count,
            metadata_hash,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Metadata(token_id), &metadata);
        // TTL proportional to sessions_count: more sessions bundled == a
        // more valuable credential, so it's kept alive longer.
        let ttl_bump = LEDGERS_PER_30_DAYS.saturating_mul(count.max(1));
        env.storage().persistent().extend_ttl(
            &DataKey::Metadata(token_id),
            ttl_bump / 2,
            ttl_bump,
        );

        let mut owner_bundles: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerBundles(learner.clone()))
            .unwrap_or(Vec::new(&env));
        owner_bundles.push_back(token_id);
        env.storage()
            .persistent()
            .set(&DataKey::OwnerBundles(learner.clone()), &owner_bundles);

        // Emit NFTMinted event
        env.events().publish(
            (symbol_short!("bundle"), symbol_short!("minted"), token_id),
            NFTMintedEvent {
                nft_id: token_id,
                learner,
                session_count: count,
                session_ids_hash,
            },
        );

        token_id
    }

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

        let mut to_bundles: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerBundles(to.clone()))
            .unwrap_or(Vec::new(&env));
        to_bundles.push_back(token_id);
        env.storage()
            .persistent()
            .set(&DataKey::OwnerBundles(to.clone()), &to_bundles);

        bundle.owner = to.clone();
        env.storage()
            .persistent()
            .set(&DataKey::Bundle(token_id), &bundle);

        env.events().publish(
            (symbol_short!("bundle"), symbol_short!("transfd"), token_id),
            (from, to),
        );
    }

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

        env.events().publish(
            (symbol_short!("bundle"), symbol_short!("redeemd"), token_id),
            (holder.clone(), bundle.sessions_remaining),
        );

        env.events().publish(
            (
                symbol_short!("registry"),
                symbol_short!("session"),
                bundle.mentor.clone(),
            ),
            (holder, env.ledger().timestamp()),
        );
    }

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

        env.storage()
            .persistent()
            .remove(&DataKey::Bundle(token_id));
        env.storage()
            .persistent()
            .remove(&DataKey::BundledSessions(token_id));

        env.events().publish(
            (symbol_short!("bundle"), symbol_short!("burned"), token_id),
            holder,
        );
    }

    /// Verify that all sessions backing the NFT are still in Completed status.
    pub fn verify_nft_provenance(env: Env, nft_id: u64, session_registry: Address) -> bool {
        let session_ids: Vec<Symbol> = match env
            .storage()
            .persistent()
            .get(&DataKey::BundledSessions(nft_id))
        {
            Some(ids) => ids,
            None => return false,
        };

        for i in 0..session_ids.len() {
            let sid = session_ids.get(i).unwrap();
            let record = env.invoke_contract::<mentorminds_session_registry::SessionRecord>(
                &session_registry,
                &Symbol::new(&env, "get_session"),
                (sid,).into_val(&env),
            );
            if record.status != mentorminds_session_registry::SessionStatus::Completed {
                return false;
            }
        }
        true
    }

    pub fn get_bundle(env: Env, token_id: u64) -> BundleNFT {
        env.storage()
            .persistent()
            .get(&DataKey::Bundle(token_id))
            .expect("Bundle not found")
    }

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

    pub fn get_bundled_sessions(env: Env, token_id: u64) -> Vec<Symbol> {
        env.storage()
            .persistent()
            .get(&DataKey::BundledSessions(token_id))
            .unwrap_or(Vec::new(&env))
    }

    /// Canonical on-chain metadata for a minted token.
    pub fn get_token_metadata(env: Env, token_id: u64) -> NftMetadata {
        env.storage()
            .persistent()
            .get(&DataKey::Metadata(token_id))
            .expect("Metadata not found")
    }

    /// Recomputes `metadata_hash` from the stored fields and compares it
    /// against the sealed value, detecting any direct-storage tampering
    /// with `name`, `skill`, `mentor`, `learner`, `completed_at`, or
    /// `rating_snapshot`.
    pub fn verify_metadata_integrity(env: Env, token_id: u64) -> bool {
        let metadata: NftMetadata = match env
            .storage()
            .persistent()
            .get(&DataKey::Metadata(token_id))
        {
            Some(m) => m,
            None => return false,
        };
        let recomputed = Self::compute_metadata_hash(
            &env,
            &metadata.name,
            &metadata.skill,
            &metadata.mentor,
            &metadata.learner,
            metadata.completed_at,
            metadata.rating_snapshot,
        );
        recomputed == metadata.metadata_hash
    }

    fn compute_metadata_hash(
        env: &Env,
        name: &String,
        skill: &Symbol,
        mentor: &Address,
        learner: &Address,
        completed_at: u64,
        rating_snapshot: u32,
    ) -> BytesN<32> {
        let tuple = (
            name.clone(),
            skill.clone(),
            mentor.clone(),
            learner.clone(),
            completed_at,
            rating_snapshot,
        );
        let bytes = tuple.to_xdr(env);
        env.crypto().sha256(&bytes).into()
    }
}

#[cfg(test)]
mod test {
    extern crate std;
    use std::string::ToString;

    use super::*;
    use mentorminds_session_registry::{SessionRegistry, SessionRegistryClient, SessionStatus};
    use soroban_sdk::testutils::{Address as _, Events, Ledger};

    fn dummy_hash(env: &Env) -> BytesN<32> {
        BytesN::from_array(env, &[0u8; 32])
    }

    mod reputation_mock {
        use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

        #[contracttype]
        #[derive(Clone)]
        pub enum RatingKey {
            Rating(Address),
        }

        #[contract]
        pub struct MockReputationN;

        #[contractimpl]
        impl MockReputationN {
            pub fn set_rating(env: Env, mentor: Address, rating_x100: u64) {
                env.storage()
                    .persistent()
                    .set(&RatingKey::Rating(mentor), &rating_x100);
            }

            pub fn get_mentor_rating(env: Env, mentor: Address) -> (u64, u64) {
                let rating: u64 = env
                    .storage()
                    .persistent()
                    .get(&RatingKey::Rating(mentor))
                    .unwrap_or(0);
                (rating, 1u64)
            }
        }
    }
    use reputation_mock::{MockReputationN, MockReputationNClient};

    fn default_name(env: &Env) -> String {
        String::from_str(env, "Rust Fundamentals Bundle")
    }

    fn default_skill(env: &Env) -> Symbol {
        Symbol::new(env, "rust")
    }

    fn compute_hash(env: &Env, session_ids: &Vec<Symbol>) -> BytesN<32> {
        let mut buf = soroban_sdk::Bytes::new(env);
        for i in 0..session_ids.len() {
            let sid = session_ids.get(i).unwrap();
            let s = sid.to_string();
            let b = soroban_sdk::Bytes::from_slice(env, s.as_bytes());
            for byte in b.iter() {
                buf.push_back(byte);
            }
        }
        env.crypto().sha256(&buf).into()
    }

    struct TestFixture {
        env: Env,
        nft_id: Address,
        registry_id: Address,
        reputation_id: Address,
        backend: Address,
        learner: Address,
        mentor: Address,
    }

    impl TestFixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().with_mut(|li| li.timestamp = 1_000_000);

            let backend = Address::generate(&env);
            let learner = Address::generate(&env);
            let mentor = Address::generate(&env);

            let nft_id = env.register_contract(None, SessionBundleNFT);
            let registry_id = env.register_contract(None, SessionRegistry);
            let reputation_id = env.register_contract(None, MockReputationN);

            let registry = SessionRegistryClient::new(&env, &registry_id);
            registry.initialize(&backend);

            TestFixture {
                env,
                nft_id,
                registry_id,
                reputation_id,
                backend,
                learner,
                mentor,
            }
        }

        fn client(&self) -> SessionBundleNFTClient {
            SessionBundleNFTClient::new(&self.env, &self.nft_id)
        }

        fn registry(&self) -> SessionRegistryClient {
            SessionRegistryClient::new(&self.env, &self.registry_id)
        }

        fn set_mentor_rating(&self, mentor: &Address, rating_x100: u64) {
            MockReputationNClient::new(&self.env, &self.reputation_id)
                .set_rating(mentor, &rating_x100);
        }

        fn mint_default(
            &self,
            session_ids: &Vec<Symbol>,
            hash: &BytesN<32>,
            expiry: u64,
        ) -> u64 {
            self.client().mint_bundle(
                &self.learner,
                &self.registry_id,
                &self.reputation_id,
                session_ids,
                hash,
                &expiry,
                &default_name(&self.env),
                &default_skill(&self.env),
            )
        }

        fn register_completed_session(&self, id: &str) -> Symbol {
            let sid = Symbol::new(&self.env, id);
            self.registry().register_session(
                &sid,
                &self.mentor,
                &self.learner,
                &1_500_000u64,
                &60u32,
                &100i128,
                &Address::generate(&self.env),
            );
            self.registry()
                .update_status(&sid, &SessionStatus::Completed);
            sid
        }

        fn register_pending_session(&self, id: &str) -> Symbol {
            let sid = Symbol::new(&self.env, id);
            self.registry().register_session(
                &sid,
                &self.mentor,
                &self.learner,
                &1_500_000u64,
                &60u32,
                &100i128,
                &Address::generate(&self.env),
            );
            sid
        }
    }

    #[test]
    fn test_mint_with_completed_sessions() {
        let f = TestFixture::setup();
        let sid1 = f.register_completed_session("s1");
        let sid2 = f.register_completed_session("s2");
        let sid3 = f.register_completed_session("s3");

        let session_ids = vec![&f.env, sid1.clone(), sid2.clone(), sid3.clone()];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id =
            f.mint_default(&session_ids, &hash, 2000u64);
        assert_eq!(token_id, 1);

        let bundle = f.client().get_bundle(&token_id);
        assert_eq!(bundle.owner, f.learner);
        assert_eq!(bundle.mentor, f.mentor);
        assert_eq!(bundle.sessions_total, 3);
        assert_eq!(bundle.sessions_remaining, 3);

        let stored = f.client().get_bundled_sessions(&token_id);
        assert_eq!(stored.len(), 3);
        assert_eq!(stored.get(0).unwrap(), sid1);
        assert_eq!(stored.get(1).unwrap(), sid2);
        assert_eq!(stored.get(2).unwrap(), sid3);
    }

    #[test]
    #[should_panic(expected = "Session not completed")]
    fn test_mint_with_pending_session_rejected() {
        let f = TestFixture::setup();
        let sid = f.register_pending_session("pending1");

        let session_ids = vec![&f.env, sid];
        let hash = dummy_hash(&f.env);
        f.mint_default(&session_ids, &hash, 2000u64);
    }

    #[test]
    #[should_panic(expected = "Session not found")]
    fn test_mint_with_unregistered_session_rejected() {
        let f = TestFixture::setup();
        let sid = Symbol::new(&f.env, "nonexistent");

        let session_ids = vec![&f.env, sid];
        let hash = dummy_hash(&f.env);
        f.mint_default(&session_ids, &hash, 2000u64);
    }

    #[test]
    #[should_panic(expected = "Session learner mismatch")]
    fn test_mint_with_wrong_learner_rejected() {
        let f = TestFixture::setup();
        let wrong_learner = Address::generate(&f.env);

        let sid = Symbol::new(&f.env, "s_wrong");
        f.registry().register_session(
            &sid,
            &f.mentor,
            &wrong_learner,
            &1_500_000u64,
            &60u32,
            &100i128,
            &Address::generate(&f.env),
        );
        f.registry().update_status(&sid, &SessionStatus::Completed);

        let session_ids = vec![&f.env, sid];
        let hash = dummy_hash(&f.env);
        f.mint_default(&session_ids, &hash, 2000u64);
    }

    #[test]
    #[should_panic(expected = "No sessions provided")]
    fn test_mint_with_empty_session_ids() {
        let f = TestFixture::setup();
        let session_ids: Vec<Symbol> = Vec::new(&f.env);
        let hash = dummy_hash(&f.env);
        f.mint_default(&session_ids, &hash, 2000u64);
    }

    #[test]
    fn test_verify_provenance_returns_true() {
        let f = TestFixture::setup();
        let sid1 = f.register_completed_session("p1");
        let sid2 = f.register_completed_session("p2");

        let session_ids = vec![&f.env, sid1, sid2];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id =
            f.mint_default(&session_ids, &hash, 2000u64);

        let valid = f.client().verify_nft_provenance(&token_id, &f.registry_id);
        assert!(valid);
    }

    #[test]
    fn test_verify_provenance_false_if_session_cancelled() {
        let f = TestFixture::setup();
        let sid1 = f.register_completed_session("c1");
        let sid2 = f.register_completed_session("c2");

        let session_ids = vec![&f.env, sid1.clone(), sid2];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id =
            f.mint_default(&session_ids, &hash, 2000u64);

        f.registry().update_status(&sid1, &SessionStatus::Cancelled);

        let valid = f.client().verify_nft_provenance(&token_id, &f.registry_id);
        assert!(!valid);
    }

    #[test]
    fn test_verify_provenance_false_for_nonexistent_nft() {
        let f = TestFixture::setup();
        let valid = f.client().verify_nft_provenance(&999u64, &f.registry_id);
        assert!(!valid);
    }

    #[test]
    fn test_redeem_still_works() {
        let f = TestFixture::setup();
        let sid = f.register_completed_session("r1");
        let session_ids = vec![&f.env, sid];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id =
            f.mint_default(&session_ids, &hash, 2000u64);

        f.client().redeem(&f.learner, &token_id);

        let bundle = f.client().get_bundle(&token_id);
        assert_eq!(bundle.sessions_remaining, 0);
    }

    #[test]
    fn test_transfer_still_works() {
        let f = TestFixture::setup();
        let sid = f.register_completed_session("t1");
        let session_ids = vec![&f.env, sid];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id =
            f.mint_default(&session_ids, &hash, 2000u64);

        let new_owner = Address::generate(&f.env);
        f.client().transfer(&f.learner, &new_owner, &token_id);

        let bundle = f.client().get_bundle(&token_id);
        assert_eq!(bundle.owner, new_owner);
    }

    #[test]
    fn test_burn_cleans_up_bundled_sessions() {
        let f = TestFixture::setup();
        let sid = f.register_completed_session("b1");
        let session_ids = vec![&f.env, sid];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id =
            f.mint_default(&session_ids, &hash, 100u64);

        f.env.ledger().with_mut(|li| li.timestamp = 200);

        f.client().burn(&f.learner, &token_id);

        let stored = f.client().get_bundled_sessions(&token_id);
        assert_eq!(stored.len(), 0);
    }

    #[test]
    fn test_nft_minted_event_contains_session_ids_hash() {
        let f = TestFixture::setup();
        let sid1 = f.register_completed_session("e1");
        let sid2 = f.register_completed_session("e2");

        let session_ids = vec![&f.env, sid1, sid2];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id =
            f.mint_default(&session_ids, &hash, 2000u64);

        let events = f.env.events().all();
        let mint_event = events.get(0).unwrap();
        let payload: NFTMintedEvent = mint_event.2.into_val(&f.env);
        assert_eq!(payload.nft_id, token_id);
        assert_eq!(payload.learner, f.learner);
        assert_eq!(payload.session_count, 2);
        assert_eq!(payload.session_ids_hash, hash);
    }

    // ── #759: NFT metadata + integrity seal ─────────────────────────────────

    #[test]
    fn test_get_token_metadata_fully_populated_after_mint() {
        let f = TestFixture::setup();
        let sid1 = f.register_completed_session("s1");
        let sid2 = f.register_completed_session("s2");
        f.set_mentor_rating(&f.mentor, 480);

        let session_ids = vec![&f.env, sid1, sid2];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id = f.mint_default(&session_ids, &hash, 2000u64);

        let metadata = f.client().get_token_metadata(&token_id);
        assert_eq!(metadata.name, default_name(&f.env));
        assert_eq!(metadata.skill, default_skill(&f.env));
        assert_eq!(metadata.mentor, f.mentor);
        assert_eq!(metadata.learner, f.learner);
        assert_eq!(metadata.completed_at, f.env.ledger().timestamp());
        assert_eq!(metadata.rating_snapshot, 480);
        assert_eq!(metadata.sessions_count, 2);
        assert_ne!(metadata.metadata_hash, dummy_hash(&f.env));
    }

    #[test]
    fn test_verify_metadata_integrity_true_for_freshly_minted_token() {
        let f = TestFixture::setup();
        let sid1 = f.register_completed_session("s1");
        let session_ids = vec![&f.env, sid1];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id = f.mint_default(&session_ids, &hash, 2000u64);

        assert!(f.client().verify_metadata_integrity(&token_id));
    }

    #[test]
    fn test_verify_metadata_integrity_false_when_tampered() {
        let f = TestFixture::setup();
        let sid1 = f.register_completed_session("s1");
        let session_ids = vec![&f.env, sid1];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id = f.mint_default(&session_ids, &hash, 2000u64);

        // Directly overwrite the stored metadata with a tampered rating,
        // bypassing mint_bundle's hash computation.
        f.env.as_contract(&f.nft_id, || {
            let mut metadata: NftMetadata = f
                .env
                .storage()
                .persistent()
                .get(&DataKey::Metadata(token_id))
                .unwrap();
            metadata.rating_snapshot = 999;
            f.env
                .storage()
                .persistent()
                .set(&DataKey::Metadata(token_id), &metadata);
        });

        assert!(!f.client().verify_metadata_integrity(&token_id));
    }

    #[test]
    fn test_verify_metadata_integrity_false_for_unknown_token() {
        let f = TestFixture::setup();
        assert!(!f.client().verify_metadata_integrity(&999u64));
    }

    #[test]
    fn test_metadata_hash_is_deterministic() {
        let env = Env::default();
        let name = default_name(&env);
        let skill = default_skill(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);

        let h1 = SessionBundleNFT::compute_metadata_hash(
            &env, &name, &skill, &mentor, &learner, 1_000, 480,
        );
        let h2 = SessionBundleNFT::compute_metadata_hash(
            &env, &name, &skill, &mentor, &learner, 1_000, 480,
        );
        assert_eq!(h1, h2);

        let h3 = SessionBundleNFT::compute_metadata_hash(
            &env, &name, &skill, &mentor, &learner, 1_001, 480,
        );
        assert_ne!(h1, h3, "different inputs must not collide");
    }

    #[test]
    fn test_integration_mint_then_read_metadata_matches_session_data() {
        let f = TestFixture::setup();
        let sid1 = f.register_completed_session("s1");
        let sid2 = f.register_completed_session("s2");
        let sid3 = f.register_completed_session("s3");
        f.set_mentor_rating(&f.mentor, 450);

        let session_ids = vec![&f.env, sid1, sid2, sid3];
        let hash = compute_hash(&f.env, &session_ids);
        let token_id = f.mint_default(&session_ids, &hash, 5000u64);

        let metadata = f.client().get_token_metadata(&token_id);
        assert_eq!(metadata.mentor, f.mentor);
        assert_eq!(metadata.learner, f.learner);
        assert_eq!(metadata.sessions_count, 3);
        assert_eq!(metadata.rating_snapshot, 450);
        assert!(f.client().verify_metadata_integrity(&token_id));
    }
}

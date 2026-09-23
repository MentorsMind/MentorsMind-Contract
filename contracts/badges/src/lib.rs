#![no_std]
mod badge_types;

use soroban_sdk::{
    contract, contractevent, contractimpl, contracttype, symbol_short, vec, Address, BytesN, Env, Vec,
};
use shared::{audit_privacy, compute_nullifier as shared_compute_nullifier, PrivacyAudit, ZKProof};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BadgeType {
    FirstSession,
    TenSessions,
    HundredSessions,
    TopRated,
    VerifiedExpert,
    EarlyAdopter,
    CommunityLeader,
}

#[contracttype]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Backend,
    MentorBadge(Address, BadgeType),
    MentorBadges(Address),
    BadgeCount(BadgeType),
    BadgeNullifier(BytesN<32>),
    BadgeProof(BytesN<32>),
    BadgePrivacyAudit(BytesN<32>),
}

#[contractevent]
#[derive(Clone)]
struct BadgeAwardedEvent {
    #[topic]
    action: Symbol,
    #[topic]
    mentor: Address,
    badge_type: BadgeType,
}

#[contractevent]
#[derive(Clone)]
struct BadgeAnonMintEvent {
    #[topic]
    action: Symbol,
    #[topic]
    nullifier: BytesN<32>,
    badge_type_hash: BytesN<32>,
}

#[contract]
pub struct Badges;

#[contractimpl]
impl Badges {
    pub fn initialize(env: Env, admin: Address, backend: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::Backend, &backend);
    }

    pub fn award_badge(env: Env, mentor: Address, badge_type: BadgeType) {
        let backend: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Backend)
            .expect("not initialized");
        backend.require_auth();

        let held_key = DataKey::MentorBadge(mentor.clone(), badge_type.clone());
        if env.storage().persistent().get(&held_key).unwrap_or(false) {
            panic!("badge already awarded");
        }

        env.storage().persistent().set(&held_key, &true);

        let list_key = DataKey::MentorBadges(mentor.clone());
        let mut badges: Vec<BadgeType> = env
            .storage()
            .persistent()
            .get(&list_key)
            .unwrap_or_else(|| vec![&env]);
        badges.push_back(badge_type.clone());
        env.storage().persistent().set(&list_key, &badges);

        let count_key = DataKey::BadgeCount(badge_type.clone());
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        env.storage().persistent().set(&count_key, &(count + 1));

        BadgeAwardedEvent {
            action: symbol_short!("badge_aw"),
            mentor,
            badge_type,
        }.publish(env);
    }

    pub fn revoke_badge(env: Env, mentor: Address, badge_type: BadgeType) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        let held_key = DataKey::MentorBadge(mentor.clone(), badge_type.clone());
        if !env.storage().persistent().get(&held_key).unwrap_or(false) {
            panic!("badge not held");
        }

        env.storage().persistent().set(&held_key, &false);

        let list_key = DataKey::MentorBadges(mentor.clone());
        let badges: Vec<BadgeType> = env
            .storage()
            .persistent()
            .get(&list_key)
            .unwrap_or_else(|| vec![&env]);
        let mut updated: Vec<BadgeType> = vec![&env];
        for b in badges.iter() {
            if b != badge_type {
                updated.push_back(b);
            }
        }
        env.storage().persistent().set(&list_key, &updated);

        let count_key = DataKey::BadgeCount(badge_type.clone());
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&count_key, &count.saturating_sub(1));

        BadgeAwardedEvent {
            action: symbol_short!("badge_rv"),
            mentor,
            badge_type,
        }.publish(env);
    }

    pub fn has_badge(env: Env, mentor: Address, badge_type: BadgeType) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::MentorBadge(mentor, badge_type))
            .unwrap_or(false)
    }

    pub fn get_badges(env: Env, mentor: Address) -> Vec<BadgeType> {
        env.storage()
            .persistent()
            .get(&DataKey::MentorBadges(mentor))
            .unwrap_or_else(|| vec![&env])
    }

    pub fn get_badge_count(env: Env, badge_type: BadgeType) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::BadgeCount(badge_type))
            .unwrap_or(0)
    }

    pub fn mint_badge_anonymous(
        env: Env,
        admin: Address,
        nullifier: BytesN<32>,
        badge_type_hash: BytesN<32>,
    ) {
        admin.require_auth();

        if env.storage().persistent().has(&DataKey::BadgeNullifier(nullifier.clone())) {
            panic!("nullifier already used");
        }

        env.storage()
            .persistent()
            .set(&DataKey::BadgeNullifier(nullifier.clone()), &badge_type_hash);

        let proof = ZKProof {
            scheme: symbol_short!("groth16"),
            circuit_hash: badge_type_hash.clone(),
            proof_hash: badge_type_hash.clone(),
            nullifier: nullifier.clone(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::BadgeProof(nullifier.clone()), &proof);
        let audit = audit_privacy(&proof, 1_500, env.ledger().timestamp());
        env.storage()
            .persistent()
            .set(&DataKey::BadgePrivacyAudit(nullifier.clone()), &audit);

        BadgeAnonMintEvent {
            action: symbol_short!("anon_mint"),
            nullifier,
            badge_type_hash,
        }.publish(env);
    }

    pub fn prove_badge(
        env: Env,
        nullifier: BytesN<32>,
        badge_type_hash: BytesN<32>,
        _challenge: BytesN<32>,
    ) -> bool {
        let stored: BytesN<32> = match env
            .storage()
            .persistent()
            .get(&DataKey::BadgeNullifier(nullifier))
        {
            Some(h) => h,
            None => return false,
        };
        stored == badge_type_hash
    }

    pub fn get_privacy_audit(env: Env, nullifier: BytesN<32>) -> Option<PrivacyAudit> {
        env.storage()
            .persistent()
            .get(&DataKey::BadgePrivacyAudit(nullifier))
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use super::badge_types;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Bytes, BytesN};

    fn deploy(env: &Env) -> (BadgesClient, Address, Address, Address) {
        let contract_id = env.register_contract(None, Badges);
        let c = BadgesClient::new(env, &contract_id);
        let admin = Address::generate(env);
        let backend = Address::generate(env);
        let mentor = Address::generate(env);
        c.initialize(&admin, &backend);
        (c, admin, backend, mentor)
    }

    fn compute_nullifier(env: &Env, address: &Address, badge_name: &str, secret: &str) -> BytesN<32> {
        let mut bytes = Bytes::new(env);
        bytes.append(&address.to_xdr(env));
        bytes.append(&Bytes::from_slice(env, badge_name.as_bytes()));
        bytes.append(&Bytes::from_slice(env, secret.as_bytes()));
        env.crypto().sha256(&bytes)
    }

    #[test]
    fn test_award_and_check() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _, _, mentor) = deploy(&env);

        c.award_badge(&mentor, &BadgeType::FirstSession);
        assert!(c.has_badge(&mentor, &BadgeType::FirstSession));
        assert!(!c.has_badge(&mentor, &BadgeType::TopRated));

        let badges = c.get_badges(&mentor);
        assert_eq!(badges.len(), 1);
        assert_eq!(badges.get(0).unwrap(), BadgeType::FirstSession);

        assert_eq!(c.get_badge_count(&BadgeType::FirstSession), 1);
    }

    #[test]
    fn test_revoke() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _, _, mentor) = deploy(&env);

        c.award_badge(&mentor, &BadgeType::TopRated);
        assert_eq!(c.get_badge_count(&BadgeType::TopRated), 1);

        c.revoke_badge(&mentor, &BadgeType::TopRated);
        assert!(!c.has_badge(&mentor, &BadgeType::TopRated));
        assert_eq!(c.get_badges(&mentor).len(), 0);
        assert_eq!(c.get_badge_count(&BadgeType::TopRated), 0);
    }

    #[test]
    #[should_panic(expected = "badge already awarded")]
    fn test_duplicate_award_prevented() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _, _, mentor) = deploy(&env);

        c.award_badge(&mentor, &BadgeType::VerifiedExpert);
        c.award_badge(&mentor, &BadgeType::VerifiedExpert);
    }

    #[test]
    fn test_multiple_badges_and_count() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _, _, mentor) = deploy(&env);
        let mentor2 = Address::generate(&env);

        c.award_badge(&mentor, &BadgeType::HundredSessions);
        c.award_badge(&mentor2, &BadgeType::HundredSessions);
        c.award_badge(&mentor, &BadgeType::EarlyAdopter);

        assert_eq!(c.get_badge_count(&BadgeType::HundredSessions), 2);
        assert_eq!(c.get_badges(&mentor).len(), 2);
        assert_eq!(c.get_badges(&mentor2).len(), 1);
    }

    #[test]
    fn test_anonymous_mint_and_prove() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, admin, _backend, _mentor) = deploy(&env);

        let address = Address::generate(&env);
        let badge_name = badge_types::BADGE_FIRST_SESSION;
        let secret = "my_secret_value";
        let nullifier = compute_nullifier(&env, &address, badge_name, secret);
        let bth = badge_types::badge_type_hash(&env, badge_name);

        c.mint_badge_anonymous(&admin, &nullifier, &bth);
        assert!(c.prove_badge(&nullifier, &bth, &BytesN::from_array(&env, &[0u8; 32])));
    }

    #[test]
    fn test_anonymous_mint_wrong_secret_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, admin, _backend, _mentor) = deploy(&env);

        let address = Address::generate(&env);
        let badge_name = badge_types::BADGE_FIRST_SESSION;
        let correct_secret = "correct_secret";
        let wrong_secret = "wrong_secret";
        let nullifier = compute_nullifier(&env, &address, badge_name, correct_secret);
        let wrong_nullifier = compute_nullifier(&env, &address, badge_name, wrong_secret);
        let bth = badge_types::badge_type_hash(&env, badge_name);

        c.mint_badge_anonymous(&admin, &nullifier, &bth);
        assert!(!c.prove_badge(&wrong_nullifier, &bth, &BytesN::from_array(&env, &[0u8; 32])));
    }

    #[test]
    fn test_anonymous_mint_wrong_badge_type_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, admin, _backend, _mentor) = deploy(&env);

        let address = Address::generate(&env);
        let first_session_name = badge_types::BADGE_FIRST_SESSION;
        let top_rated_name = badge_types::BADGE_TOP_RATED;
        let secret = "my_secret";
        let nullifier = compute_nullifier(&env, &address, first_session_name, secret);
        let first_bth = badge_types::badge_type_hash(&env, first_session_name);
        let wrong_bth = badge_types::badge_type_hash(&env, top_rated_name);

        c.mint_badge_anonymous(&admin, &nullifier, &first_bth);
        assert!(!c.prove_badge(&nullifier, &wrong_bth, &BytesN::from_array(&env, &[0u8; 32])));
    }

    #[test]
    fn test_regular_mint_still_works() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _, backend, mentor) = deploy(&env);

        c.award_badge(&mentor, &BadgeType::CommunityLeader);
        assert!(c.has_badge(&mentor, &BadgeType::CommunityLeader));
        assert_eq!(c.get_badge_count(&BadgeType::CommunityLeader), 1);
    }

    #[test]
    #[should_panic(expected = "nullifier already used")]
    fn test_duplicate_nullifier_prevented() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, admin, _backend, _mentor) = deploy(&env);

        let address = Address::generate(&env);
        let nullifier = compute_nullifier(&env, &address, badge_types::BADGE_FIRST_SESSION, "secret");
        let bth = badge_types::badge_type_hash(&env, badge_types::BADGE_FIRST_SESSION);

        c.mint_badge_anonymous(&admin, &nullifier, &bth);

        // This should panic with "nullifier already used"
        c.mint_badge_anonymous(&admin, &nullifier, &bth);
    }
}

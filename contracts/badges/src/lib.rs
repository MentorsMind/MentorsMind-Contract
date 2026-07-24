#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, vec, Address, Env, Vec,
};

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
    Admin,
    Backend,
    /// Whether a mentor holds a specific badge: (Address, BadgeType) -> bool
    MentorBadge(Address, BadgeType),
    /// All active badge types for a mentor: Address -> Vec<BadgeType>
    MentorBadges(Address),
    /// Total holders of a badge type: BadgeType -> u32
    BadgeCount(BadgeType),
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

    /// Award a badge to a mentor. Platform backend only. No-op if already held.
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

        // Append to mentor's badge list
        let list_key = DataKey::MentorBadges(mentor.clone());
        let mut badges: Vec<BadgeType> = env
            .storage()
            .persistent()
            .get(&list_key)
            .unwrap_or_else(|| vec![&env]);
        badges.push_back(badge_type.clone());
        env.storage().persistent().set(&list_key, &badges);

        // Increment global count
        let count_key = DataKey::BadgeCount(badge_type.clone());
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        env.storage().persistent().set(&count_key, &(count + 1));

        env.events()
            .publish((symbol_short!("badge_aw"), mentor), badge_type);
    }

    /// Revoke a badge from a mentor. Admin only.
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

        // Remove from mentor's badge list
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

        // Decrement global count
        let count_key = DataKey::BadgeCount(badge_type.clone());
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&count_key, &count.saturating_sub(1));

        env.events()
            .publish((symbol_short!("badge_rv"), mentor), badge_type);
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
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    fn deploy(env: &Env) -> (BadgesClient, Address, Address, Address) {
        let contract_id = env.register_contract(None, Badges);
        let c = BadgesClient::new(env, &contract_id);
        let admin = Address::generate(env);
        let backend = Address::generate(env);
        let mentor = Address::generate(env);
        c.initialize(&admin, &backend);
        (c, admin, backend, mentor)
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
}

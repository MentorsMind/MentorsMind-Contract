#![no_std]

use shared::EscrowRecord;
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, BytesN, Env, IntoVal, Symbol,
};

// ── Storage keys ────────────────────────────────────────────────────────────
const ESCROW: Symbol = symbol_short!("ESCROW");
const TTL_THRESHOLD: u32 = 500_000;
const TTL_BUMP: u32 = 1_000_000;

// ── Types ────────────────────────────────────────────────────────────────────
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewRecord {
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub rating: u32,
    pub timestamp: u64,
    pub comment_hash: BytesN<32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Review(Symbol),
    MentorRatingSum(Address),
    MentorReviewCount(Address),
    LoyaltyPoints(Address),
    LoyaltyTier(Address),
}

pub const TIER_SILVER: u32 = 100;
pub const TIER_GOLD: u32 = 500;
pub const TIER_PLATINUM: u32 = 1000;

// ── Contract ─────────────────────────────────────────────────────────────────
#[contract]
pub struct ReputationContract;

#[contractimpl]
impl ReputationContract {
    /// Initialize with the escrow contract address for cross-contract verification.
    pub fn initialize(env: Env, escrow_contract: Address) {
        if env.storage().instance().has(&ESCROW) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&ESCROW, &escrow_contract);
        env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);
    }

    /// Submit a review for a completed session.
    /// Caller must be the learner; session must be Released in escrow.
    pub fn submit_review(
        env: Env,
        session_id: Symbol,
        mentor: Address,
        learner: Address,
        rating: u32,
        comment_hash: BytesN<32>,
    ) {
        // Auth: caller must be learner
        learner.require_auth();

        // Validate rating 1–5
        if rating < 1 || rating > 5 {
            panic!("InvalidRating");
        }

        // Prevent duplicate review
        let review_key = DataKey::Review(session_id.clone());
        if env.storage().persistent().has(&review_key) {
            panic!("DuplicateReview");
        }

        // Cross-contract: verify session is Released
        let escrow_addr: Address = env
            .storage()
            .instance()
            .get(&ESCROW)
            .expect("EscrowContractNotSet");

        let escrow: EscrowRecord = env.invoke_contract(
            &escrow_addr,
            &Symbol::new(&env, "get_escrow_by_session"),
            (session_id.clone(),).into_val(&env),
        );

        if escrow.status != shared::EscrowStatus::Released {
            panic!("SessionNotReleased");
        }

        // Store review
        let record = ReviewRecord {
            session_id: session_id.clone(),
            mentor: mentor.clone(),
            learner: learner.clone(),
            rating,
            timestamp: env.ledger().timestamp(),
            comment_hash,
        };
        env.storage().persistent().set(&review_key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&review_key, TTL_THRESHOLD, TTL_BUMP);

        // Update running average
        let sum_key = DataKey::MentorRatingSum(mentor.clone());
        let cnt_key = DataKey::MentorReviewCount(mentor.clone());

        let current_sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0u64);
        let current_count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0u64);

        let new_sum = current_sum.checked_add(rating as u64).expect("sum overflow");
        let new_count = current_count.checked_add(1).expect("count overflow");

        env.storage().persistent().set(&sum_key, &new_sum);
        env.storage()
            .persistent()
            .extend_ttl(&sum_key, TTL_THRESHOLD, TTL_BUMP);
        env.storage().persistent().set(&cnt_key, &new_count);
        env.storage()
            .persistent()
            .extend_ttl(&cnt_key, TTL_THRESHOLD, TTL_BUMP);

        // Emit event
        env.events().publish(
            (
                symbol_short!("review"),
                Symbol::new(&env, "review_submitted"),
                mentor.clone(),
            ),
            (session_id, learner, rating, env.ledger().timestamp()),
        );
    }

    /// Returns (avg_rating * 100, review_count) for a mentor.
    pub fn get_mentor_rating(env: Env, mentor: Address) -> (u64, u64) {
        let sum_key = DataKey::MentorRatingSum(mentor.clone());
        let cnt_key = DataKey::MentorReviewCount(mentor.clone());

        let sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0);
        let count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0);

        if count == 0 {
            return (0, 0);
        }

        let avg_times_100 = (sum * 100) / count;
        (avg_times_100, count)
    }

    /// Returns the review record for a given session.
    pub fn get_review(env: Env, session_id: Symbol) -> ReviewRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Review(session_id))
            .expect("Review not found")
    }


    pub fn calculate_average_rating(env: Env, user: Address) -> u32 {
        let key = (symbol_short!("AvgRating"), user.clone());
        env.storage().persistent().get(&key).unwrap_or(0u32)
    }
    
    /// Accrue loyalty points for a user and update their tier (#463).
    pub fn accrue_loyalty_points(env: Env, user: Address, points: u32) {
        let current: u32 = env.storage().persistent().get(&DataKey::LoyaltyPoints(user.clone())).unwrap_or(0);
        let total = current + points;
        env.storage().persistent().set(&DataKey::LoyaltyPoints(user.clone()), &total);
        let tier = if total >= TIER_PLATINUM { 3u32 } else if total >= TIER_GOLD { 2u32 } else if total >= TIER_SILVER { 1u32 } else { 0u32 };
        env.storage().persistent().set(&DataKey::LoyaltyTier(user.clone()), &tier);
        env.events().publish(("loyalty_points_accrued", user), (total, tier));
    }

    pub fn get_loyalty_points(env: Env, user: Address) -> u32 {
        env.storage().persistent().get(&DataKey::LoyaltyPoints(user)).unwrap_or(0)
    }

    pub fn get_loyalty_tier(env: Env, user: Address) -> u32 {
        env.storage().persistent().get(&DataKey::LoyaltyTier(user)).unwrap_or(0)
    }

    /// Returns discount in bps based on loyalty tier (0=0%, 1=5%, 2=10%, 3=15%).
    pub fn get_loyalty_discount_bps(env: Env, user: Address) -> u32 {
        let tier: u32 = env.storage().persistent().get(&DataKey::LoyaltyTier(user)).unwrap_or(0);
        match tier { 1 => 500, 2 => 1000, 3 => 1500, _ => 0 }
    }

    pub fn update_reputation(env: Env, user: Address, new_rating: u32) {
        let key = (symbol_short!("AvgRating"), user.clone());
        let current: u32 = env.storage().persistent().get(&key).unwrap_or(0u32);
        let updated = if current == 0 { new_rating } else { (current + new_rating) / 2 };
        env.storage().persistent().set(&key, &updated);
        env.events().publish((symbol_short!("Reputation"), symbol_short!("updated")), (user, updated));
    }

}

// ── Tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Env,
    };

    // Mock escrow contract for testing
    #[contract]
    pub struct MockEscrow;

    #[contractimpl]
    impl MockEscrow {
        pub fn set_status(env: Env, session_id: Symbol, released: bool) {
            env.storage().persistent().set(&session_id, &released);
        }

        pub fn get_escrow_by_session(env: Env, session_id: Symbol) -> EscrowRecord {
            let released: bool = env.storage().persistent().get(&session_id).unwrap_or(false);
            let dummy = Address::generate(&env);
            EscrowRecord {
                id: 1,
                mentor: dummy.clone(),
                learner: dummy.clone(),
                amount: 100,
                session_id: session_id.clone(),
                status: if released {
                    shared::EscrowStatus::Released
                } else {
                    shared::EscrowStatus::Active
                },
                created_at: 0,
                token_address: dummy.clone(),
                platform_fee: 0,
                net_amount: 100,
                session_end_time: 0,
                auto_release_delay: 0,
                dispute_reason: Symbol::new(&env, ""),
                resolved_at: 0,
                usd_amount: 0,
                quoted_token_amount: 0,
                send_asset: dummy.clone(),
                dest_asset: dummy.clone(),
                total_sessions: 1,
                sessions_completed: 1,
            }
        }
    }

    fn setup() -> (
        Env,
        ReputationContractClient<'static>,
        Address,
        Address,
        Address,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 1_000_000);

        let escrow_id = env.register_contract(None, MockEscrow);
        let rep_id = env.register_contract(None, ReputationContract);
        let client = ReputationContractClient::new(&env, &rep_id);
        client.initialize(&escrow_id);

        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);

        (env, client, escrow_id, mentor, learner)
    }

    #[test]
    fn test_submit_review_success() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "session1");
        let comment_hash = BytesN::from_array(&env, &[1u8; 32]);

        // Mark session as released in mock escrow
        let mock = MockEscrowClient::new(&env, &escrow_id);
        mock.set_status(&session_id, &true);

        client.submit_review(&session_id, &mentor, &learner, &5, &comment_hash);

        let (avg, count) = client.get_mentor_rating(&mentor);
        assert_eq!(count, 1);
        assert_eq!(avg, 500); // 5 * 100

        let review = client.get_review(&session_id);
        assert_eq!(review.rating, 5);
        assert_eq!(review.mentor, mentor);
        assert_eq!(review.learner, learner);
    }

    #[test]
    fn test_running_average() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let mock = MockEscrowClient::new(&env, &escrow_id);
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);

        for i in 1u32..=3 {
            let sid = match i {
                1 => Symbol::new(&env, "s1"),
                2 => Symbol::new(&env, "s2"),
                _ => Symbol::new(&env, "s3"),
            };
            mock.set_status(&sid, &true);
            client.submit_review(&sid, &mentor, &learner, &(i * 2).min(5), &comment_hash);
        }

        let (avg, count) = client.get_mentor_rating(&mentor);
        assert_eq!(count, 3);
        // ratings: 2, 4, 5 → sum=11, avg*100 = 1100/3 = 366
        assert_eq!(avg, 366);
    }

    #[test]
    #[should_panic(expected = "InvalidRating")]
    fn test_invalid_rating() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "s_bad");
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);
        let mock = MockEscrowClient::new(&env, &escrow_id);
        mock.set_status(&session_id, &true);
        client.submit_review(&session_id, &mentor, &learner, &6, &comment_hash);
    }

    #[test]
    #[should_panic(expected = "SessionNotReleased")]
    fn test_session_not_released() {
        let (env, client, _escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "s_active");
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);
        // Not marking as released → status stays Active
        client.submit_review(&session_id, &mentor, &learner, &4, &comment_hash);
    }

    #[test]
    #[should_panic(expected = "DuplicateReview")]
    fn test_duplicate_review() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "s_dup");
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);
        let mock = MockEscrowClient::new(&env, &escrow_id);
        mock.set_status(&session_id, &true);
        client.submit_review(&session_id, &mentor, &learner, &3, &comment_hash);
        client.submit_review(&session_id, &mentor, &learner, &4, &comment_hash);
    }
}

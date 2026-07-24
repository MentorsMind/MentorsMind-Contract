#![no_std]

use shared::ReentrancyGuard;
use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RefereeType {
    Mentor,
    Learner,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralInfo {
    pub referrer: Address,
    pub referee_type: RefereeType,
    pub completed: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralRegisteredEventData {
    pub referee: Address,
    pub is_mentor: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RewardClaimedEventData {
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralConfig {
    /// Maximum multiplier allowed in basis points (e.g. 20000 = 2x).
    pub max_multiplier_bps: u32,
    /// Maximum lifetime MNT a single referrer can ever claim (in raw units).
    pub max_lifetime_reward: i128,
    /// Global cap on total MNT that can ever be minted through referrals.
    pub global_referral_mint_cap: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    MNTToken,
    LeaderboardContract,
    Config,
    Referral(Address),       // referee -> ReferralInfo
    ReferrerCount(Address),
    PendingReward(Address),  // referrer -> amount
    LifetimeClaimed(Address), // referrer -> total ever claimed
    GlobalMinted,            // i128: total minted through referrals
}

const REWARD_MENTOR: i128 = 50 * 10_000_000; // 50 MNT (7 decimals)
const REWARD_LEARNER: i128 = 20 * 10_000_000; // 20 MNT (7 decimals)

/// Default config values used when none is set at initialize time.
/// max_multiplier_bps = 20000 (2x), matching the leaderboard top tier.
/// max_lifetime_reward = 10_000 MNT per referrer.
/// global_referral_mint_cap = 5_000_000 MNT (5% of the 100M supply cap).
const DEFAULT_MAX_MULTIPLIER_BPS: u32 = 20_000;
const DEFAULT_MAX_LIFETIME_REWARD: i128 = 10_000 * 10_000_000;
const DEFAULT_GLOBAL_REFERRAL_MINT_CAP: i128 = 5_000_000 * 10_000_000;

#[contract]
pub struct ReferralContract;

#[contractimpl]
impl ReferralContract {
    pub fn initialize(env: Env, admin: Address, mnt_token: Address, leaderboard: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::MNTToken, &mnt_token);
        env.storage().persistent().set(&DataKey::LeaderboardContract, &leaderboard);

        let config = ReferralConfig {
            max_multiplier_bps: DEFAULT_MAX_MULTIPLIER_BPS,
            max_lifetime_reward: DEFAULT_MAX_LIFETIME_REWARD,
            global_referral_mint_cap: DEFAULT_GLOBAL_REFERRAL_MINT_CAP,
        };
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::GlobalMinted, &0i128);
    }

    /// Update referral config. Admin only.
    pub fn set_config(env: Env, config: ReferralConfig) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        if config.max_multiplier_bps == 0 {
            panic!("max_multiplier_bps must be > 0");
        }
        if config.max_lifetime_reward <= 0 {
            panic!("max_lifetime_reward must be positive");
        }
        if config.global_referral_mint_cap <= 0 {
            panic!("global_referral_mint_cap must be positive");
        }
        env.storage().instance().set(&DataKey::Config, &config);
    }

    pub fn register_referral(env: Env, referrer: Address, referee: Address, is_mentor: bool) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        if referrer == referee {
            panic!("Self-referral not allowed");
        }

        if env
            .storage()
            .persistent()
            .has(&DataKey::Referral(referee.clone()))
        {
            panic!("Referee already registered");
        }

        let referee_type = if is_mentor {
            RefereeType::Mentor
        } else {
            RefereeType::Learner
        };

        let info = ReferralInfo {
            referrer: referrer.clone(),
            referee_type,
            completed: false,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Referral(referee.clone()), &info);

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ReferrerCount(referrer.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::ReferrerCount(referrer.clone()), &(count + 1));

        env.events().publish(
            (
                Symbol::new(&env, "Referral"),
                Symbol::new(&env, "Registered"),
                referrer.clone(),
            ),
            ReferralRegisteredEventData { referee, is_mentor },
        );
    }

    pub fn fulfill_referral(env: Env, referee: Address) {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "fulfill"));
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        let mut info: ReferralInfo = env
            .storage()
            .persistent()
            .get(&DataKey::Referral(referee.clone()))
            .expect("Referral not found");
        if info.completed {
            panic!("Already completed");
        }

        info.completed = true;
        env.storage()
            .persistent()
            .set(&DataKey::Referral(referee.clone()), &info);

        let reward = match info.referee_type {
            RefereeType::Mentor => REWARD_MENTOR,
            RefereeType::Learner => REWARD_LEARNER,
        };

        let mut pending: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PendingReward(info.referrer.clone()))
            .unwrap_or(0);
        pending += reward;
        env.storage()
            .persistent()
            .set(&DataKey::PendingReward(info.referrer.clone()), &pending);

        // Update leaderboard
        let leaderboard: Address = env
            .storage()
            .persistent()
            .get(&DataKey::LeaderboardContract)
            .expect("Leaderboard not set");
        let count = Self::get_referral_count(env.clone(), info.referrer.clone());
        env.invoke_contract::<()>(
            &leaderboard,
            &Symbol::new(&env, "record_referral"),
            (info.referrer, count).into_val(&env),
        );
    }

    pub fn claim_reward(env: Env, referrer: Address) {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "claim_reward"));
        referrer.require_auth();

        let pending: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PendingReward(referrer.clone()))
            .unwrap_or(0);
        if pending <= 0 {
            panic!("No rewards to claim");
        }

        let config: ReferralConfig = env
            .storage()
            .instance()
            .get(&DataKey::Config)
            .expect("Config not set");

        // --- multiplier: clamp to max_multiplier_bps ---
        let leaderboard: Address = env
            .storage()
            .persistent()
            .get(&DataKey::LeaderboardContract)
            .expect("Leaderboard not set");
        let raw_multiplier: u32 = env.invoke_contract(
            &leaderboard,
            &Symbol::new(&env, "get_multiplier"),
            (referrer.clone(),).into_val(&env),
        );
        let multiplier = raw_multiplier.min(config.max_multiplier_bps);

        let actual_amount = (pending * multiplier as i128) / 10_000;

        // --- per-address lifetime cap ---
        let lifetime_claimed: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::LifetimeClaimed(referrer.clone()))
            .unwrap_or(0);
        let remaining_lifetime = config
            .max_lifetime_reward
            .checked_sub(lifetime_claimed)
            .unwrap_or(0);
        if remaining_lifetime <= 0 {
            panic!("Lifetime reward cap reached");
        }
        let actual_amount = actual_amount.min(remaining_lifetime);

        // --- global referral mint cap ---
        let global_minted: i128 = env
            .storage()
            .instance()
            .get(&DataKey::GlobalMinted)
            .unwrap_or(0);
        let global_remaining = config
            .global_referral_mint_cap
            .checked_sub(global_minted)
            .unwrap_or(0);
        if global_remaining <= 0 {
            panic!("Global referral mint cap reached");
        }
        let actual_amount = actual_amount.min(global_remaining);

        if actual_amount <= 0 {
            panic!("Computed reward is zero");
        }

        // --- checks-effects-interactions: clear state BEFORE external calls ---
        // Clearing pending and updating caps before mint prevents double-spend
        // if a malicious token contract re-enters claim_reward during mint.
        env.storage()
            .persistent()
            .set(&DataKey::PendingReward(referrer.clone()), &0i128);
        env.storage().persistent().set(
            &DataKey::LifetimeClaimed(referrer.clone()),
            &(lifetime_claimed + actual_amount),
        );
        env.storage()
            .instance()
            .set(&DataKey::GlobalMinted, &(global_minted + actual_amount));

        // --- mint (external call happens after all state is committed) ---
        let mnt_token: Address = env
            .storage()
            .persistent()
            .get(&DataKey::MNTToken)
            .expect("Token not set");
        env.invoke_contract::<()>(
            &mnt_token,
            &Symbol::new(&env, "mint"),
            (referrer.clone(), actual_amount).into_val(&env),
        );

        env.events().publish(
            (
                Symbol::new(&env, "Referral"),
                Symbol::new(&env, "RewardClaimed"),
                referrer.clone(),
            ),
            RewardClaimedEventData { amount: actual_amount },
        );
    }

    pub fn get_referral_count(env: Env, referrer: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ReferrerCount(referrer))
            .unwrap_or(0)
    }

    pub fn get_pending_rewards(env: Env, referrer: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::PendingReward(referrer))
            .unwrap_or(0)
    }

    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized")
    }

    /// Total MNT minted through referrals so far.
    pub fn get_global_referral_minted(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::GlobalMinted)
            .unwrap_or(0)
    }

    /// Distribute a portion of platform fees as referral rewards.
    pub fn distribute_from_fee(
        env: Env,
        referrer: Address,
        platform_fee: i128,
        reward_bps: u32,
    ) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        if platform_fee <= 0 || reward_bps == 0 {
            return;
        }

        let reward = platform_fee
            .checked_mul(reward_bps as i128)
            .expect("overflow")
            .checked_div(10_000)
            .expect("division error");

        if reward <= 0 {
            return;
        }

        let mut pending: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PendingReward(referrer.clone()))
            .unwrap_or(0);
        pending = pending.checked_add(reward).expect("overflow");
        env.storage()
            .persistent()
            .set(&DataKey::PendingReward(referrer.clone()), &pending);

        env.events().publish(
            (
                Symbol::new(&env, "Referral"),
                Symbol::new(&env, "FeeReward"),
                referrer,
            ),
            (reward,),
        );
    }
}

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use mentorminds_mnt_token::{MNTToken, MNTTokenClient};
    use mentorminds_referral_leaderboard::{ReferralLeaderboardContract, ReferralLeaderboardContractClient};
    use soroban_sdk::testutils::{Address as _, Events};
    use soroban_sdk::{IntoVal, Symbol, TryFromVal};

    struct TestFixture {
        env: Env,
        mnt_id: Address,
        ref_id: Address,
        leaderboard_id: Address,
        admin: Address,
    }

    impl TestFixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let mnt_id = env.register_contract(None, MNTToken);
            let leaderboard_id = env.register_contract(None, ReferralLeaderboardContract);
            let ref_id = env.register_contract(None, ReferralContract);

            let mnt_client = MNTTokenClient::new(&env, &mnt_id);
            mnt_client.initialize(&ref_id);

            let leaderboard_client = ReferralLeaderboardContractClient::new(&env, &leaderboard_id);
            leaderboard_client.initialize(&ref_id);

            let ref_client = ReferralContractClient::new(&env, &ref_id);
            ref_client.initialize(&admin, &mnt_id, &leaderboard_id);

            TestFixture { env, mnt_id, ref_id, leaderboard_id, admin }
        }

        fn client(&self) -> ReferralContractClient {
            ReferralContractClient::new(&self.env, &self.ref_id)
        }

        fn mnt_client(&self) -> MNTTokenClient {
            MNTTokenClient::new(&self.env, &self.mnt_id)
        }
    }

    #[test]
    fn test_initialization() {
        let f = TestFixture::setup();
        assert_eq!(f.client().get_referral_count(&Address::generate(&f.env)), 0);
        assert_eq!(f.client().get_global_referral_minted(), 0);
    }

    #[test]
    fn test_referral_flow() {
        let f = TestFixture::setup();
        let referrer = Address::generate(&f.env);
        let referee = Address::generate(&f.env);

        f.client().register_referral(&referrer, &referee, &true);
        assert_eq!(f.client().get_referral_count(&referrer), 1);
        assert_eq!(f.client().get_pending_rewards(&referrer), 0);

        let events = f.env.events().all();
        let last_event = events.last().unwrap();
        assert_eq!(last_event.0, f.ref_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&f.env, "Referral"),
                Symbol::new(&f.env, "Registered"),
                referrer.clone()
            )
                .into_val(&f.env)
        );
        let payload = ReferralRegisteredEventData::try_from_val(&f.env, &last_event.2)
            .expect("registered payload should decode");
        assert_eq!(payload, ReferralRegisteredEventData { referee: referee.clone(), is_mentor: true });

        f.client().fulfill_referral(&referee);
        assert_eq!(f.client().get_pending_rewards(&referrer), REWARD_MENTOR);

        f.client().claim_reward(&referrer);
        assert_eq!(f.client().get_pending_rewards(&referrer), 0);
        // Leaderboard rank 1 → 2x, capped at max_multiplier_bps (20000 = 2x)
        assert_eq!(f.mnt_client().balance(&referrer), REWARD_MENTOR * 2);
        assert_eq!(f.client().get_global_referral_minted(), REWARD_MENTOR * 2);

        let events2 = f.env.events().all();
        let last_event2 = events2.last().unwrap();
        assert_eq!(last_event2.0, f.ref_id.clone());
        assert_eq!(
            last_event2.1,
            (
                Symbol::new(&f.env, "Referral"),
                Symbol::new(&f.env, "RewardClaimed"),
                referrer.clone()
            )
                .into_val(&f.env)
        );
        let payload2 = RewardClaimedEventData::try_from_val(&f.env, &last_event2.2)
            .expect("reward payload should decode");
        assert_eq!(payload2, RewardClaimedEventData { amount: REWARD_MENTOR * 2 });
    }

    #[test]
    #[should_panic(expected = "Self-referral not allowed")]
    fn test_self_referral_rejection() {
        let f = TestFixture::setup();
        let user = Address::generate(&f.env);
        f.client().register_referral(&user, &user, &true);
    }

    #[test]
    #[should_panic(expected = "Referee already registered")]
    fn test_duplicate_referral_rejection() {
        let f = TestFixture::setup();
        let referrer1 = Address::generate(&f.env);
        let referrer2 = Address::generate(&f.env);
        let referee = Address::generate(&f.env);

        f.client().register_referral(&referrer1, &referee, &true);
        f.client().register_referral(&referrer2, &referee, &false);
    }

    /// Multiplier above max_multiplier_bps is clamped, not accepted.
    #[test]
    fn test_multiplier_clamped_at_max() {
        let f = TestFixture::setup();

        // Set a tight config: max multiplier 1.5x (15000 bps)
        f.client().set_config(&ReferralConfig {
            max_multiplier_bps: 15_000,
            max_lifetime_reward: DEFAULT_MAX_LIFETIME_REWARD,
            global_referral_mint_cap: DEFAULT_GLOBAL_REFERRAL_MINT_CAP,
        });

        let referrer = Address::generate(&f.env);
        let referee = Address::generate(&f.env);

        f.client().register_referral(&referrer, &referee, &true);
        f.client().fulfill_referral(&referee);
        // rank 1 → leaderboard would give 20000 (2x), but cap is 15000 (1.5x)
        f.client().claim_reward(&referrer);

        let expected = (REWARD_MENTOR * 15_000) / 10_000;
        assert_eq!(f.mnt_client().balance(&referrer), expected);
    }

    /// Referrer cannot claim beyond max_lifetime_reward.
    #[test]
    #[should_panic(expected = "Lifetime reward cap reached")]
    fn test_lifetime_reward_cap_enforced() {
        let f = TestFixture::setup();

        // Set tiny lifetime cap so it's exhausted after one claim
        f.client().set_config(&ReferralConfig {
            max_multiplier_bps: DEFAULT_MAX_MULTIPLIER_BPS,
            max_lifetime_reward: REWARD_MENTOR * 2, // exactly one 2x claim
            global_referral_mint_cap: DEFAULT_GLOBAL_REFERRAL_MINT_CAP,
        });

        let referrer = Address::generate(&f.env);

        // First claim — exhausts lifetime cap
        let referee1 = Address::generate(&f.env);
        f.client().register_referral(&referrer, &referee1, &true);
        f.client().fulfill_referral(&referee1);
        f.client().claim_reward(&referrer);

        // Second claim — must panic
        let referee2 = Address::generate(&f.env);
        f.client().register_referral(&referrer, &referee2, &true);
        f.client().fulfill_referral(&referee2);
        f.client().claim_reward(&referrer); // should panic
    }

    /// Global cap: the claim that would exceed the cap is rejected.
    #[test]
    fn test_global_cap_enforced() {
        let f = TestFixture::setup();

        // Cap = exactly two 2x mentor rewards
        let cap = REWARD_MENTOR * 2 * 2;
        f.client().set_config(&ReferralConfig {
            max_multiplier_bps: DEFAULT_MAX_MULTIPLIER_BPS,
            max_lifetime_reward: DEFAULT_MAX_LIFETIME_REWARD,
            global_referral_mint_cap: cap,
        });

        let referrer1 = Address::generate(&f.env);
        let referee1 = Address::generate(&f.env);
        f.client().register_referral(&referrer1, &referee1, &true);
        f.client().fulfill_referral(&referee1);
        f.client().claim_reward(&referrer1);

        let referrer2 = Address::generate(&f.env);
        let referee2 = Address::generate(&f.env);
        f.client().register_referral(&referrer2, &referee2, &true);
        f.client().fulfill_referral(&referee2);
        f.client().claim_reward(&referrer2);

        // Global minted should equal the cap
        assert_eq!(f.client().get_global_referral_minted(), cap);

        // Third referrer — global cap exhausted, claim must panic
        let referrer3 = Address::generate(&f.env);
        let referee3 = Address::generate(&f.env);
        f.client().register_referral(&referrer3, &referee3, &true);
        f.client().fulfill_referral(&referee3);

        let result = std::panic::catch_unwind(|| {
            f.client().claim_reward(&referrer3);
        });
        assert!(result.is_err(), "Expected panic when global cap is reached");
    }

    /// MNT total supply after all referral claims ≤ supply cap (100M).
    #[test]
    fn test_supply_invariant_held() {
        let f = TestFixture::setup();
        const MNT_SUPPLY_CAP: i128 = 100_000_000 * 10_000_000;

        // Use the default global referral mint cap (5M MNT)
        let referrer = Address::generate(&f.env);
        let referee = Address::generate(&f.env);
        f.client().register_referral(&referrer, &referee, &true);
        f.client().fulfill_referral(&referee);
        f.client().claim_reward(&referrer);

        assert!(f.mnt_client().balance(&referrer) <= MNT_SUPPLY_CAP);
        assert!(f.client().get_global_referral_minted() <= DEFAULT_GLOBAL_REFERRAL_MINT_CAP);
    }

    /// Sybil test: 1000 self-referred addresses (via different referrers) cannot exceed the global cap.
    #[test]
    fn test_sybil_resistance_1000_addresses() {
        let f = TestFixture::setup();

        // Set a small global cap to make the invariant observable
        let global_cap = 100 * REWARD_LEARNER * 2; // 100 learner 2x claims worth
        f.client().set_config(&ReferralConfig {
            max_multiplier_bps: DEFAULT_MAX_MULTIPLIER_BPS,
            max_lifetime_reward: DEFAULT_MAX_LIFETIME_REWARD,
            global_referral_mint_cap: global_cap,
        });

        let mut total_minted: i128 = 0;
        let mut claims_accepted = 0u32;

        for _ in 0..1000 {
            let referrer = Address::generate(&f.env);
            let referee = Address::generate(&f.env);
            f.client().register_referral(&referrer, &referee, &false); // learner
            f.client().fulfill_referral(&referee);

            // Stop claiming once cap is hit; just count how many went through
            if f.client().get_global_referral_minted() >= global_cap {
                break;
            }
            f.client().claim_reward(&referrer);
            let minted = f.client().get_global_referral_minted();
            assert!(minted <= global_cap, "Global cap breached at claim {}", claims_accepted);
            total_minted = minted;
            claims_accepted += 1;
        }

        assert!(total_minted <= global_cap);
        assert!(f.client().get_global_referral_minted() <= global_cap);
    }

    // -----------------------------------------------------------------------
    // Reentrancy simulation: malicious token that calls back into claim_reward
    // -----------------------------------------------------------------------

    /// A mock MNT token that attempts to re-enter claim_reward during mint.
    /// The ReentrancyGuard must block the second call and the pending balance
    /// must be zero after a single successful claim (no double-mint).
    #[contract]
    pub struct ReentrantMockMNT;

    #[contracttype]
    #[derive(Clone)]
    pub enum MockMNTKey {
        Balance(Address),
        ReferralContract,
    }

    #[contractimpl]
    impl ReentrantMockMNT {
        /// Store the referral contract address so mint knows where to call back.
        pub fn set_referral_contract(env: Env, referral: Address) {
            env.storage().instance().set(&MockMNTKey::ReferralContract, &referral);
        }

        pub fn initialize(_env: Env, _admin: Address) {}

        pub fn mint(env: Env, to: Address, amount: i128) {
            // Record the mint in internal balance
            let bal: i128 = env
                .storage()
                .persistent()
                .get(&MockMNTKey::Balance(to.clone()))
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&MockMNTKey::Balance(to.clone()), &(bal + amount));

            // Attempt re-entrant callback into claim_reward — must be blocked
            if let Some(referral_contract) =
                env.storage().instance().get::<_, Address>(&MockMNTKey::ReferralContract)
            {
                // This call should panic with "reentrant call" because the
                // referral contract's lock is still held.
                let _result = env.invoke_contract::<()>(
                    &referral_contract,
                    &Symbol::new(&env, "claim_reward"),
                    (to.clone(),).into_val(&env),
                );
            }
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&MockMNTKey::Balance(id))
                .unwrap_or(0)
        }
    }

    /// Verify that a malicious token cannot trigger a double-mint via reentrancy.
    ///
    /// Setup:
    ///   - Deploy a ReentrantMockMNT whose `mint` calls back into `claim_reward`.
    ///   - The referral contract guards `claim_reward` with ReentrancyGuard.
    ///
    /// Expected: the outer claim succeeds; the inner callback panics with
    /// "reentrant call", causing the whole transaction to revert — the net
    /// result is that no mint ever lands (Soroban atomically rolls back on panic).
    /// We verify this by asserting the call returns an error.
    #[test]
    fn test_reentrancy_blocked_single_mint() {
        extern crate std;

        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let reentrant_mnt = env.register_contract(None, ReentrantMockMNT);
        let leaderboard_id = env.register_contract(
            None,
            mentorminds_referral_leaderboard::ReferralLeaderboardContract,
        );
        let ref_id = env.register_contract(None, ReferralContract);

        // Initialize leaderboard and referral with the malicious token
        let leaderboard_client =
            mentorminds_referral_leaderboard::ReferralLeaderboardContractClient::new(
                &env,
                &leaderboard_id,
            );
        leaderboard_client.initialize(&ref_id);

        let ref_client = ReferralContractClient::new(&env, &ref_id);
        ref_client.initialize(&admin, &reentrant_mnt, &leaderboard_id);

        // Tell the malicious token where to call back
        let mock_client = ReentrantMockMNTClient::new(&env, &reentrant_mnt);
        mock_client.set_referral_contract(&ref_id);

        // Set up a referral so there is a pending reward to claim
        let referrer = Address::generate(&env);
        let referee = Address::generate(&env);
        ref_client.register_referral(&referrer, &referee, &true);
        ref_client.fulfill_referral(&referee);

        // claim_reward will invoke malicious mint → mint tries to re-enter claim_reward
        // → ReentrancyGuard panics → entire tx reverts
        let result = ref_client.try_claim_reward(&referrer);
        assert!(
            result.is_err(),
            "Expected reentrancy to be blocked — tx should revert"
        );

        // Because the tx reverted, pending reward is still intact (no partial state)
        assert_eq!(
            ref_client.get_pending_rewards(&referrer),
            REWARD_MENTOR,
            "Pending reward must remain after reverted reentrant claim"
        );
    }
}

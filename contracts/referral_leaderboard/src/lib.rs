#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LeaderboardUpdatedEventData {
    pub top_three: Vec<Address>,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    ReferralContract,
    Leaderboard,
}

#[contract]
pub struct ReferralLeaderboardContract;

#[contractimpl]
impl ReferralLeaderboardContract {
    pub fn initialize(env: Env, referral_contract: Address) {
        if env.storage().persistent().has(&DataKey::ReferralContract) {
            panic!("Already initialized");
        }
        env.storage()
            .persistent()
            .set(&DataKey::ReferralContract, &referral_contract);
    }

    pub fn record_referral(env: Env, referrer: Address, referral_count: u32) {
        let referral_contract: Address = env
            .storage()
            .persistent()
            .get(&DataKey::ReferralContract)
            .expect("Not initialized");

        if env.invoker() != referral_contract {
            panic!("Unauthorized");
        }

        let mut leaderboard: Vec<(Address, u32)> = env
            .storage()
            .persistent()
            .get(&DataKey::Leaderboard)
            .unwrap_or(Vec::new(&env));

        // Remove existing entry for referrer if present
        leaderboard = leaderboard
            .iter()
            .filter(|(addr, _)| addr != &referrer)
            .map(|(addr, count)| (addr, count))
            .collect::<Vec<_>>(&env);

        // Add or update with new count
        leaderboard.push_back((referrer.clone(), referral_count));

        // Sort by count descending, take top 50
        leaderboard.sort_by(|a, b| b.1.cmp(&a.1));
        if leaderboard.len() > 50 {
            leaderboard = leaderboard.slice(0..50);
        }

        env.storage()
            .persistent()
            .set(&DataKey::Leaderboard, &leaderboard);

        // Emit event with top 3
        let top_three = leaderboard
            .iter()
            .take(3)
            .map(|(addr, _)| addr.clone())
            .collect::<Vec<_>>(&env);

        env.events().publish(
            (Symbol::new(&env, "Leaderboard"), Symbol::new(&env, "Updated")),
            LeaderboardUpdatedEventData { top_three },
        );
    }

    pub fn get_leaderboard(env: Env) -> Vec<(Address, u32)> {
        env.storage()
            .persistent()
            .get(&DataKey::Leaderboard)
            .unwrap_or(Vec::new(&env))
    }

    pub fn get_rank(env: Env, referrer: Address) -> Option<u32> {
        let leaderboard = Self::get_leaderboard(env);
        for (i, (addr, _)) in leaderboard.iter().enumerate() {
            if addr == referrer {
                return Some((i + 1) as u32);
            }
        }
        None
    }

    pub fn get_multiplier(env: Env, referrer: Address) -> u32 {
        match Self::get_rank(env, referrer) {
            Some(rank) if rank <= 3 => 20000, // 2x
            Some(rank) if rank <= 10 => 15000, // 1.5x
            Some(rank) if rank <= 50 => 12500, // 1.25x
            _ => 10000, // 1x
        }
    }
}

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events};
    use soroban_sdk::{IntoVal, Symbol, TryFromVal};

    struct TestFixture {
        env: Env,
        leaderboard_id: Address,
        referral_contract: Address,
    }

    impl TestFixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let referral_contract = Address::generate(&env);
            let leaderboard_id = env.register_contract(None, ReferralLeaderboardContract);

            let client = ReferralLeaderboardContractClient::new(&env, &leaderboard_id);
            client.initialize(&referral_contract);

            TestFixture {
                env,
                leaderboard_id,
                referral_contract,
            }
        }

        fn client(&self) -> ReferralLeaderboardContractClient {
            ReferralLeaderboardContractClient::new(&self.env, &self.leaderboard_id)
        }
    }

    #[test]
    fn test_initialization() {
        let f = TestFixture::setup();
        // Just check it doesn't panic
    }

    #[test]
    fn test_record_referral_empty_board() {
        let f = TestFixture::setup();
        let referrer = Address::generate(&f.env);

        // Mock the invoker
        f.env.mock_auths(&[f.referral_contract.clone()]);

        f.client().record_referral(&referrer, &1);

        let board = f.client().get_leaderboard();
        assert_eq!(board.len(), 1);
        assert_eq!(board.get(0).unwrap(), (referrer.clone(), 1));

        let rank = f.client().get_rank(&referrer);
        assert_eq!(rank, Some(1));

        let mult = f.client().get_multiplier(&referrer);
        assert_eq!(mult, 20000);
    }

    #[test]
    fn test_displace_last_entry() {
        let f = TestFixture::setup();
        f.env.mock_auths(&[f.referral_contract.clone()]);

        // Add 50 entries
        let mut referrers = Vec::new(&f.env);
        for i in 0..50 {
            let addr = Address::generate(&f.env);
            referrers.push_back(addr.clone());
            f.client().record_referral(&addr, &(50 - i));
        }

        // The board should be sorted descending: counts 50,49,...,1
        let board = f.client().get_leaderboard();
        assert_eq!(board.len(), 50);
        for i in 0..50 {
            assert_eq!(board.get(i as u32).unwrap().1, 50 - i);
        }

        // Add a new one with count 51, should displace the last (count 1)
        let new_referrer = Address::generate(&f.env);
        f.client().record_referral(&new_referrer, &51);

        let board2 = f.client().get_leaderboard();
        assert_eq!(board2.len(), 50);
        assert_eq!(board2.get(0).unwrap(), (new_referrer.clone(), 51));
        // The last should now be count 2 (since 1 was displaced)
        assert_eq!(board2.get(49).unwrap().1, 2);
    }

    #[test]
    fn test_rank_query() {
        let f = TestFixture::setup();
        f.env.mock_auths(&[f.referral_contract.clone()]);

        let addr1 = Address::generate(&f.env);
        let addr2 = Address::generate(&f.env);
        let addr3 = Address::generate(&f.env);

        f.client().record_referral(&addr1, &10);
        f.client().record_referral(&addr2, &5);
        f.client().record_referral(&addr3, &1);

        assert_eq!(f.client().get_rank(&addr1), Some(1));
        assert_eq!(f.client().get_rank(&addr2), Some(2));
        assert_eq!(f.client().get_rank(&addr3), Some(3));

        let unknown = Address::generate(&f.env);
        assert_eq!(f.client().get_rank(&unknown), None);
    }

    #[test]
    fn test_multiplier_tiers() {
        let f = TestFixture::setup();
        f.env.mock_auths(&[f.referral_contract.clone()]);

        // Add 11 referrers
        let mut addrs = Vec::new(&f.env);
        for i in 0..11 {
            let addr = Address::generate(&f.env);
            addrs.push_back(addr.clone());
            f.client().record_referral(&addr, &(11 - i));
        }

        // Rank 1-3: 20000
        for i in 0..3 {
            assert_eq!(f.client().get_multiplier(&addrs.get(i).unwrap()), 20000);
        }
        // Rank 4-10: 15000
        for i in 3..10 {
            assert_eq!(f.client().get_multiplier(&addrs.get(i).unwrap()), 15000);
        }
        // Rank 11: 12500
        assert_eq!(f.client().get_multiplier(&addrs.get(10).unwrap()), 12500);

        // Non-top: 10000
        let outsider = Address::generate(&f.env);
        assert_eq!(f.client().get_multiplier(&outsider), 10000);
    }

    #[test]
    fn test_event_emission() {
        let f = TestFixture::setup();
        f.env.mock_auths(&[f.referral_contract.clone()]);

        let referrer = Address::generate(&f.env);
        f.client().record_referral(&referrer, &1);

        let events = f.env.events().all();
        let last_event = events.last().unwrap();
        assert_eq!(last_event.0, f.leaderboard_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&f.env, "Leaderboard"),
                Symbol::new(&f.env, "Updated")
            )
                .into_val(&f.env)
        );
        let payload = LeaderboardUpdatedEventData::try_from_val(&f.env, &last_event.2)
            .expect("payload should decode");
        assert_eq!(payload.top_three.len(), 1);
        assert_eq!(payload.top_three.get(0).unwrap(), referrer);
    }
}
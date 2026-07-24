#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env, Vec};

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StreakRecord {
    pub learner: Address,
    pub current_streak: u32,
    pub longest_streak: u32,
    pub last_active_week: u32,
    pub claimed_this_week: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    MntToken,
    UserStreak(Address),
    Leaderboard,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct StreakRewardsContract;

#[contractimpl]
impl StreakRewardsContract {
    pub fn initialize(env: Env, admin: Address, mnt_token: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MntToken, &mnt_token);

        let leaderboard: Vec<(Address, u32)> = Vec::new(&env);
        env.storage()
            .persistent()
            .set(&DataKey::Leaderboard, &leaderboard);
    }

    pub fn record_session(env: Env, learner: Address, session_week: u32) {
        let mut record = Self::get_streak(env.clone(), learner.clone());

        if session_week == record.last_active_week {
            return;
        }

        if session_week == record.last_active_week + 1 {
            record.current_streak += 1;
            record.claimed_this_week = false;
        } else if session_week > record.last_active_week + 1 {
            if record.last_active_week != 0 {
                env.events().publish(
                    (
                        symbol_short!("streak"),
                        symbol_short!("broken"),
                        learner.clone(),
                    ),
                    (record.current_streak,),
                );
            }
            record.current_streak = 1;
            record.claimed_this_week = false;
        } else {
            return;
        }

        if record.current_streak > record.longest_streak {
            record.longest_streak = record.current_streak;
        }
        record.last_active_week = session_week;

        env.storage()
            .persistent()
            .set(&DataKey::UserStreak(learner.clone()), &record);

        env.events().publish(
            (
                symbol_short!("streak"),
                symbol_short!("updated"),
                learner.clone(),
            ),
            (record.current_streak,),
        );

        Self::update_leaderboard(env, learner, record.current_streak);
    }

    pub fn claim_streak_reward(env: Env, learner: Address) {
        learner.require_auth();

        let mut record = Self::get_streak(env.clone(), learner.clone());
        if record.claimed_this_week {
            panic!("Reward already claimed for this week");
        }
        if record.current_streak == 0 {
            panic!("No active streak");
        }

        let reward_amount = match record.current_streak {
            1..=3 => 5,
            4..=11 => 25,
            12..=51 => 100,
            _ => 500,
        };

        let amount_i128 = (reward_amount as i128) * 10_000_000;

        let token_addr: Address = env.storage().instance().get(&DataKey::MntToken).unwrap();
        let token_client = token::Client::new(&env, &token_addr);
        token_client.transfer(&env.current_contract_address(), &learner, &amount_i128);

        record.claimed_this_week = true;
        env.storage()
            .persistent()
            .set(&DataKey::UserStreak(learner.clone()), &record);

        env.events().publish(
            (symbol_short!("reward"), symbol_short!("claimed"), learner),
            (reward_amount,),
        );
    }

    pub fn get_streak(env: Env, learner: Address) -> StreakRecord {
        env.storage()
            .persistent()
            .get(&DataKey::UserStreak(learner.clone()))
            .unwrap_or(StreakRecord {
                learner,
                current_streak: 0,
                longest_streak: 0,
                last_active_week: 0,
                claimed_this_week: false,
            })
    }

    pub fn get_leaderboard(env: Env) -> Vec<(Address, u32)> {
        env.storage()
            .persistent()
            .get(&DataKey::Leaderboard)
            .unwrap_or(Vec::new(&env))
    }

    fn update_leaderboard(env: Env, learner: Address, streak: u32) {
        let mut leaderboard: Vec<(Address, u32)> = Self::get_leaderboard(env.clone());
        let mut found = false;

        for i in 0..leaderboard.len() {
            let (addr, _) = leaderboard.get(i).unwrap();
            if addr == learner {
                leaderboard.set(i, (learner.clone(), streak));
                found = true;
                break;
            }
        }

        if !found {
            if leaderboard.len() < 10 {
                leaderboard.push_back((learner.clone(), streak));
            } else {
                let (_, min_streak) = leaderboard.get(9).unwrap();
                if streak > min_streak {
                    leaderboard.set(9, (learner.clone(), streak));
                } else {
                    return;
                }
            }
        }

        // Simple bubble sort
        for i in 0..leaderboard.len() {
            for j in 0..leaderboard.len() - 1 - i {
                let (_, s1) = leaderboard.get(j).unwrap();
                let (_, s2) = leaderboard.get(j + 1).unwrap();
                if s1 < s2 {
                    let temp = leaderboard.get(j).unwrap();
                    leaderboard.set(j, leaderboard.get(j + 1).unwrap());
                    leaderboard.set(j + 1, temp);
                }
            }
        }

        env.storage()
            .persistent()
            .set(&DataKey::Leaderboard, &leaderboard);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[contract]
    pub struct MockToken;
    #[contractimpl]
    impl MockToken {
        pub fn transfer(_e: Env, from: Address, _to: Address, _amount: i128) {
            from.require_auth();
        }
    }

    fn setup_test(
        env: &Env,
    ) -> (
        Address,
        Address,
        Address,
        StreakRewardsContractClient<'static>,
    ) {
        let admin = Address::generate(env);
        let learner = Address::generate(env);

        let token_addr = env.register_contract(None, MockToken);
        let contract_id = env.register_contract(None, StreakRewardsContract);
        let client = StreakRewardsContractClient::new(env, &contract_id);

        client.initialize(&admin, &token_addr);

        (admin, learner, token_addr, client)
    }

    #[test]
    fn test_streak_building() {
        let env = Env::default();
        let (_, learner, _, client) = setup_test(&env);

        client.record_session(&learner, &1);
        assert_eq!(client.get_streak(&learner).current_streak, 1);

        client.record_session(&learner, &2);
        assert_eq!(client.get_streak(&learner).current_streak, 2);

        client.record_session(&learner, &4); // Missed week 3
        assert_eq!(client.get_streak(&learner).current_streak, 1);
    }

    #[test]
    fn test_leaderboard() {
        let env = Env::default();
        let (_, _, _, client) = setup_test(&env);

        for _i in 0..5 {
            let u = Address::generate(&env);
            client.record_session(&u, &1);
        }

        let lb = client.get_leaderboard();
        assert_eq!(lb.len(), 5);
    }

    #[test]
    #[should_panic(expected = "Reward already claimed")]
    fn test_claim_twice_fails() {
        let env = Env::default();
        let (_, learner, _, client) = setup_test(&env);

        client.record_session(&learner, &1);

        env.mock_all_auths();
        client.claim_streak_reward(&learner);
        client.claim_streak_reward(&learner);
    }
}

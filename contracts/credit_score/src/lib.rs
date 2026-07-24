#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, IntoVal, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// External Interfaces (Types only for contract calls)
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EscrowStatus {
    Active,
    Released,
    Disputed,
    Refunded,
    Resolved,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Escrow {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub session_id: Symbol,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    pub platform_fee: i128,
    pub net_amount: i128,
    pub session_end_time: u64,
    pub auto_release_delay: u64,
    pub dispute_reason: Symbol,
    pub resolved_at: u64,
    pub usd_amount: i128,
    pub quoted_token_amount: i128,
    pub send_asset: Address,
    pub dest_asset: Address,
    pub total_sessions: u32,
    pub sessions_completed: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeRecord {
    pub mentor: Address,
    pub amount: i128,
    pub staked_at: u64,
    pub unlock_at: u64,
    pub tier: u32,
}

// ---------------------------------------------------------------------------
// Credit Score Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScoreBreakdown {
    pub payment_history: u32,
    pub session_completion: u32,
    pub account_age: u32,
    pub staking_amount: u32,
    pub dispute_history: u32,
}

#[contracttype]
pub enum DataKey {
    Admin,                  // Persistent: critical config
    EscrowContract,         // Persistent: external dependency
    StakingContract,        // Persistent: external dependency
    UserScore(Address),     // Persistent: long-term user data
    UserBreakdown(Address), // Persistent: long-term user data
    LastUpdate(Address),    // Temporary: rate limiting, auto-expires
}

const MIN_SCORE: u32 = 300;
const MAX_SCORE: u32 = 850;
const DAY_SECONDS: u64 = 86_400;
const DAY_SECONDS_TTL: u32 = 86_400;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct CreditScoreContract;

#[contractimpl]
impl CreditScoreContract {
    pub fn initialize(env: Env, admin: Address, escrow: Address, staking: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::EscrowContract, &escrow);
        env.storage()
            .persistent()
            .set(&DataKey::StakingContract, &staking);
    }

    pub fn get_score(env: Env, user: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::UserScore(user))
            .unwrap_or(MIN_SCORE)
    }

    pub fn get_score_breakdown(env: Env, user: Address) -> ScoreBreakdown {
        env.storage()
            .persistent()
            .get(&DataKey::UserBreakdown(user))
            .unwrap_or(ScoreBreakdown {
                payment_history: 0,
                session_completion: 0,
                account_age: 0,
                staking_amount: 0,
                dispute_history: 0,
            })
    }

    pub fn refresh_score(env: Env, user: Address) {
        let last_update: u64 = env
            .storage()
            .temporary()
            .get(&DataKey::LastUpdate(user.clone()))
            .unwrap_or(0);
        if env.ledger().timestamp() < last_update + DAY_SECONDS {
            panic!("Rate limited: once per day");
        }

        let (score, breakdown) = Self::do_compute(env.clone(), user.clone());

        env.storage()
            .persistent()
            .set(&DataKey::UserScore(user.clone()), &score);
        env.storage()
            .persistent()
            .set(&DataKey::UserBreakdown(user.clone()), &breakdown);
        env.storage().temporary().set(
            &DataKey::LastUpdate(user.clone()),
            &env.ledger().timestamp(),
        );
        // Extend TTL for temporary storage (1 day)
        env.storage().temporary().extend_ttl(
            &DataKey::LastUpdate(user.clone()),
            DAY_SECONDS_TTL,
            DAY_SECONDS_TTL,
        );

        env.events().publish(
            (symbol_short!("score"), symbol_short!("updated"), user),
            (score,),
        );
    }

    pub fn compute_score(env: Env, user: Address) -> u32 {
        let (score, _) = Self::do_compute(env, user);
        score
    }
}

impl CreditScoreContract {
    fn do_compute(env: Env, user: Address) -> (u32, ScoreBreakdown) {
        let escrow_addr: Address = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowContract)
            .unwrap();
        let staking_addr: Address = env
            .storage()
            .persistent()
            .get(&DataKey::StakingContract)
            .unwrap();

        // 1. Fetch historical data from Escrow
        let mentor_list: Vec<Escrow> = env.invoke_contract(
            &escrow_addr,
            &soroban_sdk::Symbol::new(&env, "get_escrows_by_mentor"),
            (user.clone(), 0u32, 50u32).into_val(&env),
        );
        let learner_list: Vec<Escrow> = env.invoke_contract(
            &escrow_addr,
            &soroban_sdk::Symbol::new(&env, "get_escrows_by_learner"),
            (user.clone(), 0u32, 50u32).into_val(&env),
        );

        let mut total_count = 0;
        let mut released_count = 0;
        let mut dispute_count = 0;
        let mut sessions_total = 0;
        let mut sessions_done = 0;
        let mut first_time = env.ledger().timestamp();

        let all_escrows = [mentor_list, learner_list];
        for list in all_escrows.iter() {
            for e in list.iter() {
                total_count += 1;
                if e.status == EscrowStatus::Released || e.status == EscrowStatus::Resolved {
                    released_count += 1;
                }
                if e.status == EscrowStatus::Disputed || e.status == EscrowStatus::Resolved {
                    dispute_count += 1;
                }
                sessions_total += e.total_sessions;
                sessions_done += e.sessions_completed;
                if e.created_at < first_time {
                    first_time = e.created_at;
                }
            }
        }

        // 2. Fetch Staking
        let stake_amount = match env.try_invoke_contract::<StakeRecord, soroban_sdk::Error>(
            &staking_addr,
            &symbol_short!("stake"),
            (user,).into_val(&env),
        ) {
            Ok(Ok(r)) => r.amount,
            _ => 0i128,
        };

        // 3. Calculation (Fixed point 10000 -> /10 for scores)
        let p_hist = if total_count > 0 {
            (released_count * 1925 / total_count) as u32
        } else {
            0
        };
        let s_comp = if sessions_total > 0 {
            (sessions_done * 1650 / sessions_total) as u32
        } else {
            0
        };
        let age_days = (env.ledger().timestamp().saturating_sub(first_time)) / 86400;
        let age_pts = if total_count > 0 {
            ((age_days as u32).min(365) * 825 / 365) as u32
        } else {
            0
        };
        let stake_pts = (stake_amount.max(0) as u32).min(2000) * 550 / 2000;
        let disp_pts = if total_count > 0 {
            ((total_count - dispute_count) * 550 / total_count) as u32
        } else {
            0
        };

        let breakdown = ScoreBreakdown {
            payment_history: p_hist / 10,
            session_completion: s_comp / 10,
            account_age: age_pts / 10,
            staking_amount: stake_pts / 10,
            dispute_history: disp_pts / 10,
        };

        let total_boost = (p_hist + s_comp + age_pts + stake_pts + disp_pts) / 10;
        let final_score = (MIN_SCORE + total_boost).min(MAX_SCORE);

        (final_score, breakdown)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[contract]
    pub struct MockEscrow;
    #[contractimpl]
    impl MockEscrow {
        pub fn mentor(env: Env, _u: Address, _p: u32, _ps: u32) -> Vec<Escrow> {
            let mut v = Vec::new(&env);
            v.push_back(Escrow {
                id: 1,
                mentor: Address::generate(&env),
                learner: Address::generate(&env),
                amount: 1000,
                session_id: symbol_short!("S1"),
                status: EscrowStatus::Released,
                created_at: env.ledger().timestamp() - (100 * 86400),
                token_address: Address::generate(&env),
                platform_fee: 0,
                net_amount: 1000,
                session_end_time: 0,
                auto_release_delay: 0,
                dispute_reason: symbol_short!("none"),
                resolved_at: 0,
                usd_amount: 0,
                quoted_token_amount: 0,
                send_asset: Address::generate(&env),
                dest_asset: Address::generate(&env),
                total_sessions: 10,
                sessions_completed: 10,
            });
            v
        }
        pub fn learner(env: Env, _u: Address, _p: u32, _ps: u32) -> Vec<Escrow> {
            Vec::new(&env)
        }
    }

    #[contract]
    pub struct MockStaking;
    #[contractimpl]
    impl MockStaking {
        pub fn stake(_env: Env, m: Address) -> StakeRecord {
            StakeRecord {
                mentor: m,
                amount: 2000,
                staked_at: 0,
                unlock_at: 0,
                tier: 3,
            }
        }
    }

    #[test]
    fn test_perfect_flow() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000_000);
        let admin = Address::generate(&env);
        let escrow = env.register_contract(None, MockEscrow);
        let staking = env.register_contract(None, MockStaking);
        let cid = env.register_contract(None, CreditScoreContract);
        let client = CreditScoreContractClient::new(&env, &cid);
        client.initialize(&admin, &escrow, &staking);

        let user = Address::generate(&env);
        client.refresh_score(&user);
        let score = client.get_score(&user);

        // Expected: 300 (base) + 192 (pay) + 165 (sess) + 22 (age) + 55 (stake) + 55 (disp) = 789
        assert!(score >= 780 && score <= 800, "Expected ~789, got {}", score);

        let breakdown = client.get_score_breakdown(&user);
        assert_eq!(breakdown.staking_amount, 55);
        assert_eq!(breakdown.session_completion, 165);
    }

    #[test]
    fn test_new_user() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let escrow = Address::generate(&env);
        let staking = Address::generate(&env);
        let cid = env.register_contract(None, CreditScoreContract);
        let client = CreditScoreContractClient::new(&env, &cid);
        client.initialize(&admin, &escrow, &staking);

        let user = Address::generate(&env);
        assert_eq!(client.get_score(&user), 300);
    }
}

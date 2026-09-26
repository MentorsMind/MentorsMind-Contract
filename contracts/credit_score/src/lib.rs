#![no_std]
use soroban_sdk::{
    contract, contractevent, contractimpl, contracttype, symbol_short, Address, Env, IntoVal, Symbol, Vec,
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

/// A single entry in the auditable score-change history for an address.
/// Written on every `set_score` and `refresh_score` call.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScoreRecord {
    pub score: u32,
    pub timestamp: u64,
    /// Human-readable label for why the score changed, e.g. `"refresh"`,
    /// `"admin_set"`, or a custom reason passed by the caller.
    pub reason: Symbol,
}

#[contracttype]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,                  // Persistent: critical config
    EscrowContract,         // Persistent: external dependency
    StakingContract,        // Persistent: external dependency
    UserScore(Address),     // Persistent: long-term user data
    UserBreakdown(Address), // Persistent: long-term user data
    LastUpdate(Address),    // Temporary: rate limiting, auto-expires
    /// Individual score-history entry: (address, sequential index).
    ScoreHistory(Address, u32),
    /// Total number of history entries stored for an address.
    ScoreHistoryLen(Address),
}

const MIN_SCORE: u32 = 300;
const MAX_SCORE: u32 = 850;
const DAY_SECONDS: u64 = 86_400;
const DAY_SECONDS_TTL: u32 = 86_400;

/// Hard ceiling on items returned by a single `get_score_history_page` call.
/// Mirrors `shared::pagination::MAX_PAGE_SIZE`.
pub const MAX_PAGE_SIZE: u32 = 50;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contractevent]
#[derive(Clone)]
struct ScoreUpdatedEvent {
    #[topic]
    category: Symbol,
    #[topic]
    action: Symbol,
    #[topic]
    user: Address,
    score: u32,
}

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

    /// Admin-only: directly set a score for `user` with an explicit `reason`.
    ///
    /// The change is appended to the address's auditable history so that
    /// lending-pool and governance consumers can detect manual manipulation.
    pub fn set_score(env: Env, admin: Address, user: Address, score: u32, reason: Symbol) {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        if stored_admin != admin {
            panic!("Unauthorized");
        }

        let clamped = score.clamp(MIN_SCORE, MAX_SCORE);
        env.storage()
            .persistent()
            .set(&DataKey::UserScore(user.clone()), &clamped);

        Self::append_history(&env, user.clone(), clamped, reason.clone());

        ScoreUpdatedEvent {
            category: symbol_short!("score"),
            action: symbol_short!("set"),
            user,
            score: clamped,
        }
        .publish(env);
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

        Self::append_history(&env, user.clone(), score, symbol_short!("refresh"));

        ScoreUpdatedEvent {
            category: symbol_short!("score"),
            action: symbol_short!("updated"),
            user,
            score,
        }
        .publish(&env);
    }

    pub fn compute_score(env: Env, user: Address) -> u32 {
        let (score, _) = Self::do_compute(env, user);
        score
    }

    /// Return a paginated slice of the score-change history for `user`.
    ///
    /// `offset` is zero-based; `limit` is clamped to [`MAX_PAGE_SIZE`] (50).
    /// Returns an empty `Vec` when `offset` is past the end of the history.
    pub fn get_score_history_page(
        env: Env,
        user: Address,
        offset: u32,
        limit: u32,
    ) -> Vec<ScoreRecord> {
        let total: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ScoreHistoryLen(user.clone()))
            .unwrap_or(0);

        let (start, end) = Self::pagination_bounds(total, offset, limit);
        let mut page = Vec::new(&env);
        for i in start..end {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<_, ScoreRecord>(&DataKey::ScoreHistory(user.clone(), i))
            {
                page.push_back(record);
            }
        }
        page
    }
}

impl CreditScoreContract {
    /// Append a new `ScoreRecord` to `user`'s history and increment the
    /// history length counter.
    fn append_history(env: &Env, user: Address, score: u32, reason: Symbol) {
        let len: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ScoreHistoryLen(user.clone()))
            .unwrap_or(0);

        let record = ScoreRecord {
            score,
            timestamp: env.ledger().timestamp(),
            reason,
        };
        env.storage()
            .persistent()
            .set(&DataKey::ScoreHistory(user.clone(), len), &record);
        env.storage()
            .persistent()
            .set(&DataKey::ScoreHistoryLen(user.clone()), &(len + 1));
    }

    /// Resolve a `(total, offset, limit)` triple into a concrete `[start, end)`
    /// index range, clamping `limit` to [`MAX_PAGE_SIZE`].
    ///
    /// Mirrors `shared::pagination::Pagination::bounds` so this contract can
    /// remain `shared`-free.
    fn pagination_bounds(total: u32, offset: u32, limit: u32) -> (u32, u32) {
        let limit = if limit == 0 {
            1
        } else if limit > MAX_PAGE_SIZE {
            MAX_PAGE_SIZE
        } else {
            limit
        };

        if offset >= total {
            return (total, total);
        }

        let remaining = total - offset;
        let end = if limit >= remaining {
            total
        } else {
            offset + limit
        };

        (offset, end)
    }

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

    // ── History accumulation ──────────────────────────────────────────────

    /// Creates a contract with no mock dependencies (for set_score tests that
    /// don't invoke the escrow/staking contracts). Returns `(admin, contract_id)`.
    fn setup_no_mock(env: &Env) -> (Address, Address) {
        env.mock_all_auths();
        let admin = Address::generate(env);
        let escrow = Address::generate(env);
        let staking = Address::generate(env);
        let cid = env.register_contract(None, CreditScoreContract);
        let client = CreditScoreContractClient::new(env, &cid);
        client.initialize(&admin, &escrow, &staking);
        (admin, cid)
    }

    #[test]
    fn set_score_appends_history_entry() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let (admin, cid) = setup_no_mock(&env);
        let client = CreditScoreContractClient::new(&env, &cid);
        let user = Address::generate(&env);

        client.set_score(&admin, &user, &600, &Symbol::new(&env, "admin_set"));

        let page = client.get_score_history_page(&user, &0, &10);
        assert_eq!(page.len(), 1, "one history entry expected");
        let rec = page.get(0).unwrap();
        assert_eq!(rec.score, 600);
        assert_eq!(rec.timestamp, 1_000);
        assert_eq!(rec.reason, Symbol::new(&env, "admin_set"));
    }

    #[test]
    fn multiple_set_score_calls_accumulate_history() {
        let env = Env::default();
        env.ledger().set_timestamp(1_000);
        let (admin, cid) = setup_no_mock(&env);
        let client = CreditScoreContractClient::new(&env, &cid);
        let user = Address::generate(&env);

        client.set_score(&admin, &user, &400, &Symbol::new(&env, "admin_set"));

        // Advance time so we can distinguish the two entries.
        env.ledger().set_timestamp(2_000);
        client.set_score(&admin, &user, &500, &Symbol::new(&env, "admin_set"));

        env.ledger().set_timestamp(3_000);
        client.set_score(&admin, &user, &600, &Symbol::new(&env, "admin_set"));

        let page = client.get_score_history_page(&user, &0, &50);
        assert_eq!(page.len(), 3, "three history entries expected");

        assert_eq!(page.get(0).unwrap().score, 400);
        assert_eq!(page.get(0).unwrap().timestamp, 1_000);

        assert_eq!(page.get(1).unwrap().score, 500);
        assert_eq!(page.get(1).unwrap().timestamp, 2_000);

        assert_eq!(page.get(2).unwrap().score, 600);
        assert_eq!(page.get(2).unwrap().timestamp, 3_000);
    }

    #[test]
    fn refresh_score_appends_history_entry() {
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

        let page = client.get_score_history_page(&user, &0, &10);
        assert_eq!(page.len(), 1, "refresh_score must append one history entry");
        let rec = page.get(0).unwrap();
        assert_eq!(rec.reason, symbol_short!("refresh"));
        assert_eq!(rec.score, client.get_score(&user));
        assert_eq!(rec.timestamp, 1_000_000);
    }

    // ── Pagination ────────────────────────────────────────────────────────

    /// Populate `count` history entries via `set_score`, incrementing the
    /// ledger timestamp by 1 for each entry so timestamps are distinguishable.
    fn populate_history(
        env: &Env,
        admin: &Address,
        user: &Address,
        client: &CreditScoreContractClient,
        count: u32,
    ) {
        for i in 0..count {
            env.ledger().set_timestamp(1_000 + i as u64);
            client.set_score(
                admin,
                user,
                &(300 + i).min(850),
                &Symbol::new(env, "admin_set"),
            );
        }
    }

    #[test]
    fn history_page_returns_correct_slice() {
        let env = Env::default();
        let (admin, cid) = setup_no_mock(&env);
        let client = CreditScoreContractClient::new(&env, &cid);
        let user = Address::generate(&env);

        populate_history(&env, &admin, &user, &client, 10);

        // offset=0, limit=3 → entries 0, 1, 2
        let page = client.get_score_history_page(&user, &0, &3);
        assert_eq!(page.len(), 3);
        assert_eq!(page.get(0).unwrap().score, 300);
        assert_eq!(page.get(2).unwrap().score, 302);

        // offset=7, limit=5 → only 3 entries remain (7, 8, 9)
        let page = client.get_score_history_page(&user, &7, &5);
        assert_eq!(page.len(), 3);
        assert_eq!(page.get(0).unwrap().score, 307);
        assert_eq!(page.get(2).unwrap().score, 309);
    }

    #[test]
    fn history_page_offset_past_end_returns_empty() {
        let env = Env::default();
        let (admin, cid) = setup_no_mock(&env);
        let client = CreditScoreContractClient::new(&env, &cid);
        let user = Address::generate(&env);

        populate_history(&env, &admin, &user, &client, 5);

        let page = client.get_score_history_page(&user, &10, &5);
        assert_eq!(page.len(), 0, "offset past end must yield empty page");
    }

    #[test]
    fn history_page_no_history_returns_empty() {
        let env = Env::default();
        let (_admin, cid) = setup_no_mock(&env);
        let client = CreditScoreContractClient::new(&env, &cid);
        let user = Address::generate(&env);

        let page = client.get_score_history_page(&user, &0, &10);
        assert_eq!(page.len(), 0, "user with no history must return empty page");
    }

    #[test]
    fn history_page_limit_clamped_to_max_page_size() {
        let env = Env::default();
        let (admin, cid) = setup_no_mock(&env);
        let client = CreditScoreContractClient::new(&env, &cid);
        let user = Address::generate(&env);

        // Write MAX_PAGE_SIZE + 10 entries so we can prove the cap bites.
        populate_history(&env, &admin, &user, &client, MAX_PAGE_SIZE + 10);

        // Requesting u32::MAX must be clamped to MAX_PAGE_SIZE.
        let page = client.get_score_history_page(&user, &0, &u32::MAX);
        assert_eq!(
            page.len(),
            MAX_PAGE_SIZE,
            "limit must be clamped to MAX_PAGE_SIZE ({})",
            MAX_PAGE_SIZE
        );
    }

    #[test]
    fn history_is_scoped_per_user() {
        let env = Env::default();
        let (admin, cid) = setup_no_mock(&env);
        let client = CreditScoreContractClient::new(&env, &cid);
        let user_a = Address::generate(&env);
        let user_b = Address::generate(&env);

        populate_history(&env, &admin, &user_a, &client, 3);
        // user_b has no entries
        let page = client.get_score_history_page(&user_b, &0, &10);
        assert_eq!(page.len(), 0, "user_b history must be empty when only user_a has entries");
    }
}

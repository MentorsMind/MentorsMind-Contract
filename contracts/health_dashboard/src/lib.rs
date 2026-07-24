#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, Env, IntoVal, Map, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Types (mirror `mentorminds_escrow` for cross-contract decode stability)
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
pub struct PlatformStats {
    pub total_value_locked: i128,
    pub active_escrows: u32,
    pub total_sessions: u32,
    pub dispute_rate_bps: u32,
    pub total_mentors: u32,
    pub total_learners: u32,
    pub mnt_staked: i128,
    pub contract_versions: Map<Symbol, u32>,
}

/// Mirrors `interface_registry::InterfaceEntry` for `list_interfaces` decoding.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceEntry {
    pub interface_id: Symbol,
    pub contract: Address,
    pub version: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Config,
    /// `(ledger_sequence, cached stats)` — invalidated when ledger advances.
    Cache,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub admin: Address,
    pub escrow: Address,
    pub session_registry: Address,
    pub staking: Address,
    pub mnt_token: Address,
    pub reputation: Address,
    pub interface_registry: Address,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct HealthDashboardContract;

#[contractimpl]
impl HealthDashboardContract {
    /// One-time configuration of dependent contract addresses.
    pub fn initialize(
        env: Env,
        admin: Address,
        escrow: Address,
        session_registry: Address,
        staking: Address,
        mnt_token: Address,
        reputation: Address,
        interface_registry: Address,
    ) {
        if env.storage().persistent().has(&DataKey::Config) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(
            &DataKey::Config,
            &Config {
                admin,
                escrow,
                session_registry,
                staking,
                mnt_token,
                reputation,
                interface_registry,
            },
        );
    }

    /// Returns platform-wide metrics, using a one-ledger cache to limit
    /// cross-contract work within the same ledger.
    pub fn get_platform_stats(env: Env) -> PlatformStats {
        let ledger = env.ledger().sequence();
        if let Some((cached_ledger, stats)) = env
            .storage()
            .persistent()
            .get::<_, (u32, PlatformStats)>(&DataKey::Cache)
        {
            if cached_ledger == ledger {
                return stats;
            }
        }

        let stats = Self::compute_platform_stats(&env);
        env.storage()
            .persistent()
            .set(&DataKey::Cache, &(ledger, stats.clone()));

        env.events().publish(
            (Symbol::new(&env, "stats_refreshed"),),
            (ledger, stats.total_value_locked, stats.active_escrows),
        );

        stats
    }

    /// Version registered for a logical contract name in the interface registry.
    pub fn get_contract_version(env: Env, contract_name: Symbol) -> u32 {
        let cfg: Config = env
            .storage()
            .persistent()
            .get(&DataKey::Config)
            .expect("Not initialized");
        env.invoke_contract(
            &cfg.interface_registry,
            &Symbol::new(&env, "get_version"),
            (contract_name,).into_val(&env),
        )
    }
}

impl HealthDashboardContract {
    fn compute_platform_stats(env: &Env) -> PlatformStats {
        let cfg: Config = env
            .storage()
            .persistent()
            .get(&DataKey::Config)
            .expect("Not initialized");

        let escrow_count: u64 = env.invoke_contract(
            &cfg.escrow,
            &Symbol::new(env, "get_escrow_count"),
            ().into_val(env),
        );

        let mut total_value_locked: i128 = 0;
        let mut active_escrows: u32 = 0;
        let mut total_sessions: u32 = 0;
        let mut dispute_hits: u32 = 0;
        let mut total_for_disputes: u32 = 0;

        let mut mentor_vec: Vec<Address> = Vec::new(env);
        let mut learner_vec: Vec<Address> = Vec::new(env);

        for id in 1u64..=escrow_count {
            let e: Escrow = env.invoke_contract(
                &cfg.escrow,
                &Symbol::new(env, "get_escrow"),
                (id,).into_val(env),
            );
            total_sessions = total_sessions.saturating_add(e.total_sessions);
            total_for_disputes = total_for_disputes.saturating_add(1);

            if e.status == EscrowStatus::Disputed || e.status == EscrowStatus::Resolved {
                dispute_hits = dispute_hits.saturating_add(1);
            }

            if e.status == EscrowStatus::Active {
                active_escrows = active_escrows.saturating_add(1);
                total_value_locked = total_value_locked.saturating_add(e.amount);
            }

            Self::push_unique(env, &mut mentor_vec, e.mentor);
            Self::push_unique(env, &mut learner_vec, e.learner);
        }

        // Session registry (optional): e.g. bundle mint counter on session NFT contract.
        let session_extra: u32 = match env.try_invoke_contract::<u64, soroban_sdk::Error>(
            &cfg.session_registry,
            &Symbol::new(env, "session_cnt"),
            ().into_val(env),
        ) {
            Ok(Ok(n)) => n as u32,
            _ => 0,
        };
        total_sessions = total_sessions.saturating_add(session_extra);

        // Staking TVL in MNT: token balance held by the staking contract.
        let token_client = token::Client::new(env, &cfg.mnt_token);
        let mnt_staked: i128 = token_client.balance(&cfg.staking);

        // Reputation contract: optional ping (keeps dependency explicit for future metrics).
        let _ = env.try_invoke_contract::<u32, soroban_sdk::Error>(
            &cfg.reputation,
            &Symbol::new(env, "ping"),
            ().into_val(env),
        );

        let dispute_rate_bps: u32 = if total_for_disputes == 0 {
            0
        } else {
            ((dispute_hits as u64 * 10_000) / (total_for_disputes as u64)) as u32
        };

        let entries: Vec<InterfaceEntry> = env.invoke_contract(
            &cfg.interface_registry,
            &Symbol::new(env, "list_interfaces"),
            ().into_val(env),
        );
        let mut contract_versions: Map<Symbol, u32> = Map::new(env);
        for i in 0..entries.len() {
            let entry = entries.get(i).unwrap();
            contract_versions.set(entry.interface_id.clone(), entry.version);
        }

        PlatformStats {
            total_value_locked,
            active_escrows,
            total_sessions,
            dispute_rate_bps,
            total_mentors: mentor_vec.len(),
            total_learners: learner_vec.len(),
            mnt_staked,
            contract_versions,
        }
    }

    fn push_unique(_env: &Env, v: &mut Vec<Address>, addr: Address) {
        for i in 0..v.len() {
            if v.get(i).unwrap() == addr {
                return;
            }
        }
        v.push_back(addr);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;

    use super::*;
    use soroban_sdk::symbol_short;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[contracttype]
    #[derive(Clone)]
    enum MockTokenKey {
        Bal(Address),
    }

    #[contract]
    pub struct MockMntToken;

    #[contractimpl]
    impl MockMntToken {
        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&MockTokenKey::Bal(id))
                .unwrap_or(0)
        }

        pub fn mint(env: Env, to: Address, amount: i128) {
            let cur: i128 = env
                .storage()
                .persistent()
                .get(&MockTokenKey::Bal(to.clone()))
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&MockTokenKey::Bal(to), &(cur + amount));
        }
    }

    #[contract]
    pub struct MockEscrow;

    #[contractimpl]
    impl MockEscrow {
        pub fn get_escrow_count(_env: Env) -> u64 {
            2
        }

        pub fn get_escrow(env: Env, id: u64) -> Escrow {
            let t = Address::generate(&env);
            let m1 = Address::generate(&env);
            let m2 = Address::generate(&env);
            let l1 = Address::generate(&env);
            let l2 = Address::generate(&env);
            if id == 1 {
                Escrow {
                    id: 1,
                    mentor: m1,
                    learner: l1,
                    amount: 1000,
                    session_id: symbol_short!("s1"),
                    status: EscrowStatus::Active,
                    created_at: 0,
                    token_address: t.clone(),
                    platform_fee: 0,
                    net_amount: 0,
                    session_end_time: 0,
                    auto_release_delay: 0,
                    dispute_reason: symbol_short!("none"),
                    resolved_at: 0,
                    usd_amount: 0,
                    quoted_token_amount: 0,
                    send_asset: t.clone(),
                    dest_asset: t,
                    total_sessions: 5,
                    sessions_completed: 0,
                }
            } else {
                Escrow {
                    id: 2,
                    mentor: m2,
                    learner: l2,
                    amount: 100,
                    session_id: symbol_short!("s2"),
                    status: EscrowStatus::Disputed,
                    created_at: 0,
                    token_address: Address::generate(&env),
                    platform_fee: 0,
                    net_amount: 0,
                    session_end_time: 0,
                    auto_release_delay: 0,
                    dispute_reason: symbol_short!("none"),
                    resolved_at: 0,
                    usd_amount: 0,
                    quoted_token_amount: 0,
                    send_asset: Address::generate(&env),
                    dest_asset: Address::generate(&env),
                    total_sessions: 3,
                    sessions_completed: 0,
                }
            }
        }
    }

    #[contract]
    pub struct MockSessionRegistry;

    #[contractimpl]
    impl MockSessionRegistry {
        pub fn session_cnt(_env: Env) -> u64 {
            7
        }
    }

    #[contract]
    pub struct MockReputation;

    #[contractimpl]
    impl MockReputation {
        pub fn ping(_env: Env) -> u32 {
            1
        }
    }

    #[contract]
    pub struct MockInterfaceRegistry;

    #[contractimpl]
    impl MockInterfaceRegistry {
        pub fn get_version(_env: Env, interface_id: Symbol) -> u32 {
            if interface_id == symbol_short!("escrow") {
                2
            } else {
                0
            }
        }

        pub fn list_interfaces(env: Env) -> Vec<InterfaceEntry> {
            let mut v = Vec::new(&env);
            v.push_back(InterfaceEntry {
                interface_id: symbol_short!("escrow"),
                contract: Address::generate(&env),
                version: 2,
            });
            v
        }
    }

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let escrow_id = env.register_contract(None, MockEscrow);
        let session_reg = env.register_contract(None, MockSessionRegistry);
        let staking = Address::generate(&env);
        let mnt = env.register_contract(None, MockMntToken);
        MockMntTokenClient::new(&env, &mnt).mint(&staking, &5000i128);

        let reputation = env.register_contract(None, MockReputation);
        let iface = env.register_contract(None, MockInterfaceRegistry);
        let dashboard = env.register_contract(None, HealthDashboardContract);

        HealthDashboardContractClient::new(&env, &dashboard).initialize(
            &admin,
            &escrow_id,
            &session_reg,
            &staking,
            &mnt,
            &reputation,
            &iface,
        );

        (env, dashboard, mnt)
    }

    #[test]
    fn test_stats_aggregation() {
        let (env, dashboard, _mnt) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let s = client.get_platform_stats();

        assert_eq!(s.total_value_locked, 1000);
        assert_eq!(s.active_escrows, 1);
        assert_eq!(s.total_sessions, 5 + 3 + 7);
        assert_eq!(s.dispute_rate_bps, 5000);
        assert_eq!(s.total_mentors, 2);
        assert_eq!(s.total_learners, 2);
        assert_eq!(s.mnt_staked, 5000);
        assert_eq!(s.contract_versions.get(symbol_short!("escrow")), Some(2));
    }

    #[test]
    fn test_get_contract_version() {
        let (env, dashboard, _) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        assert_eq!(client.get_contract_version(&symbol_short!("escrow")), 2);
    }

    #[test]
    fn test_cache_same_ledger() {
        let (env, dashboard, _) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);

        let s1 = client.get_platform_stats();
        let s2 = client.get_platform_stats();
        assert_eq!(s1.total_sessions, s2.total_sessions);
        assert_eq!(s1.mnt_staked, s2.mnt_staked);
    }

    #[test]
    fn test_cache_invalidates_next_ledger() {
        let (env, dashboard, _) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let _ = client.get_platform_stats();

        env.ledger().with_mut(|li| {
            li.sequence_number += 1;
        });

        let _ = client.get_platform_stats();
    }
}

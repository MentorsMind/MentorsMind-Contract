#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec,
};

const DAY_SECS: u64 = 86_400;
const MONTH_SECS: u64 = 30 * DAY_SECS;
const UNLIMITED: i128 = i128::MAX;

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd)]
pub enum KycLevel {
    None = 0,
    Basic = 1,
    Enhanced = 2,
    Institutional = 3,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LimitTier {
    Basic,
    Enhanced,
    Institutional,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LimitConfig {
    pub per_tx_limit: i128,
    pub daily_limit: i128,
    pub monthly_limit: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TxRecord {
    pub timestamp: u64,
    pub amount_usd: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    KycRegistry,
    LastDailyReset,
    UserTxs(Address),
    UserOnHold(Address),
}

#[contractclient(name = "KycRegistryClient")]
pub trait KycRegistryTrait {
    fn get_kyc_level(env: Env, user: Address) -> KycLevel;
}

#[contract]
pub struct VelocityLimitsContract;

#[contractimpl]
impl VelocityLimitsContract {
    pub fn initialize(env: Env, admin: Address, kyc_registry: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::KycRegistry, &kyc_registry);
        env.storage()
            .instance()
            .set(&DataKey::LastDailyReset, &0u64);
    }

    pub fn set_kyc_registry(env: Env, kyc_registry: Address) {
        Self::require_admin(&env);
        env.storage()
            .instance()
            .set(&DataKey::KycRegistry, &kyc_registry);
    }

    pub fn clear_hold(env: Env, user: Address) {
        Self::require_admin(&env);
        env.storage()
            .persistent()
            .set(&DataKey::UserOnHold(user.clone()), &false);
        env.events()
            .publish((symbol_short!("hold"), symbol_short!("cleared"), user), ());
    }

    pub fn check_and_record(env: Env, user: Address, amount_usd: i128) -> bool {
        if amount_usd <= 0 {
            env.events().publish(
                (Symbol::new(&env, "limit_exceeded"), user),
                (symbol_short!("invalid"), amount_usd),
            );
            return false;
        }

        if Self::is_on_hold(env.clone(), user.clone()) {
            env.events().publish(
                (Symbol::new(&env, "limit_exceeded"), user),
                (symbol_short!("on_hold"), amount_usd),
            );
            return false;
        }

        let now = env.ledger().timestamp();
        let (mut txs, daily_used, monthly_used) = Self::load_usage(&env, &user, now);
        let config = Self::limit_config_for(&env, &user);

        if amount_usd > config.per_tx_limit {
            // Per-transaction cap breach is treated as suspicious activity and auto-holds user.
            env.storage()
                .persistent()
                .set(&DataKey::UserOnHold(user.clone()), &true);
            env.events().publish(
                (Symbol::new(&env, "limit_exceeded"), user.clone()),
                (symbol_short!("per_tx"), amount_usd),
            );
            return false;
        }

        if daily_used.saturating_add(amount_usd) > config.daily_limit {
            env.events().publish(
                (Symbol::new(&env, "limit_exceeded"), user.clone()),
                (symbol_short!("daily"), amount_usd),
            );
            return false;
        }

        if monthly_used.saturating_add(amount_usd) > config.monthly_limit {
            env.events().publish(
                (Symbol::new(&env, "limit_exceeded"), user.clone()),
                (symbol_short!("monthly"), amount_usd),
            );
            return false;
        }

        txs.push_back(TxRecord {
            timestamp: now,
            amount_usd,
        });
        env.storage()
            .persistent()
            .set(&DataKey::UserTxs(user.clone()), &txs);

        env.events().publish(
            (Symbol::new(&env, "limit_checked"), user),
            (
                amount_usd,
                daily_used.saturating_add(amount_usd),
                monthly_used.saturating_add(amount_usd),
            ),
        );
        true
    }

    pub fn get_usage(env: Env, user: Address) -> (i128, i128) {
        let now = env.ledger().timestamp();
        let (_txs, daily_used, monthly_used) = Self::load_usage(&env, &user.clone(), now);
        (daily_used, monthly_used)
    }

    pub fn reset_daily(env: Env) {
        Self::require_admin(&env);

        let now = env.ledger().timestamp();
        let last: u64 = env
            .storage()
            .instance()
            .get(&DataKey::LastDailyReset)
            .unwrap_or(0);

        if last != 0 && now < last.saturating_add(DAY_SECS) {
            panic!("reset too early");
        }

        env.storage().instance().set(&DataKey::LastDailyReset, &now);
        env.events().publish(
            (symbol_short!("daily"), symbol_short!("reset")),
            (last, now),
        );
    }

    pub fn get_tier(env: Env, user: Address) -> LimitTier {
        Self::tier_for_kyc(Self::kyc_level_for(&env, &user))
    }

    pub fn is_on_hold(env: Env, user: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::UserOnHold(user))
            .unwrap_or(false)
    }

    fn require_admin(env: &Env) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
    }

    fn kyc_level_for(env: &Env, user: &Address) -> KycLevel {
        let kyc_registry: Address = env
            .storage()
            .instance()
            .get(&DataKey::KycRegistry)
            .expect("not initialized");
        KycRegistryClient::new(env, &kyc_registry).get_kyc_level(user)
    }

    fn tier_for_kyc(level: KycLevel) -> LimitTier {
        match level {
            KycLevel::Institutional => LimitTier::Institutional,
            KycLevel::Enhanced => LimitTier::Enhanced,
            _ => LimitTier::Basic,
        }
    }

    fn limit_config_for(env: &Env, user: &Address) -> LimitConfig {
        match Self::tier_for_kyc(Self::kyc_level_for(env, user)) {
            LimitTier::Basic => LimitConfig {
                per_tx_limit: 500,
                daily_limit: 1_000,
                monthly_limit: 30_000,
            },
            LimitTier::Enhanced => LimitConfig {
                per_tx_limit: 5_000,
                daily_limit: 10_000,
                monthly_limit: 300_000,
            },
            LimitTier::Institutional => LimitConfig {
                per_tx_limit: UNLIMITED,
                daily_limit: UNLIMITED,
                monthly_limit: UNLIMITED,
            },
        }
    }

    fn load_usage(env: &Env, user: &Address, now: u64) -> (Vec<TxRecord>, i128, i128) {
        let txs: Vec<TxRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::UserTxs(user.clone()))
            .unwrap_or(Vec::new(env));

        let day_start = now.saturating_sub(DAY_SECS);
        let month_start = now.saturating_sub(MONTH_SECS);

        let mut retained = Vec::new(env);
        let mut daily_used = 0i128;
        let mut monthly_used = 0i128;

        for tx in txs.iter() {
            if tx.timestamp >= month_start {
                monthly_used = monthly_used.saturating_add(tx.amount_usd);
                retained.push_back(tx.clone());
                if tx.timestamp >= day_start {
                    daily_used = daily_used.saturating_add(tx.amount_usd);
                }
            }
        }

        (retained, daily_used, monthly_used)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{contract, contractimpl};

    #[contracttype]
    #[derive(Clone)]
    enum MockDataKey {
        Level(Address),
    }

    #[contract]
    struct MockKyc;

    #[contractimpl]
    impl MockKyc {
        pub fn set_level(env: Env, user: Address, level: KycLevel) {
            env.storage()
                .persistent()
                .set(&MockDataKey::Level(user), &level);
        }

        pub fn get_kyc_level(env: Env, user: Address) -> KycLevel {
            env.storage()
                .persistent()
                .get(&MockDataKey::Level(user))
                .unwrap_or(KycLevel::None)
        }
    }

    struct Fixture {
        env: Env,
        velocity_id: Address,
        kyc_id: Address,
        user: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let user = Address::generate(&env);

            let kyc_id = env.register_contract(None, MockKyc);
            let velocity_id = env.register_contract(None, VelocityLimitsContract);

            let velocity = VelocityLimitsContractClient::new(&env, &velocity_id);
            velocity.initialize(&admin, &kyc_id);

            let kyc = MockKycClient::new(&env, &kyc_id);
            kyc.set_level(&user, &KycLevel::Basic);

            Self {
                env,
                velocity_id,
                kyc_id,
                user,
            }
        }

        fn velocity(&self) -> VelocityLimitsContractClient {
            VelocityLimitsContractClient::new(&self.env, &self.velocity_id)
        }

        fn kyc(&self) -> MockKycClient {
            MockKycClient::new(&self.env, &self.kyc_id)
        }
    }

    #[test]
    fn test_under_limit_passes() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(1_000);

        assert!(f.velocity().check_and_record(&f.user, &400));
        let usage = f.velocity().get_usage(&f.user);
        assert_eq!(usage.0, 400);
        assert_eq!(usage.1, 400);
    }

    #[test]
    fn test_over_limit_fails() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(1_000);

        assert!(f.velocity().check_and_record(&f.user, &500));
        assert!(!f.velocity().check_and_record(&f.user, &600)); // 500 + 600 > 1000/day

        let usage = f.velocity().get_usage(&f.user);
        assert_eq!(usage.0, 500);
    }

    #[test]
    fn test_daily_reset() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(1_000);

        assert!(f.velocity().check_and_record(&f.user, &500));
        assert_eq!(f.velocity().get_usage(&f.user).0, 500);

        // Move past 24h and run the admin-only reset.
        f.env.ledger().set_timestamp(1_000 + DAY_SECS + 1);
        f.velocity().reset_daily();

        // Daily usage drops to zero due to rolling 24h window expiry, monthly stays.
        let usage = f.velocity().get_usage(&f.user);
        assert_eq!(usage.0, 0);
        assert_eq!(usage.1, 500);
    }

    #[test]
    fn test_daily_usage_expires_without_reset() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(1_000);

        assert!(f.velocity().check_and_record(&f.user, &500));
        assert_eq!(f.velocity().get_usage(&f.user).0, 500);

        f.env.ledger().set_timestamp(1_000 + DAY_SECS + 1);

        let usage = f.velocity().get_usage(&f.user);
        assert_eq!(usage.0, 0);
        assert_eq!(usage.1, 500);
    }

    #[test]
    fn test_kyc_tier_upgrade_increases_limit() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(1_000);

        // Basic: daily 1000
        assert!(f.velocity().check_and_record(&f.user, &500));
        assert!(f.velocity().check_and_record(&f.user, &500));
        assert!(!f.velocity().check_and_record(&f.user, &1));

        // Upgrade to Enhanced and verify higher allowance.
        f.kyc().set_level(&f.user, &KycLevel::Enhanced);
        assert_eq!(f.velocity().get_tier(&f.user), LimitTier::Enhanced);
        assert!(f.velocity().check_and_record(&f.user, &2_000));
    }
}

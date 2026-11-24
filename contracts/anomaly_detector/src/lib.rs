#![no_std]

use shared::params::{get_param, init_protocol_params, set_param as write_param, ParamKey};
use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, contracterror, contracttype,
    symbol_short, Address, Env, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Typed contract events (#1241)
// ---------------------------------------------------------------------------

/// Emitted when anomalous activity is detected but a hold is not yet placed.
#[contractevent]
#[derive(Clone)]
pub struct AnomalyDetectedEvent {
    #[topic]
    pub action: Symbol,
    pub user: Address,
    pub timestamp: u64,
}

/// Emitted when a hold is placed on a user due to severe anomalous activity.
#[contractevent]
#[derive(Clone)]
pub struct AnomalyHoldPlacedEvent {
    #[topic]
    pub action: Symbol,
    pub user: Address,
    pub timestamp: u64,
}

/// Emitted when an admin clears an existing hold from a user.
#[contractevent]
#[derive(Clone)]
pub struct AnomalyHoldClearedEvent {
    #[topic]
    pub action: Symbol,
    pub user: Address,
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    NotAdmin           = 3,
    HoldActive         = 4,
    /// A manual anomaly trigger exceeded its rate limit (#1430).
    RateLimited        = 5,
    /// `set_param` was given a key that is not an anomaly threshold (#1429).
    UnknownParam       = 6,
    /// A threshold was negative (#1429).
    InvalidThreshold   = 7,
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AnomalyAction {
    CreateEscrow   = 0,
    OpenDispute    = 1,
    LargeTransfer  = 2,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
#[repr(u32)]
pub enum AnomalyResult {
    Clear   = 0,
    Warning = 1,
    Hold    = 2,
}

/// Per-address activity metrics stored in persistent storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UserMetrics {
    /// Number of escrows created in the current 1-hour window
    pub escrows_created_1h:  u32,
    /// Timestamp of the start of the current 1-hour escrow window
    pub escrow_window_start: u64,
    /// Number of disputes opened in the current 24-hour window
    pub disputes_opened_24h: u32,
    /// Timestamp of the start of the current 24-hour dispute window
    pub dispute_window_start: u64,
    /// Total volume transferred in the current 1-hour window (in token units)
    pub volume_1h:            i128,
    /// Timestamp of the start of the current 1-hour volume window
    pub volume_window_start:  u64,
}

// ---------------------------------------------------------------------------
// Configurable thresholds
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnomalyConfig {
    pub escrow_warn_per_hour:  u32,
    pub dispute_hold_per_day:  u32,
    pub volume_hold_per_hour:  i128,
}

/// Per-caller limit for manually triggered anomaly checks (#1430).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitPolicy {
    pub max_calls: u32,
    pub window_seconds: u32,
}

// ---------------------------------------------------------------------------
// Cross-contract client for the rate limiter (#1430)
// ---------------------------------------------------------------------------

/// Minimal client for the `rate_limiter` contract.
#[contractclient(name = "RateLimiterClient")]
pub trait RateLimiter {
    fn check_rate_limit(
        env: Env,
        caller: Address,
        action: Symbol,
        max_calls: u32,
        window_seconds: u32,
    ) -> bool;
}

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Config,
    Metrics(Address),
    Hold(Address),
    UserIndex(u32),      // Sequential index of monitored users (#1264)
    UserCount,            // Total count of monitored users (#1264)
    RateLimiter,          // Address of the rate limiter contract (#1430)
    RateLimitPolicy,      // Manual-trigger rate limit policy (#1430)
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const ONE_HOUR_SECS:  u64 = 3_600;
const ONE_DAY_SECS:   u64 = 86_400;

const DEFAULT_ESCROW_WARN_PER_HOUR:  u32  = 10;
const DEFAULT_DISPUTE_HOLD_PER_DAY:  u32  = 3;
const DEFAULT_VOLUME_HOLD_PER_HOUR:  i128 = 50_000 * 1_000_000;

// Default manual-trigger rate limit: 5 calls per minute (#1430).
const DEFAULT_MAX_MANUAL_CALLS:      u32  = 5;
const DEFAULT_RATE_WINDOW_SECONDS:   u32  = 60;

// TTL and pagination constants (#1265, #1264)
const PERSISTENT_LIFETIME_THRESHOLD: u32 = 34560;  // ~14 days in ledgers
const PERSISTENT_BUMP_AMOUNT: u32 = 7200;          // ~3 days in ledgers
const HOLD_TTL: u32 = 864000;                       // Maximum TTL for hold flags
const MAX_PAGE_SIZE: u32 = 100;                     // Maximum items per pagination page

// ---------------------------------------------------------------------------
// Contract Client (#1263, #1430)
// ---------------------------------------------------------------------------

#[contractclient(name = "AnomalyDetectorClient")]
pub trait AnomalyDetectorTrait {
    fn initialize(env: Env, admin: Address) -> Result<(), Error>;
    fn set_thresholds(env: Env, admin: Address, config: AnomalyConfig) -> Result<(), Error>;
    fn get_thresholds(env: Env) -> AnomalyConfig;
    fn check_anomaly(env: Env, user: Address, action: AnomalyAction, amount: i128) -> Result<AnomalyResult, Error>;
    fn trigger_anomaly_check(env: Env, caller: Address, user: Address, action: AnomalyAction, amount: i128) -> Result<AnomalyResult, Error>;
    fn set_rate_limiter(env: Env, admin: Address, limiter: Address) -> Result<(), Error>;
    fn get_rate_limiter(env: Env) -> Option<Address>;
    fn set_rate_limit_policy(env: Env, admin: Address, policy: RateLimitPolicy) -> Result<(), Error>;
    fn get_rate_limit_policy(env: Env) -> RateLimitPolicy;
    fn clear_hold(env: Env, user: Address) -> Result<(), Error>;
    fn is_on_hold(env: Env, user: Address) -> bool;
    fn get_metrics(env: Env, user: Address) -> UserMetrics;
    fn get_monitored_user_count(env: Env) -> u32;
    fn get_monitored_users_page(env: Env, offset: u32, limit: u32) -> Vec<(Address, UserMetrics)>;
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

/// Protocol-param keys for the detection thresholds (#1429).
fn key_escrow_warn() -> Symbol { soroban_sdk::symbol_short!("ANM_ESC") }
fn key_dispute_hold() -> Symbol { soroban_sdk::symbol_short!("ANM_DSP") }
fn key_volume_hold() -> Symbol { soroban_sdk::symbol_short!("ANM_VOL") }

fn is_threshold_key(key: &Symbol) -> bool {
    *key == key_escrow_warn() || *key == key_dispute_hold() || *key == key_volume_hold()
}

/// Thresholds as governance last set them, or the compile-time defaults (#1429).
fn load_config(env: &Env) -> AnomalyConfig {
    AnomalyConfig {
        escrow_warn_per_hour: u32::try_from(get_param(env, &key_escrow_warn(), DEFAULT_ESCROW_WARN_PER_HOUR as i128))
            .unwrap_or(u32::MAX),
        dispute_hold_per_day: u32::try_from(get_param(env, &key_dispute_hold(), DEFAULT_DISPUTE_HOLD_PER_DAY as i128))
            .unwrap_or(u32::MAX),
        volume_hold_per_hour: get_param(env, &key_volume_hold(), DEFAULT_VOLUME_HOLD_PER_HOUR),
    }
}

#[contract]
pub struct AnomalyDetectorContract;

#[contractimpl]
impl AnomalyDetectorContract {
    /// Initialize with an admin address and default thresholds.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        // Thresholds are protocol params; unset ones fall back to the defaults (#1429).
        Ok(())
    }

    /// Admin updates thresholds atomically. Writes the same protocol params that
    /// `set_param` writes, so governance and admin see one source of truth (#1429).
    pub fn set_thresholds(env: Env, admin: Address, config: AnomalyConfig) -> Result<(), Error> {
        Self::assert_initialized(&env)?;
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        if admin != stored_admin {
            return Err(Error::NotAdmin);
        }
        if config.volume_hold_per_hour < 0 {
            return Err(Error::InvalidThreshold);
        }
        env.storage().persistent().set(&ParamKey::Param(key_escrow_warn()), &(config.escrow_warn_per_hour as i128));
        env.storage().persistent().set(&ParamKey::Param(key_dispute_hold()), &(config.dispute_hold_per_day as i128));
        env.storage().persistent().set(&ParamKey::Param(key_volume_hold()), &config.volume_hold_per_hour);
        Ok(())
    }

    /// Points governance at the RBAC contract that authorizes `set_param` (#1429).
    pub fn set_protocol_rbac(env: Env, admin: Address, rbac_contract: Address) -> Result<(), Error> {
        Self::assert_initialized(&env)?;
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        if admin != stored_admin {
            return Err(Error::NotAdmin);
        }
        init_protocol_params(&env, &rbac_contract);
        Ok(())
    }

    /// Governance update of one detection threshold (#1429). The caller must hold
    /// GOVERNANCE_ADMIN in the RBAC contract. Accepts only the three threshold keys.
    pub fn set_param(env: Env, caller: Address, key: Symbol, value: i128) -> Result<(), Error> {
        if !is_threshold_key(&key) {
            return Err(Error::UnknownParam);
        }
        write_param(&env, &caller, &key, value);
        Ok(())
    }

    /// Returns the current threshold configuration.
    pub fn get_thresholds(env: Env) -> AnomalyConfig {
        load_config(&env)
    }

    /// Check an action for anomalies and update metrics.
    /// Returns Clear, Warning, or Hold.
    /// Trusted callers (e.g. the escrow contract) use this entry point;
    /// untrusted manual triggers must use `trigger_anomaly_check` (#1430).
    pub fn check_anomaly(
        env: Env,
        user: Address,
        action: AnomalyAction,
        amount: i128,
    ) -> Result<AnomalyResult, Error> {
        Self::do_check_anomaly(&env, user, action, amount)
    }

    /// Manually trigger an anomaly check, rate-limited per caller (#1430).
    ///
    /// The caller must authenticate and is checked against the configured
    /// `rate_limiter` contract. Exceeding the limit returns `Error::RateLimited`
    /// before any metrics are updated.
    pub fn trigger_anomaly_check(
        env: Env,
        caller: Address,
        user: Address,
        action: AnomalyAction,
        amount: i128,
    ) -> Result<AnomalyResult, Error> {
        caller.require_auth();
        Self::assert_initialized(&env)?;

        let limiter: Address = env
            .storage()
            .instance()
            .get(&DataKey::RateLimiter)
            .ok_or(Error::NotInitialized)?;
        let policy: RateLimitPolicy = env
            .storage()
            .instance()
            .get(&DataKey::RateLimitPolicy)
            .unwrap_or(RateLimitPolicy {
                max_calls: DEFAULT_MAX_MANUAL_CALLS,
                window_seconds: DEFAULT_RATE_WINDOW_SECONDS,
            });
        let action_symbol = Self::rate_limit_action(&env, action);

        let client = RateLimiterClient::new(&env, &limiter);
        if !client.check_rate_limit(&caller, &action_symbol, &policy.max_calls, &policy.window_seconds) {
            return Err(Error::RateLimited);
        }

        Self::do_check_anomaly(&env, user, action, amount)
    }

    /// Configure the `rate_limiter` used by manual triggers. Admin-only (#1430).
    pub fn set_rate_limiter(env: Env, admin: Address, limiter: Address) -> Result<(), Error> {
        Self::assert_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::RateLimiter, &limiter);
        Ok(())
    }

    /// The configured rate limiter address, if any.
    pub fn get_rate_limiter(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::RateLimiter)
    }

    /// Configure the per-caller manual-trigger limit. Admin-only (#1430).
    pub fn set_rate_limit_policy(env: Env, admin: Address, policy: RateLimitPolicy) -> Result<(), Error> {
        Self::assert_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::RateLimitPolicy, &policy);
        Ok(())
    }

    /// Current per-caller manual-trigger limit.
    pub fn get_rate_limit_policy(env: Env) -> RateLimitPolicy {
        env.storage()
            .instance()
            .get(&DataKey::RateLimitPolicy)
            .unwrap_or(RateLimitPolicy {
                max_calls: DEFAULT_MAX_MANUAL_CALLS,
                window_seconds: DEFAULT_RATE_WINDOW_SECONDS,
            })
    }

    fn rate_limit_action(env: &Env, action: AnomalyAction) -> Symbol {
        match action {
            AnomalyAction::CreateEscrow => symbol_short!("crt_esc"),
            AnomalyAction::OpenDispute => symbol_short!("opn_disp"),
            AnomalyAction::LargeTransfer => symbol_short!("lg_trans"),
        }
    }

    fn do_check_anomaly(
        env: &Env,
        user: Address,
        action: AnomalyAction,
        amount: i128,
    ) -> Result<AnomalyResult, Error> {
        Self::assert_initialized(env)?;

        // If user is already on hold, return Hold immediately
        if env.storage().persistent().get::<_, bool>(&DataKey::Hold(user.clone())).unwrap_or(false) {
            return Ok(AnomalyResult::Hold);
        }

        let config = load_config(env);

        let now = env.ledger().timestamp();
        let mut metrics = Self::get_or_default_metrics(env, &user, now);
        let mut result = AnomalyResult::Clear;

        match action {
            AnomalyAction::CreateEscrow => {
                // Reset window if expired
                if now >= metrics.escrow_window_start + ONE_HOUR_SECS {
                    metrics.escrows_created_1h = 0;
                    metrics.escrow_window_start = now;
                }
                metrics.escrows_created_1h = metrics.escrows_created_1h.saturating_add(1);

                if metrics.escrows_created_1h > config.escrow_warn_per_hour {
                    result = AnomalyResult::Warning;
                }
            }
            AnomalyAction::OpenDispute => {
                if now >= metrics.dispute_window_start + ONE_DAY_SECS {
                    metrics.disputes_opened_24h = 0;
                    metrics.dispute_window_start = now;
                }
                metrics.disputes_opened_24h = metrics.disputes_opened_24h.saturating_add(1);

                if metrics.disputes_opened_24h > config.dispute_hold_per_day {
                    result = AnomalyResult::Hold;
                }
            }
            AnomalyAction::LargeTransfer => {
                if now >= metrics.volume_window_start + ONE_HOUR_SECS {
                    metrics.volume_1h = 0;
                    metrics.volume_window_start = now;
                }
                metrics.volume_1h = metrics.volume_1h.saturating_add(amount);

                if metrics.volume_1h > config.volume_hold_per_hour {
                    result = AnomalyResult::Hold;
                }
            }
        }

        // Persist updated metrics with TTL extension (#1265)
        env.storage().persistent().set(&DataKey::Metrics(user.clone()), &metrics);
        env.storage().persistent().extend_ttl(&DataKey::Metrics(user.clone()), PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT);

        // Track user in index for pagination (#1264)
        if !env.storage().persistent().has(&DataKey::Metrics(user.clone())) {
            let count: u32 = env.storage().persistent().get(&DataKey::UserCount).unwrap_or(0);
            env.storage().persistent().set(&DataKey::UserIndex(count), &user.clone());
            env.storage().persistent().set(&DataKey::UserCount, &(count + 1));
            env.storage().persistent().extend_ttl(&DataKey::UserCount, PERSISTENT_LIFETIME_THRESHOLD, PERSISTENT_BUMP_AMOUNT);
        }

        if result == AnomalyResult::Hold {
            env.storage().persistent().set(&DataKey::Hold(user.clone()), &true);
            env.storage().persistent().extend_ttl(&DataKey::Hold(user.clone()), PERSISTENT_LIFETIME_THRESHOLD, HOLD_TTL);
            AnomalyHoldPlacedEvent {
                action:    symbol_short!("hold_plcd"),
                user:      user.clone(),
                timestamp: env.ledger().timestamp(),
            }.publish(env);
        } else if result == AnomalyResult::Warning {
            AnomalyDetectedEvent {
                action:    symbol_short!("detected"),
                user:      user.clone(),
                timestamp: env.ledger().timestamp(),
            }.publish(env);
        }

        Ok(result)
    }

    /// Admin clears a hold on a user.
    pub fn clear_hold(env: Env, user: Address) -> Result<(), Error> {
        Self::assert_initialized(&env)?;

        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        env.storage().persistent().remove(&DataKey::Hold(user.clone()));

        AnomalyHoldClearedEvent {
            action:    symbol_short!("hold_clrd"),
            user:      user.clone(),
            timestamp: env.ledger().timestamp(),
        }.publish(&env);

        Ok(())
    }

    /// Returns the number of monitored users.
    pub fn get_monitored_user_count(env: Env) -> u32 {
        env.storage().persistent().get(&DataKey::UserCount).unwrap_or(0)
    }

    /// Returns a paginated list of monitored users with their metrics.
    /// Offset and limit are used for pagination; limit is clamped to MAX_PAGE_SIZE.
    pub fn get_monitored_users_page(env: Env, offset: u32, limit: u32) -> Vec<(Address, UserMetrics)> {
        let now = env.ledger().timestamp();
        let limit = limit.min(MAX_PAGE_SIZE);
        let count = Self::get_monitored_user_count(&env);

        let mut result = Vec::new(&env);

        for i in offset..offset.saturating_add(limit).min(count) {
            if let Some(user) = env.storage().persistent().get::<_, Address>(&DataKey::UserIndex(i)) {
                let metrics = Self::get_or_default_metrics(&env, &user, now);
                result.push_back((user, metrics));
            }
        }

        result
    }

    /// Returns true if the user is currently on hold.
    pub fn is_on_hold(env: Env, user: Address) -> bool {
        env.storage()
            .persistent()
            .get::<_, bool>(&DataKey::Hold(user))
            .unwrap_or(false)
    }

    /// Returns the current metrics for a user.
    pub fn get_metrics(env: Env, user: Address) -> UserMetrics {
        let now = env.ledger().timestamp();
        Self::get_or_default_metrics(&env, &user, now)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn assert_initialized(env: &Env) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }
        Ok(())
    }

    /// Require that `admin` authenticated and is the configured admin (#1430).
    fn assert_admin(env: &Env, admin: &Address) -> Result<(), Error> {
        Self::assert_initialized(env)?;
        admin.require_auth();
        let stored: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        if stored != *admin {
            return Err(Error::NotAdmin);
        }
        Ok(())
    }

    fn get_or_default_metrics(env: &Env, user: &Address, now: u64) -> UserMetrics {
        env.storage()
            .persistent()
            .get(&DataKey::Metrics(user.clone()))
            .unwrap_or(UserMetrics {
                escrows_created_1h:   0,
                escrow_window_start:  now,
                disputes_opened_24h:  0,
                dispute_window_start: now,
                volume_1h:            0,
                volume_window_start:  now,
            })
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    struct Fixture {
        env:      Env,
        contract: Address,
        admin:    Address,
        user:     Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set_timestamp(0);

            let admin = Address::generate(&env);
            let user = Address::generate(&env);
            let contract = env.register_contract(None, AnomalyDetectorContract);

            AnomalyDetectorContractClient::new(&env, &contract).initialize(&admin);

            Fixture { env, contract, admin, user }
        }

        fn client(&self) -> AnomalyDetectorContractClient {
            AnomalyDetectorContractClient::new(&self.env, &self.contract)
        }
    }

    #[test]
    fn test_normal_activity_is_clear() {
        let f = Fixture::setup();
        let result = f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        assert_eq!(result, AnomalyResult::Clear);
        assert!(!f.client().is_on_hold(&f.user));
    }

    #[test]
    fn test_escrow_threshold_warning() {
        let f = Fixture::setup();
        // Create 10 escrows — still at threshold, not over
        for _ in 0..10 {
            f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        }
        // 11th triggers Warning
        let result = f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        assert_eq!(result, AnomalyResult::Warning);
        // Warning does not place a hold
        assert!(!f.client().is_on_hold(&f.user));
    }

    #[test]
    fn test_dispute_threshold_hold() {
        let f = Fixture::setup();
        // 3 disputes — at threshold, not over
        for _ in 0..3 {
            f.client().check_anomaly(&f.user, &AnomalyAction::OpenDispute, &0);
        }
        // 4th triggers Hold
        let result = f.client().check_anomaly(&f.user, &AnomalyAction::OpenDispute, &0);
        assert_eq!(result, AnomalyResult::Hold);
        assert!(f.client().is_on_hold(&f.user));
    }

    #[test]
    fn test_volume_threshold_hold() {
        let f = Fixture::setup();
        // $50k + 1 triggers Hold
        let result = f
            .client()
            .check_anomaly(&f.user, &AnomalyAction::LargeTransfer, &(DEFAULT_VOLUME_HOLD_PER_HOUR + 1));
        assert_eq!(result, AnomalyResult::Hold);
        assert!(f.client().is_on_hold(&f.user));
    }

    #[test]
    fn test_hold_blocks_subsequent_actions() {
        let f = Fixture::setup();
        // Trigger hold via disputes
        for _ in 0..4 {
            f.client().check_anomaly(&f.user, &AnomalyAction::OpenDispute, &0);
        }
        assert!(f.client().is_on_hold(&f.user));

        // Any subsequent action returns Hold immediately
        let result = f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        assert_eq!(result, AnomalyResult::Hold);
    }

    #[test]
    fn test_admin_clear_hold() {
        let f = Fixture::setup();
        // Place hold
        f.client()
            .check_anomaly(&f.user, &AnomalyAction::LargeTransfer, &(DEFAULT_VOLUME_HOLD_PER_HOUR + 1));
        assert!(f.client().is_on_hold(&f.user));

        // Admin clears
        f.client().clear_hold(&f.user);
        assert!(!f.client().is_on_hold(&f.user));

        // Normal activity works again
        let result = f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        assert_eq!(result, AnomalyResult::Clear);
    }

    #[test]
    fn test_window_reset_after_expiry() {
        let f = Fixture::setup();
        // Fill up escrow window
        for _ in 0..11 {
            f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        }
        // Advance past 1-hour window
        f.env.ledger().set_timestamp(ONE_HOUR_SECS + 1);

        // Window resets — should be Clear again
        let result = f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        assert_eq!(result, AnomalyResult::Clear);
    }

    #[test]
    fn test_events_emitted() {
        let f = Fixture::setup();
        // Trigger a hold — just verify no panic
        let result = f
            .client()
            .check_anomaly(&f.user, &AnomalyAction::LargeTransfer, &(DEFAULT_VOLUME_HOLD_PER_HOUR + 1));
        assert_eq!(result, AnomalyResult::Hold);
    }

    // -----------------------------------------------------------------------
    // Typed #[contractevent] emission tests (#1241)
    // -----------------------------------------------------------------------

    #[test]
    fn test_anomaly_detected_event_emitted() {
        let f = Fixture::setup();
        // Trigger enough escrow creates to produce a Warning (11th call)
        for _ in 0..10 {
            f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        }
        // 11th triggers AnomalyDetectedEvent
        let result = f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        assert_eq!(result, AnomalyResult::Warning);

        // Verify that at least one event was recorded by the environment
        let events = f.env.events().all();
        assert!(!events.is_empty(), "AnomalyDetectedEvent must have been published");
    }

    #[test]
    fn test_anomaly_hold_placed_event_emitted() {
        let f = Fixture::setup();
        // Trigger a hold via LargeTransfer
        let result = f
            .client()
            .check_anomaly(&f.user, &AnomalyAction::LargeTransfer, &(DEFAULT_VOLUME_HOLD_PER_HOUR + 1));
        assert_eq!(result, AnomalyResult::Hold);

        let events = f.env.events().all();
        assert!(!events.is_empty(), "AnomalyHoldPlacedEvent must have been published");
    }

    #[test]
    fn test_anomaly_hold_cleared_event_emitted() {
        let f = Fixture::setup();
        // Place a hold first
        f.client()
            .check_anomaly(&f.user, &AnomalyAction::LargeTransfer, &(DEFAULT_VOLUME_HOLD_PER_HOUR + 1));
        assert!(f.client().is_on_hold(&f.user));

        // Clear it and verify event
        f.client().clear_hold(&f.user);
        assert!(!f.client().is_on_hold(&f.user));

        let events = f.env.events().all();
        // There should be at least two events: one for placing, one for clearing
        assert!(events.len() >= 2, "AnomalyHoldClearedEvent must have been published");
    }

    #[test]
    fn test_hold_placed_event_is_typed_and_emitted() {
        let f = Fixture::setup();
        f.client()
            .check_anomaly(&f.user, &AnomalyAction::LargeTransfer, &(DEFAULT_VOLUME_HOLD_PER_HOUR + 1));

        let events = f.env.events().all().filter_by_contract(&f.contract);
        assert!(!events.events().is_empty(), "hold_placed event must be emitted");
    }

    #[test]
    fn test_detected_event_is_typed_and_emitted() {
        let f = Fixture::setup();
        for _ in 0..10 {
            f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        }
        // 11th crosses the warn threshold and must emit a `detected` event.
        f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);

        let events = f.env.events().all().filter_by_contract(&f.contract);
        assert!(!events.events().is_empty(), "detected event must be emitted");
    }

    #[test]
    fn test_hold_cleared_event_is_typed_and_emitted() {
        let f = Fixture::setup();
        f.client()
            .check_anomaly(&f.user, &AnomalyAction::LargeTransfer, &(DEFAULT_VOLUME_HOLD_PER_HOUR + 1));
        assert!(f.client().is_on_hold(&f.user));

        f.client().clear_hold(&f.user);

        let events = f.env.events().all().filter_by_contract(&f.contract);
        assert!(!events.events().is_empty(), "hold_cleared event must be emitted");
    }

    #[test]
    fn test_set_thresholds_and_verify_updated() {
        let f = Fixture::setup();
        // Lower threshold to 2 escrows/hour
        let new_config = AnomalyConfig {
            escrow_warn_per_hour: 2,
            dispute_hold_per_day: DEFAULT_DISPUTE_HOLD_PER_DAY,
            volume_hold_per_hour: DEFAULT_VOLUME_HOLD_PER_HOUR,
        };
        f.client().set_thresholds(&f.admin, &new_config);

        // 3 escrows should now trigger Warning
        for _ in 0..2 {
            let r = f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
            assert_eq!(r, AnomalyResult::Clear);
        }
        let r = f.client().check_anomaly(&f.user, &AnomalyAction::CreateEscrow, &0);
        assert_eq!(r, AnomalyResult::Warning);
    }

    // -----------------------------------------------------------------------
    // Manual trigger rate limiting (#1430)
    // -----------------------------------------------------------------------

    #[contract]
    struct MockRateLimiter;

    #[contracttype]
    #[derive(Clone)]
    enum MockKey {
        Count(Address, Symbol),
    }

    #[contractimpl]
    impl MockRateLimiter {
        pub fn check_rate_limit(
            env: Env,
            caller: Address,
            action: Symbol,
            max_calls: u32,
            _window_seconds: u32,
        ) -> bool {
            caller.require_auth();
            let key = MockKey::Count(caller, action);
            let count: u32 = env.storage().instance().get(&key).unwrap_or(0);
            if count >= max_calls {
                return false;
            }
            env.storage().instance().set(&key, &(count + 1));
            true
        }
    }

    impl Fixture {
        fn with_rate_limiter(&self, max_calls: u32) {
            let limiter = self.env.register_contract(None, MockRateLimiter);
            self.client().set_rate_limiter(&self.admin, &limiter);
            self.client().set_rate_limit_policy(
                &self.admin,
                &RateLimitPolicy {
                    max_calls,
                    window_seconds: 60,
                },
            );
        }
    }

    #[test]
    fn test_manual_trigger_allowed_up_to_the_limit() {
        let f = Fixture::setup();
        f.with_rate_limiter(2);

        let caller = Address::generate(&f.env);
        assert_eq!(
            f.client().trigger_anomaly_check(&caller, &f.user, &AnomalyAction::CreateEscrow, &0),
            AnomalyResult::Clear
        );
        assert_eq!(
            f.client().trigger_anomaly_check(&caller, &f.user, &AnomalyAction::CreateEscrow, &0),
            AnomalyResult::Clear
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn test_manual_trigger_over_limit_returns_rate_limited() {
        let f = Fixture::setup();
        f.with_rate_limiter(1);

        let caller = Address::generate(&f.env);
        f.client().trigger_anomaly_check(&caller, &f.user, &AnomalyAction::CreateEscrow, &0);
        // Second rapid call exceeds the limit.
        f.client().trigger_anomaly_check(&caller, &f.user, &AnomalyAction::CreateEscrow, &0);
    }

    #[test]
    fn test_rate_limit_is_per_caller() {
        let f = Fixture::setup();
        f.with_rate_limiter(1);

        let caller1 = Address::generate(&f.env);
        let caller2 = Address::generate(&f.env);
        f.client().trigger_anomaly_check(&caller1, &f.user, &AnomalyAction::CreateEscrow, &0);
        // A different caller still has its own allowance.
        assert_eq!(
            f.client().trigger_anomaly_check(&caller2, &f.user, &AnomalyAction::CreateEscrow, &0),
            AnomalyResult::Clear
        );
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #2)")]
    fn test_manual_trigger_requires_configured_rate_limiter() {
        let f = Fixture::setup();
        let caller = Address::generate(&f.env);
        f.client().trigger_anomaly_check(&caller, &f.user, &AnomalyAction::CreateEscrow, &0);
    }

    #[test]
    fn test_rate_limit_policy_is_admin_configurable() {
        let f = Fixture::setup();
        let policy = RateLimitPolicy { max_calls: 7, window_seconds: 120 };
        f.client().set_rate_limit_policy(&f.admin, &policy);
        assert_eq!(f.client().get_rate_limit_policy(), policy);
    }
}

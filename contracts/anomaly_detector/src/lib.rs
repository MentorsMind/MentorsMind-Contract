#![no_std]

use shared::events::{
    emit_anomaly_event, evt_anomaly_detected, evt_anomaly_hold_cleared, evt_anomaly_hold_placed,
};
use soroban_sdk::{
    contract, contractimpl, contracterror, contracttype, symbol_short, Address, Env, Symbol,
};

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
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Metrics(Address),
    Hold(Address),
}

// ---------------------------------------------------------------------------
// Thresholds
// ---------------------------------------------------------------------------

const ESCROW_WARN_PER_HOUR:   u32  = 10;
const DISPUTE_HOLD_PER_DAY:   u32  = 3;
/// $50k in micro-units (assuming 6 decimals like USDC: 50_000 * 1_000_000)
const VOLUME_HOLD_PER_HOUR:   i128 = 50_000 * 1_000_000;

const ONE_HOUR_SECS:  u64 = 3_600;
const ONE_DAY_SECS:   u64 = 86_400;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct AnomalyDetectorContract;

#[contractimpl]
impl AnomalyDetectorContract {
    /// Initialize with an admin address.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        Ok(())
    }

    /// Check an action for anomalies and update metrics.
    /// Returns Clear, Warning, or Hold.
    /// Callers (e.g. escrow contract) should reject on Hold.
    pub fn check_anomaly(
        env: Env,
        user: Address,
        action: AnomalyAction,
        amount: i128,
    ) -> Result<AnomalyResult, Error> {
        Self::assert_initialized(&env)?;

        // If user is already on hold, return Hold immediately
        if env.storage().persistent().get::<_, bool>(&DataKey::Hold(user.clone())).unwrap_or(false) {
            return Ok(AnomalyResult::Hold);
        }

        let now = env.ledger().timestamp();
        let mut metrics = Self::get_or_default_metrics(&env, &user, now);
        let mut result = AnomalyResult::Clear;

        match action {
            AnomalyAction::CreateEscrow => {
                // Reset window if expired
                if now >= metrics.escrow_window_start + ONE_HOUR_SECS {
                    metrics.escrows_created_1h = 0;
                    metrics.escrow_window_start = now;
                }
                metrics.escrows_created_1h = metrics.escrows_created_1h.saturating_add(1);

                if metrics.escrows_created_1h > ESCROW_WARN_PER_HOUR {
                    result = AnomalyResult::Warning;
                }
            }
            AnomalyAction::OpenDispute => {
                if now >= metrics.dispute_window_start + ONE_DAY_SECS {
                    metrics.disputes_opened_24h = 0;
                    metrics.dispute_window_start = now;
                }
                metrics.disputes_opened_24h = metrics.disputes_opened_24h.saturating_add(1);

                if metrics.disputes_opened_24h > DISPUTE_HOLD_PER_DAY {
                    result = AnomalyResult::Hold;
                }
            }
            AnomalyAction::LargeTransfer => {
                if now >= metrics.volume_window_start + ONE_HOUR_SECS {
                    metrics.volume_1h = 0;
                    metrics.volume_window_start = now;
                }
                metrics.volume_1h = metrics.volume_1h.saturating_add(amount);

                if metrics.volume_1h > VOLUME_HOLD_PER_HOUR {
                    result = AnomalyResult::Hold;
                }
            }
        }

        // Persist updated metrics
        env.storage().persistent().set(&DataKey::Metrics(user.clone()), &metrics);

        if result == AnomalyResult::Hold {
            env.storage().persistent().set(&DataKey::Hold(user.clone()), &true);
            emit_anomaly_event(&env, evt_anomaly_hold_placed(&env), (user.clone(), env.ledger().timestamp()));
        } else if result == AnomalyResult::Warning {
            emit_anomaly_event(&env, evt_anomaly_detected(&env), (user.clone(), env.ledger().timestamp()));
        }

        Ok(result)
    }

    /// Admin clears a hold on a user.
    pub fn clear_hold(env: Env, user: Address) -> Result<(), Error> {
        Self::assert_initialized(&env)?;

        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        env.storage().persistent().remove(&DataKey::Hold(user.clone()));

        emit_anomaly_event(&env, evt_anomaly_hold_cleared(&env), (user.clone(), env.ledger().timestamp()));

        Ok(())
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
            .check_anomaly(&f.user, &AnomalyAction::LargeTransfer, &(VOLUME_HOLD_PER_HOUR + 1));
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
            .check_anomaly(&f.user, &AnomalyAction::LargeTransfer, &(VOLUME_HOLD_PER_HOUR + 1));
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
            .check_anomaly(&f.user, &AnomalyAction::LargeTransfer, &(VOLUME_HOLD_PER_HOUR + 1));
        assert_eq!(result, AnomalyResult::Hold);
    }
}

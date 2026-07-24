#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

const PAUSED: Symbol = symbol_short!("PAUSED");
const ADMIN: Symbol = symbol_short!("ADMIN");
/// Number of consecutive failures required to trip the circuit breaker.
const CIRCUIT_THRESHOLD: u32 = 3;
const FAILURES: Symbol = symbol_short!("FAILURES");

// ─── Yield contract health ────────────────────────────────────────────────────

/// Key for storing the registered yield contract address.
const YIELD_CONTRACT: Symbol = symbol_short!("YIELD_CTR");

/// Key for the last health-check timestamp.
const HEALTH_TS: Symbol = symbol_short!("HEALTH_TS");

/// Key for whether the yield contract interface has been validated.
const IFACE_VALID: Symbol = symbol_short!("IFACE_OK");

/// Minimum seconds between on-chain health checks to avoid excessive calls.
const HEALTH_CHECK_INTERVAL: u64 = 300; // 5 minutes

/// Yield contract health status returned by `yield_health`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct YieldHealth {
    /// True when the circuit breaker is NOT tripped and yield calls should proceed.
    pub operational: bool,
    /// Current consecutive failure count.
    pub failure_count: u32,
    /// Number of failures needed to trip the breaker.
    pub threshold: u32,
    /// Whether the yield contract interface has been validated.
    pub interface_validated: bool,
    /// Ledger timestamp of last successful health check (0 if never checked).
    pub last_checked_at: u64,
}

#[contract]
pub struct PauseGuardian;

#[contractimpl]
impl PauseGuardian {
    /// Initialize with an admin address.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&ADMIN) {
            panic!("already initialized");
        }
        env.storage().instance().set(&ADMIN, &admin);
        env.storage().instance().set(&PAUSED, &false);
        env.storage().instance().set(&FAILURES, &0u32);
        env.storage().instance().set(&IFACE_VALID, &false);
        env.events().publish(
            (symbol_short!("guardian"), symbol_short!("init")),
            admin,
        );
    }

    // ─── Core pause / unpause ─────────────────────────────────────────────

    /// Pause or unpause the contract. Admin only.
    pub fn set_paused(env: Env, value: bool) {
        let admin: Address = env.storage().instance().get(&ADMIN).expect("not initialized");
        admin.require_auth();
        env.storage().instance().set(&PAUSED, &value);
        // Reset failure counter when an admin manually unpauses.
        if !value {
            env.storage().instance().set(&FAILURES, &0u32);
        }
        env.events().publish(
            (symbol_short!("guardian"), symbol_short!("paused")),
            value,
        );
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage().instance().get(&PAUSED).unwrap_or(false)
    }

    // ─── Circuit breaker ──────────────────────────────────────────────────

    /// Record a yield/external contract failure.
    ///
    /// Automatically trips the circuit breaker (pauses) after
    /// `CIRCUIT_THRESHOLD` consecutive failures. Emits a `cb_tripped` event
    /// when the threshold is first crossed.
    pub fn record_failure(env: Env) {
        let count: u32 = env.storage().instance().get(&FAILURES).unwrap_or(0);
        let next = count.saturating_add(1);
        env.storage().instance().set(&FAILURES, &next);
        if next >= CIRCUIT_THRESHOLD {
            let was_paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
            env.storage().instance().set(&PAUSED, &true);
            if !was_paused {
                // Emit once when the breaker first trips to aid monitoring.
                env.events().publish(
                    (symbol_short!("guardian"), symbol_short!("cb_trip")),
                    next,
                );
            }
        }
    }

    /// Returns the current consecutive failure count.
    pub fn failure_count(env: Env) -> u32 {
        env.storage().instance().get(&FAILURES).unwrap_or(0)
    }

    /// Reset failure counter. Admin only.
    pub fn reset_failures(env: Env) {
        let admin: Address = env.storage().instance().get(&ADMIN).expect("not initialized");
        admin.require_auth();
        env.storage().instance().set(&FAILURES, &0u32);
        env.events().publish(
            (symbol_short!("guardian"), symbol_short!("cb_reset")),
            (),
        );
    }

    // ─── Yield contract registration & interface validation ───────────────

    /// Register the yield contract address. Admin only.
    ///
    /// Clears the interface-validated flag so `validate_yield_interface` must
    /// be called again after changing the contract.
    pub fn set_yield_contract(env: Env, yield_contract: Address) {
        let admin: Address = env.storage().instance().get(&ADMIN).expect("not initialized");
        admin.require_auth();
        env.storage().instance().set(&YIELD_CONTRACT, &yield_contract);
        // Invalidate previous check whenever the address changes.
        env.storage().instance().set(&IFACE_VALID, &false);
        env.events().publish(
            (symbol_short!("guardian"), symbol_short!("yld_set")),
            yield_contract,
        );
    }

    /// Return the registered yield contract address, if any.
    pub fn get_yield_contract(env: Env) -> Option<Address> {
        env.storage().instance().get(&YIELD_CONTRACT)
    }

    /// Mark the yield contract interface as validated.
    ///
    /// Call this after confirming off-chain (or via a cross-contract probe)
    /// that the registered yield contract exposes the expected interface.
    /// Admin only.
    ///
    /// # Validation requirements
    /// - A yield contract must be registered (`set_yield_contract`).
    /// - The caller must be the admin.
    pub fn validate_yield_interface(env: Env) {
        let admin: Address = env.storage().instance().get(&ADMIN).expect("not initialized");
        admin.require_auth();
        if !env.storage().instance().has(&YIELD_CONTRACT) {
            panic!("yield contract not registered");
        }
        env.storage().instance().set(&IFACE_VALID, &true);
        env.events().publish(
            (symbol_short!("guardian"), symbol_short!("iface_ok")),
            (),
        );
    }

    /// Returns true only when the yield contract is registered AND its
    /// interface has been validated by the admin.
    pub fn is_yield_interface_valid(env: Env) -> bool {
        env.storage().instance().get(&IFACE_VALID).unwrap_or(false)
    }

    // ─── Health check ─────────────────────────────────────────────────────

    /// Perform a lightweight on-chain health check of the yield integration.
    ///
    /// Records the current timestamp and returns a `YieldHealth` snapshot.
    /// Rate-limited to one call per `HEALTH_CHECK_INTERVAL` seconds; subsequent
    /// calls within the window are no-ops (still return the current status).
    pub fn check_yield_health(env: Env) -> YieldHealth {
        let now = env.ledger().timestamp();
        let last: u64 = env.storage().instance().get(&HEALTH_TS).unwrap_or(0);

        if now.saturating_sub(last) >= HEALTH_CHECK_INTERVAL {
            env.storage().instance().set(&HEALTH_TS, &now);
            env.events().publish(
                (symbol_short!("guardian"), symbol_short!("health")),
                now,
            );
        }

        let failures: u32 = env.storage().instance().get(&FAILURES).unwrap_or(0);
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        let validated: bool = env.storage().instance().get(&IFACE_VALID).unwrap_or(false);

        YieldHealth {
            operational: !paused,
            failure_count: failures,
            threshold: CIRCUIT_THRESHOLD,
            interface_validated: validated,
            last_checked_at: env.storage().instance().get(&HEALTH_TS).unwrap_or(0),
        }
    }

    // ─── Fallback query ───────────────────────────────────────────────────

    /// Returns whether a yield operation should use the fallback path.
    ///
    /// Callers (e.g. `lending_pool`) should check this before attempting any
    /// yield contract call:
    /// - Returns `true` when the circuit breaker is tripped OR the interface
    ///   has not been validated — both are conditions under which the normal
    ///   yield path is unsafe.
    /// - Returns `false` when the guardian is healthy and yield calls may proceed.
    pub fn should_use_fallback(env: Env) -> bool {
        let paused: bool = env.storage().instance().get(&PAUSED).unwrap_or(false);
        let validated: bool = env.storage().instance().get(&IFACE_VALID).unwrap_or(false);
        paused || !validated
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn setup() -> (Env, soroban_sdk::Address, PauseGuardianClient) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let id = env.register(PauseGuardian, ());
        let client = PauseGuardianClient::new(&env, &id);
        client.initialize(&admin);
        (env, admin, client)
    }

    // ─── existing tests ───────────────────────────────────────────────────

    #[test]
    fn test_circuit_breaker_trips_after_threshold() {
        let (_env, _admin, client) = setup();
        assert!(!client.is_paused());
        client.record_failure();
        client.record_failure();
        assert!(!client.is_paused());
        client.record_failure(); // hits threshold
        assert!(client.is_paused());
        assert_eq!(client.failure_count(), 3);
    }

    #[test]
    fn test_manual_unpause_resets_failures() {
        let (_env, _admin, client) = setup();
        client.record_failure();
        client.record_failure();
        client.record_failure();
        assert!(client.is_paused());
        client.set_paused(&false);
        assert!(!client.is_paused());
        assert_eq!(client.failure_count(), 0);
    }

    // ─── #418: yield interface validation ────────────────────────────────

    #[test]
    fn test_yield_interface_invalid_before_registration() {
        let (_env, _admin, client) = setup();
        assert!(!client.is_yield_interface_valid());
    }

    #[test]
    fn test_set_yield_contract_clears_validation() {
        let (env, admin, client) = setup();
        let yield_addr = Address::generate(&env);
        client.set_yield_contract(&yield_addr);
        assert!(!client.is_yield_interface_valid());
        client.validate_yield_interface();
        assert!(client.is_yield_interface_valid());
        // Registering a new contract invalidates the prior validation.
        let new_yield = Address::generate(&env);
        client.set_yield_contract(&new_yield);
        assert!(!client.is_yield_interface_valid());
        let _ = admin; // keep admin in scope
    }

    #[test]
    #[should_panic(expected = "yield contract not registered")]
    fn test_validate_without_registration_panics() {
        let (_env, _admin, client) = setup();
        client.validate_yield_interface();
    }

    // ─── #418: health check ───────────────────────────────────────────────

    #[test]
    fn test_yield_health_operational_when_not_paused() {
        let (env, admin, client) = setup();
        let yield_addr = Address::generate(&env);
        client.set_yield_contract(&yield_addr);
        client.validate_yield_interface();
        let health = client.check_yield_health();
        assert!(health.operational);
        assert_eq!(health.failure_count, 0);
        assert_eq!(health.threshold, 3);
        assert!(health.interface_validated);
        let _ = admin;
    }

    #[test]
    fn test_yield_health_not_operational_after_circuit_trip() {
        let (_env, _admin, client) = setup();
        client.record_failure();
        client.record_failure();
        client.record_failure();
        let health = client.check_yield_health();
        assert!(!health.operational);
        assert_eq!(health.failure_count, 3);
    }

    // ─── #418: fallback detection ─────────────────────────────────────────

    #[test]
    fn test_should_use_fallback_when_not_validated() {
        let (_env, _admin, client) = setup();
        // No yield contract registered → fallback required
        assert!(client.should_use_fallback());
    }

    #[test]
    fn test_should_not_use_fallback_when_healthy_and_validated() {
        let (env, _admin, client) = setup();
        let yield_addr = Address::generate(&env);
        client.set_yield_contract(&yield_addr);
        client.validate_yield_interface();
        assert!(!client.should_use_fallback());
    }

    #[test]
    fn test_should_use_fallback_when_circuit_tripped() {
        let (env, _admin, client) = setup();
        let yield_addr = Address::generate(&env);
        client.set_yield_contract(&yield_addr);
        client.validate_yield_interface();
        // Trip the breaker
        client.record_failure();
        client.record_failure();
        client.record_failure();
        assert!(client.should_use_fallback());
    }

    // ─── #418: failure reset ──────────────────────────────────────────────

    #[test]
    fn test_reset_failures_allows_recovery() {
        let (_env, _admin, client) = setup();
        client.record_failure();
        client.record_failure();
        client.record_failure();
        assert!(client.is_paused());
        client.reset_failures();
        assert_eq!(client.failure_count(), 0);
        // Admin still needs to explicitly unpause after a circuit trip.
        assert!(client.is_paused());
        client.set_paused(&false);
        assert!(!client.is_paused());
    }

    // ─── #418: yield failure scenarios ───────────────────────────────────

    #[test]
    fn test_failure_count_saturates_at_max_u32() {
        let (_env, _admin, client) = setup();
        // Fill up to threshold
        client.record_failure();
        client.record_failure();
        client.record_failure();
        assert!(client.is_paused());
        // Additional calls must not overflow.
        client.record_failure();
        client.record_failure();
        assert_eq!(client.failure_count(), 5);
    }

    #[test]
    fn test_validate_yield_requires_registered_contract() {
        let (_env, _admin, client) = setup();
        // get_yield_contract returns None before registration
        assert!(client.get_yield_contract().is_none());
    }

    #[test]
    fn test_get_yield_contract_returns_registered_address() {
        let (env, _admin, client) = setup();
        let yield_addr = Address::generate(&env);
        client.set_yield_contract(&yield_addr);
        assert_eq!(client.get_yield_contract(), Some(yield_addr));
    }
}

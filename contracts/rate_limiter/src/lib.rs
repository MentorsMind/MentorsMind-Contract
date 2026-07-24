#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotAdmin = 3,
    AlreadyWhitelisted = 4,
    NotWhitelisted = 5,
}

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CallRecord {
    pub count: u32,
    pub window_start: u64,
}

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    CallCount(Address, Symbol),
    WindowStart(Address, Symbol),
    Whitelist(Address),
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct RateLimiterContract;

#[contractimpl]
impl RateLimiterContract {
    /// Initialize the rate limiter contract.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        Ok(())
    }

    /// Check if a call is allowed under rate limits.
    /// Returns true if allowed, false if rate limit exceeded.
    ///
    /// Uses temporary storage so counts auto-expire with TTL.
    pub fn check_rate_limit(
        env: Env,
        caller: Address,
        action: Symbol,
        max_calls: u32,
        window_seconds: u32,
    ) -> bool {
        caller.require_auth();
        // Check if caller is whitelisted
        if env
            .storage()
            .instance()
            .has(&DataKey::Whitelist(caller.clone()))
        {
            return true;
        }

        let now = env.ledger().timestamp();
        let window_start_key = DataKey::WindowStart(caller.clone(), action.clone());
        let call_count_key = DataKey::CallCount(caller.clone(), action.clone());

        // Get current window start time
        let window_start: u64 = env
            .storage()
            .temporary()
            .get(&window_start_key)
            .unwrap_or(0);

        let window_seconds_u64 = window_seconds as u64;

        // Check if we need to reset the window
        if now >= window_start.saturating_add(window_seconds_u64) {
            // Reset the counter and window timestamp together so the TTL always
            // reflects the active policy window rather than the previous one.
            // New window - reset counter
            env.storage().temporary().set(&window_start_key, &now);
            env.storage().temporary().set(&call_count_key, &1u32);

            // Extend TTL for the new window
            env.storage()
                .temporary()
                .extend_ttl(&window_start_key, window_seconds, window_seconds);
            env.storage()
                .temporary()
                .extend_ttl(&call_count_key, window_seconds, window_seconds);

            return true;
        }

        // Within current window - check and increment counter
        let current_count: u32 = env.storage().temporary().get(&call_count_key).unwrap_or(0);

        if current_count >= max_calls {
            // Emit a dedicated event so operators can tell policy enforcement
            // apart from genuine execution failures in downstream logs.
            // Rate limit exceeded - emit event
            env.events().publish(
                (
                    symbol_short!("rate"),
                    symbol_short!("exceeded"),
                    caller.clone(),
                ),
                (action.clone(), current_count, max_calls),
            );
            return false;
        }

        // Increment counter
        let new_count = current_count + 1;
        env.storage().temporary().set(&call_count_key, &new_count);

        // Extend TTL only for the remaining time in the window so the counter
        // expires with the policy interval.
        let remaining_u64 = window_start.saturating_add(window_seconds_u64).saturating_sub(now);
        let remaining_seconds = remaining_u64.min(u32::MAX as u64) as u32;
        env.storage()
            .temporary()
            .extend_ttl(&call_count_key, remaining_seconds, remaining_seconds);

        true
    }

    /// Get current call count for a caller and action.
    pub fn get_call_count(env: Env, caller: Address, action: Symbol) -> u32 {
        env.storage()
            .temporary()
            .get(&DataKey::CallCount(caller, action))
            .unwrap_or(0)
    }

    /// Add an address to the whitelist (bypasses rate limiting).
    /// Admin only.
    pub fn add_to_whitelist(env: Env, address: Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;

        admin.require_auth();

        if env
            .storage()
            .instance()
            .has(&DataKey::Whitelist(address.clone()))
        {
            return Err(Error::AlreadyWhitelisted);
        }

        env.storage()
            .instance()
            .set(&DataKey::Whitelist(address.clone()), &true);

        env.events().publish(
            (symbol_short!("whitelist"), symbol_short!("added")),
            address,
        );

        Ok(())
    }

    /// Remove an address from the whitelist.
    /// Admin only.
    pub fn remove_from_whitelist(env: Env, address: Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;

        admin.require_auth();

        if !env
            .storage()
            .instance()
            .has(&DataKey::Whitelist(address.clone()))
        {
            return Err(Error::NotWhitelisted);
        }

        env.storage()
            .instance()
            .remove(&DataKey::Whitelist(address.clone()));

        env.events().publish(
            (symbol_short!("whitelist"), symbol_short!("removed")),
            address,
        );

        Ok(())
    }

    /// Check if an address is whitelisted.
    pub fn is_whitelisted(env: Env, address: Address) -> bool {
        env.storage().instance().has(&DataKey::Whitelist(address))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    fn setup() -> (Env, Address, Address, RateLimiterContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, RateLimiterContract);
        let client = RateLimiterContractClient::new(&env, &contract_id);

        client.initialize(&admin);

        (env, admin, contract_id, client)
    }

    #[test]
    fn test_under_limit_passes() {
        let (env, _admin, _contract_id, client) = setup();

        let caller = Address::generate(&env);
        let action = symbol_short!("transfer");

        // First call should pass
        assert!(client.check_rate_limit(&caller, &action, &3, &60));
        assert_eq!(client.get_call_count(&caller, &action), 1);

        // Second call should pass
        assert!(client.check_rate_limit(&caller, &action, &3, &60));
        assert_eq!(client.get_call_count(&caller, &action), 2);

        // Third call should pass
        assert!(client.check_rate_limit(&caller, &action, &3, &60));
        assert_eq!(client.get_call_count(&caller, &action), 3);
    }

    #[test]
    fn test_over_limit_fails() {
        let (env, _admin, _contract_id, client) = setup();

        let caller = Address::generate(&env);
        let action = symbol_short!("transfer");

        // Use up all allowed calls
        client.check_rate_limit(&caller, &action, &2, &60);
        client.check_rate_limit(&caller, &action, &2, &60);

        // Third call should fail
        assert!(!client.check_rate_limit(&caller, &action, &2, &60));
        assert_eq!(client.get_call_count(&caller, &action), 2);
    }

    #[test]
    fn test_window_reset() {
        let (env, _admin, _contract_id, client) = setup();
        env.ledger().set_timestamp(0);

        let caller = Address::generate(&env);
        let action = symbol_short!("transfer");

        // Use up all allowed calls
        client.check_rate_limit(&caller, &action, &2, &60);
        client.check_rate_limit(&caller, &action, &2, &60);

        // Third call should fail
        assert!(!client.check_rate_limit(&caller, &action, &2, &60));

        // Advance time past the window
        env.ledger().set_timestamp(61);

        // Should pass again
        assert!(client.check_rate_limit(&caller, &action, &2, &60));
        assert_eq!(client.get_call_count(&caller, &action), 1);
    }

    #[test]
    fn test_whitelist_bypass() {
        let (env, _admin, _contract_id, client) = setup();

        let caller = Address::generate(&env);
        let action = symbol_short!("transfer");

        // Add to whitelist
        client.add_to_whitelist(&caller);

        // Should always pass even after exceeding limit
        for _ in 0..10 {
            assert!(client.check_rate_limit(&caller, &action, &2, &60));
        }
    }

    #[test]
    fn test_add_remove_whitelist() {
        let (env, _admin, _contract_id, client) = setup();

        let address = Address::generate(&env);

        assert!(!client.is_whitelisted(&address));

        client.add_to_whitelist(&address);
        assert!(client.is_whitelisted(&address));

        client.remove_from_whitelist(&address);
        assert!(!client.is_whitelisted(&address));
    }
}

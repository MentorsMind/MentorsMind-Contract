use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

/// Instance storage keys used by the pause guard.
///
/// `shared` is a library crate with no single canonical `DataKey`, so the flag
/// is keyed by symbol. Storage is namespaced per contract instance, so these
/// keys cannot collide with a host contract's own keys.
pub const PAUSE_GUARDIAN: Symbol = symbol_short!("PAUSE_GDN");
pub const PAUSE_FLAG: Symbol = symbol_short!("PAUSED");

/// Emergency pause state shared by value-moving contracts.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PauseState {
    Active,
    Paused,
}

impl PauseState {
    pub fn is_paused(&self) -> bool {
        matches!(self, PauseState::Paused)
    }
}

/// Current pause state. Defaults to `Active` so a contract that never opts in
/// behaves exactly as it did before.
pub fn pause_state(env: &Env) -> PauseState {
    if env.storage().instance().get(&PAUSE_FLAG).unwrap_or(false) {
        PauseState::Paused
    } else {
        PauseState::Active
    }
}

pub fn is_paused(env: &Env) -> bool {
    pause_state(env).is_paused()
}

/// Entry-point guard for state-mutating operations.
///
/// Panics with `contract paused` so the halt is unambiguous in simulation logs
/// and transaction metadata. Call this *first* in a guarded entry point, before
/// any `require_auth`, so a halted contract rejects the transaction outright.
pub fn require_not_paused(env: &Env) {
    if is_paused(env) {
        panic!("contract paused");
    }
}

/// Stores the guardian allowed to trigger the emergency pause. `admin` must
/// authorise the call. Passing `admin` as the guardian keeps single-operator
/// deployments to one address.
pub fn set_pause_guardian(env: &Env, admin: &Address, guardian: &Address) {
    admin.require_auth();
    env.storage().instance().set(&PAUSE_GUARDIAN, guardian);
}

pub fn pause_guardian(env: &Env) -> Option<Address> {
    env.storage().instance().get(&PAUSE_GUARDIAN)
}

pub fn require_pause_guardian(env: &Env) {
    let guardian = pause_guardian(env).unwrap_or_else(|| panic!("pause guardian not set"));
    guardian.require_auth();
}

/// Flips the contract into the paused state. Guardian only.
pub fn pause(env: &Env) {
    require_pause_guardian(env);
    set_paused(env, true);
}

/// Flips the contract back to the active state. Guardian only.
pub fn unpause(env: &Env) {
    require_pause_guardian(env);
    set_paused(env, false);
}

/// Writes the raw flag without an authorisation check.
///
/// Used by hosts that keep the guardian in their own `DataKey` (so they can
/// migrate the key without moving the flag) and therefore authorise the caller
/// themselves before calling this.
pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&PAUSE_FLAG, &paused);
//! # Pause Guardian Cross-Contract Integration
//!
//! Provides utilities for contracts to atomically check the pause state
//! via the pause_guardian contract before executing state-mutating operations.
//!
//! ## Design
//!
//! Payment-path entry points (deposit, stake, claim_rewards, deploy_escrow, etc.)
//! must call `require_not_paused(env, guardian_address)` at the top of the function.
//! If the guardian contract reports `is_paused() == true`, the call panics with
//! `ContractPaused` and the transaction is rolled back atomically.
//!
//! ## Performance
//!
//! Each cross-contract call to `is_paused()` is a single Soroban host invocation.
//! The pause state is checked **synchronously** within the same transaction ledger,
//! guaranteeing that pause takes effect immediately without requiring separate
//! ledger rounds.

use soroban_sdk::{Address, Env, Symbol};

/// Error returned when a contract is in a paused state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractPaused;

impl ContractPaused {
    pub fn msg() -> &'static str {
        "Contract is paused"
    }
}

/// Check the pause state atomically via cross-contract call.
///
/// # Arguments
///
/// * `env` — The Soroban contract environment.
/// * `guardian_address` — The address of the pause_guardian contract.
///
/// # Returns
///
/// `true` if the guardian reports `is_paused()`.
///
/// # Panics
///
/// Panics if the cross-contract call fails (e.g., guardian not found, invalid return type).
pub fn is_paused(env: &Env, guardian_address: &Address) -> bool {
    env.invoke_contract(
        guardian_address,
        &Symbol::new(env, "is_paused"),
        soroban_sdk::Vec::<soroban_sdk::Val>::new(env),
    )
}

/// Assert that the contract is not paused.
///
/// Calls `is_paused(env, guardian_address)` and panics with "Contract is paused"
/// if the result is `true`. Otherwise returns normally.
///
/// # Usage
///
/// ```ignore
/// pub fn deposit(env: Env, from: Address, token: Address, amount: i128) {
///     let guardian = get_pause_guardian(&env);
///     require_not_paused(&env, &guardian);
///     // ... rest of deposit logic
/// }
/// ```
pub fn require_not_paused(env: &Env, guardian_address: &Address) {
    if is_paused(env, guardian_address) {
        panic!("{}", ContractPaused::msg());
    }
}

//! # Pause Guardian Integration
//!
//! Provides utilities for contracts to check pause state, either via local
//! instance storage (used by collateral_loan) or atomically via cross-contract
//! calls to the `pause_guardian` contract (used by staking, treasury, referral).

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

// ---------------------------------------------------------------------------
// Cross-contract pause guard (via pause_guardian contract)
// ---------------------------------------------------------------------------

/// Error returned when a contract is in a paused state.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractPaused;

impl ContractPaused {
    pub fn msg() -> &'static str {
        "Contract is paused"
    }
}

/// Check the pause state atomically via cross-contract call.
pub fn is_paused(env: &Env, guardian_address: &Address) -> bool {
    env.invoke_contract(
        guardian_address,
        &Symbol::new(env, "is_paused"),
        soroban_sdk::Vec::<soroban_sdk::Val>::new(env),
    )
}

/// Assert that the contract is not paused via cross-contract guardian.
///
/// Calls `is_paused(env, guardian_address)` and panics with "Contract is paused"
/// if the result is `true`. Otherwise returns normally.
pub fn require_not_paused(env: &Env, guardian_address: &Address) {
    if is_paused(env, guardian_address) {
        panic!("{}", ContractPaused::msg());
    }
}

// ---------------------------------------------------------------------------
// Local instance-storage pause guard (used by collateral_loan)
// ---------------------------------------------------------------------------

/// Instance storage keys used by the local pause guard.
pub const PAUSE_GUARDIAN: Symbol = symbol_short!("PAUSE_GDN");
pub const PAUSE_FLAG: Symbol = symbol_short!("PAUSED");

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

/// Current local pause state. Defaults to `Active`.
pub fn pause_state(env: &Env) -> PauseState {
    if env.storage().instance().get(&PAUSE_FLAG).unwrap_or(false) {
        PauseState::Paused
    } else {
        PauseState::Active
    }
}

pub fn pause_guard_is_paused(env: &Env) -> bool {
    pause_state(env).is_paused()
}

pub fn require_not_paused_locally(env: &Env) {
    if pause_guard_is_paused(env) {
/// Check local instance pause flag.
pub fn is_paused_local(env: &Env) -> bool {
    pause_state(env).is_paused()
}

/// Assert that the local contract instance is not paused.
pub fn require_not_paused_local(env: &Env) {
    if is_paused_local(env) {
        panic!("contract paused");
    }
}

/// Stores the guardian allowed to trigger the emergency pause. `admin` must authorise.
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

/// Flips the local contract into the paused state. Guardian only.
pub fn pause(env: &Env) {
    require_pause_guardian(env);
    set_paused(env, true);
}

/// Flips the local contract back to the active state. Guardian only.
pub fn unpause(env: &Env) {
    require_pause_guardian(env);
    set_paused(env, false);
}

pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&PAUSE_FLAG, &paused);
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractPaused;

impl ContractPaused {
    pub fn msg() -> &'static str {
        "Contract is paused"
    }
}

pub fn is_paused(env: &Env, guardian_address: &Address) -> bool {
    env.invoke_contract(
        guardian_address,
        &Symbol::new(env, "is_paused"),
        soroban_sdk::Vec::<soroban_sdk::Val>::new(env),
    )
}

pub fn require_not_paused(env: &Env, guardian_address: &Address) {
    if is_paused(env, guardian_address) {
        panic!("{}", ContractPaused::msg());
    }
/// Writes the raw flag without an authorisation check.
pub fn set_paused(env: &Env, paused: bool) {
    env.storage().instance().set(&PAUSE_FLAG, &paused);
}

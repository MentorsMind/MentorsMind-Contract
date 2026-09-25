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
}

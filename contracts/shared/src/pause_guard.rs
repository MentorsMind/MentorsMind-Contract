use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

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
        panic!("contract paused");
    }
}

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

pub fn pause(env: &Env) {
    require_pause_guardian(env);
    set_paused(env, true);
}

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
}

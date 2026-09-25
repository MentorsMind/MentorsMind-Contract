#![no_std]

use soroban_sdk::{contracttype, Address, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossContractRecoveryState {
    pub contract_id: Address,
    pub action: Symbol,
    pub rollback_required: bool,
    pub execution_successful: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollbackProtector {
    pub snapshot_id: u64,
    pub is_active: bool,
}

pub fn trigger_rollback(contract: Address, action: Symbol) -> CrossContractRecoveryState {
    CrossContractRecoveryState {
        contract_id: contract,
        action,
        rollback_required: true,
        execution_successful: false,
    }
}

pub fn execute_with_recovery<F, R>(
    contract: Address,
    action: Symbol,
    f: F,
) -> Result<R, CrossContractRecoveryState>
where
    F: FnOnce() -> Result<R, ()>,
{
    match f() {
        Ok(res) => Ok(res),
        Err(_) => Err(trigger_rollback(contract, action)),
    }
}

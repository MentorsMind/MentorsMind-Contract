#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingAdminChange {
    pub new_admin: Address,
    pub effective_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Contracts,
}

#[contract]
pub struct AdminRotationCoordinator;

#[contractimpl]
impl AdminRotationCoordinator {
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Contracts, &Vec::<Address>::new(&env));
    }

    pub fn register_contract(env: Env, admin: Address, contract: Address) {
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).expect("not initialized");
        if stored_admin != admin {
            panic!("unauthorized");
        }
        admin.require_auth();
        let mut contracts: Vec<Address> = env.storage().instance().get(&DataKey::Contracts).unwrap_or(Vec::new(&env));
        if !contracts.iter().any(|existing| existing == contract) {
            contracts.push_back(contract);
            env.storage().instance().set(&DataKey::Contracts, &contracts);
        }
    }

    pub fn batch_propose_admin_change(env: Env, admin: Address, new_admin: Address) {
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).expect("not initialized");
        if stored_admin != admin {
            panic!("unauthorized");
        }
        admin.require_auth();
        let contracts: Vec<Address> = env.storage().instance().get(&DataKey::Contracts).unwrap_or(Vec::new(&env));
        for contract in contracts.iter() {
            let _: () = env.invoke_contract(&contract, &Symbol::new(&env, "propose_admin_change"), (admin.clone(), new_admin.clone()).into_val(&env));
        }
    }

    pub fn get_registered_contracts(env: Env) -> Vec<Address> {
        env.storage().instance().get(&DataKey::Contracts).unwrap_or(Vec::new(&env))
    }
}

#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Interface(Symbol),
    InterfaceIds,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceEntry {
    pub interface_id: Symbol,
    pub contract: Address,
    pub version: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceData {
    pub contract: Address,
    pub version: u32,
}

#[contract]
pub struct InterfaceRegistryContract;

#[contractimpl]
impl InterfaceRegistryContract {
    const YIELD_INTERFACE: &'static str = "yield_v1";

    pub fn initialize(env: Env, admin: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::InterfaceIds, &Vec::<Symbol>::new(&env));
    }

    pub fn register_interface(env: Env, contract: Address, interface_id: Symbol, version: u32) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        let key = DataKey::Interface(interface_id.clone());
        let mut is_new = false;

        let mut ids: Vec<Symbol> = env
            .storage()
            .persistent()
            .get(&DataKey::InterfaceIds)
            .unwrap_or_else(|| Vec::new(&env));

        if !env.storage().persistent().has(&key) {
            ids.push_back(interface_id.clone());
            env.storage().persistent().set(&DataKey::InterfaceIds, &ids);
            is_new = true;
        }

        env.storage().persistent().set(
            &key,
            &InterfaceData {
                contract: contract.clone(),
                version,
            },
        );

        if is_new {
            env.events().publish(
                (Symbol::new(&env, "interface_registered"), interface_id),
                (contract, version),
            );
        } else {
            env.events().publish(
                (Symbol::new(&env, "interface_updated"), interface_id),
                (contract, version),
            );
        }
    }

    pub fn get_contract(env: Env, interface_id: Symbol) -> Address {
        let key = DataKey::Interface(interface_id);
        let data: InterfaceData = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Interface not found");
        data.contract
    }

    pub fn get_version(env: Env, interface_id: Symbol) -> u32 {
        let key = DataKey::Interface(interface_id);
        let data: InterfaceData = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Interface not found");
        data.version
    }

    pub fn list_interfaces(env: Env) -> Vec<InterfaceEntry> {
        let mut result = Vec::new(&env);
        let ids: Vec<Symbol> = env
            .storage()
            .persistent()
            .get(&DataKey::InterfaceIds)
            .unwrap_or_else(|| Vec::new(&env));

        for idx in 0..ids.len() {
            let interface_id = ids.get(idx).expect("Index out of range");
            let key = DataKey::Interface(interface_id.clone());
            let data: InterfaceData = env
                .storage()
                .persistent()
                .get(&key)
                .expect("Interface not found");
            result.push_back(InterfaceEntry {
                interface_id: interface_id.clone(),
                contract: data.contract,
                version: data.version,
            });
        }

        result
    }

    pub fn register_yield_contract(env: Env, contract: Address, version: u32) {
        Self::register_interface(
            env.clone(),
            contract,
            Symbol::new(&env, Self::YIELD_INTERFACE),
            version,
        );
    }

    pub fn get_yield_contract(env: Env) -> Address {
        Self::get_contract(env.clone(), Symbol::new(&env, Self::YIELD_INTERFACE))
    }

    pub fn get_yield_contract_version(env: Env) -> u32 {
        Self::get_version(env.clone(), Symbol::new(&env, Self::YIELD_INTERFACE))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{Env, Symbol};

    fn setup(env: &Env) -> (InterfaceRegistryContractClient, Address, Address) {
        let admin = Address::generate(env);
        let registry_id = env.register_contract(None, InterfaceRegistryContract);
        let registry = InterfaceRegistryContractClient::new(env, &registry_id);
        registry.initialize(&admin);
        (registry, admin, Address::generate(env))
    }

    #[test]
    fn test_register_and_lookup() {
        let env = Env::default();
        env.mock_all_auths();

        let (registry, _admin, escrow) = setup(&env);
        registry.register_interface(&escrow, &Symbol::new(&env, "escrow_v1"), &1);

        assert_eq!(
            registry.get_contract(&Symbol::new(&env, "escrow_v1")),
            escrow
        );
        assert_eq!(registry.get_version(&Symbol::new(&env, "escrow_v1")), 1);
    }

    #[test]
    fn test_update_interface() {
        let env = Env::default();
        env.mock_all_auths();

        let (registry, _admin, escrow1) = setup(&env);
        let escrow2 = Address::generate(&env);
        let interface = Symbol::new(&env, "escrow_v1");

        registry.register_interface(&escrow1, &interface, &1);
        assert_eq!(registry.get_contract(&interface), escrow1);
        assert_eq!(registry.get_version(&interface), 1);

        registry.register_interface(&escrow2, &interface, &2);
        assert_eq!(registry.get_contract(&interface), escrow2);
        assert_eq!(registry.get_version(&interface), 2);
    }

    #[test]
    fn test_list_interfaces() {
        let env = Env::default();
        env.mock_all_auths();

        let (registry, _admin, escrow) = setup(&env);

        registry.register_interface(&escrow, &Symbol::new(&env, "escrow_v1"), &1);
        registry.register_interface(
            &Address::generate(&env),
            &Symbol::new(&env, "oracle_v1"),
            &1,
        );

        let list = registry.list_interfaces();
        assert_eq!(list.len(), 2);

        let mut interface_names: Vec<Symbol> = Vec::new(&env);
        for item in list.iter() {
            interface_names.push_back(item.interface_id.clone());
        }

        assert!(interface_names.contains(&Symbol::new(&env, "escrow_v1")));
        assert!(interface_names.contains(&Symbol::new(&env, "oracle_v1")));
    }

    #[test]
    #[should_panic]
    fn test_register_interface_unauthorized() {
        let env = Env::default();
        // do not call mock_all_auths, to enforce auth failure

        let (registry, _admin, escrow) = setup(&env);
        registry.register_interface(&escrow, &Symbol::new(&env, "escrow_v1"), &1);
    }

    #[test]
    fn test_register_and_get_yield_contract() {
        let env = Env::default();
        env.mock_all_auths();

        let (registry, _admin, yield_contract) = setup(&env);
        registry.register_yield_contract(&yield_contract, &3);

        assert_eq!(registry.get_yield_contract(), yield_contract);
        assert_eq!(registry.get_yield_contract_version(), 3);
    }
}

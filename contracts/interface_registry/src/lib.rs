#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Interface(Symbol),
    InterfaceIds,
    InterfaceDescriptor(Symbol),
    Quarantined(Address),
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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceDescriptor {
    pub functions: Vec<(Symbol, u32)>,
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

        // Store default empty descriptor if not already present
        let descriptor_key = DataKey::InterfaceDescriptor(interface_id.clone());
        if !env.storage().persistent().has(&descriptor_key) {
            env.storage().persistent().set(
                &descriptor_key,
                &InterfaceDescriptor {
                    functions: Vec::new(&env),
                },
            );
        }

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

    /// Verify that a contract at `address` is registered with the expected
    /// interface and has not been quarantined.
    pub fn verify(env: Env, address: Address, expected_interface: Symbol) -> bool {
        if Self::is_quarantined(env.clone(), address.clone()) {
            return false;
        }
        let key = DataKey::Interface(expected_interface);
        match env.storage().persistent().get::<_, InterfaceData>(&key) {
            Some(data) => data.contract == address,
            None => false,
        }
    }

    /// Emergency isolation: mark `contract` as quarantined so `verify` (and
    /// therefore every consumer that gates cross-contract calls on it, e.g.
    /// `CrossContractAuth::require_authorized_contract`) rejects it, even if
    /// it remains registered under an interface. Admin-only.
    pub fn quarantine_contract(env: Env, contract: Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        env.storage()
            .persistent()
            .set(&DataKey::Quarantined(contract.clone()), &true);

        env.events()
            .publish((Symbol::new(&env, "contract_quarantined"),), (contract, admin));
    }

    /// Lift a quarantine previously placed on `contract`. Admin-only.
    pub fn unquarantine_contract(env: Env, contract: Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        env.storage()
            .persistent()
            .remove(&DataKey::Quarantined(contract.clone()));

        env.events().publish(
            (Symbol::new(&env, "contract_unquarantined"),),
            (contract, admin),
        );
    }

    /// Whether `contract` is currently quarantined.
    pub fn is_quarantined(env: Env, contract: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Quarantined(contract))
            .unwrap_or(false)
    }

    /// Panics if the contract at `address` is not registered with the expected interface.
    pub fn require_interface(env: Env, address: Address, expected_interface: Symbol) {
        if !Self::verify(env.clone(), address, expected_interface) {
            panic!("interface mismatch");
        }
    }

    /// Store an interface descriptor for a given interface_id.
    pub fn set_interface_descriptor(
        env: Env,
        interface_id: Symbol,
        descriptor: InterfaceDescriptor,
    ) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        let descriptor_key = DataKey::InterfaceDescriptor(interface_id);
        env.storage().persistent().set(&descriptor_key, &descriptor);
    }

    /// Get interface descriptor for a given interface_id.
    pub fn get_interface_descriptor(env: Env, interface_id: Symbol) -> InterfaceDescriptor {
        let descriptor_key = DataKey::InterfaceDescriptor(interface_id);
        env.storage()
            .persistent()
            .get(&descriptor_key)
            .unwrap_or(InterfaceDescriptor {
                functions: Vec::new(&env),
            })
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

    #[test]
    fn test_verify_and_require_interface() {
        let env = Env::default();
        env.mock_all_auths();
        let (registry, _admin, escrow) = setup(&env);
        let interface = Symbol::new(&env, "escrow_v1");
        registry.register_interface(&escrow, &interface, &1);
        assert!(registry.verify(&escrow, &interface));
        let other = Address::generate(&env);
        assert!(!registry.verify(&other, &interface));
        // require_interface should not panic for correct address
        registry.require_interface(&escrow, &interface);
        // require_interface should panic for wrong address
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            registry.require_interface(&other, &interface);
        }));
        assert!(result.is_err());
    }
}

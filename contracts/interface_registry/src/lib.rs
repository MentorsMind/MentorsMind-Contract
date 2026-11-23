#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, Symbol, Vec};

/// How long a deregistration waits before it may be finalized (#1432).
pub const DEREGISTER_COOLDOWN_SECS: u64 = 48 * 3600;

/// Errors returned by interface registration (#1431).
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// A registration reused a version while changing the interface hash.
    VersionNotBumped = 1,
    /// The supplied version is zero.
    InvalidVersion = 2,
    /// No interface is registered under that id.
    InterfaceNotFound = 3,
    /// Finalize was called with no deregistration pending.
    DeregistrationNotPending = 4,
    /// The deregistration cooldown has not elapsed yet.
    CooldownActive = 5,
}

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
    /// Minimum acceptable version per interface id (#1431).
    MinVersion(Symbol),
    /// Ledger time at which `deregister_interface` was first called (#1432).
    PendingDeregistration(Symbol),
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
    /// Hash of the interface definition registered at `version` (#1431).
    pub interface_hash: BytesN<32>,
}

/// Canonical name for the stored interface record (#1431).
pub type InterfaceRecord = InterfaceData;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceDescriptor {
    pub functions: Vec<(Symbol, u32)>,
}

#[contract]
pub struct InterfaceRegistryContract;

#[contractimpl]
impl InterfaceRegistryContract {
    /// Starts the cooldown before `interface_id` is removed (#1432).
    ///
    /// The interface keeps working for `DEREGISTER_COOLDOWN_SECS`, so dependent
    /// contracts can react. A repeat call during the cooldown keeps the original
    /// timestamp. Returns the time after which `finalize_deregistration` may run.
    pub fn deregister_interface(env: Env, interface_id: Symbol) -> Result<u64, Error> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        if !env.storage().persistent().has(&DataKey::Interface(interface_id.clone())) {
            return Err(Error::InterfaceNotFound);
        }

        let key = DataKey::PendingDeregistration(interface_id);
        let proposed_at = match env.storage().persistent().get::<_, u64>(&key) {
            Some(existing) => existing,
            None => {
                let now = env.ledger().timestamp();
                env.storage().persistent().set(&key, &now);
                now
            }
        };
        Ok(proposed_at.saturating_add(DEREGISTER_COOLDOWN_SECS))
    }

    /// Removes an interface once its deregistration cooldown has elapsed (#1432).
    pub fn finalize_deregistration(env: Env, interface_id: Symbol) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        let pending_key = DataKey::PendingDeregistration(interface_id.clone());
        let proposed_at: u64 = env
            .storage()
            .persistent()
            .get(&pending_key)
            .ok_or(Error::DeregistrationNotPending)?;
        let now = env.ledger().timestamp();
        if now < proposed_at.saturating_add(DEREGISTER_COOLDOWN_SECS) {
            return Err(Error::CooldownActive);
        }

        env.storage().persistent().remove(&DataKey::Interface(interface_id.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::InterfaceDescriptor(interface_id.clone()));
        env.storage().persistent().remove(&pending_key);

        let ids: Vec<Symbol> = env
            .storage()
            .persistent()
            .get(&DataKey::InterfaceIds)
            .unwrap_or_else(|| Vec::new(&env));
        let mut kept: Vec<Symbol> = Vec::new(&env);
        for id in ids.iter() {
            if id != interface_id {
                kept.push_back(id);
            }
        }
        env.storage().persistent().set(&DataKey::InterfaceIds, &kept);
        Ok(())
    }

    const YIELD_INTERFACE: &'static str = "yield_v1";
    /// Canonical interface id under which resolution oracles are registered.
    /// Consumers (e.g. the prediction market) validate an oracle address by
    /// calling `verify(address, oracle_v1)` / `is_registered_oracle(address)`.
    const ORACLE_INTERFACE: &'static str = "oracle_v1";

    pub fn initialize(env: Env, admin: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::InterfaceIds, &Vec::<Symbol>::new(&env));
    }

    /// Register an interface without an explicit hash (legacy entry point).
    ///
    /// Uses a zero hash; callers that track interface definitions should use
    /// `register_interface_with_hash` so a version bump can be enforced.
    pub fn register_interface(
        env: Env,
        contract: Address,
        interface_id: Symbol,
        version: u32,
    ) -> Result<(), Error> {
        let zero_hash = BytesN::from_array(&env, &[0u8; 32]);
        Self::register_interface_with_hash(env, contract, interface_id, version, zero_hash)
    }

    /// Register a contract interface with its definition hash (#1431).
    ///
    /// Re-registering the same version with a different hash is rejected: the
    /// version must be bumped whenever the interface changes.
    pub fn register_interface_with_hash(
        env: Env,
        contract: Address,
        interface_id: Symbol,
        version: u32,
        interface_hash: BytesN<32>,
    ) -> Result<(), Error> {
        if version == 0 {
            return Err(Error::InvalidVersion);
        }

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        let key = DataKey::Interface(interface_id.clone());

        // Reject a changed interface that did not bump its version.
        if let Some(existing) = env.storage().persistent().get::<_, InterfaceData>(&key) {
            if existing.version == version && existing.interface_hash != interface_hash {
                return Err(Error::VersionNotBumped);
            }
        }

        let min_version = Self::get_min_version(env.clone(), interface_id.clone());
        if version < min_version {
            return Err(Error::InvalidVersion);
        }

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
                interface_hash,
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

        Ok(())
    }

    /// Recorded hash of the interface definition (#1431).
    pub fn get_interface_hash(env: Env, interface_id: Symbol) -> BytesN<32> {
        let key = DataKey::Interface(interface_id);
        let data: InterfaceData = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Interface not found");
        data.interface_hash
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

    pub fn register_yield_contract(env: Env, contract: Address, version: u32) -> Result<(), Error> {
        Self::register_interface(
            env.clone(),
            contract,
            Symbol::new(&env, Self::YIELD_INTERFACE),
            version,
        )
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
        let key = DataKey::Interface(expected_interface.clone());
        match env.storage().persistent().get::<_, InterfaceData>(&key) {
            Some(data) => {
                // Consumers must not accept a registration below the
                // governance-defined minimum version (#1431).
                data.contract == address && data.version >= Self::get_min_version(env, expected_interface)
            }
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

    /// Register `contract` as a resolution oracle under the canonical
    /// `oracle_v1` interface id. Admin-only (delegates to `register_interface`).
    pub fn register_oracle(env: Env, contract: Address, version: u32) -> Result<(), Error> {
        Self::register_interface(
            env.clone(),
            contract,
            Symbol::new(&env, Self::ORACLE_INTERFACE),
            version,
        )
    }

    // ── Minimum acceptable version (#1431) ──────────────────────────────────

    /// Set the minimum acceptable version for an interface. Admin-only.
    pub fn set_min_version(
        env: Env,
        admin: Address,
        interface_id: Symbol,
        min_version: u32,
    ) -> Result<(), Error> {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("Unauthorized");
        }
        if min_version == 0 {
            return Err(Error::InvalidVersion);
        }
        env.storage()
            .persistent()
            .set(&DataKey::MinVersion(interface_id), &min_version);
        Ok(())
    }

    /// Minimum version consumers should accept for `interface_id` (default 1).
    pub fn get_min_version(env: Env, interface_id: Symbol) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::MinVersion(interface_id))
            .unwrap_or(1)
    }

    /// Whether `address` is a registered (and non-quarantined) resolution
    /// oracle, i.e. registered under the canonical `oracle_v1` interface id.
    /// Stable cross-contract entry point for consumers such as the prediction
    /// market to validate a `resolution_oracle` before use.
    pub fn is_registered_oracle(env: Env, address: Address) -> bool {
        Self::verify(env.clone(), address, Symbol::new(&env, Self::ORACLE_INTERFACE))
    }

    /// Panics unless `address` is a registered resolution oracle.
    pub fn require_oracle(env: Env, address: Address) {
        if !Self::is_registered_oracle(env.clone(), address) {
            panic!("oracle not registered");
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
    fn test_register_oracle_and_verify() {
        let env = Env::default();
        env.mock_all_auths();

        let (registry, _admin, oracle) = setup(&env);
        let interface = Symbol::new(&env, "oracle_v1");

        registry.register_oracle(&oracle, &1);

        // Registered oracle is accepted via both entry points.
        assert!(registry.verify(&oracle, &interface));
        assert!(registry.is_registered_oracle(&oracle));
        registry.require_oracle(&oracle);

        // An unregistered address is rejected.
        let unregistered = Address::generate(&env);
        assert!(!registry.verify(&unregistered, &interface));
        assert!(!registry.is_registered_oracle(&unregistered));
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            registry.require_oracle(&unregistered);
        }));
        assert!(result.is_err());
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

    // ── #1431: interface version compatibility ──────────────────────────────

    #[test]
    fn test_version_bump_is_allowed() {
        let env = Env::default();
        env.mock_all_auths();
        let (registry, _admin, contract) = setup(&env);
        let interface = Symbol::new(&env, "escrow_v1");
        let hash_v1 = BytesN::from_array(&env, &[1u8; 32]);
        let hash_v2 = BytesN::from_array(&env, &[2u8; 32]);

        registry.register_interface_with_hash(&contract, &interface, &1, &hash_v1);
        assert_eq!(registry.get_version(&interface), 1);
        assert_eq!(registry.get_interface_hash(&interface), hash_v1);

        // A changed interface with a bumped version is accepted.
        registry.register_interface_with_hash(&contract, &interface, &2, &hash_v2);
        assert_eq!(registry.get_version(&interface), 2);
        assert_eq!(registry.get_interface_hash(&interface), hash_v2);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_changed_interface_without_version_bump_is_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (registry, _admin, contract) = setup(&env);
        let interface = Symbol::new(&env, "escrow_v1");
        let hash_v1 = BytesN::from_array(&env, &[1u8; 32]);
        let hash_changed = BytesN::from_array(&env, &[9u8; 32]);

        registry.register_interface_with_hash(&contract, &interface, &1, &hash_v1);
        registry.register_interface_with_hash(&contract, &interface, &1, &hash_changed);
    }

    #[test]
    fn test_same_version_same_hash_is_idempotent() {
        let env = Env::default();
        env.mock_all_auths();
        let (registry, _admin, contract) = setup(&env);
        let interface = Symbol::new(&env, "escrow_v1");
        let hash = BytesN::from_array(&env, &[3u8; 32]);

        registry.register_interface_with_hash(&contract, &interface, &1, &hash);
        registry.register_interface_with_hash(&contract, &interface, &1, &hash);
        assert_eq!(registry.get_version(&interface), 1);
    }

    #[test]
    fn test_minimum_acceptable_version_is_queryable_and_enforced() {
        let env = Env::default();
        env.mock_all_auths();
        let (registry, _admin, contract) = setup(&env);
        let interface = Symbol::new(&env, "escrow_v1");
        let hash = BytesN::from_array(&env, &[4u8; 32]);

        registry.register_interface_with_hash(&contract, &interface, &1, &hash);
        assert_eq!(registry.get_min_version(&interface), 1);
        assert!(registry.verify(&contract, &interface));

        registry.set_min_version(&_admin, &interface, &2);
        assert_eq!(registry.get_min_version(&interface), 2);
        // A stale (below-minimum) registration no longer verifies.
        assert!(!registry.verify(&contract, &interface));

        registry.register_interface_with_hash(&contract, &interface, &2, &hash);
        assert!(registry.verify(&contract, &interface));
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #2)")]
    fn test_zero_version_is_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (registry, _admin, contract) = setup(&env);
        let interface = Symbol::new(&env, "escrow_v1");
        let hash = BytesN::from_array(&env, &[5u8; 32]);
        registry.register_interface_with_hash(&contract, &interface, &0, &hash);
    }
}

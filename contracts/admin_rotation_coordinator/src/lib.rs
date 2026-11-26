#![no_std]

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, Address, Env, IntoVal,
    Symbol, Vec,
};

/// Canonical interface id a coordinator must be registered with in the
/// `interface_registry` before it may participate in admin rotations (#1427).
pub const COORDINATOR_INTERFACE: &str = "admin_rotation_v1";

/// Default number of distinct admin approvals required for a coordinated
/// rotation (#1428). Governance may change it with `set_min_rotation_quorum`.
pub const DEFAULT_MIN_ROTATION_QUORUM: u32 = 2;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    /// The coordinator is not registered with the expected interface (#1427).
    InvalidCoordinator = 4,
    /// The rotation proposal has fewer approvals than the configured quorum (#1428).
    QuorumNotMet = 5,
    /// The requested quorum is zero or larger than the registered admin set (#1428).
    InvalidQuorum = 6,
}

// ---------------------------------------------------------------------------
// Cross-contract client for the interface registry (#1427)
// ---------------------------------------------------------------------------

/// Minimal client for the `interface_registry` contract.
///
/// `verify(address, expected_interface)` returns `true` only when `address` is
/// registered as implementing the given interface and is not quarantined.
#[contractclient(name = "InterfaceRegistryClient")]
pub trait InterfaceRegistry {
    fn verify(env: Env, address: Address, expected_interface: Symbol) -> bool;
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

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
    /// Address of the `interface_registry` used to validate coordinators (#1427).
    InterfaceRegistry,
    /// Admins that may approve coordinated rotations (#1428).
    Admins,
    /// Governance-configured M-of-N quorum (#1428).
    MinRotationQuorum,
    /// Whether `approver` approved the rotation to `new_admin` (#1428).
    RotationApprovals(Address, Address),
    /// Number of distinct approvals for a rotation to `new_admin` (#1428).
    RotationApprovalCount(Address),
}

#[contract]
pub struct AdminRotationCoordinator;

#[contractimpl]
impl AdminRotationCoordinator {
    /// Initialize the coordinator with its admin, a default quorum and an
    /// empty contract/admin registry.
    ///
    /// The interface registry must be configured separately with
    /// `set_interface_registry` before any contract can be registered.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Contracts, &Vec::<Address>::new(&env));

        // The initial admin is the first member of the rotation approval set.
        let mut admins = Vec::new(&env);
        admins.push_back(admin);
        env.storage().instance().set(&DataKey::Admins, &admins);
        env.storage()
            .instance()
            .set(&DataKey::MinRotationQuorum, &DEFAULT_MIN_ROTATION_QUORUM);
        Ok(())
    }

    // ── Interface registry configuration (#1427) ────────────────────────────

    /// Configure the `interface_registry` used to validate coordinators.
    /// Admin-only.
    pub fn set_interface_registry(env: Env, admin: Address, registry: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::InterfaceRegistry, &registry);
        Ok(())
    }

    /// Return the configured interface registry, if any.
    pub fn get_interface_registry(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::InterfaceRegistry)
    }

    // ── Rotation admin set and quorum (#1428) ───────────────────────────────

    /// Register an additional admin allowed to approve coordinated rotations.
    /// Admin-only.
    pub fn register_admin(env: Env, admin: Address, new_admin: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        let mut admins: Vec<Address> = env.storage().instance().get(&DataKey::Admins).ok_or(Error::NotInitialized)?;
        if !admins.iter().any(|existing| existing == new_admin) {
            admins.push_back(new_admin);
            env.storage().instance().set(&DataKey::Admins, &admins);
        }
        Ok(())
    }

    /// Admins currently permitted to approve a coordinated rotation.
    pub fn get_admins(env: Env) -> Vec<Address> {
        env.storage().instance().get(&DataKey::Admins).unwrap_or(Vec::new(&env))
    }

    /// Governance-controlled minimum number of approvals for a rotation.
    pub fn get_min_rotation_quorum(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MinRotationQuorum)
            .unwrap_or(DEFAULT_MIN_ROTATION_QUORUM)
    }

    /// Update the M-of-N quorum. Admin-only (governance).
    pub fn set_min_rotation_quorum(env: Env, admin: Address, quorum: u32) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        let admin_count = Self::get_admins(env.clone()).len();
        if quorum == 0 || quorum > admin_count {
            return Err(Error::InvalidQuorum);
        }
        env.storage().instance().set(&DataKey::MinRotationQuorum, &quorum);
        Ok(())
    }

    /// Record `approver`'s approval for rotating to `new_admin`.
    /// Each registered admin may approve a given rotation once.
    pub fn approve_rotation(env: Env, approver: Address, new_admin: Address) -> Result<(), Error> {
        approver.require_auth();
        let admins = Self::get_admins(env.clone());
        if !admins.iter().any(|candidate| candidate == approver) {
            return Err(Error::Unauthorized);
        }

        let approval_key = DataKey::RotationApprovals(new_admin.clone(), approver.clone());
        if env.storage().instance().has(&approval_key) {
            return Ok(()); // idempotent
        }

        env.storage().instance().set(&approval_key, &true);
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::RotationApprovalCount(new_admin.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::RotationApprovalCount(new_admin), &(count + 1));
        Ok(())
    }

    /// Number of distinct approvals recorded for a rotation to `new_admin`.
    pub fn get_rotation_approvals(env: Env, new_admin: Address) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::RotationApprovalCount(new_admin))
            .unwrap_or(0)
    }

    // ── Contract registration (#1427) ───────────────────────────────────────

    /// Register a coordinator after verifying it implements the expected
    /// interface in the configured `interface_registry`.
    pub fn register_contract(env: Env, admin: Address, contract: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;

        // Registration queries `interface_registry` to confirm the coordinator
        // implements the expected interface. An unregistered coordinator is
        // rejected rather than silently accepted.
        let registry: Address = env
            .storage()
            .instance()
            .get(&DataKey::InterfaceRegistry)
            .ok_or(Error::InvalidCoordinator)?;
        let client = InterfaceRegistryClient::new(&env, &registry);
        if !client.verify(&contract, &Symbol::new(&env, COORDINATOR_INTERFACE)) {
            return Err(Error::InvalidCoordinator);
        }

        let mut contracts: Vec<Address> = env.storage().instance().get(&DataKey::Contracts).unwrap_or(Vec::new(&env));
        if !contracts.iter().any(|existing| existing == contract) {
            contracts.push_back(contract);
            env.storage().instance().set(&DataKey::Contracts, &contracts);
        }
        Ok(())
    }

    // ── Coordinated rotation (#1428) ────────────────────────────────────────

    /// Propose `new_admin` on every registered contract once the quorum of
    /// distinct admin approvals has been reached.
    pub fn batch_propose_admin_change(env: Env, admin: Address, new_admin: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;

        let quorum = Self::get_min_rotation_quorum(env.clone());
        let approvals = Self::get_rotation_approvals(env.clone(), new_admin.clone());
        if approvals < quorum {
            return Err(Error::QuorumNotMet);
        }

        let contracts: Vec<Address> = env.storage().instance().get(&DataKey::Contracts).unwrap_or(Vec::new(&env));
        for contract in contracts.iter() {
            let _: () = env.invoke_contract(&contract, &Symbol::new(&env, "propose_admin_change"), (admin.clone(), new_admin.clone()).into_val(&env));
        }

        // Consume the approvals so the same set cannot authorize another rotation.
        Self::clear_approvals(&env, &new_admin);
        Ok(())
    }

    pub fn get_registered_contracts(env: Env) -> Vec<Address> {
        env.storage().instance().get(&DataKey::Contracts).unwrap_or(Vec::new(&env))
    }

    // ── Internal helpers ────────────────────────────────────────────────────

    fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin: Address = env.storage().instance().get(&DataKey::Admin).ok_or(Error::NotInitialized)?;
        if stored_admin != *admin {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn clear_approvals(env: &Env, new_admin: &Address) {
        env.storage()
            .instance()
            .remove(&DataKey::RotationApprovalCount(new_admin.clone()));
        for approver in Self::get_admins(env.clone()).iter() {
            env.storage()
                .instance()
                .remove(&DataKey::RotationApprovals(new_admin.clone(), approver));
        }
    }
}

// ---------------------------------------------------------------------------
// Tests (#1262, #1427, #1428)
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::Address as _;

    /// Minimal mock of the `interface_registry` contract: `allow` marks an
    /// address as implementing the coordinator interface.
    #[contract]
    struct MockInterfaceRegistry;

    #[contractimpl]
    impl MockInterfaceRegistry {
        pub fn allow(env: Env, address: Address) {
            env.storage().instance().set(&address, &true);
        }

        pub fn verify(env: Env, address: Address, _expected_interface: Symbol) -> bool {
            env.storage().instance().get(&address).unwrap_or(false)
        }
    }

    struct Fixture {
        env: Env,
        client: AdminRotationCoordinatorClient<'static>,
        registry: MockInterfaceRegistryClient<'static>,
        admin: Address,
        second_admin: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let second_admin = Address::generate(&env);
            let coordinator = env.register_contract(None, AdminRotationCoordinator);
            let client = AdminRotationCoordinatorClient::new(&env, &coordinator);
            client.initialize(&admin);

            let registry_id = env.register_contract(None, MockInterfaceRegistry);
            let registry = MockInterfaceRegistryClient::new(&env, &registry_id);
            client.set_interface_registry(&admin, &registry_id);
            client.register_admin(&admin, &second_admin);

            Fixture { env, client, registry, admin, second_admin }
        }

        fn register_valid(&self, contract: &Address) {
            self.registry.allow(contract);
            self.client.register_contract(&self.admin, contract);
        }

        fn approve_quorum(&self, new_admin: &Address) {
            self.client.approve_rotation(&self.admin, new_admin);
            self.client.approve_rotation(&self.second_admin, new_admin);
        }
    }

    #[test]
    fn test_batch_propose_admin_change_integration() {
        let f = Fixture::setup();

        let contract1 = f.env.register_contract_wasm(None, b"contract1");
        let contract2 = f.env.register_contract_wasm(None, b"contract2");
        let contract3 = f.env.register_contract_wasm(None, b"contract3");
        f.register_valid(&contract1);
        f.register_valid(&contract2);
        f.register_valid(&contract3);
        assert_eq!(f.client.get_registered_contracts().len(), 3);

        let new_admin = Address::generate(&f.env);
        f.approve_quorum(&new_admin);
        assert_eq!(f.client.get_rotation_approvals(&new_admin), 2);
        f.client.batch_propose_admin_change(&f.admin, &new_admin);

        // Approvals are consumed after a successful rotation.
        assert_eq!(f.client.get_rotation_approvals(&new_admin), 0);
    }

    #[test]
    fn test_register_contract_prevents_duplicates() {
        let f = Fixture::setup();
        let contract = f.env.register_contract_wasm(None, b"test");
        f.register_valid(&contract);
        f.client.register_contract(&f.admin, &contract);
        assert_eq!(f.client.get_registered_contracts().len(), 1);
    }

    #[test]
    fn test_get_registered_contracts_returns_all() {
        let f = Fixture::setup();
        for name in [b"c1".as_slice(), b"c2".as_slice(), b"c3".as_slice()] {
            let contract = f.env.register_contract_wasm(None, name);
            f.register_valid(&contract);
        }
        assert_eq!(f.client.get_registered_contracts().len(), 3);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #1)")]
    fn test_initialize_already_initialized_error() {
        let f = Fixture::setup();
        f.client.initialize(&f.admin);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #2)")]
    fn test_register_contract_not_initialized_error() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract = env.register_contract_wasm(None, b"test");
        let coordinator = env.register_contract(None, AdminRotationCoordinator);
        AdminRotationCoordinatorClient::new(&env, &coordinator)
            .register_contract(&admin, &contract);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #3)")]
    fn test_register_contract_unauthorized_error() {
        let f = Fixture::setup();
        let wrong_admin = Address::generate(&f.env);
        let contract = f.env.register_contract_wasm(None, b"test");
        f.registry.allow(&contract);
        f.client.register_contract(&wrong_admin, &contract);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #2)")]
    fn test_batch_propose_admin_change_not_initialized_error() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let new_admin = Address::generate(&env);
        let coordinator = env.register_contract(None, AdminRotationCoordinator);
        AdminRotationCoordinatorClient::new(&env, &coordinator)
            .batch_propose_admin_change(&admin, &new_admin);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #3)")]
    fn test_batch_propose_admin_change_unauthorized_error() {
        let f = Fixture::setup();
        let wrong_admin = Address::generate(&f.env);
        let new_admin = Address::generate(&f.env);
        f.client.batch_propose_admin_change(&wrong_admin, &new_admin);
    }

    // ── #1427: coordinator validation ───────────────────────────────────────

    #[test]
    fn test_register_valid_coordinator_succeeds() {
        let f = Fixture::setup();
        let contract = f.env.register_contract_wasm(None, b"valid");
        f.register_valid(&contract);
        let registered = f.client.get_registered_contracts();
        assert_eq!(registered.len(), 1);
        assert_eq!(registered.get(0).unwrap(), contract);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #4)")]
    fn test_register_unregistered_coordinator_fails() {
        let f = Fixture::setup();
        let contract = f.env.register_contract_wasm(None, b"invalid");
        // Deliberately do not allow the contract in the mock registry.
        f.client.register_contract(&f.admin, &contract);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #4)")]
    fn test_register_without_configured_registry_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract = env.register_contract_wasm(None, b"test");
        let coordinator = env.register_contract(None, AdminRotationCoordinator);
        let client = AdminRotationCoordinatorClient::new(&env, &coordinator);
        client.initialize(&admin);
        client.register_contract(&admin, &contract);
    }

    // ── #1428: rotation quorum ─────────────────────────────────────────────

    #[test]
    fn test_quorum_is_configurable_and_defaults_to_two() {
        let f = Fixture::setup();
        assert_eq!(f.client.get_min_rotation_quorum(), DEFAULT_MIN_ROTATION_QUORUM);
        f.client.set_min_rotation_quorum(&f.admin, &1);
        assert_eq!(f.client.get_min_rotation_quorum(), 1);
        f.client.set_min_rotation_quorum(&f.admin, &2);
        assert_eq!(f.client.get_min_rotation_quorum(), 2);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #6)")]
    fn test_quorum_cannot_exceed_admin_count() {
        let f = Fixture::setup();
        f.client.set_min_rotation_quorum(&f.admin, &3);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn test_under_quorum_rotation_is_rejected() {
        let f = Fixture::setup();
        let new_admin = Address::generate(&f.env);
        // Only one of the two required admins approves.
        f.client.approve_rotation(&f.admin, &new_admin);
        assert_eq!(f.client.get_rotation_approvals(&new_admin), 1);
        f.client.batch_propose_admin_change(&f.admin, &new_admin);
    }

    #[test]
    #[should_panic(expected = "Error(Contract, #5)")]
    fn test_rotation_with_no_approvals_is_rejected() {
        let f = Fixture::setup();
        let new_admin = Address::generate(&f.env);
        f.client.batch_propose_admin_change(&f.admin, &new_admin);
    }

    #[test]
    fn test_non_admin_cannot_approve_rotation() {
        let f = Fixture::setup();
        let stranger = Address::generate(&f.env);
        let new_admin = Address::generate(&f.env);
        let result = f.client.try_approve_rotation(&stranger, &new_admin);
        assert!(result.is_err());
        assert_eq!(f.client.get_rotation_approvals(&new_admin), 0);
    }
}

#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, Symbol, symbol_short,
};
use mentorminds_shared::ROLE_SUPER_ADMIN;
use soroban_sdk::Vec;

// Helper function to call RBAC contract's has_role via cross-contract call
fn check_rbac_role(env: &Env, rbac_address: &Address, address: &Address, role: &Symbol) -> bool {
    let fn_name = soroban_sdk::symbol_short!("has_role");
    let args = Vec::new(env);
    args.push_back(address.clone());
    args.push_back(role.clone());
    
    let result: bool = env.invoke_contract(rbac_address, &fn_name, &args).unwrap();
    result
}

/// Contract upgrade data structure
#[contracttype]
#[derive(Clone, Debug)]
pub struct ContractUpgrade {
    pub contract_address: Address,
    pub new_implementation: Address,
    pub proposed_by: Address,
    pub proposed_at: u64,
    pub approved: bool,
    pub executed: bool,
}

/// Storage keys
const RBAC_ADDRESS: Symbol = symbol_short!("RBAC_ADDR");
const UPGRADES: Symbol = symbol_short!("UPG");
const UPGRADE_COUNT: Symbol = symbol_short!("UPG_CNT");
const INITIALIZED: Symbol = symbol_short!("INIT");

/// TTL constants
const STORAGE_TTL_THRESHOLD: u32 = 500_000;
const STORAGE_TTL_BUMP: u32 = 1_000_000;

#[contract]
pub struct UpgradeRegistryContract;

#[contractimpl]
impl UpgradeRegistryContract {
    /// Initialize the upgrade registry contract with RBAC contract address
    pub fn initialize(env: Env, rbac_address: Address) {
        // Ensure not already initialized
        if env.storage().persistent().has(&INITIALIZED) {
            panic!("Upgrade Registry already initialized");
        }

        // Store RBAC contract address
        env.storage().persistent().set(&RBAC_ADDRESS, &rbac_address);
        env.storage().persistent().extend_ttl(&RBAC_ADDRESS, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Initialize upgrade count
        env.storage().persistent().set(&UPGRADE_COUNT, &0u64);
        env.storage().persistent().extend_ttl(&UPGRADE_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Mark as initialized
        env.storage().persistent().set(&INITIALIZED, &true);
        env.storage().persistent().extend_ttl(&INITIALIZED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
    }

    /// Propose a contract upgrade (requires ROLE_SUPER_ADMIN)
    pub fn propose_upgrade(
        env: Env,
        caller: Address,
        contract_address: Address,
        new_implementation: Address,
    ) -> u64 {
        // Verify caller has super admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        // Get and increment upgrade count
        let mut count: u64 = env.storage().persistent().get(&UPGRADE_COUNT).unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&UPGRADE_COUNT, &count);
        env.storage().persistent().extend_ttl(&UPGRADE_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Create upgrade proposal
        let upgrade = ContractUpgrade {
            contract_address: contract_address.clone(),
            new_implementation: new_implementation.clone(),
            proposed_by: caller.clone(),
            proposed_at: env.ledger().timestamp(),
            approved: false,
            executed: false,
        };

        // Store upgrade
        let key = (UPGRADES, count);
        env.storage().persistent().set(&key, &upgrade);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("UPG_PROP"), count),
            (contract_address, new_implementation, caller),
        );

        count
    }

    /// Approve a contract upgrade (requires ROLE_SUPER_ADMIN)
    pub fn approve_upgrade(env: Env, caller: Address, upgrade_id: u64) {
        // Verify caller has super admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        // Get upgrade
        let key = (UPGRADES, upgrade_id);
        let mut upgrade: ContractUpgrade = env.storage().persistent().get(&key)
            .expect("Upgrade not found");

        // Check if already approved
        if upgrade.approved {
            panic!("Upgrade already approved");
        }

        // Check if already executed
        if upgrade.executed {
            panic!("Upgrade already executed");
        }

        // Approve upgrade
        upgrade.approved = true;
        env.storage().persistent().set(&key, &upgrade);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("UPG_APPR"), upgrade_id),
            (caller, upgrade.contract_address),
        );
    }

    /// Execute a contract upgrade (requires ROLE_SUPER_ADMIN)
    pub fn execute_upgrade(env: Env, caller: Address, upgrade_id: u64) {
        // Verify caller has super admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        // Get upgrade
        let key = (UPGRADES, upgrade_id);
        let mut upgrade: ContractUpgrade = env.storage().persistent().get(&key)
            .expect("Upgrade not found");

        // Check if approved
        if !upgrade.approved {
            panic!("Upgrade not approved");
        }

        // Check if already executed
        if upgrade.executed {
            panic!("Upgrade already executed");
        }

        // Mark as executed
        upgrade.executed = true;
        env.storage().persistent().set(&key, &upgrade);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("UPG_EXEC"), upgrade_id),
            (caller, upgrade.contract_address, upgrade.new_implementation),
        );
    }

    /// Cancel a contract upgrade (requires ROLE_SUPER_ADMIN)
    pub fn cancel_upgrade(env: Env, caller: Address, upgrade_id: u64) {
        // Verify caller has super admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        // Get upgrade
        let key = (UPGRADES, upgrade_id);
        let upgrade: ContractUpgrade = env.storage().persistent().get(&key)
            .expect("Upgrade not found");

        // Check if already executed
        if upgrade.executed {
            panic!("Cannot cancel executed upgrade");
        }

        // Remove upgrade
        env.storage().persistent().remove(&key);

        // Emit event
        env.events().publish(
            (symbol_short!("UPG_CAN"), upgrade_id),
            (caller, upgrade.contract_address),
        );
    }

    /// Get upgrade details
    pub fn get_upgrade(env: Env, upgrade_id: u64) -> ContractUpgrade {
        let key = (UPGRADES, upgrade_id);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&key).expect("Upgrade not found")
    }

    /// Get total upgrade count
    pub fn get_upgrade_count(env: Env) -> u64 {
        env.storage().persistent().extend_ttl(&UPGRADE_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&UPGRADE_COUNT).unwrap_or(0)
    }

    /// Get RBAC contract address
    pub fn get_rbac_address(env: Env) -> Address {
        env.storage().persistent().extend_ttl(&RBAC_ADDRESS, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&RBAC_ADDRESS).expect("RBAC address not set")
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    fn setup_env() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        let rbac_contract_id = env.register_contract(None, mentorminds_rbac::RbacContract);
        let upgrade_registry_contract_id = env.register_contract(None, UpgradeRegistryContract);
        
        let super_admin = Address::generate(&env);
        let contract_address = Address::generate(&env);
        let new_implementation = Address::generate(&env);
        
        (env, rbac_contract_id, upgrade_registry_contract_id, super_admin, contract_address, new_implementation)
    }

    #[test]
    fn test_initialize() {
        let (env, rbac_id, upgrade_registry_id, _, _, _) = setup_env();
        let upgrade_registry_client = UpgradeRegistryContractClient::new(&env, &upgrade_registry_id);
        
        upgrade_registry_client.initialize(&rbac_id);
        
        assert_eq!(upgrade_registry_client.get_rbac_address(), rbac_id);
    }

    #[test]
    fn test_propose_upgrade_with_role() {
        let (env, rbac_id, upgrade_registry_id, super_admin, contract_addr, new_impl) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let upgrade_registry_client = UpgradeRegistryContractClient::new(&env, &upgrade_registry_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Upgrade Registry
        upgrade_registry_client.initialize(&rbac_id);
        
        // Propose upgrade
        env.mock_all_auths();
        let upgrade_id = upgrade_registry_client.propose_upgrade(
            &super_admin,
            &contract_addr,
            &new_impl,
        );
        
        assert_eq!(upgrade_id, 1);
        
        let upgrade = upgrade_registry_client.get_upgrade(&upgrade_id);
        assert_eq!(upgrade.contract_address, contract_addr);
        assert_eq!(upgrade.new_implementation, new_impl);
        assert!(!upgrade.approved);
        assert!(!upgrade.executed);
    }

    #[test]
    fn test_propose_upgrade_without_role() {
        let (env, rbac_id, upgrade_registry_id, super_admin, contract_addr, new_impl) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let upgrade_registry_client = UpgradeRegistryContractClient::new(&env, &upgrade_registry_id);
        
        let unauthorized = Address::generate(&env);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Upgrade Registry
        upgrade_registry_client.initialize(&rbac_id);
        
        // Try to propose upgrade without role - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            upgrade_registry_client.propose_upgrade(
                &unauthorized,
                &contract_addr,
                &new_impl,
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_approve_upgrade() {
        let (env, rbac_id, upgrade_registry_id, super_admin, contract_addr, new_impl) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let upgrade_registry_client = UpgradeRegistryContractClient::new(&env, &upgrade_registry_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Upgrade Registry
        upgrade_registry_client.initialize(&rbac_id);
        
        // Propose upgrade
        env.mock_all_auths();
        let upgrade_id = upgrade_registry_client.propose_upgrade(
            &super_admin,
            &contract_addr,
            &new_impl,
        );
        
        // Approve upgrade
        env.mock_all_auths();
        upgrade_registry_client.approve_upgrade(&super_admin, &upgrade_id);
        
        let upgrade = upgrade_registry_client.get_upgrade(&upgrade_id);
        assert!(upgrade.approved);
    }

    #[test]
    fn test_execute_upgrade() {
        let (env, rbac_id, upgrade_registry_id, super_admin, contract_addr, new_impl) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let upgrade_registry_client = UpgradeRegistryContractClient::new(&env, &upgrade_registry_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Upgrade Registry
        upgrade_registry_client.initialize(&rbac_id);
        
        // Propose upgrade
        env.mock_all_auths();
        let upgrade_id = upgrade_registry_client.propose_upgrade(
            &super_admin,
            &contract_addr,
            &new_impl,
        );
        
        // Approve upgrade
        env.mock_all_auths();
        upgrade_registry_client.approve_upgrade(&super_admin, &upgrade_id);
        
        // Execute upgrade
        env.mock_all_auths();
        upgrade_registry_client.execute_upgrade(&super_admin, &upgrade_id);
        
        let upgrade = upgrade_registry_client.get_upgrade(&upgrade_id);
        assert!(upgrade.executed);
    }

    #[test]
    fn test_execute_without_approval() {
        let (env, rbac_id, upgrade_registry_id, super_admin, contract_addr, new_impl) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let upgrade_registry_client = UpgradeRegistryContractClient::new(&env, &upgrade_registry_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Upgrade Registry
        upgrade_registry_client.initialize(&rbac_id);
        
        // Propose upgrade
        env.mock_all_auths();
        let upgrade_id = upgrade_registry_client.propose_upgrade(
            &super_admin,
            &contract_addr,
            &new_impl,
        );
        
        // Try to execute without approval - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            upgrade_registry_client.execute_upgrade(&super_admin, &upgrade_id);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_cancel_upgrade() {
        let (env, rbac_id, upgrade_registry_id, super_admin, contract_addr, new_impl) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let upgrade_registry_client = UpgradeRegistryContractClient::new(&env, &upgrade_registry_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Upgrade Registry
        upgrade_registry_client.initialize(&rbac_id);
        
        // Propose upgrade
        env.mock_all_auths();
        let upgrade_id = upgrade_registry_client.propose_upgrade(
            &super_admin,
            &contract_addr,
            &new_impl,
        );
        
        // Cancel upgrade
        env.mock_all_auths();
        upgrade_registry_client.cancel_upgrade(&super_admin, &upgrade_id);
        
        // Try to get upgrade - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            upgrade_registry_client.get_upgrade(&upgrade_id);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_cancel_without_role() {
        let (env, rbac_id, upgrade_registry_id, super_admin, contract_addr, new_impl) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let upgrade_registry_client = UpgradeRegistryContractClient::new(&env, &upgrade_registry_id);
        
        let unauthorized = Address::generate(&env);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Upgrade Registry
        upgrade_registry_client.initialize(&rbac_id);
        
        // Propose upgrade
        env.mock_all_auths();
        let upgrade_id = upgrade_registry_client.propose_upgrade(
            &super_admin,
            &contract_addr,
            &new_impl,
        );
        
        // Try to cancel without role - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            upgrade_registry_client.cancel_upgrade(&unauthorized, &upgrade_id);
        }));
        assert!(result.is_err());
    }
}

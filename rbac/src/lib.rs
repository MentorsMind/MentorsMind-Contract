#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, Symbol, Vec,
};
use mentorminds_shared::{
    ROLE_SUPER_ADMIN, ROLE_TREASURY_ADMIN, ROLE_STAKING_ADMIN, 
    ROLE_GOVERNANCE_ADMIN
};

/// Role data structure for storage
#[contracttype]
#[derive(Clone, Debug)]
pub struct RoleData {
    pub role: Symbol,
    pub account: Address,
}

/// Storage keys
const ROLES: Symbol = soroban_sdk::symbol_short!("ROLES");
const ADMIN_COUNT: Symbol = soroban_sdk::symbol_short!("ADM_CNT");
const INITIALIZED: Symbol = soroban_sdk::symbol_short!("INIT");

/// TTL constants
const STORAGE_TTL_THRESHOLD: u32 = 500_000;
const STORAGE_TTL_BUMP: u32 = 1_000_000;

#[contract]
pub struct RbacContract;

#[contractimpl]
impl RbacContract {
    /// Initialize the RBAC contract with a super admin
    pub fn initialize(env: Env, super_admin: Address) {
        // Ensure not already initialized
        if env.storage().persistent().has(&INITIALIZED) {
            panic!("RBAC already initialized");
        }

        // Grant super admin role to the initial admin
        Self::internal_grant_role(&env, &super_admin, &ROLE_SUPER_ADMIN);

        // Mark as initialized
        env.storage().persistent().set(&INITIALIZED, &true);
        env.storage().persistent().extend_ttl(&INITIALIZED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
    }

    /// Check if an address has a specific role
    pub fn has_role(env: Env, address: Address, role: Symbol) -> bool {
        let key = (ROLES, role.clone(), address.clone());
        
        // Bump TTL before reading
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        
        env.storage().persistent()
            .get::<_, bool>(&key)
            .unwrap_or(false)
    }

    /// Grant a role to an address (requires ROLE_SUPER_ADMIN)
    pub fn grant_role(env: Env, caller: Address, address: Address, role: Symbol) {
        // Verify caller has super admin role
        if !Self::has_role(env.clone(), caller.clone(), ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        // Grant the role
        Self::internal_grant_role(&env, &address, &role);

        // Emit event
        env.events().publish(
            (soroban_sdk::symbol_short!("ROLE_GRNT"), role.clone()),
            (address.clone(), caller),
        );
    }

    /// Revoke a role from an address (requires ROLE_SUPER_ADMIN)
    pub fn revoke_role(env: Env, caller: Address, address: Address, role: Symbol) {
        // Verify caller has super admin role
        if !Self::has_role(env.clone(), caller.clone(), ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        // Check if the address has the role
        let key = (ROLES, role.clone(), address.clone());
        if !env.storage().persistent().get::<_, bool>(&key).unwrap_or(false) {
            panic!("Address does not have this role");
        }

        // Revoke the role
        env.storage().persistent().remove(&key);

        // Decrement admin count if it was an admin role
        if role == ROLE_SUPER_ADMIN || role == ROLE_TREASURY_ADMIN || 
           role == ROLE_STAKING_ADMIN || role == ROLE_GOVERNANCE_ADMIN {
            let mut count: u32 = env.storage().persistent().get(&ADMIN_COUNT).unwrap_or(0);
            if count > 0 {
                count -= 1;
                env.storage().persistent().set(&ADMIN_COUNT, &count);
                env.storage().persistent().extend_ttl(&ADMIN_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
            }
        }

        // Emit event
        env.events().publish(
            (soroban_sdk::symbol_short!("ROLE_RVK"), role.clone()),
            (address.clone(), caller),
        );
    }

    /// Revoke all roles from an address (emergency key rotation)
    /// This is atomic - removes all roles in a single transaction
    pub fn revoke_all_roles(env: Env, caller: Address, address: Address) {
        // Verify caller has super admin role
        if !Self::has_role(env.clone(), caller.clone(), ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        let roles_to_check = [
            ROLE_SUPER_ADMIN, ROLE_TREASURY_ADMIN, ROLE_STAKING_ADMIN,
            ROLE_GOVERNANCE_ADMIN
        ];

        let mut revoked_count = 0u32;

        // Revoke each role if the address has it
        for role in roles_to_check.iter() {
            let key = (ROLES, role.clone(), address.clone());
            if env.storage().persistent().get::<_, bool>(&key).unwrap_or(false) {
                env.storage().persistent().remove(&key);
                revoked_count += 1;

                // Decrement admin count if it was an admin role
                if *role == ROLE_SUPER_ADMIN || *role == ROLE_TREASURY_ADMIN || 
                   *role == ROLE_STAKING_ADMIN || *role == ROLE_GOVERNANCE_ADMIN {
                    let mut count: u32 = env.storage().persistent().get(&ADMIN_COUNT).unwrap_or(0);
                    if count > 0 {
                        count -= 1;
                        env.storage().persistent().set(&ADMIN_COUNT, &count);
                        env.storage().persistent().extend_ttl(&ADMIN_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
                    }
                }

                // Emit event for each revoked role
                env.events().publish(
                    (soroban_sdk::symbol_short!("ROLE_RVK"), role.clone()),
                    (address.clone(), caller.clone()),
                );
            }
        }

        // Emit emergency revocation event
        env.events().publish(
            (soroban_sdk::symbol_short!("EMRG_RVK"),),
            (address, caller, revoked_count),
        );
    }

    /// Get all roles for an address (for debugging/auditing)
    pub fn get_roles(env: Env, address: Address) -> Vec<Symbol> {
        let roles_to_check = [
            ROLE_SUPER_ADMIN, ROLE_TREASURY_ADMIN, ROLE_STAKING_ADMIN,
            ROLE_GOVERNANCE_ADMIN
        ];

        let mut result = Vec::new(&env);

        for role in roles_to_check.iter() {
            let key = (ROLES, role.clone(), address.clone());
            if env.storage().persistent().get::<_, bool>(&key).unwrap_or(false) {
                result.push_back(role.clone());
            }
        }

        result
    }

    /// Get the total number of admins (accounts with any admin role)
    pub fn get_admin_count(env: Env) -> u32 {
        env.storage().persistent().extend_ttl(&ADMIN_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&ADMIN_COUNT).unwrap_or(0)
    }

    /// Internal function to grant a role without authorization check
    fn internal_grant_role(env: &Env, address: &Address, role: &Symbol) {
        let key = (ROLES, role.clone(), address.clone());
        
        // Check if already has role
        if env.storage().persistent().get::<_, bool>(&key).unwrap_or(false) {
            panic!("Address already has this role");
        }

        // Grant the role
        env.storage().persistent().set(&key, &true);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Increment admin count if it's an admin role
        if *role == ROLE_SUPER_ADMIN || *role == ROLE_TREASURY_ADMIN || 
           *role == ROLE_STAKING_ADMIN || *role == ROLE_GOVERNANCE_ADMIN {
            let mut count: u32 = env.storage().persistent().get(&ADMIN_COUNT).unwrap_or(0);
            count += 1;
            env.storage().persistent().set(&ADMIN_COUNT, &count);
            env.storage().persistent().extend_ttl(&ADMIN_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        }
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    fn setup_env() -> (Env, Address, Address) {
        let env = Env::default();
        let contract_id = env.register_contract(None, RbacContract);
        let super_admin = Address::generate(&env);
        let other_address = Address::generate(&env);
        
        (env, super_admin, other_address)
    }

    #[test]
    fn test_initialize() {
        let (env, super_admin, _) = setup_env();
        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);

        // Initialize
        client.initialize(&super_admin);

        // Verify super admin has role
        assert!(client.has_role(&super_admin, &ROLE_SUPER_ADMIN));
    }

    #[test]
    fn test_prevent_reinit() {
        let (env, super_admin, other) = setup_env();
        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);

        // Initialize
        client.initialize(&super_admin);

        // Try to re-initialize - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.initialize(&other);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_grant_role() {
        let (env, super_admin, other) = setup_env();
        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);

        // Initialize
        client.initialize(&super_admin);

        // Grant treasury admin role
        env.mock_all_auths();
        client.grant_role(&super_admin, &other, &ROLE_TREASURY_ADMIN);

        // Verify role granted
        assert!(client.has_role(&other, &ROLE_TREASURY_ADMIN));
    }

    #[test]
    fn test_grant_role_unauthorized() {
        let (env, super_admin, other) = setup_env();
        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);

        // Initialize
        client.initialize(&super_admin);

        // Try to grant role without super admin - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            client.grant_role(&other, &super_admin, &ROLE_TREASURY_ADMIN);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_revoke_role() {
        let (env, super_admin, other) = setup_env();
        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);

        // Initialize
        client.initialize(&super_admin);

        // Grant treasury admin role
        env.mock_all_auths();
        client.grant_role(&super_admin, &other, &ROLE_TREASURY_ADMIN);

        // Verify role granted
        assert!(client.has_role(&other, &ROLE_TREASURY_ADMIN));

        // Revoke role
        env.mock_all_auths();
        client.revoke_role(&super_admin, &other, &ROLE_TREASURY_ADMIN);

        // Verify role revoked
        assert!(!client.has_role(&other, &ROLE_TREASURY_ADMIN));
    }

    #[test]
    fn test_revoke_all_roles() {
        let (env, super_admin, other) = setup_env();
        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);

        // Initialize
        client.initialize(&super_admin);

        // Grant multiple roles
        env.mock_all_auths();
        client.grant_role(&super_admin, &other, &ROLE_TREASURY_ADMIN);
        client.grant_role(&super_admin, &other, &ROLE_STAKING_ADMIN);
        client.grant_role(&super_admin, &other, &ROLE_ORACLE_FEEDER);

        // Verify roles granted
        assert!(client.has_role(&other, &ROLE_TREASURY_ADMIN));
        assert!(client.has_role(&other, &ROLE_STAKING_ADMIN));
        assert!(client.has_role(&other, &ROLE_ORACLE_FEEDER));

        // Revoke all roles
        env.mock_all_auths();
        client.revoke_all_roles(&super_admin, &other);

        // Verify all roles revoked
        assert!(!client.has_role(&other, &ROLE_TREASURY_ADMIN));
        assert!(!client.has_role(&other, &ROLE_STAKING_ADMIN));
        assert!(!client.has_role(&other, &ROLE_ORACLE_FEEDER));
    }

    #[test]
    fn test_get_roles() {
        let (env, super_admin, other) = setup_env();
        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);

        // Initialize
        client.initialize(&super_admin);

        // Grant multiple roles
        env.mock_all_auths();
        client.grant_role(&super_admin, &other, &ROLE_TREASURY_ADMIN);
        client.grant_role(&super_admin, &other, &ROLE_STAKING_ADMIN);

        // Get roles
        let roles = client.get_roles(&other);
        assert_eq!(roles.len(), 2);
    }

    #[test]
    fn test_admin_count() {
        let (env, super_admin, other) = setup_env();
        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);

        // Initialize
        client.initialize(&super_admin);

        // Initial count should be 1 (super admin)
        assert_eq!(client.get_admin_count(), 1);

        // Grant treasury admin role
        env.mock_all_auths();
        client.grant_role(&super_admin, &other, &ROLE_TREASURY_ADMIN);

        // Count should be 2
        assert_eq!(client.get_admin_count(), 2);

        // Revoke role
        env.mock_all_auths();
        client.revoke_role(&super_admin, &other, &ROLE_TREASURY_ADMIN);

        // Count should be back to 1
        assert_eq!(client.get_admin_count(), 1);
    }
}

// Integration tests
#[cfg(test)]
mod integration_test {
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token, Address, Env, Symbol, symbol_short,
    };
    use mentorminds_shared::{ROLE_SUPER_ADMIN, ROLE_TREASURY_ADMIN, ROLE_STAKING_ADMIN, ROLE_GOVERNANCE_ADMIN};

    /// Integration test: grant treasury admin role, perform allocation, revoke role, verify allocation rejected
    #[test]
    fn test_rbac_integration_treasury_workflow() {
        let env = Env::default();
        
        // Register contracts
        let rbac_contract_id = env.register_contract(None, mentorminds_rbac::RbacContract);
        let treasury_contract_id = env.register_contract(None, mentorminds_treasury::TreasuryContract);
        
        // Create addresses
        let super_admin = Address::generate(&env);
        let treasury_admin = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = Address::generate(&env);
        
        // Create clients
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_contract_id);
        let treasury_client = mentorminds_treasury::TreasuryContractClient::new(&env, &treasury_contract_id);
        
        // Step 1: Initialize RBAC with super admin
        rbac_client.initialize(&super_admin);
        assert!(rbac_client.has_role(&super_admin, &ROLE_SUPER_ADMIN));
        
        // Step 2: Initialize Treasury with RBAC address
        treasury_client.initialize(&rbac_contract_id);
        
        // Step 3: Grant treasury admin role to treasury_admin
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &treasury_admin, &ROLE_TREASURY_ADMIN);
        assert!(rbac_client.has_role(&treasury_admin, &ROLE_TREASURY_ADMIN));
        
        // Step 4: Perform allocation (should succeed)
        env.mock_all_auths();
        let alloc_id = treasury_client.allocate(
            &treasury_admin,
            &recipient,
            &1000,
            &token_address,
            &symbol_short!("TEST_ALLOC"),
        );
        assert_eq!(alloc_id, 1);
        
        let allocation = treasury_client.get_allocation(&alloc_id);
        assert_eq!(allocation.recipient, recipient);
        assert_eq!(allocation.amount, 1000);
        assert!(!allocation.executed);
        
        // Step 5: Revoke treasury admin role
        env.mock_all_auths();
        rbac_client.revoke_role(&super_admin, &treasury_admin, &ROLE_TREASURY_ADMIN);
        assert!(!rbac_client.has_role(&treasury_admin, &ROLE_TREASURY_ADMIN));
        
        // Step 6: Verify allocation execution is rejected (should panic)
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            treasury_client.execute_allocation(&treasury_admin, &alloc_id);
        }));
        assert!(result.is_err(), "Allocation execution should fail after role revocation");
        
        // Step 7: Verify new allocation is also rejected
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            treasury_client.allocate(
                &treasury_admin,
                &recipient,
                &500,
                &token_address,
                &symbol_short!("TEST_ALLOC2"),
            );
        }));
        assert!(result.is_err(), "New allocation should fail after role revocation");
    }

    /// Integration test: emergency revocation across multiple contracts
    #[test]
    fn test_emergency_revocation_across_contracts() {
        let env = Env::default();
        
        // Register contracts
        let rbac_contract_id = env.register_contract(None, mentorminds_rbac::RbacContract);
        let treasury_contract_id = env.register_contract(None, mentorminds_treasury::TreasuryContract);
        let staking_contract_id = env.register_contract(None, mentorminds_staking::StakingContract);
        let governance_contract_id = env.register_contract(None, mentorminds_governance::GovernanceContract);
        
        // Create addresses
        let super_admin = Address::generate(&env);
        let compromised_key = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = Address::generate(&env);
        
        // Create clients
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_contract_id);
        let treasury_client = mentorminds_treasury::TreasuryContractClient::new(&env, &treasury_contract_id);
        let staking_client = mentorminds_staking::StakingContractClient::new(&env, &staking_contract_id);
        let governance_client = mentorminds_governance::GovernanceContractClient::new(&env, &governance_contract_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize all contracts
        treasury_client.initialize(&rbac_contract_id);
        staking_client.initialize(&rbac_contract_id);
        governance_client.initialize(&rbac_contract_id);
        
        // Grant multiple roles to the compromised key
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &compromised_key, &ROLE_TREASURY_ADMIN);
        rbac_client.grant_role(&super_admin, &compromised_key, &ROLE_STAKING_ADMIN);
        rbac_client.grant_role(&super_admin, &compromised_key, &ROLE_GOVERNANCE_ADMIN);
        
        // Verify roles are granted
        assert!(rbac_client.has_role(&compromised_key, &ROLE_TREASURY_ADMIN));
        assert!(rbac_client.has_role(&compromised_key, &ROLE_STAKING_ADMIN));
        assert!(rbac_client.has_role(&compromised_key, &ROLE_GOVERNANCE_ADMIN));
        
        // Verify compromised key can perform actions
        env.mock_all_auths();
        let alloc_id = treasury_client.allocate(
            &compromised_key,
            &recipient,
            &1000,
            &token_address,
            &symbol_short!("TEST"),
        );
        assert_eq!(alloc_id, 1);
        
        // Emergency: revoke all roles from compromised key in one transaction
        env.mock_all_auths();
        rbac_client.revoke_all_roles(&super_admin, &compromised_key);
        
        // Verify all roles are revoked
        assert!(!rbac_client.has_role(&compromised_key, &ROLE_TREASURY_ADMIN));
        assert!(!rbac_client.has_role(&compromised_key, &ROLE_STAKING_ADMIN));
        assert!(!rbac_client.has_role(&compromised_key, &ROLE_GOVERNANCE_ADMIN));
        
        // Verify compromised key cannot perform actions in any contract
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            treasury_client.execute_allocation(&compromised_key, &alloc_id);
        }));
        assert!(result.is_err(), "Treasury action should fail after emergency revocation");
        
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            staking_client.force_unstake(&compromised_key, &1);
        }));
        assert!(result.is_err(), "Staking action should fail after emergency revocation");
        
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            governance_client.create_proposal(
                &compromised_key,
                &symbol_short!("TEST"),
                &symbol_short!("DESC"),
                None,
            );
        }));
        assert!(result.is_err(), "Governance action should fail after emergency revocation");
    }

    /// Integration test: grant_role requires ROLE_SUPER_ADMIN auth
    #[test]
    fn test_grant_role_requires_super_admin() {
        let env = Env::default();
        
        // Register contracts
        let rbac_contract_id = env.register_contract(None, mentorminds_rbac::RbacContract);
        
        // Create addresses
        let super_admin = Address::generate(&env);
        let regular_user = Address::generate(&env);
        let target = Address::generate(&env);
        
        // Create client
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_contract_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Try to grant role without SUPER_ADMIN role (should panic)
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            rbac_client.grant_role(&regular_user, &target, &ROLE_TREASURY_ADMIN);
        }));
        assert!(result.is_err(), "Grant role should require SUPER_ADMIN");
    }

    /// Integration test: all migrated contracts correctly reject callers lacking required role
    #[test]
    fn test_all_contracts_reject_unauthorized_callers() {
        let env = Env::default();
        
        // Register contracts
        let rbac_contract_id = env.register_contract(None, mentorminds_rbac::RbacContract);
        let treasury_contract_id = env.register_contract(None, mentorminds_treasury::TreasuryContract);
        let staking_contract_id = env.register_contract(None, mentorminds_staking::StakingContract);
        let governance_contract_id = env.register_contract(None, mentorminds_governance::GovernanceContract);
        let timelock_contract_id = env.register_contract(None, mentorminds_timelock::TimelockContract);
        let upgrade_registry_contract_id = env.register_contract(None, mentorminds_upgrade_registry::UpgradeRegistryContract);
        
        // Create addresses
        let super_admin = Address::generate(&env);
        let unauthorized = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = Address::generate(&env);
        
        // Create clients
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_contract_id);
        let treasury_client = mentorminds_treasury::TreasuryContractClient::new(&env, &treasury_contract_id);
        let staking_client = mentorminds_staking::StakingContractClient::new(&env, &staking_contract_id);
        let governance_client = mentorminds_governance::GovernanceContractClient::new(&env, &governance_contract_id);
        let timelock_client = mentorminds_timelock::TimelockContractClient::new(&env, &timelock_contract_id);
        let upgrade_registry_client = mentorminds_upgrade_registry::UpgradeRegistryContractClient::new(&env, &upgrade_registry_contract_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize all contracts
        treasury_client.initialize(&rbac_contract_id);
        staking_client.initialize(&rbac_contract_id);
        governance_client.initialize(&rbac_contract_id);
        timelock_client.initialize(&rbac_contract_id, None);
        upgrade_registry_client.initialize(&rbac_contract_id);
        
        // Test Treasury: unauthorized allocation should fail
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            treasury_client.allocate(
                &unauthorized,
                &recipient,
                &1000,
                &token_address,
                &symbol_short!("TEST"),
            );
        }));
        assert!(result.is_err(), "Treasury should reject unauthorized caller");
        
        // Test Staking: unauthorized force unstake should fail
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            staking_client.force_unstake(&unauthorized, &1);
        }));
        assert!(result.is_err(), "Staking should reject unauthorized caller");
        
        // Test Governance: unauthorized proposal creation should fail
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            governance_client.create_proposal(
                &unauthorized,
                &symbol_short!("TEST"),
                &symbol_short!("DESC"),
                None,
            );
        }));
        assert!(result.is_err(), "Governance should reject unauthorized caller");
        
        // Test Timelock: unauthorized transaction queue should fail
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            timelock_client.queue_transaction(
                &unauthorized,
                &recipient,
                &1000,
                &symbol_short!("DATA"),
                None,
            );
        }));
        assert!(result.is_err(), "Timelock should reject unauthorized caller");
        
        // Test Upgrade Registry: unauthorized upgrade proposal should fail
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            upgrade_registry_client.propose_upgrade(
                &unauthorized,
                &recipient,
                &token_address,
            );
        }));
        assert!(result.is_err(), "Upgrade Registry should reject unauthorized caller");
    }
}

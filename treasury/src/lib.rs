#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, Env, Symbol, symbol_short,
};
use mentorminds_shared::ROLE_TREASURY_ADMIN;
use soroban_sdk::{Val, Vec};

// Helper function to call RBAC contract's has_role via cross-contract call
fn check_rbac_role(env: &Env, rbac_address: &Address, address: &Address, role: &Symbol) -> bool {
    let fn_name = soroban_sdk::symbol_short!("has_role");
    let args = Vec::new(env);
    args.push_back(address.clone().into_val(env));
    args.push_back(role.clone().into_val(env));
    
    let result: bool = env.invoke_contract(rbac_address, &fn_name, &args).unwrap();
    result
}

/// Treasury allocation data structure
#[contracttype]
#[derive(Clone, Debug)]
pub struct Allocation {
    pub recipient: Address,
    pub amount: i128,
    pub token_address: Address,
    pub description: Symbol,
    pub created_at: u64,
    pub executed: bool,
}

/// Storage keys
const RBAC_ADDRESS: Symbol = symbol_short!("RBAC_ADDR");
const ALLOCATIONS: Symbol = symbol_short!("ALLOC");
const ALLOCATION_COUNT: Symbol = symbol_short!("ALL_CNT");
const INITIALIZED: Symbol = symbol_short!("INIT");

/// TTL constants
const STORAGE_TTL_THRESHOLD: u32 = 500_000;
const STORAGE_TTL_BUMP: u32 = 1_000_000;

#[contract]
pub struct TreasuryContract;

#[contractimpl]
impl TreasuryContract {
    /// Initialize the treasury contract with RBAC contract address
    pub fn initialize(env: Env, rbac_address: Address) {
        // Ensure not already initialized
        if env.storage().persistent().has(&INITIALIZED) {
            panic!("Treasury already initialized");
        }

        // Store RBAC contract address
        env.storage().persistent().set(&RBAC_ADDRESS, &rbac_address);
        env.storage().persistent().extend_ttl(&RBAC_ADDRESS, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Initialize allocation count
        env.storage().persistent().set(&ALLOCATION_COUNT, &0u64);
        env.storage().persistent().extend_ttl(&ALLOCATION_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Mark as initialized
        env.storage().persistent().set(&INITIALIZED, &true);
        env.storage().persistent().extend_ttl(&INITIALIZED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
    }

    /// Allocate funds to a recipient (requires ROLE_TREASURY_ADMIN)
    pub fn allocate(
        env: Env,
        caller: Address,
        recipient: Address,
        amount: i128,
        token_address: Address,
        description: Symbol,
    ) -> u64 {
        // Verify caller has treasury admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        let rbac_client = RbacContractClient::new(&env, &rbac_address);
        
        if !rbac_client.has_role(&caller, &ROLE_TREASURY_ADMIN) {
            panic!("Caller does not have TREASURY_ADMIN role");
        }

        // Validate amount
        if amount <= 0 {
            panic!("Amount must be greater than zero");
        }

        // Get and increment allocation count
        let mut count: u64 = env.storage().persistent().get(&ALLOCATION_COUNT).unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&ALLOCATION_COUNT, &count);
        env.storage().persistent().extend_ttl(&ALLOCATION_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Create allocation
        let allocation = Allocation {
            recipient: recipient.clone(),
            amount,
            token_address: token_address.clone(),
            description: description.clone(),
            created_at: env.ledger().timestamp(),
            executed: false,
        };

        // Store allocation
        let key = (ALLOCATIONS, count);
        env.storage().persistent().set(&key, &allocation);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("ALLOC_CRT"), count),
            (recipient, amount, token_address, description),
        );

        count
    }

    /// Execute an allocation (requires ROLE_TREASURY_ADMIN)
    pub fn execute_allocation(env: Env, caller: Address, allocation_id: u64) {
        // Verify caller has treasury admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_TREASURY_ADMIN) {
            panic!("Caller does not have TREASURY_ADMIN role");
        }

        // Get allocation
        let key = (ALLOCATIONS, allocation_id);
        let mut allocation: Allocation = env.storage().persistent().get(&key)
            .expect("Allocation not found");

        // Check if already executed
        if allocation.executed {
            panic!("Allocation already executed");
        }

        // Transfer tokens
        let token_client = token::Client::new(&env, &allocation.token_address);
        token_client.transfer(&env.current_contract_address(), &allocation.recipient, &allocation.amount);

        // Update allocation
        allocation.executed = true;
        env.storage().persistent().set(&key, &allocation);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("ALLOC_EXE"), allocation_id),
            (allocation.recipient, allocation.amount),
        );
    }

    /// Get allocation details
    pub fn get_allocation(env: Env, allocation_id: u64) -> Allocation {
        let key = (ALLOCATIONS, allocation_id);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&key).expect("Allocation not found")
    }

    /// Get total allocation count
    pub fn get_allocation_count(env: Env) -> u64 {
        env.storage().persistent().extend_ttl(&ALLOCATION_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&ALLOCATION_COUNT).unwrap_or(0)
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
    use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env, Symbol};
    use mentorminds_shared::ROLE_SUPER_ADMIN;

    fn setup_env() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        let rbac_contract_id = env.register_contract(None, mentorminds_rbac::RbacContract);
        let treasury_contract_id = env.register_contract(None, TreasuryContract);
        
        let super_admin = Address::generate(&env);
        let treasury_admin = Address::generate(&env);
        let recipient = Address::generate(&env);
        let token_address = Address::generate(&env);
        
        (env, rbac_contract_id, treasury_contract_id, super_admin, treasury_admin, recipient, token_address)
    }

    #[test]
    fn test_initialize() {
        let (env, rbac_id, treasury_id, _, _, _, _) = setup_env();
        let treasury_client = TreasuryContractClient::new(&env, &treasury_id);
        
        treasury_client.initialize(&rbac_id);
        
        assert_eq!(treasury_client.get_rbac_address(), rbac_id);
    }

    #[test]
    fn test_prevent_reinit() {
        let (env, rbac_id, treasury_id, _, _, _, _) = setup_env();
        let treasury_client = TreasuryContractClient::new(&env, &treasury_id);
        
        treasury_client.initialize(&rbac_id);
        
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            treasury_client.initialize(&rbac_id);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_allocate_with_role() {
        let (env, rbac_id, treasury_id, super_admin, treasury_admin, recipient, token_addr) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let treasury_client = TreasuryContractClient::new(&env, &treasury_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Treasury
        treasury_client.initialize(&rbac_id);
        
        // Grant treasury admin role
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &treasury_admin, &ROLE_TREASURY_ADMIN);
        
        // Allocate funds
        env.mock_all_auths();
        let alloc_id = treasury_client.allocate(
            &treasury_admin,
            &recipient,
            &1000,
            &token_addr,
            &symbol_short!("TEST"),
        );
        
        assert_eq!(alloc_id, 1);
        
        let allocation = treasury_client.get_allocation(&alloc_id);
        assert_eq!(allocation.recipient, recipient);
        assert_eq!(allocation.amount, 1000);
        assert!(!allocation.executed);
    }

    #[test]
    fn test_allocate_without_role() {
        let (env, rbac_id, treasury_id, super_admin, treasury_admin, recipient, token_addr) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let treasury_client = TreasuryContractClient::new(&env, &treasury_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Treasury
        treasury_client.initialize(&rbac_id);
        
        // Try to allocate without role - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            treasury_client.allocate(
                &treasury_admin,
                &recipient,
                &1000,
                &token_addr,
                &symbol_short!("TEST"),
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_allocation() {
        let (env, rbac_id, treasury_id, super_admin, treasury_admin, recipient, token_addr) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let treasury_client = TreasuryContractClient::new(&env, &treasury_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Treasury
        treasury_client.initialize(&rbac_id);
        
        // Grant treasury admin role
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &treasury_admin, &ROLE_TREASURY_ADMIN);
        
        // Allocate funds
        env.mock_all_auths();
        let alloc_id = treasury_client.allocate(
            &treasury_admin,
            &recipient,
            &1000,
            &token_addr,
            &symbol_short!("TEST"),
        );
        
        // Execute allocation
        env.mock_all_auths();
        treasury_client.execute_allocation(&treasury_admin, &alloc_id);
        
        let allocation = treasury_client.get_allocation(&alloc_id);
        assert!(allocation.executed);
    }

    #[test]
    fn test_execute_without_role() {
        let (env, rbac_id, treasury_id, super_admin, treasury_admin, recipient, token_addr) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let treasury_client = TreasuryContractClient::new(&env, &treasury_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Treasury
        treasury_client.initialize(&rbac_id);
        
        // Grant treasury admin role
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &treasury_admin, &ROLE_TREASURY_ADMIN);
        
        // Allocate funds
        env.mock_all_auths();
        let alloc_id = treasury_client.allocate(
            &treasury_admin,
            &recipient,
            &1000,
            &token_addr,
            &symbol_short!("TEST"),
        );
        
        // Revoke role
        env.mock_all_auths();
        rbac_client.revoke_role(&super_admin, &treasury_admin, &ROLE_TREASURY_ADMIN);
        
        // Try to execute without role - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            treasury_client.execute_allocation(&treasury_admin, &alloc_id);
        }));
        assert!(result.is_err());
    }
}

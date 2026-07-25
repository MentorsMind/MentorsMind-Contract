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

/// Transaction status enum
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransactionStatus {
    Pending,
    Ready,
    Executed,
    Cancelled,
}

/// Timelock transaction data structure
#[contracttype]
#[derive(Clone, Debug)]
pub struct TimelockTransaction {
    pub id: u64,
    pub target: Address,
    pub value: i128,
    pub data: Symbol,
    pub eta: u64,
    pub status: TransactionStatus,
    pub created_at: u64,
}

/// Storage keys
const RBAC_ADDRESS: Symbol = symbol_short!("RBAC_ADDR");
const TRANSACTIONS: Symbol = symbol_short!("TX");
const TRANSACTION_COUNT: Symbol = symbol_short!("TX_CNT");
const DELAY: Symbol = symbol_short!("DELAY");
const INITIALIZED: Symbol = symbol_short!("INIT");

/// TTL constants
const STORAGE_TTL_THRESHOLD: u32 = 500_000;
const STORAGE_TTL_BUMP: u32 = 1_000_000;

/// Default delay in seconds (2 days)
const DEFAULT_DELAY: u64 = 172_800;

#[contract]
pub struct TimelockContract;

#[contractimpl]
impl TimelockContract {
    /// Initialize the timelock contract with RBAC contract address and delay
    pub fn initialize(env: Env, rbac_address: Address, delay: Option<u64>) {
        // Ensure not already initialized
        if env.storage().persistent().has(&INITIALIZED) {
            panic!("Timelock already initialized");
        }

        // Store RBAC contract address
        env.storage().persistent().set(&RBAC_ADDRESS, &rbac_address);
        env.storage().persistent().extend_ttl(&RBAC_ADDRESS, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Store delay
        let delay_value = delay.unwrap_or(DEFAULT_DELAY);
        env.storage().persistent().set(&DELAY, &delay_value);
        env.storage().persistent().extend_ttl(&DELAY, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Initialize transaction count
        env.storage().persistent().set(&TRANSACTION_COUNT, &0u64);
        env.storage().persistent().extend_ttl(&TRANSACTION_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Mark as initialized
        env.storage().persistent().set(&INITIALIZED, &true);
        env.storage().persistent().extend_ttl(&INITIALIZED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
    }

    /// Queue a transaction (requires ROLE_SUPER_ADMIN)
    pub fn queue_transaction(
        env: Env,
        caller: Address,
        target: Address,
        value: i128,
        data: Symbol,
        delay: Option<u64>,
    ) -> u64 {
        // Verify caller has super admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        // Get delay
        let delay_value = delay.unwrap_or_else(|| {
            env.storage().persistent().get(&DELAY).expect("Delay not set")
        });

        // Calculate eta (estimated time of arrival)
        let eta = env.ledger().timestamp() + delay_value;

        // Get and increment transaction count
        let mut count: u64 = env.storage().persistent().get(&TRANSACTION_COUNT).unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&TRANSACTION_COUNT, &count);
        env.storage().persistent().extend_ttl(&TRANSACTION_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Create transaction
        let transaction = TimelockTransaction {
            id: count,
            target: target.clone(),
            value,
            data: data.clone(),
            eta,
            status: TransactionStatus::Pending,
            created_at: env.ledger().timestamp(),
        };

        // Store transaction
        let key = (TRANSACTIONS, count);
        env.storage().persistent().set(&key, &transaction);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("TX_QUEUED"), count),
            (target, value, data, eta),
        );

        count
    }

    /// Execute a transaction (requires ROLE_SUPER_ADMIN)
    pub fn execute_transaction(env: Env, caller: Address, transaction_id: u64) {
        // Verify caller has super admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        // Get transaction
        let key = (TRANSACTIONS, transaction_id);
        let mut transaction: TimelockTransaction = env.storage().persistent().get(&key)
            .expect("Transaction not found");

        // Check if transaction is ready
        if transaction.status != TransactionStatus::Ready {
            panic!("Transaction is not ready");
        }

        // Check if eta has passed
        if env.ledger().timestamp() < transaction.eta {
            panic!("Transaction is not yet executable");
        }

        // Mark as executed
        transaction.status = TransactionStatus::Executed;
        env.storage().persistent().set(&key, &transaction);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("TX_EXEC"), transaction_id),
            (transaction.target, transaction.value),
        );
    }

    /// Cancel a transaction (requires ROLE_SUPER_ADMIN)
    pub fn cancel_transaction(env: Env, caller: Address, transaction_id: u64) {
        // Verify caller has super admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        // Get transaction
        let key = (TRANSACTIONS, transaction_id);
        let mut transaction: TimelockTransaction = env.storage().persistent().get(&key)
            .expect("Transaction not found");

        // Check if transaction can be cancelled
        if transaction.status == TransactionStatus::Executed {
            panic!("Cannot cancel executed transaction");
        }

        // Mark as cancelled
        transaction.status = TransactionStatus::Cancelled;
        env.storage().persistent().set(&key, &transaction);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("TX_CAN"), transaction_id),
            (caller, transaction.target),
        );
    }

    /// Check if a transaction is ready to be executed
    pub fn check_transaction(env: Env, transaction_id: u64) {
        let key = (TRANSACTIONS, transaction_id);
        let mut transaction: TimelockTransaction = env.storage().persistent().get(&key)
            .expect("Transaction not found");

        // Check if transaction is pending
        if transaction.status != TransactionStatus::Pending {
            return;
        }

        // Check if eta has passed
        if env.ledger().timestamp() >= transaction.eta {
            transaction.status = TransactionStatus::Ready;
            env.storage().persistent().set(&key, &transaction);
            env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

            // Emit event
            env.events().publish(
                (symbol_short!("TX_READY"), transaction_id),
                transaction_id,
            );
        }
    }

    /// Update delay (requires ROLE_SUPER_ADMIN)
    pub fn set_delay(env: Env, caller: Address, new_delay: u64) {
        // Verify caller has super admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_SUPER_ADMIN) {
            panic!("Caller does not have SUPER_ADMIN role");
        }

        // Update delay
        env.storage().persistent().set(&DELAY, &new_delay);
        env.storage().persistent().extend_ttl(&DELAY, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("DEL_UPD"),),
            (caller, new_delay),
        );
    }

    /// Get transaction details
    pub fn get_transaction(env: Env, transaction_id: u64) -> TimelockTransaction {
        let key = (TRANSACTIONS, transaction_id);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&key).expect("Transaction not found")
    }

    /// Get total transaction count
    pub fn get_transaction_count(env: Env) -> u64 {
        env.storage().persistent().extend_ttl(&TRANSACTION_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&TRANSACTION_COUNT).unwrap_or(0)
    }

    /// Get current delay
    pub fn get_delay(env: Env) -> u64 {
        env.storage().persistent().extend_ttl(&DELAY, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&DELAY).expect("Delay not set")
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

    fn setup_env() -> (Env, Address, Address, Address) {
        let env = Env::default();
        let rbac_contract_id = env.register_contract(None, mentorminds_rbac::RbacContract);
        let timelock_contract_id = env.register_contract(None, TimelockContract);
        
        let super_admin = Address::generate(&env);
        let target = Address::generate(&env);
        
        (env, rbac_contract_id, timelock_contract_id, super_admin, target)
    }

    #[test]
    fn test_initialize() {
        let (env, rbac_id, timelock_id, _, _) = setup_env();
        let timelock_client = TimelockContractClient::new(&env, &timelock_id);
        
        timelock_client.initialize(&rbac_id, None);
        
        assert_eq!(timelock_client.get_rbac_address(), rbac_id);
        assert_eq!(timelock_client.get_delay(), DEFAULT_DELAY);
    }

    #[test]
    fn test_queue_transaction_with_role() {
        let (env, rbac_id, timelock_id, super_admin, target) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let timelock_client = TimelockContractClient::new(&env, &timelock_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Timelock
        timelock_client.initialize(&rbac_id, None);
        
        // Queue transaction
        env.mock_all_auths();
        let tx_id = timelock_client.queue_transaction(
            &super_admin,
            &target,
            &1000,
            &symbol_short!("DATA"),
            None,
        );
        
        assert_eq!(tx_id, 1);
        
        let tx = timelock_client.get_transaction(&tx_id);
        assert_eq!(tx.target, target);
        assert_eq!(tx.status, TransactionStatus::Pending);
    }

    #[test]
    fn test_queue_transaction_without_role() {
        let (env, rbac_id, timelock_id, super_admin, target) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let timelock_client = TimelockContractClient::new(&env, &timelock_id);
        
        let unauthorized = Address::generate(&env);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Timelock
        timelock_client.initialize(&rbac_id, None);
        
        // Try to queue transaction without role - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            timelock_client.queue_transaction(
                &unauthorized,
                &target,
                &1000,
                &symbol_short!("DATA"),
                None,
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_transaction() {
        let (env, rbac_id, timelock_id, super_admin, target) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let timelock_client = TimelockContractClient::new(&env, &timelock_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Timelock with short delay
        timelock_client.initialize(&rbac_id, Some(1));
        
        // Queue transaction
        env.mock_all_auths();
        let tx_id = timelock_client.queue_transaction(
            &super_admin,
            &target,
            &1000,
            &symbol_short!("DATA"),
            None,
        );
        
        // Advance time past delay
        env.ledger().set(env.ledger().seq() + 10, env.ledger().timestamp() + 10);
        
        // Check transaction
        timelock_client.check_transaction(&tx_id);
        
        // Execute transaction
        env.mock_all_auths();
        timelock_client.execute_transaction(&super_admin, &tx_id);
        
        let tx = timelock_client.get_transaction(&tx_id);
        assert_eq!(tx.status, TransactionStatus::Executed);
    }

    #[test]
    fn test_cancel_transaction() {
        let (env, rbac_id, timelock_id, super_admin, target) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let timelock_client = TimelockContractClient::new(&env, &timelock_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Timelock
        timelock_client.initialize(&rbac_id, None);
        
        // Queue transaction
        env.mock_all_auths();
        let tx_id = timelock_client.queue_transaction(
            &super_admin,
            &target,
            &1000,
            &symbol_short!("DATA"),
            None,
        );
        
        // Cancel transaction
        env.mock_all_auths();
        timelock_client.cancel_transaction(&super_admin, &tx_id);
        
        let tx = timelock_client.get_transaction(&tx_id);
        assert_eq!(tx.status, TransactionStatus::Cancelled);
    }

    #[test]
    fn test_set_delay() {
        let (env, rbac_id, timelock_id, super_admin, _) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let timelock_client = TimelockContractClient::new(&env, &timelock_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Timelock
        timelock_client.initialize(&rbac_id, None);
        
        // Set new delay
        env.mock_all_auths();
        timelock_client.set_delay(&super_admin, &86400);
        
        assert_eq!(timelock_client.get_delay(), 86400);
    }
}

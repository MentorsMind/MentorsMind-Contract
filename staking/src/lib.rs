#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, Env, Symbol, symbol_short,
};
use mentorminds_shared::ROLE_STAKING_ADMIN;
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

/// Stake data structure
#[contracttype]
#[derive(Clone, Debug)]
pub struct Stake {
    pub staker: Address,
    pub amount: i128,
    pub token_address: Address,
    pub staked_at: u64,
    pub lock_period: u64,
}

/// Storage keys
const RBAC_ADDRESS: Symbol = symbol_short!("RBAC_ADDR");
const STAKES: Symbol = symbol_short!("STAKE");
const STAKE_COUNT: Symbol = symbol_short!("STK_CNT");
const TOTAL_STAKED: Symbol = symbol_short!("TOT_STK");
const INITIALIZED: Symbol = symbol_short!("INIT");

/// TTL constants
const STORAGE_TTL_THRESHOLD: u32 = 500_000;
const STORAGE_TTL_BUMP: u32 = 1_000_000;

#[contract]
pub struct StakingContract;

#[contractimpl]
impl StakingContract {
    /// Initialize the staking contract with RBAC contract address
    pub fn initialize(env: Env, rbac_address: Address) {
        // Ensure not already initialized
        if env.storage().persistent().has(&INITIALIZED) {
            panic!("Staking already initialized");
        }

        // Store RBAC contract address
        env.storage().persistent().set(&RBAC_ADDRESS, &rbac_address);
        env.storage().persistent().extend_ttl(&RBAC_ADDRESS, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Initialize stake count and total staked
        env.storage().persistent().set(&STAKE_COUNT, &0u64);
        env.storage().persistent().extend_ttl(&STAKE_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        
        env.storage().persistent().set(&TOTAL_STAKED, &0i128);
        env.storage().persistent().extend_ttl(&TOTAL_STAKED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Mark as initialized
        env.storage().persistent().set(&INITIALIZED, &true);
        env.storage().persistent().extend_ttl(&INITIALIZED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
    }

    /// Stake tokens (public function)
    pub fn stake(env: Env, staker: Address, amount: i128, token_address: Address, lock_period: u64) -> u64 {
        // Validate amount
        if amount <= 0 {
            panic!("Amount must be greater than zero");
        }

        // Validate lock period
        if lock_period == 0 {
            panic!("Lock period must be greater than zero");
        }

        // Require staker authorization
        staker.require_auth();

        // Transfer tokens from staker to contract
        let token_client = token::Client::new(&env, &token_address);
        token_client.transfer(&staker, &env.current_contract_address(), &amount);

        // Get and increment stake count
        let mut count: u64 = env.storage().persistent().get(&STAKE_COUNT).unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&STAKE_COUNT, &count);
        env.storage().persistent().extend_ttl(&STAKE_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Update total staked
        let mut total: i128 = env.storage().persistent().get(&TOTAL_STAKED).unwrap_or(0);
        total += amount;
        env.storage().persistent().set(&TOTAL_STAKED, &total);
        env.storage().persistent().extend_ttl(&TOTAL_STAKED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Create stake
        let stake = Stake {
            staker: staker.clone(),
            amount,
            token_address: token_address.clone(),
            staked_at: env.ledger().timestamp(),
            lock_period,
        };

        // Store stake
        let key = (STAKES, count);
        env.storage().persistent().set(&key, &stake);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("STAKED"), count),
            (staker, amount, token_address, lock_period),
        );

        count
    }

    /// Unstake tokens (requires ROLE_STAKING_ADMIN for early withdrawal)
    pub fn unstake(env: Env, caller: Address, stake_id: u64) {
        // Get stake
        let key = (STAKES, stake_id);
        let stake: Stake = env.storage().persistent().get(&key)
            .expect("Stake not found");

        // Check if lock period has expired
        let current_time = env.ledger().timestamp();
        let unlock_time = stake.staked_at + stake.lock_period;

        if current_time < unlock_time {
            // Early withdrawal requires staking admin role
            let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
                .expect("RBAC address not set");
            let rbac_client = RbacContractClient::new(&env, &rbac_address);
            
            if !rbac_client.has_role(&caller, &ROLE_STAKING_ADMIN) {
                panic!("Early withdrawal requires STAKING_ADMIN role or lock period expired");
            }
        } else {
            // Normal withdrawal - must be staker
            if caller != stake.staker {
                panic!("Only staker can unstake after lock period");
            }
        }

        // Require caller authorization
        caller.require_auth();

        // Transfer tokens back to staker
        let token_client = token::Client::new(&env, &stake.token_address);
        token_client.transfer(&env.current_contract_address(), &stake.staker, &stake.amount);

        // Update total staked
        let mut total: i128 = env.storage().persistent().get(&TOTAL_STAKED).unwrap_or(0);
        total -= stake.amount;
        env.storage().persistent().set(&TOTAL_STAKED, &total);
        env.storage().persistent().extend_ttl(&TOTAL_STAKED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Remove stake
        env.storage().persistent().remove(&key);

        // Emit event
        env.events().publish(
            (symbol_short!("UNSTAKED"), stake_id),
            (stake.staker, stake.amount),
        );
    }

    /// Force unstake by admin (requires ROLE_STAKING_ADMIN)
    pub fn force_unstake(env: Env, caller: Address, stake_id: u64) {
        // Verify caller has staking admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_STAKING_ADMIN) {
            panic!("Caller does not have STAKING_ADMIN role");
        }

        // Get stake
        let key = (STAKES, stake_id);
        let stake: Stake = env.storage().persistent().get(&key)
            .expect("Stake not found");

        // Transfer tokens back to staker
        let token_client = token::Client::new(&env, &stake.token_address);
        token_client.transfer(&env.current_contract_address(), &stake.staker, &stake.amount);

        // Update total staked
        let mut total: i128 = env.storage().persistent().get(&TOTAL_STAKED).unwrap_or(0);
        total -= stake.amount;
        env.storage().persistent().set(&TOTAL_STAKED, &total);
        env.storage().persistent().extend_ttl(&TOTAL_STAKED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Remove stake
        env.storage().persistent().remove(&key);

        // Emit event
        env.events().publish(
            (symbol_short!("FC_UNSTK"), stake_id),
            (stake.staker, stake.amount, caller),
        );
    }

    /// Get stake details
    pub fn get_stake(env: Env, stake_id: u64) -> Stake {
        let key = (STAKES, stake_id);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&key).expect("Stake not found")
    }

    /// Get total stake count
    pub fn get_stake_count(env: Env) -> u64 {
        env.storage().persistent().extend_ttl(&STAKE_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&STAKE_COUNT).unwrap_or(0)
    }

    /// Get total staked amount
    pub fn get_total_staked(env: Env) -> i128 {
        env.storage().persistent().extend_ttl(&TOTAL_STAKED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&TOTAL_STAKED).unwrap_or(0)
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

    fn setup_env() -> (Env, Address, Address, Address, Address, Address) {
        let env = Env::default();
        let rbac_contract_id = env.register_contract(None, mentorminds_rbac::RbacContract);
        let staking_contract_id = env.register_contract(None, StakingContract);
        
        let super_admin = Address::generate(&env);
        let staking_admin = Address::generate(&env);
        let staker = Address::generate(&env);
        let token_address = Address::generate(&env);
        
        (env, rbac_contract_id, staking_contract_id, super_admin, staking_admin, staker, token_address)
    }

    #[test]
    fn test_initialize() {
        let (env, rbac_id, staking_id, _, _, _, _) = setup_env();
        let staking_client = StakingContractClient::new(&env, &staking_id);
        
        staking_client.initialize(&rbac_id);
        
        assert_eq!(staking_client.get_rbac_address(), rbac_id);
    }

    #[test]
    fn test_stake() {
        let (env, rbac_id, staking_id, super_admin, _, staker, token_addr) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let staking_client = StakingContractClient::new(&env, &staking_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Staking
        staking_client.initialize(&rbac_id);
        
        // Stake tokens
        env.mock_all_auths();
        let stake_id = staking_client.stake(&staker, &1000, &token_addr, &100);
        
        assert_eq!(stake_id, 1);
        assert_eq!(staking_client.get_total_staked(), 1000);
        
        let stake = staking_client.get_stake(&stake_id);
        assert_eq!(stake.staker, staker);
        assert_eq!(stake.amount, 1000);
    }

    #[test]
    fn test_unstake_after_lock_period() {
        let (env, rbac_id, staking_id, super_admin, _, staker, token_addr) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let staking_client = StakingContractClient::new(&env, &staking_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Staking
        staking_client.initialize(&rbac_id);
        
        // Stake tokens
        env.mock_all_auths();
        let stake_id = staking_client.stake(&staker, &1000, &token_addr, &100);
        
        // Advance time past lock period
        env.ledger().set(env.ledger().seq() + 100, env.ledger().timestamp() + 200);
        
        // Unstake
        env.mock_all_auths();
        staking_client.unstake(&staker, &stake_id);
        
        assert_eq!(staking_client.get_total_staked(), 0);
    }

    #[test]
    fn test_early_unstake_with_admin_role() {
        let (env, rbac_id, staking_id, super_admin, staking_admin, staker, token_addr) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let staking_client = StakingContractClient::new(&env, &staking_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Staking
        staking_client.initialize(&rbac_id);
        
        // Grant staking admin role
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &staking_admin, &ROLE_STAKING_ADMIN);
        
        // Stake tokens
        env.mock_all_auths();
        let stake_id = staking_client.stake(&staker, &1000, &token_addr, &100);
        
        // Early unstake by admin
        env.mock_all_auths();
        staking_client.unstake(&staking_admin, &stake_id);
        
        assert_eq!(staking_client.get_total_staked(), 0);
    }

    #[test]
    fn test_early_unstake_without_role() {
        let (env, rbac_id, staking_id, super_admin, _, staker, token_addr) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let staking_client = StakingContractClient::new(&env, &staking_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Staking
        staking_client.initialize(&rbac_id);
        
        // Stake tokens
        env.mock_all_auths();
        let stake_id = staking_client.stake(&staker, &1000, &token_addr, &100);
        
        // Try early unstake without admin role - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            staking_client.unstake(&staker, &stake_id);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_force_unstake() {
        let (env, rbac_id, staking_id, super_admin, staking_admin, staker, token_addr) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let staking_client = StakingContractClient::new(&env, &staking_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Staking
        staking_client.initialize(&rbac_id);
        
        // Grant staking admin role
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &staking_admin, &ROLE_STAKING_ADMIN);
        
        // Stake tokens
        env.mock_all_auths();
        let stake_id = staking_client.stake(&staker, &1000, &token_addr, &100);
        
        // Force unstake by admin
        env.mock_all_auths();
        staking_client.force_unstake(&staking_admin, &stake_id);
        
        assert_eq!(staking_client.get_total_staked(), 0);
    }

    #[test]
    fn test_force_unstake_without_role() {
        let (env, rbac_id, staking_id, super_admin, _, staker, token_addr) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let staking_client = StakingContractClient::new(&env, &staking_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Staking
        staking_client.initialize(&rbac_id);
        
        // Stake tokens
        env.mock_all_auths();
        let stake_id = staking_client.stake(&staker, &1000, &token_addr, &100);
        
        // Try force unstake without admin role - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            staking_client.force_unstake(&staker, &stake_id);
        }));
        assert!(result.is_err());
    }
}

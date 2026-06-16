#![no_std]
use shared::HealthReporter;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env,
    IntoVal, Symbol, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InsufficientBalance = 4,
    TokenNotApproved = 5,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationHistory {
    pub token: Address,
    pub recipient: Address,
    pub amount: i128,
    pub timestamp: u64,
}

/// Event data emitted when a token is approved or rejected in the treasury.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryTokenApprovalEvent {
    pub token: Address,
    pub approved: bool,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    StakingContract,
    AllocationCount,
    /// Individual allocation history: DataKey::Allocation(index) → AllocationHistory
    Allocation(u32),
    /// Token whitelist: DataKey::ApprovedToken(token_address) → bool
    ApprovedToken(Address),
    HealthDashboard
}

#[contract]
pub struct TreasuryContract;

#[contractimpl]
impl TreasuryContract {
    /// Initialize treasury contract with admin and staking contract address
    
    pub fn set_health_dashboard(env: soroban_sdk::Env, dashboard: soroban_sdk::Address) {
        let admin: soroban_sdk::Address = env.storage().persistent().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        env.storage().persistent().set(&DataKey::HealthDashboard, &dashboard);
    }

    pub fn initialize(env: Env, admin: Address, staking_contract: Address) -> Result<(), Error> {
        if env.storage().persistent().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::StakingContract, &staking_contract);

        env.storage()
            .persistent()
            .set(&DataKey::AllocationCount, &0u32);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Token Whitelist Management
    // -----------------------------------------------------------------------

    /// Add or remove an approved token from the treasury whitelist (admin only).
    /// Only whitelisted tokens can be deposited, allocated, or distributed.
    pub fn set_approved_token(env: Env, token_address: Address, approved: bool) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage().persistent().set(&key, &approved);

        // Emit token approval/rejection event
        if approved {
            env.events().publish(
                (symbol_short!("treasury"), symbol_short!("tok_appr")),
                TreasuryTokenApprovalEvent {
                    token: token_address,
                    approved: true,
                },
            );
        } else {
            env.events().publish(
                (symbol_short!("treasury"), symbol_short!("tok_rej")),
                TreasuryTokenApprovalEvent {
                    token: token_address,
                    approved: false,
                },
            );
        }

        Ok(())
    }

    /// Check if a token is on the treasury's approved whitelist.
    pub fn is_token_approved(env: Env, token_address: Address) -> bool {
        Self::_is_token_approved(&env, &token_address)
    }

    /// Internal token whitelist check.
    fn _is_token_approved(env: &Env, token_address: &Address) -> bool {
        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage()
            .persistent()
            .get::<_, bool>(&key)
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Deposit / Allocate / Distribute
    // -----------------------------------------------------------------------

    /// Accept deposits of approved Stellar assets only.
    /// The token must be on the treasury's whitelist.
    pub fn deposit(env: Env, from: Address, token: Address, amount: i128) -> Result<(), Error> {
        from.require_auth();

        // *** STRICT TOKEN WHITELIST VALIDATION ***
        if !Self::_is_token_approved(&env, &token) {
            panic!("Token not approved");
        }

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&from, &env.current_contract_address(), &amount);

        // Emit deposited event
        env.events().publish(
            (symbol_short!("deposit"), from.clone(), token.clone()),
            amount,
        );

        Ok(())
    }

    /// get_balance(env, token: Address) -> i128
    pub fn get_balance(env: Env, token: Address) -> i128 {
        let token_client = token::Client::new(&env, &token);
        token_client.balance(&env.current_contract_address())
    }

    /// allocate(env, token, recipient, amount) — governance/timelock only.
    /// The token must be on the treasury's whitelist.
    pub fn allocate(
        env: Env,
        token: Address,
        recipient: Address,
        amount: i128,
    ) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        // *** STRICT TOKEN WHITELIST VALIDATION ***
        if !Self::_is_token_approved(&env, &token) {
            return Err(Error::TokenNotApproved);
        }

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&env.current_contract_address(), &recipient, &amount);

        // Track allocation history with counter-based keying
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AllocationCount)
            .unwrap_or(0u32);

        env.storage().persistent().set(
            &DataKey::Allocation(count),
            &AllocationHistory {
                token: token.clone(),
                recipient: recipient.clone(),
                amount,
                timestamp: env.ledger().timestamp(),
            },
        );
        env.storage().persistent().set(&DataKey::AllocationCount, &(count + 1));

        // Emit allocated event
        env.events().publish(
            (symbol_short!("allocate"), recipient.clone(), token.clone()),
            amount,
        );

        Ok(())
    }

    /// distribute_to_stakers(env, token, total_amount) — pro-rata by stake amount.
    /// The token must be on the treasury's whitelist.
    pub fn distribute_to_stakers(
        env: Env,
        token: Address,
        total_amount: i128,
    ) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        // *** STRICT TOKEN WHITELIST VALIDATION ***
        if !Self::_is_token_approved(&env, &token) {
            return Err(Error::TokenNotApproved);
        }

        let staking_contract = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::StakingContract)
            .ok_or(Error::NotInitialized)?;

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(
            &env.current_contract_address(),
            &staking_contract,
            &total_amount,
        );

        // Call staking contract's distribute_revenue
        env.invoke_contract::<()>(
            &staking_contract,
            &Symbol::new(&env, "distribute_revenue"),
            (token.clone(), total_amount).into_val(&env),
        );

        // Emit distributed event
        env.events().publish(
            (
                symbol_short!("distrib"),
                staking_contract.clone(),
                token.clone(),
            ),
            total_amount,
        );

        Ok(())
    }

    /// buyback_and_burn(env, xlm_amount) — swap XLM for MNT on DEX, burn MNT.
    /// Both xlm_token and mnt_token must be on the treasury's whitelist.
    pub fn buyback_and_burn(
        env: Env,
        xlm_token: Address,
        mnt_token: Address,
        dex_contract: Address,
        xlm_amount: i128,
        min_mnt_out: i128,
    ) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        // *** STRICT TOKEN WHITELIST VALIDATION ***
        if !Self::_is_token_approved(&env, &xlm_token) {
            return Err(Error::TokenNotApproved);
        }
        if !Self::_is_token_approved(&env, &mnt_token) {
            return Err(Error::TokenNotApproved);
        }

        // 1. Transfer XLM to DEX (or approve)
        let xlm_client = token::Client::new(&env, &xlm_token);
        xlm_client.transfer(&env.current_contract_address(), &dex_contract, &xlm_amount);

        // 2. Call DEX swap (assuming it returns the amount of MNT received)
        let mnt_received: i128 = env.invoke_contract(
            &dex_contract,
            &Symbol::new(&env, "swap"),
            (xlm_token.clone(), mnt_token.clone(), xlm_amount).into_val(&env),
        );

        // Validate minimum output to prevent slippage attacks
        if mnt_received < min_mnt_out {
            panic!("Slippage exceeded: insufficient MNT received from DEX");
        }

        // 3. Burn MNT
        env.invoke_contract::<()>(
            &mnt_token,
            &Symbol::new(&env, "burn"),
            (env.current_contract_address(), mnt_received).into_val(&env),
        );

        // Emit buyback event
        env.events()
            .publish((symbol_short!("buyback"), mnt_token.clone()), mnt_received);

        Ok(())
    }

    pub fn get_history_page(env: Env, offset: u32, limit: u32) -> Vec<AllocationHistory> {
        let total_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AllocationCount)
            .unwrap_or(0u32);

        let mut result = Vec::new(&env);
        let end = offset.saturating_add(limit).min(total_count);

        for i in offset..end {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, AllocationHistory>(&DataKey::Allocation(i))
            {
                result.push_back(record);
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    #[contract]
    pub struct MockDEX;

    #[contractimpl]
    impl MockDEX {
        pub fn swap(_env: Env, _token_in: Address, _token_out: Address, amount_in: i128) -> i128 {
            amount_in
        }
    }

    #[contract]
    pub struct MockStaking;

    #[contractimpl]
    impl MockStaking {
        pub fn distribute_revenue(_env: Env, _token: Address, _amount: i128) {}
    }

    #[contract]
    pub struct MockMNT;

    #[contractimpl]
    impl MockMNT {
        pub fn burn(_env: Env, _from: Address, _amount: i128) {}
    }

    fn setup_test(env: &Env) -> (Address, Address, Address) {
        let admin = Address::generate(env);
        let staking = env.register_contract(None, MockStaking);

        let contract_id = env.register_contract(None, TreasuryContract);
        let client = TreasuryContractClient::new(env, &contract_id);
        client.initialize(&admin, &staking);

        (admin, staking, contract_id)
    }

    #[test]
    fn test_initialization() {
        let env = Env::default();
        let (admin, staking, _) = setup_test(&env);

        let client =
            TreasuryContractClient::new(&env, &env.register_contract(None, TreasuryContract));
        client.initialize(&admin, &staking);
        let result = client.try_initialize(&admin, &staking);
        assert!(result.is_err());
    }

    #[test]
    fn test_deposit_and_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, contract_id) = setup_test(&env);
        let user = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_client = token::Client::new(&env, &token_addr);
        let stellar_asset_client = token::StellarAssetClient::new(&env, &token_addr);

        stellar_asset_client.mint(&user, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);

        // Approve the token first
        treasury_client.set_approved_token(&token_addr, &true);
        
        treasury_client.deposit(&user, &token_addr, &500);

        assert_eq!(treasury_client.get_balance(&token_addr), 500);
        assert_eq!(token_client.balance(&user), 500);
        assert_eq!(token_client.balance(&contract_id), 500);
    }

    #[test]
    #[should_panic(expected = "Token not approved")]
    fn test_deposit_unapproved_token() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, contract_id) = setup_test(&env);
        let user = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let stellar_asset_client = token::StellarAssetClient::new(&env, &token_addr);

        stellar_asset_client.mint(&user, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);

        // Do not approve token - deposit should panic
        treasury_client.deposit(&user, &token_addr, &500);
    }

    #[test]
    fn test_allocate() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, contract_id) = setup_test(&env);
        let recipient = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_client = token::Client::new(&env, &token_addr);
        let stellar_asset_client = token::StellarAssetClient::new(&env, &token_addr);

        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);

        // Approve the token first
        treasury_client.set_approved_token(&token_addr, &true);

        env.ledger().set_timestamp(12345);
        treasury_client.allocate(&token_addr, &recipient, &400);

        assert_eq!(treasury_client.get_balance(&token_addr), 600);
        assert_eq!(token_client.balance(&recipient), 400);

        let history = treasury_client.get_history_page(&0, &10);
        assert_eq!(history.len(), 1);
        let entry = history.get(0).unwrap();
        assert_eq!(entry.token, token_addr);
        assert_eq!(entry.recipient, recipient);
        assert_eq!(entry.amount, 400);
        assert_eq!(entry.timestamp, 12345);
    }

    #[test]
    fn test_allocate_unapproved_token_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, contract_id) = setup_test(&env);
        let recipient = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let stellar_asset_client = token::StellarAssetClient::new(&env, &token_addr);

        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);

        // Do NOT approve — allocate should fail
        let result = treasury_client.try_allocate(&token_addr, &recipient, &400);
        assert!(result.is_err(), "unapproved token allocation must fail");
    }

    #[test]
    fn test_distribute_to_stakers() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, staking_addr, contract_id) = setup_test(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_client = token::Client::new(&env, &token_addr);
        let stellar_asset_client = token::StellarAssetClient::new(&env, &token_addr);

        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);

        // Approve the token first
        treasury_client.set_approved_token(&token_addr, &true);

        treasury_client.distribute_to_stakers(&token_addr, &300);

        assert_eq!(treasury_client.get_balance(&token_addr), 700);
        assert_eq!(token_client.balance(&staking_addr), 300);
    }

    #[test]
    fn test_distribute_unapproved_token_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, contract_id) = setup_test(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let stellar_asset_client = token::StellarAssetClient::new(&env, &token_addr);

        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);

        // Do NOT approve — distribution should fail
        let result = treasury_client.try_distribute_to_stakers(&token_addr, &300);
        assert!(result.is_err(), "unapproved token distribution must fail");
    }

    #[test]
    fn test_buyback_and_burn() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, contract_id) = setup_test(&env);

        let xlm_addr = env.register_stellar_asset_contract(admin.clone());
        let mnt_addr = env.register_contract(None, MockMNT);
        let dex_addr = env.register_contract(None, MockDEX);

        let xlm_client = token::Client::new(&env, &xlm_addr);
        let stellar_asset_client = token::StellarAssetClient::new(&env, &xlm_addr);
        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);

        // Approve both tokens
        treasury_client.set_approved_token(&xlm_addr, &true);
        treasury_client.set_approved_token(&mnt_addr, &true);

        treasury_client.buyback_and_burn(&xlm_addr, &mnt_addr, &dex_addr, &1000, &500);

        assert_eq!(xlm_client.balance(&contract_id), 0);
        assert_eq!(xlm_client.balance(&dex_addr), 1000);
    }

    #[test]
    fn test_buyback_unapproved_token_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, contract_id) = setup_test(&env);

        let xlm_addr = env.register_stellar_asset_contract(admin.clone());
        let mnt_addr = env.register_contract(None, MockMNT);
        let dex_addr = env.register_contract(None, MockDEX);

        let stellar_asset_client = token::StellarAssetClient::new(&env, &xlm_addr);
        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);

        // Do NOT approve tokens — buyback should fail
        let result = treasury_client.try_buyback_and_burn(&xlm_addr, &mnt_addr, &dex_addr, &1000, &500);
        assert!(result.is_err(), "unapproved token buyback must fail");
    }

    #[test]
    fn test_token_whitelist_toggle() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, contract_id) = setup_test(&env);
        let treasury_client = TreasuryContractClient::new(&env, &contract_id);

        let token = Address::generate(&env);
        assert!(!treasury_client.is_token_approved(&token));
        treasury_client.set_approved_token(&token, &true);
        assert!(treasury_client.is_token_approved(&token));
        treasury_client.set_approved_token(&token, &false);
        assert!(!treasury_client.is_token_approved(&token));
    }
}

impl shared::HealthReporter for TreasuryContract {
    fn report_metric(env: soroban_sdk::Env, metric: soroban_sdk::Symbol, value: i128) {
        if let Some(dashboard) = env.storage().persistent().get::<_, soroban_sdk::Address>(&DataKey::HealthDashboard) {
            let _ = env.try_invoke_contract::<(), soroban_sdk::Error>(
                &dashboard,
                &soroban_sdk::Symbol::new(&env, "receive_metric"),
                (soroban_sdk::Symbol::new(&env, "treasury"), metric, value).into_val(&env),
            );
        }
    }
}

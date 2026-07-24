#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracterror, contracttype, symbol_short, token, Address, Env, Symbol,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized      = 1,
    NotInitialized          = 2,
    NotAdmin                = 3,
    InsufficientShares      = 4,
    WithdrawLocked          = 5,
    InsufficientPoolBalance = 6,
    ZeroAmount              = 7,
}

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    // Instance storage
    Admin,
    Token,
    PoolBalance,
    TotalClaimsPaid,
    TotalActiveEscrow,
    // Persistent — per provider
    ProviderShares(Address),
    WithdrawUnlock(Address),
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// 7-day withdrawal lock in seconds
const WITHDRAW_LOCK_SECS: u64 = 7 * 24 * 60 * 60;

/// Yield: 0.1% of platform fees (10 bps)
const YIELD_BPS: u32 = 10;

/// Alert threshold: 500 bps (5%)
const COVERAGE_ALERT_BPS: u32 = 500;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct InsuranceContract;

#[contractimpl]
impl InsuranceContract {
    /// Initialize the insurance pool.
    pub fn initialize(env: Env, admin: Address, token: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Token, &token);
        env.storage().instance().set(&DataKey::PoolBalance, &0i128);
        env.storage().instance().set(&DataKey::TotalClaimsPaid, &0i128);
        env.storage().instance().set(&DataKey::TotalActiveEscrow, &0i128);
        Ok(())
    }

    /// Deposit USDC into the insurance pool.
    pub fn deposit(env: Env, provider: Address, amount: i128) -> Result<(), Error> {
        Self::assert_initialized(&env)?;
        if amount <= 0 {
            return Err(Error::ZeroAmount);
        }
        provider.require_auth();

        let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        token::Client::new(&env, &token).transfer(&provider, &env.current_contract_address(), &amount);

        let shares: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::ProviderShares(provider.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::ProviderShares(provider.clone()), &(shares + amount));

        let pool: i128 = env.storage().instance().get(&DataKey::PoolBalance).unwrap_or(0);
        env.storage().instance().set(&DataKey::PoolBalance, &(pool + amount));

        env.events().publish(
            (symbol_short!("insurance"), Symbol::new(&env, "deposited"), provider.clone()),
            (amount, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Withdraw USDC from the pool. Subject to 7-day lock from last deposit.
    pub fn withdraw(env: Env, provider: Address, amount: i128) -> Result<(), Error> {
        Self::assert_initialized(&env)?;
        if amount <= 0 {
            return Err(Error::ZeroAmount);
        }
        provider.require_auth();

        // Check withdrawal lock
        let unlock_time: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::WithdrawUnlock(provider.clone()))
            .unwrap_or(0);
        if env.ledger().timestamp() < unlock_time {
            return Err(Error::WithdrawLocked);
        }

        let shares: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::ProviderShares(provider.clone()))
            .unwrap_or(0);
        if shares < amount {
            return Err(Error::InsufficientShares);
        }

        let pool: i128 = env.storage().instance().get(&DataKey::PoolBalance).unwrap_or(0);
        if pool < amount {
            return Err(Error::InsufficientPoolBalance);
        }

        env.storage()
            .persistent()
            .set(&DataKey::ProviderShares(provider.clone()), &(shares - amount));
        env.storage().instance().set(&DataKey::PoolBalance, &(pool - amount));

        let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        token::Client::new(&env, &token).transfer(&env.current_contract_address(), &provider, &amount);

        env.events().publish(
            (symbol_short!("insurance"), Symbol::new(&env, "withdrawn"), provider.clone()),
            (amount, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Pay a learner from the pool when a dispute is resolved in their favor.
    /// Admin only.
    pub fn claim(env: Env, escrow_id: Symbol, learner: Address, amount: i128) -> Result<(), Error> {
        Self::assert_initialized(&env)?;
        if amount <= 0 {
            return Err(Error::ZeroAmount);
        }

        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let pool: i128 = env.storage().instance().get(&DataKey::PoolBalance).unwrap_or(0);
        if pool < amount {
            return Err(Error::InsufficientPoolBalance);
        }

        env.storage().instance().set(&DataKey::PoolBalance, &(pool - amount));

        let paid: i128 = env.storage().instance().get(&DataKey::TotalClaimsPaid).unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::TotalClaimsPaid, &(paid + amount));

        let token: Address = env.storage().instance().get(&DataKey::Token).unwrap();
        token::Client::new(&env, &token).transfer(&env.current_contract_address(), &learner, &amount);

        env.events().publish(
            (symbol_short!("insurance"), Symbol::new(&env, "claim_paid"), escrow_id),
            (learner, amount, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Accrue yield to a provider (0.1% of platform fees). Admin only.
    pub fn accrue_yield(env: Env, provider: Address, platform_fee: i128) -> Result<(), Error> {
        Self::assert_initialized(&env)?;
        if platform_fee <= 0 {
            return Err(Error::ZeroAmount);
        }
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();

        let yield_amount = platform_fee
            .checked_mul(YIELD_BPS as i128)
            .expect("overflow")
            / 10_000;

        let shares: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::ProviderShares(provider.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::ProviderShares(provider.clone()), &(shares + yield_amount));

        let pool: i128 = env.storage().instance().get(&DataKey::PoolBalance).unwrap_or(0);
        env.storage().instance().set(&DataKey::PoolBalance, &(pool + yield_amount));

        Ok(())
    }

    /// Update total active escrow value (called by escrow contract). Admin only.
    pub fn set_active_escrow_value(env: Env, value: i128) -> Result<(), Error> {
        Self::assert_initialized(&env)?;
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        env.storage().instance().set(&DataKey::TotalActiveEscrow, &value);
        Ok(())
    }

    /// Set the 7-day withdrawal lock for a provider (called on deposit).
    pub fn set_withdraw_lock(env: Env, provider: Address) -> Result<(), Error> {
        Self::assert_initialized(&env)?;
        provider.require_auth();
        let unlock = env
            .ledger()
            .timestamp()
            .checked_add(WITHDRAW_LOCK_SECS)
            .expect("overflow");
        env.storage()
            .persistent()
            .set(&DataKey::WithdrawUnlock(provider), &unlock);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // View functions
    // -----------------------------------------------------------------------

    /// Returns pool_balance / total_active_escrow_value in basis points.
    /// Returns 0 if no active escrow value is set.
    pub fn get_coverage_ratio(env: Env) -> u32 {
        let pool: i128 = env.storage().instance().get(&DataKey::PoolBalance).unwrap_or(0);
        let active: i128 = env.storage().instance().get(&DataKey::TotalActiveEscrow).unwrap_or(0);
        if active == 0 {
            return 0;
        }
        let ratio = pool.checked_mul(10_000).expect("overflow") / active;
        ratio as u32
    }

    pub fn get_pool_balance(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::PoolBalance).unwrap_or(0)
    }

    pub fn get_total_claims_paid(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::TotalClaimsPaid).unwrap_or(0)
    }

    pub fn get_provider_shares(env: Env, provider: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::ProviderShares(provider))
            .unwrap_or(0)
    }

    pub fn is_coverage_low(env: Env) -> bool {
        let ratio = Self::get_coverage_ratio(env);
        ratio > 0 && ratio < COVERAGE_ALERT_BPS
    }

    /// Calculate the premium for a given escrow amount and premium_bps.
    /// Returns 0 if premium_bps is 0.
    pub fn calculate_premium(_env: Env, escrow_amount: i128, premium_bps: u32) -> i128 {
        if premium_bps == 0 {
            return 0;
        }
        escrow_amount
            .checked_mul(premium_bps as i128)
            .expect("overflow")
            / 10_000
    }

    // -----------------------------------------------------------------------
    // Internal
    // -----------------------------------------------------------------------

    fn assert_initialized(env: &Env) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{Env, Symbol};

    // Minimal mock token for testing
    mod mock_token {
        use soroban_sdk::{contract, contractimpl, contracttype, Address, Env};

        #[contracttype]
        pub enum DataKey {
            Balance(Address),
        }

        #[contract]
        pub struct MockToken;

        #[contractimpl]
        impl MockToken {
            pub fn mint(env: Env, to: Address, amount: i128) {
                let bal: i128 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::Balance(to.clone()))
                    .unwrap_or(0);
                env.storage()
                    .persistent()
                    .set(&DataKey::Balance(to), &(bal + amount));
            }

            pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
                from.require_auth();
                let from_bal: i128 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::Balance(from.clone()))
                    .unwrap_or(0);
                let to_bal: i128 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::Balance(to.clone()))
                    .unwrap_or(0);
                env.storage()
                    .persistent()
                    .set(&DataKey::Balance(from), &(from_bal - amount));
                env.storage()
                    .persistent()
                    .set(&DataKey::Balance(to), &(to_bal + amount));
            }

            pub fn balance(env: Env, id: Address) -> i128 {
                env.storage()
                    .persistent()
                    .get(&DataKey::Balance(id))
                    .unwrap_or(0)
            }
        }
    }

    use mock_token::MockToken;

    struct Fixture {
        env:      Env,
        contract: Address,
        token:    Address,
        admin:    Address,
        provider: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set_timestamp(0);

            let admin = Address::generate(&env);
            let provider = Address::generate(&env);
            let token = env.register_contract(None, MockToken);
            let contract = env.register_contract(None, InsuranceContract);

            // Mint tokens to provider
            mock_token::MockTokenClient::new(&env, &token).mint(&provider, &1_000_000);

            InsuranceContractClient::new(&env, &contract).initialize(&admin, &token);

            Fixture { env, contract, token, admin, provider }
        }

        fn client(&self) -> InsuranceContractClient {
            InsuranceContractClient::new(&self.env, &self.contract)
        }
    }

    #[test]
    fn test_deposit() {
        let f = Fixture::setup();
        f.client().deposit(&f.provider, &500_000);
        assert_eq!(f.client().get_pool_balance(), 500_000);
        assert_eq!(f.client().get_provider_shares(&f.provider), 500_000);
    }

    #[test]
    fn test_withdraw_lock_enforced() {
        let f = Fixture::setup();
        f.client().deposit(&f.provider, &500_000);

        // Set lock
        f.client().set_withdraw_lock(&f.provider);

        // Attempt withdraw immediately — should fail
        assert_eq!(
            f.client().try_withdraw(&f.provider, &100_000),
            Err(Ok(Error::WithdrawLocked))
        );

        // Advance time past lock
        f.env.ledger().set_timestamp(WITHDRAW_LOCK_SECS + 1);
        f.client().withdraw(&f.provider, &100_000);
        assert_eq!(f.client().get_pool_balance(), 400_000);
        assert_eq!(f.client().get_provider_shares(&f.provider), 400_000);
    }

    #[test]
    fn test_claim_pays_learner() {
        let f = Fixture::setup();
        f.client().deposit(&f.provider, &500_000);

        let learner = Address::generate(&f.env);
        let escrow_id = Symbol::new(&f.env, "session1");

        f.client().claim(&escrow_id, &learner, &200_000);

        assert_eq!(f.client().get_pool_balance(), 300_000);
        assert_eq!(f.client().get_total_claims_paid(), 200_000);

        // Verify learner received tokens
        let learner_bal = mock_token::MockTokenClient::new(&f.env, &f.token).balance(&learner);
        assert_eq!(learner_bal, 200_000);
    }

    #[test]
    fn test_claim_insufficient_pool() {
        let f = Fixture::setup();
        f.client().deposit(&f.provider, &100_000);

        let learner = Address::generate(&f.env);
        let escrow_id = Symbol::new(&f.env, "session2");

        assert_eq!(
            f.client().try_claim(&escrow_id, &learner, &200_000),
            Err(Ok(Error::InsufficientPoolBalance))
        );
    }

    #[test]
    fn test_coverage_ratio() {
        let f = Fixture::setup();
        f.client().deposit(&f.provider, &500_000);
        f.client().set_active_escrow_value(&1_000_000);

        // 500_000 / 1_000_000 = 50% = 5000 bps
        assert_eq!(f.client().get_coverage_ratio(), 5000);
        assert!(!f.client().is_coverage_low());
    }

    #[test]
    fn test_coverage_low_alert() {
        let f = Fixture::setup();
        f.client().deposit(&f.provider, &40_000);
        f.client().set_active_escrow_value(&1_000_000);

        // 40_000 / 1_000_000 = 4% = 400 bps < 500 bps threshold
        assert_eq!(f.client().get_coverage_ratio(), 400);
        assert!(f.client().is_coverage_low());
    }

    #[test]
    fn test_zero_amount_rejected() {
        let f = Fixture::setup();
        assert_eq!(
            f.client().try_deposit(&f.provider, &0),
            Err(Ok(Error::ZeroAmount))
        );
    }
}

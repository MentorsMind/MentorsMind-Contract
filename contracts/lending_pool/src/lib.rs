#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    UsdcToken,
    CreditScoreContract,
    TotalLiquidity,
    TotalLpTokens,
    LenderBalance(Address),
    Loan(Address),
    LoanCount,
    /// Ledger sequence at which a lender last deposited.
    LenderDepositLedger(Address),
    /// Running borrow total for an address within the current ledger sequence.
    BlockBorrowTotal(Address),
    /// Ledger sequence recorded when per-block borrow total was last written.
    BlockBorrowLedger(Address),
    /// Snapshot of total liquidity at the start of the current ledger sequence.
    BlockLiquiditySnapshot,
    /// Ledger sequence when the liquidity snapshot was taken.
    BlockLiquidityLedger,
    /// Two-slope rate model parameters
    RateModelBaseRateBps,      // base_rate in bps
    RateModelKinkBps,          // kink utilization in bps
    RateModelSlope1Bps,        // slope1 below kink in bps
    RateModelSlope2Bps,        // slope2 above kink in bps
}

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LoanRecord {
    pub borrower: Address,
    pub amount: i128,
    pub fee: i128,
    pub session_id: Symbol,
    pub borrowed_at: u64,
    pub due_at: u64,
    pub repaid: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LenderRecord {
    pub lender: Address,
    pub lp_tokens: i128,
    pub deposited_at: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[soroban_sdk::contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    InsufficientBalance = 3,
    InsufficientLiquidity = 4,
    LowCreditScore = 5,
    BorrowLimitExceeded = 6,
    LoanNotFound = 7,
    LoanAlreadyRepaid = 8,
    NotAdmin = 9,
    InvalidAmount = 10,
    SameBlockDepositWithdraw = 11,
    PerBlockBorrowLimitExceeded = 12,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default rate model parameters (bps = basis points, 10_000 bps = 100%)
const DEFAULT_BASE_RATE_BPS: i128 = 100;        // 1% base rate
const DEFAULT_KINK_BPS: i128 = 8_000;           // 80% utilization kink
const DEFAULT_SLOPE1_BPS: i128 = 400;           // 4% slope below kink
const DEFAULT_SLOPE2_BPS: i128 = 3_000;         // 30% slope above kink

const MIN_CREDIT_SCORE: u32 = 600;
const LIQUIDATION_DAYS: u64 = 30;
const LIQUIDATION_SECONDS: u64 = LIQUIDATION_DAYS * 86_400;

/// Maximum amount a single address may borrow within one ledger sequence.
/// Set to 10% of the pool's liquidity snapshot; enforced dynamically.
const PER_BLOCK_BORROW_CAP_BPS: i128 = 1_000; // 10 %

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct LendingPool;

#[contractimpl]
impl LendingPool {
    /// Initialize the lending pool with default rate model
    pub fn initialize(
        env: Env,
        admin: Address,
        usdc_token: Address,
        credit_score_contract: Address,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::UsdcToken, &usdc_token);
        env.storage()
            .instance()
            .set(&DataKey::CreditScoreContract, &credit_score_contract);
        env.storage()
            .instance()
            .set(&DataKey::TotalLiquidity, &0i128);
        env.storage()
            .instance()
            .set(&DataKey::TotalLpTokens, &0i128);

        // Initialize rate model with defaults
        env.storage().instance().set(&DataKey::RateModelBaseRateBps, &DEFAULT_BASE_RATE_BPS);
        env.storage().instance().set(&DataKey::RateModelKinkBps, &DEFAULT_KINK_BPS);
        env.storage().instance().set(&DataKey::RateModelSlope1Bps, &DEFAULT_SLOPE1_BPS);
        env.storage().instance().set(&DataKey::RateModelSlope2Bps, &DEFAULT_SLOPE2_BPS);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Rate Model Admin
    // -----------------------------------------------------------------------

    /// Update rate model parameters (admin only)
    pub fn set_rate_model(
        env: Env,
        base_rate_bps: i128,
        kink_bps: i128,
        slope1_bps: i128,
        slope2_bps: i128,
    ) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        // Validate ranges
        if base_rate_bps < 0 || base_rate_bps > 10_000 {
            panic!("base_rate_bps must be 0-10000");
        }
        if kink_bps < 0 || kink_bps > 10_000 {
            panic!("kink_bps must be 0-10000");
        }
        if slope1_bps < 0 || slope1_bps > 10_000 {
            panic!("slope1_bps must be 0-10000");
        }
        if slope2_bps < 0 || slope2_bps > 10_000 {
            panic!("slope2_bps must be 0-10000");
        }

        env.storage().instance().set(&DataKey::RateModelBaseRateBps, &base_rate_bps);
        env.storage().instance().set(&DataKey::RateModelKinkBps, &kink_bps);
        env.storage().instance().set(&DataKey::RateModelSlope1Bps, &slope1_bps);
        env.storage().instance().set(&DataKey::RateModelSlope2Bps, &slope2_bps);

        Ok(())
    }

    /// Get current interest rate based on pool utilization
    /// Implements two-slope model:
    /// - Below kink: rate = base_rate + (utilization / kink) * slope1
    /// - Above kink: rate = base_rate + slope1 + ((utilization - kink) / (1 - kink)) * slope2
    pub fn get_current_rate(env: Env) -> i128 {
        let total_liquidity: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLiquidity)
            .unwrap_or(0);

        let total_borrowed: i128 = env
            .storage()
            .instance()
            .get(&DataKey::LoanCount)
            .unwrap_or(0i32) as i128;

        // Utilization = borrowed / (borrowed + available)
        let total_supply = total_borrowed + total_liquidity;
        if total_supply == 0 {
            return env
                .storage()
                .instance()
                .get(&DataKey::RateModelBaseRateBps)
                .unwrap_or(DEFAULT_BASE_RATE_BPS);
        }

        let utilization_bps = total_borrowed
            .checked_mul(10_000)
            .unwrap_or(i128::MAX)
            .checked_div(total_supply)
            .unwrap_or(0);

        let base_rate = env
            .storage()
            .instance()
            .get(&DataKey::RateModelBaseRateBps)
            .unwrap_or(DEFAULT_BASE_RATE_BPS);
        let kink = env
            .storage()
            .instance()
            .get(&DataKey::RateModelKinkBps)
            .unwrap_or(DEFAULT_KINK_BPS);
        let slope1 = env
            .storage()
            .instance()
            .get(&DataKey::RateModelSlope1Bps)
            .unwrap_or(DEFAULT_SLOPE1_BPS);
        let slope2 = env
            .storage()
            .instance()
            .get(&DataKey::RateModelSlope2Bps)
            .unwrap_or(DEFAULT_SLOPE2_BPS);

        if utilization_bps <= kink {
            // Below kink: rate = base_rate + (utilization / kink) * slope1
            let rate_increase = utilization_bps
                .checked_mul(slope1)
                .unwrap_or(i128::MAX)
                .checked_div(kink)
                .unwrap_or(0);
            base_rate.checked_add(rate_increase).unwrap_or(i128::MAX)
        } else {
            // Above kink: rate = base_rate + slope1 + ((utilization - kink) / (1 - kink)) * slope2
            let excess_util = utilization_bps - kink;
            let denominator = 10_000 - kink;
            let rate_increase_2 = excess_util
                .checked_mul(slope2)
                .unwrap_or(i128::MAX)
                .checked_div(denominator)
                .unwrap_or(0);
            base_rate
                .checked_add(slope1)
                .unwrap_or(i128::MAX)
                .checked_add(rate_increase_2)
                .unwrap_or(i128::MAX)
        }
    }

    /// Pure fee computation (replaces cached version)
    /// fee = amount * current_rate / 10_000
    fn compute_fee(env: &Env, amount: i128) -> i128 {
        let rate = Self::get_current_rate(env.clone());
        amount
            .checked_mul(rate)
            .expect("Overflow")
            .checked_div(10_000)
            .expect("Division error")
    }

    // -----------------------------------------------------------------------
    // Core Lending Functions (unchanged except fee computation)
    // -----------------------------------------------------------------------

    pub fn deposit(env: Env, lender: Address, amount: i128) -> Result<i128, Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        lender.require_auth();

        let usdc_token: Address = env.storage().instance().get(&DataKey::UsdcToken).unwrap();
        let token_client = soroban_sdk::token::Client::new(&env, &usdc_token);

        token_client.transfer(&lender, &env.current_contract_address(), &amount);

        let lp_tokens = amount;

        let mut total_liquidity: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLiquidity)
            .unwrap_or(0);
        total_liquidity = total_liquidity.checked_add(amount).expect("Overflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalLiquidity, &total_liquidity);

        let mut total_lp: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLpTokens)
            .unwrap_or(0);
        total_lp = total_lp.checked_add(lp_tokens).expect("Overflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalLpTokens, &total_lp);

        let mut lender_balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::LenderBalance(lender.clone()))
            .unwrap_or(0);
        lender_balance = lender_balance.checked_add(lp_tokens).expect("Overflow");
        env.storage()
            .persistent()
            .set(&DataKey::LenderBalance(lender.clone()), &lender_balance);

        env.events()
            .publish((symbol_short!("deposited"),), (lender.clone(), amount, lp_tokens));

        env.storage().instance().set(
            &DataKey::LenderDepositLedger(lender),
            &env.ledger().sequence(),
        );

        Ok(lp_tokens)
    }

    pub fn withdraw(env: Env, lender: Address, lp_amount: i128) -> Result<i128, Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        if lp_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        lender.require_auth();

        let deposit_ledger: u32 = env
            .storage()
            .instance()
            .get(&DataKey::LenderDepositLedger(lender.clone()))
            .unwrap_or(0);
        if deposit_ledger == env.ledger().sequence() {
            return Err(Error::SameBlockDepositWithdraw);
        }

        let lender_balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::LenderBalance(lender.clone()))
            .unwrap_or(0);

        if lender_balance < lp_amount {
            return Err(Error::InsufficientBalance);
        }

        let usdc_amount = lp_amount;

        let usdc_token: Address = env.storage().instance().get(&DataKey::UsdcToken).unwrap();
        let token_client = soroban_sdk::token::Client::new(&env, &usdc_token);

        token_client.transfer(&env.current_contract_address(), &lender, &usdc_amount);

        let mut total_liquidity: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLiquidity)
            .unwrap_or(0);
        total_liquidity = total_liquidity.checked_sub(usdc_amount).expect("Underflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalLiquidity, &total_liquidity);

        let mut total_lp: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLpTokens)
            .unwrap_or(0);
        total_lp = total_lp.checked_sub(lp_amount).expect("Underflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalLpTokens, &total_lp);

        let new_balance = lender_balance.checked_sub(lp_amount).expect("Underflow");
        if new_balance == 0 {
            env.storage()
                .persistent()
                .remove(&DataKey::LenderBalance(lender.clone()));
        } else {
            env.storage()
                .persistent()
                .set(&DataKey::LenderBalance(lender.clone()), &new_balance);
        }

        env.events().publish(
            (symbol_short!("withdrawn"),),
            (lender, lp_amount, usdc_amount),
        );

        Ok(usdc_amount)
    }

    pub fn borrow(
        env: Env,
        borrower: Address,
        amount: i128,
        session_id: Symbol,
    ) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        borrower.require_auth();

        let total_liquidity: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLiquidity)
            .unwrap_or(0);

        if total_liquidity < amount {
            return Err(Error::InsufficientLiquidity);
        }

        let current_seq = env.ledger().sequence();

        let snap_ledger: u32 = env
            .storage()
            .instance()
            .get(&DataKey::BlockLiquidityLedger)
            .unwrap_or(0);
        let liquidity_snapshot: i128 = if snap_ledger == current_seq {
            env.storage()
                .instance()
                .get(&DataKey::BlockLiquiditySnapshot)
                .unwrap_or(total_liquidity)
        } else {
            env.storage()
                .instance()
                .set(&DataKey::BlockLiquiditySnapshot, &total_liquidity);
            env.storage()
                .instance()
                .set(&DataKey::BlockLiquidityLedger, &current_seq);
            total_liquidity
        };

        let per_block_cap = liquidity_snapshot
            .checked_mul(PER_BLOCK_BORROW_CAP_BPS)
            .unwrap_or(i128::MAX)
            .checked_div(10_000)
            .unwrap_or(i128::MAX);

        let borrow_ledger: u32 = env
            .storage()
            .instance()
            .get(&DataKey::BlockBorrowLedger(borrower.clone()))
            .unwrap_or(0);
        let block_total: i128 = if borrow_ledger == current_seq {
            env.storage()
                .instance()
                .get(&DataKey::BlockBorrowTotal(borrower.clone()))
                .unwrap_or(0)
        } else {
            0
        };

        let new_block_total = block_total.checked_add(amount).unwrap_or(i128::MAX);
        if new_block_total > per_block_cap {
            return Err(Error::PerBlockBorrowLimitExceeded);
        }

        env.storage()
            .instance()
            .set(&DataKey::BlockBorrowTotal(borrower.clone()), &new_block_total);
        env.storage()
            .instance()
            .set(&DataKey::BlockBorrowLedger(borrower.clone()), &current_seq);

        // Compute fee using dynamic rate model (no cache)
        let fee = Self::compute_fee(&env, amount);

        let usdc_token: Address = env.storage().instance().get(&DataKey::UsdcToken).unwrap();
        let token_client = soroban_sdk::token::Client::new(&env, &usdc_token);
        token_client.transfer(&env.current_contract_address(), &borrower, &amount);

        let now = env.ledger().timestamp();
        let loan = LoanRecord {
            borrower: borrower.clone(),
            amount,
            fee,
            session_id: session_id.clone(),
            borrowed_at: now,
            due_at: now.checked_add(LIQUIDATION_SECONDS).expect("Timestamp overflow"),
            repaid: false,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Loan(borrower.clone()), &loan);

        let new_liquidity = total_liquidity.checked_sub(amount).expect("Underflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalLiquidity, &new_liquidity);

        env.events().publish(
            (symbol_short!("borrowed"),),
            (borrower, amount, fee, session_id),
        );

        Ok(())
    }

    pub fn repay(env: Env, borrower: Address, amount: i128) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        borrower.require_auth();

        let loan: LoanRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Loan(borrower.clone()))
            .ok_or(Error::LoanNotFound)?;

        if loan.repaid {
            return Err(Error::LoanAlreadyRepaid);
        }

        let total_owed = loan.amount + loan.fee;
        if amount < total_owed {
            return Err(Error::InvalidAmount);
        }

        let usdc_token: Address = env.storage().instance().get(&DataKey::UsdcToken).unwrap();
        let token_client = soroban_sdk::token::Client::new(&env, &usdc_token);
        token_client.transfer(&borrower, &env.current_contract_address(), &total_owed);

        let mut updated_loan = loan.clone();
        updated_loan.repaid = true;
        env.storage()
            .persistent()
            .set(&DataKey::Loan(borrower.clone()), &updated_loan);

        let mut total_liquidity: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLiquidity)
            .unwrap_or(0);
        total_liquidity = total_liquidity.checked_add(total_owed).expect("Overflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalLiquidity, &total_liquidity);

        env.events().publish(
            (symbol_short!("repaid"),),
            (borrower, loan.amount, loan.fee),
        );

        Ok(())
    }

    pub fn get_loan(env: Env, borrower: Address) -> Result<LoanRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Loan(borrower))
            .ok_or(Error::LoanNotFound)
    }

    pub fn get_block_borrow_total(env: Env, borrower: Address) -> i128 {
        let current_seq = env.ledger().sequence();
        let borrow_ledger: u32 = env
            .storage()
            .instance()
            .get(&DataKey::BlockBorrowLedger(borrower.clone()))
            .unwrap_or(0);
        if borrow_ledger == current_seq {
            env.storage()
                .instance()
                .get(&DataKey::BlockBorrowTotal(borrower))
                .unwrap_or(0)
        } else {
            0
        }
    }

    pub fn get_liquidity_snapshot(env: Env) -> i128 {
        let current_seq = env.ledger().sequence();
        let snap_ledger: u32 = env
            .storage()
            .instance()
            .get(&DataKey::BlockLiquidityLedger)
            .unwrap_or(0);
        if snap_ledger == current_seq {
            env.storage()
                .instance()
                .get(&DataKey::BlockLiquiditySnapshot)
                .unwrap_or(0)
        } else {
            env.storage()
                .instance()
                .get(&DataKey::TotalLiquidity)
                .unwrap_or(0)
        }
    }

    pub fn total_liquidity(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalLiquidity)
            .unwrap_or(0)
    }

    pub fn lender_balance(env: Env, lender: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::LenderBalance(lender))
            .unwrap_or(0)
    }

    pub fn liquidate(env: Env, borrower: Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        let loan: LoanRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Loan(borrower.clone()))
            .ok_or(Error::LoanNotFound)?;

        if loan.repaid {
            return Err(Error::LoanAlreadyRepaid);
        }

        let now = env.ledger().timestamp();
        if now <= loan.due_at {
            panic!("loan not yet due");
        }

        let mut updated_loan = loan.clone();
        updated_loan.repaid = true;
        env.storage()
            .persistent()
            .set(&DataKey::Loan(borrower.clone()), &updated_loan);

        env.events()
            .publish((symbol_short!("liq"),), (borrower, loan.amount));

        Ok(())
    }

    pub fn accrue_yield(env: Env, admin: Address, amount: i128) -> Result<(), Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        if stored_admin != admin {
            return Err(Error::NotAdmin);
        }

        let mut total_liquidity: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLiquidity)
            .unwrap_or(0);
        total_liquidity = total_liquidity.checked_add(amount).expect("Overflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalLiquidity, &total_liquidity);

        env.events()
            .publish((symbol_short!("yield"), symbol_short!("accrue")), amount);
        Ok(())
    }

    pub fn distribute_yield(env: Env, admin: Address, lender: Address, amount: i128) -> Result<(), Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        if stored_admin != admin {
            return Err(Error::NotAdmin);
        }

        let mut lender_balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::LenderBalance(lender.clone()))
            .unwrap_or(0);
        lender_balance = lender_balance.checked_add(amount).expect("Overflow");
        env.storage()
            .persistent()
            .set(&DataKey::LenderBalance(lender.clone()), &lender_balance);

        let mut total_lp: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLpTokens)
            .unwrap_or(0);
        total_lp = total_lp.checked_add(amount).expect("Overflow");
        env.storage().instance().set(&DataKey::TotalLpTokens, &total_lp);

        env.events().publish(
            (symbol_short!("yield"), symbol_short!("dist")),
            (lender, amount),
        );
        Ok(())
    }
}

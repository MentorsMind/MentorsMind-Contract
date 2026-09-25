#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// External contract interface: Credit Score
//
// The credit score contract exposes `get_score(env, address) -> u32`. We
// describe it here as a trait so the SDK generates a strongly-typed
// `CreditScoreClient` used for the cross-contract call in `borrow`.
// ---------------------------------------------------------------------------

#[contractclient(name = "CreditScoreClient")]
pub trait CreditScoreContractTrait {
    fn get_score(env: Env, address: Address) -> u32;
}

use shared::{
    get_all_params, get_param, init_protocol_params, set_param,
    key_interest_rate_bps, key_min_credit_score,
    DEFAULT_INTEREST_RATE_BPS, DEFAULT_MIN_CREDIT_SCORE,
};

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
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
    /// Minimum credit score required to borrow (defaults to MIN_CREDIT_SCORE).
    MinCreditScore,
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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuctionRecord {
    pub loan: LoanRecord,
    pub started_at: u64,
    pub discount_bps: i128,
    pub liquidator: Option<Address>,
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
    LoanNotDue = 13,
    AuctionNotFound = 14,
    AuctionAlreadyActive = 15,
    AuctionAlreadySettled = 16,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default rate model parameters (bps = basis points, 10_000 bps = 100%)
const DEFAULT_BASE_RATE_BPS: i128 = 100;        // 1% base rate
const DEFAULT_KINK_BPS: i128 = 8_000;           // 80% utilization kink
const DEFAULT_SLOPE1_BPS: i128 = 400;           // 4% slope below kink
const DEFAULT_SLOPE2_BPS: i128 = 3_000;         // 30% slope above kink

const LIQUIDATION_DAYS: u64 = 30;
const LIQUIDATION_SECONDS: u64 = LIQUIDATION_DAYS * 86_400;

/// Maximum amount a single address may borrow within one ledger sequence.
/// Set to 10% of the pool's liquidity snapshot; enforced dynamically.
const PER_BLOCK_BORROW_CAP_BPS: i128 = 1_000; // 10 %

/// Dutch-auction liquidation window: discount grows linearly from 0% to
/// MAX_AUCTION_DISCOUNT_BPS over AUCTION_DURATION_SECS after the auction starts.
const AUCTION_DURATION_SECS: u64 = 24 * 60 * 60; // 24 hours
const MAX_AUCTION_DISCOUNT_BPS: i128 = 2_000; // 20%

// ---------------------------------------------------------------------------
// TTL constants for flash-loan guard entries (persistent storage)
// ---------------------------------------------------------------------------

/// Retention period for flash-loan guard entries: 7 days in ledgers
/// (assuming ~5s per ledger).  This is the maximum time a ledger-guard
/// entry should be kept before it is eligible for archival.
const LEDGER_GUARD_TTL: u32 = 120_960; // 7 days at 5s/ledger
/// TTL threshold: when remaining lifetime drops below this many ledgers,
/// extend the TTL.  500k ledgers ≈ 29 days at 5s/ledger.
const LEDGER_GUARD_TTL_THRESHOLD: u32 = 500_000;
/// TTL bump amount in ledgers: extend lifetime by this amount.
/// 1_209_600 ledgers ≈ 70 days (10 weeks) at 5s/ledger — well beyond the
/// 7-day instance TTL default, guaranteeing the flash-loan guard cannot
/// be silently expired.
const LEDGER_GUARD_TTL_BUMP: u32 = 1_209_600;

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
        rbac_contract: Address,
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
        
        // Initialize regulatory reporting with placeholder
        env.storage().instance().set(&DataKey::RegulatoryReporting, &Address::generate(&env));

        init_protocol_params(&env, &rbac_contract);
        Ok(())
    }

    /// Set regulatory reporting contract address (admin only).
    pub fn set_regulatory_reporting(env: Env, reporting_address: Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::RegulatoryReporting, &reporting_address);
        Ok(())
    }

    fn _check_and_report_large_tx(
        env: &Env,
        contract: Symbol,
        function: Symbol,
        address: &Address,
        amount_usd: i128,
    ) {
        const THRESHOLD: i128 = 10_000;
        if amount_usd <= THRESHOLD {
            return;
        }

        if let Some(reporting_addr) = env
            .storage()
            .instance()
            .get::<DataKey, Address>(&DataKey::RegulatoryReporting)
        {
            use soroban_sdk::IntoVal;
            let _ = env.try_invoke_contract::<(), _>(
                &reporting_addr,
                &Symbol::new(env, "record_large_tx"),
                (
                    contract,
                    function,
                    address.clone(),
                    amount_usd,
                    env.ledger().timestamp(),
                )
                    .into_val(env),
            );
        }
    }

    // -----------------------------------------------------------------------
    // Protocol parameter registry
    // -----------------------------------------------------------------------

    /// Read a protocol parameter by key, with compile-time default fallback.
    pub fn get_param(env: Env, key: Symbol, default: i128) -> i128 {
        get_param(&env, &key, default)
    }

    /// Update a protocol parameter. Caller must hold `GOVERNANCE_ADMIN`.
    pub fn set_param(env: Env, caller: Address, key: Symbol, value: i128) {
        set_param(&env, &caller, &key, value);
    }

    /// Return all current `(Symbol, i128)` parameter pairs for monitoring.
    pub fn get_all_params(env: Env) -> Vec<(Symbol, i128)> {
        get_all_params(&env)
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

    /// Update the minimum credit score required to borrow (admin only).
    pub fn set_min_credit_score(env: Env, admin: Address, new_min: u32) -> Result<(), Error> {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        if stored_admin != admin {
            return Err(Error::NotAdmin);
        }

        env.storage()
            .instance()
            .set(&DataKey::MinCreditScore, &new_min);

        env.events().publish(
            (symbol_short!("min_score"),),
            new_min,
        );

        Ok(())
    }

    /// Get the minimum credit score required to borrow (defaults to 600).
    pub fn get_min_credit_score(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::MinCreditScore)
            .unwrap_or(MIN_CREDIT_SCORE)
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
    ///
    /// The interest rate is read from the protocol parameter registry first,
    /// falling back to `DEFAULT_INTEREST_RATE_BPS` if governance hasn't acted.
    fn compute_fee(env: &Env, amount: i128) -> i128 {
        // Use the governance-controlled interest rate if set, else the two-slope
        // dynamic model rate.  Governance sets a flat override via INT_RATE;
        // when that key is unset (== DEFAULT_INTEREST_RATE_BPS still at default),
        // we fall through to the full model.
        let gov_rate = env
            .storage()
            .persistent()
            .get::<_, i128>(&shared::params::ParamKey::Param(key_interest_rate_bps()))
            .unwrap_or(0);
        let rate = if gov_rate > 0 { gov_rate } else { Self::get_current_rate(env.clone()) };
        amount
            .checked_mul(rate)
            .expect("Overflow")
            .checked_div(10_000)
            .expect("Division error")
    }

    /// Returns the minimum credit score required to borrow, sourced from
    /// the protocol parameter registry with compile-time fallback.
    pub fn min_credit_score(env: Env) -> i128 {
        get_param(&env, &key_min_credit_score(), DEFAULT_MIN_CREDIT_SCORE)
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
            .persistent()
            .get(&deposit_ledger_key)
            .unwrap_or(0);
        if deposit_ledger != 0 {
            env.storage().persistent().extend_ttl(
                &deposit_ledger_key,
                LEDGER_GUARD_TTL_THRESHOLD,
                LEDGER_GUARD_TTL_BUMP,
            );
        }
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

        // --- Credit score gate ---
        // Query the borrower's on-chain credit score via a cross-contract call
        // and reject the borrow if it is below the configured minimum.
        let credit_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::CreditScoreContract)
            .ok_or(Error::NotInitialized)?;
        let min_credit_score: u32 = env
            .storage()
            .instance()
            .get(&DataKey::MinCreditScore)
            .unwrap_or(MIN_CREDIT_SCORE);
        let credit_score = CreditScoreClient::new(&env, &credit_contract).get_score(&borrower);
        if credit_score < min_credit_score {
            return Err(Error::LowCreditScore);
        }

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
            .persistent()
            .get(&borrow_ledger_key)
            .unwrap_or(0);
        if borrow_ledger != 0 {
            env.storage().persistent().extend_ttl(
                &borrow_ledger_key,
                LEDGER_GUARD_TTL_THRESHOLD,
                LEDGER_GUARD_TTL_BUMP,
            );
        }

        let borrow_total_key = DataKey::BlockBorrowTotal(borrower.clone());
        let block_total: i128 = if borrow_ledger == current_seq {
            let total: i128 = env
                .storage()
                .persistent()
                .get(&borrow_total_key)
                .unwrap_or(0);
            if total != 0 {
                env.storage().persistent().extend_ttl(
                    &borrow_total_key,
                    LEDGER_GUARD_TTL_THRESHOLD,
                    LEDGER_GUARD_TTL_BUMP,
                );
            }
            total
        } else {
            0
        };

        let new_block_total = block_total.checked_add(amount).unwrap_or(i128::MAX);
        if new_block_total > per_block_cap {
            return Err(Error::PerBlockBorrowLimitExceeded);
        }

        env.storage()
            .persistent()
            .set(&borrow_total_key, &new_block_total);
        env.storage().persistent().extend_ttl(
            &borrow_total_key,
            LEDGER_GUARD_TTL_THRESHOLD,
            LEDGER_GUARD_TTL_BUMP,
        );
        env.storage()
            .persistent()
            .set(&borrow_ledger_key, &current_seq);
        env.storage().persistent().extend_ttl(
            &borrow_ledger_key,
            LEDGER_GUARD_TTL_THRESHOLD,
            LEDGER_GUARD_TTL_BUMP,
        );

        // Check for large transaction and trigger regulatory reporting
        Self::_check_and_report_large_tx(&env, symbol_short!("lend_pool"), symbol_short!("borrow"), &borrower, amount);

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
        let borrow_ledger_key = DataKey::BlockBorrowLedger(borrower.clone());
        let borrow_ledger: u32 = env
            .storage()
            .persistent()
            .get(&borrow_ledger_key)
            .unwrap_or(0);
        if borrow_ledger != 0 {
            env.storage().persistent().extend_ttl(
                &borrow_ledger_key,
                LEDGER_GUARD_TTL_THRESHOLD,
                LEDGER_GUARD_TTL_BUMP,
            );
        }
        if borrow_ledger == current_seq {
            let borrow_total_key = DataKey::BlockBorrowTotal(borrower);
            let total: i128 = env
                .storage()
                .persistent()
                .get(&borrow_total_key)
                .unwrap_or(0);
            if total != 0 {
                env.storage().persistent().extend_ttl(
                    &borrow_total_key,
                    LEDGER_GUARD_TTL_THRESHOLD,
                    LEDGER_GUARD_TTL_BUMP,
                );
            }
            total
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

    // -----------------------------------------------------------------------
    // Liquidation auction (#667)
    // -----------------------------------------------------------------------

    /// Starts a Dutch-auction liquidation for `borrower`'s defaulted loan.
    /// Callable by anyone once `due_at` has passed. The discount grows
    /// linearly from 0% to MAX_AUCTION_DISCOUNT_BPS over AUCTION_DURATION_SECS.
    pub fn start_liquidation_auction(env: Env, borrower: Address) -> Result<(), Error> {
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
            return Err(Error::LoanNotDue);
        }

        if env
            .storage()
            .persistent()
            .has(&DataKey::LiquidationAuction(borrower.clone()))
        {
            return Err(Error::AuctionAlreadyActive);
        }

        let auction = AuctionRecord {
            loan,
            started_at: now,
            discount_bps: 0,
            liquidator: None,
        };
        env.storage()
            .persistent()
            .set(&DataKey::LiquidationAuction(borrower.clone()), &auction);

        env.events()
            .publish((symbol_short!("auc_start"),), (borrower, now));

        Ok(())
    }

    /// Current discount (bps) for `borrower`'s active auction, based on
    /// elapsed time since it started. Linear ramp: 0 at start, capped at
    /// MAX_AUCTION_DISCOUNT_BPS after AUCTION_DURATION_SECS have elapsed.
    pub fn get_auction_discount(env: Env, borrower: Address) -> Result<i128, Error> {
        let auction: AuctionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::LiquidationAuction(borrower))
            .ok_or(Error::AuctionNotFound)?;

        let now = env.ledger().timestamp();
        let elapsed = now.saturating_sub(auction.started_at);
        if elapsed >= AUCTION_DURATION_SECS {
            return Ok(MAX_AUCTION_DISCOUNT_BPS);
        }

        let discount = MAX_AUCTION_DISCOUNT_BPS
            .checked_mul(elapsed as i128)
            .expect("Overflow")
            .checked_div(AUCTION_DURATION_SECS as i128)
            .expect("Division error");
        Ok(discount)
    }

    /// Executes the liquidation: `liquidator` pays `loan.amount * (1 -
    /// discount_bps/10000)` and the pool's liquidity is restored by that
    /// amount. The liquidator's profit is the discount versus the full
    /// principal; no LP tokens are minted since the pool is simply repaid at
    /// a haircut instead of the borrower repaying at par.
    pub fn execute_liquidation(env: Env, liquidator: Address, borrower: Address) -> Result<(), Error> {
        liquidator.require_auth();

        let auction: AuctionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::LiquidationAuction(borrower.clone()))
            .ok_or(Error::AuctionNotFound)?;

        if auction.liquidator.is_some() {
            return Err(Error::AuctionAlreadySettled);
        }

        let discount_bps = Self::get_auction_discount(env.clone(), borrower.clone())?;

        let total_owed = auction.loan.amount + auction.loan.fee;
        let discount_amount = total_owed
            .checked_mul(discount_bps)
            .expect("Overflow")
            .checked_div(10_000)
            .expect("Division error");
        let payment = total_owed - discount_amount;

        let usdc_token: Address = env.storage().instance().get(&DataKey::UsdcToken).unwrap();
        let token_client = soroban_sdk::token::Client::new(&env, &usdc_token);
        token_client.transfer(&liquidator, &env.current_contract_address(), &payment);

        // Restore pool liquidity by the amount actually recovered.
        let mut total_liquidity: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalLiquidity)
            .unwrap_or(0);
        total_liquidity = total_liquidity.checked_add(payment).expect("Overflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalLiquidity, &total_liquidity);

        // Mark the loan repaid and settle the auction.
        let mut updated_loan = auction.loan.clone();
        updated_loan.repaid = true;
        env.storage()
            .persistent()
            .set(&DataKey::Loan(borrower.clone()), &updated_loan);
        env.storage()
            .persistent()
            .remove(&DataKey::LiquidationAuction(borrower.clone()));

        env.events().publish(
            (symbol_short!("auc_exec"),),
            (borrower, liquidator, payment, discount_bps),
        );

        Ok(())
    }

    /// Admin-only emergency liquidation that bypasses the Dutch auction.
    /// No funds are recovered from the borrower; the full outstanding
    /// principal + fee is written off to `DataKey::BadDebt` for solvency
    /// monitoring. Pool liquidity is NOT restored.
    pub fn force_liquidate(env: Env, admin: Address, borrower: Address) -> Result<(), Error> {
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        if admin != stored_admin {
            return Err(Error::NotAdmin);
        }

        let loan: LoanRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Loan(borrower.clone()))
            .ok_or(Error::LoanNotFound)?;

        if loan.repaid {
            return Err(Error::LoanAlreadyRepaid);
        }

        let mut updated_loan = loan.clone();
        updated_loan.repaid = true;
        env.storage()
            .persistent()
            .set(&DataKey::Loan(borrower.clone()), &updated_loan);
        env.storage()
            .persistent()
            .remove(&DataKey::LiquidationAuction(borrower.clone()));

        let total_owed = loan.amount + loan.fee;
        let mut bad_debt: i128 = env.storage().instance().get(&DataKey::BadDebt).unwrap_or(0);
        bad_debt = bad_debt.checked_add(total_owed).expect("Overflow");
        env.storage().instance().set(&DataKey::BadDebt, &bad_debt);

        env.events()
            .publish((symbol_short!("force_liq"),), (borrower, total_owed));

        Ok(())
    }

    /// Total unrecovered principal + fee written off via `force_liquidate`,
    /// for off-chain solvency monitoring.
    pub fn get_bad_debt(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::BadDebt).unwrap_or(0)
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

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    // --- Mock USDC token (mint / balance / transfer) ---
    #[contracttype]
    #[derive(Clone)]
    pub enum MockTokKey {
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
                .get(&MockTokKey::Balance(to.clone()))
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&MockTokKey::Balance(to), &(bal + amount));
        }
        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&MockTokKey::Balance(id))
                .unwrap_or(0)
        }
        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            let from_bal = Self::balance(env.clone(), from.clone());
            assert!(from_bal >= amount, "Insufficient balance");
            let to_bal = Self::balance(env.clone(), to.clone());
            env.storage()
                .persistent()
                .set(&MockTokKey::Balance(from), &(from_bal - amount));
            env.storage()
                .persistent()
                .set(&MockTokKey::Balance(to), &(to_bal + amount));
        }
    }

    // --- Mock CreditScore contract with a configurable global score ---
    #[contract]
    pub struct MockCreditScore;

    #[contractimpl]
    impl MockCreditScore {
        pub fn set_score(env: Env, score: u32) {
            env.storage().instance().set(&symbol_short!("score"), &score);
        }
        pub fn get_score(env: Env, _address: Address) -> u32 {
            env.storage()
                .instance()
                .get(&symbol_short!("score"))
                .unwrap_or(0u32)
        }
    }

    struct Fixture {
        env: Env,
        admin: Address,
        pool: LendingPoolClient<'static>,
        score: MockCreditScoreClient<'static>,
    }

    fn setup() -> Fixture {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);

        let token_id = env.register_contract(None, MockToken);
        let token = MockTokenClient::new(&env, &token_id);

        let score_id = env.register_contract(None, MockCreditScore);
        let score = MockCreditScoreClient::new(&env, &score_id);

        let pool_id = env.register_contract(None, LendingPool);
        let pool = LendingPoolClient::new(&env, &pool_id);
        pool.initialize(&admin, &token_id, &score_id);

        // Seed the pool with liquidity from a lender.
        let lender = Address::generate(&env);
        token.mint(&lender, &1_000_000);
        pool.deposit(&lender, &1_000_000);

        Fixture { env, admin, pool, score }
    }

    /// Attempt a borrow with a fresh borrower at the given credit score,
    /// asserting it is rejected with `LowCreditScore`.
    fn assert_borrow_rejected(f: &Fixture, score: u32) {
        f.score.set_score(&score);
        let borrower = Address::generate(&f.env);
        let result = f.pool.try_borrow(&borrower, &1_000, &symbol_short!("s1"));
        assert_eq!(result, Err(Ok(Error::LowCreditScore)));
    }

    /// Attempt a borrow with a fresh borrower at the given credit score,
    /// asserting it succeeds.
    fn assert_borrow_ok(f: &Fixture, score: u32) {
        f.score.set_score(&score);
        let borrower = Address::generate(&f.env);
        let result = f.pool.try_borrow(&borrower, &1_000, &symbol_short!("s1"));
        assert_eq!(result, Ok(Ok(())));
    }

    #[test]
    fn test_default_min_credit_score_is_600() {
        let f = setup();
        assert_eq!(f.pool.get_min_credit_score(), 600);
    }

    #[test]
    fn test_borrow_below_min_score_rejected() {
        let f = setup();
        assert_borrow_rejected(&f, 599); // 599 < 600
    }

    #[test]
    fn test_borrow_at_min_score_allowed() {
        let f = setup();
        assert_borrow_ok(&f, 600); // 600 == 600
    }

    #[test]
    fn test_borrow_above_min_score_allowed() {
        let f = setup();
        assert_borrow_ok(&f, 601); // 601 > 600
    }

    #[test]
    fn test_set_min_credit_score_changes_gate() {
        let f = setup();
        f.pool.set_min_credit_score(&f.admin, &700);
        assert_eq!(f.pool.get_min_credit_score(), 700);
        // 650 now fails against the raised minimum.
        assert_borrow_rejected(&f, 650);
        // 700 passes.
        assert_borrow_ok(&f, 700);
    }
}

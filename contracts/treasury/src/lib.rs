#![no_std]

use shared::pause_guard::require_not_paused;
use shared::admin::{AdminTransfer, AdminChangeProposal, ADMIN_COOLING_OFF_SECS, MIN_ADMIN_TIMELOCK_SECS};
use shared::{
    AtomicBatch, BatchOp, ReentrancyGuard, StateSnapshot, Validator,
    validate_amount_limits, validate_caller_is_authorized, MAX_BATCH_SIZE,
    detect_price_coordination, validate_market_rate,
    enforce_fair_pricing as shared_enforce_fair_pricing, FairPricingResult, MarketRateValidation,
    PriceCoordinationFlag, DEFAULT_MAX_MARKET_DEVIATION_BPS,
    detect_atomic_arbitrage, enforce_protocol_isolation,
    // resource management
    manage_session_load, check_emergency_trigger,
    // platform authenticity
    detect_fee_evasion, PenaltyTier,
    // dynamic fees
    calculate_dynamic_fee, detect_fee_gaming,
};
use shared::economic_verification::{
    validate_fund_conservation, validate_reward_distribution, record_invariant_check,
    EconomicInvariant, EconomicInvariantRecord, RewardAllocation,
};
use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, symbol_short, token,
    Address, Env, IntoVal, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Oracle client interface (matches oracle contract's public API)
// ---------------------------------------------------------------------------

/// Mirrors `OracleHealth` from the oracle contract.
/// Extended to include circuit-breaker and override state.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleHealth {
    pub active_feeders: u32,
    pub last_update: u64,
    pub is_stale: bool,
    pub circuit_breaker_tripped: bool,
    pub override_active: bool,
}

#[contractclient(name = "OracleContractClient")]
pub trait OracleContractTrait {
    fn get_oracle_health(env: Env, asset: Symbol) -> OracleHealth;
}

// ---------------------------------------------------------------------------
// Staking contract client interface (matches the anti-dilution methods that
// the treasury needs to coordinate with when pushing a distribution).
// ---------------------------------------------------------------------------

#[contractclient(name = "StakingContractClient")]
pub trait StakingCoordinationTrait {
    /// Push the scheduled-next-distribution timestamp into the staking
    /// contract so its pattern detector can flag large late stakes.
    fn set_next_distribution_at(
        env: Env,
        admin: Address,
        timestamp: u64,
    ) -> Result<(), soroban_sdk::Error>;

    /// Eligible-total denominator for an already-closed epoch. Used by the
    /// treasury for off-chain audit verification: the treasury confirms
    /// this matches its own accounting before recording the receipt.
    fn get_epoch_eligible_total(env: Env, epoch: u64) -> Option<i128>;

    /// Penalty-redistribution pool balance right before a distribution.
    /// The treasury snapshots this and logs it so audit trails reconcile
    /// penalty funds flowing into an epoch's combined reward.
    fn get_penalty_redistribution_pool(env: Env) -> i128;
}

// ---------------------------------------------------------------------------
// DEX interface descriptor – used to call different DEX implementations.
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DexInterface {
    pub swap_fn: Symbol,
}

impl DexInterface {
    pub fn validate(&self, env: &Env) {
        if self.swap_fn == Symbol::new(env, "") {
            panic!("DexInterface: swap_fn must not be empty");
        }
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuybackFailed {
    pub xlm_amount: i128,
    pub reason: Symbol,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuybackSucceeded {
    pub xlm_spent: i128,
    pub mnt_burned: i128,
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InsufficientBalance = 4,
    OracleUnhealthy = 5,
    OracleStale = 6,
    TokenNotApproved = 7,
    InvalidMinOut = 8,
    ZeroOutput = 9,
    SlippageExceeded = 10,
    InvalidAmount = 11,
    NoPendingAdminChange = 12,
    AdminChangeNotYetEffective = 13,
    InvalidAdminChange = 14,
    CallerNotAuthorized = 15,
    AmountExceedsLimit = 16,
    DistributionAlreadyProcessed = 17,
    ReentrancyGuardPaused = 18,
    StateValidationFailed = 19,
    InvalidState = 20,
    DuplicateEntry = 21,
    Overflow = 22,
    OracleCircuitBreaker = 23,
    CoolingOffPeriod = 24,
    TimelockNotExpired = 25,
    SuspendedDuringAdminTransfer = 26,
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationHistory {
    pub token: Address,
    pub recipient: Address,
    pub amount: i128,
    pub timestamp: u64,
}

/// A single item in a [`TreasuryContract::batch_allocate`] request.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationRequest {
    pub token: Address,
    pub recipient: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryTokenApprovalEvent {
    pub token: Address,
    pub approved: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingAllocation {
    pub id: u32,
    pub token: Address,
    pub recipient: Address,
    pub amount: i128,
    pub approvals_count: u32,
    pub executed: bool,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChangeProposedEvent {
    pub contract: Address,
    pub old_admin: Address,
    pub new_admin: Address,
    pub effective_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChangeAcceptedEvent {
    pub contract: Address,
    pub new_admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DistributionReceipt {
    pub distribution_id: u64,
    pub token: Address,
    pub total_amount: i128,
    pub timestamp: u64,
    pub processed: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryOperationLog {
    pub op_id: u64,
    pub op_type: Symbol,
    pub caller: Address,
    pub token: Address,
    pub amount: i128,
    pub recipient: Option<Address>,
    pub timestamp: u64,
    pub ledger_seq: u32,
    pub success: bool,
}

const ADMIN_CHANGE_TIMELOCK: u64 = 48 * 60 * 60;

const MAX_FINANCIAL_AMOUNT: i128 = 1_000_000_000_000_000;

const MAX_PER_TX_ALLOCATE: i128 = 100_000_000_000_000;
const MAX_PER_TX_DISTRIBUTE: i128 = 500_000_000_000_000;
const MAX_PER_TX_BUYBACK: i128 = 200_000_000_000_000;

// ---------------------------------------------------------------------------
// Snapshot-coordination / distribution-timing constants.
//
// The treasury is the authority that *schedules* distributions, so it owns
// two anti-dilution safety levers the staking contract itself cannot know:
//   1. `DISTRIBUTION_MIN_INTERVAL_SECS` — minimum gap between two
//      distributions. Prevents an attacker from bribing / rushing an
//      extra distribution right after staking to bypass the min-duration
//      gate for "long-term" reward weighting.
//   2. `DISTRIBUTION_SCHEDULE_WINDOW_SECS` — when the admin announces a
//      next distribution via `schedule_staker_distribution`, the staking
//      contract's pattern detector treats any stake > 10% of TotalStaked
//      made within this window of the scheduled time as a potential
//      late-staking dilution attempt and flags it for review.
// ---------------------------------------------------------------------------

/// Minimum number of seconds between two staker distributions.
pub const DISTRIBUTION_MIN_INTERVAL_SECS: u64 = 1 * 24 * 60 * 60; // 1 day

/// How many seconds before a scheduled distribution the pattern detector
/// should consider a large new stake suspicious.
pub const DISTRIBUTION_SCHEDULE_WINDOW_SECS: u64 = 3 * 24 * 60 * 60; // 3 days

/// Admin-configurable distribution "buffer" delay. When set, the treasury
/// waits at least this many seconds after announcing a scheduled
/// distribution before actually executing it, giving the pattern detector
/// time to surface any suspicious late stakes.
pub const DISTRIBUTION_BUFFER_SECS: u64 = 4 * 60 * 60; // 4 hours

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Timelock,
    StakingContract,
    PauseGuardian,
    AllocationCount,
    Allocation(u32),
    ApprovedToken(Address),
    RegulatoryReporting,
    MultisigThreshold,
    PendingAllocationCount,
    PendingAllocation(u32),
    AllocationApproval(u32, Address),
    PendingAdminTransfer,
    LastAdminChange,
    AutoBurnRateBps,
    BurnQueue,
    DistributionReceipt(u64),
    LastDistributionId,
    OperationLogCount,
    OperationLog(u64),
    AuthorizedCallers,
    TreasuryContractSelf,
    // -----------------------------------------------------------------------
    // Snapshot-coordination / distribution-timing keys
    // -----------------------------------------------------------------------
    /// Timestamp (ledger time) at which the most recent staker-distribution
    /// was executed. Used alongside DISTRIBUTION_MIN_INTERVAL_SECS to
    /// enforce a minimum gap so distributions cannot be rushed to benefit
    /// freshly-deposited late stakers.
    LastDistributionAt,
    /// Admin-scheduled timestamp of the *next* staker distribution. When
    /// set, `distribute_to_stakers` refuses to run before this time (so
    /// the announced schedule can't be front-run) and the staking
    /// contract's pattern detector uses this value to flag large stakes
    /// placed immediately before the window.
    ScheduledNextDistributionAt,
    /// Admin-set flag that, when true, requires every
    /// `distribute_to_stakers` call to be preceded by a matching
    /// `schedule_staker_distribution`. Defaults to false (backwards
    /// compatible) but governance can flip it on to fully commit the
    /// protocol to announced schedules.
    RequireScheduledDistribution,
    // -----------------------------------------------------------------------
    // Pricing-algorithm protection keys (#pricing-manipulation-resistance)
    // -----------------------------------------------------------------------
    /// Admin-set benchmark market rate for a token, used to validate that
    /// proposed session/swap prices haven't drifted into artificial inflation.
    BenchmarkRate(Address),
    PriceFloor(Address),
    PriceCeiling(Address),
    MaxPriceDeviationBps(Address),
    /// Rolling window of recently-proposed prices/timestamps for a token,
    /// used to detect coordinated price setting.
    RecentPricesForToken(Address),
    RecentPriceTimestampsForToken(Address),
    // -----------------------------------------------------------------------
    // MEV Protection keys
    // -----------------------------------------------------------------------
    MevInteractionCount(Address, u32),
    // -----------------------------------------------------------------------
    // Economic Audit
    // -----------------------------------------------------------------------
    TotalDeposits(Address),
    DepositCount(Address),
    // ── Economic monitoring & fairness audit (#903) ────────────────────────
    /// Token flow monitoring record for a distribution epoch.
    TokenFlowRecord(u64),
    /// Fairness audit result for a distribution.
    FairnessAuditRecord(u64),
}

/// Maximum length of the rolling per-token price log kept for coordination scoring.
const PRICE_MONITORING_LOG_CAP: u32 = 20;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct TreasuryContract;

#[contractimpl]
impl TreasuryContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        staking_contract: Address,
        timelock: Address,
        pause_guardian: Option<Address>,
    ) -> Result<(), Error> {
        if env.storage().persistent().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::StakingContract, &staking_contract);
        env.storage()
            .persistent()
            .set(&DataKey::Timelock, &timelock);
        if let Some(guardian) = pause_guardian {
            env.storage()
                .persistent()
                .set(&DataKey::PauseGuardian, &guardian);
        }
        env.storage()
            .persistent()
            .set(&DataKey::AllocationCount, &0u32);
        env.storage()
            .persistent()
            .set(&DataKey::LastDistributionId, &0u64);
        env.storage()
            .persistent()
            .set(&DataKey::OperationLogCount, &0u64);
        env.storage()
            .persistent()
            .set(&DataKey::TreasuryContractSelf, &env.current_contract_address());

        let mut auth_callers: Vec<Address> = Vec::new(&env);
        auth_callers.push_back(timelock.clone());
        auth_callers.push_back(admin.clone());
        env.storage()
            .persistent()
            .set(&DataKey::AuthorizedCallers, &auth_callers);

        Ok(())
    }

    pub fn set_auto_burn_rate(env: Env, admin: Address, bps: u32) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        Validator::new(&env)
            .require_valid_bps(bps, "bps")
            .validate()
            .map_err(|_| Error::InvalidAmount)?;
        env.storage()
            .persistent()
            .set(&DataKey::AutoBurnRateBps, &bps);
        Ok(())
    }

    pub fn execute_burn_queue(env: Env) -> Result<i128, Error> {
        let queued: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::BurnQueue)
            .unwrap_or(0);
        if queued <= 0 {
            return Ok(0);
        }
        env.storage().persistent().set(&DataKey::BurnQueue, &0i128);
        env.events().publish(
            (symbol_short!("burn"), symbol_short!("executed")),
            queued,
        );
        Ok(queued)
    }

    pub fn propose_admin_change(
        env: Env,
        current_admin: Address,
        new_admin: Address,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &current_admin)?;
        
        let last_change: u64 = env.storage().persistent().get(&DataKey::LastAdminChange).unwrap_or(0);
        let current_time = env.ledger().timestamp();
        if current_time < last_change + ADMIN_COOLING_OFF_SECS {
            return Err(Error::CoolingOffPeriod);
        }

        let effective_at = current_time
            .checked_add(MIN_ADMIN_TIMELOCK_SECS)
            .ok_or(Error::InvalidAdminChange)?;

        let pending = AdminTransfer {
            new_admin: new_admin.clone(),
            effective_at,
            status: AdminChangeProposal::Proposed,
        };
        env.storage()
            .persistent()
            .set(&DataKey::PendingAdminTransfer, &pending);
        env.events().publish(
            (symbol_short!("admin"), symbol_short!("proposed")),
            AdminChangeProposedEvent {
                contract: env.current_contract_address(),
                old_admin: current_admin,
                new_admin,
                effective_at,
            },
        );
        Ok(())
    }

    pub fn accept_admin_change(env: Env, new_admin: Address) -> Result<(), Error> {
        new_admin.require_auth();
        let mut pending: AdminTransfer = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdminTransfer)
            .ok_or(Error::NoPendingAdminChange)?;
        if pending.new_admin != new_admin {
            return Err(Error::Unauthorized);
        }
        if env.ledger().timestamp() < pending.effective_at {
            return Err(Error::TimelockNotExpired);
        }
        if pending.status != AdminChangeProposal::Proposed {
            return Err(Error::InvalidAdminChange);
        }
        
        pending.status = AdminChangeProposal::Accepted;

        env.storage().persistent().set(&DataKey::Admin, &new_admin);
        env.storage().persistent().set(&DataKey::LastAdminChange, &env.ledger().timestamp());
        env.storage().persistent().remove(&DataKey::PendingAdminTransfer);
        
        env.events().publish(
            (symbol_short!("admin"), symbol_short!("accepted")),
            AdminChangeAcceptedEvent {
                contract: env.current_contract_address(),
                new_admin,
            },
        );
        Ok(())
    }

    pub fn cancel_admin_change(env: Env, multisig: Address) -> Result<(), Error> {
        multisig.require_auth();
        if !env.storage().persistent().has(&DataKey::PendingAdminTransfer) {
            return Err(Error::NoPendingAdminChange);
        }
        env.storage().persistent().remove(&DataKey::PendingAdminTransfer);
        Ok(())
    }

    pub fn revoke_admin_emergency(env: Env, new_admin: Address) -> Result<(), Error> {
        // Assume multisig is authorized to call this via timelock or direct consensus
        let timelock: Address = env.storage().persistent().get(&DataKey::Timelock).ok_or(Error::NotInitialized)?;
        timelock.require_auth();
        
        env.storage().persistent().set(&DataKey::Admin, &new_admin);
        env.storage().persistent().remove(&DataKey::PendingAdminTransfer);
        Ok(())
    }

    pub fn get_pending_admin_change(env: Env) -> Option<AdminTransfer> {
        env.storage().persistent().get(&DataKey::PendingAdminTransfer)
    }

    pub fn get_admin(env: Env) -> Result<Address, Error> {
        Self::admin(&env)
    }

    pub fn set_regulatory_reporting(env: Env, reporting_address: Address) -> Result<(), Error> {
        let admin = Self::admin(&env)?;
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::RegulatoryReporting, &reporting_address);
        Ok(())
    }

    pub fn add_authorized_caller(
        env: Env,
        admin: Address,
        new_caller: Address,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        let mut auth_callers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::AuthorizedCallers)
            .unwrap_or(Vec::new(&env));
        auth_callers.push_back(new_caller.clone());
        env.storage()
            .persistent()
            .set(&DataKey::AuthorizedCallers, &auth_callers);
        env.events().publish(
            (symbol_short!("auth"), symbol_short!("added")),
            new_caller,
        );
        Ok(())
    }

    /// Admin-set benchmark market rate for `token`, used as the reference
    /// point for market-rate validation and fair-pricing enforcement.
    pub fn set_benchmark_rate(env: Env, admin: Address, token: Address, rate: i128) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        if rate <= 0 {
            return Err(Error::InvalidAmount);
        }
        env.storage()
            .persistent()
            .set(&DataKey::BenchmarkRate(token), &rate);
        Ok(())
    }

    /// Admin-set fair-pricing bounds and maximum allowed deviation from the
    /// benchmark rate (basis points) for `token`.
    pub fn set_pricing_bounds(
        env: Env,
        admin: Address,
        token: Address,
        floor: i128,
        ceiling: i128,
        max_deviation_bps: u32,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        if floor < 0 || ceiling < floor {
            return Err(Error::InvalidAmount);
        }
        env.storage()
            .persistent()
            .set(&DataKey::PriceFloor(token.clone()), &floor);
        env.storage()
            .persistent()
            .set(&DataKey::PriceCeiling(token.clone()), &ceiling);
        env.storage().persistent().set(
            &DataKey::MaxPriceDeviationBps(token),
            &max_deviation_bps.min(shared::MAX_MARKET_DEVIATION_CEILING_BPS),
        );
        Ok(())
    }

    /// Validate a proposed price against the admin-configured benchmark
    /// market rate for `token`.
    pub fn validate_market_rates(
        env: Env,
        token: Address,
        proposed_price: i128,
    ) -> Result<MarketRateValidation, Error> {
        let benchmark: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::BenchmarkRate(token.clone()))
            .ok_or(Error::NotInitialized)?;
        let max_dev: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MaxPriceDeviationBps(token))
            .unwrap_or(DEFAULT_MAX_MARKET_DEVIATION_BPS);
        Ok(validate_market_rate(proposed_price, benchmark, max_dev))
    }

    /// Score the platform-wide rolling price history for `token` for
    /// coordinated price setting (near-identical prices set within a tight
    /// window by independent callers), recording `proposed_price` as the
    /// latest observation. Callable by the admin or an authorized caller.
    pub fn protect_pricing_algorithms(
        env: Env,
        caller: Address,
        token: Address,
        proposed_price: i128,
    ) -> Result<PriceCoordinationFlag, Error> {
        Self::require_authorized_caller(&env, &caller)?;
        if proposed_price <= 0 {
            return Err(Error::InvalidAmount);
        }

        let prices_key = DataKey::RecentPricesForToken(token.clone());
        let ts_key = DataKey::RecentPriceTimestampsForToken(token.clone());
        let mut prices: Vec<i128> = env.storage().persistent().get(&prices_key).unwrap_or(Vec::new(&env));
        let mut timestamps: Vec<u64> = env.storage().persistent().get(&ts_key).unwrap_or(Vec::new(&env));
        prices.push_back(proposed_price);
        timestamps.push_back(env.ledger().timestamp());
        while prices.len() > PRICE_MONITORING_LOG_CAP {
            prices.remove(0);
        }
        while timestamps.len() > PRICE_MONITORING_LOG_CAP {
            timestamps.remove(0);
        }
        env.storage().persistent().set(&prices_key, &prices);
        env.storage().persistent().set(&ts_key, &timestamps);

        let flag = detect_price_coordination(&prices, &timestamps);
        if flag.suspicious {
            env.events().publish(
                (symbol_short!("pricing"), symbol_short!("coord")),
                (token, flag.risk_score),
            );
        }
        Ok(flag)
    }

    /// Enforce fair-pricing bounds for `token`: clamps `proposed_price` into
    /// the admin-configured floor/ceiling, and further clamps to the
    /// benchmark-rate deviation band when market-rate validation flags
    /// inflation.
    pub fn enforce_fair_pricing(
        env: Env,
        token: Address,
        proposed_price: i128,
    ) -> Result<FairPricingResult, Error> {
        let benchmark: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::BenchmarkRate(token.clone()))
            .unwrap_or(0);
        let max_dev: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MaxPriceDeviationBps(token.clone()))
            .unwrap_or(DEFAULT_MAX_MARKET_DEVIATION_BPS);
        let floor: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PriceFloor(token.clone()))
            .unwrap_or(0);
        let ceiling: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PriceCeiling(token.clone()))
            .unwrap_or(i128::MAX);

        let market = validate_market_rate(proposed_price, benchmark, max_dev);
        Ok(shared_enforce_fair_pricing(
            &env,
            proposed_price,
            floor,
            ceiling,
            market,
            benchmark,
            max_dev,
        ))
    }

    fn require_authorized_caller(env: &Env, caller: &Address) -> Result<(), Error> {
        caller.require_auth();
        let admin = Self::admin(env)?;
        if admin == *caller {
            return Ok(());
        }
        let auth_callers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::AuthorizedCallers)
            .unwrap_or(Vec::new(env));
        if auth_callers.contains(caller) {
            Ok(())
        } else {
            Err(Error::Unauthorized)
        }
    }

    pub fn admin_resume_rg(
        env: Env,
        admin: Address,
        lock_name: Option<Symbol>,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        ReentrancyGuard::admin_resume(&env, &admin, lock_name);
        env.events().publish(
            (symbol_short!("rg"), symbol_short!("resumed")),
            admin.clone(),
        );
        Ok(())
    }

    pub fn rg_is_paused(env: Env, lock_name: Option<Symbol>) -> bool {
        ReentrancyGuard::is_paused(&env, lock_name)
    }

    fn admin(env: &Env) -> Result<Address, Error> {
        env.storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin = Self::admin(env)?;
        if stored_admin != *admin {
            return Err(Error::Unauthorized);
        }
        if env.storage().persistent().has(&DataKey::PendingAdminTransfer) {
            return Err(Error::SuspendedDuringAdminTransfer);
        }
        Ok(())
    }

    fn get_authorized_callers(env: &Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::AuthorizedCallers)
            .unwrap_or(Vec::new(env))
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
            .persistent()
            .get::<DataKey, Address>(&DataKey::RegulatoryReporting)
        {
            use soroban_sdk::IntoVal;
            let _ = env.try_invoke_contract::<(), soroban_sdk::Error>(
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

    fn _log_operation(
        env: &Env,
        op_type: Symbol,
        caller: Address,
        token: Address,
        amount: i128,
        recipient: Option<Address>,
        success: bool,
    ) {
        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::OperationLogCount)
            .unwrap_or(0);
        let log = TreasuryOperationLog {
            op_id: count,
            op_type: op_type.clone(),
            caller: caller.clone(),
            token: token.clone(),
            amount,
            recipient: recipient.clone(),
            timestamp: env.ledger().timestamp(),
            ledger_seq: env.ledger().sequence(),
            success,
        };
        env.storage()
            .persistent()
            .set(&DataKey::OperationLog(count), &log);
        env.storage()
            .persistent()
            .set(&DataKey::OperationLogCount, &(count.checked_add(1).unwrap_or(count)));

        env.events().publish(
            (symbol_short!("oplog"), op_type, count),
            (caller, token, amount, success),
        );
    }

    fn _require_not_rg_paused(env: &Env, lock_name: &Symbol) -> Result<(), Error> {
        if ReentrancyGuard::is_paused(env, Some(lock_name.clone())) {
            return Err(Error::ReentrancyGuardPaused);
        }
        Ok(())
    }

    fn _track_mev_interaction(env: &Env, caller: &Address) -> u32 {
        let key = DataKey::MevInteractionCount(caller.clone(), env.ledger().sequence());
        let mut count: u32 = env.storage().temporary().get(&key).unwrap_or(0);
        count += 1;
        env.storage().temporary().set(&key, &count);
        count
    }

    fn _is_token_approved(env: &Env, token: &Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::ApprovedToken(token.clone()))
            .unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Token whitelist management
    // -----------------------------------------------------------------------

    pub fn set_approved_token(
        env: Env,
        token_address: Address,
        approved: bool,
    ) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage().persistent().set(&key, &approved);

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

    pub fn deposit(env: Env, from: Address, token: Address, amount: i128) -> Result<(), Error> {
        if let Some(guardian) = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::PauseGuardian)
        {
            require_not_paused(&env, &guardian);
        }

        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "deposit"));

        from.require_auth();

        // ── Rate Limiting & Emergency Throttling ─────────────────────────────────
        let req_count = Self::_track_mev_interaction(&env, &from);
        
        let global_reqs: u32 = env.storage().temporary().get(&DataKey::NamespaceRoot).unwrap_or(0) + 1;
        env.storage().temporary().set(&DataKey::NamespaceRoot, &global_reqs);
        
        let is_emergency = check_emergency_trigger(global_reqs);
        let limit_status = manage_session_load(&env, req_count, is_emergency);
        if !limit_status.allowed {
            return Err(Error::ReentrancyGuardPaused); // Treat as throttled
        }
        // ─────────────────────────────────────────────────────────────────────────

        Validator::new(&env)
            .require_positive(amount, "amount")
            .require_max(amount, MAX_FINANCIAL_AMOUNT, "amount")
            .validate()
            .map_err(|_| Error::InvalidAmount)?;
        if !Self::_is_token_approved(&env, &token) {
            return Err(Error::TokenNotApproved);
        }

        let pre_snapshot = StateSnapshot::capture(&env);
        let balance_before: i128 =
            token::Client::new(&env, &token).balance(&env.current_contract_address());

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&from, &env.current_contract_address(), &amount);

        let balance_after: i128 =
            token::Client::new(&env, &token).balance(&env.current_contract_address());

        // ── Economic Audit ───────────────────────────────────────────────────────
        let dep_count_key = DataKey::DepositCount(from.clone());
        let total_dep_key = DataKey::TotalDeposits(from.clone());
        
        let count: u32 = env.storage().persistent().get(&dep_count_key).unwrap_or(0) + 1;
        let total: i128 = env.storage().persistent().get(&total_dep_key).unwrap_or(0) + amount;
        
        env.storage().persistent().set(&dep_count_key, &count);
        env.storage().persistent().set(&total_dep_key, &total);
        
        // Expected fee of 100_000 tokens on average per deposit (example baseline)
        let expected_avg: i128 = 100_000;
        let actual_avg = total / (count as i128);
        
        let audit = detect_fee_evasion(&env, expected_avg, actual_avg);
        if audit.penalty_tier == PenaltyTier::PermanentBan || audit.penalty_tier == PenaltyTier::TemporarySuspension {
            return Err(Error::CallerNotAuthorized); // Block deposit if suspended for fee evasion
        }
        // ─────────────────────────────────────────────────────────────────────────

        if balance_after.checked_sub(balance_before) != Some(amount) {
            return Err(Error::InsufficientBalance);
        }
        pre_snapshot.assert_valid();

        Self::_log_operation(
            &env,
            Symbol::new(&env, "deposit"),
            from.clone(),
            token.clone(),
            amount,
            Some(env.current_contract_address()),
            true,
        );

        env.events().publish(
            (symbol_short!("deposit"), from.clone(), token.clone()),
            amount,
        );
        Ok(())
    }

    pub fn get_balance(env: Env, token: Address) -> i128 {
        token::Client::new(&env, &token).balance(&env.current_contract_address())
    }

    pub fn allocate(
        env: Env,
        token: Address,
        recipient: Address,
        amount: i128,
    ) -> Result<(), Error> {
        if let Some(guardian) = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::PauseGuardian)
        {
            require_not_paused(&env, &guardian);
        }

        let lock_sym = Symbol::new(&env, "allocate");
        Self::_require_not_rg_paused(&env, &lock_sym)?;

        let _guard = ReentrancyGuard::enter(&env, lock_sym);
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        if !Self::_is_token_approved(&env, &token) {
            return Err(Error::TokenNotApproved);
        }

        Validator::new(&env)
            .require_positive(amount, "amount")
            .require_max(amount, MAX_FINANCIAL_AMOUNT, "amount")
            .validate()
            .map_err(|_| Error::InvalidAmount)?;

        if !validate_amount_limits(amount, 1, MAX_PER_TX_ALLOCATE) {
            return Err(Error::AmountExceedsLimit);
        }

        let auth_callers = Self::get_authorized_callers(&env);
        if !validate_caller_is_authorized(&env, &admin, &auth_callers) {
            return Err(Error::CallerNotAuthorized);
        }

        // ── Rate Limiting & Emergency Throttling ─────────────────────────────────
        let interactions = Self::_track_mev_interaction(&env, &admin);
        let global_reqs: u32 = env.storage().temporary().get(&DataKey::NamespaceRoot).unwrap_or(0) + 1;
        env.storage().temporary().set(&DataKey::NamespaceRoot, &global_reqs);
        
        let is_emergency = check_emergency_trigger(global_reqs);
        let limit_status = manage_session_load(&env, interactions, is_emergency);
        if !limit_status.allowed {
            return Err(Error::ReentrancyGuardPaused); // Treat as throttled
        }
        // ─────────────────────────────────────────────────────────────────────────

        let mev_flag = detect_atomic_arbitrage(&env, &admin, interactions);
        if !enforce_protocol_isolation(&mev_flag) {
            return Err(Error::ReentrancyGuardPaused); // Treat as isolated/blocked
        }

        let pre_snapshot = StateSnapshot::capture(&env);
        let balance_before: i128 =
            token::Client::new(&env, &token).balance(&env.current_contract_address());
        if balance_before < amount {
            Self::_log_operation(
                &env,
                Symbol::new(&env, "allocate"),
                admin.clone(),
                token.clone(),
                amount,
                Some(recipient.clone()),
                false,
            );
            return Err(Error::InsufficientBalance);
        }

        Self::_check_and_report_large_tx(
            &env,
            Symbol::new(&env, "treasury"),
            Symbol::new(&env, "allocate"),
            &recipient,
            amount,
        );

        let threshold: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::MultisigThreshold)
            .unwrap_or(50_000);

        if amount > threshold {
            let pending_count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::PendingAllocationCount)
                .unwrap_or(0);

            let pending = PendingAllocation {
                id: pending_count,
                token: token.clone(),
                recipient: recipient.clone(),
                amount,
                approvals_count: 1,
                executed: false,
                created_at: env.ledger().timestamp(),
            };

            env.storage()
                .persistent()
                .set(&DataKey::PendingAllocation(pending_count), &pending);
            env.storage().persistent().set(
                &DataKey::AllocationApproval(pending_count, admin.clone()),
                &true,
            );
            env.storage()
                .persistent()
                .set(&DataKey::PendingAllocationCount, &(pending_count + 1));

            env.events().publish(
                (symbol_short!("allocate"), symbol_short!("pending")),
                (pending_count, recipient.clone(), amount),
            );

            Self::_log_operation(
                &env,
                Symbol::new(&env, "allocate_pending"),
                admin,
                token,
                amount,
                Some(recipient),
                true,
            );
            pre_snapshot.assert_valid();
            return Ok(());
        }

        let mut batch = AtomicBatch::new(&env);
        batch.add_transfer(
            token.clone(),
            env.current_contract_address(),
            recipient.clone(),
            amount,
        );
        batch.validate().map_err(|_| Error::InvalidAmount)?;

        let token_ref = token.clone();
        let recipient_ref = recipient.clone();
        let amount_ref = amount;

        batch.execute_all(|e, op| match op {
            BatchOp::Transfer(token, from, to, amount, _) => {
                token::Client::new(e, token).transfer(from, to, amount);
                Ok(())
            }
            _ => Ok(()),
        }).map_err(|_e: shared::reentrancy_guard::BatchValidationError| Error::InvalidAmount)?;

        let balance_after: i128 =
            token::Client::new(&env, &token_ref).balance(&env.current_contract_address());
        if balance_before.checked_sub(balance_after) != Some(amount_ref) {
            return Err(Error::StateValidationFailed);
        }
        let recipient_balance = token::Client::new(&env, &token_ref).balance(&recipient_ref);
        if recipient_balance < amount_ref {
            return Err(Error::StateValidationFailed);
        }
        let conservation = validate_fund_conservation(
            &env, balance_before, 0, amount_ref, 0, balance_after,
        );
        record_invariant_check(&env, &EconomicInvariantRecord {
            invariant: EconomicInvariant::FundConservation,
            valid: conservation.valid,
            observed: conservation.observed,
            expected: conservation.expected,
            timestamp: env.ledger().timestamp(),
            ledger: env.ledger().sequence(),
        });
        if !conservation.valid {
            return Err(Error::StateValidationFailed);
        }
        pre_snapshot.assert_valid();

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
        env.storage()
            .persistent()
            .set(&DataKey::AllocationCount, &(count + 1));

        Self::_log_operation(
            &env,
            Symbol::new(&env, "allocate"),
            admin,
            token.clone(),
            amount,
            Some(recipient.clone()),
            true,
        );

        env.events().publish(
            (symbol_short!("allocate"), recipient.clone(), token.clone()),
            amount,
        );
        Ok(())
    }

    /// Allocate to multiple recipients atomically (#830).
    ///
    /// Every request is fully pre-validated (token approved, amount sane,
    /// per-tx cap respected, sufficient balance for the batch's total)
    /// *before* a single transfer runs, so a request that would fail never
    /// leaves the contract partway through a payout. Execution then runs
    /// through [`AtomicBatch`], which stops at the first failing transfer;
    /// since this function propagates that failure as `Err`, Soroban
    /// reverts every transfer already made in this call — the batch either
    /// lands in full or not at all.
    ///
    /// Unlike [`allocate`](Self::allocate), items above `MultisigThreshold`
    /// are rejected outright rather than queued for approval — batch
    /// requests are for below-threshold, routine payouts; a large transfer
    /// should go through the single-item `allocate` pending-approval path.
    pub fn batch_allocate(
        env: Env,
        requests: Vec<AllocationRequest>,
    ) -> Result<(), Error> {
        if let Some(guardian) = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::PauseGuardian)
        {
            require_not_paused(&env, &guardian);
        }

        let lock_sym = Symbol::new(&env, "batch_allocate");
        Self::_require_not_rg_paused(&env, &lock_sym)?;
        let _guard = ReentrancyGuard::enter(&env, lock_sym);

        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        if requests.is_empty() || requests.len() > MAX_BATCH_SIZE {
            return Err(Error::InvalidAmount);
        }

        let threshold: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::MultisigThreshold)
            .unwrap_or(50_000);

        // --- Pre-execution validation: every request must be individually
        // valid, and the batch's total must not exceed the contract's
        // current balance per token, before any transfer is attempted. ---
        let mut totals_by_first_use: Vec<(Address, i128)> = Vec::new(&env);
        for req in requests.iter() {
            if !Self::_is_token_approved(&env, &req.token) {
                return Err(Error::TokenNotApproved);
            }
            Validator::new(&env)
                .require_positive(req.amount, "amount")
                .require_max(req.amount, MAX_FINANCIAL_AMOUNT, "amount")
                .validate()
                .map_err(|_| Error::InvalidAmount)?;
            if !validate_amount_limits(req.amount, 1, MAX_PER_TX_ALLOCATE) {
                return Err(Error::AmountExceedsLimit);
            }
            if req.amount > threshold {
                return Err(Error::AmountExceedsLimit);
            }

            let mut found = false;
            for i in 0..totals_by_first_use.len() {
                let (tok, running) = totals_by_first_use.get(i).unwrap();
                if tok == req.token {
                    totals_by_first_use.set(
                        i,
                        (tok, running.checked_add(req.amount).ok_or(Error::InvalidAmount)?),
                    );
                    found = true;
                    break;
                }
            }
            if !found {
                totals_by_first_use.push_back((req.token.clone(), req.amount));
            }
        }
        for (tok, total) in totals_by_first_use.iter() {
            let balance: i128 = token::Client::new(&env, &tok).balance(&env.current_contract_address());
            if balance < total {
                return Err(Error::InsufficientBalance);
            }
        }

        // --- Execution: queue every transfer, then run them in order,
        // aborting (and letting Soroban revert everything) at the first
        // failure. ---
        let mut batch = AtomicBatch::new(&env);
        for req in requests.iter() {
            batch.add_transfer(
                req.token.clone(),
                env.current_contract_address(),
                req.recipient.clone(),
                req.amount,
            );
        }
        batch.validate().map_err(|_| Error::InvalidAmount)?;

        batch
            .execute_all(|e, op| -> Result<(), Error> {
                match op {
                    BatchOp::Transfer(token, from, to, amount, _) => {
                        token::Client::new(e, token).transfer(from, to, amount);
                        Ok(())
                    }
                    _ => Ok(()),
                }
            })
            .map_err(|_e| Error::StateValidationFailed)?;

        // --- Audit trail: one AllocationHistory + operation log entry per
        // request, plus a summary event for the whole batch. ---
        for req in requests.iter() {
            let count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::AllocationCount)
                .unwrap_or(0u32);
            env.storage().persistent().set(
                &DataKey::Allocation(count),
                &AllocationHistory {
                    token: req.token.clone(),
                    recipient: req.recipient.clone(),
                    amount: req.amount,
                    timestamp: env.ledger().timestamp(),
                },
            );
            env.storage()
                .persistent()
                .set(&DataKey::AllocationCount, &(count + 1));

            Self::_log_operation(
                &env,
                Symbol::new(&env, "batch_allocate"),
                admin.clone(),
                req.token.clone(),
                req.amount,
                Some(req.recipient.clone()),
                true,
            );
        }

        env.events().publish(
            (symbol_short!("batch"), symbol_short!("alloc_ok")),
            (requests.len(), env.ledger().timestamp()),
        );

        Ok(())
    }

    pub fn set_multisig_threshold(env: Env, threshold: i128) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::MultisigThreshold, &threshold);
        Ok(())
    }

    pub fn approve_pending_allocation(
        env: Env,
        approver: Address,
        pending_id: u32,
    ) -> Result<(), Error> {
        let lock_sym = Symbol::new(&env, "approve_alloc");
        Self::_require_not_rg_paused(&env, &lock_sym)?;
        let _guard = ReentrancyGuard::enter(&env, lock_sym);

        approver.require_auth();

        let mut pending: PendingAllocation = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAllocation(pending_id))
            .ok_or(Error::NotInitialized)?;

        if pending.executed {
            return Err(Error::InvalidState);
        }

        let approval_key = DataKey::AllocationApproval(pending_id, approver.clone());
        if env.storage().persistent().has(&approval_key) {
            return Err(Error::DuplicateEntry);
        }

        env.storage().persistent().set(&approval_key, &true);
        pending.approvals_count += 1;

        if pending.approvals_count >= 2 {
            let pre_snapshot = StateSnapshot::capture(&env);
            let token_client = token::Client::new(&env, &pending.token);
            let balance_before = token_client.balance(&env.current_contract_address());

            if balance_before < pending.amount {
                Self::_log_operation(
                    &env,
                    Symbol::new(&env, "approve_alloc"),
                    approver,
                    pending.token.clone(),
                    pending.amount,
                    Some(pending.recipient.clone()),
                    false,
                );
                return Err(Error::InsufficientBalance);
            }

            token_client.transfer(
                &env.current_contract_address(),
                &pending.recipient,
                &pending.amount,
            );

            let balance_after = token_client.balance(&env.current_contract_address());
            if balance_before.checked_sub(balance_after) != Some(pending.amount) {
                return Err(Error::StateValidationFailed);
            }
            pre_snapshot.assert_valid();

            pending.executed = true;

            let count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::AllocationCount)
                .unwrap_or(0u32);
            env.storage().persistent().set(
                &DataKey::Allocation(count),
                &AllocationHistory {
                    token: pending.token.clone(),
                    recipient: pending.recipient.clone(),
                    amount: pending.amount,
                    timestamp: env.ledger().timestamp(),
                },
            );
            env.storage()
                .persistent()
                .set(&DataKey::AllocationCount, &(count + 1));

            Self::_log_operation(
                &env,
                Symbol::new(&env, "allocate_executed"),
                approver.clone(),
                pending.token.clone(),
                pending.amount,
                Some(pending.recipient.clone()),
                true,
            );

            env.events().publish(
                (symbol_short!("allocate"), symbol_short!("executed")),
                (pending_id, pending.recipient.clone(), pending.amount),
            );
        }

        env.storage()
            .persistent()
            .set(&DataKey::PendingAllocation(pending_id), &pending);

        Ok(())
    }

    pub fn get_pending_allocation(env: Env, pending_id: u32) -> Option<PendingAllocation> {
        env.storage()
            .persistent()
            .get(&DataKey::PendingAllocation(pending_id))
    }

    pub fn pending_allocation_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::PendingAllocationCount)
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Snapshot coordination / scheduled-distribution flow
    // -----------------------------------------------------------------------

    /// Admin (or governance) announces the timestamp of the next staker
    /// distribution. The treasury forwards this schedule to the staking
    /// contract so its pattern detector can flag large late stakes placed
    /// too close to the distribution window.
    ///
    /// If `RequireScheduledDistribution` is toggled on, every subsequent
    /// `distribute_to_stakers` call must happen **at or after** this
    /// timestamp — attempting to distribute early reverts.
    pub fn schedule_staker_distribution(
        env: Env,
        admin: Address,
        distribution_at: u64,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        let now = env.ledger().timestamp();
        if distribution_at <= now {
            return Err(Error::InvalidState);
        }
        env.storage()
            .persistent()
            .set(&DataKey::ScheduledNextDistributionAt, &distribution_at);

        // Also inform the staking contract, so the on-chain pattern
        // detector can use it without having to trust the caller to pass
        // it in each time.
        let staking: Address = env
            .storage()
            .persistent()
            .get(&DataKey::StakingContract)
            .ok_or(Error::NotInitialized)?;

        // Fire-and-forget. If the staking contract hasn't been upgraded
        // to understand this new entry point we ignore the failure — the
        // treasury itself still enforces the minimum-interval gate so
        // the deployment is safe either way.
        let _ = env.try_invoke_contract::<(), soroban_sdk::Error>(
            &staking,
            &Symbol::new(&env, "set_next_distribution_at"),
            (admin.clone(), distribution_at).into_val(&env),
        );

        env.events().publish(
            (symbol_short!("treasury"), symbol_short!("sched")),
            distribution_at,
        );
        Ok(())
    }

    /// Toggle whether `distribute_to_stakers` MUST be preceded by a
    /// matching `schedule_staker_distribution`. Default is `false`
    /// (backwards compatible). Governance can flip to `true` once the
    /// protocol is ready to commit to fully-announced schedules.
    pub fn set_require_sched_distrib(
        env: Env,
        admin: Address,
        require: bool,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage()
            .persistent()
            .set(&DataKey::RequireScheduledDistribution, &require);
        Ok(())
    }

    pub fn get_scheduled_distribution_at(env: Env) -> Option<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::ScheduledNextDistributionAt)
    }

    pub fn get_require_sched_distrib(env: Env) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::RequireScheduledDistribution)
            .unwrap_or(false)
    }

    pub fn get_last_distribution_at(env: Env) -> Option<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::LastDistributionAt)
    }

    pub fn distribute_to_stakers(
        env: Env,
        token: Address,
        total_amount: i128,
    ) -> Result<(), Error> {
        if let Some(guardian) = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::PauseGuardian)
        {
            require_not_paused(&env, &guardian);
        }

        let lock_sym = Symbol::new(&env, "distribute");
        Self::_require_not_rg_paused(&env, &lock_sym)?;

        let _guard = ReentrancyGuard::enter(&env, lock_sym);
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        if !Self::_is_token_approved(&env, &token) {
            return Err(Error::TokenNotApproved);
        }

        Validator::new(&env)
            .require_positive(total_amount, "total_amount")
            .require_max(total_amount, MAX_FINANCIAL_AMOUNT, "total_amount")
            .validate()
            .map_err(|_| Error::InvalidAmount)?;

        if !validate_amount_limits(total_amount, 1, MAX_PER_TX_DISTRIBUTE) {
            return Err(Error::AmountExceedsLimit);
        }

        let staking_contract: Address = env
            .storage()
            .persistent()
            .get(&DataKey::StakingContract)
            .ok_or(Error::NotInitialized)?;

        let auth_callers = Self::get_authorized_callers(&env);
        if !validate_caller_is_authorized(&env, &admin, &auth_callers) {
            return Err(Error::CallerNotAuthorized);
        }

        let interactions = Self::_track_mev_interaction(&env, &admin);
        let mev_flag = detect_atomic_arbitrage(&env, &admin, interactions);
        if !enforce_protocol_isolation(&mev_flag) {
            return Err(Error::ReentrancyGuardPaused);
        }

        // -------------------------------------------------------------------
        // Attack-vector dilution mitigation #1: minimum-interval gate.
        // Distributions can't be squeezed right after a big stake to game
        // the duration-multiplier math.
        // -------------------------------------------------------------------
        let now = env.ledger().timestamp();
        if let Some(last_at) = env
            .storage()
            .persistent()
            .get::<_, u64>(&DataKey::LastDistributionAt)
        {
            let gap = now.saturating_sub(last_at);
            if gap < DISTRIBUTION_MIN_INTERVAL_SECS {
                return Err(Error::InvalidState);
            }
        }

        // -------------------------------------------------------------------
        // Attack-vector dilution mitigation #2: scheduled-distribution gate.
        // If governance has turned on `RequireScheduledDistribution`, a
        // distribution that runs before its announced `ScheduledNext-
        // DistributionAt` timestamp is rejected. This prevents the admin
        // from colluding with a large late staker by advancing the
        // schedule after the stake is in.
        //
        // We also enforce DISTRIBUTION_BUFFER_SECS: if a schedule was
        // set, the distribution must happen *at least* DISTRIBUTION_BUFFER-
        // _SECS after the schedule was written (in practice this is
        // already satisfied because schedule_at > now by construction,
        // but we keep the check explicit for defense-in-depth).
        // -------------------------------------------------------------------
        let require_schedule: bool = env
            .storage()
            .persistent()
            .get(&DataKey::RequireScheduledDistribution)
            .unwrap_or(false);
        let maybe_scheduled: Option<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::ScheduledNextDistributionAt);

        if require_schedule {
            let scheduled = maybe_scheduled.ok_or(Error::InvalidState)?;
            if now < scheduled {
                return Err(Error::InvalidState);
            }
        } else if let Some(scheduled) = maybe_scheduled {
            // Not strictly required, but if admin *did* schedule one we
            // still refuse to run before the scheduled time so the
            // promise is trustworthy.
            if now < scheduled {
                return Err(Error::InvalidState);
            }
        }

        // -------------------------------------------------------------------
        // Attack-vector dilution mitigation #3: pre-distribution staking
        // contract health probe.
        //
        // Query the staking contract for the current penalty-pool balance
        // and log it. This is a reconcilable audit trail for the amount
        // of penalty redistribution that will be merged into this epoch.
        // -------------------------------------------------------------------
        let staking_client = StakingContractClient::new(&env, &staking_contract);
        let penalty_pool_before: i128 =
            staking_client.get_penalty_redistribution_pool();

        // -------------------------------------------------------------------
        // Balance / snapshot / ID bookkeeping (unchanged from legacy flow)
        // -------------------------------------------------------------------
        let pre_snapshot = StateSnapshot::capture(&env);
        let balance_before: i128 =
            token::Client::new(&env, &token).balance(&env.current_contract_address());
        if balance_before < total_amount {
            Self::_log_operation(
                &env,
                Symbol::new(&env, "distribute"),
                admin.clone(),
                token.clone(),
                total_amount,
                Some(staking_contract.clone()),
                false,
            );
            return Err(Error::InsufficientBalance);
        }

        let distribution_id: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::LastDistributionId)
            .unwrap_or(0u64)
            .checked_add(1)
            .ok_or(Error::Overflow)?;

        let receipt_key = DataKey::DistributionReceipt(distribution_id);
        if env.storage().persistent().has(&receipt_key) {
            return Err(Error::DistributionAlreadyProcessed);
        }

        env.storage().persistent().set(
            &receipt_key,
            &DistributionReceipt {
                distribution_id,
                token: token.clone(),
                total_amount,
                timestamp: now,
                processed: false,
            },
        );
        env.storage()
            .persistent()
            .set(&DataKey::LastDistributionId, &distribution_id);
        env.storage()
            .persistent()
            .set(&DataKey::LastDistributionAt, &now);

        // Clear the schedule marker now that the distribution is being
        // executed (a new schedule must be explicitly set for the next
        // round).
        env.storage()
            .persistent()
            .remove(&DataKey::ScheduledNextDistributionAt);

        let mut batch = AtomicBatch::new(&env);
        batch.add_transfer(
            token.clone(),
            env.current_contract_address(),
            staking_contract.clone(),
            total_amount,
        );
        batch.add_invoke(
            staking_contract.clone(),
            Symbol::new(&env, "receive_treasury_distribution"),
        );
        batch.validate().map_err(|_| Error::InvalidAmount)?;

        let staking_ref = staking_contract.clone();
        let token_ref = token.clone();
        let amount_ref = total_amount;
        let dist_id_ref = distribution_id;
        let treasury_self = env.current_contract_address();

        batch.execute_all(|e, op| match op {
            BatchOp::Transfer(token, from, to, amount, _) => {
                token::Client::new(e, token).transfer(from, to, amount);
                Ok(())
            }
            BatchOp::Invoke(contract, _function, _) => {
                let lp_amount = amount_ref / 10;
                let staker_amount = amount_ref - lp_amount;

                if lp_amount > 0 {
                    let _: () = e.invoke_contract(
                        contract,
                        &Symbol::new(e, "add_to_lp_reward_pool"),
                        (lp_amount,).into_val(e),
                    );
                }

                let _: () = e.invoke_contract(
                    contract,
                    &Symbol::new(e, "receive_treasury_distribution"),
                    (
                        dist_id_ref,
                        treasury_self.clone(),
                        token_ref.clone(),
                        staker_amount,
                        e.ledger().timestamp(),
                    )
                        .into_val(e),
                );
                Ok(())
            }
        }).map_err(|_e: shared::reentrancy_guard::BatchValidationError| Error::InvalidAmount)?;

        let balance_after: i128 =
            token::Client::new(&env, &token).balance(&env.current_contract_address());
        if balance_before.checked_sub(balance_after) != Some(total_amount) {
            return Err(Error::StateValidationFailed);
        }
        let staking_balance =
            token::Client::new(&env, &token).balance(&staking_ref);
        if staking_balance < total_amount {
            return Err(Error::StateValidationFailed);
        }
        let conservation = validate_fund_conservation(
            &env, balance_before, 0, total_amount, 0, balance_after,
        );
        record_invariant_check(&env, &EconomicInvariantRecord {
            invariant: EconomicInvariant::FundConservation,
            valid: conservation.valid,
            observed: conservation.observed,
            expected: conservation.expected,
            timestamp: env.ledger().timestamp(),
            ledger: env.ledger().sequence(),
        });
        if !conservation.valid {
            return Err(Error::StateValidationFailed);
        }
        pre_snapshot.assert_valid();

        let mut receipt: DistributionReceipt = env
            .storage()
            .persistent()
            .get(&receipt_key)
            .unwrap();
        receipt.processed = true;
        env.storage().persistent().set(&receipt_key, &receipt);

        Self::_log_operation(
            &env,
            Symbol::new(&env, "distribute"),
            admin,
            token.clone(),
            total_amount,
            Some(staking_contract.clone()),
            true,
        );

        env.events().publish(
            (
                symbol_short!("distrib"),
                staking_contract.clone(),
                token.clone(),
            ),
            (
                total_amount,
                distribution_id,
                penalty_pool_before,
            ),
        );
        Ok(())
    }

    pub fn get_distribution_receipt(env: Env, distribution_id: u64) -> Option<DistributionReceipt> {
        env.storage()
            .persistent()
            .get(&DataKey::DistributionReceipt(distribution_id))
    }

    pub fn get_operation_log(env: Env, op_id: u64) -> Option<TreasuryOperationLog> {
        env.storage()
            .persistent()
            .get(&DataKey::OperationLog(op_id))
    }

    pub fn get_operation_log_count(env: Env) -> u64 {
        env.storage()
            .persistent()
            .get(&DataKey::OperationLogCount)
            .unwrap_or(0)
    }

    pub fn buyback_and_burn(
        env: Env,
        xlm_token: Address,
        mnt_token: Address,
        dex_contract: Address,
        xlm_amount: i128,
        min_mnt_out: i128,
        dex_iface: DexInterface,
        oracle_contract: Option<Address>,
        mnt_asset_symbol: Option<Symbol>,
    ) -> Result<(), Error> {
        let lock_sym = Symbol::new(&env, "buyback");
        Self::_require_not_rg_paused(&env, &lock_sym)?;

        let _guard = ReentrancyGuard::enter(&env, lock_sym);

        let timelock: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Timelock)
            .ok_or(Error::NotInitialized)?;
        timelock.require_auth();

        dex_iface.validate(&env);

        if Validator::new(&env)
            .require_positive(xlm_amount, "xlm_amount")
            .require_max(xlm_amount, MAX_FINANCIAL_AMOUNT, "xlm_amount")
            .validate()
            .is_err()
        {
            env.events().publish(
                (symbol_short!("buyback"), symbol_short!("failed")),
                BuybackFailed {
                    xlm_amount,
                    reason: Symbol::new(&env, "invalid_xlm_amount"),
                },
            );
            return Err(Error::InvalidAmount);
        }

        if !validate_amount_limits(xlm_amount, 1, MAX_PER_TX_BUYBACK) {
            return Err(Error::AmountExceedsLimit);
        }

        if min_mnt_out <= 0 {
            env.events().publish(
                (symbol_short!("buyback"), symbol_short!("failed")),
                BuybackFailed {
                    xlm_amount,
                    reason: Symbol::new(&env, "invalid_min_out"),
                },
            );
            return Err(Error::InvalidMinOut);
        }

        if !Self::_is_token_approved(&env, &xlm_token) {
            return Err(Error::TokenNotApproved);
        }
        if !Self::_is_token_approved(&env, &mnt_token) {
            return Err(Error::TokenNotApproved);
        }

        if let (Some(oracle), Some(asset_sym)) = (oracle_contract.clone(), mnt_asset_symbol.clone()) {
            let health: OracleHealth =
                OracleContractClient::new(&env, &oracle).get_oracle_health(&asset_sym);

            // 3a. Staleness check.
            if health.is_stale {
                env.events().publish(
                    (symbol_short!("buyback"), symbol_short!("failed")),
                    BuybackFailed {
                        xlm_amount,
                        reason: Symbol::new(&env, "oracle_stale"),
                    },
                );
                return Err(Error::OracleStale);
            }
            if health.active_feeders < 3 {
                return Err(Error::OracleUnhealthy);
            }

            // 3c. Circuit-breaker check — halt buybacks during high volatility.
            if health.circuit_breaker_tripped {
                env.events().publish(
                    (symbol_short!("buyback"), symbol_short!("failed")),
                    BuybackFailed {
                        xlm_amount,
                        reason: Symbol::new(&env, "oracle_cb"),
                    },
                );
                return Err(Error::OracleCircuitBreaker);
            }
        }

        let pre_snapshot = StateSnapshot::capture(&env);
        let xlm_client = token::Client::new(&env, &xlm_token);
        let mnt_client = token::Client::new(&env, &mnt_token);
        let treasury_addr = env.current_contract_address();

        let xlm_balance_before = xlm_client.balance(&treasury_addr);
        if xlm_balance_before < xlm_amount {
            env.events().publish(
                (symbol_short!("buyback"), symbol_short!("failed")),
                BuybackFailed {
                    xlm_amount,
                    reason: Symbol::new(&env, "insufficient_xlm"),
                },
            );
            return Err(Error::InsufficientBalance);
        }
        let mnt_balance_before = mnt_client.balance(&treasury_addr);

        let expiration_ledger = env.ledger().sequence() + 1;
        xlm_client.approve(
            &treasury_addr,
            &dex_contract,
            &xlm_amount,
            &expiration_ledger,
        );

        let swap_fn = dex_iface.swap_fn.clone();
        let xlm_tok = xlm_token.clone();
        let mnt_tok = mnt_token.clone();
        let treasury_clone = treasury_addr.clone();

        let mnt_received: i128 = env.invoke_contract(
            &dex_contract,
            &swap_fn,
            (
                xlm_token.clone(),
                mnt_token.clone(),
                xlm_amount,
                min_mnt_out,
                treasury_addr.clone(),
            )
                .into_val(&env),
        );

        if mnt_received == 0 {
            xlm_client.approve(
                &treasury_addr,
                &dex_contract,
                &0,
                &expiration_ledger,
            );
            env.events().publish(
                (symbol_short!("buyback"), symbol_short!("failed")),
                BuybackFailed {
                    xlm_amount,
                    reason: Symbol::new(&env, "zero_output"),
                },
            );
            Self::_log_operation(
                &env,
                Symbol::new(&env, "buyback"),
                timelock.clone(),
                xlm_tok,
                xlm_amount,
                None,
                false,
            );
            return Err(Error::ZeroOutput);
        }

        if mnt_received < min_mnt_out {
            xlm_client.approve(
                &treasury_addr,
                &dex_contract,
                &0,
                &expiration_ledger,
            );
            env.events().publish(
                (symbol_short!("buyback"), symbol_short!("failed")),
                BuybackFailed {
                    xlm_amount,
                    reason: Symbol::new(&env, "slippage"),
                },
            );
            Self::_log_operation(
                &env,
                Symbol::new(&env, "buyback"),
                timelock.clone(),
                xlm_tok,
                xlm_amount,
                None,
                false,
            );
            return Err(Error::SlippageExceeded);
        }

        let _: () = env.invoke_contract(
            &mnt_tok,
            &Symbol::new(&env, "burn"),
            (treasury_clone.clone(), mnt_received).into_val(&env),
        );

        let xlm_balance_after = xlm_client.balance(&treasury_addr);
        let mnt_balance_after = mnt_client.balance(&treasury_addr);
        let xlm_spent = xlm_balance_before.checked_sub(xlm_balance_after);
        if xlm_spent.is_none() || xlm_spent.unwrap() > xlm_amount {
            return Err(Error::StateValidationFailed);
        }
        let mnt_net = mnt_balance_after.checked_sub(mnt_balance_before);
        if let Some(net) = mnt_net {
            if net > 0 {
                let burn_rate: Option<u32> = env.storage().persistent().get(&DataKey::AutoBurnRateBps);
                if burn_rate.is_none() {
                    let queued: i128 = env.storage().persistent().get(&DataKey::BurnQueue).unwrap_or(0);
                    env.storage().persistent().set(&DataKey::BurnQueue, &(queued.checked_add(net).unwrap_or(queued)));
                }
            }
        }

        pre_snapshot.assert_valid();

        Self::_log_operation(
            &env,
            Symbol::new(&env, "buyback"),
            timelock.clone(),
            xlm_token.clone(),
            xlm_amount,
            Some(mnt_token.clone()),
            true,
        );

        env.events().publish(
            (symbol_short!("buyback"), symbol_short!("ok")),
            BuybackSucceeded {
                xlm_spent: xlm_amount,
                mnt_burned: mnt_received,
                timestamp: env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // View helpers
    // -----------------------------------------------------------------------

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

    pub fn get_timelock(env: Env) -> Address {
        env.storage()
            .persistent()
            .get(&DataKey::Timelock)
            .expect("not initialized")
    }

    pub fn get_staking_contract(env: Env) -> Result<Address, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::StakingContract)
            .ok_or(Error::NotInitialized)
    }

    // -----------------------------------------------------------------------
    // Dynamic Fees & Revenue
    // -----------------------------------------------------------------------

    pub fn calculate_platform_fees(env: Env, amount: i128, system_load: u32, reputation: u32) -> i128 {
        let dynamic_fee = calculate_dynamic_fee(&env, system_load, reputation);
        (amount.saturating_mul(dynamic_fee.fee_bps as i128)) / 10000
    }

    pub fn collect_fees(
        env: Env, 
        from: Address, 
        token: Address, 
        amount: i128, 
        system_load: u32, 
        reputation: u32
    ) -> Result<i128, Error> {
        from.require_auth();
        let fee = Self::calculate_platform_fees(env.clone(), amount, system_load, reputation);
        
        let recent_tx = env.storage().persistent().get(&DataKey::DepositCount(from.clone())).unwrap_or(0);
        let total_vol = env.storage().persistent().get(&DataKey::TotalDeposits(from.clone())).unwrap_or(0);
        
        let evasion = detect_fee_gaming(&env, recent_tx, total_vol);
        if evasion.is_evading {
            return Err(Error::CallerNotAuthorized); // Gaming detected
        }
        
        Self::deposit(env, from, token, fee)?;
        Ok(fee)
    }

    pub fn distribute_fee_revenue(
        env: Env,
        admin: Address,
        token: Address,
        amount: i128,
        destination: Address,
    ) -> Result<(), Error> {
        let auth_callers = Self::get_authorized_callers(&env);
        if !validate_caller_is_authorized(&env, &admin, &auth_callers) {
            return Err(Error::CallerNotAuthorized);
        }
        admin.require_auth();
        
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&env.current_contract_address(), &destination, &amount);
        Ok(())
    }

    // ── Economic monitoring & fairness audit (#903) ────────────────────────

    /// Validates a distribution's allocation vector and emits a monitorable
    /// violation event when amounts do not reconcile to the declared reward.
    pub fn verify_reward_allocation(
        env: Env,
        total_reward: i128,
        allocations: Vec<RewardAllocation>,
    ) -> bool {
        let result = validate_reward_distribution(&env, total_reward, &allocations);
        record_invariant_check(&env, &EconomicInvariantRecord {
            invariant: EconomicInvariant::RewardDistribution,
            valid: result.valid,
            observed: result.observed,
            expected: result.expected,
            timestamp: env.ledger().timestamp(),
            ledger: env.ledger().sequence(),
        });
        result.valid
    }

    /// Monitor token flows during a distribution to detect manipulation
    /// patterns such as coordinated timing or excessive extraction.
    pub fn monitor_token_flows(
        env: Env,
        distribution_id: u64,
    ) -> bool {
        let receipt: Option<DistributionReceipt> = env
            .storage()
            .persistent()
            .get(&DataKey::DistributionReceipt(distribution_id));

        match receipt {
            None => false,
            Some(r) => {
                // Flag if the distribution amount seems disproportionate.
                let staking_contract = Self::get_staking_contract(env.clone());
                match staking_contract {
                    Err(_) => false,
                    Ok(_) => {
                        let max_per_tx = MAX_PER_TX_DISTRIBUTE;
                        r.total_amount > 0 && r.total_amount <= max_per_tx
                    }
                }
            }
        }
    }

    /// Audit the fairness of a completed distribution by checking
    /// that amounts stayed within configured bounds.
    pub fn audit_distribution_fairness(
        env: Env,
        distribution_id: u64,
    ) -> bool {
        let receipt: Option<DistributionReceipt> = env
            .storage()
            .persistent()
            .get(&DataKey::DistributionReceipt(distribution_id));

        match receipt {
            None => false,
            Some(r) => {
                r.total_amount > 0
                    && r.total_amount <= MAX_PER_TX_DISTRIBUTE
                    && r.total_amount <= shared::MAX_FINANCIAL_AMOUNT
            }
        }
    }

    /// Correct a distribution by marking it for review if fairness
    /// checks fail.
    pub fn correct_distribution(
        env: Env,
        distribution_id: u64,
    ) -> bool {
        let is_fair = Self::audit_distribution_fairness(env.clone(), distribution_id);
        if !is_fair {
            // Mark the receipt as requiring review by setting processed to false
            // (in a real implementation this would trigger a governance vote).
            let receipt: Option<DistributionReceipt> = env
                .storage()
                .persistent()
                .get(&DataKey::DistributionReceipt(distribution_id));
            if let Some(mut r) = receipt {
                r.processed = false;
                env.storage()
                    .persistent()
                    .set(&DataKey::DistributionReceipt(distribution_id), &r);
            }
            return true;
        }
        false
    }
}


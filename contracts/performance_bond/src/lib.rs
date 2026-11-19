#![no_std]

use soroban_sdk::{
    contract, contractclient, contracterror, contractevent, contractimpl, contracttype,
    symbol_short, token, Address, Env, Symbol, Vec,
};

use shared::{
    get_all_params, get_param, init_protocol_params, set_param,
    key_min_bond, key_cooldown_days,
    DEFAULT_MIN_BOND, DEFAULT_COOLDOWN_DAYS, ReentrancyGuard,
    validate_amount_limits, MAX_FINANCIAL_AMOUNT,
    pagination::Pagination,
    EscrowRecord, EscrowStatus,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotAdmin = 3,
    InvalidAmount = 4,
    BelowMinimum = 5,
    AlreadyBonded = 6,
    NoBondFound = 7,
    StillInCooldown = 8,
    InsufficientBond = 9,
    /// The escrow contract does not confirm that the escrow was lost by the
    /// mentor being slashed (#1488).
    DisputeOutcomeNotConfirmed = 10,
    /// `slash_bond` was called before `set_escrow_contract`.
    EscrowNotConfigured = 11,
    /// The escrow has already been used to slash a bond.
    EscrowAlreadySlashed = 12,
    /// `reason` is not one of `no_show`, `dispute_lost` or `fraud`.
    UnknownSlashReason = 13,
}

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BondRecord {
    pub mentor: Address,
    pub amount: i128,
    pub posted_at: u64,
    pub last_slash_at: u64,
    pub slash_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashEvent {
    pub amount: i128,
    pub reason: Symbol,
    pub timestamp: u64,
    pub slash_count_at_time: u32,
}

// ---------------------------------------------------------------------------
// Contract Events
// ---------------------------------------------------------------------------

#[contractevent]
#[derive(Clone)]
pub struct BondPostedEvent {
    #[topic]
    pub contract: Symbol,
    #[topic]
    pub event_type: Symbol,
    pub mentor: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone)]
pub struct BondSlashedEvent {
    #[topic]
    pub contract: Symbol,
    #[topic]
    pub event_type: Symbol,
    pub mentor: Address,
    pub amount: i128,
    pub reason: Symbol,
    /// Escrow whose dispute outcome justified the slash.
    pub escrow_id: u64,
}

#[contractevent]
#[derive(Clone)]
pub struct BondToppedUpEvent {
    #[topic]
    pub contract: Symbol,
    #[topic]
    pub event_type: Symbol,
    pub mentor: Address,
    /// Amount added by this top-up.
    pub amount: i128,
    /// Bond size after the top-up.
    pub new_bond_amount: i128,
    /// Active slash count after any rehabilitation.
    pub slash_count: u32,
}

#[contractevent]
#[derive(Clone)]
pub struct BondWithdrawnEvent {
    #[topic]
    pub contract: Symbol,
    #[topic]
    pub event_type: Symbol,
    pub mentor: Address,
    pub amount: i128,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MINIMUM_BOND: i128 = 100_000_000; // 100 MNT (with 7 decimals)
const COOLDOWN_DAYS: u64 = 30;
const COOLDOWN_SECONDS: u64 = COOLDOWN_DAYS * 86_400;

const TTL_THRESHOLD: u32 = 500_000;
const TTL_BUMP: u32 = 9_000_000;

// Slash amounts (with 7 decimals)
const SLASH_NO_SHOW: i128 = 10_000_000; // 10 MNT
const SLASH_DISPUTE_LOST: i128 = 50_000_000; // 50 MNT

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    MntToken,
    InsurancePool,
    Bond(Address),
    SlashHistory(Address),
    PerfectSessionsCount(Address),
    /// Sequential index of slash events for a mentor (for pagination).
    SlashIndex(Address, u32),
    /// Count of slash events for a mentor.
    SlashCount(Address),
    /// Sequential index of bonded mentors — Address stored at each slot.
    BondIndex(u32),
    /// Total number of mentor bond entries ever posted.
    BondCount,
    /// Escrow contract queried to confirm dispute outcomes (#1488).
    EscrowContract,
    /// Marks an escrow that has already been used to slash a bond (#1488).
    SlashedEscrow(u64),
}

// ---------------------------------------------------------------------------
// Compile-time fallbacks (used when governance hasn't acted)
// ---------------------------------------------------------------------------
const COOLDOWN_SECONDS_DEFAULT: u64 = (DEFAULT_COOLDOWN_DAYS as u64) * 86_400;

/// Governance parameter: the smallest single top-up (in MNT base units) that
/// rehabilitates a mentor by reducing their slash count (#1487).
/// `0` disables rehabilitation.
pub fn key_topup_rehab_threshold() -> Symbol {
    symbol_short!("TOPUP_TH")
}

/// Default for `key_topup_rehab_threshold`: one minimum bond (100 MNT).
pub const DEFAULT_TOPUP_REHAB_THRESHOLD: i128 = DEFAULT_MIN_BOND;

/// How much one qualifying top-up reduces the slash count.
pub const TOPUP_REHAB_SLASH_REDUCTION: u32 = 1;

// ---------------------------------------------------------------------------
// Escrow verification (#1488)
// ---------------------------------------------------------------------------

/// The slice of the escrow contract that `slash_bond` queries.
#[contractclient(name = "EscrowClient")]
pub trait EscrowInterface {
    fn get_escrow(env: Env, escrow_id: u64) -> EscrowRecord;
}

/// Whether `escrow` is an escrow that `mentor` lost.
///
/// * `Refunded`: the whole amount went back to the learner.
/// * `Resolved`: an arbitrated split in which the learner received strictly
///   more than the mentor. The escrow contract records a resolved split by
///   storing the mentor's share in `net_amount` and the learner's share in
///   `platform_fee` (see `resolve_dispute`).
///
/// Anything else (`Pending`, `Active`, `Disputed`, `Released`) is not a lost
/// escrow, and neither is an escrow that belongs to another mentor.
fn mentor_lost_escrow(escrow: &EscrowRecord, mentor: &Address) -> bool {
    if escrow.mentor != *mentor {
        return false;
    }
    match escrow.status {
        EscrowStatus::Refunded => true,
        EscrowStatus::Resolved => escrow.platform_fee > escrow.net_amount,
        _ => false,
    }
}

/// Amount slashed for `reason`: 10 MNT for `no_show`, 50 MNT for
/// `dispute_lost`, the whole bond for `fraud`. Never more than the bond.
fn slash_amount_for(env: &Env, reason: &Symbol, bond_amount: i128) -> Result<i128, Error> {
    let amount = if *reason == symbol_short!("no_show") {
        SLASH_NO_SHOW
    } else if *reason == Symbol::new(env, "dispute_lost") {
        SLASH_DISPUTE_LOST
    } else if *reason == symbol_short!("fraud") {
        bond_amount
    } else {
        return Err(Error::UnknownSlashReason);
    };
    Ok(amount.min(bond_amount))
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct PerformanceBondContract;

#[contractimpl]
impl PerformanceBondContract {
    /// Initialize the performance bond contract.
    pub fn initialize(
        env: Env,
        admin: Address,
        mnt_token: Address,
        insurance_pool: Address,
        rbac_contract: Address,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MntToken, &mnt_token);
        env.storage()
            .instance()
            .set(&DataKey::InsurancePool, &insurance_pool);
        init_protocol_params(&env, &rbac_contract);
        Ok(())
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

    /// Post a performance bond.
    /// Minimum 100 MNT required to activate.
    ///
    /// Auth: mentor must authorize this call.
    pub fn post_bond(env: Env, mentor: Address, amount: i128) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, symbol_short!("BOND"));
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let minimum_bond = get_param(&env, &key_min_bond(), DEFAULT_MIN_BOND);
        if amount < minimum_bond {
            return Err(Error::BelowMinimum);
        }

        // Check if already bonded
        if env
            .storage()
            .persistent()
            .has(&DataKey::Bond(mentor.clone()))
        {
            return Err(Error::AlreadyBonded);
        }

        mentor.require_auth();

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MntToken)
            .ok_or(Error::NotInitialized)?;

        // Transfer MNT from mentor to this contract
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&mentor, &env.current_contract_address(), &amount);

        let record = BondRecord {
            mentor: mentor.clone(),
            amount,
            posted_at: env.ledger().timestamp(),
            last_slash_at: 0,
            slash_count: 0,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Bond(mentor.clone()), &record);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Bond(mentor.clone()), TTL_THRESHOLD, TTL_BUMP);

        // Maintain sequential bond index for pagination (#1210).
        let bond_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::BondCount)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::BondIndex(bond_count), &mentor);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::BondIndex(bond_count), TTL_THRESHOLD, TTL_BUMP);
        env.storage()
            .instance()
            .set(&DataKey::BondCount, &(bond_count + 1));

        BondPostedEvent {
            contract: symbol_short!("bond"),
            event_type: symbol_short!("posted"),
            mentor: mentor.clone(),
            amount,
        }
        .publish(&env);

        Ok(())
    }

    /// Point the bond contract at the escrow contract whose dispute outcomes
    /// justify a slash (#1488). Admin only.
    pub fn set_escrow_contract(env: Env, escrow: Address) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::EscrowContract, &escrow);
        Ok(())
    }

    /// The configured escrow contract, if any.
    pub fn get_escrow_contract(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::EscrowContract)
    }

    /// Slash a mentor's bond for an escrow they lost.
    /// Admin only.
    ///
    /// The escrow contract is queried first: the slash goes ahead only if
    /// escrow `escrow_id` belongs to `mentor` and ended against them (refunded
    /// to the learner, or resolved with the learner receiving more). Otherwise
    /// it fails with `Error::DisputeOutcomeNotConfirmed` and changes nothing,
    /// including when the escrow cannot be found. An escrow can justify one
    /// slash only (`Error::EscrowAlreadySlashed`).
    ///
    /// The amount follows from `reason`: "no_show" = 10 MNT, "dispute_lost" =
    /// 50 MNT, "fraud" = the full bond (capped at the bond in every case).
    pub fn slash_bond(
        env: Env,
        mentor: Address,
        escrow_id: u64,
        reason: Symbol,
    ) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;

        admin.require_auth();

        let mut record: BondRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bond(mentor.clone()))
            .ok_or(Error::NoBondFound)?;

        let amount = slash_amount_for(&env, &reason, record.amount)?;
        if !validate_amount_limits(amount, 1, MAX_FINANCIAL_AMOUNT) {
            return Err(Error::InvalidAmount);
        }

        if env
            .storage()
            .persistent()
            .has(&DataKey::SlashedEscrow(escrow_id))
        {
            return Err(Error::EscrowAlreadySlashed);
        }

        // Cross-contract check: did this mentor actually lose that escrow?
        let escrow_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::EscrowContract)
            .ok_or(Error::EscrowNotConfigured)?;
        let escrow = match EscrowClient::new(&env, &escrow_contract).try_get_escrow(&escrow_id) {
            Ok(Ok(escrow)) => escrow,
            // A missing escrow or any failure of the call is "not confirmed".
            _ => return Err(Error::DisputeOutcomeNotConfirmed),
        };
        if !mentor_lost_escrow(&escrow, &mentor) {
            return Err(Error::DisputeOutcomeNotConfirmed);
        }

        env.storage()
            .persistent()
            .set(&DataKey::SlashedEscrow(escrow_id), &true);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::SlashedEscrow(escrow_id), TTL_THRESHOLD, TTL_BUMP);

        let insurance_pool: Address = env
            .storage()
            .instance()
            .get(&DataKey::InsurancePool)
            .ok_or(Error::NotInitialized)?;

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MntToken)
            .ok_or(Error::NotInitialized)?;

        // Transfer slashed amount to insurance pool
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&env.current_contract_address(), &insurance_pool, &amount);

        // Update bond record
        record.amount -= amount;
        record.last_slash_at = env.ledger().timestamp();
        record.slash_count += 1;

        // Record slash event in SlashHistory (Issue #751)
        let mut history: Vec<SlashEvent> = env
            .storage()
            .persistent()
            .get(&DataKey::SlashHistory(mentor.clone()))
            .unwrap_or_else(|| Vec::new(&env));

        history.push_back(SlashEvent {
            amount,
            reason: reason.clone(),
            timestamp: env.ledger().timestamp(),
            slash_count_at_time: record.slash_count,
        });

        env.storage()
            .persistent()
            .set(&DataKey::SlashHistory(mentor.clone()), &history);

        // Maintain SlashIndex for pagination (#1272)
        let slash_count = env
            .storage()
            .persistent()
            .get::<_, u32>(&DataKey::SlashCount(mentor.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::SlashIndex(mentor.clone(), slash_count), &SlashEvent {
                amount,
                reason: reason.clone(),
                timestamp: env.ledger().timestamp(),
                slash_count_at_time: record.slash_count,
            });
        env.storage()
            .persistent()
            .set(&DataKey::SlashCount(mentor.clone()), &(slash_count + 1));

        env.storage()
            .persistent()
            .set(&DataKey::PerfectSessionsCount(mentor.clone()), &0u32);

        if record.amount == 0 {
            // Bond fully slashed - remove record
            env.storage()
                .persistent()
                .remove(&DataKey::Bond(mentor.clone()));
        } else {
            env.storage()
                .persistent()
                .set(&DataKey::Bond(mentor.clone()), &record);
        }

        BondSlashedEvent {
            contract: symbol_short!("bond"),
            event_type: symbol_short!("slashed"),
            mentor: mentor.clone(),
            amount,
            reason,
            escrow_id,
        }
        .publish(&env);

        Ok(())
    }

    /// Add to an existing bond. A top-up of at least the governance-set
    /// threshold also rehabilitates the mentor (#1487).
    ///
    /// The threshold is the `TOPUP_TH` protocol parameter (default 100 MNT,
    /// settable by `GOVERNANCE_ADMIN` through `set_param`; `0` turns
    /// rehabilitation off). A single top-up of `amount >= threshold` lowers the
    /// bond's active `slash_count` by `TOPUP_REHAB_SLASH_REDUCTION`, never
    /// below 0. Several small top-ups do not add up to a qualifying one. The
    /// immutable slash history and `get_slash_count` are not touched, and the
    /// release cooldown still runs from the last slash.
    ///
    /// Returns the slash count after the top-up.
    ///
    /// Auth: mentor must authorize this call.
    pub fn top_up_bond(env: Env, mentor: Address, amount: i128) -> Result<u32, Error> {
        let _guard = ReentrancyGuard::enter(&env, symbol_short!("BOND"));
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }
        if !validate_amount_limits(amount, 1, MAX_FINANCIAL_AMOUNT) {
            return Err(Error::InvalidAmount);
        }

        let mut record: BondRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bond(mentor.clone()))
            .ok_or(Error::NoBondFound)?;

        mentor.require_auth();

        let new_amount = record
            .amount
            .checked_add(amount)
            .ok_or(Error::InvalidAmount)?;

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MntToken)
            .ok_or(Error::NotInitialized)?;
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&mentor, &env.current_contract_address(), &amount);

        let threshold = get_param(
            &env,
            &key_topup_rehab_threshold(),
            DEFAULT_TOPUP_REHAB_THRESHOLD,
        );
        if threshold > 0 && amount >= threshold {
            record.slash_count = record
                .slash_count
                .saturating_sub(TOPUP_REHAB_SLASH_REDUCTION);
        }
        record.amount = new_amount;

        env.storage()
            .persistent()
            .set(&DataKey::Bond(mentor.clone()), &record);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Bond(mentor.clone()), TTL_THRESHOLD, TTL_BUMP);

        BondToppedUpEvent {
            contract: symbol_short!("bond"),
            event_type: symbol_short!("topped_up"),
            mentor,
            amount,
            new_bond_amount: new_amount,
            slash_count: record.slash_count,
        }
        .publish(&env);

        Ok(record.slash_count)
    }

    /// Query immutable slash history for a mentor (Issue #751).
    pub fn get_slash_history(env: Env, mentor: Address) -> Vec<SlashEvent> {
        env.storage()
            .persistent()
            .get(&DataKey::SlashHistory(mentor))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Returns the total count of slash events for a mentor (#1272).
    pub fn get_slash_count(env: Env, mentor: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::SlashCount(mentor))
            .unwrap_or(0)
    }

    /// Returns a paginated slice of a mentor's slash history (#1272).
    ///
    /// # Arguments
    /// * `mentor` - The mentor address to fetch slash history for
    /// * `offset` - Starting index in the slash history
    /// * `limit` - Maximum results to return (clamped to MAX_PAGE_SIZE)
    ///
    /// # Returns
    /// Vector of SlashEvent records for the given page.
    pub fn get_slash_history_page(
        env: Env,
        mentor: Address,
        offset: u32,
        limit: u32,
    ) -> Vec<SlashEvent> {
        use shared::pagination::Pagination;

        let total = Self::get_slash_count(env.clone(), mentor.clone());
        let (start, end) = Pagination::bounds(total, offset, limit);
        let mut result = Vec::new(&env);

        for idx in start..end {
            if let Some(event) = env
                .storage()
                .persistent()
                .get::<_, SlashEvent>(&DataKey::SlashIndex(mentor.clone(), idx))
            {
                result.push_back(event);
            }
        }

        result
    }

    /// Record a perfect 5-star session for mentor recovery.
    /// Completing 10 perfect sessions reduces effective slash penalty (Issue #751).
    pub fn record_perfect_session(env: Env, mentor: Address) -> u32 {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::PerfectSessionsCount(mentor.clone()))
            .unwrap_or(0);
        let new_count = count + 1;
        env.storage()
            .persistent()
            .set(&DataKey::PerfectSessionsCount(mentor), &new_count);
        new_count
    }

    /// Release a mentor's bond after cooldown period with no disputes.
    /// Mentor can withdraw their bond after 30 days with no slashes.
    ///
    /// Auth: mentor must authorize this call.
    pub fn release_bond(env: Env, mentor: Address) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, symbol_short!("BOND"));
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        let record: BondRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bond(mentor.clone()))
            .ok_or(Error::NoBondFound)?;

        mentor.require_auth();

        let now = env.ledger().timestamp();

        // Check cooldown period since last slash
        let cooldown_secs = (get_param(&env, &key_cooldown_days(), DEFAULT_COOLDOWN_DAYS) as u64)
            .saturating_mul(86_400);
        if record.last_slash_at > 0 && now < record.last_slash_at + cooldown_secs {
            return Err(Error::StillInCooldown);
        }

        // Also check from posting time if no slashes
        if record.last_slash_at == 0 && now < record.posted_at + cooldown_secs {
            return Err(Error::StillInCooldown);
        }

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MntToken)
            .ok_or(Error::NotInitialized)?;

        // Transfer bond back to mentor
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&env.current_contract_address(), &mentor, &record.amount);

        // Remove bond record
        env.storage()
            .persistent()
            .remove(&DataKey::Bond(mentor.clone()));

        BondWithdrawnEvent {
            contract: symbol_short!("bond"),
            event_type: symbol_short!("withdrawn"),
            mentor: mentor.clone(),
            amount: record.amount,
        }
        .publish(&env);

        Ok(())
    }

    /// Returns the total count of bonds ever posted.
    pub fn get_bond_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::BondCount)
            .unwrap_or(0)
    }

    /// Returns a paginated slice of `BondRecord`s for all posted bonds (#1210).
    pub fn get_bonds_page(env: Env, offset: u32, limit: u32) -> Vec<BondRecord> {
        let total = Self::get_bond_count(env.clone());
        let (start, end) = Pagination::bounds(total, offset, limit);
        let mut result = Vec::new(&env);

        for idx in start..end {
            if let Some(mentor) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::BondIndex(idx))
            {
                if let Some(record) = env
                    .storage()
                    .persistent()
                    .get::<_, BondRecord>(&DataKey::Bond(mentor))
                {
                    result.push_back(record);
                }
            }
        }

        result
    }

    /// Get bond record for a mentor.
    pub fn get_bond(env: Env, mentor: Address) -> Result<BondRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Bond(mentor))
            .ok_or(Error::NoBondFound)
    }

    /// Check if a mentor is bonded (has active bond).
    /// Used by escrow to verify mentor can accept bookings.
    pub fn is_bonded(env: Env, mentor: Address) -> bool {
        env.storage().persistent().has(&DataKey::Bond(mentor))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    // Mock MNT Token
    #[contracttype]
    #[derive(Clone)]
    pub enum MockDataKey {
        Balance(Address),
    }

    #[contract]
    pub struct MockMNT;

    #[contractimpl]
    impl MockMNT {
        pub fn mint(env: Env, to: Address, amount: i128) {
            let bal: i128 = env
                .storage()
                .persistent()
                .get(&MockDataKey::Balance(to.clone()))
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(to), &(bal + amount));
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&MockDataKey::Balance(id))
                .unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            let from_bal = Self::balance(env.clone(), from.clone());
            assert!(from_bal >= amount, "Insufficient balance");
            let to_bal = Self::balance(env.clone(), to.clone());
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(from), &(from_bal - amount));
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(to), &(to_bal + amount));
        }
    }

    // Mock escrow contract: returns whatever record a test stored, and panics
    // for unknown ids like the real `get_escrow`.
    #[contracttype]
    #[derive(Clone)]
    pub enum MockEscrowKey {
        Record(u64),
    }

    #[contract]
    pub struct MockEscrow;

    #[contractimpl]
    impl MockEscrow {
        pub fn set_escrow(env: Env, escrow_id: u64, record: EscrowRecord) {
            env.storage()
                .persistent()
                .set(&MockEscrowKey::Record(escrow_id), &record);
        }

        pub fn get_escrow(env: Env, escrow_id: u64) -> EscrowRecord {
            env.storage()
                .persistent()
                .get(&MockEscrowKey::Record(escrow_id))
                .expect("Escrow not found")
        }
    }

    // Mock RBAC contract that grants every role, so `set_param` can run.
    #[contract]
    pub struct MockRbac;

    #[contractimpl]
    impl MockRbac {
        pub fn has_role(_env: Env, _role: Symbol, _addr: Address) -> bool {
            true
        }
    }

    const BOND: i128 = 200_000_000;

    struct Fixture {
        env: Env,
        bond_id: Address,
        mnt_id: Address,
        escrow_id: Address,
        admin: Address,
        insurance_pool: Address,
    }

    impl Fixture {
        /// Bond contract wired to a mock escrow contract.
        fn setup() -> Self {
            let f = Self::setup_without_escrow();
            f.client().set_escrow_contract(&f.escrow_id);
            f
        }

        fn setup_without_escrow() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let insurance_pool = Address::generate(&env);
            let mnt_id = env.register_contract(None, MockMNT);
            let escrow_id = env.register_contract(None, MockEscrow);
            let rbac_id = env.register_contract(None, MockRbac);

            let bond_id = env.register_contract(None, PerformanceBondContract);
            PerformanceBondContractClient::new(&env, &bond_id).initialize(
                &admin,
                &mnt_id,
                &insurance_pool,
                &rbac_id,
            );

            Fixture {
                env,
                bond_id,
                mnt_id,
                escrow_id,
                admin,
                insurance_pool,
            }
        }

        fn client(&self) -> PerformanceBondContractClient {
            PerformanceBondContractClient::new(&self.env, &self.bond_id)
        }

        fn mnt(&self) -> MockMNTClient {
            MockMNTClient::new(&self.env, &self.mnt_id)
        }

        fn escrow(&self) -> MockEscrowClient {
            MockEscrowClient::new(&self.env, &self.escrow_id)
        }

        fn fund(&self, addr: &Address, amount: i128) {
            self.mnt().mint(addr, &amount);
        }

        /// A funded mentor with a bond of `BOND`.
        fn bonded_mentor(&self) -> Address {
            let mentor = Address::generate(&self.env);
            self.fund(&mentor, 10 * BOND);
            self.client().post_bond(&mentor, &BOND);
            mentor
        }

        /// Store an escrow of `mentor` in the mock escrow contract.
        fn put_escrow(
            &self,
            id: u64,
            mentor: &Address,
            status: EscrowStatus,
            mentor_share: i128,
            learner_share: i128,
        ) {
            let record = EscrowRecord {
                id,
                mentor: mentor.clone(),
                learner: Address::generate(&self.env),
                amount: mentor_share + learner_share,
                session_id: symbol_short!("sess"),
                status,
                created_at: 0,
                token_address: self.mnt_id.clone(),
                // For a resolved escrow these hold the mentor / learner shares.
                platform_fee: learner_share,
                net_amount: mentor_share,
                session_end_time: 0,
                auto_release_delay: 0,
                dispute_reason: symbol_short!("none"),
                resolved_at: 0,
                usd_amount: 0,
                quoted_token_amount: 0,
                send_asset: self.mnt_id.clone(),
                dest_asset: self.mnt_id.clone(),
                total_sessions: 1,
                sessions_completed: 0,
            };
            self.escrow().set_escrow(&id, &record);
        }

        /// An escrow the mentor lost outright (refunded to the learner).
        fn put_lost_escrow(&self, id: u64, mentor: &Address) {
            self.put_escrow(id, mentor, EscrowStatus::Refunded, 0, 50_000_000);
        }

        /// Slash `mentor` for a no-show against a freshly lost escrow.
        fn slash_no_show(&self, mentor: &Address, id: u64) {
            self.put_lost_escrow(id, mentor);
            self.client()
                .slash_bond(mentor, &id, &symbol_short!("no_show"));
        }
    }

    // -----------------------------------------------------------------------
    // Posting and releasing
    // -----------------------------------------------------------------------

    #[test]
    fn test_post_bond() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 200_000_000);

        f.client().post_bond(&mentor, &100_000_000);

        assert!(f.client().is_bonded(&mentor));
        let bond = f.client().get_bond(&mentor);
        assert_eq!(bond.amount, 100_000_000);
        assert_eq!(f.mnt().balance(&f.bond_id), 100_000_000);
    }

    #[test]
    fn test_post_bond_below_minimum() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 50_000_000);

        let result = f.client().try_post_bond(&mentor, &50_000_000);
        assert_eq!(result, Err(Ok(Error::BelowMinimum)));
    }

    #[test]
    fn test_release_bond_after_cooldown() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 100_000_000);

        f.client().post_bond(&mentor, &100_000_000);

        // Try to release immediately - should fail
        let result = f.client().try_release_bond(&mentor);
        assert_eq!(result, Err(Ok(Error::StillInCooldown)));

        // Advance past cooldown
        f.env.ledger().set_timestamp(31 * 86_400);

        f.client().release_bond(&mentor);
        assert!(!f.client().is_bonded(&mentor));
        assert_eq!(f.mnt().balance(&mentor), 100_000_000);
    }

    #[test]
    fn test_is_bonded_booking_gate() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);

        // Not bonded
        assert!(!f.client().is_bonded(&mentor));

        // Post bond
        f.fund(&mentor, 100_000_000);
        f.client().post_bond(&mentor, &100_000_000);

        // Now bonded
        assert!(f.client().is_bonded(&mentor));
    }

    // -----------------------------------------------------------------------
    // Slashing and cross-contract escrow verification (#1488)
    // -----------------------------------------------------------------------

    #[test]
    fn test_slash_bond() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();

        // 10 MNT for a no-show, against an escrow the mentor lost.
        f.slash_no_show(&mentor, 1);

        let bond = f.client().get_bond(&mentor);
        assert_eq!(bond.amount, BOND - 10_000_000);
        assert_eq!(bond.slash_count, 1);
        assert_eq!(f.mnt().balance(&f.insurance_pool), 10_000_000);
    }

    #[test]
    fn test_slash_amount_follows_the_reason() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();

        f.put_lost_escrow(1, &mentor);
        f.client()
            .slash_bond(&mentor, &1, &Symbol::new(&f.env, "dispute_lost"));
        assert_eq!(f.client().get_bond(&mentor).amount, BOND - 50_000_000);

        f.put_lost_escrow(2, &mentor);
        f.client()
            .slash_bond(&mentor, &2, &symbol_short!("no_show"));
        assert_eq!(f.client().get_bond(&mentor).amount, BOND - 60_000_000);
        assert_eq!(f.mnt().balance(&f.insurance_pool), 60_000_000);
    }

    #[test]
    fn test_full_slash() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();

        // Fraud takes the full bond.
        f.put_lost_escrow(1, &mentor);
        f.client()
            .slash_bond(&mentor, &1, &symbol_short!("fraud"));

        assert!(!f.client().is_bonded(&mentor));
        assert_eq!(f.mnt().balance(&f.insurance_pool), BOND);
    }

    #[test]
    fn test_slash_confirmed_by_a_resolved_escrow_the_learner_won() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        // Arbitrated 30 / 70 in the learner's favor.
        f.put_escrow(1, &mentor, EscrowStatus::Resolved, 30_000_000, 70_000_000);

        f.client()
            .slash_bond(&mentor, &1, &Symbol::new(&f.env, "dispute_lost"));

        assert_eq!(f.client().get_bond(&mentor).slash_count, 1);
    }

    #[test]
    fn test_slash_rejected_when_the_mentor_won_the_dispute() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        // Resolved 80 / 20 for the mentor, and an exact split: neither is a loss.
        f.put_escrow(1, &mentor, EscrowStatus::Resolved, 80_000_000, 20_000_000);
        f.put_escrow(2, &mentor, EscrowStatus::Resolved, 50_000_000, 50_000_000);

        for id in [1u64, 2] {
            assert_eq!(
                f.client()
                    .try_slash_bond(&mentor, &id, &symbol_short!("no_show")),
                Err(Ok(Error::DisputeOutcomeNotConfirmed))
            );
        }
        assert_eq!(f.client().get_bond(&mentor).amount, BOND);
    }

    #[test]
    fn test_slash_rejected_for_escrows_that_did_not_end_against_the_mentor() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        let statuses = [
            EscrowStatus::Pending,
            EscrowStatus::Active,
            EscrowStatus::Disputed,
            EscrowStatus::Released,
        ];
        for (index, status) in statuses.into_iter().enumerate() {
            let id = index as u64 + 1;
            f.put_escrow(id, &mentor, status, 0, 50_000_000);
            assert_eq!(
                f.client()
                    .try_slash_bond(&mentor, &id, &symbol_short!("no_show")),
                Err(Ok(Error::DisputeOutcomeNotConfirmed))
            );
        }
    }

    #[test]
    fn test_slash_rejected_for_another_mentors_escrow() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        let other = Address::generate(&f.env);
        f.put_lost_escrow(1, &other);

        assert_eq!(
            f.client()
                .try_slash_bond(&mentor, &1, &symbol_short!("no_show")),
            Err(Ok(Error::DisputeOutcomeNotConfirmed))
        );
    }

    #[test]
    fn test_slash_rejected_when_the_escrow_does_not_exist() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();

        assert_eq!(
            f.client()
                .try_slash_bond(&mentor, &999, &symbol_short!("no_show")),
            Err(Ok(Error::DisputeOutcomeNotConfirmed))
        );
    }

    #[test]
    fn test_rejected_slash_changes_nothing() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        f.put_escrow(1, &mentor, EscrowStatus::Released, 50_000_000, 0);

        let _ = f
            .client()
            .try_slash_bond(&mentor, &1, &symbol_short!("fraud"));

        let bond = f.client().get_bond(&mentor);
        assert_eq!(bond.amount, BOND);
        assert_eq!(bond.slash_count, 0);
        assert_eq!(f.client().get_slash_count(&mentor), 0);
        assert_eq!(f.mnt().balance(&f.insurance_pool), 0);
    }

    #[test]
    fn test_slash_requires_a_configured_escrow_contract() {
        let f = Fixture::setup_without_escrow();
        let mentor = f.bonded_mentor();

        assert_eq!(f.client().get_escrow_contract(), None);
        assert_eq!(
            f.client()
                .try_slash_bond(&mentor, &1, &symbol_short!("no_show")),
            Err(Ok(Error::EscrowNotConfigured))
        );

        f.client().set_escrow_contract(&f.escrow_id);
        assert_eq!(f.client().get_escrow_contract(), Some(f.escrow_id.clone()));
    }

    #[test]
    fn test_one_escrow_justifies_one_slash() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        f.slash_no_show(&mentor, 1);

        assert_eq!(
            f.client()
                .try_slash_bond(&mentor, &1, &symbol_short!("no_show")),
            Err(Ok(Error::EscrowAlreadySlashed))
        );
        assert_eq!(f.client().get_bond(&mentor).slash_count, 1);
    }

    #[test]
    fn test_unknown_slash_reason_is_rejected() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        f.put_lost_escrow(1, &mentor);

        assert_eq!(
            f.client()
                .try_slash_bond(&mentor, &1, &symbol_short!("whim")),
            Err(Ok(Error::UnknownSlashReason))
        );
    }

    #[test]
    fn test_slash_without_a_bond_fails() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.put_lost_escrow(1, &mentor);

        assert_eq!(
            f.client()
                .try_slash_bond(&mentor, &1, &symbol_short!("no_show")),
            Err(Ok(Error::NoBondFound))
        );
    }

    // -----------------------------------------------------------------------
    // Slash-count rehabilitation on top-up (#1487)
    // -----------------------------------------------------------------------

    #[test]
    fn test_top_up_adds_to_the_bond() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();

        let count = f.client().top_up_bond(&mentor, &25_000_000);

        assert_eq!(count, 0);
        assert_eq!(f.client().get_bond(&mentor).amount, BOND + 25_000_000);
        assert_eq!(f.mnt().balance(&f.bond_id), BOND + 25_000_000);
    }

    #[test]
    fn test_top_up_below_the_threshold_does_not_rehabilitate() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        f.slash_no_show(&mentor, 1);

        let count = f
            .client()
            .top_up_bond(&mentor, &(DEFAULT_TOPUP_REHAB_THRESHOLD - 1));

        assert_eq!(count, 1);
        assert_eq!(f.client().get_bond(&mentor).slash_count, 1);
    }

    #[test]
    fn test_top_up_at_the_threshold_reduces_the_slash_count() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        f.slash_no_show(&mentor, 1);
        f.slash_no_show(&mentor, 2);
        assert_eq!(f.client().get_bond(&mentor).slash_count, 2);

        let count = f
            .client()
            .top_up_bond(&mentor, &DEFAULT_TOPUP_REHAB_THRESHOLD);

        assert_eq!(count, 1);
        assert_eq!(f.client().get_bond(&mentor).slash_count, 1);
    }

    #[test]
    fn test_slash_count_never_goes_below_zero() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();

        // Nothing to rehabilitate: stays at 0 rather than underflowing.
        assert_eq!(
            f.client()
                .top_up_bond(&mentor, &DEFAULT_TOPUP_REHAB_THRESHOLD),
            0
        );

        f.slash_no_show(&mentor, 1);
        for _ in 0..3 {
            f.client()
                .top_up_bond(&mentor, &DEFAULT_TOPUP_REHAB_THRESHOLD);
        }
        assert_eq!(f.client().get_bond(&mentor).slash_count, 0);
    }

    #[test]
    fn test_small_top_ups_do_not_add_up_to_a_qualifying_one() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        f.slash_no_show(&mentor, 1);

        let part = DEFAULT_TOPUP_REHAB_THRESHOLD / 2;
        f.client().top_up_bond(&mentor, &part);
        f.client().top_up_bond(&mentor, &part);

        assert_eq!(f.client().get_bond(&mentor).slash_count, 1);
    }

    #[test]
    fn test_rehabilitation_leaves_the_slash_history_intact() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        f.slash_no_show(&mentor, 1);

        f.client()
            .top_up_bond(&mentor, &DEFAULT_TOPUP_REHAB_THRESHOLD);

        assert_eq!(f.client().get_bond(&mentor).slash_count, 0);
        assert_eq!(f.client().get_slash_count(&mentor), 1);
        assert_eq!(f.client().get_slash_history(&mentor).len(), 1);
    }

    #[test]
    fn test_governance_can_change_the_threshold() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        f.slash_no_show(&mentor, 1);
        f.slash_no_show(&mentor, 2);

        // Lower the bar to 10 MNT.
        f.client()
            .set_param(&f.admin, &key_topup_rehab_threshold(), &10_000_000);
        assert_eq!(f.client().top_up_bond(&mentor, &10_000_000), 1);

        // Just below the new bar does nothing.
        assert_eq!(f.client().top_up_bond(&mentor, &9_999_999), 1);
    }

    #[test]
    fn test_a_zero_threshold_turns_rehabilitation_off() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        f.slash_no_show(&mentor, 1);

        f.client()
            .set_param(&f.admin, &key_topup_rehab_threshold(), &0);

        assert_eq!(f.client().top_up_bond(&mentor, &BOND), 1);
    }

    #[test]
    fn test_top_up_validation() {
        let f = Fixture::setup();
        let mentor = f.bonded_mentor();
        let stranger = Address::generate(&f.env);
        f.fund(&stranger, BOND);

        assert_eq!(
            f.client().try_top_up_bond(&mentor, &0),
            Err(Ok(Error::InvalidAmount))
        );
        assert_eq!(
            f.client().try_top_up_bond(&mentor, &-1),
            Err(Ok(Error::InvalidAmount))
        );
        assert_eq!(
            f.client().try_top_up_bond(&stranger, &BOND),
            Err(Ok(Error::NoBondFound))
        );
    }
}

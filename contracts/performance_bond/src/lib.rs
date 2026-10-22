#![no_std]

use soroban_sdk::{
    contract, contracterror, contractevent, contractimpl, contracttype, symbol_short, token,
    Address, Env, Symbol, Vec,
};

use shared::{
    get_all_params, get_param, init_protocol_params, set_param,
    key_min_bond, key_cooldown_days,
    DEFAULT_MIN_BOND, DEFAULT_COOLDOWN_DAYS, ReentrancyGuard,
    validate_amount_limits, MAX_FINANCIAL_AMOUNT,
    pagination::Pagination,
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
#[allow(dead_code)]
const SLASH_NO_SHOW: i128 = 10_000_000; // 10 MNT
#[allow(dead_code)]
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
}

// ---------------------------------------------------------------------------
// Compile-time fallbacks (used when governance hasn't acted)
// ---------------------------------------------------------------------------
const COOLDOWN_SECONDS_DEFAULT: u64 = (DEFAULT_COOLDOWN_DAYS as u64) * 86_400;

// Slash amounts (with 7 decimals)
#[allow(dead_code)]
const SLASH_NO_SHOW: i128 = 10_000_000; // 10 MNT
#[allow(dead_code)]
const SLASH_DISPUTE_LOST: i128 = 50_000_000; // 50 MNT

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

    /// Slash a mentor's bond.
    /// Admin only.
    ///
    /// reason: "no_show" = 10 MNT, "dispute_lost" = 50 MNT, "fraud" = full bond
    pub fn slash_bond(
        env: Env,
        mentor: Address,
        amount: i128,
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

        if !validate_amount_limits(amount, 1, MAX_FINANCIAL_AMOUNT) {
            return Err(Error::InvalidAmount);
        }

        if amount > record.amount {
            return Err(Error::InsufficientBond);
        }

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
        }
        .publish(&env);

        Ok(())
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

        let total = Self::get_slash_count(&env, mentor.clone());
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

    struct Fixture {
        env: Env,
        bond_id: Address,
        mnt_id: Address,
        admin: Address,
        insurance_pool: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let insurance_pool = Address::generate(&env);
            let mnt_id = env.register_contract(None, MockMNT);

            let bond_id = env.register_contract(None, PerformanceBondContract);
            PerformanceBondContractClient::new(&env, &bond_id).initialize(
                &admin,
                &mnt_id,
                &insurance_pool,
                &admin,  // rbac_contract — use admin address in tests
            );

            Fixture {
                env,
                bond_id,
                mnt_id,
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

        fn fund(&self, addr: &Address, amount: i128) {
            self.mnt().mint(addr, &amount);
        }
    }

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
    fn test_slash_bond() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 200_000_000);

        f.client().post_bond(&mentor, &200_000_000);

        // Slash 10 MNT for no-show
        f.client()
            .slash_bond(&mentor, &10_000_000, &symbol_short!("no_show"));

        let bond = f.client().get_bond(&mentor);
        assert_eq!(bond.amount, 190_000_000);
        assert_eq!(bond.slash_count, 1);
        assert_eq!(f.mnt().balance(&f.insurance_pool), 10_000_000);
    }

    #[test]
    fn test_full_slash() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 100_000_000);

        f.client().post_bond(&mentor, &100_000_000);

        // Full slash for fraud
        f.client()
            .slash_bond(&mentor, &100_000_000, &symbol_short!("fraud"));

        assert!(!f.client().is_bonded(&mentor));
        assert_eq!(f.mnt().balance(&f.insurance_pool), 100_000_000);
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
}

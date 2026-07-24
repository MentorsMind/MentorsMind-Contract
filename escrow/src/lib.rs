#![no_std]
use shared::{EscrowRecord, EscrowStatus};
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol, Vec, IntoVal, BytesN};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MilestoneStatus {
    Pending,
    Completed,
    Disputed,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MilestoneSpec {
    pub description_hash: BytesN<32>,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MilestoneEscrow {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub total_amount: i128,
    pub milestones: Vec<MilestoneSpec>,
    pub milestone_statuses: Vec<MilestoneStatus>,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    pub platform_fee: i128,
    pub net_amount: i128,
}

/// Type alias for shared EscrowRecord to maintain backward compatibility.
pub type Escrow = EscrowRecord;

/// Legacy escrow struct for backward-compatible deserialization.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EscrowLegacy {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub session_id: Symbol,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    pub platform_fee: i128,
    pub net_amount: i128,
    pub session_end_time: u64,
    pub auto_release_delay: u64,
    pub dispute_reason: Symbol,
    pub resolved_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowCreatedEventData {
    pub escrow_id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub session_id: Symbol,
    pub token_address: Address,
    pub session_end_time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowReleasedEventData {
    pub escrow_id: u64,
    pub mentor: Address,
    pub amount: i128,
    pub net_amount: i128,
    pub platform_fee: i128,
    pub token_address: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowAutoReleasedEventData {
    pub escrow_id: u64,
    pub time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeOpenedEventData {
    pub escrow_id: u64,
    pub caller: Address,
    pub reason: Symbol,
    pub token_address: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeResolvedEventData {
    pub escrow_id: u64,
    pub mentor_pct: u32,
    pub mentor_amount: i128,
    pub learner_amount: i128,
    pub token_address: Address,
    pub time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowRefundedEventData {
    pub escrow_id: u64,
    pub learner: Address,
    pub amount: i128,
    pub token_address: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewSubmittedEventData {
    pub caller: Address,
    pub reason: Symbol,
    pub mentor: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeDistributedEventData {
    pub escrow_id: u64,
    pub gross_amount: i128,
    pub platform_fee: i128,
    pub net_amount: i128,
    pub token_address: Address,
}

/// Event data emitted when a token is approved or rejected from the whitelist.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenApprovalEventData {
    pub token_address: Address,
    pub approved: bool,
}

// ---------------------------------------------------------------------------
// DataKey enum — typed storage key for all persistent state
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Treasury,
    FeeBps,
    EscrowCount,
    AutoRelDelay,
    Escrow(u64),
    ApprovedToken(Address),
}

// ---------------------------------------------------------------------------
// Storage keys (Symbol-based, used alongside DataKey where appropriate)
// ---------------------------------------------------------------------------

const ESCROW_COUNT: Symbol = symbol_short!("ESC_CNT");
const MILESTONE_ESCROW_COUNT: Symbol = symbol_short!("MESC_CNT");
const ADMIN: Symbol = symbol_short!("ADMIN");
const TREASURY: Symbol = symbol_short!("TREASURY");
const FEE_BPS: Symbol = symbol_short!("FEE_BPS");
/// Default auto-release delay in seconds (configurable at init).
const AUTO_REL_DLY: Symbol = symbol_short!("AR_DELAY");
const SESSION_KEY: Symbol = symbol_short!("SESSION");
const ORACLE_ID: Symbol = symbol_short!("ORACLE");
const ORACLE_MAX_AGE: Symbol = symbol_short!("OR_AGE");
const MENTOR_ESCROWS: Symbol = symbol_short!("MNT_ESC");
const LEARNER_ESCROWS: Symbol = symbol_short!("LRN_ESC");
const MAX_FEE_BPS: u32 = 1_000;

/// Default auto-release delay: 72 hours in seconds.
const DEFAULT_AUTO_RELEASE_DELAY: u64 = 72 * 60 * 60;

// Approved token registry key prefix: ("APRV_TOK", address → bool
const APPROVED_TOKEN_KEY: Symbol = symbol_short!("APRV_TOK");

// Cache key for admin address
const CACHE_ADMIN_KEY: Symbol = symbol_short!("CADM_KEY");

// Dynamic fee constants
const DYNAMIC_FEE_ENABLED: Symbol = symbol_short!("DYN_FEE");
const LIQUIDITY_POOL: Symbol = symbol_short!("LIQ_POOL");
const PRICE_CACHE: Symbol = symbol_short!("PRC_CACH");
const PRICE_CACHE_TIME: Symbol = symbol_short!("PRC_TIME");
const DEFAULT_FEE_BPS: u32 = 500;

// ---------------------------------------------------------------------------
// TTL constants (in ledgers; ~5 s/ledger → 1 000 000 ≈ 57 days)
// ---------------------------------------------------------------------------

const ESCROW_TTL_THRESHOLD: u32 = 500_000;
const ESCROW_TTL_BUMP: u32 = 1_000_000;

// Cache TTL constants for frequently accessed data
const CACHE_TTL_THRESHOLD: u32 = 100_000;
const CACHE_TTL_BUMP: u32 = 500_000;

// Cache statistics keys
const CACHE_HITS_KEY: Symbol = symbol_short!("CACH_HIT");
const CACHE_MISSES_KEY: Symbol = symbol_short!("CACH_MIS");

/// Cache of accrued yield per escrow to avoid repeated get_value calls.
const YIELD_ACCRUED_CACHE: Symbol = symbol_short!("YLD_ACC");

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    // -----------------------------------------------------------------------
    // Admin / initialization
    // -----------------------------------------------------------------------

    /// Initialize the contract with an admin, treasury, initial fee, approved
    /// tokens, and an optional auto-release delay.
    ///
    /// - `fee_bps`: platform fee in basis points (e.g. 500 = 5%). Must be ≤ 1 000 (10%).
    /// - `treasury`: address that receives the platform fee on every release.
    /// - `auto_release_delay_secs`: seconds after session end before funds
    ///   auto-release to the mentor. Pass `0` to use the default (72 hours).
    /// - Approved tokens must satisfy SEP-41 (XLM, USDC, PYUSD, …).
    ///
    /// Calling this a second time will panic — persistent storage ensures the
    /// `ADMIN` key survives ledger archival so the guard cannot be bypassed.
    pub fn initialize(
        env: Env,
        admin: Address,
        treasury: Address,
        fee_bps: u32,
        approved_tokens: soroban_sdk::Vec<Address>,
        auto_release_delay_secs: u64,
    ) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }

        if fee_bps > MAX_FEE_BPS {
            panic!("Fee exceeds maximum (1000 bps)");
        }

        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.storage().persistent().set(&DataKey::Treasury, &treasury);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.storage().persistent().set(&DataKey::FeeBps, &fee_bps);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.storage().persistent().set(&DataKey::EscrowCount, &0u64);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::EscrowCount, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.storage().persistent().set(&MILESTONE_ESCROW_COUNT, &0u64);
        env.storage()
            .persistent()
            .extend_ttl(&MILESTONE_ESCROW_COUNT, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // Store configurable auto-release delay; fall back to 72 hours if 0.
        let delay = if auto_release_delay_secs == 0 {
            DEFAULT_AUTO_RELEASE_DELAY
        } else {
            auto_release_delay_secs
        };
        env.storage().persistent().set(&DataKey::AutoRelDelay, &delay);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::AutoRelDelay, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // Register each approved token and emit events
        for token_addr in approved_tokens.iter() {
            Self::_set_token_approved(&env, &token_addr, true);
            env.events().publish(
                (Symbol::new(&env, "Token"), Symbol::new(&env, "Approved")),
                TokenApprovalEventData {
                    token_address: token_addr,
                    approved: true,
                },
            );
        }
    }

    /// Update the platform fee — admin only, capped at 1 000 bps (10%).
    pub fn update_fee(env: Env, new_fee_bps: u32) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        if new_fee_bps > MAX_FEE_BPS {
            panic!("Fee exceeds maximum (1000 bps)");
        }

        env.storage().persistent().set(&DataKey::FeeBps, &new_fee_bps);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
    }

    /// Get dynamic fee based on MNT/USDC price from liquidity pool.
    /// Returns fee in basis points (bps).
    ///
    /// Fee schedule:
    /// - Price < $0.10 → 500 bps (5%)
    /// - Price $0.10–$0.50 → 400 bps (4%)
    /// - Price $0.50–$1.00 → 300 bps (3%)
    /// - Price > $1.00 → 200 bps (2%)
    pub fn get_dynamic_fee(env: Env) -> u32 {
        let dynamic_enabled: bool = env
            .storage()
            .instance()
            .get(&DYNAMIC_FEE_ENABLED)
            .unwrap_or(true);

        if !dynamic_enabled {
            return env.storage().persistent().get(&DataKey::FeeBps).unwrap_or(DEFAULT_FEE_BPS);
        }

        let current_ledger = env.ledger().sequence();
        let cached_ledger: u32 = env
            .storage()
            .instance()
            .get(&PRICE_CACHE_TIME)
            .unwrap_or(0);

        if cached_ledger == current_ledger {
            if let Some(cached_price) = env.storage().instance().get::<_, i128>(&PRICE_CACHE) {
                return Self::_calculate_fee_from_price(cached_price);
            }
        }

        let price = Self::_fetch_mnt_usdc_price(&env);

        env.storage().instance().set(&PRICE_CACHE, &price);
        env.storage().instance().set(&PRICE_CACHE_TIME, &current_ledger);

        Self::_calculate_fee_from_price(price)
    }

    fn _calculate_fee_from_price(price: i128) -> u32 {
        if price <= 0 {
            return DEFAULT_FEE_BPS;
        }

        let threshold_010 = 1_000_000;
        let threshold_050 = 5_000_000;
        let threshold_100 = 10_000_000;

        if price < threshold_010 {
            500
        } else if price < threshold_050 {
            400
        } else if price < threshold_100 {
            300
        } else {
            200
        }
    }

    fn _fetch_mnt_usdc_price(env: &Env) -> i128 {
        let pool_address: Option<Address> = env.storage().instance().get(&LIQUIDITY_POOL);

        if let Some(_pool) = pool_address {
            // TODO: Implement actual pool contract integration
            // Placeholder: return $0.75 (7,500,000) for testing
            return 7_500_000;
        }

        0
    }

    /// Set liquidity pool address (admin only)
    pub fn set_liquidity_pool(env: Env, pool_address: Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        env.storage().instance().set(&LIQUIDITY_POOL, &pool_address);
    }

    /// Enable or disable dynamic fee (admin only)
    pub fn set_dynamic_fee_enabled(env: Env, enabled: bool) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        env.storage().instance().set(&DYNAMIC_FEE_ENABLED, &enabled);
    }

    /// Update the treasury address — admin only.
    pub fn update_treasury(env: Env, new_treasury: Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        env.storage().persistent().set(&DataKey::Treasury, &new_treasury);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
    }

    /// Add or remove an approved token (admin only).
    /// Emits a TokenApproved or TokenRejected event.
    pub fn set_approved_token(env: Env, token_address: Address, approved: bool) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        Self::_set_token_approved(&env, &token_address, approved);

        // Emit token approval/rejection event
        if approved {
            env.events().publish(
                (Symbol::new(&env, "Token"), Symbol::new(&env, "Approved")),
                TokenApprovalEventData {
                    token_address,
                    approved: true,
                },
            );
        } else {
            env.events().publish(
                (Symbol::new(&env, "Token"), Symbol::new(&env, "Rejected")),
                TokenApprovalEventData {
                    token_address,
                    approved: false,
                },
            );
        }
    }

    // -----------------------------------------------------------------------
    // Escrow lifecycle
    // -----------------------------------------------------------------------

    /// Create a new escrow.
    ///
    /// Auth: Only the learner can create an escrow for themselves.
    ///
    /// Transfers `amount` tokens from `learner` to the contract.
    ///
    /// - `session_end_time`: unix timestamp (seconds) marking when the session
    ///   ends. After this plus the contract's `auto_release_delay`, anyone may
    ///   call `try_auto_release` to release funds to the mentor.
    ///
    /// Panics if:
    /// - `amount` ≤ 0
    /// - `token_address` is not on the approved whitelist
    /// - learner's on-chain balance is insufficient
    /// - Caller is not the learner
    pub fn create_escrow(
        env: Env,
        mentor: Address,
        learner: Address,
        amount: i128,
        session_id: Symbol,
        token_address: Address,
        session_end_time: u64,
        total_sessions: u32,
    ) -> u64 {
        Self::_create_escrow_internal(
            env,
            mentor,
            learner,
            amount,
            session_id,
            token_address.clone(),
            session_end_time,
            0,
            amount,
            token_address.clone(),
            token_address,
            total_sessions,
        )
    }

    /// Release funds to the mentor (called by learner or admin).
    ///
    /// Calculates the platform fee (`gross * fee_bps / 10_000`), transfers the
    /// fee to the treasury, and transfers the remainder to the mentor.
    pub fn release_funds(env: Env, caller: Address, escrow_id: u64) {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Admin not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // Auth check: caller must be learner OR admin
        caller.require_auth();
        if caller != escrow.learner && caller != admin {
            panic!("Caller not authorized");
        }

        Self::_do_release(&env, &mut escrow, &key);
    }

    /// Release a partial amount (one session worth) from a multi-session escrow.
    pub fn release_partial(env: Env, caller: Address, escrow_id: u64) {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env.storage().persistent().get(&key).expect("Escrow not found");

        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        if escrow.sessions_completed >= escrow.total_sessions {
            panic!("All sessions already released");
        }

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Admin not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // Auth check: caller must be learner OR admin
        caller.require_auth();
        if caller != escrow.learner && caller != admin {
            panic!("Caller not authorized");
        }

        // Calculate amount to release: total_amount / total_sessions
        // For the last session, release whatever is remaining to handle rounding.
        let amount_to_release = if escrow.sessions_completed + 1 == escrow.total_sessions {
            escrow.amount
        } else {
            escrow.quoted_token_amount
                .checked_div(escrow.total_sessions as i128)
                .expect("Division error")
        };

        let fee_bps: u32 = env.storage().persistent().get(&DataKey::FeeBps).unwrap_or(0u32);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let platform_fee: i128 = amount_to_release
            .checked_mul(fee_bps as i128)
            .expect("Overflow")
            .checked_div(10_000)
            .expect("Division error");
        let net_amount: i128 = amount_to_release
            .checked_sub(platform_fee)
            .expect("Underflow");

        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let token_client = token::Client::new(&env, &escrow.token_address);

        if platform_fee > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &platform_fee);
        }

        token_client.transfer(&env.current_contract_address(), &escrow.mentor, &net_amount);

        escrow.sessions_completed += 1;
        escrow.amount = escrow.amount.checked_sub(amount_to_release).expect("Underflow");
        escrow.platform_fee = escrow.platform_fee.checked_add(platform_fee).expect("Overflow");
        escrow.net_amount = escrow.net_amount.checked_add(net_amount).expect("Overflow");

        if escrow.sessions_completed == escrow.total_sessions {
            escrow.status = EscrowStatus::Released;
            let session_key = (SESSION_KEY, escrow.session_id.clone());
            env.storage().persistent().remove(&session_key);
        }

        env.storage().persistent().set(&key, &escrow);

        env.events().publish(
            (symbol_short!("partial"), escrow.id),
            (escrow.sessions_completed, amount_to_release),
        );
    }

    /// Admin release — admin can force-release any active escrow.
    pub fn admin_release(env: Env, escrow_id: u64) {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage().persistent().extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow = Self::_load_escrow(&env, &key);
        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        let admin: Address = env.storage().persistent().get(&DataKey::Admin).expect("Admin not found");
        env.storage().persistent().extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        env.events().publish(
            (symbol_short!("Escrow"), symbol_short!("adm_rel"), escrow_id),
            (escrow_id, env.ledger().timestamp()),
        );

        Self::_do_release(&env, &mut escrow, &key);
    }

    /// Permissionless auto-release.
    ///
    /// Anyone may call this once `env.ledger().timestamp() >=
    /// escrow.session_end_time + escrow.auto_release_delay` and the escrow is
    /// still `Active`.
    pub fn try_auto_release(env: Env, escrow_id: u64) {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        let now = env.ledger().timestamp();
        let release_after = escrow
            .session_end_time
            .checked_add(escrow.auto_release_delay)
            .expect("Timestamp overflow");

        if now < release_after {
            panic!("Auto-release window has not elapsed");
        }

        // Emit a dedicated `auto_released` event before the internal release
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "AutoReleased"), escrow_id),
            EscrowAutoReleasedEventData { escrow_id, time: now },
        );

        Self::_do_release(&env, &mut escrow, &key);
    }

    /// Open a dispute (called by mentor or learner).
    pub fn dispute(env: Env, caller: Address, escrow_id: u64, reason: Symbol) {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        // Auth check: caller must be mentor OR learner
        caller.require_auth();
        if caller != escrow.mentor && caller != escrow.learner {
            panic!("Caller not authorized to dispute");
        }

        escrow.status = EscrowStatus::Disputed;
        escrow.dispute_reason = reason.clone();
        env.storage().persistent().set(&key, &escrow);

        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "DisputeOpened"), escrow_id),
            DisputeOpenedEventData {
                escrow_id,
                caller,
                reason,
                token_address: escrow.token_address,
            },
        );
    }

    /// Resolve a disputed escrow by splitting funds between mentor and learner.
    ///
    /// Admin only. Can only be called on `Disputed` escrows.
    ///
    /// - `mentor_pct`: percentage (0–100) of `escrow.amount` sent to the mentor.
    ///   The remainder (`100 - mentor_pct`) goes to the learner. No platform fee
    ///   is deducted — the full escrowed amount is split between the parties.
    pub fn resolve_dispute(env: Env, escrow_id: u64, mentor_pct: u32) {
        // --- Admin auth ---
        let admin: Address = env.storage().persistent().get(&DataKey::Admin).expect("Not initialized");
        env.storage().persistent().extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        if mentor_pct > 100 {
            panic!("mentor_pct must be 0–100");
        }

        // --- Load escrow ---
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage().persistent().extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env.storage().persistent().get(&key).expect("Escrow not found");

        if escrow.status != EscrowStatus::Disputed {
            panic!("Escrow is not in Disputed status");
        }

        let now = env.ledger().timestamp();
        let token_client = soroban_sdk::token::Client::new(&env, &escrow.token_address);

        let mentor_amount = escrow.amount
            .checked_mul(mentor_pct as i128)
            .expect("Overflow")
            .checked_div(100)
            .expect("Division error");
        let learner_amount = escrow.amount
            .checked_sub(mentor_amount)
            .expect("Underflow");

        if mentor_amount > 0 {
            token_client.transfer(&env.current_contract_address(), &escrow.mentor, &mentor_amount);
        }
        if learner_amount > 0 {
            token_client.transfer(&env.current_contract_address(), &escrow.learner, &learner_amount);
        }

        // Update escrow record
        escrow.status = EscrowStatus::Resolved;
        escrow.net_amount = mentor_amount;
        escrow.platform_fee = learner_amount; // repurposed: learner share in resolved state
        escrow.resolved_at = now;
        env.storage().persistent().set(&key, &escrow);

        // Emit event
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "DisputeResolved"), escrow_id),
            DisputeResolvedEventData {
                escrow_id,
                mentor_pct,
                mentor_amount,
                learner_amount,
                token_address: escrow.token_address.clone(),
                time: now,
            },
        );
    }

    /// Refund tokens to the learner (admin only).
    ///
    /// Can be called on `Active` or `Disputed` escrows; panics if already
    /// `Released`, `Refunded`, or `Resolved`.
    pub fn refund(env: Env, escrow_id: u64) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Admin not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        if escrow.status == EscrowStatus::Released
            || escrow.status == EscrowStatus::Refunded
            || escrow.status == EscrowStatus::Resolved
        {
            panic!("Cannot refund");
        }

        // Transfer tokens: contract → learner
        let token_client = token::Client::new(&env, &escrow.token_address);
        token_client.transfer(
            &env.current_contract_address(),
            &escrow.learner,
            &escrow.amount,
        );

        escrow.status = EscrowStatus::Refunded;
        env.storage().persistent().set(&key, &escrow);

        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "Refunded"), escrow_id),
            EscrowRefundedEventData {
                escrow_id,
                learner: escrow.learner.clone(),
                amount: escrow.amount,
                token_address: escrow.token_address,
            },
        );
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    pub fn get_escrow(env: Env, escrow_id: u64) -> Escrow {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found")
    }

    pub fn get_escrow_count(env: Env) -> u64 {
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::EscrowCount, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage().persistent().get(&DataKey::EscrowCount).unwrap_or(0)
    }

    pub fn get_fee_bps(env: Env) -> u32 {
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage().persistent().get(&DataKey::FeeBps).unwrap_or(0)
    }

    pub fn get_treasury(env: Env) -> Address {
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not set")
    }

    pub fn get_auto_release_delay(env: Env) -> u64 {
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::AutoRelDelay, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&DataKey::AutoRelDelay)
            .unwrap_or(DEFAULT_AUTO_RELEASE_DELAY)
    }

    pub fn is_token_approved(env: Env, token_address: Address) -> bool {
        Self::_is_token_approved(&env, &token_address)
    }

    pub fn get_escrows_by_mentor(env: Env, mentor: Address, page: u32, page_size: u32) -> Vec<Escrow> {
        let page_size = if page_size > 50 { 50 } else { page_size };
        let mentor_key = (MENTOR_ESCROWS, mentor);
        let mentor_escrows: Vec<u64> = env.storage().persistent().get(&mentor_key).unwrap_or(Vec::new(&env));
        let start = page.checked_mul(page_size).unwrap_or(0);
        let mut result = Vec::new(&env);

        if start >= mentor_escrows.len() {
            return result;
        }

        let end = (start + page_size).min(mentor_escrows.len());
        for i in start..end {
            let id = mentor_escrows.get(i).unwrap();
            let key = (symbol_short!("ESCROW"), id);
            if let Some(escrow) = env.storage().persistent().get::<_, Escrow>(&key) {
                result.push_back(escrow);
            }
        }
        result
    }

    pub fn get_escrows_by_learner(env: Env, learner: Address, page: u32, page_size: u32) -> Vec<Escrow> {
        let page_size = if page_size > 50 { 50 } else { page_size };
        let learner_key = (LEARNER_ESCROWS, learner);
        let learner_escrows: Vec<u64> = env.storage().persistent().get(&learner_key).unwrap_or(Vec::new(&env));
        let start = page.checked_mul(page_size).unwrap_or(0);
        let mut result = Vec::new(&env);

        if start >= learner_escrows.len() {
            return result;
        }

        let end = (start + page_size).min(learner_escrows.len());
        for i in start..end {
            let id = learner_escrows.get(i).unwrap();
            let key = (symbol_short!("ESCROW"), id);
            if let Some(escrow) = env.storage().persistent().get::<_, Escrow>(&key) {
                result.push_back(escrow);
            }
        }
        result
    }

    pub fn get_escrows_by_status(env: Env, status: EscrowStatus) -> Vec<u64> {
        let count: u64 = env.storage().persistent().get(&DataKey::EscrowCount).unwrap_or(0u64);
        let mut result = Vec::new(&env);

        for i in 1..=count {
            let key = (symbol_short!("ESCROW"), i);
            if let Some(escrow) = env.storage().persistent().get::<_, Escrow>(&key) {
                if escrow.status == status {
                    result.push_back(i);
                }
            }
        }
        result
    }

    /// Submit a review for a completed escrow (learner only).
    pub fn submit_review(env: Env, caller: Address, escrow_id: u64, reason: Symbol) {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        // Only learner can submit review
        caller.require_auth();
        if caller != escrow.learner {
            panic!("Only learner can submit review");
        }

        // Can only review released escrows
        if escrow.status != EscrowStatus::Released {
            panic!("Can only review released escrows");
        }

        // Store review reason in a separate key
        let review_key = (symbol_short!("REVIEW"), escrow_id);
        env.storage().persistent().set(&review_key, &reason);
        env.storage()
            .persistent()
            .extend_ttl(&review_key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "ReviewSubmitted"), escrow_id),
            ReviewSubmittedEventData {
                caller,
                reason,
                mentor: escrow.mentor,
            },
        );
    }

    // -----------------------------------------------------------------------
    // USD / path-payment escrow creation
    // -----------------------------------------------------------------------

    pub fn create_escrow_usd(
        env: Env,
        mentor: Address,
        learner: Address,
        usd_amount: i128,
        token_address: Address,
        total_sessions: u32,
    ) -> u64 {
        let oracle: Address = env.storage().persistent().get(&ORACLE_ID).expect("oracle not set");
        let max_age: u64 = env.storage().persistent().get(&ORACLE_MAX_AGE).unwrap_or(300);
        let oracle_sym = Symbol::new(&env, "get_price");
        let (price, updated_at): (i128, u64) = env.invoke_contract(
            &oracle,
            &oracle_sym,
            (Symbol::new(&env, "USD"),).into_val(&env),
        );
        let now = env.ledger().timestamp();
        if now.saturating_sub(updated_at) > max_age || price <= 0 {
            panic!("stale oracle");
        }
        let token_amount = usd_amount
            .checked_mul(10_000_000)
            .expect("overflow")
            .checked_div(price)
            .expect("div");
        env.events().publish(
            (symbol_short!("Escrow"), symbol_short!("usd_rate"), learner.clone()),
            (usd_amount, price, token_amount),
        );
        Self::_create_escrow_internal(
            env,
            mentor,
            learner,
            token_amount,
            symbol_short!("USD_SES"),
            token_address.clone(),
            now,
            usd_amount,
            token_amount,
            token_address.clone(),
            token_address,
            total_sessions,
        )
    }

    pub fn create_escrow_with_path_payment(
        env: Env,
        learner: Address,
        mentor: Address,
        send_asset: Address,
        send_max: i128,
        dest_asset: Address,
        dest_amount: i128,
        _path: Vec<Address>,
        total_sessions: u32,
    ) -> u64 {
        // *** WHITELIST FIX: Validate BOTH send_asset and dest_asset ***
        if !Self::_is_token_approved(&env, &send_asset) {
            panic!("Send asset token not approved");
        }
        if !Self::_is_token_approved(&env, &dest_asset) {
            panic!("Dest asset token not approved");
        }

        if send_max < dest_amount {
            panic!("path slippage exceeded");
        }
        let rate_scaled = if dest_amount == 0 {
            0
        } else {
            send_max * 10_000_000 / dest_amount
        };
        env.events().publish(
            (symbol_short!("Escrow"), symbol_short!("path_pay"), learner.clone()),
            rate_scaled,
        );
        Self::_create_escrow_internal(
            env,
            mentor,
            learner,
            dest_amount,
            symbol_short!("PATHPAY"),
            dest_asset.clone(),
            0,
            0,
            dest_amount,
            send_asset,
            dest_asset,
            total_sessions,
        )
    }

    // -----------------------------------------------------------------------
    // Milestone escrow functions
    // -----------------------------------------------------------------------

    pub fn create_milestone_escrow(
        env: Env,
        mentor: Address,
        learner: Address,
        milestones: Vec<MilestoneSpec>,
        token_address: Address,
    ) -> u64 {
        if milestones.is_empty() {
            panic!("At least one milestone required");
        }

        // *** WHITELIST VALIDATION ***
        if !Self::_is_token_approved(&env, &token_address) {
            panic!("Token not approved");
        }

        let total_amount = milestones
            .iter()
            .fold(0i128, |acc, m| acc.checked_add(m.amount).expect("Amount overflow"));

        if total_amount <= 0 {
            panic!("Total amount must be greater than zero");
        }

        learner.require_auth();

        let token_client = token::Client::new(&env, &token_address);
        if token_client.balance(&learner) < total_amount {
            panic!("Insufficient token balance");
        }

        let mut count: u64 = env
            .storage()
            .persistent()
            .get(&MILESTONE_ESCROW_COUNT)
            .unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&MILESTONE_ESCROW_COUNT, &count);
        env.storage()
            .persistent()
            .extend_ttl(&MILESTONE_ESCROW_COUNT, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        token_client.transfer(&learner, &env.current_contract_address(), &total_amount);

        let mut milestone_statuses: Vec<MilestoneStatus> = Vec::new(&env);
        for _i in 0..milestones.len() {
            milestone_statuses.push_back(MilestoneStatus::Pending);
        }

        let milestone_escrow = MilestoneEscrow {
            id: count,
            mentor: mentor.clone(),
            learner: learner.clone(),
            total_amount,
            milestones: milestones.clone(),
            milestone_statuses,
            status: EscrowStatus::Active,
            created_at: env.ledger().timestamp(),
            token_address: token_address.clone(),
            platform_fee: 0,
            net_amount: 0,
        };

        let key = (symbol_short!("MESCROW"), count);
        env.storage().persistent().set(&key, &milestone_escrow);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.events().publish(
            (symbol_short!("ms_creat"), count),
            (mentor, learner, total_amount, milestones.len()),
        );

        count
    }

    pub fn complete_milestone(env: Env, escrow_id: u64, milestone_index: u32) {
        let key = (symbol_short!("MESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut milestone_escrow: MilestoneEscrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Milestone escrow not found");

        if milestone_escrow.status != EscrowStatus::Active {
            panic!("Milestone escrow not active");
        }

        if (milestone_index as u32) >= milestone_escrow.milestones.len() {
            panic!("Invalid milestone index");
        }

        let current_status = milestone_escrow
            .milestone_statuses
            .get(milestone_index)
            .unwrap();
        if current_status != MilestoneStatus::Pending {
            panic!("Milestone not pending");
        }

        milestone_escrow.learner.require_auth();

        let milestone = milestone_escrow.milestones.get(milestone_index).unwrap();
        let fee_bps: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::FeeBps)
            .unwrap_or(0u32);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let platform_fee: i128 = milestone
            .amount
            .checked_mul(fee_bps as i128)
            .expect("Overflow")
            .checked_div(10_000)
            .expect("Division error");
        let net_amount: i128 = milestone.amount.checked_sub(platform_fee).expect("Underflow");

        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let token_client = token::Client::new(&env, &milestone_escrow.token_address);

        if platform_fee > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &platform_fee);
        }

        token_client.transfer(
            &env.current_contract_address(),
            &milestone_escrow.mentor,
            &net_amount,
        );

        milestone_escrow
            .milestone_statuses
            .set(milestone_index, MilestoneStatus::Completed);
        milestone_escrow.platform_fee = milestone_escrow
            .platform_fee
            .checked_add(platform_fee)
            .expect("Overflow");
        milestone_escrow.net_amount = milestone_escrow
            .net_amount
            .checked_add(net_amount)
            .expect("Overflow");

        let all_completed = milestone_escrow
            .milestone_statuses
            .iter()
            .all(|s| s == MilestoneStatus::Completed);
        if all_completed {
            milestone_escrow.status = EscrowStatus::Released;
        }

        env.storage().persistent().set(&key, &milestone_escrow);

        env.events().publish(
            (symbol_short!("ms_compl"), escrow_id),
            (milestone_index, milestone.amount, net_amount),
        );
    }

    pub fn dispute_milestone(env: Env, escrow_id: u64, milestone_index: u32, reason: Symbol) {
        let key = (symbol_short!("MESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut milestone_escrow: MilestoneEscrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Milestone escrow not found");

        if milestone_escrow.status != EscrowStatus::Active {
            panic!("Milestone escrow not active");
        }

        if (milestone_index as u32) >= milestone_escrow.milestones.len() {
            panic!("Invalid milestone index");
        }

        let current_status = milestone_escrow
            .milestone_statuses
            .get(milestone_index)
            .unwrap();
        if current_status != MilestoneStatus::Pending {
            panic!("Milestone not pending");
        }

        milestone_escrow.mentor.require_auth();
        milestone_escrow.learner.require_auth();

        milestone_escrow
            .milestone_statuses
            .set(milestone_index, MilestoneStatus::Disputed);
        milestone_escrow.status = EscrowStatus::Disputed;

        env.storage().persistent().set(&key, &milestone_escrow);

        env.events().publish(
            (symbol_short!("ms_disp"), escrow_id),
            (milestone_index, reason),
        );
    }

    pub fn get_milestone_escrow(env: Env, escrow_id: u64) -> MilestoneEscrow {
        let key = (symbol_short!("MESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&key)
            .expect("Milestone escrow not found")
    }

    pub fn get_milestone_escrow_count(env: Env) -> u64 {
        env.storage()
            .persistent()
            .extend_ttl(&MILESTONE_ESCROW_COUNT, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&MILESTONE_ESCROW_COUNT)
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Shared release logic used by both `release_funds` and `try_auto_release`.
    fn _do_release(env: &Env, escrow: &mut Escrow, key: &(Symbol, u64)) {
        let release_amount = escrow.amount;
        let fee_bps: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::FeeBps)
            .unwrap_or(DEFAULT_FEE_BPS);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let platform_fee: i128 = release_amount
            .checked_mul(fee_bps as i128)
            .expect("Overflow")
            .checked_div(10_000)
            .expect("Division error");
        let net_amount: i128 = release_amount
            .checked_sub(platform_fee)
            .expect("Underflow");

        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let token_client = soroban_sdk::token::Client::new(env, &escrow.token_address);

        if platform_fee > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &platform_fee);
        }

        token_client.transfer(&env.current_contract_address(), &escrow.mentor, &net_amount);

        escrow.status = EscrowStatus::Released;
        escrow.platform_fee = escrow.platform_fee.checked_add(platform_fee).expect("Overflow");
        escrow.net_amount = escrow.net_amount.checked_add(net_amount).expect("Overflow");
        escrow.amount = 0; // all remaining amount is released
        env.storage().persistent().set(key, escrow);

        env.events().publish(
            (Symbol::new(env, "Escrow"), Symbol::new(env, "Released"), escrow.id),
            EscrowReleasedEventData {
                escrow_id: escrow.id,
                mentor: escrow.mentor.clone(),
                amount: release_amount,
                net_amount,
                platform_fee,
                token_address: escrow.token_address.clone(),
            },
        );

        env.events().publish(
            (Symbol::new(env, "Escrow"), Symbol::new(env, "FeeDistributed"), escrow.id),
            FeeDistributedEventData {
                escrow_id: escrow.id,
                gross_amount: release_amount,
                platform_fee,
                net_amount,
                token_address: escrow.token_address.clone(),
            },
        );
    }

    /// Internal token whitelist setter. Stores approval state in persistent storage.
    fn _set_token_approved(env: &Env, token_address: &Address, approved: bool) {
        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage().persistent().set(&key, &approved);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
    }

    /// Internal token whitelist checker.
    /// Returns `true` only if the token was explicitly approved via `set_approved_token`
    /// or during `initialize`. Any unknown/unregistered token returns `false`.
    fn _is_token_approved(env: &Env, token_address: &Address) -> bool {
        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage()
            .persistent()
            .get::<_, bool>(&key)
            .unwrap_or(false)
    }

    /// Shared escrow creation logic with strict token whitelist validation.
    ///
    /// This function is the single entry point for all escrow creation paths
    /// (create_escrow, create_escrow_usd, create_escrow_with_path_payment).
    /// It enforces:
    /// 1. Amount > 0
    /// 2. Token is on the approved whitelist
    /// 3. Learner has sufficient balance
    /// 4. Learner authorization
    /// 5. Session uniqueness
    fn _create_escrow_internal(
        env: Env,
        mentor: Address,
        learner: Address,
        amount: i128,
        session_id: Symbol,
        token_address: Address,
        session_end_time: u64,
        usd_amount: i128,
        quoted_token_amount: i128,
        send_asset: Address,
        dest_asset: Address,
        total_sessions: u32,
    ) -> u64 {
        // --- Strict input validation ---
        if amount <= 0 {
            panic!("Amount must be greater than zero");
        }

        // *** STRICT WHITELIST VALIDATION ***
        // This is the critical security check: only admin-approved tokens are accepted.
        // The token address is checked against the persistent whitelist registry.
        // There is no fallback, no bypass, no secondary path that skips this check.
        if !Self::_is_token_approved(&env, &token_address) {
            panic!("Token not approved");
        }

        // Also validate send_asset and dest_asset if they differ from token_address
        if send_asset != token_address && !Self::_is_token_approved(&env, &send_asset) {
            panic!("Send asset token not approved");
        }
        if dest_asset != token_address && !Self::_is_token_approved(&env, &dest_asset) {
            panic!("Dest asset token not approved");
        }

        // --- Auth ---
        learner.require_auth();

        // --- Balance check ---
        let token_client = token::Client::new(&env, &token_address);
        if token_client.balance(&learner) < amount {
            panic!("Insufficient token balance");
        }

        // --- Session uniqueness ---
        let session_key = (SESSION_KEY, session_id.clone());
        if env.storage().persistent().has(&session_key) {
            panic!("Session already exists");
        }
        env.storage().persistent().set(&session_key, &true);

        // --- Auto-release delay ---
        let auto_release_delay: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AutoRelDelay)
            .unwrap_or(DEFAULT_AUTO_RELEASE_DELAY);

        // --- Counter ---
        let mut count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowCount)
            .unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&DataKey::EscrowCount, &count);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::EscrowCount, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // --- Transfer tokens into escrow ---
        token_client.transfer(&learner, &env.current_contract_address(), &amount);

        // --- Build escrow record ---
        let escrow = Escrow {
            id: count,
            mentor: mentor.clone(),
            learner: learner.clone(),
            amount,
            session_id: session_id.clone(),
            status: EscrowStatus::Active,
            created_at: env.ledger().timestamp(),
            token_address: token_address.clone(),
            platform_fee: 0,
            net_amount: 0,
            session_end_time,
            auto_release_delay,
            dispute_reason: symbol_short!(""),
            resolved_at: 0,
            usd_amount,
            quoted_token_amount,
            send_asset,
            dest_asset,
            total_sessions,
            sessions_completed: 0,
        };
        let key = (symbol_short!("ESCROW"), count);
        env.storage().persistent().set(&key, &escrow);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // --- Update index maps ---
        let mentor_key = (MENTOR_ESCROWS, mentor.clone());
        let mut mentor_escrows: Vec<u64> = env
            .storage()
            .persistent()
            .get(&mentor_key)
            .unwrap_or(Vec::new(&env));
        mentor_escrows.push_back(count);
        env.storage().persistent().set(&mentor_key, &mentor_escrows);
        env.storage()
            .persistent()
            .extend_ttl(&mentor_key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let learner_key = (LEARNER_ESCROWS, learner.clone());
        let mut learner_escrows: Vec<u64> = env
            .storage()
            .persistent()
            .get(&learner_key)
            .unwrap_or(Vec::new(&env));
        learner_escrows.push_back(count);
        env.storage()
            .persistent()
            .set(&learner_key, &learner_escrows);
        env.storage()
            .persistent()
            .extend_ttl(&learner_key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // --- Emit event ---
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "Created"), count),
            EscrowCreatedEventData {
                escrow_id: count,
                mentor,
                learner,
                amount,
                session_id,
                token_address,
                session_end_time,
            },
        );

        count
    }

    /// Load an escrow with backward-compatible deserialization.
    fn _load_escrow(env: &Env, key: &(Symbol, u64)) -> Escrow {
        if let Some(current) = env.storage().persistent().get::<_, Escrow>(key) {
            return current;
        }
        if let Some(old) = env.storage().persistent().get::<_, EscrowLegacy>(key) {
            return Escrow {
                id: old.id,
                mentor: old.mentor,
                learner: old.learner,
                amount: old.amount,
                session_id: old.session_id,
                status: old.status,
                created_at: old.created_at,
                token_address: old.token_address.clone(),
                platform_fee: old.platform_fee,
                net_amount: old.net_amount,
                session_end_time: old.session_end_time,
                auto_release_delay: old.auto_release_delay,
                dispute_reason: old.dispute_reason,
                resolved_at: old.resolved_at,
                usd_amount: 0,
                quoted_token_amount: old.amount,
                send_asset: old.token_address.clone(),
                dest_asset: old.token_address,
                total_sessions: 1,
                sessions_completed: 0,
            };
        }
        panic!("Escrow not found");
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger, Events},
        token::{Client as TokenClient, StellarAssetClient},
        Address, Env, Vec, IntoVal, Symbol,
    };

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn create_token<'a>(env: &'a Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
        let token_address = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        let sac = StellarAssetClient::new(env, &token_address);
        (token_address, sac)
    }

    fn advance_time(env: &Env, seconds: u64) {
        let current = env.ledger().timestamp();
        env.ledger().set_timestamp(current + seconds);
    }

    struct TestFixture {
        env: Env,
        contract_id: Address,
        admin: Address,
        mentor: Address,
        learner: Address,
        treasury: Address,
        token_address: Address,
        sac: StellarAssetClient<'static>,
    }

    impl TestFixture {
        fn setup() -> Self {
            Self::setup_full(0, 0)
        }

        fn setup_with_fee(fee_bps: u32) -> Self {
            Self::setup_full(fee_bps, 0)
        }

        fn setup_full(fee_bps: u32, auto_release_delay: u64) -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let mentor = Address::generate(&env);
            let learner = Address::generate(&env);
            let treasury = Address::generate(&env);

            let (token_address, sac) = create_token(&env, &admin);

            let contract_id = env.register_contract(None, EscrowContract);
            let client = EscrowContractClient::new(&env, &contract_id);

            let mut approved_tokens = Vec::new(&env);
            approved_tokens.push_back(token_address.clone());

            client.initialize(
                &admin,
                &treasury,
                &fee_bps,
                &approved_tokens,
                &auto_release_delay,
            );

            // Mint tokens to learner
            sac.mint(&learner, &100_000);

            // We need to leak the env to get 'static lifetime for sac
            // Instead, we'll store the pieces separately
            let sac_static = unsafe {
                core::mem::transmute::<StellarAssetClient<'_>, StellarAssetClient<'static>>(sac)
            };

            TestFixture {
                env,
                contract_id,
                admin,
                mentor,
                learner,
                treasury,
                token_address,
                sac: sac_static,
            }
        }

        fn client(&self) -> EscrowContractClient {
            EscrowContractClient::new(&self.env, &self.contract_id)
        }

        fn token(&self) -> TokenClient {
            TokenClient::new(&self.env, &self.token_address)
        }

        fn create_escrow_at(&self, amount: i128, session_end_time: u64) -> u64 {
            self.client().create_escrow(
                &self.mentor,
                &self.learner,
                &amount,
                &symbol_short!("S1"),
                &self.token_address,
                &session_end_time,
                &1u32,
            )
        }

        fn open_dispute(&self, escrow_id: u64) {
            self.client()
                .dispute(&self.learner, &escrow_id, &symbol_short!("NO_SHOW"));
        }
    }

    fn setup_disputed(f: &TestFixture) -> u64 {
        let id = f.create_escrow_at(1_000, 0);
        f.open_dispute(id);
        id
    }

    // -----------------------------------------------------------------------
    // Dynamic fee tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_dynamic_fee_price_below_10_cents() {
        let fee = EscrowContract::_calculate_fee_from_price(500_000);
        assert_eq!(fee, 500);
    }

    #[test]
    fn test_dynamic_fee_price_10_to_50_cents() {
        let fee = EscrowContract::_calculate_fee_from_price(3_000_000);
        assert_eq!(fee, 400);
    }

    #[test]
    fn test_dynamic_fee_price_50_to_100_cents() {
        let fee = EscrowContract::_calculate_fee_from_price(7_500_000);
        assert_eq!(fee, 300);
    }

    #[test]
    fn test_dynamic_fee_price_above_100_cents() {
        let fee = EscrowContract::_calculate_fee_from_price(15_000_000);
        assert_eq!(fee, 200);
    }

    #[test]
    fn test_dynamic_fee_fallback_when_price_zero() {
        let fee = EscrowContract::_calculate_fee_from_price(0);
        assert_eq!(fee, 500);
    }

    // -----------------------------------------------------------------------
    // initialize
    // -----------------------------------------------------------------------

    #[test]
    fn test_initialize_stores_config() {
        let f = TestFixture::setup_full(500, 3_600);
        let client = f.client();
        assert_eq!(client.get_fee_bps(), 500);
        assert_eq!(client.get_treasury(), f.treasury);
        assert_eq!(client.get_auto_release_delay(), 3_600);
        assert!(client.is_token_approved(&f.token_address));
    }

    #[test]
    fn test_initialize_double_init_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, EscrowContract);
        let client = EscrowContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let approved: Vec<Address> = Vec::new(&env);
        client.initialize(&admin, &treasury, &500u32, &approved, &0u64);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.initialize(&admin, &treasury, &500u32, &approved, &0u64);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_initialize_fee_over_cap_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, EscrowContract);
        let client = EscrowContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let approved: Vec<Address> = Vec::new(&env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.initialize(&admin, &treasury, &1_001u32, &approved, &0u64);
        }));
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // create_escrow
    // -----------------------------------------------------------------------

    #[test]
    fn test_create_escrow_valid() {
        let f = TestFixture::setup();
        let token = f.token();
        let learner_before = token.balance(&f.learner);
        let id = f.create_escrow_at(1_000, 0);
        assert_eq!(id, 1);
        assert_eq!(token.balance(&f.learner), learner_before - 1_000);
        assert_eq!(token.balance(&f.contract_id), 1_000);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Active);
        assert_eq!(e.mentor, f.mentor);
        assert_eq!(e.learner, f.learner);
    }

    #[test]
    fn test_create_escrow_counter_increments() {
        let f = TestFixture::setup();
        assert_eq!(f.client().get_escrow_count(), 0);
        // Note: each call uses session_id "S1" which would conflict;
        // we use different amounts to verify counter
        let id1 = f.client().create_escrow(
            &f.mentor, &f.learner, &500, &symbol_short!("S1"),
            &f.token_address, &0u64, &1u32,
        );
        let id2 = f.client().create_escrow(
            &f.mentor, &f.learner, &500, &symbol_short!("S2"),
            &f.token_address, &0u64, &1u32,
        );
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(f.client().get_escrow_count(), 2);
    }

    #[test]
    fn test_create_escrow_zero_amount_panics() {
        let f = TestFixture::setup();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.create_escrow_at(0, 0);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_create_escrow_negative_amount_panics() {
        let f = TestFixture::setup();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.create_escrow_at(-1, 0);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_create_escrow_unapproved_token_panics() {
        let f = TestFixture::setup();
        let bad_token = Address::generate(&f.env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().create_escrow(
                &f.mentor,
                &f.learner,
                &500,
                &symbol_short!("S1"),
                &bad_token,
                &0u64,
                &1u32,
            );
        }));
        assert!(result.is_err(), "unapproved token must panic");
    }

    #[test]
    fn test_create_escrow_insufficient_balance_panics() {
        let f = TestFixture::setup();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.create_escrow_at(999_999_999, 0);
        }));
        assert!(result.is_err(), "insufficient balance must panic");
    }

    // -----------------------------------------------------------------------
    // release_funds
    // -----------------------------------------------------------------------

    #[test]
    fn test_release_funds_by_learner() {
        let f = TestFixture::setup_with_fee(500);
        let token = f.token();
        let id = f.create_escrow_at(1_000, 0);
        let mentor_before = token.balance(&f.mentor);
        let treasury_before = token.balance(&f.treasury);
        f.client().release_funds(&f.learner, &id);
        assert_eq!(token.balance(&f.mentor), mentor_before + 950);
        assert_eq!(token.balance(&f.treasury), treasury_before + 50);
        assert_eq!(token.balance(&f.contract_id), 0);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Released);
        assert_eq!(e.platform_fee, 50);
        assert_eq!(e.net_amount, 950);
    }

    #[test]
    fn test_release_funds_by_admin() {
        let f = TestFixture::setup_with_fee(0);
        let id = f.create_escrow_at(1_000, 0);
        f.client().release_funds(&f.admin, &id);
        assert_eq!(f.client().get_escrow(&id).status, EscrowStatus::Released);
    }

    #[test]
    fn test_release_funds_unauthorized_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        let rando = Address::generate(&f.env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().release_funds(&rando, &id);
        }));
        assert!(result.is_err(), "unauthorized caller must panic");
    }

    #[test]
    fn test_release_funds_non_active_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().release_funds(&f.learner, &id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().release_funds(&f.learner, &id);
        }));
        assert!(result.is_err(), "double-release must panic");
    }

    #[test]
    fn test_release_funds_mentor_cannot_release() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().release_funds(&f.mentor, &id);
        }));
        assert!(result.is_err(), "mentor must not be able to self-release");
    }

    // -----------------------------------------------------------------------
    // dispute
    // -----------------------------------------------------------------------

    #[test]
    fn test_dispute_by_mentor() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client()
            .dispute(&f.mentor, &id, &symbol_short!("NO_SHOW"));
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Disputed);
        assert_eq!(e.dispute_reason, symbol_short!("NO_SHOW"));
    }

    #[test]
    fn test_dispute_by_learner() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client()
            .dispute(&f.learner, &id, &symbol_short!("BAD_SVC"));
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Disputed);
        assert_eq!(e.dispute_reason, symbol_short!("BAD_SVC"));
    }

    #[test]
    fn test_dispute_unauthorized_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        let rando = Address::generate(&f.env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().dispute(&rando, &id, &symbol_short!("FRAUD"));
        }));
        assert!(result.is_err(), "unauthorized dispute must panic");
    }

    #[test]
    fn test_dispute_non_active_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().release_funds(&f.learner, &id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().dispute(&f.mentor, &id, &symbol_short!("LATE"));
        }));
        assert!(result.is_err(), "dispute on released escrow must panic");
    }

    // -----------------------------------------------------------------------
    // resolve_dispute
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_dispute_100_0_all_to_mentor() {
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let id = setup_disputed(&f);
        let mentor_before = token.balance(&f.mentor);
        let learner_before = token.balance(&f.learner);
        f.client().resolve_dispute(&id, &100u32);
        assert_eq!(token.balance(&f.mentor), mentor_before + 1_000);
        assert_eq!(token.balance(&f.learner), learner_before);
        assert_eq!(token.balance(&f.contract_id), 0);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Resolved);
        assert_eq!(e.net_amount, 1_000);
        assert_eq!(e.platform_fee, 0);
        assert!(e.resolved_at > 0);
    }

    #[test]
    fn test_resolve_dispute_50_50_equal_split() {
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let id = setup_disputed(&f);
        let mentor_before = token.balance(&f.mentor);
        let learner_before = token.balance(&f.learner);
        f.client().resolve_dispute(&id, &50u32);
        assert_eq!(token.balance(&f.mentor), mentor_before + 500);
        assert_eq!(token.balance(&f.learner), learner_before + 500);
        assert_eq!(token.balance(&f.contract_id), 0);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Resolved);
        assert_eq!(e.net_amount, 500);
        assert_eq!(e.platform_fee, 500);
    }

    #[test]
    fn test_resolve_dispute_0_100_all_to_learner() {
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let id = setup_disputed(&f);
        let mentor_before = token.balance(&f.mentor);
        let learner_before = token.balance(&f.learner);
        f.client().resolve_dispute(&id, &0u32);
        assert_eq!(token.balance(&f.mentor), mentor_before);
        assert_eq!(token.balance(&f.learner), learner_before + 1_000);
        assert_eq!(token.balance(&f.contract_id), 0);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.net_amount, 0);
        assert_eq!(e.platform_fee, 1_000);
    }

    #[test]
    fn test_resolve_dispute_non_disputed_panics() {
        let f = TestFixture::setup_with_fee(0);
        let id = f.create_escrow_at(500, 0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().resolve_dispute(&id, &50u32);
        }));
        assert!(result.is_err(), "resolve on non-disputed must panic");
    }

    #[test]
    fn test_resolve_dispute_invalid_pct_panics() {
        let f = TestFixture::setup_with_fee(0);
        let id = setup_disputed(&f);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().resolve_dispute(&id, &101u32);
        }));
        assert!(result.is_err(), "mentor_pct > 100 must panic");
    }

    #[test]
    fn test_resolve_dispute_double_resolve_panics() {
        let f = TestFixture::setup_with_fee(0);
        let id = setup_disputed(&f);
        f.client().resolve_dispute(&id, &50u32);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().resolve_dispute(&id, &50u32);
        }));
        assert!(result.is_err(), "double-resolve must panic");
    }

    #[test]
    fn test_resolve_dispute_rounding_no_dust() {
        // 1_000 * 33 / 100 = 330 mentor, 670 learner; total = 1_000
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let id = setup_disputed(&f);
        let mentor_before = token.balance(&f.mentor);
        let learner_before = token.balance(&f.learner);
        f.client().resolve_dispute(&id, &33u32);
        let m = token.balance(&f.mentor) - mentor_before;
        let l = token.balance(&f.learner) - learner_before;
        assert_eq!(m, 330);
        assert_eq!(l, 670);
        assert_eq!(m + l, 1_000);
        assert_eq!(token.balance(&f.contract_id), 0);
    }

    #[test]
    fn test_resolve_dispute_resolved_at_set() {
        let f = TestFixture::setup_with_fee(0);
        let id = setup_disputed(&f);
        let now = f.env.ledger().timestamp();
        f.client().resolve_dispute(&id, &50u32);
        assert_eq!(f.client().get_escrow(&id).resolved_at, now);
    }

    // -----------------------------------------------------------------------
    // refund
    // -----------------------------------------------------------------------

    #[test]
    fn test_refund_admin_only_active() {
        let f = TestFixture::setup();
        let token = f.token();
        let id = f.create_escrow_at(1_000, 0);
        let learner_before = token.balance(&f.learner);
        f.client().refund(&id);
        assert_eq!(token.balance(&f.learner), learner_before + 1_000);
        assert_eq!(token.balance(&f.contract_id), 0);
        assert_eq!(f.client().get_escrow(&id).status, EscrowStatus::Refunded);
    }

    #[test]
    fn test_refund_admin_only_disputed() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().dispute(&f.mentor, &id, &symbol_short!("LATE"));
        f.client().refund(&id);
        assert_eq!(f.client().get_escrow(&id).status, EscrowStatus::Refunded);
    }

    #[test]
    fn test_refund_already_released_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().release_funds(&f.learner, &id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().refund(&id);
        }));
        assert!(result.is_err(), "refund on Released must panic");
    }

    #[test]
    fn test_refund_already_refunded_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().refund(&id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().refund(&id);
        }));
        assert!(result.is_err(), "double-refund must panic");
    }

    #[test]
    fn test_refund_already_resolved_panics() {
        let f = TestFixture::setup_with_fee(0);
        let id = setup_disputed(&f);
        f.client().resolve_dispute(&id, &50u32);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().refund(&id);
        }));
        assert!(result.is_err(), "refund on Resolved must panic");
    }

    // -----------------------------------------------------------------------
    // try_auto_release
    // -----------------------------------------------------------------------

    #[test]
    fn test_auto_release_before_window_panics() {
        let f = TestFixture::setup_full(500, 3_600);
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now + 100);
        // advance to 1 s before window: now + 100 + 3600 - 1
        advance_time(&f.env, 100 + 3_600 - 1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().try_auto_release(&id);
        }));
        assert!(result.is_err(), "early auto-release must panic");
    }

    #[test]
    fn test_auto_release_after_window_succeeds() {
        let f = TestFixture::setup_full(500, 3_600);
        let token = f.token();
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now);
        advance_time(&f.env, 3_600 + 1);
        let mentor_before = token.balance(&f.mentor);
        let treasury_before = token.balance(&f.treasury);
        f.client().try_auto_release(&id);
        assert_eq!(token.balance(&f.mentor), mentor_before + 950);
        assert_eq!(token.balance(&f.treasury), treasury_before + 50);
        assert_eq!(token.balance(&f.contract_id), 0);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Released);
        assert_eq!(e.platform_fee, 50);
        assert_eq!(e.net_amount, 950);
    }

    #[test]
    fn test_auto_release_exactly_at_boundary() {
        let f = TestFixture::setup_full(0, 3_600);
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now.saturating_sub(200));
        advance_time(&f.env, 3_600 - 200);
        f.client().try_auto_release(&id);
        assert_eq!(f.client().get_escrow(&id).status, EscrowStatus::Released);
    }

    #[test]
    fn test_auto_release_already_released_panics() {
        let f = TestFixture::setup_full(0, 3_600);
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now);
        f.client().release_funds(&f.learner, &id);
        advance_time(&f.env, 3_600 + 1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().try_auto_release(&id);
        }));
        assert!(result.is_err(), "auto-release on Released must panic");
    }

    #[test]
    fn test_auto_release_disputed_panics() {
        let f = TestFixture::setup_full(0, 3_600);
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now);
        f.client().dispute(&f.learner, &id, &symbol_short!("LATE"));
        advance_time(&f.env, 3_600 + 1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().try_auto_release(&id);
        }));
        assert!(result.is_err(), "auto-release on Disputed must panic");
    }

    #[test]
    fn test_auto_release_default_72h() {
        let f = TestFixture::setup_full(0, 0); // 0 → 72 h default
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now);
        let delay = 72u64 * 60 * 60;
        advance_time(&f.env, delay - 1);
        let too_early = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().try_auto_release(&id);
        }));
        assert!(too_early.is_err());
        advance_time(&f.env, 1);
        f.client().try_auto_release(&id);
        assert_eq!(f.client().get_escrow(&id).status, EscrowStatus::Released);
    }

    // -----------------------------------------------------------------------
    // Fee deduction
    // -----------------------------------------------------------------------

    #[test]
    fn test_fee_deduction_zero_percent() {
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let id = f.create_escrow_at(1_000, 0);
        let treasury_before = token.balance(&f.treasury);
        f.client().release_funds(&f.learner, &id);
        assert_eq!(token.balance(&f.treasury), treasury_before); // no fee
        let e = f.client().get_escrow(&id);
        assert_eq!(e.platform_fee, 0);
        assert_eq!(e.net_amount, 1_000);
    }

    #[test]
    fn test_fee_deduction_five_percent() {
        let f = TestFixture::setup_with_fee(500);
        let token = f.token();
        let id = f.create_escrow_at(1_000, 0);
        let treasury_before = token.balance(&f.treasury);
        f.client().release_funds(&f.learner, &id);
        assert_eq!(token.balance(&f.treasury), treasury_before + 50);
        assert_eq!(token.balance(&f.mentor), 950);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.platform_fee, 50);
        assert_eq!(e.net_amount, 950);
    }

    #[test]
    fn test_fee_deduction_ten_percent() {
        let f = TestFixture::setup_with_fee(1_000);
        let token = f.token();
        let id = f.create_escrow_at(2_000, 0);
        let treasury_before = token.balance(&f.treasury);
        f.client().release_funds(&f.learner, &id);
        assert_eq!(token.balance(&f.treasury), treasury_before + 200);
        assert_eq!(token.balance(&f.mentor), 1_800);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.platform_fee, 200);
        assert_eq!(e.net_amount, 1_800);
    }

    // -----------------------------------------------------------------------
    // update_fee / update_treasury
    // -----------------------------------------------------------------------

    #[test]
    fn test_update_fee_by_admin() {
        let f = TestFixture::setup_with_fee(500);
        f.client().update_fee(&200u32);
        assert_eq!(f.client().get_fee_bps(), 200);
    }

    #[test]
    fn test_update_fee_over_cap_panics() {
        let f = TestFixture::setup();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().update_fee(&1_001u32);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_update_treasury_redirects_fee() {
        let f = TestFixture::setup_with_fee(500);
        let token = f.token();
        let new_treasury = Address::generate(&f.env);
        f.client().update_treasury(&new_treasury);
        let id = f.create_escrow_at(1_000, 0);
        f.client().release_funds(&f.learner, &id);
        assert_eq!(token.balance(&new_treasury), 50);
        assert_eq!(token.balance(&f.treasury), 0);
    }

    // -----------------------------------------------------------------------
    // set_approved_token (whitelist tests)
    // -----------------------------------------------------------------------

    #[test]
    fn test_set_approved_token_toggle() {
        let f = TestFixture::setup();
        let client = f.client();
        let new_token = Address::generate(&f.env);
        assert!(!client.is_token_approved(&new_token));
        client.set_approved_token(&new_token, &true);
        assert!(client.is_token_approved(&new_token));
        client.set_approved_token(&new_token, &false);
        assert!(!client.is_token_approved(&new_token));
    }

    /// Test: Revoked token cannot be used to create escrow
    #[test]
    fn test_revoked_token_cannot_create_escrow() {
        let f = TestFixture::setup();
        let client = f.client();
        // Revoke the approved token
        client.set_approved_token(&f.token_address, &false);
        assert!(!client.is_token_approved(&f.token_address));
        // Try to create escrow with revoked token — must panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.create_escrow_at(500, 0);
        }));
        assert!(result.is_err(), "revoked token must be rejected");
    }

    /// Test: Token approval events are emitted
    #[test]
    fn test_token_approval_events_emitted() {
        let f = TestFixture::setup();
        let new_token = Address::generate(&f.env);
        f.client().set_approved_token(&new_token, &true);
        let events = f.env.events().all();
        let last_event = events.last().unwrap();
        assert_eq!(last_event.0, f.contract_id.clone());
        // The event topic should contain "Token" and "Approved"
        assert_eq!(
            last_event.1,
            (Symbol::new(&f.env, "Token"), Symbol::new(&f.env, "Approved")).into_val(&f.env)
        );
    }

    /// Test: Token rejection events are emitted
    #[test]
    fn test_token_rejection_events_emitted() {
        let f = TestFixture::setup();
        let new_token = Address::generate(&f.env);
        f.client().set_approved_token(&new_token, &true);
        f.client().set_approved_token(&new_token, &false);
        let events = f.env.events().all();
        let last_event = events.last().unwrap();
        assert_eq!(last_event.0, f.contract_id.clone());
        assert_eq!(
            last_event.1,
            (Symbol::new(&f.env, "Token"), Symbol::new(&f.env, "Rejected")).into_val(&f.env)
        );
    }

    /// Test: Unknown/random token address is not approved by default
    #[test]
    fn test_unknown_token_not_approved() {
        let f = TestFixture::setup();
        for _ in 0..5 {
            let random_token = Address::generate(&f.env);
            assert!(
                !f.client().is_token_approved(&random_token),
                "unknown tokens must default to not-approved"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Malicious token bypass tests
    // -----------------------------------------------------------------------

    /// Test: Cannot bypass whitelist by using an unapproved token in path payment
    #[test]
    fn test_path_payment_unapproved_send_asset_panics() {
        let f = TestFixture::setup();
        let malicious_token = Address::generate(&f.env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().create_escrow_with_path_payment(
                &f.learner,
                &f.mentor,
                &malicious_token,     // unapproved send asset
                &1000,
                &f.token_address,     // approved dest asset
                &1000,
                &Vec::new(&f.env),
                &1u32,
            );
        }));
        assert!(result.is_err(), "unapproved send_asset must be rejected in path payment");
    }

    /// Test: Cannot bypass whitelist by using unapproved dest_asset in path payment
    #[test]
    fn test_path_payment_unapproved_dest_asset_panics() {
        let f = TestFixture::setup();
        let malicious_token = Address::generate(&f.env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().create_escrow_with_path_payment(
                &f.learner,
                &f.mentor,
                &f.token_address,     // approved send asset
                &1000,
                &malicious_token,     // unapproved dest asset
                &1000,
                &Vec::new(&f.env),
                &1u32,
            );
        }));
        assert!(result.is_err(), "unapproved dest_asset must be rejected in path payment");
    }

    /// Test: Cannot bypass whitelist with milestone escrow using unapproved token
    #[test]
    fn test_milestone_escrow_unapproved_token_panics() {
        let f = TestFixture::setup();
        let malicious_token = Address::generate(&f.env);
        let mut milestones = Vec::new(&f.env);
        milestones.push_back(MilestoneSpec {
            description_hash: BytesN::from_array(&f.env, &[0u8; 32]),
            amount: 1000,
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().create_milestone_escrow(
                &f.mentor,
                &f.learner,
                &milestones,
                &malicious_token,
            );
        }));
        assert!(result.is_err(), "unapproved token must be rejected in milestone escrow");
    }

    /// Test: Re-adding removed token restores access
    #[test]
    fn test_re_add_token_after_removal() {
        let f = TestFixture::setup();
        let client = f.client();
        // Remove
        client.set_approved_token(&f.token_address, &false);
        assert!(!client.is_token_approved(&f.token_address));
        // Re-add
        client.set_approved_token(&f.token_address, &true);
        assert!(client.is_token_approved(&f.token_address));
        // Should be able to create escrow again
        let id = f.create_escrow_at(500, 0);
        assert_eq!(id, 1);
    }

    // -----------------------------------------------------------------------
    // Balance consistency tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_balances_create_then_refund() {
        let f = TestFixture::setup_with_fee(500);
        let token = f.token();
        let learner_start = token.balance(&f.learner);

        let id = f.create_escrow_at(1_000, 0);
        assert_eq!(token.balance(&f.learner), learner_start - 1_000);
        assert_eq!(token.balance(&f.contract_id), 1_000);

        f.client().refund(&id);
        assert_eq!(token.balance(&f.learner), learner_start); // fully restored
        assert_eq!(token.balance(&f.contract_id), 0);
        assert_eq!(token.balance(&f.treasury), 0); // no fee on refund
    }

    #[test]
    fn test_balances_create_dispute_resolve() {
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let learner_start = token.balance(&f.learner);
        let mentor_start = token.balance(&f.mentor);

        let id = f.create_escrow_at(1_000, 0);
        assert_eq!(token.balance(&f.contract_id), 1_000);

        f.open_dispute(id);
        assert_eq!(token.balance(&f.contract_id), 1_000); // still held

        f.client().resolve_dispute(&id, &75u32); // 750 mentor, 250 learner
        assert_eq!(token.balance(&f.mentor), mentor_start + 750);
        assert_eq!(token.balance(&f.learner), learner_start - 1_000 + 250);
        assert_eq!(token.balance(&f.contract_id), 0);
    }
}

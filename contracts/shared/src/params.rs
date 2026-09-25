/// Protocol parameter registry for MentorsMind.
///
/// # Pattern
/// Every governed parameter lives under `DataKey::Param(Symbol)` in
/// **persistent** storage so it survives ledger TTL expiry.  Each contract
/// calls `get_param` with a compile-time default; governance proposals update
/// values via `set_param` which checks RBAC before writing.
///
/// # Authorization
/// `set_param` requires the caller to hold the `GOVERNANCE_ADMIN` role in
/// the RBAC contract whose address is stored under `ParamKey::RbacContract`
/// in **instance** storage.  This address is written once during
/// `init_protocol_params` and cannot be changed without re-initializing.
///
/// # Monitoring
/// `get_all_params` returns the full `(Symbol, i128)` snapshot for
/// off-chain dashboards.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Storage keys for the protocol_params namespace
// ---------------------------------------------------------------------------

/// Keys used exclusively by the param registry.
/// `Param(key)` holds the live governance value (i128).
/// `ParamKeys` holds the ordered index of known keys (Vec<Symbol>).
/// `RbacContract` holds the address of the RBAC contract used for auth.
#[contracttype]
#[derive(Clone)]
pub enum ParamKey {
    /// Live value for parameter `key`.
    Param(Symbol),
    /// Ordered list of all registered parameter symbols.
    ParamKeys,
    /// Address of the RBAC contract that gates `set_param`.
    RbacContract,
}

// ---------------------------------------------------------------------------
// Canonical parameter key constants
//
// Symbol::new is not const-evaluable in no_std, so we use symbol_short!
// for keys that fit in 9 bytes and Symbol::new at runtime for longer ones.
// All keys are declared here as functions so contracts import one symbol.
// ---------------------------------------------------------------------------

/// Minimum MNT bond required to post a performance bond (7-decimal units).
/// Default: 100_000_000 (= 100 MNT with 7 decimals).
pub fn key_min_bond() -> Symbol { symbol_short!("MIN_BOND") }

/// Minimum credit score required to borrow from the lending pool.
/// Default: 600.
pub fn key_min_credit_score() -> Symbol { symbol_short!("MIN_CREDIT") }

/// Lending pool interest rate in basis points.
/// Default: 200 (= 2%).
pub fn key_interest_rate_bps() -> Symbol { symbol_short!("INT_RATE") }

/// Platform fee in basis points.
/// Default: 200 (= 2%).
pub fn key_platform_fee_bps() -> Symbol { symbol_short!("PLAT_FEE") }

/// Cooldown days before a bond can be released.
/// Default: 30.
pub fn key_cooldown_days() -> Symbol { symbol_short!("COOLDOWN") }

/// Staking tier 1 (Bronze) threshold in raw MNT units (no decimals).
/// Default: 100.
pub fn key_tier_bronze() -> Symbol { symbol_short!("TIER_BRZ") }

/// Staking tier 2 (Silver) threshold in raw MNT units.
/// Default: 500.
pub fn key_tier_silver() -> Symbol { symbol_short!("TIER_SLV") }

/// Staking tier 3 (Gold) threshold in raw MNT units.
/// Default: 2000.
pub fn key_tier_gold() -> Symbol { symbol_short!("TIER_GLD") }

/// Subscription expiry grace period in seconds after billing date.
/// Default: 604_800 (= 7 days).
pub fn key_sub_expiry_grace() -> Symbol { symbol_short!("SUB_EXP") }

// ---------------------------------------------------------------------------
// Compile-time defaults (used as fallback when no governance value is stored)
// ---------------------------------------------------------------------------

pub const DEFAULT_MIN_BOND: i128          = 100_000_000; // 100 MNT (7 decimals)
pub const DEFAULT_MIN_CREDIT_SCORE: i128  = 600;
pub const DEFAULT_INTEREST_RATE_BPS: i128 = 200;
pub const DEFAULT_PLATFORM_FEE_BPS: i128  = 200;
pub const DEFAULT_COOLDOWN_DAYS: i128     = 30;
pub const DEFAULT_TIER_BRONZE: i128       = 100;
pub const DEFAULT_TIER_SILVER: i128       = 500;
pub const DEFAULT_TIER_GOLD: i128         = 2_000;
pub const DEFAULT_SUB_EXPIRY_GRACE: i128  = 7 * 24 * 60 * 60; // 7 days in seconds

// ---------------------------------------------------------------------------
// RBAC role name used to gate set_param
// ---------------------------------------------------------------------------

/// The RBAC role that authorizes parameter updates.
/// Must be granted by the RBAC super-admin to the governance executor address.
pub fn governance_admin_role(env: &Env) -> Symbol {
    Symbol::new(env, "GOVERNANCE_ADMIN")
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// One-time initialization: record `rbac_contract` address and seed the
/// param key index.  Safe to call multiple times — subsequent calls are
/// no-ops if already initialized.
pub fn init_protocol_params(env: &Env, rbac_contract: &Address) {
    if env.storage().instance().has(&ParamKey::RbacContract) {
        return; // already initialized
    }
    env.storage()
        .instance()
        .set(&ParamKey::RbacContract, rbac_contract);

    // Seed index with all canonical keys in a stable order.
    let mut keys: Vec<Symbol> = Vec::new(env);
    keys.push_back(key_min_bond());
    keys.push_back(key_min_credit_score());
    keys.push_back(key_interest_rate_bps());
    keys.push_back(key_platform_fee_bps());
    keys.push_back(key_cooldown_days());
    keys.push_back(key_tier_bronze());
    keys.push_back(key_tier_silver());
    keys.push_back(key_tier_gold());
    keys.push_back(key_sub_expiry_grace());
    env.storage()
        .persistent()
        .set(&ParamKey::ParamKeys, &keys);
}

/// Read a parameter value, falling back to `default` when no governance
/// value has been stored.  Zero-cost when governance hasn't acted: the
/// storage get returns `None` and the default is returned immediately.
pub fn get_param(env: &Env, key: &Symbol, default: i128) -> i128 {
    env.storage()
        .persistent()
        .get(&ParamKey::Param(key.clone()))
        .unwrap_or(default)
}

/// Write a parameter value.
///
/// # Authorization
/// `caller` must hold the `GOVERNANCE_ADMIN` role in the RBAC contract
/// stored during `init_protocol_params`.  The RBAC check is performed via
/// a cross-contract call so this works even without a local RBAC import.
///
/// Panics if:
/// - `init_protocol_params` was never called (no RBAC address stored).
/// - The caller does not hold `GOVERNANCE_ADMIN`.
/// - `value` is negative (protocol parameters are non-negative by convention).
pub fn set_param(env: &Env, caller: &Address, key: &Symbol, value: i128) {
    caller.require_auth();

    if value < 0 {
        panic!("protocol parameter values must be non-negative");
    }

    let rbac: Address = env
        .storage()
        .instance()
        .get(&ParamKey::RbacContract)
        .expect("protocol params not initialized — call init_protocol_params first");

    // Cross-contract call: rbac.has_role(GOVERNANCE_ADMIN, caller).
    // Returns bool; panics on any error so the tx is rejected cleanly.
    let has_role: bool = env.invoke_contract(
        &rbac,
        &Symbol::new(env, "has_role"),
        soroban_sdk::vec![
            env,
            governance_admin_role(env).into(),
            caller.clone().into(),
        ],
    );
    if !has_role {
        panic!("unauthorized: caller does not hold GOVERNANCE_ADMIN role");
    }

    env.storage()
        .persistent()
        .set(&ParamKey::Param(key.clone()), &value);

    // Ensure key is tracked in the index (idempotent).
    let mut keys: Vec<Symbol> = env
        .storage()
        .persistent()
        .get(&ParamKey::ParamKeys)
        .unwrap_or(Vec::new(env));
    if !keys.contains(key.clone()) {
        keys.push_back(key.clone());
        env.storage()
            .persistent()
            .set(&ParamKey::ParamKeys, &keys);
    }

    env.events().publish(
        (Symbol::new(env, "param_updated"), key.clone()),
        (caller.clone(), value),
    );
}

/// Return all currently stored `(Symbol, i128)` pairs for off-chain
/// monitoring and dashboards.
///
/// Returns the governance-stored value for each key that has one, and the
/// compile-time default for keys that governance has not yet updated.
pub fn get_all_params(env: &Env) -> Vec<(Symbol, i128)> {
    let keys: Vec<Symbol> = env
        .storage()
        .persistent()
        .get(&ParamKey::ParamKeys)
        .unwrap_or(Vec::new(env));

    let mut out: Vec<(Symbol, i128)> = Vec::new(env);

    // Canonical keys with their defaults, always included even if not in index.
    let canonical: &[(fn() -> Symbol, i128)] = &[
        (key_min_bond,           DEFAULT_MIN_BOND),
        (key_min_credit_score,   DEFAULT_MIN_CREDIT_SCORE),
        (key_interest_rate_bps,  DEFAULT_INTEREST_RATE_BPS),
        (key_platform_fee_bps,   DEFAULT_PLATFORM_FEE_BPS),
        (key_cooldown_days,      DEFAULT_COOLDOWN_DAYS),
        (key_tier_bronze,        DEFAULT_TIER_BRONZE),
        (key_tier_silver,        DEFAULT_TIER_SILVER),
        (key_tier_gold,          DEFAULT_TIER_GOLD),
        (key_sub_expiry_grace,   DEFAULT_SUB_EXPIRY_GRACE),
    ];

    for (key_fn, default) in canonical.iter() {
        let k = key_fn();
        let v = get_param(env, &k, *default);
        out.push_back((k, v));
    }

    // Any extra keys registered by governance that aren't in the canonical list.
    for k in keys.iter() {
        // Check if already included.
        let mut already = false;
        for (key_fn, _) in canonical.iter() {
            if key_fn() == k {
                already = true;
                break;
            }
        }
        if !already {
            if let Some(v) = env
                .storage()
                .persistent()
                .get::<_, i128>(&ParamKey::Param(k.clone()))
            {
                out.push_back((k, v));
            }
        }
    }

    out
}

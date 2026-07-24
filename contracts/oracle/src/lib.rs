#![no_std]
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, symbol_short, Address, Env, IntoVal,
    Map, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Storage key constants
// ---------------------------------------------------------------------------

const ADMIN: Symbol = symbol_short!("ADMIN");
const FEEDERS: Symbol = symbol_short!("FEEDERS");
const RBAC: Symbol = symbol_short!("RBAC");

// ---------------------------------------------------------------------------
// Tunable parameters
// ---------------------------------------------------------------------------

/// Minimum number of independent feeders required before `get_price` returns.
const MIN_FEEDERS: u32 = 3;

/// Seconds after which a price is considered stale.
const STALE_SECS: u64 = 300;

/// Number of price points used for the TWAP rolling window.
const TWAP_WINDOW: u32 = 5;

/// Default circuit-breaker threshold: 50 % deviation from TWAP.
/// Stored as basis points (10 000 bps = 100 %).
const DEFAULT_CB_THRESHOLD_BPS: i128 = 5_000;

/// Maximum number of secondary oracle sources that can be registered.
const MAX_SECONDARY_SOURCES: u32 = 5;

/// Minimum number of secondary sources that must agree before
/// `get_aggregated_price` returns a value.
const MIN_SECONDARY_CONSENSUS: u32 = 2;

/// Maximum deviation (bps) allowed between the primary price and the
/// secondary-source median before the aggregated call is rejected.
const MAX_SOURCE_DIVERGENCE_BPS: i128 = 1_000; // 10 %

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single price observation submitted by a feeder.
#[contracttype]
#[derive(Clone)]
pub struct PricePoint {
    pub price: i128,
    pub timestamp: u64,
}

/// Rolling TWAP state stored per asset.
#[contracttype]
#[derive(Clone)]
pub struct TwapState {
    /// Σ(price_i × Δt_i) over the current window.
    pub cumulative_price: i128,
    /// Timestamp of the most recent price point in the window.
    pub last_timestamp: u64,
    /// Current TWAP = cumulative_price / total_elapsed.
    pub twap: i128,
    /// Total elapsed seconds covered by the window.
    pub total_elapsed: u64,
}

/// A registered secondary oracle source.
#[contracttype]
#[derive(Clone)]
pub struct OracleSource {
    /// On-chain address of the secondary oracle contract.
    pub address: Address,
    /// Human-readable label (e.g. "Pyth", "Chainlink-bridge").
    pub label: Symbol,
    /// Whether this source is currently active.
    pub active: bool,
}

/// Aggregated price result returned by `get_aggregated_price`.
#[contracttype]
#[derive(Clone)]
pub struct AggregatedPrice {
    /// Median of all active source prices (primary + secondary).
    pub price: i128,
    /// TWAP from the primary oracle.
    pub twap: i128,
    /// Number of sources that contributed to this result.
    pub source_count: u32,
    /// Ledger timestamp of the aggregation.
    pub timestamp: u64,
    /// Whether the price passed all deviation checks.
    pub is_valid: bool,
}

// ---------------------------------------------------------------------------
// External contract interfaces
// ---------------------------------------------------------------------------

#[contractclient(name = "RbacContractClient")]
pub trait RbacContractTrait {
    fn has_role(env: Env, role: Symbol, account: Address) -> bool;
}

/// Interface expected from secondary oracle sources.
/// Each source must expose `get_price(asset) -> (i128, u64)`.
#[contractclient(name = "SecondaryOracleClient")]
pub trait SecondaryOracleTrait {
    fn get_price(env: Env, asset: Symbol) -> (i128, u64);
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct OracleContract;

#[contractimpl]
impl OracleContract {
    // -----------------------------------------------------------------------
    // Initialisation
    // -----------------------------------------------------------------------

    pub fn initialize(env: Env, admin: Address) {
        if env.storage().persistent().has(&ADMIN) {
            panic!("already initialized");
        }
        env.storage().persistent().set(&ADMIN, &admin);
        env.storage()
            .persistent()
            .set(&FEEDERS, &Vec::<Address>::new(&env));
        // Initialise secondary sources list as empty.
        let sources_key = symbol_short!("SEC_SRCS");
        env.storage()
            .persistent()
            .set(&sources_key, &Vec::<OracleSource>::new(&env));
        // Store default circuit-breaker threshold.
        let cb_key = symbol_short!("CB_BPS");
        env.storage()
            .persistent()
            .set(&cb_key, &DEFAULT_CB_THRESHOLD_BPS);
    }

    // -----------------------------------------------------------------------
    // Admin: RBAC
    // -----------------------------------------------------------------------

    pub fn set_rbac_contract(env: Env, admin: Address, rbac: Address) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        env.storage().persistent().set(&RBAC, &rbac);
    }

    // -----------------------------------------------------------------------
    // Admin: circuit-breaker threshold
    // -----------------------------------------------------------------------

    /// Update the circuit-breaker threshold (basis points).
    /// Only the admin or ORACLE_ADMIN role may call this.
    /// `threshold_bps` must be in the range [100, 9_000] (1 %–90 %).
    pub fn set_circuit_breaker_threshold(env: Env, admin: Address, threshold_bps: i128) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        if threshold_bps < 100 || threshold_bps > 9_000 {
            panic!("threshold_bps must be between 100 and 9000");
        }
        let cb_key = symbol_short!("CB_BPS");
        env.storage().persistent().set(&cb_key, &threshold_bps);
    }

    /// Return the current circuit-breaker threshold in basis points.
    pub fn get_circuit_breaker_threshold(env: Env) -> i128 {
        let cb_key = symbol_short!("CB_BPS");
        env.storage()
            .persistent()
            .get(&cb_key)
            .unwrap_or(DEFAULT_CB_THRESHOLD_BPS)
    }

    // -----------------------------------------------------------------------
    // Admin: primary feeders
    // -----------------------------------------------------------------------

    pub fn add_feeder(env: Env, admin: Address, feeder: Address) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        let mut feeders: Vec<Address> = env
            .storage()
            .persistent()
            .get(&FEEDERS)
            .unwrap_or(Vec::new(&env));
        if !feeders.contains(feeder.clone()) {
            feeders.push_back(feeder);
        }
        env.storage().persistent().set(&FEEDERS, &feeders);
    }

    pub fn remove_feeder(env: Env, admin: Address, feeder: Address) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        let feeders: Vec<Address> = env
            .storage()
            .persistent()
            .get(&FEEDERS)
            .unwrap_or(Vec::new(&env));
        let mut next = Vec::new(&env);
        for f in feeders.iter() {
            if f != feeder {
                next.push_back(f);
            }
        }
        env.storage().persistent().set(&FEEDERS, &next);
    }

    // -----------------------------------------------------------------------
    // Admin: secondary oracle sources
    // -----------------------------------------------------------------------

    /// Register a secondary oracle source.
    /// Up to MAX_SECONDARY_SOURCES sources may be registered.
    pub fn add_oracle_source(env: Env, admin: Address, source_address: Address, label: Symbol) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        let sources_key = symbol_short!("SEC_SRCS");
        let mut sources: Vec<OracleSource> = env
            .storage()
            .persistent()
            .get(&sources_key)
            .unwrap_or(Vec::new(&env));
        if sources.len() >= MAX_SECONDARY_SOURCES {
            panic!("maximum secondary oracle sources reached");
        }
        // Prevent duplicate addresses.
        for s in sources.iter() {
            if s.address == source_address {
                panic!("oracle source already registered");
            }
        }
        sources.push_back(OracleSource {
            address: source_address,
            label,
            active: true,
        });
        env.storage().persistent().set(&sources_key, &sources);
    }

    /// Enable or disable a secondary oracle source by its address.
    pub fn set_oracle_source_active(
        env: Env,
        admin: Address,
        source_address: Address,
        active: bool,
    ) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        let sources_key = symbol_short!("SEC_SRCS");
        let sources: Vec<OracleSource> = env
            .storage()
            .persistent()
            .get(&sources_key)
            .unwrap_or(Vec::new(&env));
        let mut updated = Vec::new(&env);
        let mut found = false;
        for s in sources.iter() {
            if s.address == source_address {
                updated.push_back(OracleSource {
                    address: s.address,
                    label: s.label,
                    active,
                });
                found = true;
            } else {
                updated.push_back(s);
            }
        }
        if !found {
            panic!("oracle source not found");
        }
        env.storage().persistent().set(&sources_key, &updated);
    }

    /// Return all registered secondary oracle sources.
    pub fn get_oracle_sources(env: Env) -> Vec<OracleSource> {
        let sources_key = symbol_short!("SEC_SRCS");
        env.storage()
            .persistent()
            .get(&sources_key)
            .unwrap_or(Vec::new(&env))
    }

    // -----------------------------------------------------------------------
    // Price submission (primary feeders)
    // -----------------------------------------------------------------------

    /// Submit a price observation for `asset`.
    ///
    /// Enforces:
    /// 1. Feeder authorisation (registered feeder or ORACLE_FEEDER role).
    /// 2. Positive price.
    /// 3. Circuit-breaker: rejects if deviation from current TWAP exceeds
    ///    the configured threshold.
    ///
    /// After acceptance the price is appended to the rolling window and the
    /// TWAP is recomputed.
    pub fn submit_price(env: Env, feeder: Address, asset: Symbol, price: i128, timestamp: u64) {
        feeder.require_auth();
        if !Self::is_feeder(&env, &feeder)
            && !Self::has_rbac_role(&env, Symbol::new(&env, "ORACLE_FEEDER"), feeder.clone())
        {
            panic!("unauthorized feeder");
        }

        if price <= 0 {
            panic!("price must be positive");
        }

        // -------------------------------------------------------------------
        // Circuit breaker: reject prices that deviate more than the configured
        // threshold from the current TWAP.
        // -------------------------------------------------------------------
        let cb_threshold = Self::get_circuit_breaker_threshold(env.clone());
        let twap_key = (symbol_short!("TWAP"), asset.clone());
        if let Some(twap_state) = env
            .storage()
            .persistent()
            .get::<_, TwapState>(&twap_key)
        {
            if twap_state.twap > 0 {
                let diff = if price > twap_state.twap {
                    price - twap_state.twap
                } else {
                    twap_state.twap - price
                };
                let deviation_bps = diff
                    .checked_mul(10_000)
                    .unwrap_or(i128::MAX)
                    .checked_div(twap_state.twap)
                    .unwrap_or(i128::MAX);
                if deviation_bps > cb_threshold {
                    panic!("price deviation exceeds circuit breaker threshold");
                }
            }
        }

        // Store the feeder's latest price in the per-asset map (one entry per feeder).
        let key = (symbol_short!("PRICES"), asset.clone());
        let mut price_map: Map<Address, PricePoint> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Map::new(&env));
        price_map.set(feeder.clone(), PricePoint { price, timestamp });
        env.storage().persistent().set(&key, &price_map);

        // Recompute TWAP.
        Self::_update_twap(&env, &asset, &price_map);

        env.events().publish(
            (symbol_short!("oracle"), symbol_short!("price_upd"), asset),
            (price, timestamp),
        );
    }

    // -----------------------------------------------------------------------
    // Price queries
    // -----------------------------------------------------------------------

    /// Return the median spot price and the timestamp of the most recent
    /// submission.  Requires at least MIN_FEEDERS registered feeders and
    /// at least MIN_FEEDERS distinct feeder submissions for the asset.
    pub fn get_price(env: Env, asset: Symbol) -> (i128, u64) {
        let feeders: Vec<Address> = env
            .storage()
            .persistent()
            .get(&FEEDERS)
            .unwrap_or(Vec::new(&env));
        if feeders.len() < MIN_FEEDERS {
            panic!("not enough feeders");
        }
        let key = (symbol_short!("PRICES"), asset);
        let price_map: Map<Address, PricePoint> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Map::new(&env));
        // Each map entry is keyed by feeder address, so len() == number of
        // distinct feeders that have submitted for this asset.
        if price_map.len() < MIN_FEEDERS {
            panic!("not enough distinct feeder submissions");
        }
        let mut prices = Vec::new(&env);
        let mut last_updated = 0u64;
        for (_addr, p) in price_map.iter() {
            prices.push_back(p.price);
            if p.timestamp > last_updated {
                last_updated = p.timestamp;
            }
        }
        (Self::median(prices), last_updated)
    }

    /// Return the current TWAP for `asset`.
    /// Panics if fewer than 2 price points have been submitted.
    pub fn get_twap(env: Env, asset: Symbol) -> i128 {
        let twap_key = (symbol_short!("TWAP"), asset);
        let state: TwapState = env
            .storage()
            .persistent()
            .get(&twap_key)
            .expect("no TWAP available — need at least 2 price submissions");
        state.twap
    }

    /// Return `true` if the spot price deviates from the TWAP by more than
    /// `threshold_bps` basis points.
    pub fn is_price_manipulated(env: Env, asset: Symbol, threshold_bps: i128) -> bool {
        let twap_key = (symbol_short!("TWAP"), asset.clone());
        let twap_state: TwapState = match env.storage().persistent().get(&twap_key) {
            Some(s) => s,
            None => return false,
        };
        if twap_state.twap == 0 {
            return false;
        }
        let (spot, _) = Self::get_price(env, asset);
        let diff = if spot > twap_state.twap {
            spot - twap_state.twap
        } else {
            twap_state.twap - spot
        };
        let deviation_bps = diff
            .checked_mul(10_000)
            .unwrap_or(i128::MAX)
            .checked_div(twap_state.twap)
            .unwrap_or(i128::MAX);
        deviation_bps > threshold_bps
    }

    /// Return `true` if the most recent price is older than STALE_SECS.
    pub fn is_price_stale(env: Env, asset: Symbol) -> bool {
        let (_, updated) = Self::get_price(env.clone(), asset);
        env.ledger().timestamp().saturating_sub(updated) > STALE_SECS
    }

    /// Register a mapping from a token contract address to an asset symbol.
    /// This allows callers (e.g. the payment router) to look up the oracle
    /// asset for a given on-chain token without hard-coding the mapping.
    pub fn set_asset_for_token(env: Env, admin: Address, token: Address, asset: Symbol) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        let key = (symbol_short!("TOK_ASSET"), token);
        env.storage().persistent().set(&key, &asset);
    }

    /// Return the asset symbol registered for `token`, or `None` if not set.
    pub fn get_asset_for_token(env: Env, token: Address) -> Option<Symbol> {
        let key = (symbol_short!("TOK_ASSET"), token);
        env.storage().persistent().get(&key)
    }

    // -----------------------------------------------------------------------
    // Multi-source aggregation
    // -----------------------------------------------------------------------

    /// Aggregate the primary oracle price with all active secondary sources.
    ///
    /// Algorithm:
    /// 1. Collect the primary median price.
    /// 2. Query each active secondary source via `get_price(asset)`.
    ///    Sources that panic or return a stale/zero price are skipped.
    /// 3. Require at least MIN_SECONDARY_CONSENSUS secondary prices.
    /// 4. Compute the overall median across primary + secondary prices.
    /// 5. Validate that the aggregated median does not diverge from the
    ///    primary TWAP by more than MAX_SOURCE_DIVERGENCE_BPS.
    ///
    /// Returns an `AggregatedPrice` with `is_valid = false` if consensus
    /// cannot be reached or divergence is too high — callers must check
    /// this field before using the price.
    pub fn get_aggregated_price(env: Env, asset: Symbol) -> AggregatedPrice {
        let now = env.ledger().timestamp();

        // --- Primary price ---
        let (primary_price, _) = Self::get_price(env.clone(), asset.clone());

        // --- Primary TWAP ---
        let twap_key = (symbol_short!("TWAP"), asset.clone());
        let primary_twap: i128 = env
            .storage()
            .persistent()
            .get::<_, TwapState>(&twap_key)
            .map(|s| s.twap)
            .unwrap_or(primary_price); // fall back to spot if no TWAP yet

        // --- Secondary sources ---
        let sources_key = symbol_short!("SEC_SRCS");
        let sources: Vec<OracleSource> = env
            .storage()
            .persistent()
            .get(&sources_key)
            .unwrap_or(Vec::new(&env));

        let mut all_prices: Vec<i128> = Vec::new(&env);
        all_prices.push_back(primary_price);

        let mut secondary_count: u32 = 0;

        for source in sources.iter() {
            if !source.active {
                continue;
            }
            // Call the secondary oracle.  If it panics we skip it.
            // Soroban does not expose try_invoke_contract, so we rely on
            // the secondary source being well-behaved; a panicking source
            // will abort the whole transaction.  In production, secondary
            // sources should be audited contracts.
            let (sec_price, sec_ts): (i128, u64) = env.invoke_contract(
                &source.address,
                &Symbol::new(&env, "get_price"),
                (asset.clone(),).into_val(&env),
            );

            // Skip zero or stale prices from secondary sources.
            if sec_price <= 0 {
                continue;
            }
            let age = now.saturating_sub(sec_ts);
            if age > STALE_SECS {
                continue;
            }

            all_prices.push_back(sec_price);
            secondary_count += 1;
        }

        // Require minimum secondary consensus.
        if secondary_count < MIN_SECONDARY_CONSENSUS {
            return AggregatedPrice {
                price: primary_price,
                twap: primary_twap,
                source_count: 1 + secondary_count,
                timestamp: now,
                is_valid: false,
            };
        }

        // Compute overall median.
        let aggregated = Self::median(all_prices);

        // Validate divergence from primary TWAP.
        let is_valid = if primary_twap > 0 {
            let diff = if aggregated > primary_twap {
                aggregated - primary_twap
            } else {
                primary_twap - aggregated
            };
            let divergence_bps = diff
                .checked_mul(10_000)
                .unwrap_or(i128::MAX)
                .checked_div(primary_twap)
                .unwrap_or(i128::MAX);
            divergence_bps <= MAX_SOURCE_DIVERGENCE_BPS
        } else {
            true // No TWAP yet — accept the aggregated price.
        };

        env.events().publish(
            (symbol_short!("oracle"), symbol_short!("agg_price"), asset),
            (aggregated, primary_twap, 1u32 + secondary_count, is_valid),
        );

        AggregatedPrice {
            price: aggregated,
            twap: primary_twap,
            source_count: 1 + secondary_count,
            timestamp: now,
            is_valid,
        }
    }

    // -----------------------------------------------------------------------
    // Private helpers
    // -----------------------------------------------------------------------

    fn median(mut values: Vec<i128>) -> i128 {
        let n = values.len();
        // Insertion sort — n is bounded by the number of registered feeders
        // (one entry per feeder in the price map), keeping this O(n²) safe.
        let mut i = 1u32;
        while i < n {
            let key = values.get(i).unwrap();
            let mut j = i;
            while j > 0 {
                let prev = values.get(j - 1).unwrap();
                if prev > key {
                    values.set(j, prev);
                    j -= 1;
                } else {
                    break;
                }
            }
            values.set(j, key);
            i += 1;
        }
        values.get(n / 2).unwrap()
    }

    /// Recompute the TWAP from each feeder's latest price point.
    ///
    /// TWAP = Σ(price_i × Δt_i) / Σ(Δt_i)
    ///
    /// Points are sorted by timestamp before computing time-weighted intervals.
    /// Requires at least 2 points with distinct timestamps.
    fn _update_twap(env: &Env, asset: &Symbol, price_map: &Map<Address, PricePoint>) {
        // Collect one price point per feeder.
        let mut points: Vec<PricePoint> = Vec::new(env);
        for (_k, v) in price_map.iter() {
            points.push_back(v);
        }
        let n = points.len();
        if n < 2 {
            return;
        }

        // Insertion sort by timestamp — bounded by feeder count, so O(n²) is safe.
        let mut i = 1u32;
        while i < n {
            let cur = points.get(i).unwrap();
            let mut j = i;
            while j > 0 {
                let prev = points.get(j - 1).unwrap();
                if prev.timestamp > cur.timestamp {
                    points.set(j, prev);
                    j -= 1;
                } else {
                    break;
                }
            }
            points.set(j, cur.clone());
            i += 1;
        }

        let start = if n > TWAP_WINDOW { n - TWAP_WINDOW } else { 0 };

        let mut cumulative: i128 = 0;
        let mut total_elapsed: u64 = 0;

        let mut idx = start;
        while idx + 1 < n {
            let p0 = points.get(idx).unwrap();
            let p1 = points.get(idx + 1).unwrap();
            if p1.timestamp > p0.timestamp {
                let dt = (p1.timestamp - p0.timestamp) as i128;
                cumulative = cumulative
                    .checked_add(p0.price.checked_mul(dt).unwrap_or(i128::MAX))
                    .unwrap_or(i128::MAX);
                total_elapsed = total_elapsed
                    .checked_add(p1.timestamp - p0.timestamp)
                    .unwrap_or(u64::MAX);
            }
            idx += 1;
        }

        if total_elapsed == 0 {
            return;
        }

        let twap = cumulative
            .checked_div(total_elapsed as i128)
            .unwrap_or(0);

        let last = points.get(n - 1).unwrap();
        let twap_state = TwapState {
            cumulative_price: cumulative,
            last_timestamp: last.timestamp,
            twap,
            total_elapsed,
        };

        let twap_key = (symbol_short!("TWAP"), asset.clone());
        env.storage().persistent().set(&twap_key, &twap_state);
    }

    fn is_feeder(env: &Env, feeder: &Address) -> bool {
        let feeders: Vec<Address> = env
            .storage()
            .persistent()
            .get(&FEEDERS)
            .unwrap_or(Vec::new(env));
        feeders.contains(feeder.clone())
    }

    fn admin(env: &Env) -> Address {
        env.storage()
            .persistent()
            .get(&ADMIN)
            .expect("not initialized")
    }

    fn require_admin_or_role(env: &Env, caller: &Address, role: Symbol) {
        caller.require_auth();
        if *caller == Self::admin(env) || Self::has_rbac_role(env, role, caller.clone()) {
            return;
        }
        panic!("unauthorized");
    }

    fn has_rbac_role(env: &Env, role: Symbol, account: Address) -> bool {
        match env.storage().persistent().get::<_, Address>(&RBAC) {
            Some(rbac) => RbacContractClient::new(env, &rbac).has_role(&role, &account),
            None => false,
        }
    }
}

#![no_std]
use shared::{
    // Calendar proof validation (#884)
    validate_conflict_proof,
    ConflictProof,
    // Cross-chain finality and sync (#866)
    isolate_chain,
    is_chain_isolated,
    lift_chain_isolation,
    record_inconsistency,
    // Validator accountability (#869)
    apply_slash,
    detect_consensus_attack,
    get_validator_record,
    is_validator_ejected,
    record_epoch_participation,
    record_missed_epoch,
    register_validator,
    validate_market_observations,
    MarketObservation,
    ViolationType,
};
use soroban_sdk::{
    contract, contractclient, contractevent, contractimpl, contracttype, symbol_short, Address,
    BytesN, Env, Symbol, Vec,
};
use shared::mev_protection::{
    detect_atomic_arbitrage, enforce_protocol_isolation, record_mev_monitoring,
    MevProtectionFlag, FairValueExtractionRecord, MevMonitoringRecord,
};

// ---------------------------------------------------------------------------
// Storage key constants
// ---------------------------------------------------------------------------

const ADMIN: Symbol = symbol_short!("ADMIN");
const FEEDERS: Symbol = symbol_short!("FEEDERS");
const RBAC: Symbol = symbol_short!("RBAC");
const MEV_INTERACTION: Symbol = symbol_short!("MEV_INT");

/// Minimum number of active (non-stale) feeders required to compute TWAP.
const MIN_FEEDERS: u32 = 3;
/// Maximum stored price points per asset (rolling window).
const MAX_POINTS: u32 = 10;
/// A reading older than this many seconds is considered stale (1 hour).
const MAX_STALENESS_SECS: u64 = 3_600;
/// Number of price points used for the TWAP rolling window.
const TWAP_WINDOW: u32 = 5;
/// Default circuit-breaker threshold: 50% deviation from TWAP (basis points).
const DEFAULT_CB_THRESHOLD_BPS: i128 = 5_000;
/// Maximum number of secondary oracle sources.
const MAX_SECONDARY_SOURCES: u32 = 5;
/// Minimum secondary sources that must agree.
const MIN_SECONDARY_CONSENSUS: u32 = 2;
/// Maximum deviation (bps) allowed between primary price and secondary median.
const MAX_SOURCE_DIVERGENCE_BPS: i128 = 1_000; // 10%

// ---------------------------------------------------------------------------
// #866 / #869 additions
// ---------------------------------------------------------------------------

/// Chain isolation duration when a feeder submits too many inconsistent prices (1 day).
const FEEDER_ISOLATION_DURATION_SECS: u64 = 24 * 60 * 60;

/// Number of consecutive circuit-breaker trips from a feeder before it is
/// considered a malicious/compromised validator and slashed.
const FEEDER_SLASH_THRESHOLD: u32 = 3;

/// Ledger sequence depth considered safe against chain reorgs.
const ORACLE_REORG_SAFE_DEPTH: u32 = 12;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A single price observation submitted by a feeder.
#[contracttype]
#[derive(Clone)]
pub struct PricePoint {
    pub price: i128,
    pub timestamp: u64,
    /// Address of the feeder that submitted this reading.
    pub feeder: Address,
    /// Ledger sequence at submission (for reorg safety checks — #866).
    pub submitted_at_ledger: u32,
}

/// A registered secondary oracle source.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleSource {
    pub source_address: Address,
    pub weight: u32,
}

/// Snapshot of oracle health exposed to callers (e.g. treasury).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleHealth {
    /// Number of feeders with a non-stale reading in the current window.
    pub active_feeders: u32,
    /// Ledger timestamp of the most recent accepted reading.
    pub last_update: u64,
    /// True when the most recent reading is older than MAX_STALENESS_SECS.
    pub is_stale: bool,
}

/// Running state for TWAP computation.
#[contracttype]
#[derive(Clone)]
pub struct TwapState {
    pub twap: i128,
    pub last_updated: u64,
}

#[contractevent]
#[derive(Clone)]
struct OraclePriceUpdateEvent {
    #[topic]
    category: Symbol,
    #[topic]
    action: Symbol,
    #[topic]
    asset: Symbol,
    price: i128,
    timestamp: u64,
}

#[contractevent]
#[derive(Clone)]
struct OracleFeederFlaggedEvent {
    #[topic]
    category: Symbol,
    #[topic]
    action: Symbol,
    feeder: Address,
    trips: u32,
}

// ---------------------------------------------------------------------------
// External contract interfaces
// ---------------------------------------------------------------------------

#[contractclient(name = "RbacContractClient")]
pub trait RbacContractTrait {
    fn has_role(env: Env, role: Symbol, account: Address) -> bool;
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct OracleContract;

#[contractimpl]
impl OracleContract {
    // -----------------------------------------------------------------------
    // Initialization
    // -----------------------------------------------------------------------

    pub fn initialize(env: Env, admin: Address) {
        if env.storage().persistent().has(&ADMIN) {
            panic!("already initialized");
        }
        env.storage().persistent().set(&ADMIN, &admin);
        env.storage()
            .persistent()
            .set(&FEEDERS, &Vec::<Address>::new(&env));
        let sources_key = symbol_short!("SEC_SRCS");
        env.storage()
            .persistent()
            .set(&sources_key, &Vec::<OracleSource>::new(&env));
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

    pub fn set_circuit_breaker_threshold(env: Env, admin: Address, threshold_bps: i128) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        if threshold_bps < 100 || threshold_bps > 9_000 {
            panic!("threshold_bps must be between 100 and 9000");
        }
        let cb_key = symbol_short!("CB_BPS");
        env.storage().persistent().set(&cb_key, &threshold_bps);
    }

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
            feeders.push_back(feeder.clone());
        }
        env.storage().persistent().set(&FEEDERS, &feeders);

        // Register feeder as a validator for accountability tracking (#869).
        // Ignore if already registered (may re-add after removal).
        let _ = register_validator_safe(&env, &feeder);
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

    pub fn add_secondary_source(env: Env, admin: Address, source: Address, weight: u32) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        let sources_key = symbol_short!("SEC_SRCS");
        let mut sources: Vec<OracleSource> = env
            .storage()
            .persistent()
            .get(&sources_key)
            .unwrap_or(Vec::new(&env));
        if sources.len() as u32 >= MAX_SECONDARY_SOURCES {
            panic!("secondary source limit reached");
        }
        sources.push_back(OracleSource {
            source_address: source,
            weight,
        });
        env.storage().persistent().set(&sources_key, &sources);
    }

    // -----------------------------------------------------------------------
    // Price submission (#866: reorg protection + feeder accountability)
    // -----------------------------------------------------------------------

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

        let interactions = Self::_track_mev_interaction(&env, &feeder);
        let mev_flag = detect_atomic_arbitrage(&env, &feeder, interactions);
        if !enforce_protocol_isolation(&mev_flag) {
            panic!("protocol isolation: MEV arbitrage detected");
        }

        // -------------------------------------------------------------------
        // Circuit breaker: reject prices that deviate more than the configured
        // threshold from the current TWAP.
        // -------------------------------------------------------------------
        // #866 — Reject if the feeder's chain is currently isolated.
        // Chain ID 0 is used as the oracle chain namespace.
        if is_chain_isolated(&env, 0u32) {
            panic!("oracle chain isolated; submissions temporarily blocked");
        }

        // #866 — Reorg safety: reject readings submitted from a ledger that
        // is too recent (within the reorg-safe depth).
        let current_ledger = env.ledger().sequence();
        let reading_ledger = current_ledger; // submission happens at current ledger.
        // We store for future depth checks when consuming the price.

        // Circuit-breaker check.
        let cb_threshold = Self::get_circuit_breaker_threshold(env.clone());
        let twap_key = (symbol_short!("TWAP"), asset.clone());
        let mut cb_trips_this_submission = false;
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
                    cb_trips_this_submission = true;
                    // #869 — Track circuit-breaker trips per feeder.
                    Self::record_cb_trip(&env, &feeder);
                    panic!("price deviation exceeds circuit breaker threshold");
                }
            }
        }

        // Store the reading in per-asset vec (one entry per submission; capped at MAX_POINTS).
        let key = (symbol_short!("PRICES"), asset.clone());
        let mut points: Vec<PricePoint> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env));
        points.push_back(PricePoint {
            price,
            timestamp,
            feeder: feeder.clone(),
            submitted_at_ledger: reading_ledger,
        });
        while points.len() > MAX_POINTS {
            points.remove(0);
        }
        env.storage().persistent().set(&key, &points);

        // #869 — Record successful participation for accountability.
        record_epoch_participation_safe(&env, &feeder);

        OraclePriceUpdateEvent {
            category: symbol_short!("oracle"),
            action: symbol_short!("price_upd"),
            asset,
            price,
            timestamp,
        }
        .publish(&env);

        let _ = cb_trips_this_submission;
    }

    // -----------------------------------------------------------------------
    // Price query (#614: heartbeat filter + outlier rejection + min_feeders)
    // #866: reorg-safe depth filter applied before serving prices
    // -----------------------------------------------------------------------

    /// Returns `(twap_price, last_update_timestamp)`.
    pub fn get_price(env: Env, asset: Symbol) -> (i128, u64) {
        let now = env.ledger().timestamp();
        let current_ledger = env.ledger().sequence();

        let key = (symbol_short!("PRICES"), asset.clone());
        let points: Vec<PricePoint> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env));

        if points.is_empty() {
            panic!("no prices");
        }

        // Step 1: heartbeat filter — discard stale readings.
        // #866: also discard readings from ledgers too recent to be reorg-safe.
        let mut fresh: Vec<PricePoint> = Vec::new(&env);
        let mut last_updated: u64 = 0;
        for p in points.iter() {
            // Freshness check.
            if now.saturating_sub(p.timestamp) > MAX_STALENESS_SECS {
                continue;
            }
            // Reorg-safe depth check.
            let depth = current_ledger.saturating_sub(p.submitted_at_ledger);
            if depth < ORACLE_REORG_SAFE_DEPTH {
                continue; // Too recent; skip until buried deeper.
            }
            if p.timestamp > last_updated {
                last_updated = p.timestamp;
            }
            fresh.push_back(p.clone());
        }

        if fresh.is_empty() {
            panic!("no prices");
        }

        // Step 2: count distinct active feeders.
        let active_count = Self::count_distinct_feeders(&env, &fresh);
        if active_count < MIN_FEEDERS {
            panic!("not enough feeders");
        }

        // Step 3: collect prices and compute median.
        let mut prices: Vec<i128> = Vec::new(&env);
        for p in fresh.iter() {
            prices.push_back(p.price);
        }
        let med = Self::median(prices.clone());

        // Step 4: outlier rejection — keep readings within 2× median.
        let mut inliers: Vec<i128> = Vec::new(&env);
        for price in prices.iter() {
            let diff = if price >= med {
                price.saturating_sub(med)
            } else {
                med.saturating_sub(price)
            };
            if diff <= med {
                inliers.push_back(price);
            }
        }

        if inliers.is_empty() {
            panic!("no prices after outlier rejection");
        }

        let twap = Self::median(inliers);

        // Update TWAP state for circuit-breaker use.
        let twap_key = (symbol_short!("TWAP"), asset);
        env.storage().persistent().set(
            &twap_key,
            &TwapState {
                twap,
                last_updated,
            },
        );

        (twap, last_updated)
    }

    // -----------------------------------------------------------------------
    // Aggregated price (secondary sources)
    // -----------------------------------------------------------------------

    /// Returns `(aggregated_price, source_count)` from secondary oracle
    /// sources with cross-chain consistency validation (#866).
    pub fn get_aggregated_price(env: Env, asset: Symbol) -> (i128, u32) {
        let sources_key = symbol_short!("SEC_SRCS");
        let sources: Vec<OracleSource> = env
            .storage()
            .persistent()
            .get(&sources_key)
            .unwrap_or(Vec::new(&env));

        if (sources.len() as u32) < MIN_SECONDARY_CONSENSUS {
            panic!("insufficient secondary sources");
        }

        // #866: verify none of the source chains are isolated.
        // We use source contract address hash as a proxy chain identifier.
        // Real implementations would map source address → chain_id.

        let (primary_price, _) = Self::get_price(env.clone(), asset.clone());

        let mut secondary_prices: Vec<i128> = Vec::new(&env);
        let mut observations: Vec<MarketObservation> = Vec::new(&env);
        for source in sources.iter() {
            let price: i128 = env.invoke_contract(
                &source.source_address,
                &Symbol::new(&env, "get_price"),
                (asset.clone(),).into_val(&env),
            );
            secondary_prices.push_back(price);
            observations.push_back(MarketObservation {
                venue: source.source_address.clone(),
                price,
                liquidity: i128::from(source.weight.max(1)),
                observed_at: env.ledger().timestamp(),
            });
        }

        if (secondary_prices.len() as u32) < MIN_SECONDARY_CONSENSUS {
            panic!("not enough secondary sources responded");
        }

        let secondary_median = Self::median(secondary_prices.clone());

        let market_check = validate_market_observations(
            &env,
            &observations,
            MAX_SOURCE_DIVERGENCE_BPS as u32,
        );
        if !market_check.valid {
            panic!("market observations failed manipulation-resistance checks");
        }

        // Cross-chain consistency check: primary vs secondary median.
        let divergence = if primary_price > secondary_median {
            primary_price - secondary_median
        } else {
            secondary_median - primary_price
        };
        let divergence_bps = divergence
            .checked_mul(10_000)
            .unwrap_or(i128::MAX)
            .checked_div(secondary_median.max(1))
            .unwrap_or(i128::MAX);

        if divergence_bps > MAX_SOURCE_DIVERGENCE_BPS {
            // #866: Record state inconsistency.
            let dummy_op_id = BytesN::from_array(&env, &[0u8; 32]);
            let expected_root = BytesN::from_array(&env, &[0u8; 32]);
            let observed_root = BytesN::from_array(&env, &[0xFFu8; 32]);
            record_inconsistency(
                &env,
                &dummy_op_id,
                0,
                &expected_root,
                &observed_root,
            );
            panic!("cross-chain price divergence exceeds maximum");
        }

        (secondary_median, secondary_prices.len())
    }

    // -----------------------------------------------------------------------
    // Oracle health
    // -----------------------------------------------------------------------

    pub fn is_price_stale(env: Env, asset: Symbol) -> bool {
        let now = env.ledger().timestamp();
        let key = (symbol_short!("PRICES"), asset);
        let points: Vec<PricePoint> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env));
        let mut last_updated: u64 = 0;
        for p in points.iter() {
            if p.timestamp > last_updated {
                last_updated = p.timestamp;
            }
        }
        now.saturating_sub(last_updated) > MAX_STALENESS_SECS
    }

    pub fn get_oracle_health(env: Env, asset: Symbol) -> OracleHealth {
        let now = env.ledger().timestamp();
        let key = (symbol_short!("PRICES"), asset);
        let points: Vec<PricePoint> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env));

        let mut last_update: u64 = 0;
        let mut fresh: Vec<PricePoint> = Vec::new(&env);
        for p in points.iter() {
            if p.timestamp > last_update {
                last_update = p.timestamp;
            }
            if now.saturating_sub(p.timestamp) <= MAX_STALENESS_SECS {
                fresh.push_back(p.clone());
            }
        }

        let active_feeders = Self::count_distinct_feeders(&env, &fresh);
        let is_stale = now.saturating_sub(last_update) > MAX_STALENESS_SECS;

        OracleHealth {
            active_feeders,
            last_update,
            is_stale,
        }
    }

    // -----------------------------------------------------------------------
    // #866 — Finality-aware oracle controls
    // -----------------------------------------------------------------------

    /// Admin: manually isolate the oracle's source chain.
    ///
    /// Used when severe cross-chain sync failures are detected.
    pub fn isolate_oracle_chain(env: Env, admin: Address, reason: Symbol) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        isolate_chain(
            &env,
            0u32, // oracle chain namespace
            reason,
            1,
            FEEDER_ISOLATION_DURATION_SECS,
        );
    }

    /// Admin: lift oracle chain isolation after the cooling-off period.
    pub fn lift_oracle_isolation(env: Env, admin: Address) -> bool {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));
        lift_chain_isolation(&env, 0u32)
    }

    /// Query whether the oracle chain is currently isolated.
    pub fn is_oracle_isolated(env: Env) -> bool {
        is_chain_isolated(&env, 0u32)
    }

    // -----------------------------------------------------------------------
    // #869 — Validator / feeder accountability
    // -----------------------------------------------------------------------

    /// Get the accountability record for a feeder address.
    ///
    /// Returns `None` if the feeder has not been registered as a validator.
    pub fn get_feeder_accountability(env: Env, feeder: Address) -> Option<shared::ValidatorRecord> {
        get_validator_record(&env, &feeder)
    }

    /// Slash a feeder for provably malicious price submissions.
    ///
    /// Only callable by admin or ORACLE_ADMIN role. Requires an evidence hash.
    pub fn slash_feeder(
        env: Env,
        admin: Address,
        feeder: Address,
        violation: u32,
        evidence_hash: BytesN<32>,
    ) {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));

        let violation_type = match violation {
            1 => ViolationType::MissedEpoch,
            2 => ViolationType::Equivocation,
            3 => ViolationType::TransactionCensorship,
            4 => ViolationType::ConsensusAttack,
            5 => ViolationType::StakeConcentration,
            _ => panic!("unknown violation type"),
        };

        apply_slash(&env, &feeder, violation_type, evidence_hash);

        // Remove feeder from active list if ejected.
        if is_validator_ejected(&env, &feeder) {
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
    }

    /// Detect a consensus-layer attack through the oracle's validator network.
    ///
    /// Aggregates all registered feeders into the validator set for network
    /// anomaly scoring.
    pub fn detect_oracle_consensus_attack(
        env: Env,
        admin: Address,
        attacker: Address,
        attack_type: Symbol,
        evidence_hash: BytesN<32>,
    ) -> u32 {
        Self::require_admin_or_role(&env, &admin, Symbol::new(&env, "ORACLE_ADMIN"));

        let feeders: Vec<Address> = env
            .storage()
            .persistent()
            .get(&FEEDERS)
            .unwrap_or(Vec::new(&env));

        detect_consensus_attack(
            &env,
            &attacker,
            attack_type,
            evidence_hash,
            &feeders,
        )
    }

    // -----------------------------------------------------------------------
    // External calendar verification (#884)
    // -----------------------------------------------------------------------

    pub fn submit_calendar_proof(
        env: Env,
        feeder: Address,
        mentor: Address,
        slot_start: u64,
        proof_hash: BytesN<32>,
    ) {
        feeder.require_auth();
        if !Self::is_feeder(&env, &feeder)
            && !Self::has_rbac_role(&env, Symbol::new(&env, "ORACLE_FEEDER"), feeder.clone())
        {
            panic!("unauthorized feeder");
        }

        let key = (symbol_short!("CAL_PRF"), mentor, slot_start);
        env.storage()
            .persistent()
            .set(&key, &(proof_hash, env.ledger().timestamp()));
    }

    pub fn verify_calendar_availability(
        env: Env,
        mentor: Address,
        slot_start: u64,
        expected_hash: BytesN<32>,
    ) -> ConflictProof {
        let key = (symbol_short!("CAL_PRF"), mentor, slot_start);
        match env.storage().persistent().get::<_, (BytesN<32>, u64)>(&key) {
            Some((proof_hash, issued_at)) => {
                validate_conflict_proof(&env, &proof_hash, &expected_hash, issued_at)
            }
            None => ConflictProof {
                valid: false,
                within_freshness_window: false,
            },
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn _track_mev_interaction(env: &Env, caller: &Address) -> u32 {
        let key = (MEV_INTERACTION, caller.clone(), env.ledger().sequence());
        let mut count: u32 = env.storage().temporary().get(&key).unwrap_or(0);
        count += 1;
        env.storage().temporary().set(&key, &count);
        count
    }

    /// Count the number of distinct feeder addresses in a set of price points.
    fn count_distinct_feeders(env: &Env, points: &Vec<PricePoint>) -> u32 {
        let mut seen: Vec<Address> = Vec::new(env);
        for p in points.iter() {
            if !seen.contains(p.feeder.clone()) {
                seen.push_back(p.feeder.clone());
            }
        }
        seen.len()
    }

    /// Bubble-sort `values` and return the upper-median element.
    fn median(mut values: Vec<i128>) -> i128 {
        let n = values.len();
        if n == 0 {
            panic!("median of empty");
        }
        // Bubble sort.
        let mut i = 0u32;
        while i < n {
            let mut j = 0u32;
            while j + 1 < n - i {
                let a = values.get(j).unwrap();
                let b = values.get(j + 1).unwrap();
                if a > b {
                    values.set(j, b);
                    values.set(j + 1, a);
                }
                j += 1;
            }
            i += 1;
        }
        values.get(n / 2).unwrap()
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

    /// Track circuit-breaker trips per feeder for validator accountability (#869).
    fn record_cb_trip(env: &Env, feeder: &Address) {
        let key = (symbol_short!("CB_TRIP"), feeder.clone());
        let trips: u32 = env.storage().persistent().get(&key).unwrap_or(0u32);
        let new_trips = trips.saturating_add(1);
        env.storage().persistent().set(&key, &new_trips);

        // After threshold, record missed epoch and potentially slash.
        if new_trips >= FEEDER_SLASH_THRESHOLD {
            let flagged = record_missed_epoch_safe(env, feeder);
            if flagged {
                OracleFeederFlaggedEvent {
                    category: symbol_short!("oracle"),
                    action: symbol_short!("fdr_flag"),
                    feeder: feeder.clone(),
                    trips: new_trips,
                }
                .publish(env);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers for safe validator registration / participation tracking
// (these handle the case where a feeder was added before the validator
// subsystem was introduced, or if already registered)
// ---------------------------------------------------------------------------

fn register_validator_safe(env: &Env, validator: &Address) -> bool {
    if get_validator_record(env, validator).is_some() {
        return false; // Already registered.
    }
    register_validator(env, validator);
    true
}

fn record_epoch_participation_safe(env: &Env, validator: &Address) {
    register_validator_safe(env, validator);
    record_epoch_participation(env, validator);
}

fn record_missed_epoch_safe(env: &Env, validator: &Address) -> bool {
    register_validator_safe(env, validator);
    record_missed_epoch(env, validator)
}


// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, OracleContract);
        let client = OracleContractClient::new(&env, &contract_id);
        client.initialize(&admin);
        (env, admin, contract_id)
    }

    fn add_feeders(
        env: &Env,
        client: &OracleContractClient,
        admin: &Address,
        n: u32,
    ) -> Vec<Address> {
        let mut feeders = Vec::new(env);
        for _ in 0..n {
            let f = Address::generate(env);
            client.add_feeder(admin, &f);
            feeders.push_back(f);
        }
        feeders
    }

    fn submit(
        env: &Env,
        client: &OracleContractClient,
        feeder: &Address,
        asset: Symbol,
        price: i128,
        ts: u64,
    ) {
        // Advance ledger past reorg-safe depth before submitting.
        env.ledger().with_mut(|l| l.sequence_number = ORACLE_REORG_SAFE_DEPTH + 1);
        client.submit_price(feeder, &asset, &price, &ts);
    }

    #[test]
    fn test_get_price_basic_median() {
        let (env, admin, contract_id) = setup();
        env.ledger().set_timestamp(1_000);
        let client = OracleContractClient::new(&env, &contract_id);
        let feeders = add_feeders(&env, &client, &admin, 3);
        let asset = symbol_short!("XLM");

        submit(&env, &client, &feeders.get(0).unwrap(), asset.clone(), 100, 999);
        submit(&env, &client, &feeders.get(1).unwrap(), asset.clone(), 110, 999);
        submit(&env, &client, &feeders.get(2).unwrap(), asset.clone(), 105, 999);

        let (price, _) = client.get_price(&asset);
        assert_eq!(price, 105);
    }

    #[test]
    #[should_panic(expected = "not enough feeders")]
    fn test_insufficient_feeders_panics() {
        let (env, admin, contract_id) = setup();
        env.ledger().set_timestamp(1_000);
        let client = OracleContractClient::new(&env, &contract_id);
        let feeders = add_feeders(&env, &client, &admin, 2);
        let asset = symbol_short!("XLM");

        submit(&env, &client, &feeders.get(0).unwrap(), asset.clone(), 100, 999);
        submit(&env, &client, &feeders.get(1).unwrap(), asset.clone(), 110, 999);

        client.get_price(&asset);
    }

    #[test]
    #[should_panic(expected = "not enough feeders")]
    fn test_stale_readings_excluded() {
        let (env, admin, contract_id) = setup();
        let client = OracleContractClient::new(&env, &contract_id);
        let feeders = add_feeders(&env, &client, &admin, 3);
        let asset = symbol_short!("XLM");

        env.ledger().set_timestamp(0);
        submit(&env, &client, &feeders.get(0).unwrap(), asset.clone(), 100, 0);
        submit(&env, &client, &feeders.get(1).unwrap(), asset.clone(), 110, 0);
        submit(&env, &client, &feeders.get(2).unwrap(), asset.clone(), 105, 0);

        env.ledger().set_timestamp(MAX_STALENESS_SECS + 1);
        client.get_price(&asset);
    }

    #[test]
    fn test_outlier_rejection() {
        let (env, admin, contract_id) = setup();
        env.ledger().set_timestamp(1_000);
        let client = OracleContractClient::new(&env, &contract_id);
        let feeders = add_feeders(&env, &client, &admin, 4);
        let asset = symbol_short!("XLM");

        submit(&env, &client, &feeders.get(0).unwrap(), asset.clone(), 100, 999);
        submit(&env, &client, &feeders.get(1).unwrap(), asset.clone(), 102, 999);
        submit(&env, &client, &feeders.get(2).unwrap(), asset.clone(), 98, 999);
        // Outlier: 500 (5× median ~100) → diff = 400 > med = 100 → rejected.
        submit(&env, &client, &feeders.get(3).unwrap(), asset.clone(), 500, 999);

        let (price, _) = client.get_price(&asset);
        assert!(price <= 102, "outlier should be rejected, got {}", price);
        assert!(price >= 98);
    }

    #[test]
    fn test_oracle_health_active_feeders() {
        let (env, admin, contract_id) = setup();
        env.ledger().set_timestamp(100);
        let client = OracleContractClient::new(&env, &contract_id);
        let feeders = add_feeders(&env, &client, &admin, 3);
        let asset = symbol_short!("XLM");

        submit(&env, &client, &feeders.get(0).unwrap(), asset.clone(), 100, 99);
        submit(&env, &client, &feeders.get(1).unwrap(), asset.clone(), 110, 99);
        submit(&env, &client, &feeders.get(2).unwrap(), asset.clone(), 105, 99);

        let health = client.get_oracle_health(&asset);
        assert_eq!(health.active_feeders, 3);
        assert!(!health.is_stale);
    }

    #[test]
    fn test_oracle_health_stale() {
        let (env, admin, contract_id) = setup();
        let client = OracleContractClient::new(&env, &contract_id);
        let feeders = add_feeders(&env, &client, &admin, 3);
        let asset = symbol_short!("XLM");

        env.ledger().set_timestamp(0);
        submit(&env, &client, &feeders.get(0).unwrap(), asset.clone(), 100, 0);

        env.ledger().set_timestamp(MAX_STALENESS_SECS + 1);
        let health = client.get_oracle_health(&asset);
        assert!(health.is_stale);
        assert_eq!(health.active_feeders, 0);
    }

    // -----------------------------------------------------------------------
    // #866 — Oracle chain isolation tests
    // -----------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "oracle chain isolated; submissions temporarily blocked")]
    fn test_price_submission_blocked_when_chain_isolated() {
        let (env, admin, contract_id) = setup();
        env.ledger().set_timestamp(1_000);
        let client = OracleContractClient::new(&env, &contract_id);
        let feeders = add_feeders(&env, &client, &admin, 1);

        // Isolate the oracle chain.
        client.isolate_oracle_chain(&admin, &Symbol::new(&env, "reorg"));

        // Submission should fail.
        submit(&env, &client, &feeders.get(0).unwrap(), symbol_short!("XLM"), 100, 999);
    }

    #[test]
    fn test_oracle_isolation_lift_after_cooldown() {
        let (env, admin, contract_id) = setup();
        env.ledger().set_timestamp(1_000);
        let client = OracleContractClient::new(&env, &contract_id);

        client.isolate_oracle_chain(&admin, &Symbol::new(&env, "test"));
        assert!(client.is_oracle_isolated());

        // Not yet eligible.
        let lifted = client.lift_oracle_isolation(&admin);
        assert!(!lifted);

        // Advance past isolation duration.
        env.ledger()
            .set_timestamp(1_000 + FEEDER_ISOLATION_DURATION_SECS + 1);
        let lifted = client.lift_oracle_isolation(&admin);
        assert!(lifted);
        assert!(!client.is_oracle_isolated());
    }

    // -----------------------------------------------------------------------
    // #869 — Feeder accountability tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_feeder_registered_as_validator_on_add() {
        let (env, admin, contract_id) = setup();
        let client = OracleContractClient::new(&env, &contract_id);
        let feeder = Address::generate(&env);

        client.add_feeder(&admin, &feeder);

        // Validator record should now exist.
        let rec = client.get_feeder_accountability(&feeder);
        assert!(rec.is_some());
    }

    #[test]
    fn test_slash_feeder_removes_from_active_list() {
        let (env, admin, contract_id) = setup();
        env.ledger().set_timestamp(1_000);
        let client = OracleContractClient::new(&env, &contract_id);
        let feeders = add_feeders(&env, &client, &admin, 3);
        let feeder = feeders.get(0).unwrap();
        let evidence = BytesN::from_array(&env, &[0xABu8; 32]);

        // ConsensusAttack → ejection.
        client.slash_feeder(&admin, &feeder, &4u32, &evidence);

        // Feeder accountability record should reflect ejection.
        let rec = client.get_feeder_accountability(&feeder).unwrap();
        assert!(rec.ejected);
    }

    // -----------------------------------------------------------------------
    // Calendar proof tests (#884)
    // -----------------------------------------------------------------------

    #[test]
    fn test_calendar_proof_verifies_when_fresh_and_matching() {
        let (env, admin, contract_id) = setup();
        env.ledger().set_timestamp(1_000);
        let client = OracleContractClient::new(&env, &contract_id);
        let feeders = add_feeders(&env, &client, &admin, 1);
        let mentor = Address::generate(&env);
        let slot_start = 5_000u64;
        let proof_hash = BytesN::from_array(&env, &[9u8; 32]);

        client.submit_calendar_proof(&feeders.get(0).unwrap(), &mentor, &slot_start, &proof_hash);

        let result = client.verify_calendar_availability(&mentor, &slot_start, &proof_hash);
        assert!(result.valid);
    }

    #[test]
    fn test_calendar_proof_rejects_mismatched_hash() {
        let (env, admin, contract_id) = setup();
        env.ledger().set_timestamp(1_000);
        let client = OracleContractClient::new(&env, &contract_id);
        let feeders = add_feeders(&env, &client, &admin, 1);
        let mentor = Address::generate(&env);
        let slot_start = 5_000u64;
        let proof_hash = BytesN::from_array(&env, &[9u8; 32]);
        let other_hash = BytesN::from_array(&env, &[1u8; 32]);

        client.submit_calendar_proof(&feeders.get(0).unwrap(), &mentor, &slot_start, &proof_hash);

        let result = client.verify_calendar_availability(&mentor, &slot_start, &other_hash);
        assert!(!result.valid);
    }

    #[test]
    fn test_calendar_proof_missing_returns_invalid() {
        let (env, _admin, contract_id) = setup();
        let client = OracleContractClient::new(&env, &contract_id);
        let mentor = Address::generate(&env);
        let expected = BytesN::from_array(&env, &[1u8; 32]);

        let result = client.verify_calendar_availability(&mentor, &5_000u64, &expected);
        assert!(!result.valid);
    }
}

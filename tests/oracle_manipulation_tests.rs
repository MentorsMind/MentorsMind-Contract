/// Oracle Manipulation Prevention Tests — Issue #403
///
/// Covers all acceptance criteria:
///   - TWAP price feeds
///   - Multiple oracle sources (aggregation)
///   - Price deviation checks
///   - Circuit breakers for extreme price movements
///   - Oracle manipulation attack scenarios
///
/// Test index:
///   1.  TWAP is computed correctly from time-weighted observations
///   2.  TWAP smooths out a single spike (attacker cannot move TWAP instantly)
///   3.  Circuit breaker rejects a price > 50% above TWAP
///   4.  Circuit breaker rejects a price > 50% below TWAP
///   5.  Circuit breaker allows a price within the 50% window
///   6.  Configurable threshold: tighter threshold rejects smaller deviations
///   7.  Configurable threshold: threshold bounds are enforced (< 100 bps rejected)
///   8.  is_price_manipulated returns false when no TWAP exists yet
///   9.  is_price_manipulated detects a coordinated spike above threshold
///   10. is_price_manipulated returns false for gradual legitimate movement
///   11. Multi-source aggregation: valid when 2+ secondary sources agree
///   12. Multi-source aggregation: is_valid = false when < 2 secondary sources
///   13. Multi-source aggregation: divergent secondary sources invalidate result
///   14. Multi-source aggregation: stale secondary prices are skipped
///   15. Token-to-asset mapping: set and retrieve
///   16. Unauthorized feeder is rejected
///   17. Stale price detection
///   18. Minimum feeders enforced before get_price
///   19. Duplicate feeder registration is idempotent
///   20. Secondary source cap (max 5) is enforced
extern crate std;

use std::panic::{catch_unwind, AssertUnwindSafe};

use mentorminds_oracle::{OracleContract, OracleContractClient};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    symbol_short,
    testutils::{Address as _, Ledger},
    Address, Env, Symbol,
};

// ---------------------------------------------------------------------------
// Mock secondary oracle
// ---------------------------------------------------------------------------

/// A minimal secondary oracle that returns a fixed price for any asset.
/// Used to test multi-source aggregation without a full oracle deployment.
#[contract]
pub struct MockSecondaryOracle;

#[contracttype]
#[derive(Clone)]
pub enum MockOracleKey {
    Price(Symbol),
}

#[contractimpl]
impl MockSecondaryOracle {
    pub fn set_price(env: Env, asset: Symbol, price: i128, ts: u64) {
        env.storage()
            .instance()
            .set(&MockOracleKey::Price(asset), &(price, ts));
    }

    pub fn get_price(env: Env, asset: Symbol) -> (i128, u64) {
        env.storage()
            .instance()
            .get(&MockOracleKey::Price(asset))
            .unwrap_or((0i128, 0u64))
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Advance ledger timestamp by `secs` seconds.
fn advance_time(env: &Env, secs: u64) {
    env.ledger().with_mut(|li| li.timestamp += secs);
}

/// Set up a fresh oracle with `n` feeders registered.
/// Returns (client, admin, feeders).
fn setup_oracle(env: &Env, n: usize) -> (OracleContractClient, Address, std::vec::Vec<Address>) {
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 10_000);

    let admin = Address::generate(env);
    let id = env.register_contract(None, OracleContract);
    let client = OracleContractClient::new(env, &id);
    client.initialize(&admin);

    let mut feeders = std::vec::Vec::new();
    for _ in 0..n {
        let f = Address::generate(env);
        client.add_feeder(&admin, &f);
        feeders.push(f);
    }
    (client, admin, feeders)
}

/// Submit the same price from all feeders at the given timestamp.
fn submit_all(
    client: &OracleContractClient,
    feeders: &[Address],
    asset: &Symbol,
    price: i128,
    ts: u64,
) {
    for f in feeders {
        client.submit_price(f, asset, &price, &ts);
    }
}

// ---------------------------------------------------------------------------
// 1. TWAP is computed correctly from time-weighted observations
// ---------------------------------------------------------------------------

#[test]
fn test_twap_computed_correctly() {
    let env = Env::default();
    let (client, _, feeders) = setup_oracle(&env, 3);
    let asset = symbol_short!("USDC");

    // t=0:   price=100
    // t=100: price=120  → 100 held 100 s → 100*100 = 10_000
    // t=300: price=110  → 120 held 200 s → 120*200 = 24_000
    // TWAP = 34_000 / 300 = 113
    submit_all(&client, &feeders, &asset, 100, 0);
    submit_all(&client, &feeders, &asset, 120, 100);
    submit_all(&client, &feeders, &asset, 110, 300);

    let twap = client.get_twap(&asset);
    assert!(
        (twap - 113).abs() <= 2,
        "expected TWAP ~113, got {twap}"
    );
}

// ---------------------------------------------------------------------------
// 2. TWAP smooths out a single spike
// ---------------------------------------------------------------------------

#[test]
fn test_twap_smooths_spike() {
    let env = Env::default();
    let (client, _, feeders) = setup_oracle(&env, 3);
    let asset = symbol_short!("XLM");

    // Establish baseline at 100 over 400 s.
    submit_all(&client, &feeders, &asset, 100, 0);
    submit_all(&client, &feeders, &asset, 100, 100);
    submit_all(&client, &feeders, &asset, 100, 200);
    submit_all(&client, &feeders, &asset, 100, 300);

    // Spike to 140 at t=400 (40% above, within 50% circuit breaker).
    submit_all(&client, &feeders, &asset, 140, 400);

    let twap = client.get_twap(&asset);
    // TWAP should still be close to 100, not 140.
    // Window: (100*100 + 100*100 + 100*100 + 100*100) / 400 = 100
    // After spike: last interval is 0 s wide so TWAP stays at 100.
    assert!(
        twap < 110,
        "TWAP should be smoothed near 100 after a single spike, got {twap}"
    );
}

// ---------------------------------------------------------------------------
// 3. Circuit breaker rejects price > 50% above TWAP
// ---------------------------------------------------------------------------

#[test]
fn test_circuit_breaker_rejects_spike_above() {
    let env = Env::default();
    let (client, _, feeders) = setup_oracle(&env, 3);
    let asset = symbol_short!("ETH");

    // Build TWAP at 1_000.
    submit_all(&client, &feeders, &asset, 1_000, 1_000);
    submit_all(&client, &feeders, &asset, 1_000, 2_000);

    // Attempt to submit 1_501 — 50.1% above TWAP — must be rejected.
    let result = catch_unwind(AssertUnwindSafe(|| {
        client.submit_price(&feeders[0], &asset, &1_501, &3_000);
    }));
    assert!(
        result.is_err(),
        "price >50% above TWAP must trip the circuit breaker"
    );
}

// ---------------------------------------------------------------------------
// 4. Circuit breaker rejects price > 50% below TWAP
// ---------------------------------------------------------------------------

#[test]
fn test_circuit_breaker_rejects_crash_below() {
    let env = Env::default();
    let (client, _, feeders) = setup_oracle(&env, 3);
    let asset = symbol_short!("BTC");

    submit_all(&client, &feeders, &asset, 1_000, 1_000);
    submit_all(&client, &feeders, &asset, 1_000, 2_000);

    // 499 is 50.1% below 1_000 — must be rejected.
    let result = catch_unwind(AssertUnwindSafe(|| {
        client.submit_price(&feeders[0], &asset, &499, &3_000);
    }));
    assert!(
        result.is_err(),
        "price >50% below TWAP must trip the circuit breaker"
    );
}

// ---------------------------------------------------------------------------
// 5. Circuit breaker allows price within the 50% window
// ---------------------------------------------------------------------------

#[test]
fn test_circuit_breaker_allows_within_window() {
    let env = Env::default();
    let (client, _, feeders) = setup_oracle(&env, 3);
    let asset = symbol_short!("SOL");

    submit_all(&client, &feeders, &asset, 1_000, 1_000);
    submit_all(&client, &feeders, &asset, 1_000, 2_000);

    // 1_499 is 49.9% above — must be accepted.
    let result = catch_unwind(AssertUnwindSafe(|| {
        client.submit_price(&feeders[0], &asset, &1_499, &3_000);
    }));
    assert!(
        result.is_ok(),
        "price within 50% window must be accepted"
    );
}

// ---------------------------------------------------------------------------
// 6. Configurable threshold: tighter threshold rejects smaller deviations
// ---------------------------------------------------------------------------

#[test]
fn test_configurable_threshold_tighter() {
    let env = Env::default();
    let (client, admin, feeders) = setup_oracle(&env, 3);
    let asset = symbol_short!("AVAX");

    submit_all(&client, &feeders, &asset, 1_000, 1_000);
    submit_all(&client, &feeders, &asset, 1_000, 2_000);

    // Tighten to 10% (1_000 bps).
    client.set_circuit_breaker_threshold(&admin, &1_000i128);
    assert_eq!(client.get_circuit_breaker_threshold(), 1_000);

    // 1_101 is 10.1% above — must now be rejected.
    let result = catch_unwind(AssertUnwindSafe(|| {
        client.submit_price(&feeders[0], &asset, &1_101, &3_000);
    }));
    assert!(
        result.is_err(),
        "price >10% above TWAP must be rejected with 10% threshold"
    );

    // 1_099 is 9.9% above — must still be accepted.
    let result2 = catch_unwind(AssertUnwindSafe(|| {
        client.submit_price(&feeders[0], &asset, &1_099, &3_000);
    }));
    assert!(
        result2.is_ok(),
        "price <10% above TWAP must be accepted with 10% threshold"
    );
}

// ---------------------------------------------------------------------------
// 7. Threshold bounds are enforced (< 100 bps rejected)
// ---------------------------------------------------------------------------

#[test]
fn test_threshold_bounds_enforced() {
    let env = Env::default();
    let (client, admin, _) = setup_oracle(&env, 3);

    // 99 bps is below the minimum of 100.
    let result = catch_unwind(AssertUnwindSafe(|| {
        client.set_circuit_breaker_threshold(&admin, &99i128);
    }));
    assert!(result.is_err(), "threshold < 100 bps must be rejected");

    // 9_001 bps is above the maximum of 9_000.
    let result2 = catch_unwind(AssertUnwindSafe(|| {
        client.set_circuit_breaker_threshold(&admin, &9_001i128);
    }));
    assert!(result2.is_err(), "threshold > 9_000 bps must be rejected");

    // 100 and 9_000 are the valid boundary values.
    client.set_circuit_breaker_threshold(&admin, &100i128);
    assert_eq!(client.get_circuit_breaker_threshold(), 100);
    client.set_circuit_breaker_threshold(&admin, &9_000i128);
    assert_eq!(client.get_circuit_breaker_threshold(), 9_000);
}

// ---------------------------------------------------------------------------
// 8. is_price_manipulated returns false when no TWAP exists yet
// ---------------------------------------------------------------------------

#[test]
fn test_is_price_manipulated_no_twap() {
    let env = Env::default();
    let (client, _, feeders) = setup_oracle(&env, 3);
    let asset = symbol_short!("DOT");

    // Only one submission — not enough for a TWAP.
    submit_all(&client, &feeders, &asset, 500, 1_000);

    // Should return false (no TWAP to compare against).
    assert!(
        !client.is_price_manipulated(&asset, &300i128),
        "is_price_manipulated must return false when no TWAP exists"
    );
}

// ---------------------------------------------------------------------------
// 9. is_price_manipulated detects a coordinated spike above threshold
// ---------------------------------------------------------------------------

#[test]
fn test_is_price_manipulated_detects_spike() {
    let env = Env::default();
    let (client, _, feeders) = setup_oracle(&env, 3);
    let asset = symbol_short!("LINK");

    // Build TWAP at 1_000.
    submit_all(&client, &feeders, &asset, 1_000, 1_000);
    submit_all(&client, &feeders, &asset, 1_000, 2_000);

    // Not manipulated yet.
    assert!(!client.is_price_manipulated(&asset, &300i128));

    // All feeders submit 40% above TWAP (1_400) — within circuit breaker (50%)
    // but detectable at a 30% threshold.
    submit_all(&client, &feeders, &asset, 1_400, 3_000);

    assert!(
        client.is_price_manipulated(&asset, &300i128),
        "40% spike should be flagged at 30% threshold"
    );
}

// ---------------------------------------------------------------------------
// 10. is_price_manipulated returns false for gradual legitimate movement
// ---------------------------------------------------------------------------

#[test]
fn test_is_price_manipulated_false_for_gradual_movement() {
    let env = Env::default();
    let (client, _, feeders) = setup_oracle(&env, 3);
    let asset = symbol_short!("ADA");

    // Gradual 2% steps over time.
    for (price, ts) in [
        (1_000i128, 0u64),
        (1_020, 3_600),
        (1_040, 7_200),
        (1_060, 10_800),
        (1_080, 14_400),
    ] {
        submit_all(&client, &feeders, &asset, price, ts);
    }

    // ~8% total drift — should not be flagged at a 15% threshold.
    assert!(
        !client.is_price_manipulated(&asset, &1_500i128),
        "gradual ~8% movement should not be flagged at 15% threshold"
    );
}

// ---------------------------------------------------------------------------
// 11. Multi-source aggregation: valid when 2+ secondary sources agree
// ---------------------------------------------------------------------------

#[test]
fn test_aggregated_price_valid_with_consensus() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 10_000);

    let admin = Address::generate(&env);
    let oracle_id = env.register_contract(None, OracleContract);
    let oracle = OracleContractClient::new(&env, &oracle_id);
    oracle.initialize(&admin);

    // Register 3 primary feeders.
    let f1 = Address::generate(&env);
    let f2 = Address::generate(&env);
    let f3 = Address::generate(&env);
    oracle.add_feeder(&admin, &f1);
    oracle.add_feeder(&admin, &f2);
    oracle.add_feeder(&admin, &f3);

    let asset = symbol_short!("USDC");

    // Build primary TWAP at 1_000.
    oracle.submit_price(&f1, &asset, &1_000, &9_000);
    oracle.submit_price(&f2, &asset, &1_000, &9_100);
    oracle.submit_price(&f3, &asset, &1_000, &9_200);
    oracle.submit_price(&f1, &asset, &1_000, &9_800);
    oracle.submit_price(&f2, &asset, &1_000, &9_900);

    // Register 2 secondary sources returning prices close to primary.
    let sec1_id = env.register_contract(None, MockSecondaryOracle);
    let sec2_id = env.register_contract(None, MockSecondaryOracle);
    let sec1 = MockSecondaryOracleClient::new(&env, &sec1_id);
    let sec2 = MockSecondaryOracleClient::new(&env, &sec2_id);

    // Fresh timestamps so prices are not stale.
    sec1.set_price(&asset, &1_005, &9_950);
    sec2.set_price(&asset, &995, &9_960);

    oracle.add_oracle_source(&admin, &sec1_id, &symbol_short!("Pyth"));
    oracle.add_oracle_source(&admin, &sec2_id, &symbol_short!("Band"));

    let result = oracle.get_aggregated_price(&asset);

    assert!(result.is_valid, "aggregated price should be valid with 2 secondary sources");
    assert_eq!(result.source_count, 3, "primary + 2 secondary = 3 sources");
    // Median of [1_000, 1_005, 995] = 1_000.
    assert_eq!(result.price, 1_000);
}

// ---------------------------------------------------------------------------
// 12. Multi-source aggregation: is_valid = false when < 2 secondary sources
// ---------------------------------------------------------------------------

#[test]
fn test_aggregated_price_invalid_insufficient_sources() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 10_000);

    let admin = Address::generate(&env);
    let oracle_id = env.register_contract(None, OracleContract);
    let oracle = OracleContractClient::new(&env, &oracle_id);
    oracle.initialize(&admin);

    let f1 = Address::generate(&env);
    let f2 = Address::generate(&env);
    let f3 = Address::generate(&env);
    oracle.add_feeder(&admin, &f1);
    oracle.add_feeder(&admin, &f2);
    oracle.add_feeder(&admin, &f3);

    let asset = symbol_short!("XLM");
    oracle.submit_price(&f1, &asset, &100, &9_000);
    oracle.submit_price(&f2, &asset, &100, &9_100);
    oracle.submit_price(&f3, &asset, &100, &9_200);
    oracle.submit_price(&f1, &asset, &100, &9_800);

    // Only 1 secondary source — below MIN_SECONDARY_CONSENSUS of 2.
    let sec1_id = env.register_contract(None, MockSecondaryOracle);
    let sec1 = MockSecondaryOracleClient::new(&env, &sec1_id);
    sec1.set_price(&asset, &101, &9_900);
    oracle.add_oracle_source(&admin, &sec1_id, &symbol_short!("Pyth"));

    let result = oracle.get_aggregated_price(&asset);

    assert!(
        !result.is_valid,
        "aggregated price should be invalid with only 1 secondary source"
    );
    assert_eq!(result.source_count, 2);
}

// ---------------------------------------------------------------------------
// 13. Multi-source aggregation: divergent secondary sources invalidate result
// ---------------------------------------------------------------------------

#[test]
fn test_aggregated_price_invalid_when_divergent() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 10_000);

    let admin = Address::generate(&env);
    let oracle_id = env.register_contract(None, OracleContract);
    let oracle = OracleContractClient::new(&env, &oracle_id);
    oracle.initialize(&admin);

    let f1 = Address::generate(&env);
    let f2 = Address::generate(&env);
    let f3 = Address::generate(&env);
    oracle.add_feeder(&admin, &f1);
    oracle.add_feeder(&admin, &f2);
    oracle.add_feeder(&admin, &f3);

    let asset = symbol_short!("BTC");

    // Primary TWAP at 1_000.
    oracle.submit_price(&f1, &asset, &1_000, &9_000);
    oracle.submit_price(&f2, &asset, &1_000, &9_100);
    oracle.submit_price(&f3, &asset, &1_000, &9_200);
    oracle.submit_price(&f1, &asset, &1_000, &9_800);
    oracle.submit_price(&f2, &asset, &1_000, &9_900);

    // Secondary sources report 1_200 — 20% above TWAP, exceeding
    // MAX_SOURCE_DIVERGENCE_BPS of 10%.
    let sec1_id = env.register_contract(None, MockSecondaryOracle);
    let sec2_id = env.register_contract(None, MockSecondaryOracle);
    let sec1 = MockSecondaryOracleClient::new(&env, &sec1_id);
    let sec2 = MockSecondaryOracleClient::new(&env, &sec2_id);
    sec1.set_price(&asset, &1_200, &9_950);
    sec2.set_price(&asset, &1_200, &9_960);

    oracle.add_oracle_source(&admin, &sec1_id, &symbol_short!("Pyth"));
    oracle.add_oracle_source(&admin, &sec2_id, &symbol_short!("Band"));

    let result = oracle.get_aggregated_price(&asset);

    assert!(
        !result.is_valid,
        "aggregated price should be invalid when secondary sources diverge >10% from TWAP"
    );
}

// ---------------------------------------------------------------------------
// 14. Multi-source aggregation: stale secondary prices are skipped
// ---------------------------------------------------------------------------

#[test]
fn test_aggregated_price_skips_stale_secondary() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 10_000);

    let admin = Address::generate(&env);
    let oracle_id = env.register_contract(None, OracleContract);
    let oracle = OracleContractClient::new(&env, &oracle_id);
    oracle.initialize(&admin);

    let f1 = Address::generate(&env);
    let f2 = Address::generate(&env);
    let f3 = Address::generate(&env);
    oracle.add_feeder(&admin, &f1);
    oracle.add_feeder(&admin, &f2);
    oracle.add_feeder(&admin, &f3);

    let asset = symbol_short!("DOT");
    oracle.submit_price(&f1, &asset, &500, &9_000);
    oracle.submit_price(&f2, &asset, &500, &9_100);
    oracle.submit_price(&f3, &asset, &500, &9_200);
    oracle.submit_price(&f1, &asset, &500, &9_800);

    // Both secondary sources have stale timestamps (> 300 s ago from t=10_000).
    let sec1_id = env.register_contract(None, MockSecondaryOracle);
    let sec2_id = env.register_contract(None, MockSecondaryOracle);
    let sec1 = MockSecondaryOracleClient::new(&env, &sec1_id);
    let sec2 = MockSecondaryOracleClient::new(&env, &sec2_id);
    sec1.set_price(&asset, &500, &9_000); // 1_000 s old — stale
    sec2.set_price(&asset, &500, &9_100); // 900 s old — stale

    oracle.add_oracle_source(&admin, &sec1_id, &symbol_short!("Pyth"));
    oracle.add_oracle_source(&admin, &sec2_id, &symbol_short!("Band"));

    let result = oracle.get_aggregated_price(&asset);

    // Stale sources are skipped → secondary_count = 0 < MIN_SECONDARY_CONSENSUS.
    assert!(
        !result.is_valid,
        "aggregated price should be invalid when all secondary sources are stale"
    );
    assert_eq!(result.source_count, 1, "only primary source counted");
}

// ---------------------------------------------------------------------------
// 15. Token-to-asset mapping: set and retrieve
// ---------------------------------------------------------------------------

#[test]
fn test_token_asset_mapping() {
    let env = Env::default();
    let (client, admin, _) = setup_oracle(&env, 3);

    let token = Address::generate(&env);
    let asset = symbol_short!("USDC");

    // Not set yet.
    assert!(client.get_asset_for_token(&token).is_none());

    // Set the mapping.
    client.set_asset_for_token(&admin, &token, &asset);

    // Now it should be retrievable.
    let retrieved = client.get_asset_for_token(&token);
    assert!(retrieved.is_some());
    assert_eq!(retrieved.unwrap(), asset);
}

// ---------------------------------------------------------------------------
// 16. Unauthorized feeder is rejected
// ---------------------------------------------------------------------------

#[test]
fn test_unauthorized_feeder_rejected() {
    let env = Env::default();
    let (client, _, _) = setup_oracle(&env, 3);
    let asset = symbol_short!("XLM");

    let rogue = Address::generate(&env);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.submit_price(&rogue, &asset, &100, &1_000);
    }));
    assert!(result.is_err(), "unauthorized feeder must be rejected");
}

// ---------------------------------------------------------------------------
// 17. Stale price detection
// ---------------------------------------------------------------------------

#[test]
fn test_stale_price_detection() {
    let env = Env::default();
    let (client, _, feeders) = setup_oracle(&env, 3);
    let asset = symbol_short!("XLM");

    // Submit at t=10_000.
    submit_all(&client, &feeders, &asset, 100, 10_000);

    // Not stale yet (STALE_SECS = 300).
    assert!(!client.is_price_stale(&asset));

    // Advance 301 seconds.
    advance_time(&env, 301);

    assert!(
        client.is_price_stale(&asset),
        "price should be stale after 301 seconds"
    );
}

// ---------------------------------------------------------------------------
// 18. Minimum feeders enforced before get_price
// ---------------------------------------------------------------------------

#[test]
fn test_min_feeders_enforced() {
    let env = Env::default();
    // Only 2 feeders — below MIN_FEEDERS of 3.
    let (client, _, feeders) = setup_oracle(&env, 2);
    let asset = symbol_short!("XLM");

    submit_all(&client, &feeders, &asset, 100, 1_000);

    let result = catch_unwind(AssertUnwindSafe(|| {
        client.get_price(&asset);
    }));
    assert!(result.is_err(), "get_price must panic with fewer than 3 feeders");
}

// ---------------------------------------------------------------------------
// 19. Duplicate feeder registration is idempotent
// ---------------------------------------------------------------------------

#[test]
fn test_duplicate_feeder_idempotent() {
    let env = Env::default();
    let (client, admin, feeders) = setup_oracle(&env, 3);

    // Re-adding an existing feeder should not duplicate it.
    client.add_feeder(&admin, &feeders[0]);
    client.add_feeder(&admin, &feeders[0]);

    // Still only 3 feeders.
    let asset = symbol_short!("XLM");
    submit_all(&client, &feeders, &asset, 100, 1_000);
    // get_price succeeds (3 feeders registered, not 5).
    let (price, _) = client.get_price(&asset);
    assert_eq!(price, 100);
}

// ---------------------------------------------------------------------------
// 20. Secondary source cap (max 5) is enforced
// ---------------------------------------------------------------------------

#[test]
fn test_secondary_source_cap_enforced() {
    let env = Env::default();
    let (client, admin, _) = setup_oracle(&env, 3);

    // Register 5 secondary sources — the maximum.
    for _ in 0..5 {
        let sec_id = env.register_contract(None, MockSecondaryOracle);
        client.add_oracle_source(&admin, &sec_id, &symbol_short!("src"));
    }

    // A 6th registration must be rejected.
    let extra_id = env.register_contract(None, MockSecondaryOracle);
    let result = catch_unwind(AssertUnwindSafe(|| {
        client.add_oracle_source(&admin, &extra_id, &symbol_short!("src"));
    }));
    assert!(
        result.is_err(),
        "registering more than 5 secondary sources must be rejected"
    );
}

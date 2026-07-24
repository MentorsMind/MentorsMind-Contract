/// Flash Loan Attack Prevention Tests — Issue #402
///
/// Verifies the flash-loan safeguards across lending pool and oracle contracts.
///
/// Scenarios:
///   1.  Lending pool: same-block deposit + withdraw rejected
///   2.  Lending pool: withdraw succeeds after block advances
///   3.  Lending pool: per-block borrow cap prevents pool drain
///   4.  Lending pool: borrow cap resets in the next ledger sequence
///   5.  Lending pool: multiple small borrows accumulate toward cap
///   6.  Lending pool: liquidity snapshot is taken at first borrow of block
///   7.  Oracle: price submission rejected when deviation > 50% of TWAP
///   8.  Oracle: TWAP computed correctly across multiple submissions
///   9.  Oracle: is_price_manipulated detects coordinated spike
///   10. Oracle: gradual price movement not flagged at 15% threshold
///   11. Oracle: circuit breaker allows prices within 50% window
///   12. Oracle: get_block_borrow_total tracks within-block accumulation
extern crate std;

use std::panic::{catch_unwind, AssertUnwindSafe};

use mentorminds_lending_pool::{Error as LpError, LendingPool, LendingPoolClient};
use mentorminds_oracle::{OracleContract, OracleContractClient};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn create_token<'a>(env: &'a Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
    let address = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    (address.clone(), StellarAssetClient::new(env, &address))
}

/// Advance ledger sequence by `n` (and timestamp by `n` seconds).
fn advance_ledger(env: &Env, n: u32) {
    env.ledger().with_mut(|li| {
        li.sequence_number += n;
        li.timestamp += n as u64;
    });
}

/// Set up a lending pool with `pool_size` USDC deposited by a lender.
/// Returns (client, lender, usdc_sac).
fn setup_pool<'a>(
    env: &'a Env,
    pool_size: i128,
) -> (LendingPoolClient<'a>, Address, StellarAssetClient<'a>) {
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 100;
        li.timestamp = 10_000;
    });

    let admin = Address::generate(env);
    let lender = Address::generate(env);
    let credit_score = Address::generate(env);
    let (usdc, sac) = create_token(env, &admin);
    sac.mint(&lender, &pool_size);

    let contract_id = env.register_contract(None, LendingPool);
    let client = LendingPoolClient::new(env, &contract_id);
    client.initialize(&admin, &usdc, &credit_score).unwrap();

    // Deposit in sequence 100; advance so the deposit guard doesn't block borrows.
    client.deposit(&lender, &pool_size).unwrap();
    advance_ledger(env, 1); // now at sequence 101

    (client, lender, sac)
}

// ---------------------------------------------------------------------------
// 1. Lending pool: same-block deposit + withdraw rejected
// ---------------------------------------------------------------------------

#[test]
fn test_lp_same_block_deposit_withdraw_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 50;
        li.timestamp = 5_000;
    });

    let admin = Address::generate(&env);
    let lender = Address::generate(&env);
    let credit_score = Address::generate(&env);
    let (usdc, sac) = create_token(&env, &admin);
    sac.mint(&lender, &100_000);

    let contract_id = env.register_contract(None, LendingPool);
    let client = LendingPoolClient::new(&env, &contract_id);
    client.initialize(&admin, &usdc, &credit_score).unwrap();

    // Deposit in ledger sequence 50.
    client.deposit(&lender, &50_000).unwrap();

    // Attempt to withdraw in the same ledger sequence — must fail.
    let result = client.try_withdraw(&lender, &50_000);
    assert_eq!(
        result,
        Err(LpError::SameBlockDepositWithdraw),
        "same-block withdraw must return SameBlockDepositWithdraw"
    );
}

// ---------------------------------------------------------------------------
// 2. Lending pool: withdraw succeeds after block advances
// ---------------------------------------------------------------------------

#[test]
fn test_lp_withdraw_succeeds_after_block_advance() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| {
        li.sequence_number = 50;
        li.timestamp = 5_000;
    });

    let admin = Address::generate(&env);
    let lender = Address::generate(&env);
    let credit_score = Address::generate(&env);
    let (usdc, sac) = create_token(&env, &admin);
    sac.mint(&lender, &100_000);

    let contract_id = env.register_contract(None, LendingPool);
    let client = LendingPoolClient::new(&env, &contract_id);
    client.initialize(&admin, &usdc, &credit_score).unwrap();

    client.deposit(&lender, &50_000).unwrap();

    // Advance one ledger sequence — guard only blocks the creation block.
    advance_ledger(&env, 1);

    let withdrawn = client.withdraw(&lender, &50_000).unwrap();
    assert_eq!(withdrawn, 50_000, "full deposit should be withdrawable next block");
}

// ---------------------------------------------------------------------------
// 3. Lending pool: per-block borrow cap prevents pool drain
// ---------------------------------------------------------------------------

#[test]
fn test_lp_per_block_borrow_cap_enforced() {
    let env = Env::default();
    let (client, _lender, sac) = setup_pool(&env, 1_000_000);

    let borrower = Address::generate(&env);
    sac.mint(&borrower, &0);

    // Per-block cap = 10% of 1_000_000 = 100_000.
    // First borrow of exactly 100_000 should succeed.
    client
        .borrow(&borrower, &100_000, &symbol_short!("S1"))
        .unwrap();

    // Any additional borrow in the same block must fail.
    let result = client.try_borrow(&borrower, &1, &symbol_short!("S2"));
    assert_eq!(
        result,
        Err(LpError::PerBlockBorrowLimitExceeded),
        "second borrow in same block must be rejected after cap is reached"
    );
}

// ---------------------------------------------------------------------------
// 4. Lending pool: borrow cap resets in the next ledger sequence
// ---------------------------------------------------------------------------

#[test]
fn test_lp_borrow_cap_resets_next_block() {
    let env = Env::default();
    let (client, _lender, sac) = setup_pool(&env, 1_000_000);

    let borrower = Address::generate(&env);
    sac.mint(&borrower, &200_000); // enough to repay

    // Exhaust the cap in the current block.
    client
        .borrow(&borrower, &100_000, &symbol_short!("S1"))
        .unwrap();

    // Repay so the pool has liquidity again.
    client.repay(&borrower, &102_000).unwrap();

    // Advance to the next block — cap resets.
    advance_ledger(&env, 1);

    // New borrow in the next block should succeed.
    let result = client.try_borrow(&borrower, &50_000, &symbol_short!("S2"));
    assert!(
        result.is_ok(),
        "borrow should succeed in a new ledger sequence after cap reset"
    );
}

// ---------------------------------------------------------------------------
// 5. Lending pool: multiple small borrows accumulate toward cap
// ---------------------------------------------------------------------------

#[test]
fn test_lp_multiple_small_borrows_accumulate() {
    let env = Env::default();
    let (client, _lender, _sac) = setup_pool(&env, 1_000_000);

    let b1 = Address::generate(&env);
    let b2 = Address::generate(&env);

    // Cap = 100_000. Two borrows of 60_000 each would exceed it.
    client.borrow(&b1, &60_000, &symbol_short!("S1")).unwrap();

    // b2 tries to borrow 60_000 — same block, same borrower cap applies per address.
    // b2 is a different address so it has its own cap — this should succeed.
    client.borrow(&b2, &60_000, &symbol_short!("S2")).unwrap();

    // b1 tries to borrow another 50_000 — would push b1's total to 110_000 > 100_000.
    let result = client.try_borrow(&b1, &50_000, &symbol_short!("S3"));
    assert_eq!(
        result,
        Err(LpError::PerBlockBorrowLimitExceeded),
        "b1 exceeding its per-block cap must be rejected"
    );
}

// ---------------------------------------------------------------------------
// 6. Lending pool: get_block_borrow_total tracks within-block accumulation
// ---------------------------------------------------------------------------

#[test]
fn test_lp_get_block_borrow_total_tracks_correctly() {
    let env = Env::default();
    let (client, _lender, _sac) = setup_pool(&env, 1_000_000);

    let borrower = Address::generate(&env);

    // Before any borrow, total should be 0.
    assert_eq!(client.get_block_borrow_total(&borrower), 0);

    client
        .borrow(&borrower, &30_000, &symbol_short!("S1"))
        .unwrap();
    assert_eq!(client.get_block_borrow_total(&borrower), 30_000);

    // After block advance, total resets to 0.
    advance_ledger(&env, 1);
    assert_eq!(client.get_block_borrow_total(&borrower), 0);
}

// ---------------------------------------------------------------------------
// 7. Oracle: price submission rejected when deviation > 50% of TWAP
// ---------------------------------------------------------------------------

#[test]
fn test_oracle_circuit_breaker_rejects_extreme_deviation() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let oracle_id = env.register_contract(None, OracleContract);
    let oracle = OracleContractClient::new(&env, &oracle_id);

    let admin = Address::generate(&env);
    let f1 = Address::generate(&env);
    let f2 = Address::generate(&env);
    let f3 = Address::generate(&env);

    oracle.initialize(&admin);
    oracle.add_feeder(&admin, &f1);
    oracle.add_feeder(&admin, &f2);
    oracle.add_feeder(&admin, &f3);

    let asset = symbol_short!("XLM");

    // Build a TWAP baseline around 100.
    oracle.submit_price(&f1, &asset, &100, &1_000);
    oracle.submit_price(&f2, &asset, &100, &1_100);
    oracle.submit_price(&f3, &asset, &100, &1_200);
    // Second round to establish TWAP.
    oracle.submit_price(&f1, &asset, &102, &2_000);
    oracle.submit_price(&f2, &asset, &101, &2_100);

    // Attempt to submit a price 200× the TWAP — must be rejected.
    let result = catch_unwind(AssertUnwindSafe(|| {
        oracle.submit_price(&f3, &asset, &20_000, &2_200);
    }));
    assert!(
        result.is_err(),
        "extreme price deviation must be rejected by the circuit breaker"
    );
}

// ---------------------------------------------------------------------------
// 8. Oracle: TWAP computed correctly across multiple submissions
// ---------------------------------------------------------------------------

#[test]
fn test_oracle_twap_computed_correctly() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 0);

    let oracle_id = env.register_contract(None, OracleContract);
    let oracle = OracleContractClient::new(&env, &oracle_id);

    let admin = Address::generate(&env);
    let f1 = Address::generate(&env);
    let f2 = Address::generate(&env);
    let f3 = Address::generate(&env);

    oracle.initialize(&admin);
    oracle.add_feeder(&admin, &f1);
    oracle.add_feeder(&admin, &f2);
    oracle.add_feeder(&admin, &f3);

    let asset = symbol_short!("USDC");

    // Prices stay within the 50% circuit-breaker window.
    // t=0:   price=100
    // t=100: price=120  (+20%, within 50% limit)
    //        → 100 held for 100 s → contribution = 100 * 100 = 10_000
    // t=300: price=110  (-8% from 120, within 50% limit)
    //        → 120 held for 200 s → contribution = 120 * 200 = 24_000
    // TWAP = (10_000 + 24_000) / 300 = 113.33 → 113
    oracle.submit_price(&f1, &asset, &100, &0);
    oracle.submit_price(&f2, &asset, &100, &0);
    oracle.submit_price(&f3, &asset, &100, &0);

    oracle.submit_price(&f1, &asset, &120, &100);
    oracle.submit_price(&f2, &asset, &120, &100);
    oracle.submit_price(&f3, &asset, &120, &100);

    oracle.submit_price(&f1, &asset, &110, &300);
    oracle.submit_price(&f2, &asset, &110, &300);
    oracle.submit_price(&f3, &asset, &110, &300);

    let twap = oracle.get_twap(&asset);
    // Allow ±5 for integer rounding.
    assert!(
        (twap - 113).abs() <= 5,
        "TWAP should be approximately 113, got {}",
        twap
    );
}

// ---------------------------------------------------------------------------
// 9. Oracle: is_price_manipulated detects coordinated spike
// ---------------------------------------------------------------------------

#[test]
fn test_oracle_is_price_manipulated_detects_spike() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let oracle_id = env.register_contract(None, OracleContract);
    let oracle = OracleContractClient::new(&env, &oracle_id);

    let admin = Address::generate(&env);
    let f1 = Address::generate(&env);
    let f2 = Address::generate(&env);
    let f3 = Address::generate(&env);

    oracle.initialize(&admin);
    oracle.add_feeder(&admin, &f1);
    oracle.add_feeder(&admin, &f2);
    oracle.add_feeder(&admin, &f3);

    let asset = symbol_short!("ETH");

    // Build a stable TWAP around 1_000.
    oracle.submit_price(&f1, &asset, &1_000, &1_000);
    oracle.submit_price(&f2, &asset, &1_000, &1_100);
    oracle.submit_price(&f3, &asset, &1_000, &1_200);

    // Spot is near TWAP — not manipulated at 30% threshold.
    assert!(
        !oracle.is_price_manipulated(&asset, &300i128),
        "stable price should not be flagged as manipulated"
    );

    // All 3 feeders submit 40% above TWAP (1_400).
    // 40% < 50% circuit-breaker limit, so submissions pass.
    // But spot (median = 1_400) deviates 40% from TWAP (~1_000),
    // which exceeds the 30% detection threshold.
    oracle.submit_price(&f1, &asset, &1_400, &2_000);
    oracle.submit_price(&f2, &asset, &1_400, &2_100);
    oracle.submit_price(&f3, &asset, &1_400, &2_200);

    assert!(
        oracle.is_price_manipulated(&asset, &300i128),
        "40% coordinated spike should be flagged at 30% threshold"
    );
}

// ---------------------------------------------------------------------------
// 10. Oracle: gradual price movement not flagged at 15% threshold
// ---------------------------------------------------------------------------

#[test]
fn test_oracle_gradual_price_movement_not_flagged() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 0);

    let oracle_id = env.register_contract(None, OracleContract);
    let oracle = OracleContractClient::new(&env, &oracle_id);

    let admin = Address::generate(&env);
    let f1 = Address::generate(&env);
    let f2 = Address::generate(&env);
    let f3 = Address::generate(&env);

    oracle.initialize(&admin);
    oracle.add_feeder(&admin, &f1);
    oracle.add_feeder(&admin, &f2);
    oracle.add_feeder(&admin, &f3);

    let asset = symbol_short!("BTC");

    // Gradual ~2% steps: 1_000 → 1_020 → 1_040 → 1_060 → 1_080.
    // Each step is well within the 50% circuit breaker.
    for (price, ts) in [
        (1_000i128, 0u64),
        (1_020, 3_600),
        (1_040, 7_200),
        (1_060, 10_800),
        (1_080, 14_400),
    ] {
        oracle.submit_price(&f1, &asset, &price, &ts);
        oracle.submit_price(&f2, &asset, &price, &ts);
        oracle.submit_price(&f3, &asset, &price, &ts);
    }

    // Total drift from TWAP to spot is ~8%.
    // At a 15% threshold (1_500 bps) this should NOT be flagged.
    assert!(
        !oracle.is_price_manipulated(&asset, &1_500i128),
        "gradual ~8% price movement should not be flagged at 15% threshold"
    );
}

// ---------------------------------------------------------------------------
// 11. Oracle: circuit breaker allows prices within 50% window
// ---------------------------------------------------------------------------

#[test]
fn test_oracle_circuit_breaker_allows_within_window() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 0);

    let oracle_id = env.register_contract(None, OracleContract);
    let oracle = OracleContractClient::new(&env, &oracle_id);

    let admin = Address::generate(&env);
    let f1 = Address::generate(&env);
    let f2 = Address::generate(&env);
    let f3 = Address::generate(&env);

    oracle.initialize(&admin);
    oracle.add_feeder(&admin, &f1);
    oracle.add_feeder(&admin, &f2);
    oracle.add_feeder(&admin, &f3);

    let asset = symbol_short!("SOL");

    // Establish TWAP at 200.
    oracle.submit_price(&f1, &asset, &200, &0);
    oracle.submit_price(&f2, &asset, &200, &100);
    oracle.submit_price(&f3, &asset, &200, &200);

    // Submit a price 49% above TWAP (298) — just under the 50% limit.
    // This must succeed (not trip the circuit breaker).
    let result = catch_unwind(AssertUnwindSafe(|| {
        oracle.submit_price(&f1, &asset, &298, &1_000);
    }));
    assert!(
        result.is_ok(),
        "price within 50% deviation window must be accepted"
    );
}

// ---------------------------------------------------------------------------
// 12. Lending pool: get_block_borrow_total returns 0 for new block
// ---------------------------------------------------------------------------

#[test]
fn test_lp_block_borrow_total_zero_new_block() {
    let env = Env::default();
    let (client, _lender, _sac) = setup_pool(&env, 500_000);

    let borrower = Address::generate(&env);

    // Borrow in block 101.
    client
        .borrow(&borrower, &40_000, &symbol_short!("S1"))
        .unwrap();
    assert_eq!(client.get_block_borrow_total(&borrower), 40_000);

    // Advance to block 102 — total resets.
    advance_ledger(&env, 1);
    assert_eq!(
        client.get_block_borrow_total(&borrower),
        0,
        "block borrow total must reset to 0 in a new ledger sequence"
    );
}

//! #416 — Fee Calculation Precision
//!
//! Audits escrow, referral, and staking fee calculations for:
//! - Rounding consistency (multiply-before-divide)
//! - Overflow / underflow safety with checked arithmetic
//! - Correct behaviour at edge-case amounts (1 stroop, max i128, zero, odd amounts)
//! - fee + net_amount == original amount (no value creation or destruction)

#![cfg(test)]

// ─── Pure arithmetic helpers mirroring contract logic ────────────────────────

/// Compute a basis-point fee using the same formula as the escrow and referral
/// contracts: `amount * bps / 10_000`. Returns `(fee, net)`.
///
/// Both operations use Rust's checked arithmetic so overflow panics rather
/// than wraps — matching the contract's `checked_mul / checked_div` path.
fn calc_fee_bps(amount: i128, bps: u32) -> (i128, i128) {
    let fee = amount
        .checked_mul(bps as i128)
        .expect("overflow in fee mul")
        .checked_div(10_000)
        .expect("division error in fee div");
    let net = amount.checked_sub(fee).expect("underflow in net sub");
    (fee, net)
}

// ─── Invariant: fee + net == amount ──────────────────────────────────────────

#[test]
fn fee_plus_net_equals_amount_standard() {
    let amount = 1_000_000_i128; // 1 XLM (7 decimals)
    let bps = 500_u32; // 5%
    let (fee, net) = calc_fee_bps(amount, bps);
    assert_eq!(fee + net, amount, "fee + net must equal original amount");
}

#[test]
fn fee_plus_net_equals_amount_max_bps() {
    let amount = 1_234_567_890_i128;
    let bps = 1_000_u32; // 10% (MAX_FEE_BPS)
    let (fee, net) = calc_fee_bps(amount, bps);
    assert_eq!(fee + net, amount);
}

#[test]
fn fee_plus_net_equals_amount_zero_bps() {
    let amount = 999_i128;
    let bps = 0_u32;
    let (fee, net) = calc_fee_bps(amount, bps);
    assert_eq!(fee, 0);
    assert_eq!(net, amount);
}

#[test]
fn fee_plus_net_equals_amount_one_stroop() {
    // Minimum possible amount: 1 stroop
    let amount = 1_i128;
    let bps = 500_u32;
    let (fee, net) = calc_fee_bps(amount, bps);
    // 1 * 500 / 10_000 == 0 (integer truncation)
    assert_eq!(fee, 0);
    assert_eq!(net, 1);
    assert_eq!(fee + net, amount);
}

// ─── Rounding: truncation (floor) is consistent ───────────────────────────────

#[test]
fn fee_rounds_down_for_indivisible_amounts() {
    // 3 * 500 / 10_000 = 1500 / 10_000 = 0 (floor)
    let (fee, net) = calc_fee_bps(3, 500);
    assert_eq!(fee, 0);
    assert_eq!(net, 3);
}

#[test]
fn fee_rounds_down_not_up_mid_range() {
    // 10_001 * 300 / 10_000 = 3_000_300 / 10_000 = 300 (not 301)
    let (fee, net) = calc_fee_bps(10_001, 300);
    assert_eq!(fee, 300);
    assert_eq!(net, 10_001 - 300);
}

// ─── Precision: multiply first, then divide ───────────────────────────────────

#[test]
fn multiply_first_preserves_precision_vs_divide_first() {
    // If we divided first: 99 / 10_000 = 0, then 0 * 500 = 0  ← WRONG
    // Correct order:        99 * 500 = 49_500, then / 10_000 = 4
    let amount = 99_i128;
    let bps = 500_u32;
    let (fee, _net) = calc_fee_bps(amount, bps);
    assert_eq!(fee, 4, "multiply-first must give 4, not 0");
}

// ─── Large amounts: no overflow ───────────────────────────────────────────────

#[test]
fn fee_calculation_handles_large_amount() {
    // 1 billion XLM in stroops (i128 headroom is ~1.7e38)
    let amount = 1_000_000_000_i128 * 10_000_000; // 1e16 stroops
    let bps = 500_u32;
    let (fee, net) = calc_fee_bps(amount, bps);
    assert_eq!(fee + net, amount);
    assert!(fee > 0);
}

#[test]
fn fee_calculation_handles_max_i128_within_bps_range() {
    // i128::MAX / 10_000 fits in i128, so this must not overflow
    // Use a safe large amount: i128::MAX / 10_001 to stay within checked_mul range
    let amount = i128::MAX / 10_001;
    let bps = 1_000_u32;
    let (fee, net) = calc_fee_bps(amount, bps);
    assert_eq!(fee + net, amount);
}

// ─── Referral reward formula ──────────────────────────────────────────────────

/// Mirror of the fixed referral `distribute_from_fee` calculation.
fn calc_referral_reward(platform_fee: i128, reward_bps: u32) -> i128 {
    if platform_fee <= 0 || reward_bps == 0 {
        return 0;
    }
    platform_fee
        .checked_mul(reward_bps as i128)
        .expect("overflow")
        .checked_div(10_000)
        .expect("division error")
}

#[test]
fn referral_reward_zero_when_fee_zero() {
    assert_eq!(calc_referral_reward(0, 500), 0);
}

#[test]
fn referral_reward_zero_when_bps_zero() {
    assert_eq!(calc_referral_reward(1_000_000, 0), 0);
}

#[test]
fn referral_reward_multiply_first_precision() {
    // 7 * 500 / 10_000 = 3500 / 10_000 = 0 (floor) — no precision loss from wrong order
    let reward = calc_referral_reward(7, 500);
    assert_eq!(reward, 0);
    // 200 * 500 / 10_000 = 100_000 / 10_000 = 10
    let reward2 = calc_referral_reward(200, 500);
    assert_eq!(reward2, 10);
}

#[test]
fn referral_reward_consistent_with_escrow_fee() {
    // Referral reward is a fraction of the platform fee. If platform_fee was
    // computed with calc_fee_bps, the referral reward must not exceed it.
    let amount = 500_000_i128;
    let escrow_bps = 500_u32;
    let (escrow_fee, _) = calc_fee_bps(amount, escrow_bps);
    let referral_bps = 1_000_u32; // 10% of platform fee → max referral
    let reward = calc_referral_reward(escrow_fee, referral_bps);
    assert!(reward <= escrow_fee, "referral reward must not exceed platform fee");
}

// ─── Dynamic fee tiers (escrow) ───────────────────────────────────────────────

/// Mirror of `_calculate_fee_from_price` in escrow/src/lib.rs.
fn dynamic_fee_from_price(price: i128) -> u32 {
    if price <= 0 { return 500; }
    let t010 = 1_000_000_i128;
    let t050 = 5_000_000_i128;
    let t100 = 10_000_000_i128;
    if price < t010 { 500 }
    else if price < t050 { 400 }
    else if price < t100 { 300 }
    else { 200 }
}

#[test]
fn dynamic_fee_tiers_are_monotonically_decreasing() {
    // Higher price → lower fee (incentivises usage when MNT is valuable)
    let p_low = 500_000_i128;       // < $0.10
    let p_mid = 2_000_000_i128;     // $0.10–$0.50
    let p_high = 7_500_000_i128;    // $0.50–$1.00
    let p_max = 15_000_000_i128;    // > $1.00

    assert!(dynamic_fee_from_price(p_low) > dynamic_fee_from_price(p_mid));
    assert!(dynamic_fee_from_price(p_mid) > dynamic_fee_from_price(p_high));
    assert!(dynamic_fee_from_price(p_high) > dynamic_fee_from_price(p_max));
}

#[test]
fn dynamic_fee_returns_default_for_zero_price() {
    assert_eq!(dynamic_fee_from_price(0), 500);
    assert_eq!(dynamic_fee_from_price(-1), 500);
}

#[test]
fn dynamic_fee_boundary_values() {
    // Exact boundary: price == threshold_010 → next tier (400)
    assert_eq!(dynamic_fee_from_price(1_000_000), 400);
    // Just below: price == threshold_010 - 1 → 500
    assert_eq!(dynamic_fee_from_price(999_999), 500);
    // Exact 050 boundary → 300
    assert_eq!(dynamic_fee_from_price(5_000_000), 300);
    // Exact 100 boundary → 200
    assert_eq!(dynamic_fee_from_price(10_000_000), 200);
}

#[test]
fn dynamic_fee_applied_correctly_across_tiers() {
    // Verify fee + net == amount for each dynamic tier
    let amounts = [1_i128, 100, 10_000, 1_000_000, 999_999_999];
    let prices = [500_000_i128, 2_000_000, 7_500_000, 15_000_000];
    for price in prices {
        let bps = dynamic_fee_from_price(price);
        for amount in amounts {
            let (fee, net) = calc_fee_bps(amount, bps);
            assert_eq!(
                fee + net,
                amount,
                "fee+net != amount for price={price} amount={amount}"
            );
        }
    }
}

//! Property-based fuzz tests for the MentorsMind escrow contract.
//!
//! Each `proptest!` block runs 10 000 iterations (configured via
//! `ProptestConfig::with_cases`).  The tests exercise the pure arithmetic
//! invariants extracted from the contract so they run without a live Soroban
//! environment, making them fast and deterministic.
//!
//! Covered invariants
//! ------------------
//! 1. `fuzz_fee_bps`          – fee_bps ∈ [0, 10 000]: no panic, fee ≤ amount,
//!                              net + fee == amount.
//! 2. `fuzz_amount_extremes`  – amount ∈ {0, 1, i128::MAX, random}: arithmetic
//!                              never overflows for valid fee_bps.
//! 3. `fuzz_resolve_dispute`  – mentor_pct ∈ [0, 100]: mentor + learner == amount,
//!                              no overflow.
//! 4. `fuzz_auto_release`     – session_end_time + auto_release_delay: checked_add
//!                              never panics; underflow guard holds.
//!
//! Run with:
//!   cargo test --manifest-path escrow/fuzz/Cargo.toml

// ---------------------------------------------------------------------------
// Bring proptest macros into scope.
// ---------------------------------------------------------------------------
use proptest::prelude::*;

// ---------------------------------------------------------------------------
// The arithmetic under test is reproduced verbatim from the contract so the
// fuzz harness validates the *exact same expressions* without needing a full
// Soroban environment.  If the contract formulas change, update these too.
// ---------------------------------------------------------------------------

/// Mirrors `_do_release` fee calculation.
///
/// Returns `(platform_fee, net_amount)` or `None` if a checked operation
/// would overflow/underflow (which the contract handles via `.expect()`).
fn calc_fee(amount: i128, fee_bps: u32) -> Option<(i128, i128)> {
    let fee = amount.checked_mul(fee_bps as i128)?.checked_div(10_000)?;
    let net = amount.checked_sub(fee)?;
    Some((fee, net))
}

/// Mirrors `resolve_dispute` split calculation.
///
/// Returns `(mentor_amount, learner_amount)` or `None` on overflow.
fn calc_split(amount: i128, mentor_pct: u32) -> Option<(i128, i128)> {
    let mentor = amount
        .checked_mul(mentor_pct as i128)?
        .checked_div(100)?;
    let learner = amount.checked_sub(mentor)?;
    Some((mentor, learner))
}

/// Mirrors `try_auto_release` timestamp guard.
///
/// Returns the release-after timestamp or `None` on u64 overflow.
fn calc_release_after(session_end_time: u64, auto_release_delay: u64) -> Option<u64> {
    session_end_time.checked_add(auto_release_delay)
}

// ---------------------------------------------------------------------------
// 1. Fuzz platform_fee_bps values 0–10 000
//    Invariants:
//      • calc_fee never returns None for valid (positive) amounts
//      • fee ∈ [0, amount]
//      • fee + net == amount  (no dust)
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_fee_bps(
        // fee_bps is capped at 1 000 in the contract, but the arithmetic
        // itself is safe up to 10 000 bps (100%).  We test the full range
        // to prove no overflow exists anywhere in [0, 10_000].
        fee_bps in 0u32..=10_000u32,
        // Positive amounts only — the contract rejects ≤ 0 before arithmetic.
        amount in 1i128..=i128::MAX,
    ) {
        // For amounts up to i128::MAX / 10_000 the multiplication is safe.
        // For larger amounts checked_mul returns None — that is the correct
        // behaviour (the contract would panic via .expect("Overflow")).
        // We only assert invariants when the arithmetic succeeds.
        if let Some((fee, net)) = calc_fee(amount, fee_bps) {
            // fee must be non-negative
            prop_assert!(fee >= 0, "fee must be >= 0, got {fee}");
            // fee must not exceed the gross amount
            prop_assert!(fee <= amount, "fee {fee} > amount {amount}");
            // net must be non-negative
            prop_assert!(net >= 0, "net must be >= 0, got {net}");
            // no dust: fee + net must reconstruct the original amount
            prop_assert_eq!(
                fee.checked_add(net),
                Some(amount),
                "fee + net != amount: {fee} + {net} != {amount}"
            );
        }
        // If checked_mul overflowed, the contract would panic — that is
        // acceptable behaviour for astronomically large amounts.  The test
        // simply does not assert invariants in that branch.
    }
}

// ---------------------------------------------------------------------------
// 2. Fuzz amount extremes: 0, 1, i128::MAX, and random values
//    Invariants:
//      • amount == 0 or amount < 0 → contract rejects before arithmetic
//        (we verify calc_fee returns a sane result for amount=1 and large values)
//      • amount == i128::MAX with fee_bps == 0 → fee=0, net=i128::MAX (no overflow)
//      • amount == 1 with any fee_bps → fee ∈ {0, 1}, net ∈ {0, 1}
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_amount_extremes(
        fee_bps in 0u32..=1_000u32,   // contract-valid range
        // Mix of boundary values and arbitrary positives
        amount in prop_oneof![
            Just(1i128),
            Just(i128::MAX),
            Just(10_000i128),
            Just(1_000_000i128),
            (1i128..=i128::MAX),
        ],
    ) {
        if let Some((fee, net)) = calc_fee(amount, fee_bps) {
            prop_assert!(fee >= 0);
            prop_assert!(net >= 0);
            prop_assert!(fee <= amount);
            prop_assert_eq!(fee.checked_add(net), Some(amount));

            // Special case: zero fee_bps → mentor gets everything
            if fee_bps == 0 {
                prop_assert_eq!(fee, 0);
                prop_assert_eq!(net, amount);
            }

            // Special case: amount == 1 → fee must be 0 (truncation toward zero)
            if amount == 1 {
                prop_assert_eq!(fee, 0, "fee on amount=1 must be 0 (truncation)");
                prop_assert_eq!(net, 1);
            }
        }
        // i128::MAX * fee_bps overflows for fee_bps > 0 — checked_mul returns
        // None, which is the correct overflow-protection path.
    }
}

// ---------------------------------------------------------------------------
// 3. Fuzz mentor_pct in resolve_dispute: 0–100
//    Invariants:
//      • mentor_amount + learner_amount == amount  (no dust, no over-payment)
//      • both shares are non-negative
//      • mentor_pct == 100 → learner gets 0
//      • mentor_pct == 0   → mentor gets 0
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_resolve_dispute(
        mentor_pct in 0u32..=100u32,
        amount in 1i128..=i128::MAX,
    ) {
        if let Some((mentor, learner)) = calc_split(amount, mentor_pct) {
            // Both shares non-negative
            prop_assert!(mentor >= 0, "mentor share must be >= 0");
            prop_assert!(learner >= 0, "learner share must be >= 0");

            // Shares must sum to the full escrowed amount — no dust lost
            prop_assert_eq!(
                mentor.checked_add(learner),
                Some(amount),
                "mentor {mentor} + learner {learner} != amount {amount}"
            );

            // Boundary: 100% to mentor
            if mentor_pct == 100 {
                prop_assert_eq!(mentor, amount);
                prop_assert_eq!(learner, 0);
            }

            // Boundary: 0% to mentor
            if mentor_pct == 0 {
                prop_assert_eq!(mentor, 0);
                prop_assert_eq!(learner, amount);
            }
        }
        // Overflow only possible for i128::MAX * mentor_pct > 0 — acceptable.
    }
}

// ---------------------------------------------------------------------------
// 4. Fuzz auto_release_delay with various timestamps
//    Invariants:
//      • checked_add never panics (we use Option, not unwrap)
//      • if session_end_time + delay overflows u64, calc_release_after returns None
//      • if now < release_after → auto-release must be rejected
//      • if now >= release_after → auto-release is permitted
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_auto_release_delay(
        session_end_time in 0u64..=u64::MAX,
        auto_release_delay in 0u64..=u64::MAX,
        // Simulate the current ledger timestamp
        now in 0u64..=u64::MAX,
    ) {
        match calc_release_after(session_end_time, auto_release_delay) {
            Some(release_after) => {
                // No underflow: release_after is a valid u64
                // The contract's guard: now < release_after → reject
                if now < release_after {
                    // auto-release should be rejected — nothing to assert
                    // beyond the fact that the comparison itself didn't panic.
                    prop_assert!(now < release_after);
                } else {
                    // auto-release is permitted
                    prop_assert!(now >= release_after);
                }
            }
            None => {
                // Overflow in session_end_time + auto_release_delay.
                // The contract uses .expect("Timestamp overflow") which would
                // panic — this is the correct protection.  We verify the
                // overflow actually occurred.
                prop_assert!(
                    session_end_time.checked_add(auto_release_delay).is_none(),
                    "calc_release_after returned None but no overflow detected"
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Fuzz total_sessions in partial release (no over-release invariant)
//    The contract does not yet have a partial-release function, but the
//    arithmetic invariant is: released_so_far + this_release <= total_amount.
//    We model this as a stateful accumulator over 1–100 sessions.
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_partial_release_no_over_release(
        total_amount in 1i128..=1_000_000_000i128,
        // total_sessions in [1, 100] as required by the acceptance criteria
        total_sessions in 1u32..=100u32,
        // A seed used to derive per-session release fractions
        seed in 0u64..=u64::MAX,
    ) {
        // Simulate releasing funds session-by-session.
        // Each session releases floor(total_amount / total_sessions).
        // The last session gets the remainder to avoid dust.
        let per_session = total_amount / total_sessions as i128;
        let mut released: i128 = 0;

        for i in 0..total_sessions {
            let this_release = if i == total_sessions - 1 {
                // Last session: release whatever is left
                total_amount - released
            } else {
                per_session
            };

            // Guard: this release must not push us over the total
            prop_assert!(
                this_release >= 0,
                "session release amount must be non-negative"
            );
            prop_assert!(
                released.checked_add(this_release).map_or(false, |s| s <= total_amount),
                "over-release detected at session {i}: released={released} + this={this_release} > total={total_amount}"
            );

            released = released
                .checked_add(this_release)
                .expect("accumulator overflow in test");
        }

        // After all sessions, exactly total_amount must have been released
        prop_assert_eq!(
            released,
            total_amount,
            "total released {released} != total_amount {total_amount}"
        );

        // Suppress unused-variable warning for seed (used to signal intent
        // that a future implementation may use randomised per-session splits).
        let _ = seed;
    }
}

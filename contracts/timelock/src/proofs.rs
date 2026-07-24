//! Kani formal-verification harnesses for the TimelockController — Issue #622.
//!
//! Kani verifies native Rust, not the Soroban WASM calling convention, and the
//! `#[contractimpl]` entry points cannot be symbolically executed through the
//! host `Env`. We therefore prove the security-critical invariants over the
//! pure predicates in [`crate::logic`], which are the exact functions the
//! contract entry points depend on. See `VERIFICATION.md` for the full
//! verification boundary, assumptions, and unverified paths.
//!
//! Run with: `cargo kani` (from `contracts/timelock`).

use crate::logic;
use crate::{MAX_DELAY, MIN_DELAY, OPERATION_EXPIRY_SECS, TIMESTAMP_TOLERANCE_SECS};

/// Invariant 1: `schedule` always sets `ready_at > now`.
///
/// For any non-overflowing `now` and any valid delay, the computed `ready_at`
/// is strictly greater than the current timestamp (because `MIN_DELAY > 0`).
#[kani::proof]
#[kani::unwind(32)]
fn proof_schedule_sets_future_ready_at() {
    let now: u64 = kani::any();
    let delay: u64 = kani::any();

    kani::assume(logic::is_valid_delay(delay));
    // Bound `now` so `now + delay` cannot overflow u64 — the contract uses
    // `checked_add(...).expect(...)`, so overflow is a rejected (panicking)
    // path, not a reachable success path.
    kani::assume(now <= u64::MAX - MAX_DELAY);

    let ready_at = logic::compute_ready_at(now, delay).unwrap();
    assert!(ready_at > now, "ready_at must be strictly in the future");
    // MIN_DELAY is the tightest lower bound on the gap.
    assert!(ready_at >= now + MIN_DELAY);
}

/// Invariant 2: `execute` is only reachable when
/// `now >= ready_at + TOLERANCE && now < ready_at + EXPIRY`.
///
/// We prove both directions of the executability predicate.
#[kani::proof]
#[kani::unwind(32)]
fn proof_execute_window() {
    let now: u64 = kani::any();
    let ready_at: u64 = kani::any();

    // Avoid the overflow path in the expiry computation (rejected by the
    // contract's `checked_add`).
    kani::assume(ready_at <= u64::MAX - OPERATION_EXPIRY_SECS);

    let executable = logic::is_executable(now, ready_at);

    if executable {
        // Forward: executable implies inside the window.
        assert!(now >= ready_at.saturating_add(TIMESTAMP_TOLERANCE_SECS));
        assert!(now < ready_at + OPERATION_EXPIRY_SECS);
    } else {
        // Reverse: inside the window implies executable (no false negatives).
        let in_window = now >= ready_at.saturating_add(TIMESTAMP_TOLERANCE_SECS)
            && now < ready_at + OPERATION_EXPIRY_SECS;
        assert!(!in_window);
    }
}

/// Invariant 3: once `done = true`, no further state transition is possible.
///
/// Both `execute` and `cancel` guard on `!done`; `can_transition(done)` is the
/// shared predicate. A done operation can never transition again.
#[kani::proof]
#[kani::unwind(32)]
fn proof_done_is_terminal() {
    let done: bool = kani::any();
    let transitionable = logic::can_transition(done);
    if done {
        assert!(!transitionable, "a done operation must be terminal");
    } else {
        assert!(transitionable);
    }
}

/// Invariant 4: `cancel` is callable by proposer OR admin, and never requires
/// both simultaneously.
///
/// We prove that authorization holds whenever at least one role matches, and
/// that a single matching role is always sufficient (no conjunction required).
#[kani::proof]
#[kani::unwind(32)]
fn proof_cancel_authorization() {
    let is_proposer: bool = kani::any();
    let is_admin: bool = kani::any();

    let allowed = logic::can_cancel(is_proposer, is_admin);

    // Either role alone is sufficient.
    if is_proposer || is_admin {
        assert!(allowed);
    } else {
        assert!(!allowed);
    }

    // Never requires both: whenever exactly one role is present, still allowed.
    if is_proposer && !is_admin {
        assert!(allowed);
    }
    if is_admin && !is_proposer {
        assert!(allowed);
    }
}

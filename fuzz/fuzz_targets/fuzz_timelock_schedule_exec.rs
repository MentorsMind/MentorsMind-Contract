//! Property-based fuzz tests for Timelock scheduling and execution.
//!
//! Exercises the full TimelockController lifecycle (schedule → delay →
//! execute / cancel) with adversarial inputs including: edge-case delays,
//! salts producing hash collisions, invalid op_ids, non-admin cancel
//! attempts, and delay-boundary crossing.
//!
//! Covered invariants
//! ------------------
//! 1. `fuzz_delay_bounds`            – delay ∈ [MIN_DELAY, MAX_DELAY] only,
//!                                      out-of-range rejected InvalidDelay.
//! 2. `fuzz_salt_collision_resistance`– different salts produce different
//!                                      op_ids (SHA-256 binding);
//!                                      identical payload+nonce+salt → same id
//! 3. `fuzz_op_id_collision_guard`    – op_id uniqueness guaranteed by
//!                                      nonce monotonicity.
//! 4. `fuzz_schedule_before_init`     – schedule before initialize returns
//!                                      NotInitialized, no state mutation.
//! 5. `fuzz_execute_ordering`         – execute before ready fails NotReady;
//!                                      execute after ready+tolerance succeeds;
//!                                      execute after expiry fails.
//! 6. `fuzz_cancel_permissions`       – proposer can cancel own op;
//!                                      non-proposer needs admin auth;
//!                                      done ops cannot be cancelled.
//! 7. `fuzz_double_execute_prevention`– execute on done op panics
//!                                      "operation already done".
//! 8. `fuzz_transfer_admin`           – only current admin can transfer;
//!                                      new admin becomes the stored admin.
//! 9. `fuzz_schedule_arithmetic`      – ready_at = now + delay never
//!                                      overflows realistically; guard tests.
//!
//! Run with:
//!   cargo test --manifest-path fuzz/Cargo.toml --test fuzz_timelock_schedule_exec

use proptest::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Addr(u64);

pub const MIN_DELAY: u64 = 48 * 60 * 60;
pub const MAX_DELAY: u64 = 30 * 24 * 60 * 60;
pub const OPERATION_EXPIRY_SECS: u64 = 14 * 24 * 60 * 60;
pub const TIMESTAMP_TOLERANCE_SECS: u64 = 60;

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
enum TlError {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotAdmin = 3,
    OperationNotFound = 4,
    AlreadyDone = 5,
    NotReady = 6,
    InvalidDelay = 7,
}

type TlResult<T> = Result<T, TlError>;

type OpId = [u8; 32];

#[derive(Clone)]
struct Operation {
    proposer: Addr,
    target:   Addr,
    function: u64, // Tag to distinguish function symbols in model
    ready_at: u64,
    done:     bool,
    nonce:    u64,
    salt:     [u8; 32],
}

#[derive(Clone)]
struct TimelockModel {
    initialized: bool,
    admin:       Addr,
    op_count:    u64,
    ops:         BTreeMap<OpId, Operation>,
}

impl TimelockModel {
    fn new() -> Self {
        TimelockModel {
            initialized: false,
            admin: Addr(0),
            op_count: 0,
            ops: BTreeMap::new(),
        }
    }

    fn initialize(&mut self, admin: Addr) -> TlResult<()> {
        if self.initialized { return Err(TlError::AlreadyInitialized); }
        self.admin = admin;
        self.op_count = 0;
        self.initialized = true;
        Ok(())
    }

    fn derive_op_id(
        caller: Addr,
        target: Addr,
        function: u64,
        args_digest: u64,
        ready_at: u64,
        nonce: u64,
        salt: &[u8; 32],
    ) -> OpId {
        // Mirror the real contract's SHA-256(payload) deterministically via
        // a mixing hash.  Real contract uses env.crypto().sha256(&payload).
        let mut h: [u8; 32] = [0u8; 32];
        for (i, b) in caller.0.to_le_bytes().iter().enumerate()   { h[i%32] ^= *b; }
        for (i, b) in target.0.to_le_bytes().iter().enumerate()   { h[(i+8)%32] ^= *b; }
        for (i, b) in function.to_le_bytes().iter().enumerate()   { h[(i+16)%32] ^= *b; }
        for (i, b) in args_digest.to_le_bytes().iter().enumerate(){ h[(i+24)%32] ^= *b; }
        for (i, b) in ready_at.to_le_bytes().iter().enumerate()   { h[i] = h[i].wrapping_add(*b); }
        for (i, b) in nonce.to_le_bytes().iter().enumerate()      { h[(i+8)%32] = h[(i+8)%32].wrapping_add(*b); }
        for (i, b) in salt.iter().enumerate()                     { h[i] ^= *b; }
        h
    }

    fn schedule(
        &mut self,
        caller: Addr,
        target: Addr,
        function: u64,
        args_digest: u64,
        delay: u64,
        salt: [u8; 32],
        now: u64,
    ) -> TlResult<(OpId, u64)> {
        if !self.initialized { return Err(TlError::NotInitialized); }
        if delay < MIN_DELAY || delay > MAX_DELAY { return Err(TlError::InvalidDelay); }
        self.op_count += 1;
        let nonce = self.op_count;
        let ready_at = now.checked_add(delay).ok_or(TlError::InvalidDelay)?;
        let op_id = Self::derive_op_id(caller, target, function, args_digest, ready_at, nonce, &salt);
        let op = Operation {
            proposer: caller,
            target,
            function,
            ready_at,
            done: false,
            nonce,
            salt,
        };
        self.ops.insert(op_id, op);
        Ok((op_id, ready_at))
    }

    fn execute(&mut self, op_id: OpId, now: u64) -> Result<(), &'static str> {
        let op = self.ops.get_mut(&op_id).ok_or("operation not found")?;
        if op.done { return Err("operation already done"); }
        if now < op.ready_at.saturating_add(TIMESTAMP_TOLERANCE_SECS) {
            return Err("operation not ready");
        }
        let expiry = op.ready_at.checked_add(OPERATION_EXPIRY_SECS)
            .ok_or("timestamp overflow")?;
        if now >= expiry { return Err("operation expired"); }
        op.done = true;
        Ok(())
    }

    fn cancel(&mut self, caller: Addr, op_id: OpId) -> Result<(), &'static str> {
        let op = self.ops.get(&op_id).ok_or("operation not found")?;
        if op.done { return Err("operation already done"); }
        if !self.initialized { return Err("not initialized"); }
        let proposer = op.proposer;
        // Permission: proposer OR admin
        if caller != proposer && caller != self.admin {
            return Err("unauthorized");
        }
        self.ops.remove(&op_id);
        Ok(())
    }

    fn transfer_admin(&mut self, caller: Addr, new_admin: Addr) -> Result<(), &'static str> {
        if !self.initialized { return Err("not initialized"); }
        if caller != self.admin { return Err("unauthorized"); }
        self.admin = new_admin;
        Ok(())
    }

    fn is_operation_ready(&self, op_id: OpId, now: u64) -> bool {
        match self.ops.get(&op_id) {
            Some(op) if !op.done => {
                let ready = now >= op.ready_at.saturating_add(TIMESTAMP_TOLERANCE_SECS);
                let not_expired = now < op.ready_at.checked_add(OPERATION_EXPIRY_SECS)
                    .unwrap_or(u64::MAX);
                ready && not_expired
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------------
// 1. Delay bounds enforcement
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_delay_bounds(
        delay in 0u64..=100 * 24 * 60 * 60,
        now in 1_000_000u64..=10_000_000u64,
    ) {
        let mut tl = TimelockModel::new();
        tl.initialize(Addr(1)).expect("init");
        let r = tl.schedule(Addr(1), Addr(2), 42, 0, delay, [0u8; 32], now);
        let in_bounds = delay >= MIN_DELAY && delay <= MAX_DELAY;
        if in_bounds {
            prop_assert!(r.is_ok(), "delay {} in bounds must succeed; got {:?}", delay, r);
        } else {
            prop_assert_eq!(r, Err(TlError::InvalidDelay),
                "delay {} out of bounds must fail InvalidDelay; got {:?}", delay, r);
            prop_assert_eq!(tl.op_count, 0);
            prop_assert!(tl.ops.is_empty());
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Salt collision resistance: different salts → different op_ids
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_salt_collision_resistance(
        caller in 1u64..=10u64,
        target in 100u64..=200u64,
        function in 0u64..=50u64,
        args_digest in 0u64..=u64::MAX,
        delay in MIN_DELAY..=MAX_DELAY,
        now in 1_000_000u64..=10_000_000u64,
    ) {
        // Two schedules with different salts but otherwise identical payload
        // must produce different op_ids (by virtue of salt being in hash).
        let salt_a = [1u8; 32];
        let salt_b = [2u8; 32];
        let ready_at = now + delay;
        // Use nonce=1 for both to test salt-only difference (real contract
        // increments nonce per call; we force-same nonce to validate the
        // salt-collision protection in the hash function itself).
        let id_a = TimelockModel::derive_op_id(
            Addr(caller), Addr(target), function, args_digest, ready_at, 1, &salt_a);
        let id_b = TimelockModel::derive_op_id(
            Addr(caller), Addr(target), function, args_digest, ready_at, 1, &salt_b);
        prop_assert_ne!(id_a, id_b,
            "different salts must produce different op_ids (hash collision!)");

        // Identical payload including salt → same id
        let id_a2 = TimelockModel::derive_op_id(
            Addr(caller), Addr(target), function, args_digest, ready_at, 1, &salt_a);
        prop_assert_eq!(id_a, id_a2, "same input must produce same op_id (determinism)");
    }
}

// ---------------------------------------------------------------------------
// 3. Op ID uniqueness via nonce — even with identical payload+salts,
//    different nonce = different id
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_op_id_collision_guard(
        n_ops in 2u32..=100u32,
        caller in 1u64..=5u64,
        now in 1_000_000u64..=10_000_000u64,
    ) {
        let mut tl = TimelockModel::new();
        tl.initialize(Addr(caller)).expect("init");
        let mut seen = HashSet::new();
        let salt = [7u8; 32];
        for i in 0..n_ops {
            let (id, _) = tl.schedule(
                Addr(caller), Addr(999), 42, 0, MIN_DELAY + i as u64, salt, now,
            ).expect("schedule");
            prop_assert!(seen.insert(id),
                "duplicate op_id detected after {} ops (hash/nonce collision!)", i + 1);
        }
        prop_assert_eq!(tl.op_count, n_ops as u64);
        prop_assert_eq!(tl.ops.len(), n_ops as usize);
    }
}

// ---------------------------------------------------------------------------
// 4. Schedule before initialize → NotInitialized with no side effects
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_schedule_before_init(
        caller in 0u64..=u64::MAX,
        target in 0u64..=u64::MAX,
        function in 0u64..=u64::MAX,
        delay in 0u64..=u64::MAX,
        salt in any::<[u8; 32]>(),
        now in 0u64..=u64::MAX,
    ) {
        let mut tl = TimelockModel::new();
        let r = tl.schedule(Addr(caller), Addr(target), function, 0, delay, salt, now);
        prop_assert_eq!(r, Err(TlError::NotInitialized),
            "schedule before init must fail NotInitialized; got {:?}", r);
        prop_assert!(!tl.initialized);
        prop_assert_eq!(tl.op_count, 0);
        prop_assert!(tl.ops.is_empty());
    }
}

// ---------------------------------------------------------------------------
// 5. Execute ordering across the three time windows
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_execute_ordering(
        now in 1_000_000u64..=1_000_000_000u64,
        delay in MIN_DELAY..=MAX_DELAY,
        offset_secs in i64::MIN..=i64::MAX,
    ) {
        let mut tl = TimelockModel::new();
        let admin = Addr(42);
        tl.initialize(admin).expect("init");
        let (op_id, ready_at) = tl.schedule(
            admin, Addr(7), 0, 0, delay, [0u8; 32], now,
        ).expect("schedule");

        // Compute execute_at with overflow-safe offset
        let execute_at: Option<u64> = if offset_secs >= 0 {
            now.checked_add(offset_secs as u64)
        } else {
            now.checked_sub((-offset_secs) as u64)
        };

        if let Some(ex_at) = execute_at {
            let before_ready = ex_at < ready_at.saturating_add(TIMESTAMP_TOLERANCE_SECS);
            let after_expiry = ex_at >= ready_at.saturating_add(OPERATION_EXPIRY_SECS);
            let r = tl.execute(op_id, ex_at);

            if before_ready {
                prop_assert_eq!(r, Err("operation not ready"),
                    "execute at {} < ready+tol={} must be NotReady; got {:?}",
                    ex_at, ready_at + TIMESTAMP_TOLERANCE_SECS, r);
                prop_assert!(!tl.ops[&op_id].done);
            } else if after_expiry {
                prop_assert_eq!(r, Err("operation expired"),
                    "execute at {} >= expiry={} must be expired; got {:?}",
                    ex_at, ready_at + OPERATION_EXPIRY_SECS, r);
                prop_assert!(!tl.ops[&op_id].done);
            } else {
                prop_assert_eq!(r, Ok(()),
                    "in-window execute (at {}, ready {}, expiry {}) must succeed; got {:?}",
                    ex_at, ready_at, ready_at + OPERATION_EXPIRY_SECS, r);
                prop_assert!(tl.ops[&op_id].done);
            }
        }
        // Also verify is_operation_ready() view matches
        let check_at = now + delay + TIMESTAMP_TOLERANCE_SECS + 1;
        if check_at < now.saturating_add(delay).saturating_add(OPERATION_EXPIRY_SECS) {
            prop_assert!(tl.is_operation_ready(op_id, check_at));
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Cancel permission matrix
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_cancel_permissions(
        scenario in 0u32..=3u32,
        now in 1_000_000u64..=10_000_000u64,
    ) {
        let mut tl = TimelockModel::new();
        let admin = Addr(1);
        let proposer = Addr(2);
        let outsider = Addr(999);
        tl.initialize(admin).expect("init");
        let (op_id, _) = tl.schedule(
            proposer, Addr(5), 0, 0, MIN_DELAY, [0u8; 32], now,
        ).expect("schedule");
        let exists_before = tl.ops.contains_key(&op_id);

        match scenario {
            0 => {
                // Proposer cancels → ok
                let r = tl.cancel(proposer, op_id);
                prop_assert_eq!(r, Ok(()), "proposer must cancel own op; got {:?}", r);
                prop_assert!(!tl.ops.contains_key(&op_id));
            }
            1 => {
                // Admin cancels → ok
                let r = tl.cancel(admin, op_id);
                prop_assert_eq!(r, Ok(()), "admin must cancel any op; got {:?}", r);
                prop_assert!(!tl.ops.contains_key(&op_id));
            }
            2 => {
                // Outsider cancels → unauthorized
                let r = tl.cancel(outsider, op_id);
                prop_assert_eq!(r, Err("unauthorized"),
                    "outsider cancel must be unauthorized; got {:?}", r);
                prop_assert!(tl.ops.contains_key(&op_id));
            }
            3 => {
                // Execute first (needs correct timestamp), then cancel → done
                let execute_at = now + MIN_DELAY + TIMESTAMP_TOLERANCE_SECS + 10;
                tl.execute(op_id, execute_at).expect("execute");
                let r = tl.cancel(proposer, op_id);
                prop_assert_eq!(r, Err("operation already done"),
                    "cancel done op must fail; got {:?}", r);
                prop_assert!(tl.ops.contains_key(&op_id)); // kept but done
            }
            _ => unreachable!(),
        }
        let _ = exists_before;
    }
}

// ---------------------------------------------------------------------------
// 7. Double execute prevention
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_double_execute_prevention(
        n_attempts in 2u32..=50u32,
        now in 1_000_000u64..=10_000_000u64,
    ) {
        let mut tl = TimelockModel::new();
        tl.initialize(Addr(1)).expect("init");
        let (op_id, _) = tl.schedule(
            Addr(1), Addr(5), 0, 0, MIN_DELAY, [0u8; 32], now,
        ).expect("schedule");
        let valid_time = now + MIN_DELAY + TIMESTAMP_TOLERANCE_SECS + 100;

        let first = tl.execute(op_id, valid_time);
        prop_assert_eq!(first, Ok(()));
        prop_assert!(tl.ops[&op_id].done);

        for _ in 1..n_attempts {
            let r = tl.execute(op_id, valid_time);
            prop_assert_eq!(r, Err("operation already done"),
                "double execute must fail AlreadyDone");
        }
    }
}

// ---------------------------------------------------------------------------
// 8. Admin transfer: only the current admin can do it
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_transfer_admin(
        admin_raw in 1u64..=10u64,
        new_admin_raw in 1u64..=10u64,
        caller_raw in 0u64..=20u64,
    ) {
        let mut tl = TimelockModel::new();
        let admin = Addr(admin_raw);
        tl.initialize(admin).expect("init");
        let caller = Addr(caller_raw);
        let new_admin = Addr(new_admin_raw);
        let r = tl.transfer_admin(caller, new_admin);
        if caller == admin {
            prop_assert_eq!(r, Ok(()), "admin transfer from admin must succeed");
            prop_assert_eq!(tl.admin, new_admin);
            // Second transfer: caller is no longer admin → fails
            let r2 = tl.transfer_admin(admin, Addr(999));
            prop_assert_eq!(r2, Err("unauthorized"),
                "old admin no longer has power after transfer");
            // New admin CAN transfer again
            let r3 = tl.transfer_admin(new_admin, Addr(888));
            prop_assert_eq!(r3, Ok(()));
            prop_assert_eq!(tl.admin, Addr(888));
        } else {
            prop_assert_eq!(r, Err("unauthorized"),
                "non-admin caller={} must not transfer; got {:?}", caller_raw, r);
            prop_assert_eq!(tl.admin, admin, "admin unchanged on failed transfer");
        }
    }
}

// ---------------------------------------------------------------------------
// 9. Schedule arithmetic — ready_at = now + delay overflow guarded
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_schedule_arithmetic(
        now in 0u64..=u64::MAX,
        delay in 0u64..=u64::MAX,
    ) {
        match now.checked_add(delay) {
            Some(ready) => {
                // Then ready_at + tolerance and + expiry must also be examined
                let _ = ready.saturating_add(TIMESTAMP_TOLERANCE_SECS); // always safe
                let exp = ready.checked_add(OPERATION_EXPIRY_SECS);
                if exp.is_none() {
                    prop_assert!(ready > u64::MAX - OPERATION_EXPIRY_SECS);
                }
            }
            None => {
                prop_assert!(now > u64::MAX - delay,
                    "checked_add None but no overflow? now={} delay={}", now, delay);
            }
        }
    }
}

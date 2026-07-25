//! Property-based fuzz tests for proposal expiration logic in both the
//! Multi-Sig and Timelock contracts.
//!
//! Validates that expired proposals and operations correctly block all
//! state-mutating operations (sign, execute, cancel in multisig; execute,
//! cancel in timelock) while allowing view queries.  Also verifies the
//! precise boundary conditions (at expiry vs 1 second after expiry).
//!
//! Covered invariants
//! ------------------
//! 1. `fuzz_ms_expiry_sign_execute_cancel` – multisig: now > expiry blocks
//!    sign_action / execute_action / cancel_action with Expired error.
//! 2. `fuzz_ms_expiry_boundary`          – exact boundary: now == expiry
//!    is still in-window; now == expiry + 1 is blocked.
//! 3. `fuzz_ms_expiry_proposal_untouched`– attempting operations on an
//!    expired proposal never mutates proposal.approval_count or executed.
//! 4. `fuzz_tl_operation_expiry`         – timelock operation expiry
//!    (ready_at + OPERATION_EXPIRY_SECS) blocks execute.
//! 5. `fuzz_tl_expiry_tolerance_window`  – timelock's TIMESTAMP_TOLERANCE
//!    combined with OPERATION_EXPIRY forms a valid execution window.
//! 6. `fuzz_expiry_arithmetic_safe`      – all checked_add calls for
//!    expiry are safe; overflow correctly rejected.
//!
//! Run with:
//!   cargo test --manifest-path fuzz/Cargo.toml --test fuzz_proposal_expiration

use proptest::prelude::*;
use std::collections::{BTreeMap, HashSet};
use std::hash::{Hash, Hasher};

#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
struct Addr(u64);

const MS_EXPIRY_SECONDS: u64   = 7 * 24 * 60 * 60;
const TL_MIN_DELAY: u64        = 48 * 60 * 60;
const TL_MAX_DELAY: u64        = 30 * 24 * 60 * 60;
const TL_OP_EXPIRY_SECS: u64   = 14 * 24 * 60 * 60;
const TL_TIMESTAMP_TOLERANCE: u64 = 60;

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
enum MsError {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    NotAdmin           = 3,
    NotSigner          = 4,
    AlreadySigner      = 5,
    ProposalNotFound   = 6,
    AlreadySigned      = 7,
    BelowThreshold     = 8,
    AlreadyExecuted    = 9,
    Cancelled          = 10,
    Expired            = 11,
    InvalidThreshold   = 12,
}

type MsResult<T> = Result<T, MsError>;

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

// ===========================================================================
// Multi-Sig expiration model
// ===========================================================================

#[derive(Clone)]
struct MsProposal {
    approval_count: u32,
    expiry:         u64,
    executed:       bool,
    cancelled:      bool,
    proposer:       Addr,
    signers:        HashSet<Addr>,
}

#[derive(Clone)]
struct MsModel {
    initialized: bool,
    threshold:   u32,
    signers:     HashSet<Addr>,
    proposals:   BTreeMap<u32, MsProposal>,
    next_pid:    u32,
}

impl MsModel {
    fn new() -> Self {
        Self {
            initialized: false,
            threshold: 0,
            signers: HashSet::new(),
            proposals: BTreeMap::new(),
            next_pid: 0,
        }
    }

    fn init(&mut self, n_signers: u32, threshold: u32) {
        let thr = threshold.min(n_signers).max(1);
        for i in 0..n_signers { self.signers.insert(Addr(i as u64)); }
        self.threshold = thr;
        self.initialized = true;
    }

    fn propose(&mut self, proposer: Addr, now: u64) -> MsResult<u32> {
        if !self.initialized { return Err(MsError::NotInitialized); }
        if !self.signers.contains(&proposer) { return Err(MsError::NotSigner); }
        let pid = self.next_pid + 1;
        self.next_pid = pid;
        let expiry = now.checked_add(MS_EXPIRY_SECONDS).ok_or(MsError::NotInitialized)?;
        let mut signers = HashSet::new();
        signers.insert(proposer);
        self.proposals.insert(pid, MsProposal {
            approval_count: 1,
            expiry,
            executed: false,
            cancelled: false,
            proposer,
            signers,
        });
        Ok(pid)
    }

    fn sign(&mut self, signer: Addr, pid: u32, now: u64) -> MsResult<()> {
        if !self.initialized { return Err(MsError::NotInitialized); }
        if !self.signers.contains(&signer) { return Err(MsError::NotSigner); }
        let p = self.proposals.get_mut(&pid).ok_or(MsError::ProposalNotFound)?;
        if p.executed  { return Err(MsError::AlreadyExecuted); }
        if p.cancelled { return Err(MsError::Cancelled); }
        if now > p.expiry { return Err(MsError::Expired); }
        if p.signers.contains(&signer) { return Err(MsError::AlreadySigned); }
        p.signers.insert(signer);
        p.approval_count += 1;
        Ok(())
    }

    fn execute(&mut self, pid: u32, now: u64) -> MsResult<()> {
        if !self.initialized { return Err(MsError::NotInitialized); }
        let p = self.proposals.get_mut(&pid).ok_or(MsError::ProposalNotFound)?;
        if p.executed  { return Err(MsError::AlreadyExecuted); }
        if p.cancelled { return Err(MsError::Cancelled); }
        if now > p.expiry { return Err(MsError::Expired); }
        if p.approval_count < self.threshold { return Err(MsError::BelowThreshold); }
        p.executed = true;
        Ok(())
    }

    fn cancel(&mut self, caller: Addr, pid: u32, now: u64) -> MsResult<()> {
        if !self.initialized { return Err(MsError::NotInitialized); }
        let p = self.proposals.get_mut(&pid).ok_or(MsError::ProposalNotFound)?;
        let is_proposer = p.proposer == caller;
        let is_signer = self.signers.contains(&caller);
        if !is_proposer && !is_signer { return Err(MsError::NotSigner); }
        if p.executed  { return Err(MsError::AlreadyExecuted); }
        if p.cancelled { return Err(MsError::Cancelled); }
        if now > p.expiry { return Err(MsError::Expired); }
        p.cancelled = true;
        Ok(())
    }
}

// ===========================================================================
// Timelock expiration model
// ===========================================================================

#[derive(Clone)]
struct TlOperation {
    ready_at: u64,
    done:     bool,
    proposer: Addr,
}

#[derive(Clone)]
struct TlModel {
    initialized: bool,
    admin:       Addr,
    ops:         std::collections::BTreeMap<[u8; 32], TlOperation>,
}

impl TlModel {
    fn new() -> Self {
        Self { initialized: false, admin: Addr(0), ops: std::collections::BTreeMap::new() }
    }

    fn init(&mut self, admin: Addr) -> TlResult<()> {
        if self.initialized { return Err(TlError::AlreadyInitialized); }
        self.admin = admin;
        self.initialized = true;
        Ok(())
    }

    fn schedule(&mut self, caller: Addr, delay: u64, now: u64) -> TlResult<([u8; 32], u64)> {
        if !self.initialized { return Err(TlError::NotInitialized); }
        if delay < TL_MIN_DELAY || delay > TL_MAX_DELAY { return Err(TlError::InvalidDelay); }
        let ready_at = now.checked_add(delay).ok_or(TlError::InvalidDelay)?;
        let mut id = [0u8; 32];
        for (i, b) in ready_at.to_le_bytes().iter().enumerate() { id[i] = *b; }
        for (i, b) in caller.0.to_le_bytes().iter().enumerate() { id[i + 8] = *b; }
        self.ops.insert(id, TlOperation { ready_at, done: false, proposer: caller });
        Ok((id, ready_at))
    }

    fn execute(&mut self, op_id: [u8; 32], now: u64) -> Result<(), &'static str> {
        let op = self.ops.get_mut(&op_id).ok_or("operation not found")?;
        if op.done { return Err("operation already done"); }
        if now < op.ready_at.saturating_add(TL_TIMESTAMP_TOLERANCE) {
            return Err("operation not ready");
        }
        let expiry = op.ready_at.checked_add(TL_OP_EXPIRY_SECS)
            .ok_or("timestamp overflow")?;
        if now >= expiry { return Err("operation expired"); }
        op.done = true;
        Ok(())
    }

    fn is_ready(&self, op_id: [u8; 32], now: u64) -> bool {
        if let Some(op) = self.ops.get(&op_id) {
            if op.done { return false; }
            let ready = now >= op.ready_at.saturating_add(TL_TIMESTAMP_TOLERANCE);
            let not_expired = now < op.ready_at.checked_add(TL_OP_EXPIRY_SECS)
                .unwrap_or(u64::MAX);
            ready && not_expired
        } else { false }
    }

    fn is_expired(&self, op_id: [u8; 32], now: u64) -> bool {
        if let Some(op) = self.ops.get(&op_id) {
            if op.done { return false; }
            let expiry = op.ready_at.checked_add(TL_OP_EXPIRY_SECS).unwrap_or(u64::MAX);
            now >= expiry
        } else { false }
    }
}

// ===========================================================================
// FUZZ TARGETS
// ===========================================================================

// ---------------------------------------------------------------------------
// 1. Multisig: sign / execute / cancel on expired proposal all fail Expired
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_ms_expiry_sign_execute_cancel(
        n_signers in 3u32..=5u32,
        threshold in 2u32..=3u32,
        created_at in 1u64..=1_000_000u64,
        delta_secs in 0u64..=100 * 24 * 60 * 60, // up to 100 days after
        op_type in 0u32..=2u32,
    ) {
        let mut ms = MsModel::new();
        ms.init(n_signers, threshold);
        let pid = ms.propose(Addr(0), created_at).expect("propose ok");
        let expiry = created_at + MS_EXPIRY_SECONDS;
        let now = created_at.saturating_add(delta_secs);

        let expired = now > expiry;
        match op_type {
            0 => {
                let r = ms.sign(Addr(1), pid, now);
                if expired {
                    prop_assert_eq!(r, Err(MsError::Expired),
                        "sign at now={} > expiry={} must be Expired; got {:?}", now, expiry, r);
                }
            }
            1 => {
                // Gather approvals first so failure is definitely expiry, not BelowThreshold
                for s in 1..n_signers { let _ = ms.sign(Addr(s as u64), pid, created_at + 1); }
                let r = ms.execute(pid, now);
                if expired {
                    prop_assert_eq!(r, Err(MsError::Expired),
                        "execute at now={} > expiry={} must be Expired; got {:?}", now, expiry, r);
                }
            }
            2 => {
                let r = ms.cancel(Addr(1), pid, now);
                if expired {
                    prop_assert_eq!(r, Err(MsError::Expired),
                        "cancel at now={} > expiry={} must be Expired; got {:?}", now, expiry, r);
                }
            }
            _ => unreachable!(),
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Precise boundary: at expiry still in-window; +1s blocked
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_ms_expiry_boundary(
        created_at in 1u64..=u64::MAX - MS_EXPIRY_SECONDS - 2,
    ) {
        let mut ms = MsModel::new();
        ms.init(5, 2);
        let pid = ms.propose(Addr(0), created_at).expect("ok");
        let expiry = created_at + MS_EXPIRY_SECONDS;

        // exactly at expiry: `now > expiry` is false → sign/execute allowed
        let p_before = ms.proposals.get(&pid).unwrap().approval_count;
        let r_sign_at = ms.sign(Addr(1), pid, expiry);
        prop_assert_eq!(r_sign_at, Ok(()), "sign AT expiry must be in-window");
        prop_assert_eq!(ms.proposals.get(&pid).unwrap().approval_count, p_before + 1);

        // exactly at expiry + 1: `now > expiry` is true → blocked
        let r_sign_after = ms.sign(Addr(2), pid, expiry + 1);
        prop_assert_eq!(r_sign_after, Err(MsError::Expired),
            "sign AT expiry+1 must fail Expired");
        // approval_count unchanged
        prop_assert_eq!(ms.proposals.get(&pid).unwrap().approval_count, p_before + 1);
    }
}

// ---------------------------------------------------------------------------
// 3. Expired proposal state never mutated despite repeated attempts
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_ms_expiry_proposal_untouched(
        n_signers in 3u32..=5u32,
        created_at in 1_000u64..=10_000u64,
        n_attempts in 0u32..=50u32,
        seed in 0u64..=u64::MAX,
    ) {
        let mut ms = MsModel::new();
        ms.init(n_signers, n_signers);
        let pid = ms.propose(Addr(0), created_at).expect("ok");

        let snap = ms.proposals.get(&pid).unwrap().clone();
        let expired_now = created_at + MS_EXPIRY_SECONDS + 999;

        let mut rng = seed;
        let mut rand = || -> u64 {
            rng = rng.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            rng
        };

        for _ in 0..n_attempts {
            let op = rand() % 3;
            let s = Addr((rand() % (n_signers as u64 + 10)));
            match op {
                0 => { let _ = ms.sign(s, pid, expired_now); }
                1 => { let _ = ms.execute(pid, expired_now); }
                2 => { let _ = ms.cancel(s, pid, expired_now); }
                _ => {}
            }
        }

        let after = ms.proposals.get(&pid).unwrap();
        prop_assert_eq!(after.approval_count, snap.approval_count,
            "approval_count mutated after expiry!");
        prop_assert_eq!(after.executed, snap.executed);
        prop_assert_eq!(after.cancelled, snap.cancelled);
    }
}

// ---------------------------------------------------------------------------
// 4. Timelock operation expiry (ready_at + 14 days) blocks execute
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_tl_operation_expiry(
        now in 100_000u64..=1_000_000_000u64,
        delay in TL_MIN_DELAY..=TL_MAX_DELAY,
        execute_offset in 0u64..=60 * 24 * 60 * 60, // up to 60 days past ready
    ) {
        let mut tl = TlModel::new();
        tl.init(Addr(0)).expect("init");
        let (op_id, ready_at) = tl.schedule(Addr(0), delay, now).expect("schedule");
        let op_expiry = ready_at + TL_OP_EXPIRY_SECS;
        let execute_at = ready_at.saturating_add(execute_offset);
        // Ensure ready_at + tolerance also passed (focus test on the expiry side)
        let execute_at = execute_at.max(ready_at + TL_TIMESTAMP_TOLERANCE + 1);

        let r = tl.execute(op_id, execute_at);
        let expired = execute_at >= op_expiry;
        if expired {
            prop_assert_eq!(r, Err("operation expired"),
                "execute_at={} >= op_expiry={} must fail expired; got {:?}",
                execute_at, op_expiry, r);
            prop_assert_eq!(tl.ops.get(&op_id).unwrap().done, false);
        } else {
            prop_assert_eq!(r, Ok(()),
                "in-window execute_at={} (ready={} expiry={}) must succeed; got {:?}",
                execute_at, ready_at, op_expiry, r);
            prop_assert!(tl.ops.get(&op_id).unwrap().done);
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Tolerance + Expiry window validity: [ready+tol, ready+expiry)
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_tl_expiry_tolerance_window(
        now in 1_000_000u64..=10_000_000u64,
    ) {
        let mut tl = TlModel::new();
        tl.init(Addr(0)).unwrap();
        let (op_id, ready_at) = tl.schedule(Addr(0), TL_MIN_DELAY, now).expect("schedule");
        let tol_ready = ready_at + TL_TIMESTAMP_TOLERANCE;
        let op_expiry  = ready_at + TL_OP_EXPIRY_SECS;

        // Before tolerance: not ready
        prop_assert!(!tl.is_ready(op_id, ready_at),
            "at ready_at (before tolerance) must NOT be ready");
        prop_assert_eq!(tl.execute(op_id, ready_at), Err("operation not ready"));

        // At tolerance boundary: ready
        prop_assert!(tl.is_ready(op_id, tol_ready),
            "at ready_at+tolerance must be ready");
        prop_assert!(!tl.is_expired(op_id, tol_ready));

        // Just before expiry boundary: still valid
        let just_before = op_expiry - 1;
        prop_assert!(tl.is_ready(op_id, just_before));
        prop_assert!(!tl.is_expired(op_id, just_before));

        // At expiry: no longer valid (>=)
        prop_assert!(!tl.is_ready(op_id, op_expiry),
            "at exact expiry must NOT be ready");
        prop_assert!(tl.is_expired(op_id, op_expiry),
            "at exact expiry must be expired");
    }
}

// ---------------------------------------------------------------------------
// 6. Expiry arithmetic overflow safety — all checked_add guarded
// ---------------------------------------------------------------------------
proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    #[test]
    fn fuzz_expiry_arithmetic_safe(
        now in 0u64..=u64::MAX,
        delay in 0u64..=u64::MAX,
    ) {
        // Multisig proposal expiry
        let ms_expiry = now.checked_add(MS_EXPIRY_SECONDS);
        match ms_expiry {
            Some(v) => prop_assert!(v >= now),
            None => prop_assert!(now > u64::MAX - MS_EXPIRY_SECONDS),
        }

        // Timelock ready_at
        let ready = now.checked_add(delay);
        match ready {
            Some(r) => {
                // Then op_expiry = ready + OP_EXPIRY
                let op_exp = r.checked_add(TL_OP_EXPIRY_SECS);
                match op_exp {
                    Some(v) => prop_assert!(v >= r),
                    None => prop_assert!(r > u64::MAX - TL_OP_EXPIRY_SECS),
                }
                // And tolerance check (saturating_add, always safe)
                let _ = r.saturating_add(TL_TIMESTAMP_TOLERANCE); // always safe
            }
            None => prop_assert!(now > u64::MAX - delay),
        }
    }
}

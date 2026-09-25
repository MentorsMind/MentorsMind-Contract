//! Escrow payment-integrity protection primitives (#886).
//!
//! The session-payment escrow can be gamed through strategic dispute
//! initiation, payment-timing manipulation, or coordinated attacks that
//! attempt to drain funds while bypassing legitimate dispute resolution.
//! These helpers give contracts a deterministic, storage-agnostic way to
//! score evidence sufficiency, validate payment timing, gate resolutions
//! behind multi-signature approval, and isolate funds under attack.
//! Contracts own the storage of raw escrow/dispute history; these
//! functions are pure scoring/decision logic over data the caller already
//! has on hand.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum number of evidence items required before a dispute may be
/// resolved (prevents "no-evidence" strategic disputes).
pub const MIN_EVIDENCE_ITEMS: u32 = 1;

/// Minimum time (seconds) that must elapse between a dispute opening and
/// its resolution, giving both parties a fair window to respond and
/// preventing timing-manipulation attacks that rush a favorable ruling.
pub const MIN_DISPUTE_COOLDOWN_SECS: u64 = 24 * 3_600;

/// Number of independent approvals required to release funds from an
/// escrow flagged as high-risk (multi-signature protection).
pub const ESCROW_MULTISIG_THRESHOLD: u32 = 2;

/// Risk score (0-100) at or above which a payment-timing pattern is
/// considered manipulative.
pub const PAYMENT_TIMING_RISK_THRESHOLD: u32 = 60;

/// Repeated dispute/release attempts within this window are treated as a
/// coordinated gaming pattern.
pub const RAPID_ACTION_WINDOW_SECS: u64 = 600;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Result of validating whether submitted evidence is sufficient to
/// support a dispute resolution.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct EvidenceSufficiency {
    pub sufficient: bool,
    pub evidence_count: u32,
    pub cooldown_elapsed: bool,
}

/// Result of checking a payment-release timing pattern for manipulation.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PaymentTimingCheck {
    pub manipulation_suspected: bool,
    pub risk_score: u32,
    pub rapid_action_count: u32,
}

/// Multi-signature approval state for releasing a flagged escrow.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct EscrowMultisigApproval {
    pub approvals: u32,
    pub threshold_met: bool,
}

/// Emergency fund-isolation decision for an escrow under suspected attack.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyFundLock {
    pub isolate: bool,
    pub reason: Symbol,
    pub locked_at: u64,
}

/// A single payment-audit trail entry for an escrow.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PaymentAuditEntry {
    pub escrow_id: u64,
    pub dispute_opened_at: u64,
    pub evidence_count: u32,
    pub resolved_at: u64,
    pub isolated: bool,
}

// ---------------------------------------------------------------------------
// Evidence sufficiency & completion verification
// ---------------------------------------------------------------------------

/// Validate that a dispute has both enough submitted evidence and has
/// respected the minimum cooldown before it may be resolved.
pub fn validate_evidence_sufficiency(
    env: &Env,
    evidence_count: u32,
    dispute_opened_at: u64,
) -> EvidenceSufficiency {
    let now = env.ledger().timestamp();
    let cooldown_elapsed = now.saturating_sub(dispute_opened_at) >= MIN_DISPUTE_COOLDOWN_SECS;
    EvidenceSufficiency {
        sufficient: evidence_count >= MIN_EVIDENCE_ITEMS && cooldown_elapsed,
        evidence_count,
        cooldown_elapsed,
    }
}

/// Verify objective session-completion criteria before allowing funds to
/// release: the session must have actually ended and not be under an
/// active, unresolved dispute.
pub fn verify_completion_criteria(now: u64, session_end_time: u64, is_disputed: bool) -> bool {
    now >= session_end_time && !is_disputed
}

// ---------------------------------------------------------------------------
// Payment timing validation
// ---------------------------------------------------------------------------

/// Detect manipulation of payment release/dispute timing by scoring how
/// many recent actions on this escrow happened in tight succession
/// (a hallmark of coordinated gaming rather than organic dispute activity).
pub fn detect_payment_timing_manipulation(
    action_timestamps: &Vec<u64>,
) -> PaymentTimingCheck {
    let count = action_timestamps.len();
    let mut rapid_action_count = 0u32;
    if count >= 2 {
        for i in 1..count {
            let prev = action_timestamps.get(i - 1).unwrap_or(0);
            let cur = action_timestamps.get(i).unwrap_or(prev);
            if cur.saturating_sub(prev) < RAPID_ACTION_WINDOW_SECS {
                rapid_action_count = rapid_action_count.saturating_add(1);
            }
        }
    }

    let risk_score = rapid_action_count.saturating_mul(25).min(100);
    PaymentTimingCheck {
        manipulation_suspected: risk_score >= PAYMENT_TIMING_RISK_THRESHOLD,
        risk_score,
        rapid_action_count,
    }
}

// ---------------------------------------------------------------------------
// Escrow security (multi-signature)
// ---------------------------------------------------------------------------

/// Check whether a set of distinct signer approvals meets the
/// multi-signature threshold required to release a flagged escrow.
pub fn check_multisig_threshold(env: &Env, distinct_approvers: &Vec<Address>) -> EscrowMultisigApproval {
    let mut seen: Vec<Address> = Vec::new(env);
    for approver in distinct_approvers.iter() {
        if !seen.contains(approver.clone()) {
            seen.push_back(approver);
        }
    }
    let approvals = seen.len();
    EscrowMultisigApproval {
        approvals,
        threshold_met: approvals >= ESCROW_MULTISIG_THRESHOLD,
    }
}

// ---------------------------------------------------------------------------
// Emergency fund protection
// ---------------------------------------------------------------------------

/// Decide whether an escrow should be automatically isolated given
/// combined timing-manipulation and evidence-sufficiency signals.
pub fn compute_emergency_isolation(
    env: &Env,
    timing: PaymentTimingCheck,
    evidence: EvidenceSufficiency,
    reason: Symbol,
) -> EmergencyFundLock {
    let isolate = timing.manipulation_suspected && !evidence.sufficient;
    EmergencyFundLock {
        isolate,
        reason,
        locked_at: env.ledger().timestamp(),
    }
}

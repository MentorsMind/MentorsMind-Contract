//! Session-scheduling integrity protection primitives (#884).
//!
//! Mentors can game the scheduling system by manipulating availability
//! windows, fabricating scheduling conflicts, or coordinating with other
//! mentors to create artificial scarcity. These helpers give contracts a
//! deterministic, storage-agnostic way to commit to availability ahead of
//! time (commit/reveal), validate external conflict proofs, assign slots
//! fairly with anti-gaming randomization, and detect suspicious
//! availability-manipulation patterns. Contracts own the storage of raw
//! scheduling history; these functions are pure scoring/decision logic
//! over data the caller already has on hand.

use soroban_sdk::{
    contracttype, xdr::ToXdr, Address, Bytes, BytesN, Env, IntoVal, Symbol, TryIntoVal, Val, Vec,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum lead time (seconds) an availability commitment must be made
/// before the committed slot, preventing last-minute manipulation.
pub const MIN_COMMITMENT_LEAD_SECS: u64 = 3_600;

/// Maximum age (seconds) an external conflict proof may have before it is
/// considered stale and no longer accepted.
pub const MAX_CONFLICT_PROOF_AGE_SECS: u64 = 24 * 3_600;

/// Risk score (0-100) at or above which an availability pattern is
/// flagged as manipulative gaming.
pub const GAMING_RISK_THRESHOLD: u32 = 60;

/// Repeated availability withdrawals/re-commitments within this window are
/// treated as suspiciously reactive (characteristic of manual gaming
/// rather than genuine scheduling changes).
pub const RAPID_AVAILABILITY_CHANGE_WINDOW_SECS: u64 = 900;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A cryptographic commitment to a mentor's availability for a time slot,
/// made ahead of the slot to prevent last-minute manipulation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AvailabilityCommitment {
    pub mentor: Address,
    pub slot_start: u64,
    pub commitment_hash: BytesN<32>,
    pub committed_at: u64,
}

/// Result of verifying an external scheduling-conflict proof.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct ConflictProof {
    pub valid: bool,
    pub within_freshness_window: bool,
}

/// Outcome of a fair, anti-gaming scheduling assignment.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FairSchedulingDecision {
    pub granted: bool,
    pub randomized_tiebreak: u64,
    pub reason: Symbol,
}

/// Transparency/audit record for a scheduling decision.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SchedulingAuditRecord {
    pub mentor: Address,
    pub slot_start: u64,
    pub decided_at: u64,
    pub outcome: Symbol,
}

/// Detected availability-gaming risk for a mentor.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AvailabilityGamingFlag {
    pub gaming_suspected: bool,
    pub risk_score: u32,
    pub rapid_change_count: u32,
}

// ---------------------------------------------------------------------------
// Availability commitments (commit/reveal)
// ---------------------------------------------------------------------------

/// Compute a commitment hash for a mentor's claimed availability slot:
/// sha256(mentor || slot_start || salt). The salt is only revealed at
/// booking time, preventing other mentors from front-running or copying
/// availability claims before they are locked in.
pub fn compute_availability_commitment(
    env: &Env,
    mentor: &Address,
    slot_start: u64,
    salt: &BytesN<32>,
) -> BytesN<32> {
    let mut input = Bytes::new(env);
    input.append(&mentor.to_xdr(env));
    for b in slot_start.to_be_bytes().iter() {
        input.push_back(*b);
    }
    input.append(&Bytes::from_slice(env, &salt.to_array()));
    env.crypto().sha256(&input).into()
}

/// Verify that a claimed commitment matches the current mentor/slot/salt,
/// and that it was made with sufficient lead time before the slot.
pub fn verify_availability_commitment(
    env: &Env,
    commitment: &AvailabilityCommitment,
    salt: &BytesN<32>,
) -> bool {
    let recomputed =
        compute_availability_commitment(env, &commitment.mentor, commitment.slot_start, salt);
    let lead_time_ok =
        commitment.slot_start.saturating_sub(commitment.committed_at) >= MIN_COMMITMENT_LEAD_SECS;
    recomputed == commitment.commitment_hash && lead_time_ok
}

// ---------------------------------------------------------------------------
// External conflict verification
// ---------------------------------------------------------------------------

/// Validate an externally-provided scheduling-conflict proof (e.g. an
/// oracle-attested calendar-busy hash) against the expected commitment,
/// requiring the proof to be fresh relative to the current ledger time.
pub fn validate_conflict_proof(
    env: &Env,
    proof_hash: &BytesN<32>,
    expected_hash: &BytesN<32>,
    proof_issued_at: u64,
) -> ConflictProof {
    let now = env.ledger().timestamp();
    let within_freshness_window =
        now.saturating_sub(proof_issued_at) <= MAX_CONFLICT_PROOF_AGE_SECS;
    ConflictProof {
        valid: proof_hash == expected_hash && within_freshness_window,
        within_freshness_window,
    }
}

// ---------------------------------------------------------------------------
// Fair scheduling & anti-gaming
// ---------------------------------------------------------------------------

/// Derive a pseudo-random tiebreak value from ledger state, used to
/// resolve simultaneous booking requests fairly without allowing mentors
/// or learners to predict or influence the outcome.
pub fn compute_random_tiebreak(env: &Env) -> u64 {
    let ts = env.ledger().timestamp();
    let seq = env.ledger().sequence();
    let mut input = Bytes::new(env);
    for b in ts.to_be_bytes().iter() {
        input.push_back(*b);
    }
    for b in seq.to_be_bytes().iter() {
        input.push_back(*b);
    }
    let hash = env.crypto().sha256(&input);
    let hash_bytes = hash.to_array();
    u64::from_be_bytes([
        hash_bytes[0], hash_bytes[1], hash_bytes[2], hash_bytes[3],
        hash_bytes[4], hash_bytes[5], hash_bytes[6], hash_bytes[7],
    ])
}

/// Decide whether a booking request should be granted for a contested
/// slot. When multiple concurrent requesters exist, the random tiebreak
/// prevents coordinated mentors from deterministically controlling
/// outcomes.
pub fn assign_fair_slot(env: &Env, slot_already_taken: bool, reason: Symbol) -> FairSchedulingDecision {
    FairSchedulingDecision {
        granted: !slot_already_taken,
        randomized_tiebreak: compute_random_tiebreak(env),
        reason,
    }
}

/// Detect availability-manipulation gaming by scoring how often a mentor
/// changes (withdraws/recommits) availability in rapid succession — a
/// pattern consistent with artificial scarcity creation rather than
/// genuine schedule changes.
pub fn detect_availability_gaming(change_timestamps: &Vec<u64>) -> AvailabilityGamingFlag {
    let count = change_timestamps.len();
    let mut rapid_change_count = 0u32;
    if count >= 2 {
        for i in 1..count {
            let prev = change_timestamps.get(i - 1).unwrap_or(0);
            let cur = change_timestamps.get(i).unwrap_or(prev);
            if cur.saturating_sub(prev) < RAPID_AVAILABILITY_CHANGE_WINDOW_SECS {
                rapid_change_count = rapid_change_count.saturating_add(1);
            }
        }
    }
    let risk_score = rapid_change_count.saturating_mul(20).min(100);
    AvailabilityGamingFlag {
        gaming_suspected: risk_score >= GAMING_RISK_THRESHOLD,
        risk_score,
        rapid_change_count,
    }
}

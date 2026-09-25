//! Shared governance voting primitives.

use soroban_sdk::{contracttype, Address, Bytes, BytesN, Env, IntoVal, Symbol, TryIntoVal, Val};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum holding period before tokens are eligible for voting (7 days).
pub const MIN_HOLDING_PERIOD_SECS: u64 = 7 * 24 * 60 * 60;

/// Maximum random extension added to voting deadlines (24 hours).
pub const MAX_RANDOM_EXTENSION_SECS: u64 = 24 * 60 * 60;

/// Fraction of the voting period allocated to the commit phase (50%).
pub const COMMIT_PHASE_BPS: u32 = 5_000;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A vote commitment stored during the commit phase.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VoteCommitment {
    pub proposal_id: u32,
    pub voter: Address,
    pub commitment_hash: BytesN<32>,
    pub committed_at: u64,
}

/// A revealed vote stored during the reveal phase.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevealedVote {
    pub proposal_id: u32,
    pub voter: Address,
    pub support: bool,
    pub voting_weight: i128,
    pub revealed_at: u64,
}

/// The current phase of a proposal's voting lifecycle.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VotePhase {
    /// Commit phase: voters submit commitment hashes.
    Commit = 1,
    /// Reveal phase: voters reveal their actual votes.
    Reveal = 2,
    /// Voting has ended.
    Ended = 3,
}

/// A flag raised when vote manipulation is detected.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManipulationFlag {
    pub proposal_id: u32,
    pub voter: Address,
    pub reason: Symbol,
    pub detected_at: u64,
    pub stake_snapshot: i128,
    pub vote_weight: i128,
}

// ---------------------------------------------------------------------------
// Commitment hash computation
// ---------------------------------------------------------------------------

/// Compute a commitment hash: sha256(proposal_id || voter || support || salt).
pub fn compute_commitment_hash(
    env: &Env,
    proposal_id: u32,
    voter: &Address,
    support: bool,
    salt: &BytesN<32>,
) -> BytesN<32> {
    let mut input = Bytes::new(env);
    input.push_back((proposal_id >> 24) as u8);
    input.push_back((proposal_id >> 16) as u8);
    input.push_back((proposal_id >> 8) as u8);
    input.push_back(proposal_id as u8);
    let voter_val: Val = voter.clone().into_val(env);
    let mut voter_bytes: Bytes = voter_val.try_into_val(env).unwrap();
    input.append(&mut voter_bytes);
    input.push_back(if support { 1 } else { 0 });
    let salt_val: Val = salt.clone().into_val(env);
    let mut salt_bytes: Bytes = salt_val.try_into_val(env).unwrap();
    input.append(&mut salt_bytes);
    env.crypto().sha256(&input).into()
}

// ---------------------------------------------------------------------------
// Voting weight calculation (snapshot-based, no time multiplier)
// ---------------------------------------------------------------------------

/// Calculate voting weight from snapshot and delegated power.
///
/// This is the fixed, snapshot-based calculation. Unlike the previous
/// time-weighted system, this returns the raw stake amount plus delegated
/// power without applying any time-based multiplier.
pub fn calculate_voting_weight(snapshot_weight: i128, delegated_power: i128) -> i128 {
    snapshot_weight
        .checked_add(delegated_power)
        .expect("voting weight overflow")
}

// ---------------------------------------------------------------------------
// Vote phase determination
// ---------------------------------------------------------------------------

/// Determine the current voting phase based on timestamps.
pub fn get_vote_phase(
    env: &Env,
    _created_at: u64,
    commit_phase_ends_at: u64,
    voting_ends_at: u64,
) -> VotePhase {
    let now = env.ledger().timestamp();
    if now >= voting_ends_at {
        VotePhase::Ended
    } else if now < commit_phase_ends_at {
        VotePhase::Commit
    } else {
        VotePhase::Reveal
    }
}

// ---------------------------------------------------------------------------
// Random deadline extension (MEV protection)
// ---------------------------------------------------------------------------

/// Derive a pseudo-random extension from the ledger state.
///
/// Uses SHA-256 over the current timestamp and ledger sequence to produce
/// a uniform random value in `[0, max_extension_secs]`.
pub fn compute_random_deadline_extension(env: &Env, max_extension_secs: u64) -> u64 {
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
    let upper = u64::from_be_bytes([
        hash_bytes[0], hash_bytes[1], hash_bytes[2], hash_bytes[3],
        hash_bytes[4], hash_bytes[5], hash_bytes[6], hash_bytes[7],
    ]);
    if max_extension_secs == 0 {
        0
    } else {
        upper % (max_extension_secs + 1)
    }
}

// ---------------------------------------------------------------------------
// Minimum holding period validation
// ---------------------------------------------------------------------------

/// Validate that the voter has held tokens for at least the minimum period.
///
/// Returns `Ok(())` if `proposal_created_at - staked_at >= MIN_HOLDING_PERIOD_SECS`.
pub fn validate_minimum_holding_period(
    staked_at: u64,
    proposal_created_at: u64,
) -> Result<(), &'static str> {
    if proposal_created_at < staked_at.saturating_add(MIN_HOLDING_PERIOD_SECS) {
        return Err("minimum holding period not met");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Manipulation detection
// ---------------------------------------------------------------------------

/// Detect potential vote manipulation based on stake patterns.
///
/// Flags voters who staked very recently relative to the proposal creation
/// time, as this pattern is characteristic of MEV-based vote manipulation.
pub fn detect_vote_manipulation(
    env: &Env,
    staked_at: u64,
    proposal_created_at: u64,
    snapshot_amount: i128,
    voter: &Address,
) -> Option<ManipulationFlag> {
    let holding_duration = proposal_created_at.saturating_sub(staked_at);
    let suspicious_threshold = 24 * 60 * 60; // 24 hours

    if holding_duration < suspicious_threshold && snapshot_amount > 0 {
        let reason = Symbol::new(env, "recent_stake_acquired");
        Some(ManipulationFlag {
            proposal_id: 0,
            voter: voter.clone(),
            reason,
            detected_at: proposal_created_at,
            stake_snapshot: snapshot_amount,
            vote_weight: snapshot_amount,
        })
    } else {
        None
    }
}

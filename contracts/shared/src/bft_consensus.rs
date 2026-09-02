#![no_std]

use soroban_sdk::{contracterror, contracttype, BytesN, Env, Symbol, Vec};

use crate::vrf::{EpochInfo, VrfOutput};

// ---------------------------------------------------------------------------
// Practical Byzantine Fault Tolerant (pBFT) Consensus
// ---------------------------------------------------------------------------
// Implements a simplified pBFT consensus mechanism for validator agreement.
// The protocol tolerates up to f = floor(n/3) malicious validators.

/// Consensus round phases.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ConsensusPhase {
    /// Idle state, waiting for new proposal
    Idle,
    /// A block/proposal has been proposed
    PrePrepare,
    /// Validators have sent prepare messages
    Prepare,
    /// Validators have sent commit messages
    Commit,
    /// Consensus reached, block finalized
    Finalized,
    /// Consensus failed (view change needed)
    Failed,
}

/// A single consensus round.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusRound {
    /// Round number (monotonically increasing)
    pub round: u64,
    /// Current phase of this round
    pub phase: ConsensusPhase,
    /// Block/proposal hash being proposed
    pub block_hash: BytesN<32>,
    /// Address of the proposer for this round
    pub proposer: soroban_sdk::Address,
    /// Epoch this round belongs to
    pub epoch: u64,
    /// Timestamp when round started
    pub started_at: u64,
    /// Number of prepare messages received
    pub prepare_count: u32,
    /// Number of commit messages received
    pub commit_count: u32,
    /// Quorum threshold for this round
    pub quorum_threshold: u32,
    /// Whether view change was requested
    pub view_change_requested: bool,
}

/// Quorum certificate: proof that 2f+1 validators agreed.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct QuorumCertificate {
    /// The block hash that was certified
    pub block_hash: BytesN<32>,
    /// Round number
    pub round: u64,
    /// Addresses of validators who signed
    pub signers: Vec<soroban_sdk::Address>,
    /// Number of signatures (must >= quorum)
    pub signature_count: u32,
    /// Quorum threshold that was met
    pub quorum_threshold: u32,
    /// Timestamp of certificate creation
    pub created_at: u64,
}

/// View change message for leader rotation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ViewChange {
    /// New view/round number
    pub new_view: u64,
    /// Address of the validator requesting view change
    pub validator: soroban_sdk::Address,
    /// Last finalized block hash known to this validator
    pub last_block_hash: BytesN<32>,
    /// Timestamp of view change request
    pub requested_at: u64,
    /// Reason for view change
    pub reason: ViewChangeReason,
}

/// Reasons for view change.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViewChangeReason {
    /// Leader failed to propose within timeout
    LeaderTimeout,
    /// Leader proposed invalid block
    InvalidProposal,
    /// Network partition detected
    NetworkPartition,
    /// Byzantine behavior detected
    ByzantineBehavior,
}

/// Validator status within a consensus round.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ValidatorVote {
    /// Has not voted in this round
    None,
    /// Voted prepare
    Prepare,
    /// Voted commit
    Commit,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum ConsensusError {
    /// Not a registered validator
    NotValidator = 1,
    /// Round already finalized
    AlreadyFinalized = 2,
    /// Invalid phase transition
    InvalidPhase = 3,
    /// Quorum not met
    QuorumNotMet = 4,
    /// Double voting detected
    DoubleVote = 5,
    /// Block hash mismatch
    HashMismatch = 6,
    /// Round expired
    RoundExpired = 7,
    /// Insufficient signers for certificate
    InsufficientSigners = 8,
    /// Proposer not selected for this round
    InvalidProposer = 9,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum duration of a consensus round before view change (30 seconds)
pub const ROUND_TIMEOUT_SECS: u64 = 30;

/// Maximum consecutive timeouts before validator ejection
pub const MAX_TIMEOUTS_BEFORE_EJECT: u32 = 3;

/// View change quorum: f+1 validators must agree to trigger view change
pub fn view_change_quorum(validator_count: u32) -> u32 {
    let f = validator_count / 3;
    f + 1
}

// ---------------------------------------------------------------------------
// Consensus Functions
// ---------------------------------------------------------------------------

/// Create a new consensus round.
pub fn start_round(
    env: &Env,
    round: u64,
    block_hash: BytesN<32>,
    proposer: soroban_sdk::Address,
    epoch: u64,
    quorum_threshold: u32,
) -> ConsensusRound {
    ConsensusRound {
        round,
        phase: ConsensusPhase::PrePrepare,
        block_hash,
        proposer,
        epoch,
        started_at: env.ledger().timestamp(),
        prepare_count: 0,
        commit_count: 0,
        quorum_threshold,
        view_change_requested: false,
    }
}

/// Cast a prepare vote in a consensus round.
/// Returns the updated round, or error if vote is invalid.
pub fn cast_prepare(
    env: &Env,
    round: &mut ConsensusRound,
    voter: &soroban_sdk::Address,
    block_hash: &BytesN<32>,
) -> Result<(), ConsensusError> {
    if round.phase != ConsensusPhase::PrePrepare {
        return Err(ConsensusError::InvalidPhase);
    }
    if &round.block_hash != block_hash {
        return Err(ConsensusError::HashMismatch);
    }
    if is_round_expired(env, round) {
        return Err(ConsensusError::RoundExpired);
    }

    round.prepare_count = round.prepare_count.saturating_add(1);

    // Transition to Prepare phase if we have enough prepares
    if round.prepare_count >= round.quorum_threshold {
        round.phase = ConsensusPhase::Prepare;
    }

    Ok(())
}

/// Cast a commit vote in a consensus round.
/// Returns the updated round, or error if vote is invalid.
pub fn cast_commit(
    env: &Env,
    round: &mut ConsensusRound,
    voter: &soroban_sdk::Address,
    block_hash: &BytesN<32>,
) -> Result<(), ConsensusError> {
    if round.phase != ConsensusPhase::Prepare {
        return Err(ConsensusError::InvalidPhase);
    }
    if &round.block_hash != block_hash {
        return Err(ConsensusError::HashMismatch);
    }
    if is_round_expired(env, round) {
        return Err(ConsensusError::RoundExpired);
    }

    round.commit_count = round.commit_count.saturating_add(1);

    // Transition to Commit phase if we have enough commits
    if round.commit_count >= round.quorum_threshold {
        round.phase = ConsensusPhase::Commit;
    }

    Ok(())
}

/// Finalize a consensus round and create a quorum certificate.
/// Must be called after the round reaches Commit phase with sufficient commits.
pub fn finalize_round(
    env: &Env,
    round: &mut ConsensusRound,
    signers: Vec<soroban_sdk::Address>,
) -> Result<QuorumCertificate, ConsensusError> {
    if round.phase != ConsensusPhase::Commit {
        return Err(ConsensusError::InvalidPhase);
    }
    if round.commit_count < round.quorum_threshold {
        return Err(ConsensusError::QuorumNotMet);
    }

    round.phase = ConsensusPhase::Finalized;

    Ok(QuorumCertificate {
        block_hash: round.block_hash.clone(),
        round: round.round,
        signers,
        signature_count: round.commit_count,
        quorum_threshold: round.quorum_threshold,
        created_at: env.ledger().timestamp(),
    })
}

/// Mark a round as failed and trigger view change eligibility.
pub fn fail_round(round: &mut ConsensusRound) {
    round.phase = ConsensusPhase::Failed;
    round.view_change_requested = true;
}

/// Check if a round has timed out.
pub fn is_round_expired(env: &Env, round: &ConsensusRound) -> bool {
    let now = env.ledger().timestamp();
    now >= round.started_at.saturating_add(ROUND_TIMEOUT_SECS)
}

// ---------------------------------------------------------------------------
// Quorum Certificate Functions
// ---------------------------------------------------------------------------

/// Verify that a quorum certificate is valid.
pub fn verify_quorum_certificate(
    env: &Env,
    qc: &QuorumCertificate,
    expected_block_hash: &BytesN<32>,
    expected_round: u64,
) -> bool {
    if &qc.block_hash != expected_block_hash {
        return false;
    }
    if qc.round != expected_round {
        return false;
    }
    if qc.signature_count < qc.quorum_threshold {
        return false;
    }
    // Check no duplicate signers
    let signer_count = qc.signers.len();
    let unique_count = count_unique_signers(&qc.signers);
    signer_count == unique_count
}

/// Count unique signers in a certificate.
fn count_unique_signers(signers: &Vec<soroban_sdk::Address>) -> u32 {
    let mut count = 0;
    for i in 0..signers.len() {
        let signer = signers.get(i).unwrap();
        let mut is_unique = true;
        for j in 0..i {
            if signers.get(j).unwrap() == signer {
                is_unique = false;
                break;
            }
        }
        if is_unique {
            count += 1;
        }
    }
    count
}

// ---------------------------------------------------------------------------
// View Change Functions
// ---------------------------------------------------------------------------

/// Create a view change request.
pub fn create_view_change(
    env: &Env,
    new_view: u64,
    validator: soroban_sdk::Address,
    last_block_hash: BytesN<32>,
    reason: ViewChangeReason,
) -> ViewChange {
    ViewChange {
        new_view,
        validator,
        last_block_hash,
        requested_at: env.ledger().timestamp(),
        reason,
    }
}

/// Check if enough view changes have been received to trigger leader rotation.
pub fn has_view_change_quorum(view_change_count: u32, validator_count: u32) -> bool {
    view_change_count >= view_change_quorum(validator_count)
}

// ---------------------------------------------------------------------------
// Proposer Selection
// ---------------------------------------------------------------------------

/// Select the proposer for a given round using round-robin on the
/// VRF-sorted validator set. This ensures predictable, fair rotation
/// while preventing a single validator from dominating block proposal.
///
/// The proposer is selected as: validators[round % validator_count]
/// where validators are sorted by VRF output (already sorted from
/// `select_validators_for_epoch`).
pub fn select_proposer(
    validators: &Vec<soroban_sdk::Address>,
    round: u64,
) -> Option<soroban_sdk::Address> {
    if validators.is_empty() {
        return None;
    }
    let idx = (round % (validators.len() as u64)) as u32;
    validators.get(idx)
}

/// Verify that a given address is the expected proposer for a round.
pub fn is_valid_proposer(
    validators: &Vec<soroban_sdk::Address>,
    round: u64,
    proposer: &soroban_sdk::Address,
) -> bool {
    match select_proposer(validators, round) {
        Some(expected) => expected == *proposer,
        None => false,
    }
}

// ---------------------------------------------------------------------------
// Consensus State Management
// ---------------------------------------------------------------------------

/// Full consensus state for the protocol.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusState {
    /// Current epoch
    pub current_epoch: u64,
    /// Current round within the epoch
    pub current_round: u64,
    /// Current consensus phase
    pub current_phase: ConsensusPhase,
    /// Active consensus round (if any)
    pub active_round: Option<ConsensusRound>,
    /// Last finalized quorum certificate
    pub last_qc: Option<QuorumCertificate>,
    /// Number of consecutive timeouts (for ejection logic)
    pub consecutive_timeouts: u32,
    /// Whether emergency consensus is active
    pub emergency_mode: bool,
}

/// Initialize a new consensus state for an epoch.
pub fn init_consensus_state(epoch: u64) -> ConsensusState {
    ConsensusState {
        current_epoch: epoch,
        current_round: 0,
        current_phase: ConsensusPhase::Idle,
        active_round: None,
        last_qc: None,
        consecutive_timeouts: 0,
        emergency_mode: false,
    }
}

/// Start a new round in the consensus state.
pub fn start_new_round(
    state: &mut ConsensusState,
    env: &Env,
    block_hash: BytesN<32>,
    proposer: soroban_sdk::Address,
    quorum_threshold: u32,
) {
    state.current_round = state.current_round.saturating_add(1);
    state.current_phase = ConsensusPhase::PrePrepare;
    state.active_round = Some(start_round(
        env,
        state.current_round,
        block_hash,
        proposer,
        state.current_epoch,
        quorum_threshold,
    ));
}

/// Handle round timeout: increment timeout counter and check for ejection.
pub fn handle_round_timeout(state: &mut ConsensusState) -> bool {
    state.consecutive_timeouts = state.consecutive_timeouts.saturating_add(1);
    state.current_phase = ConsensusPhase::Failed;

    if state.consecutive_timeouts >= MAX_TIMEOUTS_BEFORE_EJECT {
        // Proposer should be ejected
        true
    } else {
        false
    }
}

/// Reset timeout counter after successful consensus.
pub fn reset_timeouts(state: &mut ConsensusState) {
    state.consecutive_timeouts = 0;
}

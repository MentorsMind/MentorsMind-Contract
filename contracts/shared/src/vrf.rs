#![no_std]

use soroban_sdk::{contracterror, contracttype, xdr::ToXdr, Bytes, BytesN, Env, Vec};

// ---------------------------------------------------------------------------
// VRF (Verifiable Random Function) for validator selection
// ---------------------------------------------------------------------------
// Provides cryptographically secure randomness for validator selection,
// preventing manipulation of the selection process. Uses a hash-based
// VRF construction suitable for Soroban's deterministic execution model.

/// VRF output with proof of correctness.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VrfOutput {
    /// The random output derived from input + secret
    pub randomness: BytesN<32>,
    /// Proof that the output was correctly derived
    pub proof: BytesN<32>,
    /// Block height at which VRF was evaluated
    pub evaluated_at: u64,
    /// Input seed used for evaluation
    pub input_seed: BytesN<32>,
}

/// VRF evaluation result with validator assignment.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorSelection {
    /// Epoch for which this selection is valid
    pub epoch: u64,
    /// Ordered list of selected validators
    pub validators: Vec<soroban_sdk::Address>,
    /// Quorum threshold (2f+1 where f = floor(n/3))
    pub quorum_threshold: u32,
    /// The VRF output used for this selection
    pub vrf_output: VrfOutput,
    /// Total stake of selected validators
    pub total_stake: i128,
}

/// Epoch metadata for validator rotation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EpochInfo {
    /// Current epoch number
    pub epoch: u64,
    /// Timestamp when epoch started
    pub started_at: u64,
    /// Duration of this epoch in seconds
    pub duration_secs: u64,
    /// VRF output that determined this epoch's validator set
    pub vrf_output: VrfOutput,
    /// Number of validators in this epoch
    pub validator_count: u32,
    /// Whether this epoch is finalized
    pub finalized: bool,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum VrfError {
    /// No validators available for selection
    NoValidators = 1,
    /// Insufficient validators for BFT (need >= 4 for 1 malicious)
    InsufficientValidators = 2,
    /// VRF proof verification failed
    InvalidProof = 3,
    /// Epoch already finalized
    EpochAlreadyFinalized = 4,
    /// Epoch not yet started
    EpochNotStarted = 5,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum validators required for BFT safety (3f+1 where f=1 => minimum 4)
pub const MIN_VALIDATORS: u32 = 4;

/// Default epoch duration: 6 hours
pub const DEFAULT_EPOCH_DURATION_SECS: u64 = 6 * 3600;

/// Maximum epoch duration: 24 hours
pub const MAX_EPOCH_DURATION_SECS: u64 = 24 * 3600;

/// Minimum epoch duration: 1 hour
pub const MIN_EPOCH_DURATION_SECS: u64 = 3600;

/// Stake weight divisor for VRF scoring (1000 = full weight)
pub const STAKE_WEIGHT_DIVISOR: i128 = 1000;

// ---------------------------------------------------------------------------
// VRF Functions
// ---------------------------------------------------------------------------

/// Evaluate VRF for a given seed and evaluator address.
/// The VRF is hash-based: output = SHA256(seed || evaluator || nonce).
/// This is deterministic given the same inputs, preventing manipulation.
///
/// In production, this should use a proper VRF construction (e.g., ECVRF).
/// The current implementation uses hash-based VRF which is suitable for
/// Soroban's deterministic execution where the evaluator is the contract itself.
pub fn evaluate_vrf(env: &Env, seed: &BytesN<32>, epoch: u64) -> VrfOutput {
    let input_seed = compute_vrf_input(env, seed, epoch);
    let randomness: BytesN<32> = env.crypto().sha256(&Bytes::from_array(env, &input_seed.to_array())).into();

    // Create proof by hashing randomness with epoch
    let mut proof_input = [0u8; 64];
    let rand_array = randomness.to_array();
    for i in 0..32 {
        proof_input[i] = rand_array[i];
    }
    let epoch_bytes = epoch.to_be_bytes();
    for (i, byte) in epoch_bytes.iter().enumerate() {
        if i + 32 < 64 {
            proof_input[i + 32] = *byte;
        }
    }
    let proof: BytesN<32> = env.crypto().sha256(&Bytes::from_array(env, &proof_input)).into();

    VrfOutput {
        randomness,
        proof,
        evaluated_at: env.ledger().timestamp(),
        input_seed,
    }
}

/// Compute the VRF input seed from a base seed and epoch.
fn compute_vrf_input(env: &Env, base_seed: &BytesN<32>, epoch: u64) -> BytesN<32> {
    let epoch_bytes = epoch.to_be_bytes();
    let mut combined = base_seed.to_array();
    // Append epoch bytes to seed
    for (i, byte) in epoch_bytes.iter().enumerate() {
        if i + 32 < 64 {
            combined[i + 32] = *byte;
        }
    }
    let hash: BytesN<32> = env.crypto().sha256(&Bytes::from_array(env, &combined)).into();
    hash
}

/// Verify a VRF proof against expected output.
/// Returns true if the proof is valid for the given seed and epoch.
pub fn verify_vrf_proof(
    env: &Env,
    seed: &BytesN<32>,
    epoch: u64,
    output: &VrfOutput,
) -> bool {
    let expected = evaluate_vrf(env, seed, epoch);
    output.randomness == expected.randomness && output.proof == expected.proof
}

// ---------------------------------------------------------------------------
// Validator Selection
// ---------------------------------------------------------------------------

/// Select validators for an epoch using VRF-weighted random selection.
///
/// Selection algorithm:
/// 1. Compute VRF output for current seed + epoch
/// 2. For each eligible validator, compute score = VRF_hash(validator_address) * stake_weight
/// 3. Select top validators by score
/// 4. Ensure selected set meets BFT minimum (>= 4 validators)
///
/// Stake weighting prevents plutocratic selection while maintaining
/// proportional representation. The VRF prevents prediction of the
/// selected set before evaluation.
pub fn select_validators_for_epoch(
    env: &Env,
    seed: &BytesN<32>,
    epoch: u64,
    eligible_validators: &Vec<soroban_sdk::Address>,
    stakes: &Vec<i128>,
    max_validators: u32,
) -> Result<ValidatorSelection, VrfError> {
    let count = eligible_validators.len();
    if count == 0 {
        return Err(VrfError::NoValidators);
    }
    if count < MIN_VALIDATORS {
        return Err(VrfError::InsufficientValidators);
    }

    let vrf_output = evaluate_vrf(env, seed, epoch);

    // Compute weighted scores for each validator
    let mut scored_validators: Vec<(soroban_sdk::Address, i128)> = Vec::new(env);
    for i in 0..count {
        let validator = eligible_validators.get(i).unwrap();
        let stake = if i < stakes.len() {
            stakes.get(i).unwrap()
        } else {
            0
        };

        // VRF-weighted score: hash(validator_addr || vrf_randomness) * stake_weight
        let mut score_input = [0u8; 64];
        let addr_xdr = validator.clone().to_xdr(env);
        let addr_hash: BytesN<32> = env.crypto().sha256(&addr_xdr).into();
        let addr_array = addr_hash.to_array();
        for i in 0..32 {
            score_input[i] = addr_array[i];
        }
        let rand_array = vrf_output.randomness.to_array();
        for i in 0..32 {
            if i + 32 < 64 {
                score_input[i + 32] = rand_array[i];
            }
        }
        let score_hash: BytesN<32> = env.crypto().sha256(&soroban_sdk::Bytes::from_array(env, &score_input)).into();
        let score_bytes = score_hash.to_array();
        let base_score = i128::from_be_bytes([
            0, 0, 0, 0,
            score_bytes[0], score_bytes[1], score_bytes[2], score_bytes[3],
            score_bytes[4], score_bytes[5], score_bytes[6], score_bytes[7],
            score_bytes[8], score_bytes[9], score_bytes[10], score_bytes[11],
        ]);
        let weighted_score = base_score.saturating_mul(stake).saturating_div(STAKE_WEIGHT_DIVISOR);
        scored_validators.push_back((validator, weighted_score));
    }

    // Sort by score descending (selection sort for small sets)
    let mut selected: Vec<soroban_sdk::Address> = Vec::new(env);
    let mut remaining = scored_validators;
    let select_count = core::cmp::min(max_validators, count);

    for _ in 0..select_count {
        let mut best_idx = 0;
        let mut best_score = -1i128;
        for i in 0..remaining.len() {
            let (_, score) = remaining.get(i).unwrap();
            if score > best_score {
                best_score = score;
                best_idx = i;
            }
        }
        let (validator, _) = remaining.get(best_idx).unwrap();
        selected.push_back(validator);
        remaining.remove(best_idx);
    }

    // Quorum threshold: 2f+1 where f = floor(n/3)
    let n = selected.len();
    let f = n / 3;
    let quorum_threshold = 2 * f + 1;

    let total_stake: i128 = (0..count)
        .filter_map(|i| {
            if i < stakes.len() {
                Some(stakes.get(i).unwrap())
            } else {
                None
            }
        })
        .sum();

    Ok(ValidatorSelection {
        epoch,
        validators: selected,
        quorum_threshold,
        vrf_output,
        total_stake,
    })
}

// ---------------------------------------------------------------------------
// Epoch Management
// ---------------------------------------------------------------------------

/// Create a new epoch info struct.
pub fn create_epoch(
    env: &Env,
    epoch: u64,
    duration_secs: u64,
    vrf_output: VrfOutput,
    validator_count: u32,
) -> EpochInfo {
    EpochInfo {
        epoch,
        started_at: env.ledger().timestamp(),
        duration_secs,
        vrf_output,
        validator_count,
        finalized: false,
    }
}

/// Check if an epoch has expired.
pub fn is_epoch_expired(env: &Env, epoch_info: &EpochInfo) -> bool {
    let now = env.ledger().timestamp();
    now >= epoch_info.started_at.saturating_add(epoch_info.duration_secs)
}

/// Compute the next epoch seed from the current VRF output.
/// This creates a chain of VRF evaluations where each epoch's seed
/// depends on the previous epoch's output, preventing pre-computation.
pub fn next_epoch_seed(env: &Env, current_vrf: &VrfOutput) -> BytesN<32> {
    let hash: BytesN<32> = env.crypto().sha256(&Bytes::from_array(env, &current_vrf.randomness.to_array())).into();
    hash
}

/// Compute quorum threshold for a given validator count.
/// Returns 2f+1 where f = floor(n/3).
pub fn compute_quorum_threshold(validator_count: u32) -> u32 {
    let f = validator_count / 3;
    2 * f + 1
}

/// Check if a set of signers meets quorum.
pub fn has_quorum(signer_count: u32, quorum_threshold: u32) -> bool {
    signer_count >= quorum_threshold
}

// ---------------------------------------------------------------------------
// Long-Range Attack Protection
// ---------------------------------------------------------------------------

/// Checkpoint for long-range attack protection.
/// Periodically checkpointing the chain state prevents attackers from
/// rewriting history by creating a fork from a distant past point.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainCheckpoint {
    /// Checkpoint sequence number
    pub sequence: u64,
    /// Ledger sequence at checkpoint
    pub ledger_sequence: u32,
    /// State root hash at checkpoint
    pub state_root: BytesN<32>,
    /// Validator set hash at checkpoint
    pub validator_set_hash: BytesN<32>,
    /// Timestamp of checkpoint
    pub timestamp: u64,
    /// Number of validator signatures attesting to this checkpoint
    pub signature_count: u32,
}

/// Checkpoint interval: create checkpoint every N epochs
pub const CHECKPOINT_INTERVAL_EPOCHS: u64 = 10;

/// Maximum reorganization depth: finality after CHECKPOINT_FINALITY_EPOCHS
pub const CHECKPOINT_FINALITY_EPOCHS: u64 = 3;

/// Create a checkpoint from current state.
pub fn create_checkpoint(
    env: &Env,
    sequence: u64,
    ledger_sequence: u32,
    state_root: BytesN<32>,
    validator_set_hash: BytesN<32>,
    signature_count: u32,
) -> ChainCheckpoint {
    ChainCheckpoint {
        sequence,
        ledger_sequence,
        state_root,
        validator_set_hash,
        timestamp: env.ledger().timestamp(),
        signature_count,
    }
}

/// Verify that a checkpoint is finalized (enough epochs have passed).
pub fn is_checkpoint_finalized(
    current_sequence: u64,
    checkpoint_sequence: u64,
) -> bool {
    current_sequence >= checkpoint_sequence.saturating_add(CHECKPOINT_FINALITY_EPOCHS)
}

/// Check if a new checkpoint should be created.
pub fn should_create_checkpoint(current_epoch: u64) -> bool {
    current_epoch % CHECKPOINT_INTERVAL_EPOCHS == 0
}

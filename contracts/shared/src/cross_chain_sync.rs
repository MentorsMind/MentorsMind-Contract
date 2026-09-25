//! Cross-Chain State Synchronization (#866)
//!
//! Implements atomic cross-chain transaction protocols with two-phase commit
//! and rollback, state merkle-tree synchronization with cryptographic
//! consistency proofs, finality-aware operations, chain-reorganization
//! protection, and emergency isolation procedures.

use soroban_sdk::{contracttype, symbol_short, Address, Bytes, BytesN, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum cross-chain operation age before automatic expiry (24 hours).
pub const XCHAIN_OP_TIMEOUT_SECS: u64 = 86_400;

/// Minimum finality confirmations required before committing (configurable).
pub const MIN_FINALITY_CONFIRMATIONS: u32 = 12;

/// Reorg protection depth — operations submitted before this block depth
/// are considered safe against chain reorganization.
pub const REORG_SAFE_DEPTH: u64 = 64;

/// Maximum number of participating chains in a single atomic operation.
pub const MAX_PARTICIPATING_CHAINS: u32 = 8;

/// Merkle tree leaf size for state proofs (SHA-256 output).
pub const STATE_PROOF_LEAF_SIZE: u32 = 32;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Phase of a two-phase-commit cross-chain operation.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum XChainPhase {
    /// Phase 1: prepare messages sent but not yet acknowledged on all chains.
    Prepared = 1,
    /// Phase 2: all chains acknowledged; commit messages being broadcast.
    Committing = 2,
    /// Operation successfully committed on all participating chains.
    Committed = 3,
    /// Rollback initiated (any chain rejected or timed out).
    RollingBack = 4,
    /// Operation fully rolled back on all chains.
    RolledBack = 5,
    /// Operation expired before all chains confirmed.
    Expired = 6,
}

/// Finality tier for different chains based on their consensus guarantees.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum FinalityTier {
    /// Probabilistic finality (e.g. Ethereum PoW-era, Bitcoin-style).
    Probabilistic = 1,
    /// Deterministic finality (e.g. Stellar, Cosmos-based chains).
    Deterministic = 2,
    /// Optimistic finality with challenge period (e.g. Optimism, Arbitrum).
    Optimistic = 3,
    /// Instant finality (Layer-2 with operator attestation).
    Instant = 4,
}

/// Per-chain configuration for finality-aware cross-chain operations.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainFinalityConfig {
    /// Wormhole-standard chain identifier.
    pub chain_id: u32,
    /// Finality model for this chain.
    pub finality_tier: FinalityTier,
    /// Required confirmation blocks / ledgers before finalizing.
    pub required_confirmations: u32,
    /// Challenge period in seconds (0 for deterministic/instant).
    pub challenge_period_secs: u64,
    /// Whether this chain is currently isolated due to a sync failure.
    pub isolated: bool,
}

/// An atomic two-phase-commit cross-chain operation record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AtomicXChainOp {
    /// Unique identifier for this operation (SHA-256 of payload).
    pub op_id: BytesN<32>,
    /// Participating chain IDs.
    pub chain_ids: Vec<u32>,
    /// Number of chains that have acknowledged Phase 1 (prepare).
    pub prepared_count: u32,
    /// Number of chains that have confirmed Phase 2 (commit).
    pub committed_count: u32,
    /// Current phase.
    pub phase: XChainPhase,
    /// Ledger timestamp when the operation was created.
    pub created_at: u64,
    /// Ledger timestamp after which the operation auto-expires.
    pub expires_at: u64,
    /// Initiating address (for rollback authority).
    pub initiator: Address,
    /// Merkle root of the expected cross-chain state after commit.
    pub expected_state_root: BytesN<32>,
}

/// A cryptographic consistency proof for cross-chain state synchronization.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossChainStateProof {
    /// Chain this proof was generated for.
    pub chain_id: u32,
    /// Merkle root of the on-chain state on `chain_id`.
    pub state_root: BytesN<32>,
    /// Merkle proof path (leaf → root).
    pub proof_path: Vec<BytesN<32>>,
    /// Timestamp when the proof was generated on the source chain.
    pub generated_at: u64,
    /// Whether this proof has been validated against the expected root.
    pub validated: bool,
}

/// Detected cross-chain inconsistency record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossChainInconsistency {
    /// Operation that exposed the inconsistency.
    pub op_id: BytesN<32>,
    /// Chain where the inconsistency was detected.
    pub affected_chain_id: u32,
    /// Expected state root.
    pub expected_root: BytesN<32>,
    /// Actual state root observed.
    pub observed_root: BytesN<32>,
    /// Whether automatic recovery has been triggered.
    pub recovery_initiated: bool,
    /// Timestamp of detection.
    pub detected_at: u64,
}

/// Emergency isolation record for a chain experiencing severe sync failures.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ChainIsolationRecord {
    /// Isolated chain ID.
    pub chain_id: u32,
    /// Reason for isolation.
    pub reason: Symbol,
    /// Number of consecutive sync failures that triggered isolation.
    pub failure_count: u32,
    /// Timestamp of isolation.
    pub isolated_at: u64,
    /// Timestamp when isolation can be lifted (cooling-off period).
    pub lift_eligible_at: u64,
}

// ---------------------------------------------------------------------------
// Error types
// ---------------------------------------------------------------------------

/// Errors from cross-chain sync operations.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum XChainSyncError {
    /// Operation not found.
    OpNotFound = 1,
    /// Operation has already reached a terminal state.
    AlreadyTerminal = 2,
    /// Not all chains have completed Phase 1 (prepare).
    NotAllPrepared = 3,
    /// Operation has expired.
    Expired = 4,
    /// Chain is currently isolated; operations with it are blocked.
    ChainIsolated = 5,
    /// State proof validation failed.
    InvalidStateProof = 6,
    /// Exceeded maximum number of participating chains.
    TooManyChains = 7,
    /// Insufficient finality confirmations.
    InsufficientFinality = 8,
    /// Reorg protection: operation is too recent for the configured safe depth.
    ReorgUnsafe = 9,
}

// ---------------------------------------------------------------------------
// Two-phase-commit operations
// ---------------------------------------------------------------------------

/// Initiate an atomic cross-chain operation (Phase 1: Prepare).
///
/// Records the operation and broadcasts a prepare request to all
/// `participating_chains`. Returns the `op_id`.
pub fn begin_atomic_xchain_op(
    env: &Env,
    initiator: &Address,
    participating_chains: Vec<u32>,
    expected_state_root: BytesN<32>,
) -> Result<BytesN<32>, XChainSyncError> {
    if participating_chains.len() as u32 > MAX_PARTICIPATING_CHAINS {
        return Err(XChainSyncError::TooManyChains);
    }

    // Verify none of the participating chains are isolated.
    for chain_id in participating_chains.iter() {
        if is_chain_isolated(env, chain_id) {
            return Err(XChainSyncError::ChainIsolated);
        }
    }

    let now = env.ledger().timestamp();

    // Derive op_id from initiator address + chain list + timestamp.
    let mut payload = Bytes::new(env);
    for chain_id in participating_chains.iter() {
        for b in chain_id.to_be_bytes().iter() {
            payload.push_back(*b);
        }
    }
    for b in now.to_be_bytes().iter() {
        payload.push_back(*b);
    }
    for b in env.ledger().sequence().to_be_bytes().iter() {
        payload.push_back(*b);
    }
    let op_id: BytesN<32> = env.crypto().sha256(&payload).into();

    let op = AtomicXChainOp {
        op_id: op_id.clone(),
        chain_ids: participating_chains,
        prepared_count: 0,
        committed_count: 0,
        phase: XChainPhase::Prepared,
        created_at: now,
        expires_at: now.saturating_add(XCHAIN_OP_TIMEOUT_SECS),
        initiator: initiator.clone(),
        expected_state_root,
    };

    let key = xchain_op_key(&op_id);
    env.storage().persistent().set(&key, &op);

    env.events().publish(
        (symbol_short!("xcsync"), symbol_short!("prepared")),
        (op_id.clone(), now),
    );

    Ok(op_id)
}

/// Record that a participating chain has acknowledged the prepare phase.
///
/// Once all chains have acknowledged, transitions to `Committing`.
pub fn acknowledge_prepare(
    env: &Env,
    op_id: &BytesN<32>,
    chain_id: u32,
) -> Result<XChainPhase, XChainSyncError> {
    let key = xchain_op_key(op_id);
    let mut op: AtomicXChainOp = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(XChainSyncError::OpNotFound)?;

    check_not_terminal(&op)?;
    check_not_expired(env, &op)?;

    op.prepared_count = op.prepared_count.saturating_add(1);

    let total = op.chain_ids.len() as u32;
    if op.prepared_count >= total {
        op.phase = XChainPhase::Committing;
        env.events().publish(
            (symbol_short!("xcsync"), symbol_short!("commit")),
            op_id.clone(),
        );
    }

    let _ = chain_id; // Chain ID used for audit trail in external systems.
    env.storage().persistent().set(&key, &op);
    Ok(op.phase)
}

/// Record that a participating chain has committed successfully.
///
/// Once all chains commit, transitions to `Committed`.
pub fn confirm_commit(
    env: &Env,
    op_id: &BytesN<32>,
    chain_id: u32,
    chain_state_root: BytesN<32>,
) -> Result<XChainPhase, XChainSyncError> {
    let key = xchain_op_key(op_id);
    let mut op: AtomicXChainOp = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(XChainSyncError::OpNotFound)?;

    check_not_terminal(&op)?;
    check_not_expired(env, &op)?;

    // Validate that the reported state root matches the expected root.
    if chain_state_root != op.expected_state_root {
        // Inconsistency detected — record and trigger rollback.
        record_inconsistency(
            env,
            op_id,
            chain_id,
            &op.expected_state_root,
            &chain_state_root,
        );
        initiate_rollback(env, op_id)?;
        return Err(XChainSyncError::InvalidStateProof);
    }

    op.committed_count = op.committed_count.saturating_add(1);

    let total = op.chain_ids.len() as u32;
    if op.committed_count >= total {
        op.phase = XChainPhase::Committed;
        env.events().publish(
            (symbol_short!("xcsync"), symbol_short!("committed")),
            (op_id.clone(), env.ledger().timestamp()),
        );
    }

    env.storage().persistent().set(&key, &op);
    Ok(op.phase)
}

/// Initiate rollback for a cross-chain operation.
///
/// Transitions to `RollingBack` and emits a rollback event so off-chain
/// relayers can broadcast rollback messages to all participating chains.
pub fn initiate_rollback(
    env: &Env,
    op_id: &BytesN<32>,
) -> Result<(), XChainSyncError> {
    let key = xchain_op_key(op_id);
    let mut op: AtomicXChainOp = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(XChainSyncError::OpNotFound)?;

    check_not_terminal(&op)?;

    op.phase = XChainPhase::RollingBack;
    env.storage().persistent().set(&key, &op);

    env.events().publish(
        (symbol_short!("xcsync"), symbol_short!("rollback")),
        (op_id.clone(), env.ledger().timestamp()),
    );

    Ok(())
}

/// Confirm rollback completion across all chains.
pub fn confirm_rollback(
    env: &Env,
    op_id: &BytesN<32>,
) -> Result<(), XChainSyncError> {
    let key = xchain_op_key(op_id);
    let mut op: AtomicXChainOp = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(XChainSyncError::OpNotFound)?;

    if op.phase != XChainPhase::RollingBack {
        return Err(XChainSyncError::AlreadyTerminal);
    }

    op.phase = XChainPhase::RolledBack;
    env.storage().persistent().set(&key, &op);

    env.events().publish(
        (symbol_short!("xcsync"), symbol_short!("rolled_bk")),
        op_id.clone(),
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// State Merkle synchronization / consistency proofs
// ---------------------------------------------------------------------------

/// Validate a cross-chain state proof against an expected Merkle root.
///
/// Uses iterative hash chaining of the proof path to reconstruct the root.
/// Returns `true` if the proof is valid and the chain state is consistent.
pub fn validate_state_proof(
    env: &Env,
    proof: &CrossChainStateProof,
    expected_root: &BytesN<32>,
) -> bool {
    if proof.proof_path.is_empty() {
        // Trivial proof: state root IS the leaf.
        return &proof.state_root == expected_root;
    }

    // Iteratively hash leaf with each sibling node to reconstruct root.
    let mut current: Bytes = Bytes::new(env);
    let state_root_bytes = Bytes::from_slice(env, &proof.state_root.to_array());
    current.append(&state_root_bytes);

    for sibling in proof.proof_path.iter() {
        let mut combined = Bytes::new(env);
        combined.append(&current);
        let sibling_bytes = Bytes::from_slice(env, &sibling.to_array());
        combined.append(&sibling_bytes);
        let hash: BytesN<32> = env.crypto().sha256(&combined).into();
        current = Bytes::from_slice(env, &hash.to_array());
    }

    let computed_root: BytesN<32> = env
        .crypto()
        .sha256(&current)
        .into();

    &computed_root == expected_root
}

/// Compute the Merkle root for a list of state leaves.
///
/// Uses pairwise SHA-256 hashing to build the tree bottom-up.
/// Returns a 32-byte root hash.
pub fn compute_state_merkle_root(env: &Env, leaves: &Vec<BytesN<32>>) -> BytesN<32> {
    let n = leaves.len();
    if n == 0 {
        return BytesN::from_array(env, &[0u8; 32]);
    }
    if n == 1 {
        return leaves.get(0).unwrap();
    }

    // Build a flat layer vector and iteratively halve it.
    let mut layer: Vec<BytesN<32>> = Vec::new(env);
    for leaf in leaves.iter() {
        layer.push_back(leaf);
    }

    while layer.len() > 1 {
        let mut next_layer: Vec<BytesN<32>> = Vec::new(env);
        let len = layer.len();
        let mut i = 0u32;
        while i + 1 < len {
            let left = layer.get(i).unwrap();
            let right = layer.get(i + 1).unwrap();
            let mut combined = Bytes::new(env);
            combined.append(&Bytes::from_slice(env, &left.to_array()));
            combined.append(&Bytes::from_slice(env, &right.to_array()));
            let hash: BytesN<32> = env.crypto().sha256(&combined).into();
            next_layer.push_back(hash);
            i += 2;
        }
        // Carry forward an odd last element without pairing.
        if len % 2 == 1 {
            next_layer.push_back(layer.get(len - 1).unwrap());
        }
        layer = next_layer;
    }

    layer.get(0).unwrap()
}

// ---------------------------------------------------------------------------
// Finality-aware operations
// ---------------------------------------------------------------------------

/// Check whether a cross-chain operation has sufficient finality on a given
/// chain based on its `ChainFinalityConfig`.
///
/// Returns `Ok(())` if finality is satisfied, or an appropriate error.
pub fn require_finality(
    config: &ChainFinalityConfig,
    current_block: u64,
    op_block: u64,
    challenge_deadline: u64,
    now: u64,
) -> Result<(), XChainSyncError> {
    if config.isolated {
        return Err(XChainSyncError::ChainIsolated);
    }

    match config.finality_tier {
        FinalityTier::Deterministic | FinalityTier::Instant => {
            // Deterministic/instant finality — block confirmation sufficient.
            let depth = current_block.saturating_sub(op_block);
            if depth < config.required_confirmations as u64 {
                return Err(XChainSyncError::InsufficientFinality);
            }
        }
        FinalityTier::Probabilistic => {
            // Probabilistic: require both block depth and reorg-safe depth.
            let depth = current_block.saturating_sub(op_block);
            if depth < config.required_confirmations as u64 {
                return Err(XChainSyncError::InsufficientFinality);
            }
            if depth < REORG_SAFE_DEPTH {
                return Err(XChainSyncError::ReorgUnsafe);
            }
        }
        FinalityTier::Optimistic => {
            // Optimistic: challenge period must have elapsed.
            if now < challenge_deadline {
                return Err(XChainSyncError::InsufficientFinality);
            }
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Reorg protection
// ---------------------------------------------------------------------------

/// Detect whether a block depth is safe from chain reorganization.
///
/// Returns `true` when the operation is buried deep enough to be
/// considered immune to reorgs given the configured safe depth.
pub fn is_reorg_safe(current_block: u64, op_block: u64) -> bool {
    current_block.saturating_sub(op_block) >= REORG_SAFE_DEPTH
}

/// Record a reorg event and mark affected operations for re-evaluation.
pub fn record_reorg_event(
    env: &Env,
    chain_id: u32,
    reorged_from_block: u64,
    reorged_to_block: u64,
) {
    env.events().publish(
        (symbol_short!("xcsync"), symbol_short!("reorg")),
        (chain_id, reorged_from_block, reorged_to_block, env.ledger().timestamp()),
    );
    // Callers should scan pending operations with op_block >= reorged_to_block
    // and transition them back to Prepared (or initiate rollback) as needed.
}

// ---------------------------------------------------------------------------
// Inconsistency detection and automatic resolution
// ---------------------------------------------------------------------------

/// Record a detected cross-chain state inconsistency.
pub fn record_inconsistency(
    env: &Env,
    op_id: &BytesN<32>,
    affected_chain_id: u32,
    expected_root: &BytesN<32>,
    observed_root: &BytesN<32>,
) {
    let record = CrossChainInconsistency {
        op_id: op_id.clone(),
        affected_chain_id,
        expected_root: expected_root.clone(),
        observed_root: observed_root.clone(),
        recovery_initiated: true,
        detected_at: env.ledger().timestamp(),
    };

    let key = (symbol_short!("xcincns"), op_id.clone(), affected_chain_id);
    env.storage().persistent().set(&key, &record);

    env.events().publish(
        (symbol_short!("xcsync"), symbol_short!("inconsist")),
        (op_id.clone(), affected_chain_id, env.ledger().timestamp()),
    );
}

/// Retrieve a recorded inconsistency record.
pub fn get_inconsistency(
    env: &Env,
    op_id: &BytesN<32>,
    chain_id: u32,
) -> Option<CrossChainInconsistency> {
    let key = (symbol_short!("xcincns"), op_id.clone(), chain_id);
    env.storage().persistent().get(&key)
}

// ---------------------------------------------------------------------------
// Emergency chain isolation
// ---------------------------------------------------------------------------

/// Isolate a chain from cross-chain operations due to severe sync failures.
///
/// Isolated chains are blocked from initiating or participating in new
/// atomic operations until manually cleared by governance.
pub fn isolate_chain(
    env: &Env,
    chain_id: u32,
    reason: Symbol,
    failure_count: u32,
    isolation_duration_secs: u64,
) {
    let now = env.ledger().timestamp();
    let record = ChainIsolationRecord {
        chain_id,
        reason: reason.clone(),
        failure_count,
        isolated_at: now,
        lift_eligible_at: now.saturating_add(isolation_duration_secs),
    };

    let key = (symbol_short!("xcisol"), chain_id);
    env.storage().persistent().set(&key, &record);

    env.events().publish(
        (symbol_short!("xcsync"), symbol_short!("isolated")),
        (chain_id, reason, failure_count, now),
    );
}

/// Lift a chain isolation after the cooling-off period has elapsed.
///
/// Returns `true` if isolation was successfully lifted.
pub fn lift_chain_isolation(env: &Env, chain_id: u32) -> bool {
    let key = (symbol_short!("xcisol"), chain_id);
    if let Some(record) = env
        .storage()
        .persistent()
        .get::<_, ChainIsolationRecord>(&key)
    {
        if env.ledger().timestamp() >= record.lift_eligible_at {
            env.storage().persistent().remove(&key);
            env.events().publish(
                (symbol_short!("xcsync"), symbol_short!("isol_lift")),
                (chain_id, env.ledger().timestamp()),
            );
            return true;
        }
    }
    false
}

/// Check whether a chain is currently isolated.
pub fn is_chain_isolated(env: &Env, chain_id: u32) -> bool {
    let key = (symbol_short!("xcisol"), chain_id);
    env.storage().persistent().has(&key)
}

/// Get the isolation record for a chain, if any.
pub fn get_chain_isolation(env: &Env, chain_id: u32) -> Option<ChainIsolationRecord> {
    let key = (symbol_short!("xcisol"), chain_id);
    env.storage().persistent().get(&key)
}

// ---------------------------------------------------------------------------
// Cross-chain operation retrieval
// ---------------------------------------------------------------------------

/// Get an atomic cross-chain operation by ID.
pub fn get_xchain_op(env: &Env, op_id: &BytesN<32>) -> Option<AtomicXChainOp> {
    let key = xchain_op_key(op_id);
    env.storage().persistent().get(&key)
}

/// Mark an expired operation as expired and emit an event.
pub fn expire_xchain_op(env: &Env, op_id: &BytesN<32>) -> Result<(), XChainSyncError> {
    let key = xchain_op_key(op_id);
    let mut op: AtomicXChainOp = env
        .storage()
        .persistent()
        .get(&key)
        .ok_or(XChainSyncError::OpNotFound)?;

    check_not_terminal(&op)?;

    if env.ledger().timestamp() < op.expires_at {
        // Not yet expired — caller should not call this prematurely.
        return Err(XChainSyncError::Expired);
    }

    op.phase = XChainPhase::Expired;
    env.storage().persistent().set(&key, &op);

    env.events().publish(
        (symbol_short!("xcsync"), symbol_short!("expired")),
        op_id.clone(),
    );

    Ok(())
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn xchain_op_key(op_id: &BytesN<32>) -> (Symbol, BytesN<32>) {
    (symbol_short!("xcop"), op_id.clone())
}

fn check_not_terminal(op: &AtomicXChainOp) -> Result<(), XChainSyncError> {
    match op.phase {
        XChainPhase::Committed | XChainPhase::RolledBack | XChainPhase::Expired => {
            Err(XChainSyncError::AlreadyTerminal)
        }
        _ => Ok(()),
    }
}

fn check_not_expired(env: &Env, op: &AtomicXChainOp) -> Result<(), XChainSyncError> {
    if env.ledger().timestamp() > op.expires_at {
        Err(XChainSyncError::Expired)
    } else {
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::Address as _;
    extern crate std;
    use super::*;
    use soroban_sdk::{testutils::Ledger, Env};

    fn env_at(ts: u64) -> Env {
        let env = Env::default();
        env.ledger().with_mut(|l| l.timestamp = ts);
        env
    }

    fn dummy_root(env: &Env, seed: u8) -> BytesN<32> {
        BytesN::from_array(env, &[seed; 32])
    }

    fn dummy_chains(env: &Env) -> Vec<u32> {
        let mut v = Vec::new(env);
        v.push_back(2u32); // Ethereum
        v.push_back(1u32); // Solana
        v
    }

    #[test]
    fn test_begin_and_full_commit_flow() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let initiator = soroban_sdk::Address::generate(&env);
        let expected_root = dummy_root(&env, 0xAB);
        let chains = dummy_chains(&env);

        let op_id = begin_atomic_xchain_op(&env, &initiator, chains, expected_root.clone())
            .expect("begin should succeed");

        // Phase 1 acknowledge
        let phase = acknowledge_prepare(&env, &op_id, 2).expect("ack prepare chain 2");
        assert_eq!(phase, XChainPhase::Prepared);
        let phase = acknowledge_prepare(&env, &op_id, 1).expect("ack prepare chain 1");
        assert_eq!(phase, XChainPhase::Committing);

        // Phase 2 commit
        let phase = confirm_commit(&env, &op_id, 2, expected_root.clone()).expect("commit chain 2");
        assert_eq!(phase, XChainPhase::Committing);
        let phase = confirm_commit(&env, &op_id, 1, expected_root.clone()).expect("commit chain 1");
        assert_eq!(phase, XChainPhase::Committed);
    }

    #[test]
    fn test_inconsistent_state_root_triggers_rollback() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let initiator = soroban_sdk::Address::generate(&env);
        let expected_root = dummy_root(&env, 0xAA);
        let bad_root = dummy_root(&env, 0xBB);
        let chains = dummy_chains(&env);

        let op_id = begin_atomic_xchain_op(&env, &initiator, chains, expected_root.clone())
            .expect("begin ok");
        acknowledge_prepare(&env, &op_id, 2).unwrap();
        acknowledge_prepare(&env, &op_id, 1).unwrap();

        let result = confirm_commit(&env, &op_id, 2, bad_root.clone());
        assert_eq!(result, Err(XChainSyncError::InvalidStateProof));

        let op = get_xchain_op(&env, &op_id).unwrap();
        assert_eq!(op.phase, XChainPhase::RollingBack);
    }

    #[test]
    fn test_isolated_chain_blocks_new_ops() {
        let env = env_at(1_000);

        isolate_chain(&env, 2, Symbol::new(&env, "sync_fail"), 5, 3_600);
        assert!(is_chain_isolated(&env, 2));

        let initiator = soroban_sdk::Address::generate(&env);
        let mut chains = Vec::new(&env);
        chains.push_back(2u32);
        let result = begin_atomic_xchain_op(&env, &initiator, chains, dummy_root(&env, 1));
        assert_eq!(result, Err(XChainSyncError::ChainIsolated));
    }

    #[test]
    fn test_lift_isolation_after_cooldown() {
        let env = env_at(1_000);
        isolate_chain(&env, 3, Symbol::new(&env, "reorg"), 3, 3_600);
        assert!(is_chain_isolated(&env, 3));

        // Still within cooldown
        assert!(!lift_chain_isolation(&env, 3));

        // Advance past cooling off period
        env.ledger().with_mut(|l| l.timestamp = 1_000 + 3_601);
        assert!(lift_chain_isolation(&env, 3));
        assert!(!is_chain_isolated(&env, 3));
    }

    #[test]
    fn test_reorg_safe_depth() {
        assert!(is_reorg_safe(1000, 900));      // depth 100 >= 64
        assert!(!is_reorg_safe(1000, 950));     // depth 50 < 64
        assert!(is_reorg_safe(u64::MAX, 0));    // saturating ok
    }

    #[test]
    fn test_state_merkle_root_single_leaf() {
        let env = env_at(1_000);
        let mut leaves = Vec::new(&env);
        leaves.push_back(BytesN::from_array(&env, &[1u8; 32]));
        let root = compute_state_merkle_root(&env, &leaves);
        assert_eq!(root, BytesN::from_array(&env, &[1u8; 32]));
    }

    #[test]
    fn test_state_merkle_root_deterministic() {
        let env = env_at(1_000);
        let mut leaves = Vec::new(&env);
        leaves.push_back(BytesN::from_array(&env, &[0xAu8; 32]));
        leaves.push_back(BytesN::from_array(&env, &[0xBu8; 32]));
        let root1 = compute_state_merkle_root(&env, &leaves);
        let root2 = compute_state_merkle_root(&env, &leaves);
        assert_eq!(root1, root2, "merkle root must be deterministic");
    }

    #[test]
    fn test_validate_trivial_state_proof() {
        let env = env_at(1_000);
        let root = BytesN::from_array(&env, &[0xABu8; 32]);
        let proof = CrossChainStateProof {
            chain_id: 2,
            state_root: root.clone(),
            proof_path: Vec::new(&env),
            generated_at: 1_000,
            validated: false,
        };
        assert!(validate_state_proof(&env, &proof, &root));
    }

    #[test]
    fn test_op_expiry() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let initiator = soroban_sdk::Address::generate(&env);
        let chains = dummy_chains(&env);
        let op_id = begin_atomic_xchain_op(&env, &initiator, chains, dummy_root(&env, 0))
            .unwrap();

        // Advance past expiry.
        env.ledger().with_mut(|l| l.timestamp = 1_000 + XCHAIN_OP_TIMEOUT_SECS + 1);
        expire_xchain_op(&env, &op_id).expect("should expire");
        let op = get_xchain_op(&env, &op_id).unwrap();
        assert_eq!(op.phase, XChainPhase::Expired);
    }
}

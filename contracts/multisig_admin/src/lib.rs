#![no_std]
#![allow(deprecated)] // Temporarily allow deprecated Events::publish until we migrate to #[contractevent]

use shared::{
    compute_checksum, compute_justice_intervention, protect_arbitration_fairness,
    push_snapshot_index, ArbitrationBiasFlag, DisputeIndependenceFlag, EvidenceAuthenticity,
    JusticeInterventionRecord, MultisigValidation, RollbackAuthorization, RollbackJustification,
    RollbackProposal, SnapshotMeta, StateVerificationReport, EMERGENCY_MSIG_SIGNERS,
    EMERGENCY_MSIG_THRESHOLD, EMERGENCY_THRESHOLD, JUSTICE_RESTORATION_COOLDOWN_SECS,
    MAX_SNAPSHOTS, SecureStorageAccess,
    // #868 — Key management and rotation
    emergency_revoke_key, execute_key_rotation, get_current_key, is_key_revoked,
    is_rotation_due, propose_key_rotation, register_key,
    KeyRecord, KeyRotationProposal, KeyScheme,
    // #867 — Transaction intent / high-risk operation protection
    evaluate_transaction_intent, get_protection_state, TransactionIntent,
};
use soroban_sdk::{
    contract, contractimpl, contracterror, contracttype, symbol_short, Address, Bytes, BytesN,
    Env, Symbol, TryIntoVal, Val, Vec,
};


// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
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
    /// Emergency signature set failed 4-of-7 validation.
    InvalidEmergencySignatures = 13,
    /// Duplicate or unregistered emergency signer in aggregation.
    InvalidEmergencySigner = 14,
}

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub struct ProposalRecord {
    pub id:             u32,
    pub proposer:       Address,
    pub target:         Address,
    pub function:       Symbol,
    pub args:           Vec<Val>,
    pub approval_count: u32,
    pub expiry:         u64,
    pub executed:       bool,
    pub cancelled:      bool,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const EXPIRY_SECONDS: u64 = 7 * 24 * 60 * 60; // 7 days

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Threshold,
    SignerCount,
    ProposalCount,
    Signer(Address),
    Proposal(u32),
    Approval(u32, Address),
    // -----------------------------------------------------------------------
    // Disaster-recovery keys
    // -----------------------------------------------------------------------
    /// Serialised governance config snapshot for snapshot `n`.
    GovSnapshot(u32),
    /// SnapshotMeta for governance snapshot `n`.
    GovSnapshotMeta(u32),
    /// Ordered Vec<u32> of retained governance snapshot IDs.
    GovSnapshotIndex,
    /// Vec<Address> of up to 7 emergency signers for governance rollback.
    GovEmergencySigners,
    /// RollbackProposal for governance rollback proposal `n`.
    GovRollbackProposal(u32),
    /// Boolean approval for (proposal_id, signer) governance rollback.
    GovRollbackApproval(u32, Address),
    /// Auto-incremented governance rollback proposal counter.
    GovRollbackProposalCount,
    // -----------------------------------------------------------------------
    // Justice-monitoring / dispute-oversight keys (#justice-protection)
    // -----------------------------------------------------------------------
    /// Latest recorded dispute-oversight audit for a given escrow_id.
    DisputeAudit(u64),
    /// Latest recorded arbitration-fairness audit for a given arbitrator.
    ArbitratorAudit(Address),
    // -----------------------------------------------------------------------
    // #868 — Key management keys
    // -----------------------------------------------------------------------
    /// Key management namespace root for this multisig instance.
    KeyManagementRoot,
}

/// Record of a multisig-signer-reviewed dispute oversight audit.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeOversightRecord {
    pub escrow_id: u64,
    pub reviewer: Address,
    pub justice_status: JusticeInterventionRecord,
    pub reviewed_at: u64,
}

// ---------------------------------------------------------------------------
// DR-only TTL constants (persistent; 57 day window)
// ---------------------------------------------------------------------------
const DR_TTL_THRESHOLD: u32 = 500_000;
const DR_TTL_BUMP: u32 = 1_000_000;

/// Compact governance config snapshot stored per snapshot_id.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GovConfigSnapshot {
    /// Approval threshold at snapshot time.
    pub threshold: u32,
    /// Number of registered signers at snapshot time.
    pub signer_count: u32,
    /// Total proposals created at snapshot time.
    pub proposal_count: u32,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct MultisigAdminContract;

#[contractimpl]
impl MultisigAdminContract {
    /// Initialize with a list of signers and an approval threshold.
    /// Supports 2-of-3 or 3-of-5 (or any valid combination).
    pub fn initialize(
        env: Env,
        signers: Vec<Address>,
        threshold: u32,
    ) -> Result<(), Error> {
        SecureStorageAccess::install_namespace(&env, &DataKey::NamespaceRoot, symbol_short!("mm_msig"));

        if env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::AlreadyInitialized);
        }
        if signers.is_empty() || threshold == 0 {
            return Err(Error::InvalidThreshold);
        }
        if threshold > signers.len() as u32 {
            return Err(Error::InvalidThreshold);
        }
        for signer in signers.iter() {
            if env.storage().persistent().has(&DataKey::Signer(signer.clone())) {
                return Err(Error::AlreadySigner);
            }
            env.storage().persistent().set(&DataKey::Signer(signer.clone()), &true);
        }
        env.storage().instance().set(&DataKey::Threshold, &threshold);
        env.storage().instance().set(&DataKey::SignerCount, &(signers.len() as u32));
        env.storage().instance().set(&DataKey::ProposalCount, &0u32);
        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("init")),
            (signers.len() as u32, threshold),
        );
        Ok(())
    }

    /// Propose an action requiring multi-sig approval.
    /// The proposer automatically counts as the first approval.
    pub fn propose_action(
        env: Env,
        proposer: Address,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
    ) -> Result<u32, Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        if !env.storage().persistent().get::<_, bool>(&DataKey::Signer(proposer.clone())).unwrap_or(false) {
            return Err(Error::NotSigner);
        }
        proposer.require_auth();
        let count: u32 = env.storage().instance().get(&DataKey::ProposalCount).unwrap_or(0);
        let new_id = count.checked_add(1).expect("proposal count overflow");
        env.storage().instance().set(&DataKey::ProposalCount, &new_id);
        let expiry = env.ledger().timestamp().checked_add(EXPIRY_SECONDS).expect("expiry overflow");
        let proposal = ProposalRecord {
            id: new_id,
            proposer: proposer.clone(),
            target,
            function,
            args,
            approval_count: 1,
            expiry,
            executed: false,
            cancelled: false,
        };
        env.storage().persistent().set(&DataKey::Proposal(new_id), &proposal);
        env.storage().persistent().set(&DataKey::Approval(new_id, proposer.clone()), &true);
        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("proposed"), new_id),
            (proposer, expiry),
        );
        Ok(new_id)
    }

    /// Sign (approve) an existing proposal.
    pub fn sign_action(
        env: Env,
        signer: Address,
        action_id: u32,
    ) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        if !env.storage().persistent().get::<_, bool>(&DataKey::Signer(signer.clone())).unwrap_or(false) {
            return Err(Error::NotSigner);
        }
        let mut proposal: ProposalRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(action_id))
            .ok_or(Error::ProposalNotFound)?;
        if proposal.executed  { return Err(Error::AlreadyExecuted); }
        if proposal.cancelled { return Err(Error::Cancelled); }
        if env.ledger().timestamp() > proposal.expiry { return Err(Error::Expired); }
        if env.storage().persistent().get::<_, bool>(&DataKey::Approval(action_id, signer.clone())).unwrap_or(false) {
            return Err(Error::AlreadySigned);
        }
        signer.require_auth();
        env.storage().persistent().set(&DataKey::Approval(action_id, signer.clone()), &true);
        proposal.approval_count = proposal.approval_count.checked_add(1).expect("approval count overflow");
        env.storage().persistent().set(&DataKey::Proposal(action_id), &proposal);
        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("signed"), action_id),
            (signer, proposal.approval_count),
        );
        Ok(())
    }

    /// Execute a proposal once threshold approvals are reached.
    /// Supports self-targeted admin operations (add_signer, remove_signer, update_threshold)
    /// and external contract calls.
    pub fn execute_action(
        env: Env,
        action_id: u32,
    ) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        let mut proposal: ProposalRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(action_id))
            .ok_or(Error::ProposalNotFound)?;
        if proposal.executed  { return Err(Error::AlreadyExecuted); }
        if proposal.cancelled { return Err(Error::Cancelled); }
        if env.ledger().timestamp() > proposal.expiry { return Err(Error::Expired); }
        let threshold: u32 = env.storage().instance().get(&DataKey::Threshold).unwrap();
        if proposal.approval_count < threshold { return Err(Error::BelowThreshold); }

        // Mark executed before dispatch (prevents re-entrancy)
        proposal.executed = true;
        env.storage().persistent().set(&DataKey::Proposal(action_id), &proposal);

        let event_target   = proposal.target.clone();
        let event_function = proposal.function.clone();

        if proposal.target == env.current_contract_address() {
            let add_fn    = Symbol::new(&env, "add_signer");
            let rem_fn    = Symbol::new(&env, "remove_signer");
            let thresh_fn = Symbol::new(&env, "update_threshold");
            if proposal.function == add_fn {
                let new_signer: Address = proposal.args.get(0)
                    .ok_or(Error::ProposalNotFound)?
                    .try_into_val(&env)
                    .map_err(|_| Error::ProposalNotFound)?;
                apply_add_signer(&env, new_signer)?;
            } else if proposal.function == rem_fn {
                let target_signer: Address = proposal.args.get(0)
                    .ok_or(Error::ProposalNotFound)?
                    .try_into_val(&env)
                    .map_err(|_| Error::ProposalNotFound)?;
                apply_remove_signer(&env, target_signer)?;
            } else if proposal.function == thresh_fn {
                let new_threshold: u32 = proposal.args.get(0)
                    .ok_or(Error::ProposalNotFound)?
                    .try_into_val(&env)
                    .map_err(|_| Error::ProposalNotFound)?;
                apply_update_threshold(&env, new_threshold)?;
            } else {
                return Err(Error::ProposalNotFound);
            }
        } else {
            env.invoke_contract::<()>(&proposal.target, &proposal.function, proposal.args.clone());
        }

        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("executed"), action_id),
            (action_id, event_target, event_function),
        );
        Ok(())
    }

    /// Cancel a proposal. Only the proposer or any current signer may cancel.
    pub fn cancel_action(
        env: Env,
        caller: Address,
        action_id: u32,
    ) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        let mut proposal: ProposalRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Proposal(action_id))
            .ok_or(Error::ProposalNotFound)?;
        let is_proposer = proposal.proposer == caller;
        let is_signer   = env.storage().persistent()
            .get::<_, bool>(&DataKey::Signer(caller.clone()))
            .unwrap_or(false);
        if !is_proposer && !is_signer { return Err(Error::NotSigner); }
        if proposal.executed  { return Err(Error::AlreadyExecuted); }
        if proposal.cancelled { return Err(Error::Cancelled); }
        if env.ledger().timestamp() > proposal.expiry { return Err(Error::Expired); }
        caller.require_auth();
        proposal.cancelled = true;
        env.storage().persistent().set(&DataKey::Proposal(action_id), &proposal);
        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("cancelled"), action_id),
            caller,
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // View functions
    // -----------------------------------------------------------------------

    pub fn get_proposal(env: Env, action_id: u32) -> Result<ProposalRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Proposal(action_id))
            .ok_or(Error::ProposalNotFound)
    }

    pub fn is_signer(env: Env, address: Address) -> Result<bool, Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        Ok(env.storage().persistent()
            .get::<_, bool>(&DataKey::Signer(address))
            .unwrap_or(false))
    }

    pub fn get_threshold(env: Env) -> Result<u32, Error> {
        env.storage().instance()
            .get(&DataKey::Threshold)
            .ok_or(Error::NotInitialized)
    }

    pub fn get_signer_count(env: Env) -> Result<u32, Error> {
        env.storage().instance()
            .get(&DataKey::SignerCount)
            .ok_or(Error::NotInitialized)
    }

    // -----------------------------------------------------------------------
    // Emergency signature validation (4-of-7)
    // -----------------------------------------------------------------------

    /// Validate that `approvals` is an exact 4-of-7 set drawn from the
    /// registered governance emergency signers.
    ///
    /// Used by escrow and other consumers that need host-side confirmation
    /// that an aggregated emergency signature set meets threshold policy.
    pub fn validate_emergency_signatures(
        env: Env,
        approvals: Vec<Address>,
    ) -> Result<bool, Error> {
        let registered: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::GovEmergencySigners)
            .ok_or(Error::NotInitialized)?;
        if !MultisigValidation::validate_emergency_signatures(&registered, &approvals) {
            return Err(Error::InvalidEmergencySignatures);
        }
        Ok(true)
    }

    /// Aggregate a newly authenticated emergency signer into an approval set.
    ///
    /// `signer` must `require_auth` and must be a registered emergency signer.
    /// Returns the updated approval vector (deduplicated).
    pub fn aggregate_signatures(
        env: Env,
        signer: Address,
        mut approvals: Vec<Address>,
    ) -> Result<Vec<Address>, Error> {
        let registered: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::GovEmergencySigners)
            .ok_or(Error::NotInitialized)?;
        if !MultisigValidation::is_emergency_signer(&registered, &signer) {
            return Err(Error::InvalidEmergencySigner);
        }
        signer.require_auth();
        MultisigValidation::aggregate_signatures(&mut approvals, signer);
        // Cap at threshold — callers should stop collecting once exact 4 is reached.
        if (approvals.len() as u32) > EMERGENCY_MSIG_THRESHOLD {
            return Err(Error::InvalidEmergencySignatures);
        }
        Ok(approvals)
    }

    /// Return the emergency multisig threshold constant (always 4).
    pub fn get_emergency_threshold(_env: Env) -> u32 {
        EMERGENCY_MSIG_THRESHOLD
    }

    /// Return the emergency signer slot count constant (always 7).
    pub fn get_emergency_signer_slots(_env: Env) -> u32 {
        EMERGENCY_MSIG_SIGNERS
    }

    // =======================================================================
    // Disaster Recovery — Governance Contract
    // =======================================================================

    /// Register emergency signers for governance-level rollback (admin-threshold signers only).
    ///
    /// Must provide exactly 7 addresses. A signer added here does not need to
    /// be a regular multisig signer — they form a separate emergency break-glass
    /// authority.
    ///
    /// # Auth
    /// Requires that the calling set has already reached the current threshold
    /// (enforced by requiring a valid threshold-passing proposal to be provided
    /// or by the caller being among the existing signers with sufficient approvals).
    /// For simplicity in the DR path, this is callable by any current signer.
    pub fn set_emergency_signers(
        env: Env,
        caller: Address,
        signers: Vec<Address>,
    ) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        // Caller must be a regular multisig signer to set emergency signers.
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Signer(caller.clone()))
            .unwrap_or(false)
        {
            return Err(Error::NotSigner);
        }
        caller.require_auth();
        if !MultisigValidation::is_valid_emergency_config(&signers, EMERGENCY_MSIG_THRESHOLD) {
            return Err(Error::InvalidThreshold);
        }
        env.storage()
            .persistent()
            .set(&DataKey::GovEmergencySigners, &signers);
        env.storage().persistent().extend_ttl(
            &DataKey::GovEmergencySigners,
            DR_TTL_THRESHOLD,
            DR_TTL_BUMP,
        );
        env.events().publish(
            (
                symbol_short!("DR"),
                symbol_short!("sgn_set"),
            ),
            signers.len() as u32,
        );
        Ok(())
    }

    /// Capture a governance config snapshot before an upgrade.
    ///
    /// Records `Threshold`, `SignerCount`, `ProposalCount` and associated
    /// `SnapshotMeta` under `DataKey::GovSnapshot(snapshot_id)`.  Manages
    /// a rolling window of at most `MAX_SNAPSHOTS` (3); the oldest is evicted
    /// automatically when a 4th is created.
    ///
    /// # Auth
    /// Caller must be a registered multisig signer.
    pub fn snapshot_state(
        env: Env,
        caller: Address,
        snapshot_id: u32,
    ) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Signer(caller.clone()))
            .unwrap_or(false)
        {
            return Err(Error::NotSigner);
        }
        caller.require_auth();

        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::Threshold)
            .unwrap_or(0);
        let signer_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::SignerCount)
            .unwrap_or(0);
        let proposal_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalCount)
            .unwrap_or(0);

        // Compute checksum
        let mut checksum_input = Bytes::new(&env);
        for b in threshold.to_be_bytes().iter() {
            checksum_input.push_back(*b);
        }
        for b in signer_count.to_be_bytes().iter() {
            checksum_input.push_back(*b);
        }
        for b in proposal_count.to_be_bytes().iter() {
            checksum_input.push_back(*b);
        }
        let checksum = compute_checksum(&env, &checksum_input);

        // Build config snapshot
        let config = GovConfigSnapshot {
            threshold,
            signer_count,
            proposal_count,
        };

        // WASM hash for version tracking
        let wasm_hash: BytesN<32> = BytesN::from_array(&env, &[0; 32]);

        // Manage rolling window
        let mut index: Vec<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::GovSnapshotIndex)
            .unwrap_or(Vec::new(&env));
        let snapshot_pos = index.len() as u32;
        let evicted = push_snapshot_index(&mut index, snapshot_id);
        if let Some(old_id) = evicted {
            env.storage()
                .persistent()
                .remove(&DataKey::GovSnapshot(old_id));
            env.storage()
                .persistent()
                .remove(&DataKey::GovSnapshotMeta(old_id));
        }
        env.storage()
            .persistent()
            .set(&DataKey::GovSnapshotIndex, &index);
        env.storage().persistent().extend_ttl(
            &DataKey::GovSnapshotIndex,
            DR_TTL_THRESHOLD,
            DR_TTL_BUMP,
        );

        let meta = SnapshotMeta {
            created_at: env.ledger().timestamp(),
            block_height: env.ledger().sequence(),
            contract_version: wasm_hash,
            admin: caller.clone(),
            checksum,
            record_count: signer_count as u64,
            snapshot_index: snapshot_pos.min(MAX_SNAPSHOTS - 1),
        };

        env.storage()
            .persistent()
            .set(&DataKey::GovSnapshot(snapshot_id), &config);
        env.storage().persistent().extend_ttl(
            &DataKey::GovSnapshot(snapshot_id),
            DR_TTL_THRESHOLD,
            DR_TTL_BUMP,
        );
        env.storage()
            .persistent()
            .set(&DataKey::GovSnapshotMeta(snapshot_id), &meta);
        env.storage().persistent().extend_ttl(
            &DataKey::GovSnapshotMeta(snapshot_id),
            DR_TTL_THRESHOLD,
            DR_TTL_BUMP,
        );

        env.events().publish(
            (symbol_short!("DR"), symbol_short!("gov_snap"), snapshot_id),
            (signer_count, env.ledger().sequence()),
        );
        Ok(())
    }

    /// Compare governance snapshot against current state.
    ///
    /// Checks `Threshold`, `SignerCount`, and `ProposalCount` from the
    /// snapshot metadata against live instance storage.
    ///
    /// # Returns
    /// A `StateVerificationReport` (mismatches empty = state intact).
    ///
    /// # Errors
    /// `ProposalNotFound` if `snapshot_id` does not exist.
    pub fn verify_post_upgrade_state(
        env: Env,
        snapshot_id: u32,
    ) -> Result<StateVerificationReport, Error> {
        let config: GovConfigSnapshot = env
            .storage()
            .persistent()
            .get(&DataKey::GovSnapshot(snapshot_id))
            .ok_or(Error::ProposalNotFound)?;

        let mut mismatches: Vec<soroban_sdk::String> = Vec::new(&env);
        let mut fields_checked: u32 = 0;

        let cur_threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::Threshold)
            .unwrap_or(0);
        fields_checked += 1;
        if cur_threshold != config.threshold {
            mismatches.push_back(soroban_sdk::String::from_str(&env, "Threshold mismatch"));
        }

        let cur_signer_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::SignerCount)
            .unwrap_or(0);
        fields_checked += 1;
        if cur_signer_count != config.signer_count {
            mismatches.push_back(soroban_sdk::String::from_str(
                &env,
                "SignerCount mismatch",
            ));
        }

        let cur_proposal_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ProposalCount)
            .unwrap_or(0);
        fields_checked += 1;
        if cur_proposal_count != config.proposal_count {
            mismatches.push_back(soroban_sdk::String::from_str(
                &env,
                "ProposalCount mismatch",
            ));
        }

        Ok(StateVerificationReport {
            fields_checked,
            mismatches,
        })
    }

    /// Open an emergency governance rollback proposal.
    ///
    /// `proposer` must be a registered governance emergency signer.
    pub fn propose_emergency_rollback(
        env: Env,
        proposer: Address,
        snapshot_id: u32,
        old_wasm_hash: BytesN<32>,
    ) -> Result<u32, Error> {
        let signers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::GovEmergencySigners)
            .ok_or(Error::NotInitialized)?;
        if !signers.iter().any(|s| s == proposer) {
            return Err(Error::NotSigner);
        }
        if !env
            .storage()
            .persistent()
            .has(&DataKey::GovSnapshotMeta(snapshot_id))
        {
            return Err(Error::ProposalNotFound);
        }
        proposer.require_auth();

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::GovRollbackProposalCount)
            .unwrap_or(0);
        let new_id = count.checked_add(1).expect("Governance rollback count overflow");
        env.storage()
            .persistent()
            .set(&DataKey::GovRollbackProposalCount, &new_id);

        let proposal = RollbackProposal {
            id: new_id,
            snapshot_id,
            old_wasm_hash: old_wasm_hash.clone(),
            approval_count: 1,
            executed: false,
            created_at: env.ledger().timestamp(),
            proposer: proposer.clone(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::GovRollbackProposal(new_id), &proposal);
        env.storage().persistent().extend_ttl(
            &DataKey::GovRollbackProposal(new_id),
            DR_TTL_THRESHOLD,
            DR_TTL_BUMP,
        );
        env.storage().persistent().set(
            &DataKey::GovRollbackApproval(new_id, proposer.clone()),
            &true,
        );

        env.events().publish(
            (
                symbol_short!("DR"),
                symbol_short!("grb_prop"),
                new_id,
            ),
            (snapshot_id, proposer, old_wasm_hash),
        );
        Ok(new_id)
    }

    /// Cast an approval on an open governance rollback proposal.
    ///
    /// `signer` must be a registered governance emergency signer.
    pub fn approve_emergency_rollback(
        env: Env,
        signer: Address,
        proposal_id: u32,
    ) -> Result<(), Error> {
        let signers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::GovEmergencySigners)
            .ok_or(Error::NotInitialized)?;
        if !signers.iter().any(|s| s == signer) {
            return Err(Error::NotSigner);
        }

        let mut proposal: RollbackProposal = env
            .storage()
            .persistent()
            .get(&DataKey::GovRollbackProposal(proposal_id))
            .ok_or(Error::ProposalNotFound)?;
        if proposal.executed {
            return Err(Error::AlreadyExecuted);
        }
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::GovRollbackApproval(proposal_id, signer.clone()))
            .unwrap_or(false)
        {
            return Err(Error::AlreadySigned);
        }
        signer.require_auth();

        env.storage().persistent().set(
            &DataKey::GovRollbackApproval(proposal_id, signer.clone()),
            &true,
        );
        proposal.approval_count = proposal
            .approval_count
            .checked_add(1)
            .expect("Approval count overflow");
        env.storage()
            .persistent()
            .set(&DataKey::GovRollbackProposal(proposal_id), &proposal);

        env.events().publish(
            (
                symbol_short!("DR"),
                symbol_short!("grb_aprv"),
                proposal_id,
            ),
            (signer, proposal.approval_count),
        );
        Ok(())
    }

    /// Execute a governance rollback after 4-of-7 approval.
    ///
    /// Restores `Threshold`, `SignerCount`, and `ProposalCount` from the
    /// snapshot, then re-applies the old WASM binary.
    ///
    /// # Pre-conditions
    /// * Old WASM must be pre-uploaded via `soroban contract install`.
    /// * `EMERGENCY_THRESHOLD` (4) distinct approvals required.
    pub fn execute_emergency_rollback(
        env: Env,
        proposal_id: u32,
    ) -> Result<(), Error> {
        let mut proposal: RollbackProposal = env
            .storage()
            .persistent()
            .get(&DataKey::GovRollbackProposal(proposal_id))
            .ok_or(Error::ProposalNotFound)?;
        if proposal.executed {
            return Err(Error::AlreadyExecuted);
        }
        if proposal.approval_count < EMERGENCY_THRESHOLD {
            return Err(Error::BelowThreshold);
        }

        let config: GovConfigSnapshot = env
            .storage()
            .persistent()
            .get(&DataKey::GovSnapshot(proposal.snapshot_id))
            .ok_or(Error::ProposalNotFound)?;

        // Restore governance config from snapshot
        env.storage()
            .instance()
            .set(&DataKey::Threshold, &config.threshold);
        env.storage()
            .instance()
            .set(&DataKey::SignerCount, &config.signer_count);
        env.storage()
            .instance()
            .set(&DataKey::ProposalCount, &config.proposal_count);

        // Re-apply old WASM
        env.deployer()
            .update_current_contract_wasm(proposal.old_wasm_hash.clone());

        proposal.executed = true;
        env.storage()
            .persistent()
            .set(&DataKey::GovRollbackProposal(proposal_id), &proposal);

        env.events().publish(
            (
                symbol_short!("DR"),
                symbol_short!("grb_exec"),
                proposal_id,
            ),
            (proposal.snapshot_id, proposal.old_wasm_hash),
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Disaster Recovery — View helpers
    // -----------------------------------------------------------------------

    /// Return governance snapshot metadata, or `None` if not found.
    pub fn get_gov_snapshot_meta(
        env: Env,
        snapshot_id: u32,
    ) -> Option<SnapshotMeta> {
        env.storage()
            .persistent()
            .get(&DataKey::GovSnapshotMeta(snapshot_id))
    }

    /// Return the ordered list of retained governance snapshot IDs.
    pub fn get_gov_snapshot_index(env: Env) -> Vec<u32> {
        env.storage()
            .persistent()
            .get(&DataKey::GovSnapshotIndex)
            .unwrap_or(Vec::new(&env))
    }

    /// Return a governance rollback proposal by ID.
    pub fn get_gov_rollback_proposal(
        env: Env,
        proposal_id: u32,
    ) -> Option<RollbackProposal> {
        env.storage()
            .persistent()
            .get(&DataKey::GovRollbackProposal(proposal_id))
    }

    /// Validate cryptographic rollback evidence digests (non-zero required).
    pub fn validate_rollback_evidence(
        env: Env,
        evidence_hash: BytesN<32>,
        incident_hash: BytesN<32>,
    ) -> Result<(), Error> {
        let justification = RollbackJustification {
            evidence_hash,
            incident_hash,
            description_hash: RollbackAuthorization::zero_hash(&env),
        };
        if RollbackAuthorization::validate_justification(&env, &justification) {
            Ok(())
        } else {
            Err(Error::InvalidEmergencySignatures)
        }
    }

    /// Validate that a snapshot timestamp is within the 24-hour rollback window.
    pub fn validate_rollback_scope(env: Env, snapshot_created_at: u64) -> Result<(), Error> {
        if RollbackAuthorization::validate_scope_window(env.ledger().timestamp(), snapshot_created_at)
        {
            Ok(())
        } else {
            Err(Error::InvalidEmergencySignatures)
        }
    }

    // -----------------------------------------------------------------------
    // Justice monitoring / dispute oversight (#justice-protection)
    // -----------------------------------------------------------------------

    /// Combine dispute-independence, evidence-authenticity, and
    /// arbitration-bias signals (as computed by the dispute-evidence
    /// contract) into a single audited justice-protection decision for
    /// `escrow_id`, recorded under multisig oversight. Caller must be a
    /// registered signer.
    pub fn oversee_dispute_resolution(
        env: Env,
        caller: Address,
        escrow_id: u64,
        independence: DisputeIndependenceFlag,
        evidence: EvidenceAuthenticity,
        bias: ArbitrationBiasFlag,
    ) -> Result<JusticeInterventionRecord, Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Signer(caller.clone()))
            .unwrap_or(false)
        {
            return Err(Error::NotSigner);
        }
        caller.require_auth();

        let record = compute_justice_intervention(
            &env,
            independence,
            evidence,
            bias,
            JUSTICE_RESTORATION_COOLDOWN_SECS,
        );
        let oversight = DisputeOversightRecord {
            escrow_id,
            reviewer: caller.clone(),
            justice_status: record.clone(),
            reviewed_at: env.ledger().timestamp(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::DisputeAudit(escrow_id), &oversight);
        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("dsp_audit"), escrow_id),
            (caller, record.intervene, record.combined_risk_score),
        );
        Ok(record)
    }

    /// Audit an arbitrator's recent ruling-favor history for systematic
    /// bias, recording the result under multisig oversight. Caller must be
    /// a registered signer.
    pub fn ensure_arbitration_fairness(
        env: Env,
        caller: Address,
        arbitrator: Address,
        favor_history: Vec<bool>,
    ) -> Result<ArbitrationBiasFlag, Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        if !env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::Signer(caller.clone()))
            .unwrap_or(false)
        {
            return Err(Error::NotSigner);
        }
        caller.require_auth();

        let flag = protect_arbitration_fairness(&favor_history);
        env.storage()
            .persistent()
            .set(&DataKey::ArbitratorAudit(arbitrator.clone()), &flag);
        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("bias_aud")),
            (arbitrator, flag.fair, flag.bias_risk_score),
        );
        Ok(flag)
    }

    /// Return the last recorded dispute-oversight audit for `escrow_id`.
    pub fn get_dispute_audit(env: Env, escrow_id: u64) -> Option<DisputeOversightRecord> {
        env.storage().persistent().get(&DataKey::DisputeAudit(escrow_id))
    }

    /// Return the last recorded arbitration-fairness audit for `arbitrator`.
    pub fn get_arbitrator_audit(env: Env, arbitrator: Address) -> Option<ArbitrationBiasFlag> {
        env.storage().persistent().get(&DataKey::ArbitratorAudit(arbitrator))
    }

    // =======================================================================
    // #868 — Key Management and Rotation
    // =======================================================================

    /// Register or rotate a signing key for a multisig signer.
    ///
    /// Uses hierarchical deterministic derivation so each signer can maintain
    /// forward secrecy. Only the signer themselves may register their own key.
    pub fn register_signer_key(
        env: Env,
        signer: Address,
        pubkey_commitment: BytesN<32>,
        derivation_path_hash: BytesN<32>,
    ) -> Result<KeyRecord, Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        if !env.storage().persistent()
            .get::<_, bool>(&DataKey::Signer(signer.clone()))
            .unwrap_or(false)
        {
            return Err(Error::NotSigner);
        }
        signer.require_auth();

        let record = register_key(
            &env,
            &signer,
            KeyScheme::Ed25519,
            pubkey_commitment,
            derivation_path_hash,
            true, // enable forward secrecy
        );

        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("key_reg")),
            (signer, record.version),
        );

        Ok(record)
    }

    /// Propose an automatic key rotation for a signer.
    ///
    /// The new key becomes active immediately upon `execute_signer_key_rotation`.
    /// The old key remains valid for `KEY_ROTATION_OVERLAP_SECS` (7 days).
    pub fn propose_signer_key_rotation(
        env: Env,
        signer: Address,
        new_pubkey_commitment: BytesN<32>,
        new_derivation_path_hash: BytesN<32>,
    ) -> Result<KeyRotationProposal, Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        if !env.storage().persistent()
            .get::<_, bool>(&DataKey::Signer(signer.clone()))
            .unwrap_or(false)
        {
            return Err(Error::NotSigner);
        }
        signer.require_auth();

        let proposal = propose_key_rotation(
            &env,
            &signer,
            KeyScheme::Ed25519,
            new_pubkey_commitment,
            new_derivation_path_hash,
        );

        Ok(proposal)
    }

    /// Execute a pending key rotation for a signer.
    pub fn execute_signer_key_rotation(
        env: Env,
        signer: Address,
    ) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        signer.require_auth();

        execute_key_rotation(&env, &signer);
        Ok(())
    }

    /// Emergency revoke a compromised signer key.
    ///
    /// Can be called by any registered signer (peer accountability).
    /// The affected signer cannot re-register for `REVOCATION_COOLDOWN_SECS`.
    pub fn emergency_revoke_signer_key(
        env: Env,
        caller: Address,
        compromised_signer: Address,
        key_version: u32,
        reason: Symbol,
    ) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Threshold) {
            return Err(Error::NotInitialized);
        }
        if !env.storage().persistent()
            .get::<_, bool>(&DataKey::Signer(caller.clone()))
            .unwrap_or(false)
        {
            return Err(Error::NotSigner);
        }
        caller.require_auth();

        if is_key_revoked(&env, &compromised_signer, key_version) {
            return Err(Error::AlreadyExecuted);
        }

        emergency_revoke_key(&env, &compromised_signer, key_version, reason.clone());

        env.events().publish(
            (symbol_short!("multisig"), symbol_short!("key_rev")),
            (caller, compromised_signer, key_version, reason),
        );

        Ok(())
    }

    /// Check whether a signer's current key is due for rotation.
    pub fn is_key_rotation_due(env: Env, signer: Address) -> bool {
        is_rotation_due(&env, &signer)
    }

    /// Get the current key record for a signer, if any.
    pub fn get_signer_key(env: Env, signer: Address) -> Option<KeyRecord> {
        get_current_key(&env, &signer)
    }

    // =======================================================================
    // #867 — Transaction Intent Verification
    // =======================================================================

    /// Evaluate the risk of a proposed multisig action before signing.
    ///
    /// Returns a `TransactionIntent` with risk level, cooling-off requirements,
    /// anomaly score, and whether the caller's account is blocked. Callers
    /// should check `account_blocked` and `requires_cooling_off` before
    /// proceeding with `propose_action` or `sign_action`.
    pub fn evaluate_action_risk(
        env: Env,
        caller: Address,
        operation: Symbol,
        amount: i128,
        is_new_target: bool,
    ) -> TransactionIntent {
        evaluate_transaction_intent(
            &env,
            &caller,
            operation,
            amount,
            is_new_target,
        )
    }

    /// Check whether an account is blocked due to suspicious activity.
    ///
    /// High-risk operations should call this before executing.
    pub fn is_account_blocked(env: Env, account: Address) -> bool {
        let state = get_protection_state(&env, &account);
        state.blocked
    }
}


// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn apply_add_signer(env: &Env, new_signer: Address) -> Result<(), Error> {
    if env.storage().persistent().get::<_, bool>(&DataKey::Signer(new_signer.clone())).unwrap_or(false) {
        return Err(Error::AlreadySigner);
    }
    env.storage().persistent().set(&DataKey::Signer(new_signer.clone()), &true);
    let count: u32 = env.storage().instance().get(&DataKey::SignerCount).unwrap_or(0);
    let new_count = count.checked_add(1).expect("Signer count overflow");
    env.storage().instance().set(&DataKey::SignerCount, &new_count);
    env.events().publish(
        (symbol_short!("multisig"), symbol_short!("sgn_added"), new_signer),
        new_count,
    );
    Ok(())
}

fn apply_remove_signer(env: &Env, signer: Address) -> Result<(), Error> {
    if !env.storage().persistent().get::<_, bool>(&DataKey::Signer(signer.clone())).unwrap_or(false) {
        return Err(Error::NotSigner);
    }
    let count: u32     = env.storage().instance().get(&DataKey::SignerCount).unwrap_or(0);
    let threshold: u32 = env.storage().instance().get(&DataKey::Threshold).unwrap_or(0);
    let new_count = count.checked_sub(1).expect("Signer count underflow");
    if new_count < threshold {
        return Err(Error::InvalidThreshold);
    }
    env.storage().persistent().remove(&DataKey::Signer(signer.clone()));
    env.storage().instance().set(&DataKey::SignerCount, &new_count);
    env.events().publish(
        (symbol_short!("multisig"), symbol_short!("sgn_rmvd"), signer),
        new_count,
    );
    Ok(())
}

/// Update the approval threshold via multi-sig proposal.
fn apply_update_threshold(env: &Env, new_threshold: u32) -> Result<(), Error> {
    if new_threshold == 0 {
        return Err(Error::InvalidThreshold);
    }
    let count: u32 = env.storage().instance().get(&DataKey::SignerCount).unwrap_or(0);
    if new_threshold > count {
        return Err(Error::InvalidThreshold);
    }
    let old_threshold: u32 = env.storage().instance().get(&DataKey::Threshold).unwrap_or(0);
    env.storage().instance().set(&DataKey::Threshold, &new_threshold);
    env.events().publish(
        (symbol_short!("multisig"), symbol_short!("thresh"), new_threshold),
        old_threshold,
    );
    Ok(())
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{vec, Env, IntoVal, Symbol};

    struct Fixture {
        env:      Env,
        contract: Address,
        signers:  [Address; 5],
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();
            env.ledger().set_timestamp(0);
            let s = core::array::from_fn::<Address, 5, _>(|_| Address::generate(&env));
            let contract = env.register_contract(None, MultisigAdminContract);
            MultisigAdminContractClient::new(&env, &contract)
                .initialize(
                    &vec![&env, s[0].clone(), s[1].clone(), s[2].clone(), s[3].clone(), s[4].clone()],
                    &3u32,
                );
            Fixture { env, contract, signers: s }
        }

        fn client(&self) -> MultisigAdminContractClient {
            MultisigAdminContractClient::new(&self.env, &self.contract)
        }

        fn propose(&self, target: &Address, function: &str) -> u32 {
            self.client().propose_action(
                &self.signers[0],
                target,
                &Symbol::new(&self.env, function),
                &vec![&self.env],
            )
        }

        fn sign_n(&self, pid: u32, n: usize) {
            for i in 1..n {
                self.client().sign_action(&self.signers[i], &pid);
            }
        }

        fn self_proposal(&self, function: &str, arg: &Address) -> u32 {
            self.client().propose_action(
                &self.signers[0],
                &self.contract,
                &Symbol::new(&self.env, function),
                &vec![&self.env, arg.clone().into_val(&self.env)],
            )
        }

        fn threshold_proposal(&self, new_threshold: u32) -> u32 {
            self.client().propose_action(
                &self.signers[0],
                &self.contract,
                &Symbol::new(&self.env, "update_threshold"),
                &vec![&self.env, new_threshold.into_val(&self.env)],
            )
        }
    }

    #[test]
    fn test_initialize() {
        let env = Env::default();
        env.mock_all_auths();
        let s0 = Address::generate(&env);
        let s1 = Address::generate(&env);
        let id = env.register_contract(None, MultisigAdminContract);
        let client = MultisigAdminContractClient::new(&env, &id);

        // threshold == 0
        assert_eq!(client.try_initialize(&vec![&env, s0.clone()], &0u32), Err(Ok(Error::InvalidThreshold)));
        // threshold > signer count
        assert_eq!(client.try_initialize(&vec![&env, s0.clone()], &2u32), Err(Ok(Error::InvalidThreshold)));
        // duplicate signer
        assert_eq!(client.try_initialize(&vec![&env, s0.clone(), s0.clone()], &1u32), Err(Ok(Error::AlreadySigner)));
        // happy path — 2-of-3 style
        client.initialize(&vec![&env, s0.clone(), s1.clone()], &1u32);
        assert_eq!(client.get_threshold(), 1u32);
        assert_eq!(client.get_signer_count(), 2u32);
        assert_eq!(client.is_signer(&s0), true);
        assert_eq!(client.is_signer(&s1), true);
        // double initialize
        assert_eq!(client.try_initialize(&vec![&env, s0.clone()], &1u32), Err(Ok(Error::AlreadyInitialized)));
    }

    #[test]
    fn test_propose_action() {
        let f = Fixture::setup();
        let target   = Address::generate(&f.env);
        let outsider = Address::generate(&f.env);

        assert_eq!(
            f.client().try_propose_action(&outsider, &target, &Symbol::new(&f.env, "do_thing"), &vec![&f.env]),
            Err(Ok(Error::NotSigner))
        );

        let pid1 = f.propose(&target, "do_thing");
        assert_eq!(pid1, 1u32);
        let pid2 = f.propose(&target, "do_thing");
        assert_eq!(pid2, 2u32);

        let proposal = f.client().get_proposal(&pid1);
        assert_eq!(proposal.approval_count, 1u32);
        assert_eq!(proposal.proposer, f.signers[0]);
        assert_eq!(proposal.executed, false);
        assert_eq!(proposal.cancelled, false);
        assert_eq!(proposal.expiry, EXPIRY_SECONDS);
    }

    #[test]
    fn test_sign_action() {
        let f = Fixture::setup();
        let target   = Address::generate(&f.env);
        let outsider = Address::generate(&f.env);
        let pid = f.propose(&target, "do_thing");

        assert_eq!(f.client().try_sign_action(&outsider, &pid), Err(Ok(Error::NotSigner)));

        f.client().sign_action(&f.signers[1], &pid);
        assert_eq!(f.client().get_proposal(&pid).approval_count, 2u32);

        assert_eq!(f.client().try_sign_action(&f.signers[1], &pid), Err(Ok(Error::AlreadySigned)));

        f.env.ledger().set_timestamp(EXPIRY_SECONDS + 1);
        assert_eq!(f.client().try_sign_action(&f.signers[2], &pid), Err(Ok(Error::Expired)));
    }

    #[test]
    fn test_execute_below_threshold() {
        let f = Fixture::setup();
        let target = Address::generate(&f.env);
        let pid = f.propose(&target, "do_thing");

        assert_eq!(f.client().try_execute_action(&pid), Err(Ok(Error::BelowThreshold)));

        f.client().sign_action(&f.signers[1], &pid);
        assert_eq!(f.client().try_execute_action(&pid), Err(Ok(Error::BelowThreshold)));
    }

    #[test]
    fn test_signer_management_via_proposal() {
        let f = Fixture::setup();
        let new_signer = Address::generate(&f.env);

        // add_signer
        let pid_add = f.self_proposal("add_signer", &new_signer);
        f.sign_n(pid_add, 3);
        f.client().execute_action(&pid_add);
        assert_eq!(f.client().get_signer_count(), 6u32);
        assert_eq!(f.client().is_signer(&new_signer), true);

        // remove_signer
        let pid_rem = f.self_proposal("remove_signer", &new_signer);
        f.sign_n(pid_rem, 3);
        f.client().execute_action(&pid_rem);
        assert_eq!(f.client().get_signer_count(), 5u32);
        assert_eq!(f.client().is_signer(&new_signer), false);

        // removal that would violate threshold is rejected
        let pid_r1 = f.self_proposal("remove_signer", &f.signers[3]);
        f.sign_n(pid_r1, 3);
        f.client().execute_action(&pid_r1); // 4 signers

        let pid_r2 = f.self_proposal("remove_signer", &f.signers[4]);
        f.sign_n(pid_r2, 3);
        f.client().execute_action(&pid_r2); // 3 signers == threshold

        let pid_r3 = f.self_proposal("remove_signer", &f.signers[2]);
        f.sign_n(pid_r3, 3);
        assert_eq!(f.client().try_execute_action(&pid_r3), Err(Ok(Error::InvalidThreshold)));
    }

    #[test]
    fn test_update_threshold_via_proposal() {
        let f = Fixture::setup();

        // Propose to change threshold from 3 to 2 (2-of-5)
        let pid = f.threshold_proposal(2u32);
        f.sign_n(pid, 3);
        f.client().execute_action(&pid);
        assert_eq!(f.client().get_threshold(), 2u32);

        // Propose to change threshold to 5 (5-of-5)
        let pid2 = f.threshold_proposal(5u32);
        // Only need 2 approvals now
        f.client().sign_action(&f.signers[1], &pid2);
        f.client().execute_action(&pid2);
        assert_eq!(f.client().get_threshold(), 5u32);

        // Threshold > signer count is rejected
        let pid3 = f.threshold_proposal(6u32);
        // Need 5 approvals now
        f.client().sign_action(&f.signers[1], &pid3);
        f.client().sign_action(&f.signers[2], &pid3);
        f.client().sign_action(&f.signers[3], &pid3);
        f.client().sign_action(&f.signers[4], &pid3);
        assert_eq!(f.client().try_execute_action(&pid3), Err(Ok(Error::InvalidThreshold)));
    }

    #[test]
    fn test_cancel_action() {
        let f = Fixture::setup();
        let target   = Address::generate(&f.env);
        let outsider = Address::generate(&f.env);
        let pid = f.propose(&target, "do_thing");

        assert_eq!(f.client().try_cancel_action(&outsider, &pid), Err(Ok(Error::NotSigner)));

        f.client().cancel_action(&f.signers[1], &pid);
        assert_eq!(f.client().get_proposal(&pid).cancelled, true);

        assert_eq!(f.client().try_cancel_action(&f.signers[0], &pid), Err(Ok(Error::Cancelled)));
    }

    #[test]
    fn test_three_of_five_threshold() {
        // Verify 3-of-5 configuration works end-to-end
        let f = Fixture::setup(); // initialized with 5 signers, threshold 3
        assert_eq!(f.client().get_threshold(), 3u32);
        assert_eq!(f.client().get_signer_count(), 5u32);

        let target = Address::generate(&f.env);
        let pid = f.propose(&target, "admin_action");
        // 1 approval — not enough
        assert_eq!(f.client().try_execute_action(&pid), Err(Ok(Error::BelowThreshold)));
        // 2 approvals — not enough
        f.client().sign_action(&f.signers[1], &pid);
        assert_eq!(f.client().try_execute_action(&pid), Err(Ok(Error::BelowThreshold)));
        // 3 approvals — meets threshold; external call will panic (no contract at target)
        // so we just verify the threshold check passes by checking BelowThreshold is NOT returned
        f.client().sign_action(&f.signers[2], &pid);
        let result = f.client().try_execute_action(&pid);
        assert_ne!(result, Err(Ok(Error::BelowThreshold)));
    }
}

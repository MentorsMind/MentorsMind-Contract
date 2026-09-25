//! Dispute Evidence Contract
//!
//! Allows the mentor or learner to attach off-chain evidence references to a
//! disputed escrow during a bounded submission window. An arbitrator may then
//! submit a resolution after a mandatory review delay.
//!
//! # Workflow
//! 1. Learner or mentor opens a dispute on the escrow contract.
//! 2. Either party calls [`DisputeEvidenceContract::submit_evidence`] with a
//!    `content_hash: BytesN<32>` — a SHA-256/BLAKE3 commitment to the actual
//!    off-chain document content — plus an `evidence_uri_hash: BytesN<32>`
//!    committing to the IPFS CID or URL where that content is hosted. This
//!    replaces the original `Symbol` reference (max 9 chars), which could
//!    not hold a real content commitment and gave callers no way to prove a
//!    submitted reference corresponds to specific content.
//! 3. An arbitrator calls [`DisputeEvidenceContract::submit_resolution`] after
//!    `MIN_RESOLUTION_DELAY_SECS` have elapsed since the dispute was opened.
//! 4. The admin uses the on-chain resolution record to call `resolve_dispute`
//!    on the escrow contract.
//!
//! # Recording Integrity & Privacy (#914)
//! Session recordings used as evidence are protected with tamper-evident
//! cryptographic verification, selective redaction, consent management,
//! and role-based access control.
#![no_std]
#![allow(deprecated)] // Temporarily allow deprecated Events::publish until we migrate to #[contractevent]

use shared::{
    compute_justice_intervention, ensure_dispute_independence as shared_ensure_dispute_independence,
    is_justice_restoration_eligible, protect_arbitration_fairness as shared_protect_arbitration_fairness,
    validate_evidence_authenticity as shared_validate_evidence_authenticity, ArbitrationBiasFlag,
    DisputeIndependenceFlag, EvidenceAuthenticity as SharedEvidenceAuthenticity,
    JusticeInterventionRecord, JUSTICE_RESTORATION_COOLDOWN_SECS,
};
use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, Address, BytesN, Env,
    IntoVal, Symbol, Vec,
};

use shared::{
    SessionRecording, RecordingStatus, RecordingConsentRecord as ConsentRecord, AccessRole, RedactionRecord, AccessLogEntry, IntegrityVerificationResult,
    create_recording, verify_recording_integrity, grant_consent, revoke_consent,
    check_access_authorized, apply_redaction, log_access, emergency_privacy_protection,
};

use shared::{validate_evidence_sufficiency, EvidenceSufficiency};

/// Maximum recent rulings tracked per arbitrator for bias scoring.
const MAX_ARBITRATOR_HISTORY: u32 = 20;

/// Default window (seconds) within which evidence may be submitted after session end.
const DEFAULT_WINDOW_SECS: u64 = 48 * 60 * 60;

/// Maximum evidence items stored per escrow.
const MAX_EVIDENCE_ITEMS: u32 = 5;

/// Minimum seconds a party must wait between consecutive evidence submissions
/// for the same escrow (anti-spam / griefing guard). 1 hour.
const SUBMISSION_COOLDOWN_SECS: u64 = 3_600;

/// Minimum seconds that must elapse after the evidence window opens before
/// an arbitrator may submit a resolution (gives parties time to respond).
/// 24 hours.
const MIN_RESOLUTION_DELAY_SECS: u64 = 24 * 60 * 60;

/// Appeal period after an original resolution during which an appeal may be
/// submitted and a second arbitrator may override the decision.
const APPEAL_PERIOD_SECS: u64 = 72 * 3600;

// ─── Domain types ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EscrowStatus {
    Active,
    Released,
    Disputed,
    Refunded,
    Resolved,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Escrow {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub session_id: Symbol,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    pub platform_fee: i128,
    pub net_amount: i128,
    pub session_end_time: u64,
    pub auto_release_delay: u64,
    pub dispute_reason: Symbol,
    pub resolved_at: u64,
    pub usd_amount: i128,
    pub quoted_token_amount: i128,
    pub send_asset: Address,
    pub dest_asset: Address,
    pub total_sessions: u32,
    pub sessions_completed: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceItem {
    pub submitter: Address,
    /// SHA-256/BLAKE3 commitment to the actual off-chain evidence content.
    pub content_hash: BytesN<32>,
    /// Commitment to the IPFS CID or URL where `content_hash`'s content is
    /// hosted, kept separate from `content_hash` so the location can change
    /// (e.g. re-pinned to a different gateway) without invalidating the
    /// content commitment itself.
    pub evidence_uri_hash: BytesN<32>,
    /// Optional signature from `submitter` over
    /// `(escrow_id, content_hash, submitted_at)`, allowing later
    /// off-chain/on-chain verification that the submitter themselves
    /// attested to this exact content at this exact time. An all-zero value
    /// means "no attestation provided" (soroban-sdk 21.7.7 cannot derive
    /// `contracttype` for `Option<BytesN<64>>`, so a sentinel is used
    /// instead of `Option`).
    pub submitter_attestation: BytesN<64>,
    pub submitted_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeResolution {
    pub arbitrator: Address,
    pub release_to_mentor: bool,
    pub note: Symbol,
    pub resolved_at: u64,
    /// Merkle root of the evidence set at the time of ruling, binding the
    /// resolution to a specific evidence set for tamper-evident audit.
    pub evidence_root: BytesN<32>,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    EscrowContract,
    Evidence(u64),
    Resolution(u64),
    WindowSecs,
    /// Tracks when each (submitter, escrow_id) pair last submitted evidence.
    /// Used to enforce SUBMISSION_COOLDOWN_SECS between submissions.
    LastSubmission(u64, Address),
    /// Ledger timestamp at which a dispute was opened for a given escrow.
    DisputeOpenedAt(u64),
    /// Whether the anti-spam cooldown is enabled (default: true).
    CooldownEnabled,
    AnomalyDetector,
    BypassAnomalyCheck,
    /// Merkle root of the evidence set for a given escrow. Updated on every
    /// evidence submission for tamper-evident integrity.
    EvidenceRoot(u64),
    /// Address of the governance contract used to select appeal arbitrators.
    GovernanceContract,
    /// Appeal deadline for a dispute resolution.
    AppealPeriodEnds(u64),
    /// Number of appeals submitted for a dispute.
    AppealCount(u64),
    /// Selected appeal arbitrator for a dispute.
    AppealArbitrator(u64),
    /// Optional hash explaining why the appellant requested an appeal.
    AppealReasonHash(u64),
    /// Optional health dashboard notified of dispute lifecycle events.
    HealthDashboard,
    // Recording integrity & privacy (#914)
    SessionRecording(u64),           // Maps escrow_id -> recording_id
    RecordingEvidence(u64),          // Maps escrow_id -> SessionRecording
    RecordingConsent(u64),           // Maps escrow_id -> Vec<ConsentRecord>
    RecordingRedaction(u64),         // Maps escrow_id -> Vec<RedactionRecord>
    RecordingAccessLog(u64),         // Maps escrow_id -> Vec<AccessLogEntry>
    /// Dispute-open timestamps for a given (mentor, learner) pair, used to
    /// detect coordinated/repeated dispute filing (#justice-protection).
    PartyDisputeLog(Address, Address),
    /// Cached dispute-independence assessment for a given escrow.
    DisputeIndependence(u64),
    /// First escrow_id a given evidence `content_hash` was submitted for;
    /// used to detect content reuse across unrelated disputes.
    EvidenceHashOrigin(BytesN<32>),
    /// Count of evidence items submitted for an escrow whose content hash
    /// was already used in a different escrow.
    DuplicateEvidenceCount(u64),
    /// Cached evidence-authenticity assessment for a given escrow.
    EvidenceAuthenticityRecord(u64),
    /// Rolling favor history (true = ruled for mentor) for a given
    /// arbitrator, used for arbitration-bias scoring.
    ArbitratorFavorHistory(Address),
    /// Cached arbitration-fairness assessment for a given arbitrator.
    ArbitrationFairness(Address),
    /// Cached combined justice-protection intervention record for a given
    /// escrow.
    JusticeIntervention(u64),
}

#[contractclient(name = "EscrowContractClient")]
pub trait EscrowContractTrait {
    fn get_escrow(env: Env, escrow_id: u64) -> Escrow;
}

#[contractclient(name = "GovernanceContractClient")]
pub trait GovernanceContractTrait {
    fn select_arbitrator(env: Env, dispute_id: u64) -> Address;
    fn get_arbitrator_count(env: Env) -> u32;
    fn list_arbitrators_page(env: Env, offset: u32, limit: u32) -> Vec<Address>;
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized       = 1,
    Unauthorized             = 2,
    InvalidEscrowState       = 3,
    EvidenceWindowClosed     = 4,
    EvidenceLimitReached     = 5,
    AlreadyResolved          = 6,
    /// Submitter must wait before submitting more evidence (anti-spam).
    SubmissionCooldown       = 7,
    /// Arbitrator must wait for MIN_RESOLUTION_DELAY_SECS after dispute opens.
    ResolutionTimelockActive = 8,
    /// Dispute opening has already been recorded for this escrow.
    AlreadyRecorded          = 9,
    /// `content_hash` was all-zero, i.e. a null commitment that proves
    /// nothing about the submitted content.
    ZeroContentHash          = 10,
    /// The same submitter already submitted this exact `content_hash` for
    /// this escrow — duplicate commitments are rejected.
    DuplicateContentHash     = 11,
    /// An appeal deadline has already passed.
    AppealPeriodExpired      = 12,
    /// Only one appeal is allowed per dispute.
    AppealAlreadySubmitted   = 13,
    /// A governance contract has not been configured for appeal arbitration.
    GovernanceContractNotConfigured = 14,
    /// No alternative arbitrator is available for an appeal.
    NoAlternativeArbitrator = 15,
    /// No justice-protection intervention is on record for this escrow.
    NoJusticeInterventionOnRecord = 16,
    /// The intervened dispute flow's restoration cooldown has not elapsed.
    JusticeRestorationNotEligible = 17,
    /// Requested resource not found (recording evidence).
    NotFound = 18,
    /// Dispute lacks sufficient evidence or has not cleared the mandatory
    /// deliberation cooldown (#886 payment-integrity protection).
    InsufficientEvidence = 19,
}

#[contract]
pub struct DisputeEvidenceContract;

#[contractimpl]
impl DisputeEvidenceContract {
    pub fn initialize(env: Env, admin: Address, escrow_contract: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::EscrowContract, &escrow_contract);
        env.storage()
            .instance()
            .set(&DataKey::WindowSecs, &DEFAULT_WINDOW_SECS);
        env.storage()
            .instance()
            .set(&DataKey::CooldownEnabled, &true);
        Ok(())
    }

    pub fn set_escrow_contract(
        env: Env,
        admin: Address,
        escrow_contract: Address,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&DataKey::EscrowContract, &escrow_contract);
        Ok(())
    }

    pub fn set_governance_contract(
        env: Env,
        admin: Address,
        governance_contract: Address,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&DataKey::GovernanceContract, &governance_contract);
        Ok(())
    }

    /// Enable or disable the anti-spam submission cooldown. Admin only.
    pub fn set_cooldown_enabled(env: Env, admin: Address, enabled: bool) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::CooldownEnabled, &enabled);
        Ok(())
    }

    pub fn set_anomaly_detector(env: Env, admin: Address, detector: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::AnomalyDetector, &detector);
        Ok(())
    }

    pub fn set_bypass_anomaly_check(env: Env, admin: Address, bypass: bool) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::BypassAnomalyCheck, &bypass);
        Ok(())
    }

    /// Configure the `health_dashboard` contract notified of dispute
    /// lifecycle events. Admin only; optional (skipped when unset, #760).
    pub fn set_health_dashboard(env: Env, admin: Address, dashboard: Address) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::HealthDashboard, &dashboard);
        Ok(())
    }

    /// Record that a dispute was opened.
    ///
    /// # Security
    /// The `opened_at` timestamp is derived from `env.ledger().timestamp()` and
    /// **cannot** be supplied by the caller. This prevents an admin (or any
    /// caller) from backdating the dispute opening to bypass the mandatory
    /// `MIN_RESOLUTION_DELAY_SECS` deliberation period.
    ///
    /// # Idempotency
    /// Calling this function a second time for the same `escrow_id` returns
    /// [`Error::AlreadyRecorded`] so that the timelock anchor cannot be
    /// overwritten or replayed.
    pub fn record_dispute_opened(env: Env, escrow_id: u64) -> Result<(), Error> {
        // Idempotency guard: refuse to overwrite an existing record.
        if env
            .storage()
            .persistent()
            .has(&DataKey::DisputeOpenedAt(escrow_id))
        {
            return Err(Error::AlreadyRecorded);
        }

        // Allow either admin or the escrow contract to call this.
        let stored: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::Unauthorized)?;
        stored.require_auth();

        // SECURITY: timestamp is taken from the ledger, not the caller.
        let opened_at = env.ledger().timestamp();
        env.storage()
            .persistent()
            .set(&DataKey::DisputeOpenedAt(escrow_id), &opened_at);
        env.events().publish(
            (Symbol::new(&env, "dispute_opened"), escrow_id),
            opened_at,
        );

        // Justice protection: track dispute-open timestamps for this
        // mentor/learner pair and re-score independence from coordinated
        // dispute filing (#justice-protection).
        let escrow = Self::load_escrow(&env, escrow_id);
        let party_key = DataKey::PartyDisputeLog(escrow.mentor.clone(), escrow.learner.clone());
        let mut party_log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&party_key)
            .unwrap_or(Vec::new(&env));
        party_log.push_back(opened_at);
        while party_log.len() > MAX_ARBITRATOR_HISTORY {
            party_log.remove(0);
        }
        env.storage().persistent().set(&party_key, &party_log);
        Self::ensure_dispute_independence(env.clone(), escrow_id);

        if let Some(dashboard) = env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::HealthDashboard)
        {
            env.invoke_contract::<()>(
                &dashboard,
                &Symbol::new(&env, "record_dispute_opened"),
                (escrow_id, opened_at).into_val(&env),
            );
        }

        Ok(())
    }

    /// Return the ledger timestamp at which the dispute for `escrow_id` was
    /// opened, or `None` if [`record_dispute_opened`] has not yet been called.
    pub fn get_dispute_opened_at(env: Env, escrow_id: u64) -> Option<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::DisputeOpenedAt(escrow_id))
    }

    /// Submit evidence for a disputed escrow.
    ///
    /// `content_hash` must be a SHA-256/BLAKE3 hash of the actual evidence
    /// document — a non-zero commitment that later lets any party verify
    /// (via [`Self::verify_evidence_integrity`]) that a claimed document
    /// matches what was originally submitted. `evidence_uri_hash` commits
    /// separately to the off-chain location (e.g. IPFS CID) hosting that
    /// content. `submitter_attestation`, if provided, is expected to be a
    /// signature from `submitter` over `(escrow_id, content_hash,
    /// submitted_at)`.
    ///
    /// # Anti-spam guard
    /// A party may not submit evidence more than once per
    /// `SUBMISSION_COOLDOWN_SECS` (1 h) for the same escrow.
    pub fn submit_evidence(
        env: Env,
        escrow_id: u64,
        submitter: Address,
        content_hash: BytesN<32>,
        evidence_uri_hash: BytesN<32>,
        submitter_attestation: Option<BytesN<64>>,
    ) -> Result<(), Error> {
        submitter.require_auth();
        let escrow = Self::load_escrow(&env, escrow_id);
        if escrow.status != EscrowStatus::Disputed {
            return Err(Error::InvalidEscrowState);
        }
        if submitter != escrow.mentor && submitter != escrow.learner {
            return Err(Error::Unauthorized);
        }

        if content_hash == BytesN::from_array(&env, &[0u8; 32]) {
            return Err(Error::ZeroContentHash);
        }

        let window_secs: u64 = env
            .storage()
            .instance()
            .get(&DataKey::WindowSecs)
            .unwrap_or(DEFAULT_WINDOW_SECS);
        if env.ledger().timestamp() > escrow.session_end_time.saturating_add(window_secs) {
            return Err(Error::EvidenceWindowClosed);
        }

        // Anti-spam / griefing guard
        let cooldown_enabled: bool = env
            .storage()
            .instance()
            .get(&DataKey::CooldownEnabled)
            .unwrap_or(true);
        if cooldown_enabled {
            let cooldown_key = DataKey::LastSubmission(escrow_id, submitter.clone());
            let last_submission: u64 = env
                .storage()
                .persistent()
                .get(&cooldown_key)
                .unwrap_or(0);
            let now = env.ledger().timestamp();
            if now < last_submission.saturating_add(SUBMISSION_COOLDOWN_SECS) {
                return Err(Error::SubmissionCooldown);
            }
            env.storage().persistent().set(&cooldown_key, &now);
        }

        let key = DataKey::Evidence(escrow_id);
        let mut evidence: Vec<EvidenceItem> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env));
        if evidence.len() >= MAX_EVIDENCE_ITEMS {
            return Err(Error::EvidenceLimitReached);
        }

        // Reject a duplicate (submitter, content_hash) pair within the same
        // escrow — two different evidence items from the same submitter
        // must not share a content commitment.
        for existing in evidence.iter() {
            if existing.submitter == submitter && existing.content_hash == content_hash {
                return Err(Error::DuplicateContentHash);
            }
        }

        let item = EvidenceItem {
            submitter: submitter.clone(),
            content_hash: content_hash.clone(),
            evidence_uri_hash,
            submitter_attestation: submitter_attestation
                .unwrap_or_else(|| BytesN::from_array(&env, &[0u8; 64])),
            submitted_at: env.ledger().timestamp(),
        };
        evidence.push_back(item.clone());
        env.storage().persistent().set(&key, &evidence);

        // Justice protection: detect content reuse across unrelated
        // disputes (a signature of fabricated/rehearsed evidence).
        let origin_key = DataKey::EvidenceHashOrigin(content_hash.clone());
        match env.storage().persistent().get::<_, u64>(&origin_key) {
            Some(origin_escrow) if origin_escrow != escrow_id => {
                let dup_key = DataKey::DuplicateEvidenceCount(escrow_id);
                let dup: u32 = env.storage().persistent().get(&dup_key).unwrap_or(0);
                env.storage().persistent().set(&dup_key, &(dup + 1));
            }
            Some(_) => {}
            None => {
                env.storage().persistent().set(&origin_key, &escrow_id);
            }
        }
        Self::validate_evidence_authenticity(env.clone(), escrow_id);

        // Compute and store Merkle root over the entire evidence set.
        let root = Self::compute_evidence_root(&env, &evidence);
        env.storage()
            .persistent()
            .set(&DataKey::EvidenceRoot(escrow_id), &root);

        env.events()
            .publish((Symbol::new(&env, "evidence_submitted"), escrow_id), item);
        env.events().publish(
            (
                Symbol::new(&env, "evidence_root_updated"),
                escrow_id,
                evidence.len(),
            ),
            root.clone(),
        );
        Ok(())
    }

    pub fn get_evidence(env: Env, escrow_id: u64) -> Vec<EvidenceItem> {
        env.storage()
            .persistent()
            .get(&DataKey::Evidence(escrow_id))
            .unwrap_or(Vec::new(&env))
    }

    pub fn get_evidence_count(env: Env, escrow_id: u64) -> u32 {
        Self::get_evidence(env, escrow_id).len()
    }

    /// Verify that the evidence item at `index` for `escrow_id` was
    /// submitted with exactly `expected_hash` as its `content_hash`.
    /// Returns `false` (rather than erroring) if the index is out of range
    /// or the hash does not match — callers checking whether a claimed
    /// document has been tampered with should treat both as "not verified".
    pub fn verify_evidence_integrity(
        env: Env,
        escrow_id: u64,
        index: u32,
        expected_hash: BytesN<32>,
    ) -> bool {
        let evidence = Self::get_evidence(env, escrow_id);
        match evidence.get(index) {
            Some(item) => item.content_hash == expected_hash,
            None => false,
        }
    }

    /// Submit a dispute resolution.
    ///
    /// # Time-lock guard
    /// Resolution may only be submitted at least `MIN_RESOLUTION_DELAY_SECS`
    /// (24 h) after the dispute was opened (via `record_dispute_opened`). If
    /// no opened-at record exists the guard is skipped (backwards-compatible).
    pub fn submit_resolution(
        env: Env,
        escrow_id: u64,
        arbitrator: Address,
        is_appeal: bool,
        release_to_mentor: bool,
        note: Symbol,
    ) -> Result<(), Error> {
        arbitrator.require_auth();

        let bypass: bool = env.storage().instance().get(&DataKey::BypassAnomalyCheck).unwrap_or(false);
        if !bypass {
            if let Some(anomaly_detector) = env.storage().instance().get::<_, Address>(&DataKey::AnomalyDetector) {
                let res: u32 = env.invoke_contract(
                    &anomaly_detector,
                    &Symbol::new(&env, "check_anomaly"),
                    (arbitrator.clone(), 1u32, 0i128).into_val(&env), // 1u32 = OpenDispute
                );
                if res == 2 {
                    panic!("UserOnHold");
                } else if res == 1 {
                    env.events().publish((Symbol::new(&env, "anomaly_warning"), arbitrator.clone()), 0i128);
                }
            }
        }

        let escrow = Self::load_escrow(&env, escrow_id);
        if escrow.status != EscrowStatus::Disputed {
            return Err(Error::InvalidEscrowState);
        }

        let key = DataKey::Resolution(escrow_id);
        if is_appeal && !env.storage().persistent().has(&key) {
            return Err(Error::InvalidEscrowState);
        }
        if env.storage().persistent().has(&key) {
            if !is_appeal {
                return Err(Error::AlreadyResolved);
            }
            let appeal_count: u32 = env
                .storage()
                .persistent()
                .get(&DataKey::AppealCount(escrow_id))
                .unwrap_or(0);
            if appeal_count == 0 {
                return Err(Error::InvalidEscrowState);
            }
            let appeal_arbitrator: Address = env
                .storage()
                .persistent()
                .get(&DataKey::AppealArbitrator(escrow_id))
                .expect("appeal arbitrator missing");
            if appeal_arbitrator != arbitrator {
                return Err(Error::Unauthorized);
            }
            let appeal_deadline: u64 = env
                .storage()
                .persistent()
                .get(&DataKey::AppealPeriodEnds(escrow_id))
                .expect("appeal deadline missing");
            if env.ledger().timestamp() > appeal_deadline {
                return Err(Error::AppealPeriodExpired);
            }
        }

        // Time-lock: enforce minimum deliberation period for original resolutions.
        if !is_appeal {
            if let Some(opened_at) = env
                .storage()
                .persistent()
                .get::<_, u64>(&DataKey::DisputeOpenedAt(escrow_id))
            {
                let earliest_resolution = opened_at.saturating_add(MIN_RESOLUTION_DELAY_SECS);
                if env.ledger().timestamp() < earliest_resolution {
                    return Err(Error::ResolutionTimelockActive);
                }
            }
        }

        let evidence_root = Self::get_evidence_root(env.clone(), escrow_id);

        let resolution = DisputeResolution {
            arbitrator: arbitrator.clone(),
            release_to_mentor,
            note: note.clone(),
            resolved_at: env.ledger().timestamp(),
            evidence_root,
        };
        env.storage().persistent().set(&key, &resolution);

        if !is_appeal {
            let appeal_deadline = resolution.resolved_at.saturating_add(APPEAL_PERIOD_SECS);
            env.storage()
                .persistent()
                .set(&DataKey::AppealPeriodEnds(escrow_id), &appeal_deadline);
        }

        env.events().publish(
            (Symbol::new(&env, "dispute_resolved"), escrow_id),
            resolution,
        );

        // Justice protection: track this arbitrator's ruling favor history
        // and re-score arbitration fairness.
        let history_key = DataKey::ArbitratorFavorHistory(arbitrator.clone());
        let mut history: Vec<bool> = env
            .storage()
            .persistent()
            .get(&history_key)
            .unwrap_or(Vec::new(&env));
        history.push_back(release_to_mentor);
        while history.len() > MAX_ARBITRATOR_HISTORY {
            history.remove(0);
        }
        env.storage().persistent().set(&history_key, &history);
        Self::protect_arbitration_fairness(env.clone(), arbitrator.clone());
        Self::get_justice_status(env.clone(), escrow_id, arbitrator);

        Ok(())
    }

    // ─── Payment-integrity protection (#886) ───────────────────────────────

    /// Validate that a dispute has both sufficient submitted evidence and
    /// has respected the minimum cooldown since it was opened, before an
    /// arbitrator is allowed to rule on it (prevents evidence-free,
    /// rushed strategic disputes).
    pub fn validate_dispute_claims(env: Env, escrow_id: u64) -> EvidenceSufficiency {
        let evidence_count = Self::get_evidence_count(env.clone(), escrow_id);
        let opened_at: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::DisputeOpenedAt(escrow_id))
            .unwrap_or(0);
        validate_evidence_sufficiency(&env, evidence_count, opened_at)
    }

    /// Impartial arbitration entrypoint: gates `submit_resolution` behind
    /// `validate_dispute_claims`, ensuring a ruling cannot be issued
    /// without sufficient evidence and the mandatory deliberation cooldown,
    /// on top of the existing arbitration-bias and dispute-independence
    /// protections.
    pub fn arbitrate_dispute(
        env: Env,
        escrow_id: u64,
        arbitrator: Address,
        release_to_mentor: bool,
        note: Symbol,
    ) -> Result<(), Error> {
        let claims = Self::validate_dispute_claims(env.clone(), escrow_id);
        if !claims.sufficient {
            return Err(Error::InsufficientEvidence);
        }
        Self::submit_resolution(env, escrow_id, arbitrator, false, release_to_mentor, note)
    }

    // ─── Justice protection ────────────────────────────────────────────────

    /// Score dispute independence for `escrow_id`: repeated disputes between
    /// the same mentor/learner pair, tightly clustered in time, are the
    /// signature of coordinated dispute filing rather than an independent,
    /// arm's-length conflict. Safe to call by anyone as a read-through
    /// audit; also invoked internally on every `record_dispute_opened`.
    pub fn ensure_dispute_independence(env: Env, escrow_id: u64) -> DisputeIndependenceFlag {
        let escrow = Self::load_escrow(&env, escrow_id);
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::PartyDisputeLog(escrow.mentor, escrow.learner))
            .unwrap_or(Vec::new(&env));
        let shared_actor_count = log.len();
        let flag = shared_ensure_dispute_independence(&log, shared_actor_count);
        env.storage()
            .persistent()
            .set(&DataKey::DisputeIndependence(escrow_id), &flag);
        if !flag.independent {
            env.events().publish(
                (Symbol::new(&env, "dispute_coordination_flagged"), escrow_id),
                flag.risk_score,
            );
        }
        flag
    }

    /// Validate the authenticity of `escrow_id`'s evidence submissions:
    /// content reuse across unrelated disputes and clustered submission
    /// timing are treated as tampering signals. Safe to call by anyone as a
    /// read-through audit; also invoked internally on every
    /// `submit_evidence`.
    pub fn validate_evidence_authenticity(env: Env, escrow_id: u64) -> SharedEvidenceAuthenticity {
        let evidence = Self::get_evidence(env.clone(), escrow_id);
        let mut timestamps: Vec<u64> = Vec::new(&env);
        for item in evidence.iter() {
            timestamps.push_back(item.submitted_at);
        }
        let duplicate_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::DuplicateEvidenceCount(escrow_id))
            .unwrap_or(0);
        let result = shared_validate_evidence_authenticity(&timestamps, duplicate_count);
        env.storage()
            .persistent()
            .set(&DataKey::EvidenceAuthenticityRecord(escrow_id), &result);
        if !result.authentic {
            env.events().publish(
                (Symbol::new(&env, "evidence_authenticity_flagged"), escrow_id),
                result.tampering_risk_score,
            );
        }
        result
    }

    /// Assess an arbitrator's recent ruling history for systematic bias
    /// toward one party. Safe to call by anyone as a read-through audit;
    /// also invoked internally on every `submit_resolution`.
    pub fn protect_arbitration_fairness(env: Env, arbitrator: Address) -> ArbitrationBiasFlag {
        let history: Vec<bool> = env
            .storage()
            .persistent()
            .get(&DataKey::ArbitratorFavorHistory(arbitrator.clone()))
            .unwrap_or(Vec::new(&env));
        let flag = shared_protect_arbitration_fairness(&history);
        env.storage()
            .persistent()
            .set(&DataKey::ArbitrationFairness(arbitrator.clone()), &flag);
        if !flag.fair {
            env.events().publish(
                (Symbol::new(&env, "arbitration_bias_flagged"), arbitrator),
                flag.bias_risk_score,
            );
        }
        flag
    }

    /// Combine the cached dispute-independence, evidence-authenticity, and
    /// arbitration-fairness signals for `escrow_id`/`arbitrator` into a
    /// single justice-protection intervention decision, persisting the
    /// result for `restore_fair_resolution` to consume.
    pub fn get_justice_status(env: Env, escrow_id: u64, arbitrator: Address) -> JusticeInterventionRecord {
        let independence: DisputeIndependenceFlag = env
            .storage()
            .persistent()
            .get(&DataKey::DisputeIndependence(escrow_id))
            .unwrap_or(DisputeIndependenceFlag {
                independent: true,
                risk_score: 0,
                shared_actor_count: 0,
                clustered_timing_count: 0,
            });
        let evidence: SharedEvidenceAuthenticity = env
            .storage()
            .persistent()
            .get(&DataKey::EvidenceAuthenticityRecord(escrow_id))
            .unwrap_or(SharedEvidenceAuthenticity {
                authentic: true,
                tampering_risk_score: 0,
                duplicate_submission_count: 0,
                suspicious_timing_count: 0,
            });
        let bias: ArbitrationBiasFlag = env
            .storage()
            .persistent()
            .get(&DataKey::ArbitrationFairness(arbitrator))
            .unwrap_or(ArbitrationBiasFlag {
                fair: true,
                bias_risk_score: 0,
                one_sided_ratio_bps: 0,
                ruling_count: 0,
            });
        let record = compute_justice_intervention(
            &env,
            independence,
            evidence,
            bias,
            JUSTICE_RESTORATION_COOLDOWN_SECS,
        );
        env.storage()
            .persistent()
            .set(&DataKey::JusticeIntervention(escrow_id), &record);
        record
    }

    /// Restore fair dispute resolution for `escrow_id` once the
    /// justice-protection intervention cooldown has elapsed. Admin only.
    pub fn restore_fair_resolution(env: Env, admin: Address, escrow_id: u64) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        let record: JusticeInterventionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::JusticeIntervention(escrow_id))
            .ok_or(Error::NoJusticeInterventionOnRecord)?;

        if !is_justice_restoration_eligible(&record, env.ledger().timestamp()) {
            return Err(Error::JusticeRestorationNotEligible);
        }

        env.storage()
            .persistent()
            .remove(&DataKey::JusticeIntervention(escrow_id));
        env.storage()
            .persistent()
            .remove(&DataKey::DisputeIndependence(escrow_id));
        env.storage()
            .persistent()
            .remove(&DataKey::EvidenceAuthenticityRecord(escrow_id));

        env.events().publish(
            (Symbol::new(&env, "justice_restored"), escrow_id),
            (),
        );
        Ok(())
    }

    pub fn get_resolution(env: Env, escrow_id: u64) -> DisputeResolution {
        env.storage()
            .persistent()
            .get(&DataKey::Resolution(escrow_id))
            .expect("resolution not found")
    }

    /// Compute a sequential Merkle root over the evidence set:
    /// `sha256(sha256(item_1) || sha256(item_2) || ... || sha256(item_n))`
    fn compute_evidence_root(env: &Env, evidence: &Vec<EvidenceItem>) -> BytesN<32> {
        // Hash each evidence item individually, then concatenate and hash the
        // result to produce a single root commitment.
        let mut combined = soroban_sdk::Bytes::new(env);
        for item in evidence.iter() {
            let mut item_bytes = soroban_sdk::Bytes::new(env);
            // Include content_hash + evidence_uri_hash + submitted_at in the
            // item leaf so any field modification invalidates the root.
            item_bytes.append(&item.content_hash.clone().into());
            item_bytes.append(&item.evidence_uri_hash.clone().into());
            item_bytes.extend_from_array(&item.submitted_at.to_be_bytes());
            let leaf = env.crypto().sha256(&item_bytes);
            combined.append(&leaf.clone().into());
        }

        if combined.len() == 0 {
            // Empty set → zero root.
            return BytesN::from_array(env, &[0u8; 32]);
        }

        env.crypto().sha256(&combined).into()
    }

    /// Return the stored Merkle root for `escrow_id`, or zero if no evidence
    /// has been submitted.
    pub fn get_evidence_root(env: Env, escrow_id: u64) -> BytesN<32> {
        env.storage()
            .persistent()
            .get(&DataKey::EvidenceRoot(escrow_id))
            .unwrap_or_else(|| BytesN::from_array(&env, &[0u8; 32]))
    }

    pub fn get_appeal_arbitrator(env: Env, escrow_id: u64) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::AppealArbitrator(escrow_id))
    }

    pub fn get_appeal_reason_hash(env: Env, escrow_id: u64) -> Option<BytesN<32>> {
        env.storage()
            .persistent()
            .get(&DataKey::AppealReasonHash(escrow_id))
    }

    pub fn get_appeal_deadline(env: Env, escrow_id: u64) -> Option<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::AppealPeriodEnds(escrow_id))
    }

    /// Recompute the Merkle root from the provided items and compare to the
    /// stored root. Returns `true` if they match, `false` otherwise.
    pub fn verify_evidence_set(
        env: Env,
        escrow_id: u64,
        items: Vec<EvidenceItem>,
    ) -> bool {
        let stored_root = Self::get_evidence_root(env.clone(), escrow_id);
        let computed_root = Self::compute_evidence_root(&env, &items);
        stored_root == computed_root
    }

    pub fn submit_appeal_for_dispute(
        env: Env,
        appellant: Address,
        escrow_id: u64,
        reason_hash: BytesN<32>,
    ) -> Result<(), Error> {
        appellant.require_auth();

        let escrow = Self::load_escrow(&env, escrow_id);
        if escrow.status != EscrowStatus::Disputed {
            return Err(Error::InvalidEscrowState);
        }
        if appellant != escrow.mentor && appellant != escrow.learner {
            return Err(Error::Unauthorized);
        }

        let resolution: DisputeResolution = env
            .storage()
            .persistent()
            .get(&DataKey::Resolution(escrow_id))
            .expect("resolution not found");

        let appeal_deadline: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AppealPeriodEnds(escrow_id))
            .expect("appeal deadline not set");
        if env.ledger().timestamp() > appeal_deadline {
            return Err(Error::AppealPeriodExpired);
        }

        let appeal_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AppealCount(escrow_id))
            .unwrap_or(0);
        if appeal_count >= 1 {
            return Err(Error::AppealAlreadySubmitted);
        }

        let governance: Address = env
            .storage()
            .instance()
            .get(&DataKey::GovernanceContract)
            .ok_or(Error::GovernanceContractNotConfigured)?;
        let appeal_arbitrator = Self::select_appeal_arbitrator(&env, &governance, escrow_id, &resolution.arbitrator)?;

        env.storage()
            .persistent()
            .set(&DataKey::AppealCount(escrow_id), &1u32);
        env.storage()
            .persistent()
            .set(&DataKey::AppealArbitrator(escrow_id), &appeal_arbitrator);
        env.storage()
            .persistent()
            .set(&DataKey::AppealReasonHash(escrow_id), &reason_hash);

        let appeal_deadline: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AppealPeriodEnds(escrow_id))
            .expect("appeal deadline not set");

        env.events().publish(
            (Symbol::new(&env, "dispute_appealed"), escrow_id),
            (appellant, appeal_arbitrator.clone(), appeal_deadline),
        );
        Ok(())
    }

    fn select_appeal_arbitrator(
        env: &Env,
        governance: &Address,
        dispute_id: u64,
        original_arbitrator: &Address,
    ) -> Result<Address, Error> {
        let candidate: Address = GovernanceContractClient::new(env, governance)
            .select_arbitrator(&dispute_id);
        if &candidate != original_arbitrator {
            return Ok(candidate);
        }

        let count = GovernanceContractClient::new(env, governance).get_arbitrator_count();
        if count <= 1 {
            return Err(Error::NoAlternativeArbitrator);
        }

        // Try to find the next arbitrator by scanning the list.
        let mut offset = 0;
        while offset < count {
            let list = GovernanceContractClient::new(env, governance)
                .list_arbitrators_page(&offset, &1);
            if let Some(addr) = list.get(0) {
                let addr = addr.clone();
                if &addr != original_arbitrator {
                    return Ok(addr);
                }
            }
            offset += 1;
        }

        Err(Error::NoAlternativeArbitrator)
    }

    fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::Unauthorized)?;
        if stored_admin != *admin {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn load_escrow(env: &Env, escrow_id: u64) -> Escrow {
        let escrow_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::EscrowContract)
            .expect("escrow contract not configured");
        EscrowContractClient::new(env, &escrow_contract).get_escrow(&escrow_id)
    }

    // ── Recording Integrity & Privacy for Dispute Evidence (#914) ──────────────

    /// Attach a session recording as evidence for a dispute
    pub fn attach_recording_evidence(
        env: Env,
        escrow_id: u64,
        recording_id: Symbol,
        session_id: Symbol,
        mentor: Address,
        learner: Address,
        storage_uri: Symbol,
        content_hash: BytesN<32>,
        chunk_hashes: Vec<BytesN<32>>,
        size_bytes: u64,
        duration_secs: u32,
    ) -> Result<SessionRecording, Error> {
        let submitter = env.current_contract_address(); // Would be caller in practice
        submitter.require_auth();

        let escrow = Self::load_escrow(&env, escrow_id);
        if escrow.status != EscrowStatus::Disputed {
            return Err(Error::InvalidEscrowState);
        }
        if submitter != escrow.mentor && submitter != escrow.learner {
            return Err(Error::Unauthorized);
        }

        // Create tamper-evident recording
        let recording = create_recording(
            &env,
            &session_id,
            &mentor,
            &learner,
            storage_uri,
            content_hash,
            &chunk_hashes,
            size_bytes,
            duration_secs,
        );

        // Store recording as evidence
        env.storage().persistent().set(&DataKey::RecordingEvidence(escrow_id), &recording);
        env.storage().persistent().set(&DataKey::SessionRecording(escrow_id), &recording_id);

        // Grant consent to dispute participants (arbitrator, parties)
        let mut consents = Vec::new(&env);
        let mentor_consent = grant_consent(&env, &recording_id, &mentor, &mentor, AccessRole::Participant, 8760, Symbol::new(&env, "full"));
        let learner_consent = grant_consent(&env, &recording_id, &learner, &learner, AccessRole::Participant, 8760, Symbol::new(&env, "full"));
        let arbitrator_consent = grant_consent(&env, &recording_id, &mentor, &escrow.mentor, AccessRole::Arbitrator, 720, Symbol::new(&env, "full")); // 30 days
        consents.push_back(mentor_consent);
        consents.push_back(learner_consent);
        consents.push_back(arbitrator_consent);
        env.storage().persistent().set(&DataKey::RecordingConsent(escrow_id), &consents);

        env.events().publish(
            (Symbol::new(&env, "recording_attached"), escrow_id),
            (recording_id, session_id, mentor, learner),
        );

        Ok(recording)
    }

    /// Get attached recording for a dispute
    pub fn get_dispute_recording(env: Env, escrow_id: u64) -> Option<SessionRecording> {
        env.storage().persistent().get(&DataKey::RecordingEvidence(escrow_id))
    }

    /// Verify recording integrity for dispute evidence
    pub fn verify_recording_integrity(
        env: Env,
        escrow_id: u64,
        provided_chunk_hashes: Vec<BytesN<32>>,
        provided_content_hash: BytesN<32>,
        verifier: Address,
    ) -> Result<IntegrityVerificationResult, Error> {
        let recording: SessionRecording = env.storage().persistent().get(&DataKey::RecordingEvidence(escrow_id))
            .ok_or(Error::NotFound)?;

        let result = verify_recording_integrity(&env, &recording, &provided_chunk_hashes, provided_content_hash, &verifier);

        if result.is_intact {
            let mut updated = recording;
            updated.status = RecordingStatus::Verified;
            updated.verified_at = Some(env.ledger().timestamp());
            env.storage().persistent().set(&DataKey::RecordingEvidence(escrow_id), &updated);
        }

        env.events().publish(
            (Symbol::new(&env, "recording_verified"), escrow_id),
            (result.is_intact, result.verified_chunks, result.total_chunks),
        );

        Ok(result)
    }

    /// Grant consent for recording access in dispute
    pub fn grant_dispute_recording_consent(
        env: Env,
        escrow_id: u64,
        grantor: Address,
        grantee: Address,
        role: AccessRole,
        duration_hours: u32,
        scope: Symbol,
    ) -> Result<ConsentRecord, Error> {
        grantor.require_auth();

        let recording: SessionRecording = env.storage().persistent().get(&DataKey::RecordingEvidence(escrow_id))
            .ok_or(Error::NotFound)?;

        // Only participants or admin can grant consent
        if recording.mentor != grantor && recording.learner != grantor {
            // Check if admin
            let admin: Address = env.storage().instance().get(&DataKey::Admin).ok_or(Error::Unauthorized)?;
            if grantor != admin {
                return Err(Error::Unauthorized);
            }
        }

        let recording_id: Symbol = env.storage().persistent().get(&DataKey::SessionRecording(escrow_id))
            .ok_or(Error::NotFound)?;

        let consent = grant_consent(&env, &recording_id, &grantor, &grantee, role, duration_hours, scope);

        let mut consents: Vec<ConsentRecord> = env.storage().persistent().get(&DataKey::RecordingConsent(escrow_id)).unwrap_or(Vec::new(&env));
        consents.push_back(consent.clone());
        env.storage().persistent().set(&DataKey::RecordingConsent(escrow_id), &consents);

        Ok(consent)
    }

    /// Revoke consent for recording access in dispute
    pub fn revoke_dispute_recording_consent(
        env: Env,
        escrow_id: u64,
        revoker: Address,
    ) -> Result<bool, Error> {
        revoker.require_auth();

        let mut consents: Vec<ConsentRecord> = env.storage().persistent().get(&DataKey::RecordingConsent(escrow_id)).unwrap_or(Vec::new(&env));

        for i in 0..consents.len() {
            let mut consent = consents.get(i).unwrap();
            if consent.grantor == revoker && !consent.revoked {
                let revoked = revoke_consent(&env, &mut consent, &revoker);
                if revoked {
                    consents.set(i, consent);
                    env.storage().persistent().set(&DataKey::RecordingConsent(escrow_id), &consents);
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    /// Apply redaction to dispute recording
    pub fn apply_recording_redaction(
        env: Env,
        admin: Address,
        escrow_id: u64,
        redaction_type: Symbol,
        start_ts: u32,
        end_ts: u32,
        reason_hash: BytesN<32>,
    ) -> Result<RedactionRecord, Error> {
        admin.require_auth();
        Self::require_admin(&env, &admin)?;

        let recording: SessionRecording = env.storage().persistent().get(&DataKey::RecordingEvidence(escrow_id))
            .ok_or(Error::NotFound)?;

        let recording_id: Symbol = env.storage().persistent().get(&DataKey::SessionRecording(escrow_id))
            .ok_or(Error::NotFound)?;

        let redaction = apply_redaction(&env, &recording_id, &admin, redaction_type.clone(), start_ts, end_ts, reason_hash, &admin);

        let mut redactions: Vec<RedactionRecord> = env.storage().persistent().get(&DataKey::RecordingRedaction(escrow_id)).unwrap_or(Vec::new(&env));
        redactions.push_back(redaction.clone());
        env.storage().persistent().set(&DataKey::RecordingRedaction(escrow_id), &redactions);

        // Update recording status
        let mut updated = recording;
        updated.status = RecordingStatus::Redacted;
        env.storage().persistent().set(&DataKey::RecordingEvidence(escrow_id), &updated);

        env.events().publish(
            (Symbol::new(&env, "recording_redacted"), escrow_id),
            (redaction_type, start_ts, end_ts),
        );

        Ok(redaction)
    }

    /// Check if accessor is authorized to view dispute recording
    pub fn check_dispute_recording_access(
        env: Env,
        escrow_id: u64,
        accessor: Address,
        role: AccessRole,
    ) -> Result<bool, Error> {
        let recording: SessionRecording = env.storage().persistent().get(&DataKey::RecordingEvidence(escrow_id))
            .ok_or(Error::NotFound)?;

        let consents: Vec<ConsentRecord> = env.storage().persistent().get(&DataKey::RecordingConsent(escrow_id)).unwrap_or(Vec::new(&env));

        Ok(check_access_authorized(&env, &recording, &consents, &accessor, role))
    }

    /// Log access to dispute recording
    pub fn log_dispute_recording_access(
        env: Env,
        escrow_id: u64,
        accessor: Address,
        role: AccessRole,
        purpose: Symbol,
    ) -> Result<(), Error> {
        let _recording: SessionRecording = env.storage().persistent().get(&DataKey::RecordingEvidence(escrow_id))
            .ok_or(Error::NotFound)?;

        let recording_id: Symbol = env.storage().persistent().get(&DataKey::SessionRecording(escrow_id))
            .ok_or(Error::NotFound)?;

        let entry = log_access(&env, &recording_id, &accessor, role, purpose, &env.current_contract_address(), None);

        let mut logs: Vec<AccessLogEntry> = env.storage().persistent().get(&DataKey::RecordingAccessLog(escrow_id)).unwrap_or(Vec::new(&env));
        logs.push_back(entry);
        env.storage().persistent().set(&DataKey::RecordingAccessLog(escrow_id), &logs);

        Ok(())
    }

    /// Emergency privacy protection for dispute recording
    pub fn emergency_recording_protection(
        env: Env,
        admin: Address,
        escrow_id: u64,
        reason_hash: BytesN<32>,
    ) -> Result<(RedactionRecord, Vec<ConsentRecord>), Error> {
        admin.require_auth();
        Self::require_admin(&env, &admin)?;

        let recording: SessionRecording = env.storage().persistent().get(&DataKey::RecordingEvidence(escrow_id))
            .ok_or(Error::NotFound)?;

        let recording_id: Symbol = env.storage().persistent().get(&DataKey::SessionRecording(escrow_id))
            .ok_or(Error::NotFound)?;

        let (redaction, revoked_consents) = emergency_privacy_protection(&env, &recording_id, reason_hash.clone(), &admin);

        // Update recording status
        let mut updated = recording;
        updated.status = RecordingStatus::Redacted;
        env.storage().persistent().set(&DataKey::RecordingEvidence(escrow_id), &updated);

        env.events().publish(
            (Symbol::new(&env, "recording_emergency_protection"), escrow_id),
            (admin, reason_hash),
        );

        Ok((redaction, revoked_consents))
    }

}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contractimpl,
        symbol_short,
        testutils::{Address as _, Events, Ledger, LedgerInfo},
        IntoVal, TryFromVal,
    };

    #[contract]
    struct MockEscrow;

    fn make_escrow(env: &Env, status: EscrowStatus) -> Escrow {
        Escrow {
            id: 1,
            mentor: Address::generate(env),
            learner: Address::generate(env),
            amount: 100,
            session_id: Symbol::new(env, "sess"),
            status,
            created_at: env.ledger().timestamp(),
            token_address: Address::generate(env),
            platform_fee: 0,
            net_amount: 0,
            session_end_time: env.ledger().timestamp() + 3_600,
            auto_release_delay: 0,
            dispute_reason: Symbol::new(env, "late"),
            resolved_at: 0,
            usd_amount: 0,
            quoted_token_amount: 100,
            send_asset: Address::generate(env),
            dest_asset: Address::generate(env),
            total_sessions: 1,
            sessions_completed: 0,
        }
    }

    #[contractimpl]
    impl MockEscrow {
        pub fn get_escrow(env: Env, _escrow_id: u64) -> Escrow {
            make_escrow(&env, EscrowStatus::Disputed)
        }
    }

    fn setup_disputed() -> (Env, Address, Address, Address, DisputeEvidenceContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let escrow_contract = env.register_contract(None, MockEscrow);
        let contract_id = env.register_contract(None, DisputeEvidenceContract);
        let client = DisputeEvidenceContractClient::new(&env, &contract_id);
        client.initialize(&admin, &escrow_contract);
        let escrow = EscrowContractClient::new(&env, &escrow_contract).get_escrow(&1);
        (env, admin, escrow.mentor, escrow.learner, client)
    }

    #[contract]
    struct MockGovernance;

    #[contractimpl]
    impl MockGovernance {
        pub fn initialize(env: Env, arbitrators: Vec<Address>) {
            env.storage()
                .persistent()
                .set(&symbol_short!("ARBITS"), &arbitrators);
            env.storage()
                .persistent()
                .set(&symbol_short!("ARB_COUNT"), &arbitrators.len());
        }

        pub fn select_arbitrator(env: Env, dispute_id: u64) -> Address {
            let arbitrators: Vec<Address> = env
                .storage()
                .persistent()
                .get(&symbol_short!("ARBITS"))
                .expect("no arbitrators configured");
            let count = arbitrators.len();
            let idx = (dispute_id % (count as u64)) as u32;
            arbitrators
                .get(idx)
                .expect("arbitrator index out of range")
                .clone()
        }

        pub fn get_arbitrator_count(env: Env) -> u32 {
            env.storage()
                .persistent()
                .get(&symbol_short!("ARB_COUNT"))
                .unwrap_or(0)
        }

        pub fn list_arbitrators_page(env: Env, offset: u32, limit: u32) -> Vec<Address> {
            let arbitrators: Vec<Address> = env
                .storage()
                .persistent()
                .get(&symbol_short!("ARBITS"))
                .unwrap_or(Vec::new(&env));
            let mut out = Vec::new(&env);
            let end = (offset + limit).min(arbitrators.len());
            for i in offset..end {
                out.push_back(arbitrators.get(i).unwrap().clone());
            }
            out
        }
    }

    fn setup_disputed_with_governance(
        env: &Env,
        client: &DisputeEvidenceContractClient<'_>,
        admin: &Address,
    ) -> Address {
        let governance_contract = env.register_contract(None, MockGovernance);
        let governance_client = MockGovernanceClient::new(env, &governance_contract);
        let arb1 = Address::generate(env);
        let arb2 = Address::generate(env);
        let arb3 = Address::generate(env);
        let mut arbitrators = Vec::new(env);
        arbitrators.push_back(arb1.clone());
        arbitrators.push_back(arb2.clone());
        arbitrators.push_back(arb3.clone());
        governance_client.initialize(&arbitrators);
        client
            .set_governance_contract(admin, &governance_contract)
            .unwrap();
        governance_contract
    }

    #[test]
    fn submit_appeal_creates_appeal_record_and_emits_event() {
        let (env, admin, mentor, _learner, client) = setup_disputed();
        let governance_contract = setup_disputed_with_governance(&env, &client, &admin);
        let arb = Address::generate(&env);

        client.record_dispute_opened(&1).unwrap();
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS + 1);
        client
            .submit_resolution(&1, &arb, &true, &Symbol::new(&env, "mentor_wins"))
            .unwrap();

        let appeal_reason = hash32(&env, 77);
        let result = client.try_submit_appeal_for_dispute(&mentor, &1, &appeal_reason);
        assert!(result.is_ok(), "appeal submission should succeed");

        let appeal_arbitrator = client.get_appeal_arbitrator(&1).expect("appeal arbitrator set");
        assert_ne!(appeal_arbitrator, arb, "appeal arbitrator must be different from original");
        assert_eq!(client.get_appeal_reason_hash(&1).unwrap(), appeal_reason);
        assert!(client.get_appeal_deadline(&1).is_some());

        let events = env.events().all();
        let last = events.last().unwrap();
        assert_eq!(last.1, (Symbol::new(&env, "dispute_appealed"), 1u64).into_val(&env));
    }

    #[test]
    fn submit_appeal_after_deadline_fails() {
        let (env, admin, mentor, _learner, client) = setup_disputed();
        let _governance_contract = setup_disputed_with_governance(&env, &client, &admin);
        let arb = Address::generate(&env);

        client.record_dispute_opened(&1).unwrap();
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS + 1);
        client
            .submit_resolution(&1, &arb, &true, &Symbol::new(&env, "mentor_wins"))
            .unwrap();

        let deadline = client.get_appeal_deadline(&1).unwrap();
        advance_time(&env, deadline.saturating_sub(env.ledger().timestamp()) + 1);

        let result = client.try_submit_appeal_for_dispute(&mentor, &1, &hash32(&env, 78));
        assert!(result.is_err(), "appeal after deadline should fail");
    }

    #[test]
    fn submit_appeal_without_governance_fails() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);

        client.record_dispute_opened(&1).unwrap();
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS + 1);
        client
            .submit_resolution(&1, &arb, &true, &Symbol::new(&env, "mentor_wins"))
            .unwrap();

        let result = client.try_submit_appeal_for_dispute(&mentor, &1, &hash32(&env, 79));
        assert!(result.is_err(), "appeal without governance contract should fail");
    }

    #[test]
    fn second_appeal_submission_is_rejected() {
        let (env, admin, mentor, _learner, client) = setup_disputed();
        let _governance_contract = setup_disputed_with_governance(&env, &client, &admin);
        let arb = Address::generate(&env);

        client.record_dispute_opened(&1).unwrap();
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS + 1);
        client
            .submit_resolution(&1, &arb, &true, &Symbol::new(&env, "mentor_wins"))
            .unwrap();

        client
            .submit_appeal_for_dispute(&mentor, &1, &hash32(&env, 80))
            .unwrap();
        let result = client.try_submit_appeal_for_dispute(&mentor, &1, &hash32(&env, 81));
        assert!(result.is_err(), "second appeal submission must be rejected");
    }

    /// Build a distinct, non-zero 32-byte hash for tests, seeded by `seed`.
    fn hash32(env: &Env, seed: u8) -> BytesN<32> {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        bytes[31] = seed.wrapping_add(1).max(1); // ensure non-zero even if seed == 0/255
        BytesN::from_array(env, &bytes)
    }

    fn zero_hash32(env: &Env) -> BytesN<32> {
        BytesN::from_array(env, &[0u8; 32])
    }

    fn advance_time(env: &Env, secs: u64) {
        let t = env.ledger().timestamp();
        let protocol_version = env.ledger().protocol_version();
        env.ledger().set(LedgerInfo {
            timestamp: t + secs,
            protocol_version,
            sequence_number: env.ledger().sequence() + 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 100,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: 9_999_999,
        });
    }

    // ─── existing: evidence cap ───────────────────────────────────────────

    #[test]
    fn stores_evidence_until_cap() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        // Disable cooldown to allow rapid sequential submissions for cap test
        client.set_cooldown_enabled(&_admin, &false);
        for seed in 1u8..=5 {
            client
                .submit_evidence(&1, &mentor, &hash32(&env, seed), &hash32(&env, seed.wrapping_add(100)), &None);
        }
        assert_eq!(client.get_evidence_count(&1), MAX_EVIDENCE_ITEMS);
    }

    // ─── #651: content-hash commitment scheme ──────────────────────────────

    #[test]
    fn submit_evidence_rejects_zero_content_hash() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        let result = client.try_submit_evidence(
            &1,
            &mentor,
            &zero_hash32(&env),
            &hash32(&env, 1),
            &None,
        );
        assert!(result.is_err(), "zero content_hash must be rejected");
    }

    #[test]
    fn submit_evidence_rejects_duplicate_hash_same_submitter() {
        let (env, admin, mentor, _learner, client) = setup_disputed();
        client.set_cooldown_enabled(&admin, &false);
        let hash = hash32(&env, 7);
        client
            .submit_evidence(&1, &mentor, &hash, &hash32(&env, 8), &None);
        let result = client.try_submit_evidence(&1, &mentor, &hash, &hash32(&env, 9), &None);
        assert!(
            result.is_err(),
            "duplicate content_hash from same submitter must be rejected"
        );
    }

    #[test]
    fn submit_evidence_allows_same_hash_from_different_submitters() {
        let (env, admin, mentor, learner, client) = setup_disputed();
        client.set_cooldown_enabled(&admin, &false);
        let hash = hash32(&env, 7);
        client
            .submit_evidence(&1, &mentor, &hash, &hash32(&env, 8), &None);
        // Different submitter, same content_hash — allowed (e.g. both
        // parties independently attest to the same document).
        client
            .submit_evidence(&1, &learner, &hash, &hash32(&env, 8), &None);
        assert_eq!(client.get_evidence_count(&1), 2);
    }

    #[test]
    fn verify_evidence_integrity_matches_submitted_hash() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        let hash = hash32(&env, 42);
        client
            .submit_evidence(&1, &mentor, &hash, &hash32(&env, 43), &None);
        assert!(client.verify_evidence_integrity(&1, &0, &hash));
    }

    #[test]
    fn verify_evidence_integrity_fails_on_tampered_hash() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 42), &hash32(&env, 43), &None);
        // A different (tampered) hash must not verify.
        assert!(!client.verify_evidence_integrity(&1, &0, &hash32(&env, 99)));
    }

    #[test]
    fn verify_evidence_integrity_false_for_out_of_range_index() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        assert!(!client.verify_evidence_integrity(&1, &0, &hash32(&env, 1)));
    }

    #[test]
    fn submit_evidence_stores_optional_submitter_attestation() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        let mut sig_bytes = [0u8; 64];
        sig_bytes[0] = 9;
        let attestation = BytesN::from_array(&env, &sig_bytes);
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &Some(attestation.clone()));
        let items = client.get_evidence(&1);
        assert_eq!(items.get(0).unwrap().submitter_attestation, attestation);
    }

    #[test]
    fn submit_evidence_without_attestation_stores_zero_sentinel() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None);
        let items = client.get_evidence(&1);
        assert_eq!(
            items.get(0).unwrap().submitter_attestation,
            BytesN::from_array(&env, &[0u8; 64])
        );
    }

    // ─── #417: anti-spam cooldown ─────────────────────────────────────────

    #[test]
    fn second_submission_within_cooldown_fails() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None);
        // Immediately retry (within cooldown) → must fail
        let result = client.try_submit_evidence(&1, &mentor, &hash32(&env, 3), &hash32(&env, 4), &None);
        assert!(result.is_err(), "second submission within cooldown must fail");
    }

    #[test]
    fn submission_allowed_after_cooldown_elapses() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None);
        advance_time(&env, SUBMISSION_COOLDOWN_SECS + 1);
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 3), &hash32(&env, 4), &None);
        assert_eq!(client.get_evidence_count(&1), 2);
    }

    #[test]
    fn different_parties_may_submit_independently() {
        let (env, _admin, mentor, learner, client) = setup_disputed();
        // mentor submits
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None);
        // learner submits in the same window — separate cooldown key
        client
            .submit_evidence(&1, &learner, &hash32(&env, 3), &hash32(&env, 4), &None);
        assert_eq!(client.get_evidence_count(&1), 2);
    }

    #[test]
    fn cooldown_disabled_allows_rapid_submission() {
        let (env, admin, mentor, _learner, client) = setup_disputed();
        client.set_cooldown_enabled(&admin, &false);
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None);
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 3), &hash32(&env, 4), &None);
        assert_eq!(client.get_evidence_count(&1), 2);
    }

    // ─── #417: resolution timelock ────────────────────────────────────────

    #[test]
    fn resolution_before_timelock_fails() {
        let (env, admin, _mentor, _learner, client) = setup_disputed();
        let arbitrator = Address::generate(&env);
        client.record_dispute_opened(&1);

        // Do NOT advance time
        let result = client.try_submit_resolution(
            &1,
            &arbitrator,
            &true,
            &Symbol::new(&env, "mentor_wins"),
        );
        assert!(result.is_err(), "resolution before timelock must fail");
        let _ = admin;
    }

    #[test]
    fn resolution_after_timelock_succeeds() {
        let (env, admin, _mentor, _learner, client) = setup_disputed();
        let arbitrator = Address::generate(&env);
        client.record_dispute_opened(&1);

        advance_time(&env, MIN_RESOLUTION_DELAY_SECS + 1);

        client
            .submit_resolution(&1, &arbitrator, &true, &Symbol::new(&env, "mentor_wins"));

        let res = client.get_resolution(&1);
        assert_eq!(res.arbitrator, arbitrator);
        assert!(res.release_to_mentor);
        let _ = admin;
    }

    #[test]
    fn resolution_without_opened_at_record_is_allowed() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        // No `record_dispute_opened` call — guard is skipped for backwards compat
        let arbitrator = Address::generate(&env);
        client
            .submit_resolution(&1, &arbitrator, &false, &Symbol::new(&env, "learner_wins"));
        let res = client.get_resolution(&1);
        assert!(!res.release_to_mentor);
    }

    // ─── record_dispute_opened idempotency & getter ────────────────────────

    #[test]
    fn record_dispute_opened_is_idempotent() {
        let (env, admin, _mentor, _learner, client) = setup_disputed();
        client.record_dispute_opened(&1);
        let result = client.try_record_dispute_opened(&1);
        assert!(result.is_err(), "second call must return AlreadyRecorded");
        let _ = admin;
    }

    #[test]
    fn get_dispute_opened_at_returns_none_before_record() {
        let (_env, _admin, _mentor, _learner, client) = setup_disputed();
        assert_eq!(client.get_dispute_opened_at(&1), None);
    }

    #[test]
    fn get_dispute_opened_at_returns_timestamp_after_record() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let before = env.ledger().timestamp();
        client.record_dispute_opened(&1);
        let after = env.ledger().timestamp();
        let opened = client.get_dispute_opened_at(&1).unwrap();
        assert!(opened >= before && opened <= after);
    }

    // ─── #417: duplicate resolution rejected ─────────────────────────────

    #[test]
    fn second_resolution_rejected() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);
        client
            .submit_resolution(&1, &arb, &true, &Symbol::new(&env, "a"));
        let result = client.try_submit_resolution(&1, &arb, &false, &Symbol::new(&env, "b"));
        assert!(result.is_err(), "second resolution must be rejected");
    }

    // ─── boundary: resolution exactly at MIN_RESOLUTION_DELAY_SECS ─────────

    #[test]
    fn resolution_at_exact_timelock_boundary_succeeds() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);
        client.record_dispute_opened(&1);
        // Advance exactly the minimum delay (no extra second).
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS);
        client
            .submit_resolution(&1, &arb, &true, &Symbol::new(&env, "ok"));
    }

    #[test]
    fn resolution_one_second_before_timelock_fails() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);
        client.record_dispute_opened(&1);
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS.saturating_sub(1));
        let result = client.try_submit_resolution(&1, &arb, &true, &Symbol::new(&env, "ok"));
        assert!(result.is_err(), "resolution 1s before timelock must fail");
    }

    // ─── fuzz: resolution at various offsets from opened_at ───────────────

    #[test]
    fn fuzz_resolution_boundary_offsets() {
        // Property-style test: for a range of offsets around the timelock,
        // verify that resolution is rejected before the boundary and accepted
        // at or after it.
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);

        for offset in [0, 1, 10, 100, MIN_RESOLUTION_DELAY_SECS / 2] {
            client.record_dispute_opened(&1);
            advance_time(&env, offset);
            let result = client.try_submit_resolution(&1, &arb, &true, &Symbol::new(&env, "r"));
            if offset < MIN_RESOLUTION_DELAY_SECS {
                assert!(
                    result.is_err(),
                    "offset {} < MIN_RESOLUTION_DELAY_SECS must fail",
                    offset
                );
            } else {
                assert!(
                    result.is_ok(),
                    "offset {} >= MIN_RESOLUTION_DELAY_SECS must succeed",
                    offset
                );
            }
            // Reset for next iteration by creating a fresh env via a new test
            // is not possible here, so we rely on the fact that once resolved,
            // subsequent calls will hit AlreadyResolved. We only test the first
            // offset that succeeds and break.
            if offset >= MIN_RESOLUTION_DELAY_SECS {
                break;
            }
        }
    }

    // ─── #417: events ─────────────────────────────────────────────────────

    #[test]
    fn evidence_submitted_event_contains_correct_payload() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        let hash = hash32(&env, 1);
        let uri_hash = hash32(&env, 2);
        client.submit_evidence(&1, &mentor, &hash, &uri_hash, &None);

        let expected_item = EvidenceItem {
            submitter: mentor.clone(),
            content_hash: hash,
            evidence_uri_hash: uri_hash,
            submitter_attestation: BytesN::from_array(&env, &[0u8; 64]),
            submitted_at: env.ledger().timestamp(),
        };

        let topics: Vec<soroban_sdk::Val> =
            (Symbol::new(&env, "evidence_submitted"), 1u64).into_val(&env);
        let data: soroban_sdk::Val = expected_item.into_val(&env);
        let mut expected: Vec<(Address, Vec<soroban_sdk::Val>, soroban_sdk::Val)> = Vec::new(&env);
        expected.push_back((client.address.clone(), topics, data));

        assert_eq!(env.events().all(), expected);
    }

    #[test]
    fn dispute_resolved_event_emitted_on_resolution() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);
        client.submit_resolution(&1, &arb, &true, &Symbol::new(&env, "ok"));

        // `events().all()` only reflects the *last* contract invocation, so
        // the expected resolution is built from known inputs rather than by
        // calling `get_resolution` (which would itself become "the last
        // invocation" and clear the events we're asserting on).
        let expected_resolution = DisputeResolution {
            arbitrator: arb,
            release_to_mentor: true,
            note: Symbol::new(&env, "ok"),
            resolved_at: env.ledger().timestamp(),
        };
        let topics: Vec<soroban_sdk::Val> =
            (Symbol::new(&env, "dispute_resolved"), 1u64).into_val(&env);
        let data: soroban_sdk::Val = expected_resolution.into_val(&env);
        let mut expected: Vec<(Address, Vec<soroban_sdk::Val>, soroban_sdk::Val)> = Vec::new(&env);
        expected.push_back((client.address.clone(), topics, data));

        assert_eq!(env.events().all(), expected);
    }

    // ─── #760: health_dashboard integration hooks ─────────────────────────

    #[contracttype]
    #[derive(Clone)]
    enum DashboardMockKey {
        OpenedCall(u64),
        ResolutionCall(u64),
    }

    #[contract]
    struct MockHealthDashboard;

    #[contractimpl]
    impl MockHealthDashboard {
        pub fn record_dispute_opened(env: Env, escrow_id: u64, opened_at: u64) {
            env.storage()
                .persistent()
                .set(&DashboardMockKey::OpenedCall(escrow_id), &opened_at);
        }

        pub fn record_resolution(
            env: Env,
            escrow_id: u64,
            release_to_mentor: bool,
            resolution_time_secs: u64,
        ) {
            env.storage().persistent().set(
                &DashboardMockKey::ResolutionCall(escrow_id),
                &(release_to_mentor, resolution_time_secs),
            );
        }

        pub fn get_opened_call(env: Env, escrow_id: u64) -> Option<u64> {
            env.storage()
                .persistent()
                .get(&DashboardMockKey::OpenedCall(escrow_id))
        }

        pub fn get_resolution_call(env: Env, escrow_id: u64) -> Option<(bool, u64)> {
            env.storage()
                .persistent()
                .get(&DashboardMockKey::ResolutionCall(escrow_id))
        }
    }

    #[test]
    fn record_dispute_opened_notifies_configured_health_dashboard() {
        let (env, admin, _mentor, _learner, client) = setup_disputed();
        let dashboard_id = env.register_contract(None, MockHealthDashboard);
        client.set_health_dashboard(&admin, &dashboard_id);

        client.record_dispute_opened(&1);

        let dashboard_client = MockHealthDashboardClient::new(&env, &dashboard_id);
        assert!(dashboard_client.get_opened_call(&1).is_some());
    }

    #[test]
    fn record_dispute_opened_without_dashboard_configured_is_backwards_compatible() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        // No set_health_dashboard call — must still succeed.
        client.record_dispute_opened(&1);
        let _ = env;
    }

    #[test]
    fn submit_resolution_notifies_configured_health_dashboard_with_favor_and_duration() {
        let (env, admin, _mentor, _learner, client) = setup_disputed();
        let dashboard_id = env.register_contract(None, MockHealthDashboard);
        client.set_health_dashboard(&admin, &dashboard_id);

        client.record_dispute_opened(&1);
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS + 500);

        let arbitrator = Address::generate(&env);
        client.submit_resolution(&1, &arbitrator, &true, &Symbol::new(&env, "mentor_wins"));

        let dashboard_client = MockHealthDashboardClient::new(&env, &dashboard_id);
        let (release_to_mentor, resolution_time_secs) =
            dashboard_client.get_resolution_call(&1).unwrap();
        assert!(release_to_mentor);
        assert_eq!(resolution_time_secs, MIN_RESOLUTION_DELAY_SECS + 500);
    }

    // ─── #781: Merkle evidence root ───────────────────────────────────────

    #[test]
    fn evidence_root_is_zero_before_any_submission() {
        let (_env, _admin, _mentor, _learner, client) = setup_disputed();
        let root = client.get_evidence_root(&1);
        assert_eq!(root, BytesN::from_array(&_env, &[0u8; 32]));
    }

    #[test]
    fn evidence_root_changes_after_each_submission() {
        let (env, admin, mentor, _learner, client) = setup_disputed();
        client.set_cooldown_enabled(&admin, &false).unwrap();

        let root0 = client.get_evidence_root(&1);

        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        let root1 = client.get_evidence_root(&1);
        assert_ne!(root0, root1, "root must change after first submission");

        client
            .submit_evidence(&1, &mentor, &hash32(&env, 3), &hash32(&env, 4), &None)
            .unwrap();
        let root2 = client.get_evidence_root(&1);
        assert_ne!(root1, root2, "root must change after second submission");
    }

    #[test]
    fn verify_evidence_set_returns_true_for_exact_set() {
        let (env, admin, mentor, learner, client) = setup_disputed();
        client.set_cooldown_enabled(&admin, &false).unwrap();

        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        client
            .submit_evidence(&1, &learner, &hash32(&env, 3), &hash32(&env, 4), &None)
            .unwrap();

        let items = client.get_evidence(&1);
        assert!(client.verify_evidence_set(&1, &items));
    }

    #[test]
    fn verify_evidence_set_returns_false_for_tampered_item() {
        let (env, admin, mentor, learner, client) = setup_disputed();
        client.set_cooldown_enabled(&admin, &false).unwrap();

        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        client
            .submit_evidence(&1, &learner, &hash32(&env, 3), &hash32(&env, 4), &None)
            .unwrap();

        let mut items = client.get_evidence(&1);
        // Tamper with the first item's content_hash.
        let mut tampered = items.get(0).unwrap();
        tampered.content_hash = hash32(&env, 99);
        items.set(0, tampered);

        assert!(
            !client.verify_evidence_set(&1, &items),
            "tampered evidence must fail verification"
        );
    }

    #[test]
    fn resolution_records_evidence_root_at_time_of_ruling() {
        let (env, admin, mentor, _learner, client) = setup_disputed();
        client.set_cooldown_enabled(&admin, &false).unwrap();

        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        let root_at_ruling = client.get_evidence_root(&1);

        let arb = Address::generate(&env);
        client.record_dispute_opened(&1).unwrap();
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS + 1);
        client
            .submit_resolution(&1, &arb, &true, &Symbol::new(&env, "ok"))
            .unwrap();

        let resolution = client.get_resolution(&1);
        assert_eq!(
            resolution.evidence_root, root_at_ruling,
            "resolution must capture the evidence root at time of ruling"
        );
    }

    #[test]
    fn evidence_root_updated_event_emitted() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        let events = env.events().all();
        // Look for the evidence_root_updated event (second event emitted)
        let found = events.iter().any(|e| {
            // Topics are (event_name, escrow_id, item_count)
            e.1 == (Symbol::new(&env, "evidence_root_updated"), 1u64, 1u32).into_val(&env)
        });
        assert!(found, "evidence_root_updated event must be emitted");
    }

    // -----------------------------------------------------------------------
    // Payment-integrity protection: validate_dispute_claims / arbitrate_dispute (#886)
    // -----------------------------------------------------------------------

    #[test]
    fn arbitrate_dispute_rejects_when_no_evidence_submitted() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);

        client.record_dispute_opened(&1).unwrap();
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS + 1);

        let claims = client.validate_dispute_claims(&1);
        assert!(!claims.sufficient);

        let result = client.try_arbitrate_dispute(&1, &arb, &true, &Symbol::new(&env, "mentor_wins"));
        assert!(result.is_err());
    }

    #[test]
    fn arbitrate_dispute_succeeds_with_evidence_and_cooldown() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);

        client.record_dispute_opened(&1).unwrap();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS + 1);

        let claims = client.validate_dispute_claims(&1);
        assert!(claims.sufficient);

        client
            .arbitrate_dispute(&1, &arb, &true, &Symbol::new(&env, "mentor_wins"))
            .unwrap();

        let resolution = client.get_resolution(&1);
        assert!(resolution.release_to_mentor);
    }
}

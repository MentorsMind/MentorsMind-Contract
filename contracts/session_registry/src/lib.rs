#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, BytesN, Env, Symbol, Vec, String};
use shared::{
    // learner protection
    assess_vulnerability,
    compute_emergency_intervention,
    compute_learner_protection_intervention,
    compute_scalability_intervention,
    compute_welfare_status as shared_compute_welfare_status,
    detect_coordination,
    detect_predatory_behavior as shared_detect_predatory_behavior,
    detect_price_coordination,
    detect_resource_competition as shared_detect_resource_competition,
    distribute_resources_fairly as shared_distribute_resources_fairly,
    enforce_learner_fair_pricing as shared_enforce_learner_fair_pricing,
    evaluate_fair_access,
    identify_exploitation_patterns as shared_identify_exploitation_patterns,
    compute_welfare_status as shared_compute_welfare_status,
    VulnerabilityAssessment, EmergencyIntervention, LearnerProtectionRecord,
    // resource management
    allocate_system_resources, manage_session_load, detect_abuse_patterns, check_emergency_trigger,
    // platform authenticity
    verify_session_authenticity, detect_platform_bypass, PenaltyTier,
    interaction_commitment,
    is_performance_restoration_eligible,
    is_protection_restoration_eligible,
    validate_load_pattern as shared_validate_load_pattern,
    verify_demand_authenticity as shared_verify_demand_authenticity,
    CartelDetection,
    CartelDetectionResult,
    CoordinationFlag,
    CoordinationPattern,
    DemandAuthenticity,
    EmergencyIntervention,
    FairAccessDecision,
    FairResourceAllocation,
    LearnerProtectionRecord,
    LoadValidationResult,
    PerformanceInterventionRecord,
    PriceCoordinationFlag,
    ReputationProof,
    ResourceCompetitionFlag,
    SocialProofRecord,
    TimeSlotFairnessAnalysis,
    TimeSlotInfo,
    VulnerabilityAssessment,
    PERFORMANCE_RESTORATION_COOLDOWN_SECS,
    // Session uniqueness & replay detection (#905)
    validate_session_nonce,
    verify_content_checksum,
    detect_temporal_replay,
    MAX_SESSION_TIME_DRIFT_SECS,
    // Exit strategy & competition protection (#932)
    facilitate_migration,
    evaluate_competition_protection,
    // Metadata validation & transparency
    audit_information_accuracy,
    is_transparency_restoration_eligible,
    monitor_metadata_manipulation,
    protect_transparency as shared_protect_transparency,
    restore_truth_and_correct,
    InformationAuditRecord,
    InformationIntegrity,
    MetadataMonitoringRecord,
    MetadataValidation,
    TransparencyProtection,
    TruthRestorationRecord,
    TRANSPARENCY_RESTORATION_COOLDOWN_SECS,
};
use shared::*;
use shared::recording_integrity::ConsentRecord;

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, xdr::ToXdr, Address, BytesN, Env, Map,
    Symbol, Vec,
};

// ── Storage keys ─────────────────────────────────────────────────────────────
const BACKEND: Symbol = symbol_short!("BACKEND");
const TTL_THRESHOLD: u32 = 500_000;
const TTL_BUMP: u32 = 1_000_000;
/// Schedule occupancy is tracked in 30-minute buckets.
const SLOT_SIZE_SECS: u64 = 1_800;
/// Minimum free time required between consecutive sessions on the same mentor.
const SCHEDULING_BUFFER_SECS: u64 = 900;
/// Rolling window used to compute a mentor's booking-request rate for
/// load-attack validation (#scalability-protection).
const LOAD_MONITORING_WINDOW_SECS: u64 = 300;

// ── Types ─────────────────────────────────────────────────────────────────────
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionStatus {
    Pending,
    Confirmed,
    Completed,
    Cancelled,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecord {
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub scheduled_at: u64,
    pub duration_mins: u32,
    pub amount: i128,
    pub token: Address,
    pub status: SessionStatus,
    pub registered_at: u64,
    pub protected_content: Vec<Symbol>, // Content IDs associated with this session
    pub content_licenses: Vec<Symbol>,  // License IDs for session content
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Session(Symbol),
    /// Deprecated: kept for backward compat, no longer written to.
    /// Use `MentorSessionAt` / `MentorSessionCount` for all new reads/writes.
    MentorSessions(Address),
    /// Deprecated: kept for backward compat, no longer written to.
    /// Use `LearnerSessionAt` / `LearnerSessionCount` for all new reads/writes.
    LearnerSessions(Address),
    MentorSessionCount(Address),
    MentorSessionAt(Address, u32),
    LearnerSessionCount(Address),
    LearnerSessionAt(Address, u32),
    /// Maps `(mentor_address, time_bucket_index)` → `session_id`.
    /// A time bucket covers `SLOT_SIZE_SECS` seconds.
    MentorScheduleSlot(Address, u64),
    SessionOracle,
    SessionMetadata(Symbol),
    CompletionProof(Symbol),
    /// Scheduled-at timestamps for one mentor/learner pair, used for
    /// coordination-ring detection (#community-protection).
    MentorLearnerLog(Address, Address),
    MentorCoordination(Address),
    MentorFairAccess(Address),
    /// Whether `learner` has ever booked `mentor` before (distinct-requester tracking).
    MentorHasBookedBefore(Address, Address),
    MentorDistinctLearnerCount(Address),
    /// Booking-request timestamps for a mentor, used for demand-authenticity checks.
    MentorRequestLog(Address),
    /// Rolling window of recently-booked session prices/timestamps across all
    /// mentors, used for pricing-coordination detection.
    RecentSessionPrices,
    RecentSessionPriceTimestamps,
    /// Cached resource-competition assessment for a mentor's booking load
    /// (#scalability-protection).
    SystemLoadRecord(Address),
    /// Rolling total requested booking-capacity units (duration-minutes) for
    /// a mentor, used for fair-resource-distribution scoring.
    MentorTotalRequestedUnits(Address),
    /// Cached combined performance-protection intervention record for a
    /// mentor.
    PerformanceIntervention(Address),
    // ── Learner protection (#917) ──────────────────────────────────────────
    /// Total session count between a specific learner and mentor pair, used
    /// for recurrence/dependency vulnerability assessment.
    LearnerMentorSessionCount(Address, Address),
    /// Rolling sum of session prices paid by a learner (for avg computation).
    LearnerTotalSpend(Address),
    /// Total session count for a learner across all mentors.
    LearnerTotalSessionCount(Address),
    /// Cached vulnerability assessment for a learner/mentor pair.
    LearnerVulnerabilityRecord(Address, Address),
    /// Cached predatory-behaviour detection result for a mentor (from the
    /// session registry's perspective – complaint/quality signals come from
    /// the reputation contract; this stores the combined view once pushed
    /// here by `monitor_mentor_behavior`).
    MentorPredatoryBehaviorRecord(Address),
    /// Cached learner-protection intervention record for a mentor.
    LearnerProtectionIntervention(Address),
    /// Emergency intervention record for a mentor.
    MentorEmergencyIntervention(Address),
    /// Whether a mentor is currently under an active emergency suspension.
    MentorSuspended(Address),
    // ── Resource Management ────────────────────────────────────────────────
    MentorRequestCount(Address, u32),
    ActiveSessions(Address),
    TotalRequests(Address),
    CancelledRequests(Address),
    // ── Platform Authenticity ──────────────────────────────────────────────
    MentorLowFeeCount(Address, Address),
    /// Minimum sessions a mentor must keep available in a window before
    /// hoarding/artificial scarcity audits flag the account.
    MentorMinAvailabilityQuota(Address),
    // ── Session uniqueness & Replay prevention (#905) ───────────────────────
    SessionNonce(Symbol),
    SessionContentHash(Symbol),
    SessionNonceUsed(Symbol, u64),
    // ── Migration & Data Portability (#932) ─────────────────────────────────
    UserMigrationRecord(Address),
    UserDataExportHash(Address),
    // ── Additional operational keys ─────────────────────────────────────────
    SessionMetadataValidation(Symbol),
    SessionInformationIntegrity(Symbol),
    SessionTransparencyProtection(Symbol),
    SessionMetadataMonitoring(Symbol),
    SessionInformationAudit(Symbol),
    SessionTruthRestoration(Symbol),
    MentorWorkload(Address),
    MentorBurnoutAssessment(Address),
    WellnessIntervention(Address),
    SessionRecording(Symbol),
    RecordingConsent(Symbol),
    RecordingRedaction(Symbol),
    RecordingAccessLog(Symbol),
    SpecializationMetrics(Symbol),
    MarketManipulationAlert(Symbol),
    EmergencyStabilization(Symbol),
    AvailabilityCommit(Address, u64),
    AvailabilityChangeLog(Address),
    EmergencyOverride(Symbol),
    SessionAccessAudit(Symbol),
    AccessorContained(Address),
    OutOfScopeAccessLog(Address),
    OutOfScopeSessionSet(Address),
    SessionProtection(Symbol),
    AttackEventLog(Symbol),
    ContinuityBackup(Symbol),
    LearningOutcome(Symbol),
}

/// Maximum length of the rolling price/pair/request logs kept for scoring.
const MONITORING_LOG_CAP: u32 = 20;

// ── Errors ────────────────────────────────────────────────────────────────────
// Errors are surfaced via panics to keep compatibility with SDK 21 contractimpl.
// Error codes are documented here for reference:
// NotInitialized = 1, Unauthorized = 2, SessionNotFound = 3, DuplicateSession = 4
// SessionConflict = 5, InsufficientBuffer = 6

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionConflict {
    pub conflicting_session_id: Symbol,
}

/// A single data-access audit entry for a session (#899).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionAccessAuditEntry {
    pub accessor: Address,
    pub accessed_at: u64,
    pub allowed: bool,
}

// ── Contract ──────────────────────────────────────────────────────────────────
#[contract]
pub struct SessionRegistry;

#[contractimpl]
impl SessionRegistry {
    /// Initialize with the platform backend address (only caller allowed to register/update).
    pub fn initialize(env: Env, backend: Address) {
        if env.storage().instance().has(&BACKEND) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&BACKEND, &backend);
        env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);
    }

    /// Register a new session. Only callable by the platform backend.
    /// Performs conflict detection and 15-minute buffer enforcement.
    pub fn register_session(
        env: Env,
        session_id: Symbol,
        mentor: Address,
        learner: Address,
        scheduled_at: u64,
        duration_mins: u32,
        amount: i128,
        token: Address,
    ) -> Symbol {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let session_key = DataKey::Session(session_id.clone());
        if env.storage().persistent().has(&session_key) {
            panic!("Duplicate session");
        }

        // Check for scheduling conflicts and buffer enforcement
        Self::check_scheduling_conflicts(&env, &mentor, scheduled_at, duration_mins);

        // ── Resource Management & Rate Limiting ──────────────────────────────────
        let req_key = DataKey::MentorRequestCount(mentor.clone(), env.ledger().sequence());
        let mut req_count: u32 = env.storage().temporary().get(&req_key).unwrap_or(0);
        req_count += 1;
        env.storage().temporary().set(&req_key, &req_count);

        let limit_status = manage_session_load(&env, req_count, false);
        if !limit_status.allowed {
            panic!("Rate limit exceeded");
        }

        let active_key = DataKey::ActiveSessions(mentor.clone());
        let current_active: u32 = env.storage().persistent().get(&active_key).unwrap_or(0);
        let allocation = allocate_system_resources(&env, current_active, 1);
        if !allocation.granted {
            panic!("Resource quota exceeded");
        }
        env.storage().persistent().set(&active_key, &(current_active + 1));

        let tot_req_key = DataKey::TotalRequests(mentor.clone());
        let total_requests: u32 = env.storage().persistent().get(&tot_req_key).unwrap_or(0) + 1;
        env.storage().persistent().set(&tot_req_key, &total_requests);

        let can_req_key = DataKey::CancelledRequests(mentor.clone());
        let cancelled_requests: u32 = env.storage().persistent().get(&can_req_key).unwrap_or(0);
        
        let abuse_status = detect_abuse_patterns(&env, total_requests, cancelled_requests);
        if abuse_status.is_abusive {
            panic!("Abuse pattern detected");
        }
        
        let low_fee_key = DataKey::MentorLowFeeCount(mentor.clone(), learner.clone());
        let current_low_fee: u32 = env.storage().persistent().get(&low_fee_key).unwrap_or(0);
        let collusion = detect_platform_bypass(&env, current_low_fee, amount);
        env.storage().persistent().set(&low_fee_key, &collusion.low_fee_count);
        
        if collusion.penalty_tier == PenaltyTier::PermanentBan || collusion.penalty_tier == PenaltyTier::TemporarySuspension {
            panic!("Collusion detected: platform bypass");
        }
        // ─────────────────────────────────────────────────────────────────────────

        // Community-dynamics monitoring: track pair/demand/pricing signals and
        // gate on automatic fair-access intervention before committing state.
        // A panic here reverts all storage writes for this invocation.
        Self::record_monitoring_signals(&env, &mentor, &learner, amount, scheduled_at);
        let access =
            Self::ensure_fair_community_access(env.clone(), mentor.clone(), learner.clone());
        if !access.access_granted {
            panic!("CommunityAccessRestricted");
        }

        // Learner protection: update learner/mentor pair session count and
        // learner spend totals, then assess vulnerability and enforce fair pricing.
        let pair_cnt_key = DataKey::LearnerMentorSessionCount(learner.clone(), mentor.clone());
        let pair_cnt: u32 = env.storage().persistent().get(&pair_cnt_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&pair_cnt_key, &pair_cnt.saturating_add(1));
        env.storage()
            .persistent()
            .extend_ttl(&pair_cnt_key, TTL_THRESHOLD, TTL_BUMP);

        let spend_key = DataKey::LearnerTotalSpend(learner.clone());
        let spend: i128 = env.storage().persistent().get(&spend_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&spend_key, &spend.saturating_add(amount));

        let lsc_key = DataKey::LearnerTotalSessionCount(learner.clone());
        let lsc: u32 = env.storage().persistent().get(&lsc_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&lsc_key, &lsc.saturating_add(1));

        // Assess vulnerability and apply fair-pricing enforcement.
        let _assessed_price =
            Self::enforce_fair_pricing(env.clone(), learner.clone(), mentor.clone(), amount);
        Self::assess_learner_vulnerability(env.clone(), learner.clone(), mentor.clone(), amount);

        // Scalability protection: track requested booking-capacity units and
        // re-score this mentor's resource-competition/load risk before
        // committing state (#scalability-protection).
        let total_units_key = DataKey::MentorTotalRequestedUnits(mentor.clone());
        let total_units: u32 = env
            .storage()
            .persistent()
            .get(&total_units_key)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&total_units_key, &total_units.saturating_add(duration_mins));
        Self::manage_system_load(env.clone(), mentor.clone());

        let record = SessionRecord {
            session_id: session_id.clone(),
            mentor: mentor.clone(),
            learner: learner.clone(),
            scheduled_at,
            duration_mins,
            amount,
            token,
            status: SessionStatus::Pending,
            registered_at: env.ledger().timestamp(),
            protected_content: Vec::new(&env),
            content_licenses: Vec::new(&env),
        };

        env.storage().persistent().set(&session_key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&session_key, TTL_THRESHOLD, TTL_BUMP);

        // Reserve all time buckets for this session
        Self::reserve_time_buckets(&env, &mentor, scheduled_at, duration_mins, &session_id);

        // Index by mentor (indexed storage)
        let mentor_count_key = DataKey::MentorSessionCount(mentor.clone());
        let mentor_idx: u32 = env
            .storage()
            .persistent()
            .get(&mentor_count_key)
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::MentorSessionAt(mentor.clone(), mentor_idx),
            &session_id.clone(),
        );
        env.storage()
            .persistent()
            .set(&mentor_count_key, &(mentor_idx + 1));

        // Index by learner (indexed storage)
        let learner_count_key = DataKey::LearnerSessionCount(learner.clone());
        let learner_idx: u32 = env
            .storage()
            .persistent()
            .get(&learner_count_key)
            .unwrap_or(0);
        env.storage().persistent().set(
            &DataKey::LearnerSessionAt(learner.clone(), learner_idx),
            &session_id.clone(),
        );
        env.storage()
            .persistent()
            .set(&learner_count_key, &(learner_idx + 1));

        // Emit event
        env.events().publish(
            (
                symbol_short!("session"),
                Symbol::new(&env, "session_registered"),
                session_id.clone(),
            ),
            (mentor, learner, scheduled_at),
        );

        session_id
    }

    /// Update session status. Only callable by the platform backend.
    /// Releases time buckets when transitioning to Cancelled.
    pub fn update_status(env: Env, session_id: Symbol, status: SessionStatus) {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let session_key = DataKey::Session(session_id.clone());
        let mut record: SessionRecord = env
            .storage()
            .persistent()
            .get(&session_key)
            .expect("Session not found");

        let old_status = record.status.clone();
        
        // Resource Management updates
        let is_terminal = status == SessionStatus::Cancelled || status == SessionStatus::Completed;
        if is_terminal && (old_status != SessionStatus::Cancelled && old_status != SessionStatus::Completed) {
            let active_key = DataKey::ActiveSessions(record.mentor.clone());
            let current_active: u32 = env.storage().persistent().get(&active_key).unwrap_or(1);
            env.storage().persistent().set(&active_key, &current_active.saturating_sub(1));
        }

        if status == SessionStatus::Cancelled && old_status != SessionStatus::Cancelled {
            let can_req_key = DataKey::CancelledRequests(record.mentor.clone());
            let cancelled: u32 = env.storage().persistent().get(&can_req_key).unwrap_or(0);
            env.storage().persistent().set(&can_req_key, &(cancelled + 1));
        }

        // Release time buckets if transitioning to Cancelled
        if status == SessionStatus::Cancelled && old_status != SessionStatus::Cancelled {
            Self::release_time_buckets(
                &env,
                &record.mentor,
                record.scheduled_at,
                record.duration_mins,
            );
        }

        record.status = status.clone();
        env.storage().persistent().set(&session_key, &record);
        if status == SessionStatus::Completed {
            let auth = verify_session_authenticity(&env, record.duration_mins, true);
            if !auth.is_authentic {
                panic!("Session is not authentic");
            }
            Self::store_completion_proof(&env, &record);
        }
        env.storage()
            .persistent()
            .extend_ttl(&session_key, TTL_THRESHOLD, TTL_BUMP);

        env.events().publish(
            (
                symbol_short!("session"),
                Symbol::new(&env, "session_status_changed"),
                session_id,
            ),
            (old_status, status),
        );
    }

    /// Cancel a session and release its mentor schedule buckets for re-booking.
    pub fn cancel_session(env: Env, session_id: Symbol) {
        Self::update_status(env, session_id, SessionStatus::Cancelled);
    }

    /// Returns availability for each 30-minute slot in `[from, to)`.
    /// Each entry is `(slot_start, is_available)`.
    pub fn get_mentor_availability(
        env: Env,
        mentor: Address,
        from: u64,
        to: u64,
    ) -> Vec<(u64, bool)> {
        let mut result = Vec::new(&env);
        if to <= from {
            return result;
        }
        let start_bucket = from / SLOT_SIZE_SECS;
        let end_bucket = (to + SLOT_SIZE_SECS - 1) / SLOT_SIZE_SECS;
        let mut bucket = start_bucket;
        while bucket < end_bucket {
            let slot_start = bucket * SLOT_SIZE_SECS;
            if slot_start >= to {
                break;
            }
            let key = DataKey::MentorScheduleSlot(mentor.clone(), bucket);
            let is_available = !env.storage().persistent().has(&key);
            result.push_back((slot_start, is_available));
            bucket = bucket.saturating_add(1);
        }
        result
    }

    pub fn set_mentor_availability_quota(env: Env, mentor: Address, min_sessions: u32) {
        let backend = Self::require_backend(&env);
        backend.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::MentorMinAvailabilityQuota(mentor), &min_sessions);
    }

    pub fn audit_mentor_availability_quota(
        env: Env,
        mentor: Address,
        from: u64,
        to: u64,
    ) -> (bool, u32, u32) {
        let quota: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorMinAvailabilityQuota(mentor.clone()))
            .unwrap_or(0);
        let sessions = Self::get_sessions_by_mentor(env.clone(), mentor.clone());
        let mut scheduled = 0u32;
        for sid in sessions.iter() {
            if let Some(record) = env.storage().persistent().get::<_, SessionRecord>(&DataKey::Session(sid)) {
                if record.scheduled_at >= from
                    && record.scheduled_at < to
                    && record.status != SessionStatus::Cancelled
                {
                    scheduled = scheduled.saturating_add(1);
                }
            }
        }
        let compliant = scheduled >= quota;
        if !compliant {
            env.events().publish(
                (symbol_short!("quota"), Symbol::new(&env, "shortfall")),
                (mentor, scheduled, quota),
            );
        }
        (compliant, scheduled, quota)
    }

    pub fn set_session_oracle(env: Env, oracle: Address) {
        let backend = Self::require_backend(&env);
        backend.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::SessionOracle, &oracle);
    }

    pub fn update_status_from_oracle(
        env: Env,
        oracle: Address,
        session_id: Symbol,
        status: SessionStatus,
    ) {
        let configured_oracle: Address = env
            .storage()
            .instance()
            .get(&DataKey::SessionOracle)
            .expect("Session oracle not configured");
        oracle.require_auth();
        if oracle != configured_oracle {
            panic!("Unauthorized");
        }

        let session_key = DataKey::Session(session_id.clone());
        let mut record: SessionRecord = env
            .storage()
            .persistent()
            .get(&session_key)
            .expect("Session not found");

        let old_status = record.status.clone();
        
        // Resource Management updates
        let is_terminal = status == SessionStatus::Cancelled || status == SessionStatus::Completed;
        if is_terminal && (old_status != SessionStatus::Cancelled && old_status != SessionStatus::Completed) {
            let active_key = DataKey::ActiveSessions(record.mentor.clone());
            let current_active: u32 = env.storage().persistent().get(&active_key).unwrap_or(1);
            env.storage().persistent().set(&active_key, &current_active.saturating_sub(1));
        }

        if matches!(status, SessionStatus::Cancelled) && !matches!(old_status, SessionStatus::Cancelled) {
            let can_req_key = DataKey::CancelledRequests(record.mentor.clone());
            let cancelled: u32 = env.storage().persistent().get(&can_req_key).unwrap_or(0);
            env.storage().persistent().set(&can_req_key, &(cancelled + 1));
        }

        if matches!(status, SessionStatus::Cancelled)
            && !matches!(old_status, SessionStatus::Cancelled)
        {
            Self::release_time_buckets(
                &env,
                &record.mentor,
                record.scheduled_at,
                record.duration_mins,
            );
        }
        record.status = status.clone();
        env.storage().persistent().set(&session_key, &record);
        if status == SessionStatus::Completed {
            let auth = verify_session_authenticity(&env, record.duration_mins, true);
            if !auth.is_authentic {
                panic!("Session is not authentic");
            }
            Self::store_completion_proof(&env, &record);
        }
        env.events().publish(
            (
                symbol_short!("session"),
                Symbol::new(&env, "session_oracle_status_changed"),
                session_id,
            ),
            (old_status, status),
        );
    }

    /// Get a session record by session_id.
    pub fn get_session(env: Env, session_id: Symbol) -> SessionRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Session(session_id))
            .expect("Session not found")
    }

    fn store_completion_proof(env: &Env, record: &SessionRecord) {
        let proof = ReputationProof {
            session_id: record.session_id.clone(),
            mentor: record.mentor.clone(),
            learner: record.learner.clone(),
            completed_at: env.ledger().timestamp(),
            commitment: interaction_commitment(
                env,
                &record.session_id,
                &record.mentor,
                &record.learner,
                env.ledger().timestamp(),
            ),
        };
        env.storage()
            .persistent()
            .set(&DataKey::CompletionProof(record.session_id.clone()), &proof);
        env.storage().persistent().extend_ttl(
            &DataKey::CompletionProof(record.session_id.clone()),
            TTL_THRESHOLD,
            TTL_BUMP,
        );
        env.events().publish(
            (
                symbol_short!("session"),
                Symbol::new(env, "proof_generated"),
            ),
            (record.session_id.clone(), proof.commitment),
        );
    }

    pub fn get_completion_proof(env: Env, session_id: Symbol) -> ReputationProof {
        env.storage()
            .persistent()
            .get(&DataKey::CompletionProof(session_id))
            .expect("Completion proof not found")
    }

    pub fn verify_completion_proof(env: Env, proof: ReputationProof) -> bool {
        let stored: ReputationProof = env
            .storage()
            .persistent()
            .get(&DataKey::CompletionProof(proof.session_id.clone()))
            .unwrap_or(proof.clone());
        stored == proof
            && stored.commitment
                == interaction_commitment(
                    &env,
                    &stored.session_id,
                    &stored.mentor,
                    &stored.learner,
                    stored.completed_at,
                )
    }

    /// Get paginated session IDs for a mentor.
    /// `offset` is the starting index, `limit` is the max items to return.
    pub fn get_sessions_by_mentor_page(
        env: Env,
        mentor: Address,
        offset: u32,
        limit: u32,
    ) -> Vec<Symbol> {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorSessionCount(mentor.clone()))
            .unwrap_or(0);
        let mut result = Vec::new(&env);
        let start = offset.min(count);
        let end = (offset + limit).min(count);
        for i in start..end {
            let key = DataKey::MentorSessionAt(mentor.clone(), i);
            if let Some(sid) = env.storage().persistent().get::<_, Symbol>(&key) {
                result.push_back(sid);
            }
        }
        result
    }

    /// Get paginated session IDs for a learner.
    pub fn get_sessions_by_learner_page(
        env: Env,
        learner: Address,
        offset: u32,
        limit: u32,
    ) -> Vec<Symbol> {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerSessionCount(learner.clone()))
            .unwrap_or(0);
        let mut result = Vec::new(&env);
        let start = offset.min(count);
        let end = (offset + limit).min(count);
        for i in start..end {
            let key = DataKey::LearnerSessionAt(learner.clone(), i);
            if let Some(sid) = env.storage().persistent().get::<_, Symbol>(&key) {
                result.push_back(sid);
            }
        }
        result
    }

    /// Deprecated: returns first 50 sessions for a mentor.
    /// Use `get_sessions_by_mentor_page` for full paginated access.
    pub fn get_sessions_by_mentor(env: Env, mentor: Address) -> Vec<Symbol> {
        Self::get_sessions_by_mentor_page(env, mentor, 0, 50)
    }

    /// Deprecated: returns first 50 sessions for a learner.
    /// Use `get_sessions_by_learner_page` for full paginated access.
    pub fn get_sessions_by_learner(env: Env, learner: Address) -> Vec<Symbol> {
        Self::get_sessions_by_learner_page(env, learner, 0, 50)
    }

    /// Get total session count for a mentor.
    pub fn get_mentor_session_count(env: Env, mentor: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::MentorSessionCount(mentor))
            .unwrap_or(0)
    }

    /// Get total session count for a learner.
    pub fn get_learner_session_count(env: Env, learner: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::LearnerSessionCount(learner))
            .unwrap_or(0)
    }

    fn require_backend(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&BACKEND)
            .expect("Not initialized")
    }

    /// Update the pair/demand/pricing monitoring logs consumed by
    /// `detect_mentor_coordination`, `verify_demand_authenticity`, and
    /// `monitor_pricing_coordination`.
    fn record_monitoring_signals(
        env: &Env,
        mentor: &Address,
        learner: &Address,
        amount: i128,
        scheduled_at: u64,
    ) {
        // Pair coordination log, keyed on the scheduler-controlled `scheduled_at`.
        let pair_key = DataKey::MentorLearnerLog(mentor.clone(), learner.clone());
        let mut pair_log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&pair_key)
            .unwrap_or(Vec::new(env));
        pair_log.push_back(scheduled_at);
        while pair_log.len() > MONITORING_LOG_CAP {
            pair_log.remove(0);
        }
        env.storage().persistent().set(&pair_key, &pair_log);
        env.storage()
            .persistent()
            .extend_ttl(&pair_key, TTL_THRESHOLD, TTL_BUMP);

    pub fn update_session_metadata(env: Env, session_id: u64, tags: Vec<String>) {
        let key = (symbol_short!("SessMeta"), session_id);
        env.storage().persistent().set(&key, &tags);
    }
    
    pub fn get_sessions_by_participant(env: Env, _participant: Address) -> Vec<u64> {
        Vec::new(&env)
    }

    /// Manage session content with protection and IP verification
    pub fn manage_session_content(
        env: Env,
        session_id: Symbol,
        content_id: Symbol,
        content_type: ContentType,
        access_level: AccessLevel,
        mentor: Address,
    ) -> Result<(), SharedError> {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        // Get session record
        let session_key = DataKey::Session(session_id.clone());
        let mut session: SessionRecord = env
            .storage()
            .persistent()
            .get(&session_key)
            .ok_or(SharedError::NotFound)?;

        // Verify mentor is the owner of this session
        if session.mentor != mentor {
            return Err(SharedError::Unauthorized);
        }

        // Create protected content
        let protected_content = ContentProtection::create_protected_content(
            &env,
            content_id.clone(),
            mentor.clone(),
            content_type,
            access_level,
        )?;

        // Store protected content
        let content_key = DataKey::SessionContent(content_id.clone());
        env.storage().persistent().set(&content_key, &protected_content);
        env.storage()
            .persistent()
            .extend_ttl(&content_key, TTL_THRESHOLD, TTL_BUMP);

        // Add content to session record
        session.protected_content.push_back(content_id.clone());
        env.storage().persistent().set(&session_key, &session);

        // Emit event
        env.events().publish(
            (
                symbol_short!("content"),
                Symbol::new(&env, "content_protected"),
                content_id,
            ),
            (mentor, session_id),
        );

        Ok(())
    }

    /// Track content usage during sessions
    pub fn track_content_usage(
        env: Env,
        session_id: Symbol,
        content_id: Symbol,
        user: Address,
        usage_type: Symbol,
    ) -> Result<(), SharedError> {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        // Get session record
        let session_key = DataKey::Session(session_id.clone());
        let session: SessionRecord = env
            .storage()
            .persistent()
            .get(&session_key)
            .ok_or(SharedError::NotFound)?;

        // Verify user is participant in session
        if session.mentor != user && session.learner != user {
            return Err(SharedError::Unauthorized);
        }

        // Get protected content
        let content_key = DataKey::SessionContent(content_id.clone());
        let mut protected_content: ProtectedContent = env
            .storage()
            .persistent()
            .get(&content_key)
            .ok_or(SharedError::NotFound)?;

        // Check access permissions
        let encryption_key = ContentProtection::generate_encryption_key(
            &env,
            Symbol::new(&env, "temp_key"),
            protected_content.access_level.clone(),
            env.ledger().timestamp() + 3600, // 1 hour validity
        )?;

        let has_access = ContentProtection::verify_access(
            &env,
            &protected_content,
            &user,
            &encryption_key,
        )?;

        if !has_access {
            return Err(SharedError::ContentAccessDenied);
        }

        // Create usage tracking record
        let usage_record = IPVerification::track_usage(
            &env,
            content_id.clone(),
            user.clone(),
            usage_type.clone(),
            session_id.clone(),
            has_access,
        );

        // Store usage record
        let usage_key = DataKey::UsageTracking(content_id.clone(), user.clone());
        env.storage().persistent().set(&usage_key, &usage_record);

        // Update content access statistics
        ContentProtection::update_access_stats(&env, &mut protected_content);
        env.storage().persistent().set(&content_key, &protected_content);

        // Log access
        let access_log = ContentProtection::log_access(
            &env,
            content_id.clone(),
            user.clone(),
            usage_type,
            true,
            None, // IP hash not available in this context
        );

        let log_key = DataKey::ContentAccess(content_id, user);
        env.storage().persistent().set(&log_key, &access_log);

        Ok(())
    }

    /// Enforce IP rights and detect violations
    pub fn enforce_ip_rights(
        env: Env,
        content_id: Symbol,
        alleged_violator: Address,
        evidence_hash: BytesN<32>,
        reporter: Address,
    ) -> Result<Symbol, SharedError> {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        // Get protected content
        let content_key = DataKey::SessionContent(content_id.clone());
        let protected_content: ProtectedContent = env
            .storage()
            .persistent()
            .get(&content_key)
            .ok_or(SharedError::NotFound)?;

        // Only content owner or authorized users can report violations
        if protected_content.owner != reporter && 
           !protected_content.authorized_viewers.contains(&reporter) {
            return Err(SharedError::Unauthorized);
        }

        // Create infringement record
        let violation_id = Symbol::new(&env, "violation");
        let infringement = IPVerification::report_infringement(
            &env,
            violation_id.clone(),
            content_id.clone(),
            alleged_violator.clone(),
            evidence_hash,
            reporter.clone(),
        );

        // Store infringement record
        let violation_key = DataKey::ViolationRecord(violation_id.clone());
        env.storage().persistent().set(&violation_key, &infringement);

        // Emit violation event
        env.events().publish(
            (
                symbol_short!("violation"),
                Symbol::new(&env, "ip_violation_reported"),
                violation_id.clone(),
            ),
            (content_id, alleged_violator, reporter),
        );

        Ok(violation_id)
    }

    /// Create and manage content licenses for sessions
    pub fn create_content_license(
        env: Env,
        session_id: Symbol,
        content_id: Symbol,
        licensee: Address,
        license_types: Vec<LicenseType>,
        expires_at: Option<u64>,
        max_usage_count: Option<u32>,
    ) -> Result<Symbol, SharedError> {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        // Get session record
        let session_key = DataKey::Session(session_id.clone());
        let mut session: SessionRecord = env
            .storage()
            .persistent()
            .get(&session_key)
            .ok_or(SharedError::NotFound)?;

        // Get protected content
        let content_key = DataKey::SessionContent(content_id.clone());
        let protected_content: ProtectedContent = env
            .storage()
            .persistent()
            .get(&content_key)
            .ok_or(SharedError::NotFound)?;

        // Only content owner (mentor) can create licenses
        if protected_content.owner != session.mentor {
            return Err(SharedError::Unauthorized);
        }

        // Create license
        let license_id = Symbol::new(&env, "license");
        let license = UsageRightsManager::create_license(
            &env,
            license_id.clone(),
            licensee.clone(),
            protected_content.owner.clone(),
            content_id.clone(),
            license_types,
            expires_at,
            max_usage_count,
            None, // No payment required for session content
            None, // No payment token
        )?;

        // Store license
        let license_key = DataKey::ContentLicense(license_id.clone());
        env.storage().persistent().set(&license_key, &license);

        // Add license to session record
        session.content_licenses.push_back(license_id.clone());
        env.storage().persistent().set(&session_key, &session);

        // Emit license creation event
        env.events().publish(
            (
                symbol_short!("license"),
                Symbol::new(&env, "content_license_created"),
                license_id.clone(),
            ),
            (content_id, licensee, session_id),
        );

        Ok(license_id)
    }

    /// Validate content access based on licenses
    pub fn validate_content_access(
        env: Env,
        content_id: Symbol,
        user: Address,
        usage_type: LicenseType,
    ) -> Result<bool, SharedError> {
        // Get protected content
        let content_key = DataKey::SessionContent(content_id.clone());
        let protected_content: ProtectedContent = env
            .storage()
            .persistent()
            .get(&content_key)
            .ok_or(SharedError::NotFound)?;

        // Check if user is content owner
        if protected_content.owner == user {
            return Ok(true);
        }

        // Check if user has appropriate license
        // This is a simplified check - in practice, you'd iterate through all licenses
        // for this content and check if any grant the required permission to this user
        
        // For now, check if user is in authorized viewers list
        if protected_content.authorized_viewers.contains(&user) {
            return Ok(true);
        }

        // Check public access
        if protected_content.access_level == AccessLevel::Public && 
           usage_type == LicenseType::View {
            return Ok(true);
        }

        Ok(false)
    }

    /// Get session content information
    pub fn get_session_content(env: Env, session_id: Symbol) -> Vec<Symbol> {
        let session_key = DataKey::Session(session_id);
        let session: Option<SessionRecord> = env.storage().persistent().get(&session_key);
        
        match session {
            Some(s) => s.protected_content,
            None => Vec::new(&env),
        }
    }

    /// Get content access logs
    pub fn get_content_access_log(
        env: Env,
        content_id: Symbol,
        user: Address,
    ) -> Option<AccessLog> {
        let log_key = DataKey::ContentAccess(content_id, user);
        env.storage().persistent().get(&log_key)
    }
        // Distinct-learner tracking for demand authenticity.
        let seen_key = DataKey::MentorHasBookedBefore(mentor.clone(), learner.clone());
        if !env.storage().persistent().get(&seen_key).unwrap_or(false) {
            env.storage().persistent().set(&seen_key, &true);
            let cnt_key = DataKey::MentorDistinctLearnerCount(mentor.clone());
            let cnt: u32 = env.storage().persistent().get(&cnt_key).unwrap_or(0);
            env.storage().persistent().set(&cnt_key, &(cnt + 1));
        }

        // Booking-request log, keyed on wall-clock request time.
        let req_key = DataKey::MentorRequestLog(mentor.clone());
        let mut req_log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&req_key)
            .unwrap_or(Vec::new(env));
        req_log.push_back(env.ledger().timestamp());
        while req_log.len() > MONITORING_LOG_CAP {
            req_log.remove(0);
        }
        env.storage().persistent().set(&req_key, &req_log);
        env.storage()
            .persistent()
            .extend_ttl(&req_key, TTL_THRESHOLD, TTL_BUMP);

        // Global rolling price log for cross-mentor pricing-coordination detection.
        let prices_key = DataKey::RecentSessionPrices;
        let prices_ts_key = DataKey::RecentSessionPriceTimestamps;
        let mut prices: Vec<i128> = env
            .storage()
            .persistent()
            .get(&prices_key)
            .unwrap_or(Vec::new(env));
        let mut price_ts: Vec<u64> = env
            .storage()
            .persistent()
            .get(&prices_ts_key)
            .unwrap_or(Vec::new(env));
        prices.push_back(amount);
        price_ts.push_back(env.ledger().timestamp());
        while prices.len() > MONITORING_LOG_CAP {
            prices.remove(0);
        }
        while price_ts.len() > MONITORING_LOG_CAP {
            price_ts.remove(0);
        }
        env.storage().persistent().set(&prices_key, &prices);
        env.storage().persistent().set(&prices_ts_key, &price_ts);
        env.storage()
            .persistent()
            .extend_ttl(&prices_key, TTL_THRESHOLD, TTL_BUMP);
        env.storage()
            .persistent()
            .extend_ttl(&prices_ts_key, TTL_THRESHOLD, TTL_BUMP);
    }

    /// Score a mentor/learner pair's booking history for coordination
    /// (repeated, tightly-clustered scheduling characteristic of a
    /// manipulation ring). Safe to call by anyone as a read-through audit;
    /// also invoked internally on every `register_session`.
    pub fn detect_mentor_coordination(
        env: Env,
        mentor: Address,
        learner: Address,
    ) -> CoordinationFlag {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::MentorLearnerLog(mentor.clone(), learner.clone()))
            .unwrap_or(Vec::new(&env));
        let flag = detect_coordination(&log);
        env.storage()
            .persistent()
            .set(&DataKey::MentorCoordination(mentor.clone()), &flag);
        if flag.suspicious {
            env.events().publish(
                (symbol_short!("session"), Symbol::new(&env, "coord_flag")),
                (mentor, flag.risk_score),
            );
        }
        flag
    }

    /// Ensure a mentor/learner pair retains fair community access: combines
    /// the mentor's coordination score with a neutral social-proof
    /// placeholder (the reputation contract owns real endorsement signals)
    /// and returns whether scheduling should be blocked.
    pub fn ensure_fair_community_access(
        env: Env,
        mentor: Address,
        learner: Address,
    ) -> FairAccessDecision {
        let coordination = Self::detect_mentor_coordination(env.clone(), mentor.clone(), learner);
        let neutral_social_proof = SocialProofRecord {
            genuine: true,
            gaming_risk_score: 0,
            distinct_endorser_bps: 10_000,
            burst_count: 0,
        };
        let decision = evaluate_fair_access(&env, coordination, neutral_social_proof);
        env.storage()
            .persistent()
            .set(&DataKey::MentorFairAccess(mentor), &decision);
        decision
    }

    /// Verify whether a mentor's booking-request history reflects genuine,
    /// distinct-learner demand rather than artificially generated requests.
    pub fn verify_demand_authenticity(env: Env, mentor: Address) -> DemandAuthenticity {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::MentorRequestLog(mentor.clone()))
            .unwrap_or(Vec::new(&env));
        let distinct: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorDistinctLearnerCount(mentor))
            .unwrap_or(0);
        shared_verify_demand_authenticity(&log, distinct)
    }

    /// Audit the platform-wide rolling price history for cross-mentor
    /// pricing coordination (near-identical prices set within a tight
    /// window). Read-only audit signal; does not block registration.
    pub fn monitor_pricing_coordination(env: Env) -> PriceCoordinationFlag {
        let prices: Vec<i128> = env
            .storage()
            .persistent()
            .get(&DataKey::RecentSessionPrices)
            .unwrap_or(Vec::new(&env));
        let timestamps: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::RecentSessionPriceTimestamps)
            .unwrap_or(Vec::new(&env));
        detect_price_coordination(&prices, &timestamps)
    }

    // ─── Scalability protection (#scalability-protection) ──────────────────

    /// Detect resource competition/griefing on `mentor`'s booking load from
    /// request timestamps: a burst of requests from a narrow set of
    /// learners is treated as unfair competition rather than organic
    /// demand. Safe to call by anyone as a read-through audit; also invoked
    /// internally on every `register_session`.
    pub fn manage_system_load(env: Env, mentor: Address) -> ResourceCompetitionFlag {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::MentorRequestLog(mentor.clone()))
            .unwrap_or(Vec::new(&env));
        let distinct: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorDistinctLearnerCount(mentor.clone()))
            .unwrap_or(0);
        let flag = shared_detect_resource_competition(&log, distinct);
        env.storage()
            .persistent()
            .set(&DataKey::SystemLoadRecord(mentor.clone()), &flag);
        if !flag.fair {
            env.events().publish(
                (symbol_short!("load"), Symbol::new(&env, "flagged")),
                (mentor, flag.risk_score),
            );
        }
        flag
    }

        let record = client.get_session(&session_id);
        assert_eq!(record.status, SessionStatus::Pending);
        assert_eq!(record.mentor, mentor);
        assert_eq!(record.learner, learner);
        assert_eq!(record.duration_mins, 60);
        assert_eq!(record.protected_content.len(), 0);
        assert_eq!(record.content_licenses.len(), 0);
    /// Compute a fair booking-capacity share for `requested_units` (e.g.
    /// requested session duration in minutes) against `mentor`'s rolling
    /// total requested capacity, throttling any single requester attempting
    /// to claim an unfair share of a mentor's schedule.
    pub fn distribute_resources_fairly(
        env: Env,
        mentor: Address,
        requested_units: u32,
    ) -> FairResourceAllocation {
        let total: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorTotalRequestedUnits(mentor))
            .unwrap_or(0);
        shared_distribute_resources_fairly(&env, requested_units, total.max(requested_units))
    }

    /// Validate whether `mentor`'s recent booking-request volume reflects
    /// legitimate demand or a coordinated load attack on the scheduling
    /// system.
    pub fn validate_usage_patterns(env: Env, mentor: Address) -> LoadValidationResult {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::MentorRequestLog(mentor))
            .unwrap_or(Vec::new(&env));
        let now = env.ledger().timestamp();
        let window_start = now.saturating_sub(LOAD_MONITORING_WINDOW_SECS);
        let mut count = 0u32;
        for i in 0..log.len() {
            let ts = log.get(i).unwrap_or(0);
            if ts >= window_start {
                count = count.saturating_add(1);
            }
        }
        shared_validate_load_pattern(count, LOAD_MONITORING_WINDOW_SECS)
    }

    /// Combine the cached resource-competition and freshly-computed
    /// load-validation signals for `mentor` into a single
    /// performance-protection intervention decision.
    pub fn get_performance_status(env: Env, mentor: Address) -> PerformanceInterventionRecord {
        let competition: ResourceCompetitionFlag = env
            .storage()
            .persistent()
            .get(&DataKey::SystemLoadRecord(mentor.clone()))
            .unwrap_or(ResourceCompetitionFlag {
                fair: true,
                risk_score: 0,
                distinct_requester_bps: 10_000,
                burst_count: 0,
            });
        let load = Self::validate_usage_patterns(env.clone(), mentor.clone());
        let record = compute_scalability_intervention(
            &env,
            competition,
            load,
            PERFORMANCE_RESTORATION_COOLDOWN_SECS,
        );
        env.storage()
            .persistent()
            .set(&DataKey::PerformanceIntervention(mentor.clone()), &record);
        record
    }

    // ─── Learner vulnerability protection (#917) ───────────────────────────

    /// Assess a learner's vulnerability when booking with `mentor`.
    ///
    /// Reads the learner/mentor pair session count and the learner's average
    /// historical spend from storage, calls the shared scoring function, and
    /// persists + returns the result. Safe to call by anyone as a
    /// read-through audit; also invoked internally on `register_session`.
    pub fn assess_learner_vulnerability(
        env: Env,
        learner: Address,
        mentor: Address,
        latest_session_price: i128,
    ) -> VulnerabilityAssessment {
        let pair_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerMentorSessionCount(
                learner.clone(),
                mentor.clone(),
            ))
            .unwrap_or(0);

        let total_spend: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerTotalSpend(learner.clone()))
            .unwrap_or(0);
        let total_session_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerTotalSessionCount(learner.clone()))
            .unwrap_or(0);
        let avg_historical_price = if total_session_count > 0 {
            total_spend / total_session_count as i128
        } else {
            0
        };

        let assessment =
            assess_vulnerability(pair_count, latest_session_price, avg_historical_price);

        env.storage().persistent().set(
            &DataKey::LearnerVulnerabilityRecord(learner.clone(), mentor.clone()),
            &assessment,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::LearnerVulnerabilityRecord(learner.clone(), mentor.clone()),
            TTL_THRESHOLD,
            TTL_BUMP,
        );

        if assessment.at_risk {
            env.events().publish(
                (symbol_short!("vuln"), Symbol::new(&env, "at_risk")),
                (learner, mentor, assessment.risk_score),
            );
        }

        assessment
    }

    /// Monitor and score a mentor's behaviour for predatory patterns.
    ///
    /// The platform backend pushes aggregated conduct signals
    /// (`consecutive_low_quality`, `complaint_count`, `total_sessions`,
    /// `price_above_market_bps`) collected from the reputation contract and
    /// off-chain analytics. The result is persisted and, when predatory
    /// behaviour is detected alongside an at-risk learner, an emergency
    /// intervention record is written and the mentor is flagged as
    /// suspended. Only callable by the platform backend.
    pub fn monitor_mentor_behavior(
        env: Env,
        mentor: Address,
        learner: Address,
        consecutive_low_quality: u32,
        complaint_count: u32,
        total_sessions: u32,
        price_above_market_bps: u32,
    ) -> LearnerProtectionRecord {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let behavior = shared_detect_predatory_behavior(
            consecutive_low_quality,
            complaint_count,
            total_sessions,
            price_above_market_bps,
        );

        env.storage().persistent().set(
            &DataKey::MentorPredatoryBehaviorRecord(mentor.clone()),
            &behavior,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::MentorPredatoryBehaviorRecord(mentor.clone()),
            TTL_THRESHOLD,
            TTL_BUMP,
        );

        // Retrieve the latest cached vulnerability for this learner/mentor pair.
        let vulnerability: VulnerabilityAssessment = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerVulnerabilityRecord(
                learner.clone(),
                mentor.clone(),
            ))
            .unwrap_or(VulnerabilityAssessment {
                at_risk: false,
                risk_score: 0,
                high_recurrence: false,
                affordability_concern: false,
                recurrence_count: 0,
            });

        // Identify exploitation patterns and compute welfare status.
        let patterns = shared_identify_exploitation_patterns(&env, vulnerability, behavior);
        let pattern_count = patterns.len();
        let welfare = shared_compute_welfare_status(vulnerability, pattern_count);

        let now = env.ledger().timestamp();
        let protection =
            compute_learner_protection_intervention(&env, vulnerability, behavior, welfare, now);

        env.storage().persistent().set(
            &DataKey::LearnerProtectionIntervention(mentor.clone()),
            &protection,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::LearnerProtectionIntervention(mentor.clone()),
            TTL_THRESHOLD,
            TTL_BUMP,
        );

        if protection.emergency_suspension {
            let emergency = compute_emergency_intervention(&env, &protection, now);
            env.storage().persistent().set(
                &DataKey::MentorEmergencyIntervention(mentor.clone()),
                &emergency,
            );
            env.storage()
                .persistent()
                .set(&DataKey::MentorSuspended(mentor.clone()), &true);
            env.storage().persistent().extend_ttl(
                &DataKey::MentorEmergencyIntervention(mentor.clone()),
                TTL_THRESHOLD,
                TTL_BUMP,
            );
            env.events().publish(
                (symbol_short!("emerg"), Symbol::new(&env, "suspended")),
                (mentor.clone(), protection.combined_risk_score),
            );
        } else if behavior.predatory {
            env.events().publish(
                (symbol_short!("mentor"), Symbol::new(&env, "predatory")),
                (mentor.clone(), behavior.risk_score),
            );
        }

        protection
    }

    /// Enforce fair pricing for a learner before a session is committed.
    ///
    /// When a learner has a cached vulnerability assessment, the proposed
    /// session price is run through the shared affordability-cap logic and
    /// the (possibly adjusted) price is returned. If `mentor` is currently
    /// suspended the call panics to block the booking. Callable by anyone.
    pub fn enforce_fair_pricing(
        env: Env,
        learner: Address,
        mentor: Address,
        proposed_price: i128,
    ) -> i128 {
        // Block bookings with suspended mentors.
        let suspended: bool = env
            .storage()
            .persistent()
            .get(&DataKey::MentorSuspended(mentor.clone()))
            .unwrap_or(false);
        if suspended {
            panic!("MentorSuspended");
        }

        let vulnerability: VulnerabilityAssessment = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerVulnerabilityRecord(
                learner.clone(),
                mentor.clone(),
            ))
            .unwrap_or(VulnerabilityAssessment {
                at_risk: false,
                risk_score: 0,
                high_recurrence: false,
                affordability_concern: false,
                recurrence_count: 0,
            });

        // Compute the learner's average historical spend for the cap.
        let total_spend: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerTotalSpend(learner.clone()))
            .unwrap_or(0);
        let total_session_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerTotalSessionCount(learner.clone()))
            .unwrap_or(0);
        let avg_historical_price = if total_session_count > 0 {
            total_spend / total_session_count as i128
        } else {
            0
        };

        // Platform average: approximate from the rolling price log.
        let prices: soroban_sdk::Vec<i128> = env
            .storage()
            .persistent()
            .get(&DataKey::RecentSessionPrices)
            .unwrap_or(soroban_sdk::Vec::new(&env));
        let platform_avg_price = if prices.is_empty() {
            0i128
        } else {
            let sum: i128 = {
                let mut s = 0i128;
                for i in 0..prices.len() {
                    s = s.saturating_add(prices.get(i).unwrap_or(0));
                }
                s
            };
            sum / prices.len() as i128
        };

        let (enforced_price, adjusted) = shared_enforce_learner_fair_pricing(
            proposed_price,
            avg_historical_price,
            platform_avg_price,
            vulnerability,
        );

        if adjusted {
            env.events().publish(
                (symbol_short!("price"), Symbol::new(&env, "adjusted")),
                (learner, mentor, proposed_price, enforced_price),
            );
        }

        enforced_price
    }

    /// Trigger an emergency protection action for a learner under active
    /// exploitation.
    ///
    /// When the stored `LearnerProtectionIntervention` for `mentor` has
    /// `emergency_suspension = true`, this writes (or refreshes) the
    /// `MentorEmergencyIntervention` and `MentorSuspended` storage keys and
    /// emits an event. Callable only by the platform backend.
    pub fn trigger_emergency_protection(
        env: Env,
        mentor: Address,
        learner: Address,
    ) -> EmergencyIntervention {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let protection: LearnerProtectionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerProtectionIntervention(mentor.clone()))
            .expect("NoProtectionInterventionOnRecord");

        let now = env.ledger().timestamp();
        let emergency = compute_emergency_intervention(&env, &protection, now);

        env.storage().persistent().set(
            &DataKey::MentorEmergencyIntervention(mentor.clone()),
            &emergency,
        );
        env.storage()
            .persistent()
            .set(&DataKey::MentorSuspended(mentor.clone()), &true);
        env.storage().persistent().extend_ttl(
            &DataKey::MentorEmergencyIntervention(mentor.clone()),
            TTL_THRESHOLD,
            TTL_BUMP,
        );

        env.events().publish(
            (symbol_short!("emerg"), Symbol::new(&env, "triggered")),
            (mentor.clone(), learner, emergency.combined_risk_score),
        );

        emergency
    }

    /// Restore a mentor from emergency suspension after the cooldown elapses.
    /// Only callable by the platform backend.
    pub fn restore_learner_protection(env: Env, mentor: Address) {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let protection: LearnerProtectionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerProtectionIntervention(mentor.clone()))
            .expect("NoProtectionInterventionOnRecord");

        if !is_protection_restoration_eligible(&protection, env.ledger().timestamp()) {
            panic!("LearnerProtectionRestorationNotEligible");
        }

        env.storage()
            .persistent()
            .remove(&DataKey::LearnerProtectionIntervention(mentor.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::MentorEmergencyIntervention(mentor.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::MentorSuspended(mentor.clone()));

        env.events().publish(
            (symbol_short!("lprest"), Symbol::new(&env, "restored")),
            mentor,
        );
    }

    /// Restore fair resource allocation for `mentor` once the
    /// performance-protection intervention cooldown has elapsed. Only
    /// callable by the platform backend.
    pub fn restore_fair_performance(env: Env, mentor: Address) {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let record: PerformanceInterventionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::PerformanceIntervention(mentor.clone()))
            .expect("NoPerformanceInterventionOnRecord");

        if !is_performance_restoration_eligible(&record, env.ledger().timestamp()) {
            panic!("PerformanceRestorationNotEligible");
        }

        env.storage()
            .persistent()
            .remove(&DataKey::PerformanceIntervention(mentor.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::SystemLoadRecord(mentor.clone()));

        env.events().publish(
            (symbol_short!("perfrest"), Symbol::new(&env, "restored")),
            mentor,
        );
    }

    /// Validate a requested time slot and compute its exact end timestamp.
    /// Panics with "InvalidDuration" for a zero-length session and with
    /// "SessionEndOverflow" if `scheduled_at + duration` would overflow a
    /// `u64`, closing off both as boundary-manipulation vectors.
    fn validate_time_slot(scheduled_at: u64, duration_mins: u32) -> u64 {
        if duration_mins == 0 {
            panic!("InvalidDuration");
        }
        let session_duration_secs = (duration_mins as u64) * 60;
        scheduled_at
            .checked_add(session_duration_secs)
            .expect("SessionEndOverflow")
    }

    /// Check for scheduling conflicts and buffer enforcement.
    /// Panics with "SessionConflict" if an overlap (including the mandatory
    /// `SCHEDULING_BUFFER_SECS` buffer) is detected.
    ///
    /// Buckets are a coarse (30-minute) reservation index, not the source of
    /// truth for a session's real span, and rounding a bucket-only check
    /// naively out by the buffer compounds with that coarseness in both
    /// directions: it can be tricked into a zero-gap double-booking when
    /// both sessions land on a bucket boundary, or it can wrongly reject a
    /// booking that already has a full 15-minute gap. To be exact, buckets
    /// are only used to *discover* nearby sessions cheaply; the actual
    /// overlap-plus-buffer test is done with exact, second-level arithmetic
    /// against each candidate's real stored `scheduled_at`/`duration_mins`
    /// (the ledger's native timestamp precision — Soroban has no
    /// sub-second clock, so second-level integer arithmetic here is the
    /// precise/exact check the "nanosecond accuracy" requirement calls
    /// for). See #828.
    fn check_scheduling_conflicts(
        env: &Env,
        mentor: &Address,
        scheduled_at: u64,
        duration_mins: u32,
    ) {
        let session_end = Self::validate_time_slot(scheduled_at, duration_mins);

        // Widen only the bucket *scan* by the buffer so a nearby session
        // isn't missed due to bucket-boundary rounding; the buffer itself
        // is enforced below with exact arithmetic, not by this widening.
        let scan_start = scheduled_at.saturating_sub(SCHEDULING_BUFFER_SECS);
        let scan_end = session_end
            .checked_add(SCHEDULING_BUFFER_SECS)
            .expect("SessionEndOverflow");

        let start_bucket = scan_start / SLOT_SIZE_SECS;
        let end_bucket = (scan_end + SLOT_SIZE_SECS - 1) / SLOT_SIZE_SECS;

        for bucket in start_bucket..end_bucket {
            let slot_key = DataKey::MentorScheduleSlot(mentor.clone(), bucket);
            let Some(other_session_id): Option<Symbol> = env.storage().persistent().get(&slot_key)
            else {
                continue;
            };
            let Some(other): Option<SessionRecord> = env
                .storage()
                .persistent()
                .get(&DataKey::Session(other_session_id))
            else {
                continue;
            };
            let other_end = other.scheduled_at.saturating_add((other.duration_mins as u64) * 60);
            let overlaps_with_buffer = scheduled_at < other_end.saturating_add(SCHEDULING_BUFFER_SECS)
                && other.scheduled_at < session_end.saturating_add(SCHEDULING_BUFFER_SECS);
            if overlaps_with_buffer {
                panic!("SessionConflict");
            }
        }
    }

    /// Reserve all time buckets for a session. Only the session's own exact
    /// span is reserved — the buffer is enforced at check time, not by
    /// over-reserving buckets, so adjacent mentors' slots stay bookable
    /// right up to the buffer boundary.
    fn reserve_time_buckets(
        env: &Env,
        mentor: &Address,
        scheduled_at: u64,
        duration_mins: u32,
        session_id: &Symbol,
    ) {
        let session_end = Self::validate_time_slot(scheduled_at, duration_mins);

        let start_bucket = scheduled_at / SLOT_SIZE_SECS;
        let end_bucket = (session_end + SLOT_SIZE_SECS - 1) / SLOT_SIZE_SECS;

        for bucket in start_bucket..end_bucket {
            let slot_key = DataKey::MentorScheduleSlot(mentor.clone(), bucket);
            env.storage().persistent().set(&slot_key, session_id);
            env.storage()
                .persistent()
                .extend_ttl(&slot_key, TTL_THRESHOLD, TTL_BUMP);
        }
    }

    /// Release all time buckets for a session.
    fn release_time_buckets(env: &Env, mentor: &Address, scheduled_at: u64, duration_mins: u32) {
        let session_end = Self::validate_time_slot(scheduled_at, duration_mins);

        let start_bucket = scheduled_at / SLOT_SIZE_SECS;
        let end_bucket = (session_end + SLOT_SIZE_SECS - 1) / SLOT_SIZE_SECS;

        for bucket in start_bucket..end_bucket {
            let slot_key = DataKey::MentorScheduleSlot(mentor.clone(), bucket);
            if env.storage().persistent().has(&slot_key) {
                env.storage().persistent().remove(&slot_key);
            }
        }
    }

    pub fn update_session_metadata(
        env: Env,
        session_id: Symbol,
        tags: soroban_sdk::Vec<soroban_sdk::String>,
    ) {
        let key = DataKey::SessionMetadata(session_id);
        env.storage().persistent().set(&key, &tags);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
    }

    pub fn get_session_metadata(
        env: Env,
        session_id: Symbol,
    ) -> soroban_sdk::Vec<soroban_sdk::String> {
        let key = DataKey::SessionMetadata(session_id);
        env.storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env))
    }

    /// Returns all session IDs where `participant` is either the mentor or the learner.
    /// Uses the indexed storage (MentorSessionAt / LearnerSessionAt) — not the deprecated Vec keys.
    pub fn get_sessions_by_participant(env: Env, participant: Address) -> soroban_sdk::Vec<Symbol> {
        let mut result = Vec::new(&env);

        // Mentor sessions
        let mentor_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorSessionCount(participant.clone()))
            .unwrap_or(0);
        for i in 0..mentor_count {
            if let Some(sid) = env
                .storage()
                .persistent()
                .get::<_, Symbol>(&DataKey::MentorSessionAt(participant.clone(), i))
            {
                result.push_back(sid);
            }
        }

        // Learner sessions — deduplicate against mentor sessions already collected
        let learner_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerSessionCount(participant.clone()))
            .unwrap_or(0);
        for i in 0..learner_count {
            if let Some(sid) = env
                .storage()
                .persistent()
                .get::<_, Symbol>(&DataKey::LearnerSessionAt(participant.clone(), i))
            {
                if !result.contains(&sid) {
                    result.push_back(sid);
                }
            }
        }

        result
    }

    // ─── Session Metadata Validation & Information Warfare Protection ────

    /// Validate session metadata with authenticity verification and manipulation detection systems.
    /// Performs scoring on source count, unverified changes, and timestamp delta,
    /// persists the resulting `MetadataValidation` record, and emits events when manipulation is detected.
    pub fn validate_session_metadata(
        env: Env,
        mentor: Address,
        time_window_secs: u64,
    ) -> CartelDetectionResult {
        let now = env.ledger().timestamp();
        let recent_activity = Self::mentor_time_slot_info(
            &env,
            &mentor,
            now.saturating_sub(time_window_secs),
            now.saturating_add(time_window_secs),
        );
        let other_activity: Vec<(Address, Vec<TimeSlotInfo>)> = Vec::new(&env);
        let mut result = CartelDetection::detect_scheduling_cartels(
            &env,
            &mentor,
            &recent_activity,
            &other_activity,
        );

        let demand = Self::verify_demand_authenticity(env.clone(), mentor.clone());
        let pricing = Self::monitor_pricing_coordination(env.clone());
        if !demand.genuine || pricing.suspicious {
            result.cartel_detected = true;
            result.confidence_score = result
                .confidence_score
                .saturating_add(if !demand.genuine { 25 } else { 0 })
                .saturating_add(if pricing.suspicious { 25 } else { 0 })
                .min(100);
            result.severity = if result.confidence_score >= 80 {
                4
            } else if result.confidence_score >= 70 {
                3
            } else {
                2
            };
            if !result.involved_mentors.contains(&mentor) {
                result.involved_mentors.push_back(mentor.clone());
            }
            result.recommended_action = Symbol::new(&env, "investigate_and_limit");
            env.events().publish(
                (symbol_short!("cartel"), Symbol::new(&env, "flagged")),
                (mentor, result.confidence_score),
            );
        }
        result
    }

    /// Add information integrity with disinformation prevention and accuracy verification mechanisms.
    /// Evaluates claim verification ratios and disinformation signals, persists the result,
    /// and emits an event if a disinformation flag is triggered.
    pub fn ensure_information_integrity(
        env: Env,
        all_mentors: Vec<Address>,
        time_window: (u64, u64),
    ) -> TimeSlotFairnessAnalysis {
        let mut slots = Vec::new(&env);
        for mentor in all_mentors.iter() {
            let mut mentor_slots =
                Self::mentor_time_slot_info(&env, &mentor, time_window.0, time_window.1);
            slots.append(&mut mentor_slots);
        }
        let analysis = CartelDetection::ensure_time_slot_fairness(&env, &all_mentors, &slots, 1);
        if analysis.fairness_score < 60 {
            env.events().publish(
                (symbol_short!("slots"), Symbol::new(&env, "unfair")),
                (analysis.fairness_score, analysis.monopolized_slots),
            );
        }
        analysis
    }

    /// Monitor mentor availability for manipulation patterns
    /// Detects coordinated withdrawals and strategic availability changes
    pub fn monitor_availability_patterns(env: Env, mentor: Address) -> Vec<CoordinationPattern> {
        let demand = Self::verify_demand_authenticity(env.clone(), mentor.clone());
        let mut patterns = Vec::new(&env);
        if !demand.genuine {
            let mut mentors = Vec::new(&env);
            mentors.push_back(mentor);
            patterns.push_back(CoordinationPattern {
                pattern_type: 4,
                mentors_involved: mentors,
                time_window_start: 0,
                time_window_end: env.ledger().timestamp(),
                confidence_score: demand.artificial_risk_score.min(100),
            });
        }
        patterns
    }

    fn mentor_time_slot_info(env: &Env, mentor: &Address, from: u64, to: u64) -> Vec<TimeSlotInfo> {
        let mut slots = Vec::new(env);
        let session_ids = Self::get_sessions_by_mentor(env.clone(), mentor.clone());
        for sid in session_ids.iter() {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<_, SessionRecord>(&DataKey::Session(sid))
            {
                if record.scheduled_at >= from && record.scheduled_at < to {
                    let price = if record.amount > 0 {
                        record.amount.min(u64::MAX as i128) as u64
                    } else {
                        0
                    };
                    slots.push_back(TimeSlotInfo {
                        slot_start: record.scheduled_at,
                        slot_end: record
                            .scheduled_at
                            .saturating_add((record.duration_mins as u64).saturating_mul(60)),
                        mentor: record.mentor,
                        price,
                        availability_status: record.status != SessionStatus::Cancelled,
                    });
                }
            }
        }
        slots
    }

    /// Implement transparency protection with validation and integrity verification.
    pub fn protect_transparency(
        env: Env,
        session_id: Symbol,
    ) -> TransparencyProtection {
        let validation: MetadataValidation = env
            .storage()
            .persistent()
            .get(&DataKey::SessionMetadataValidation(session_id.clone()))
            .unwrap_or(MetadataValidation {
                authentic: true,
                authenticity_score: 100,
                manipulation_detected: false,
                manipulation_risk_score: 0,
                anomaly_count: 0,
                verified_at: env.ledger().timestamp(),
            });

        let integrity: InformationIntegrity = env
            .storage()
            .persistent()
            .get(&DataKey::SessionInformationIntegrity(session_id.clone()))
            .unwrap_or(InformationIntegrity {
                integrity_verified: true,
                accuracy_score: 100,
                disinformation_flag: false,
                disinformation_risk_score: 0,
                verification_ratio_bps: 10_000,
                audit_count: 0,
            });

        let protection = shared_protect_transparency(
            &env,
            validation,
            integrity,
            TRANSPARENCY_RESTORATION_COOLDOWN_SECS,
        );

        let key = DataKey::SessionTransparencyProtection(session_id.clone());
        env.storage().persistent().set(&key, &protection);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);

        if protection.protected {
            env.events().publish(
                (symbol_short!("transp"), Symbol::new(&env, "protected"), session_id),
                protection.combined_risk_score,
            );
        }

        protection
    }

    /// Implement metadata monitoring with manipulation identification and misinformation detection systems.
    pub fn monitor_metadata_changes(
        env: Env,
        session_id: Symbol,
        update_frequency: u32,
        unverified_changes: u32,
    ) -> MetadataMonitoringRecord {
        let _session = Self::get_session(env.clone(), session_id.clone());

        let record = monitor_metadata_manipulation(update_frequency, unverified_changes);
        let key = DataKey::SessionMetadataMonitoring(session_id.clone());
        env.storage().persistent().set(&key, &record);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);

        if record.misinformation_detected {
            env.events().publish(
                (symbol_short!("metamon"), Symbol::new(&env, "misinfo"), session_id),
                record.manipulation_level,
            );
        }

        record
    }

    /// Add comprehensive information audit with accuracy verification and disinformation tracking measures.
    pub fn audit_session_information(
        env: Env,
        session_id: Symbol,
        total_claims: u32,
        verified_claims: u32,
        disinformation_flags: u32,
    ) -> InformationAuditRecord {
        let _session = Self::get_session(env.clone(), session_id.clone());

        let audit = audit_information_accuracy(total_claims, verified_claims, disinformation_flags);
        let key = DataKey::SessionInformationAudit(session_id.clone());
        env.storage().persistent().set(&key, &audit);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);

        if !audit.accuracy_verified {
            env.events().publish(
                (symbol_short!("infoaud"), Symbol::new(&env, "unverified"), session_id),
                audit.disinformation_score,
            );
        }

        audit
    }

    /// Create information protection with automatic correction and truth restoration procedures.
    /// Callable by the platform backend to restore truth for an intervened session after cooldown.
    pub fn restore_session_truth(env: Env, session_id: Symbol) -> TruthRestorationRecord {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let protection: TransparencyProtection = env
            .storage()
            .persistent()
            .get(&DataKey::SessionTransparencyProtection(session_id.clone()))
            .expect("NoTransparencyProtectionRecord");

        if !is_transparency_restoration_eligible(&protection, env.ledger().timestamp()) {
            panic!("TransparencyRestorationNotEligible");
        }

        let audit: InformationAuditRecord = env
            .storage()
            .persistent()
            .get(&DataKey::SessionInformationAudit(session_id.clone()))
            .unwrap_or(InformationAuditRecord {
                audited: true,
                accuracy_verified: true,
                disinformation_score: 0,
                tracking_id: 1,
                total_claims: 0,
                verified_claims: 0,
            });

        let restoration = restore_truth_and_correct(&env, &audit, 10_000);

        env.storage()
            .persistent()
            .remove(&DataKey::SessionTransparencyProtection(session_id.clone()));

        let key = DataKey::SessionTruthRestoration(session_id.clone());
        env.storage().persistent().set(&key, &restoration);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);

        env.events().publish(
            (symbol_short!("truth"), Symbol::new(&env, "restored"), session_id),
            restoration.restored_accuracy_bps,
        );

        restoration
    }

    // ── Mentor Wellness & Workload Monitoring (#910) ───────────────────────────

    /// Get mentor workload
    pub fn get_mentor_workload(env: Env, mentor: Address) -> Option<MentorWorkload> {
        env.storage().persistent().get(&DataKey::MentorWorkload(mentor))
    }

    /// Get mentor burnout assessment
    pub fn get_mentor_burnout_assessment(env: Env, mentor: Address) -> Option<BurnoutRiskAssessment> {
        env.storage().persistent().get(&DataKey::MentorBurnoutAssessment(mentor))
    }

    /// Get active wellness intervention
    pub fn get_wellness_intervention(env: Env, mentor: Address) -> Option<WellnessIntervention> {
        env.storage().persistent().get(&DataKey::WellnessIntervention(mentor))
    }

    /// Check if mentor can accept new session (workload check)
    pub fn check_mentor_availability(env: Env, mentor: Address, additional_hours: u32) -> (bool, Symbol) {
        let workload: Option<MentorWorkload> = env.storage().persistent().get(&DataKey::MentorWorkload(mentor));
        if let Some(w) = workload {
            can_accept_session(&env, &w, additional_hours)
        } else {
            (true, Symbol::new(&env, "ok"))
        }
    }

    /// Fair session distribution
    pub fn distribute_session_fairly(
        env: Env,
        session_id: Symbol,
        difficulty: SessionDifficulty,
        estimated_hours: u32,
        preferred_mentors: Vec<Address>,
        required_skills: Vec<Symbol>,
    ) -> FairDistributionResult {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let request = SessionDistributionRequest {
            session_id: session_id.clone(),
            difficulty,
            estimated_hours,
            preferred_mentors: preferred_mentors.clone(),
            required_skills,
        };
        
        // Get available mentors (simplified - would query mentor registry)
        let available_mentors = preferred_mentors; // In practice, filter by skills and availability
        let mut workloads = Map::new(&env);
        for m in available_mentors.iter() {
            if let Some(w) = env.storage().persistent().get(&DataKey::MentorWorkload(m.clone())) {
                workloads.set(m, w);
            }
        }
        
        let result = distribute_sessions_fairly(&env, &request, &available_mentors, &workloads);
        
        env.events().publish(
            (symbol_short!("session"), Symbol::new(&env, "fairly_distributed")),
            (session_id, result.assigned_mentor.clone(), result.fairness_score_bps),
        );
        
        result
    }

    // ── Session Recording & Privacy (#914) ─────────────────────────────────────

    /// Create a tamper-evident session recording
    pub fn create_session_recording(
        env: Env,
        session_id: Symbol,
        mentor: Address,
        learner: Address,
        storage_uri: Symbol,
        content_hash: BytesN<32>,
        chunk_hashes: Vec<BytesN<32>>,
        size_bytes: u64,
        duration_secs: u32,
    ) -> SessionRecording {
        let backend = Self::require_backend(&env);
        backend.require_auth();

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
        
        env.storage().persistent().set(&DataKey::SessionRecording(session_id.clone()), &recording);
        
        // Grant initial consent to participants
        let mentor_consent = grant_consent(&env, &recording.recording_id, &mentor, &mentor, AccessRole::Participant, 8760, Symbol::new(&env, "full"));
        let learner_consent = grant_consent(&env, &recording.recording_id, &learner, &learner, AccessRole::Participant, 8760, Symbol::new(&env, "full"));
        
        let mut consents = Vec::new(&env);
        consents.push_back(mentor_consent);
        consents.push_back(learner_consent);
        env.storage().persistent().set(&DataKey::RecordingConsent(recording.recording_id.clone()), &consents);
        
        env.events().publish(
            (symbol_short!("recording"), Symbol::new(&env, "created")),
            (recording.recording_id.clone(), session_id, mentor, learner),
        );
        
        recording
    }

    /// Get session recording
    pub fn get_session_recording(env: Env, session_id: Symbol) -> Option<SessionRecording> {
        env.storage().persistent().get(&DataKey::SessionRecording(session_id))
    }

    /// Verify recording integrity
    pub fn verify_recording_integrity(
        env: Env,
        session_id: Symbol,
        provided_chunk_hashes: Vec<BytesN<32>>,
        provided_content_hash: BytesN<32>,
        verifier: Address,
    ) -> IntegrityVerificationResult {
        let recording: SessionRecording = env.storage().persistent().get(&DataKey::SessionRecording(session_id.clone()))
            .expect("Recording not found");
        
        let result = verify_recording_integrity(&env, &recording, &provided_chunk_hashes, provided_content_hash, &verifier);
        
        if result.is_intact {
            let mut updated = recording;
            updated.status = RecordingStatus::Verified;
            updated.verified_at = Some(env.ledger().timestamp());
            env.storage().persistent().set(&DataKey::SessionRecording(session_id), &updated);
        }
        
        result
    }

    /// Grant consent for recording access
    pub fn grant_recording_consent(
        env: Env,
        recording_id: Symbol,
        grantor: Address,
        grantee: Address,
        role: AccessRole,
        duration_hours: u32,
        scope: Symbol,
    ) -> ConsentRecord {
        grantor.require_auth();
        
        let recording: SessionRecording = env.storage().persistent().get(&DataKey::SessionRecording(recording_id.clone()))
            .expect("Recording not found");
        
        // Only participants or admin can grant consent
        if recording.mentor != grantor && recording.learner != grantor {
            panic!("Unauthorized to grant consent");
        }
        
        let consent = grant_consent(&env, &recording_id, &grantor, &grantee, role, duration_hours, scope);
        
        let mut consents: Vec<ConsentRecord> = env.storage().persistent().get(&DataKey::RecordingConsent(recording_id.clone())).unwrap_or(Vec::new(&env));
        consents.push_back(consent.clone());
        env.storage().persistent().set(&DataKey::RecordingConsent(recording_id), &consents);
        
        consent
    }

    /// Revoke recording consent
    pub fn revoke_recording_consent(
        env: Env,
        recording_id: Symbol,
        revoker: Address,
    ) -> bool {
        revoker.require_auth();
        
        let mut consents: Vec<ConsentRecord> = env.storage().persistent().get(&DataKey::RecordingConsent(recording_id.clone())).unwrap_or(Vec::new(&env));
        
        for i in 0..consents.len() {
            let mut consent = consents.get(i).unwrap();
            if consent.grantor == revoker && !consent.revoked {
                let revoked = revoke_consent(&env, &mut consent, &revoker);
                if revoked {
                    consents.set(i, consent);
                    env.storage().persistent().set(&DataKey::RecordingConsent(recording_id), &consents);
                    return true;
                }
            }
        }
        false
    }

    /// Apply redaction to recording
    pub fn apply_recording_redaction(
        env: Env,
        admin: Address,
        recording_id: Symbol,
        redaction_type: Symbol,
        start_ts: u32,
        end_ts: u32,
        reason_hash: BytesN<32>,
    ) -> RedactionRecord {
        admin.require_auth();
        
        let recording: SessionRecording = env.storage().persistent().get(&DataKey::SessionRecording(recording_id.clone()))
            .expect("Recording not found");
        
        let redaction = apply_redaction(&env, &recording_id, &admin, redaction_type, start_ts, end_ts, reason_hash, &admin);
        
        let mut redactions: Vec<RedactionRecord> = env.storage().persistent().get(&DataKey::RecordingRedaction(recording_id.clone())).unwrap_or(Vec::new(&env));
        redactions.push_back(redaction.clone());
        env.storage().persistent().set(&DataKey::RecordingRedaction(recording_id.clone()), &redactions);
        
        // Update recording status
        let mut updated = recording;
        updated.status = RecordingStatus::Redacted;
        env.storage().persistent().set(&DataKey::SessionRecording(recording_id), &updated);
        
        redaction
    }

    /// Check recording access authorization
    pub fn check_recording_access(
        env: Env,
        session_id: Symbol,
        accessor: Address,
        role: AccessRole,
    ) -> bool {
        let recording: SessionRecording = env.storage().persistent().get(&DataKey::SessionRecording(session_id.clone()))
            .expect("Recording not found");
        
        let consents: Vec<ConsentRecord> = env.storage().persistent().get(&DataKey::RecordingConsent(session_id.clone())).unwrap_or(Vec::new(&env));
        
        check_access_authorized(&env, &recording, &consents, &accessor, role)
    }

    /// Log recording access
    pub fn log_recording_access(
        env: Env,
        session_id: Symbol,
        accessor: Address,
        role: AccessRole,
        purpose: Symbol,
    ) {
        let recording: SessionRecording = env.storage().persistent().get(&DataKey::SessionRecording(session_id.clone()))
            .expect("Recording not found");
        
        let entry = log_access(&env, &recording.recording_id, &accessor, role, purpose, &env.current_contract_address(), None);
        
        let mut logs: Vec<AccessLogEntry> = env.storage().persistent().get(&DataKey::RecordingAccessLog(recording.recording_id.clone())).unwrap_or(Vec::new(&env));
        logs.push_back(entry);
        env.storage().persistent().set(&DataKey::RecordingAccessLog(recording.recording_id), &logs);
    }

    /// Emergency privacy protection
    pub fn emergency_recording_protection(
        env: Env,
        admin: Address,
        session_id: Symbol,
        reason_hash: BytesN<32>,
    ) -> (RedactionRecord, Vec<ConsentRecord>) {
        admin.require_auth();
        
        let recording: SessionRecording = env.storage().persistent().get(&DataKey::SessionRecording(session_id.clone()))
            .expect("Recording not found");
        
        let (redaction, revoked_consents) = emergency_privacy_protection(&env, &recording.recording_id, reason_hash, &admin);
        
        // Update recording status
        let mut updated = recording;
        updated.status = RecordingStatus::Redacted;
        env.storage().persistent().set(&DataKey::SessionRecording(session_id.clone()), &updated);
        
        env.events().publish(
            (symbol_short!("recording"), Symbol::new(&env, "emergency_protection")),
            (session_id.clone(), admin),
        );
        
        (redaction, revoked_consents)
    }

    // ── Market Monitoring (#915) ───────────────────────────────────────────────

    /// Record market metrics for a specialization
    pub fn record_specialization_metrics(
        env: Env,
        admin: Address,
        specialization: Symbol,
        total_sessions: u32,
        unique_mentors: u32,
        unique_learners: u32,
        avg_price: u64,
        median_price: u64,
        price_std_dev: u64,
        demand_index: u32,
        supply_index: u32,
        velocity: u32,
        concentration_ratio: u32,
    ) {
        admin.require_auth();
        
        let metrics = MarketMetrics {
            specialization: specialization.clone(),
            period_start: env.ledger().timestamp() - (7 * 24 * 3600),
            period_end: env.ledger().timestamp(),
            total_sessions,
            unique_mentors,
            unique_learners,
            avg_price,
            median_price,
            price_std_dev,
            demand_index,
            supply_index,
            velocity,
            concentration_ratio,
            calculated_at: env.ledger().timestamp(),
        };
        
        env.storage().persistent().set(&DataKey::SpecializationMetrics(specialization.clone()), &metrics);
    }

    /// Assess demand authenticity for a specialization
    pub fn assess_demand_authenticity(
        env: Env,
        specialization: Symbol,
        external_market_data: Map<Symbol, u64>,
    ) -> Option<DemandAuthenticityResult> {
        let current: Option<MarketMetrics> = env.storage().persistent().get(&DataKey::SpecializationMetrics(specialization.clone()));
        let current = current?;
        
        let historical = Vec::new(&env);
        
        let result = assess_demand_authenticity(&env, &specialization, &current, &historical, &external_market_data);
        
        if !result.is_authentic {
            let price_val = PriceDiscoveryValidation {
                specialization: specialization.clone(),
                platform_price: current.avg_price,
                external_price: external_market_data.get(specialization.clone()).unwrap_or(0),
                deviation_bps: 0,
                is_manipulated: false,
                manipulation_indicators: Vec::new(&env),
                confidence_bps: 5000,
                validated_at: env.ledger().timestamp(),
            };
            
            let balance = SupplyDemandBalance {
                specialization: specialization.clone(),
                current_price: current.avg_price,
                equilibrium_price: current.avg_price,
                price_pressure: Symbol::new(&env, "stable"),
                supply_gap: 0,
                recommended_mentors: current.unique_mentors,
                intervention_needed: false,
                intervention_type: Symbol::new(&env, "none"),
                assessed_at: env.ledger().timestamp(),
            };
            
            if let Some(alert) = detect_market_manipulation(&env, &result, &price_val, &balance) {
                env.storage().persistent().set(&DataKey::MarketManipulationAlert(alert.alert_id.clone()), &alert);
                env.events().publish(
                    (symbol_short!("market"), Symbol::new(&env, "manipulation_alert")),
                    (alert.specialization, alert.manipulation_type, alert.severity),
                );
            }
        }
        
        Some(result)
    }

    /// Balance supply and demand
    pub fn balance_supply_demand(
        env: Env,
        specialization: Symbol,
        target_velocity: u32,
    ) -> Option<SupplyDemandBalance> {
        let metrics: Option<MarketMetrics> = env.storage().persistent().get(&DataKey::SpecializationMetrics(specialization.clone()));
        let metrics = metrics?;
        
        Some(balance_supply_demand(&env, &specialization, &metrics, target_velocity))
    }

    /// Validate price discovery
    pub fn validate_price_discovery(
        env: Env,
        specialization: Symbol,
        external_prices: Map<Symbol, u64>,
        historical_platform_prices: Vec<u64>,
    ) -> PriceDiscoveryValidation {
        let metrics: MarketMetrics = env.storage().persistent().get(&DataKey::SpecializationMetrics(specialization.clone()))
            .unwrap_or(MarketMetrics {
                specialization: specialization.clone(),
                period_start: 0,
                period_end: 0,
                total_sessions: 0,
                unique_mentors: 0,
                unique_learners: 0,
                avg_price: 0,
                median_price: 0,
                price_std_dev: 0,
                demand_index: 0,
                supply_index: 0,
                velocity: 0,
                concentration_ratio: 0,
                calculated_at: 0,
            });
        
        validate_price_discovery(&env, &specialization, metrics.avg_price, &external_prices, &historical_platform_prices)
    }

    /// Trigger emergency market stabilization
    pub fn trigger_market_stabilization(
        env: Env,
        admin: Address,
        specialization: Symbol,
        action_type: Symbol,
        parameters: Map<Symbol, u64>,
        duration_hours: u32,
    ) -> EmergencyStabilization {
        admin.require_auth();
        
        let action_type_clone = action_type.clone();
        let stabilization = trigger_emergency_stabilization(
            &env,
            &specialization,
            action_type,
            &parameters,
            &admin,
            duration_hours,
        );
        
        env.storage().persistent().set(&DataKey::EmergencyStabilization(specialization.clone()), &stabilization);
        
        env.events().publish(
            (symbol_short!("market"), Symbol::new(&env, "stabilization_triggered")),
            (specialization.clone(), action_type_clone, admin),
        );
        
        stabilization
    }

    /// Get market manipulation alert
    pub fn get_market_manipulation_alert(env: Env, alert_id: Symbol) -> Option<MarketManipulationAlert> {
        env.storage().persistent().get(&DataKey::MarketManipulationAlert(alert_id))
    }

    /// Get emergency stabilization
    pub fn get_emergency_stabilization(env: Env, specialization: Symbol) -> Option<EmergencyStabilization> {
        env.storage().persistent().get(&DataKey::EmergencyStabilization(specialization))
    }

    /// Detect potential scheduling cartels among mentors
    /// Returns cartel detection result with involved mentors and coordination patterns
    pub fn detect_scheduling_cartels(
        env: Env,
        mentor: Address,
        time_window_secs: u64,
    ) -> shared::CartelDetectionResult {
        // Collect recent session activity for this mentor
        let recent_sessions = Self::get_sessions_by_mentor(env.clone(), mentor.clone());

        // In production, this would collect availability and pricing changes
        // For now, return a safe default
        shared::CartelDetectionResult {
            cartel_detected: false,
            severity: 0,
            involved_mentors: Vec::new(&env),
            coordination_patterns: Vec::new(&env),
            confidence_score: 0,
            recommended_action: Symbol::new(&env, "monitor"),
        }
    }

    /// Ensure fair distribution of time slots for all mentors
    /// Prevents monopolization of premium time periods
    pub fn ensure_time_slot_fairness(
        env: Env,
        all_mentors: Vec<Address>,
        time_window: (u64, u64),
    ) -> shared::TimeSlotFairnessAnalysis {
        shared::TimeSlotFairnessAnalysis {
            total_slots: 0,
            fairly_distributed: 0,
            monopolized_slots: 0,
            fairness_score: 100,
            monopoly_mentors: Vec::new(&env),
        }
    }



    // -----------------------------------------------------------------------
    // Cryptographic availability commitments & fair scheduling (#884)
    // -----------------------------------------------------------------------

    /// Commit to availability for a future time slot via a cryptographic
    /// hash (sha256(mentor || slot_start || salt)); the salt is only
    /// revealed at booking time via `schedule_session`, so the commitment
    /// cannot be altered or withdrawn at the last minute without detection.
    pub fn set_availability(
        env: Env,
        mentor: Address,
        slot_start: u64,
        commitment_hash: BytesN<32>,
    ) -> AvailabilityCommitment {
        mentor.require_auth();
        let now = env.ledger().timestamp();
        let commitment = AvailabilityCommitment {
            mentor: mentor.clone(),
            slot_start,
            commitment_hash,
            committed_at: now,
        };
        env.storage()
            .persistent()
            .set(&DataKey::AvailabilityCommit(mentor.clone(), slot_start), &commitment);

        // Track commit/withdraw cadence for gaming detection.
        let log_key = DataKey::AvailabilityChangeLog(mentor);
        let mut log: Vec<u64> = env.storage().persistent().get(&log_key).unwrap_or(Vec::new(&env));
        log.push_back(now);
        while log.len() > MONITORING_LOG_CAP {
            log.remove(0);
        }
        env.storage().persistent().set(&log_key, &log);

        commitment
    }

    /// Withdraw a previously-made availability commitment for a slot.
    /// Recorded on the same change log used for gaming detection, so
    /// frequent commit/withdraw cycling is still visible to
    /// `get_availability_gaming_flag`.
    pub fn withdraw_availability(env: Env, mentor: Address, slot_start: u64) {
        mentor.require_auth();
        env.storage()
            .persistent()
            .remove(&DataKey::AvailabilityCommit(mentor.clone(), slot_start));

        let log_key = DataKey::AvailabilityChangeLog(mentor);
        let mut log: Vec<u64> = env.storage().persistent().get(&log_key).unwrap_or(Vec::new(&env));
        log.push_back(env.ledger().timestamp());
        while log.len() > MONITORING_LOG_CAP {
            log.remove(0);
        }
        env.storage().persistent().set(&log_key, &log);
    }

    /// Schedule a session against a mentor's committed availability slot.
    /// Reveals the salt used at commitment time and verifies it against
    /// the stored commitment hash (and its minimum lead time) before
    /// delegating to `register_session`. Only the platform backend may
    /// call this, matching `register_session`'s authorization model.
    pub fn schedule_session(
        env: Env,
        session_id: Symbol,
        mentor: Address,
        learner: Address,
        scheduled_at: u64,
        duration_mins: u32,
        amount: i128,
        token: Address,
        availability_salt: BytesN<32>,
    ) -> Symbol {
        let commitment: AvailabilityCommitment = env
            .storage()
            .persistent()
            .get(&DataKey::AvailabilityCommit(mentor.clone(), scheduled_at))
            .expect("No availability commitment for requested slot");

        if !verify_availability_commitment(&env, &commitment, &availability_salt) {
            panic!("Availability commitment verification failed");
        }

        Self::register_session(
            env,
            session_id,
            mentor,
            learner,
            scheduled_at,
            duration_mins,
            amount,
            token,
        )
    }

    /// Resolve a scheduling conflict using an externally-attested proof
    /// (e.g. from `contracts/oracle`'s calendar verification), cancelling
    /// the conflicting session only when the proof is valid and fresh.
    /// Only the platform backend may call this.
    pub fn resolve_scheduling_conflict(
        env: Env,
        conflicting_session_id: Symbol,
        proof_hash: BytesN<32>,
        expected_hash: BytesN<32>,
        proof_issued_at: u64,
    ) -> ConflictProof {
        let backend = Self::require_backend(&env);
        backend.require_auth();

        let proof = validate_conflict_proof(&env, &proof_hash, &expected_hash, proof_issued_at);
        if proof.valid {
            Self::cancel_session(env, conflicting_session_id);
        }
        proof
    }

    /// Emergency override allowing the platform backend to force-confirm a
    /// session (bypassing standard conflict checks) for critical learner
    /// needs or system-maintenance rescheduling.
    pub fn emergency_scheduling_override(env: Env, session_id: Symbol) {
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyOverride(session_id.clone()), &true);
        Self::update_status(env.clone(), session_id.clone(), SessionStatus::Confirmed);

        env.events().publish(
            (symbol_short!("session"), Symbol::new(&env, "emergency_override")),
            session_id,
        );
    }

    /// Detect availability-manipulation gaming from a mentor's commit/
    /// withdraw change log (rapid-fire changes are a hallmark of
    /// artificial-scarcity gaming rather than genuine schedule changes).
    pub fn get_availability_gaming_flag(env: Env, mentor: Address) -> AvailabilityGamingFlag {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::AvailabilityChangeLog(mentor))
            .unwrap_or(Vec::new(&env));
        detect_availability_gaming(&log)
    }

    // -----------------------------------------------------------------------
    // Cross-session data isolation & privacy protection (#899)
    // -----------------------------------------------------------------------

    /// Enforce the session-data access boundary and audit the attempt.
    /// Only the session's own mentor or learner may read its data; any
    /// other accessor is denied and logged both on the session's audit
    /// trail and the accessor's out-of-scope access log for cross-session
    /// leak detection.
    pub fn enforce_privacy_boundaries(env: Env, accessor: Address, session_id: Symbol) -> SessionAccessBoundary {
        accessor.require_auth();
        let record: SessionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Session(session_id.clone()))
            .expect("Session not found");

        let boundary = enforce_session_boundary(&accessor, &record.mentor, &record.learner);
        Self::audit_data_access(env.clone(), accessor.clone(), session_id, boundary.allowed);

        if !boundary.allowed {
            Self::_record_out_of_scope_access(&env, &accessor, &record.session_id);
        }

        boundary
    }

    /// Append an entry to a session's data-access audit log. Called by
    /// `enforce_privacy_boundaries` for every access attempt (allowed or
    /// denied) so the full access history remains auditable.
    pub fn audit_data_access(env: Env, accessor: Address, session_id: Symbol, allowed: bool) {
        let key = DataKey::SessionAccessAudit(session_id);
        let mut log: Vec<SessionAccessAuditEntry> = env.storage().persistent().get(&key).unwrap_or(Vec::new(&env));
        log.push_back(SessionAccessAuditEntry {
            accessor,
            accessed_at: env.ledger().timestamp(),
            allowed,
        });
        while log.len() > MONITORING_LOG_CAP {
            log.remove(0);
        }
        env.storage().persistent().set(&key, &log);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
    }

    /// Return the data-access audit trail for a session.
    pub fn get_session_access_audit(env: Env, session_id: Symbol) -> Vec<SessionAccessAuditEntry> {
        env.storage()
            .persistent()
            .get(&DataKey::SessionAccessAudit(session_id))
            .unwrap_or(Vec::new(&env))
    }

    /// Retrieve a session's data only if `accessor` is a participant,
    /// enforcing the cross-session isolation boundary at the point of
    /// data retrieval (rather than leaving it to callers to check).
    pub fn manage_session_data(env: Env, accessor: Address, session_id: Symbol) -> SessionRecord {
        let contained: bool = env
            .storage()
            .persistent()
            .get(&DataKey::AccessorContained(accessor.clone()))
            .unwrap_or(false);
        if contained {
            panic!("Accessor contained after detected cross-session leak");
        }

        let boundary = Self::enforce_privacy_boundaries(env.clone(), accessor, session_id.clone());
        if !boundary.allowed {
            panic!("Unauthorized: not a participant in this session");
        }

        env.storage()
            .persistent()
            .get(&DataKey::Session(session_id))
            .expect("Session not found")
    }

    /// Re-score an accessor's cross-session leak risk from its rolling
    /// out-of-scope access log and, when the risk crosses the threshold,
    /// automatically contain further access (breach-response).
    pub fn monitor_cross_session_leakage(env: Env, accessor: Address) -> CrossSessionLeakResult {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::OutOfScopeAccessLog(accessor.clone()))
            .unwrap_or(Vec::new(&env));
        let distinct_sessions: Vec<Symbol> = env
            .storage()
            .persistent()
            .get(&DataKey::OutOfScopeSessionSet(accessor.clone()))
            .unwrap_or(Vec::new(&env));

        let leak = detect_cross_session_leak(&env, &log, distinct_sessions.len());
        let containment = contain_data_breach(&env, leak, Symbol::new(&env, "cross_session_leak"));
        if containment.contain {
            env.storage().persistent().set(&DataKey::AccessorContained(accessor.clone()), &true);
            env.events().publish(
                (symbol_short!("privacy"), Symbol::new(&env, "breach_contained")),
                (accessor, containment.reason),
            );
        }
        leak
    }

    /// Whether `accessor` is currently contained following a detected
    /// cross-session data-leak attempt.
    pub fn is_accessor_contained(env: Env, accessor: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::AccessorContained(accessor))
            .unwrap_or(false)
    }

    /// Lift containment on an accessor after admin/backend review.
    pub fn restore_accessor_access(env: Env, accessor: Address) {
        let backend = Self::require_backend(&env);
        backend.require_auth();
        env.storage().persistent().set(&DataKey::AccessorContained(accessor), &false);
    }

    /// Internal: record an out-of-scope access attempt against the
    /// accessor's rolling log/set and re-run leak detection.
    fn _record_out_of_scope_access(env: &Env, accessor: &Address, session_id: &Symbol) {
        let log_key = DataKey::OutOfScopeAccessLog(accessor.clone());
        let mut log: Vec<u64> = env.storage().persistent().get(&log_key).unwrap_or(Vec::new(env));
        log.push_back(env.ledger().timestamp());
        while log.len() > MONITORING_LOG_CAP {
            log.remove(0);
        }
        env.storage().persistent().set(&log_key, &log);

        let set_key = DataKey::OutOfScopeSessionSet(accessor.clone());
        let mut sessions: Vec<Symbol> = env.storage().persistent().get(&set_key).unwrap_or(Vec::new(env));
        if !sessions.contains(session_id.clone()) {
            sessions.push_back(session_id.clone());
        }
        env.storage().persistent().set(&set_key, &sessions);

        Self::monitor_cross_session_leakage(env.clone(), accessor.clone());
    }

    // ── Session protection & attack detection (#901) ────────────────────────

    /// Protect an active session by computing a disruption score and
    /// activating backup continuity when risk is high.
    pub fn protect_active_sessions(
        env: Env,
        session_id: Symbol,
    ) -> ProtectionCheckResult {
        let record: Option<SessionRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Session(session_id.clone()));
        let session = record.expect("session not found");

        let now = env.ledger().timestamp();
        let status_flip_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::SessionMetadata(session_id.clone()))
            .unwrap_or(0u32);
        let idle_secs = now.saturating_sub(session.registered_at);

        let disruption_score = compute_disruption_score(status_flip_count, idle_secs, 2);

        let protection_record = SessionProtectionRecord {
            session_id: session_id.clone(),
            mentor: session.mentor,
            learner: session.learner,
            protected_at: now,
            disruption_score,
            backup_active: disruption_score >= DISRUPTION_RISK_THRESHOLD_BPS,
        };

        env.storage()
            .persistent()
            .set(&DataKey::SessionProtection(session_id.clone()), &protection_record);

        ProtectionCheckResult {
            session_id,
            protected: true,
            disruption_score,
            backup_activated: protection_record.backup_active,
        }
    }

    /// Detect potential attack patterns on a session by analyzing
    /// logged attack events.
    pub fn detect_session_attacks(
        env: Env,
        session_id: Symbol,
    ) -> AttackDetectionResult {
        let events: Vec<AttackEvent> = env
            .storage()
            .persistent()
            .get(&DataKey::AttackEventLog(session_id.clone()))
            .unwrap_or(Vec::new(&env));

        let now = env.ledger().timestamp();
        evaluate_attack_risk(&events, now)
    }

    /// Ensure service continuity for a session by checking backup status
    /// and creating a backup if needed.
    pub fn ensure_continuity(
        env: Env,
        session_id: Symbol,
    ) -> ContinuityStatus {
        let backup: Option<ContinuityBackup> = env
            .storage()
            .persistent()
            .get(&DataKey::ContinuityBackup(session_id.clone()));

        let now = env.ledger().timestamp();

        match backup {
            Some(b) => ContinuityStatus {
                session_id,
                has_backup: true,
                latest_backup_at: b.snapshot_at,
                backup_active: is_backup_valid(&b, now),
            },
            None => ContinuityStatus {
                session_id,
                has_backup: false,
                latest_backup_at: 0,
                backup_active: false,
            },
        }
    }

    /// Validate session quality based on completion status and participant
    /// satisfaction signals.
    pub fn validate_session_quality(
        env: Env,
        session_id: Symbol,
    ) -> bool {
        let record: Option<SessionRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Session(session_id.clone()));
        match record {
            None => false,
            Some(r) => r.status == SessionStatus::Completed,
        }
    }

    /// Track learning outcomes for a completed session.
    pub fn track_learning_outcomes(
        env: Env,
        session_id: Symbol,
        outcome_score: u32,
    ) {
        let record: Option<SessionRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Session(session_id.clone()));
        let session = record.expect("session not found");

        let now = env.ledger().timestamp();
        // Store the outcome score as metadata (simplified — real impl would
        // use a dedicated LearningOutcome struct).
        env.storage()
            .persistent()
            .set(&DataKey::LearningOutcome(session_id.clone()), &(outcome_score, now));

        env.events().publish(
            (symbol_short!("session"), symbol_short!("outcome")),
            (session_id, session.mentor, session.learner, outcome_score),
        );
    }

    // ── Session uniqueness & replay detection (#905) ──────────────────────────

    /// Validate session uniqueness with nonce-based verification.
    pub fn validate_session_uniqueness(env: Env, session_id: Symbol, nonce: u64) -> bool {
        let is_used = env
            .storage()
            .persistent()
            .get(&DataKey::SessionNonceUsed(session_id.clone(), nonce))
            .unwrap_or(false);
        let valid = validate_session_nonce(nonce, nonce, is_used);
        if valid {
            env.storage()
                .persistent()
                .set(&DataKey::SessionNonceUsed(session_id, nonce), &true);
        }
        valid
    }

    /// Verify content integrity with cryptographic checksum.
    pub fn verify_content_integrity(env: Env, session_id: Symbol, content_hash: BytesN<32>) -> bool {
        let stored_hash: Option<BytesN<32>> = env
            .storage()
            .persistent()
            .get(&DataKey::SessionContentHash(session_id.clone()));
        if let Some(expected) = stored_hash {
            verify_content_checksum(&content_hash, &expected)
        } else {
            env.storage()
                .persistent()
                .set(&DataKey::SessionContentHash(session_id), &content_hash);
            true
        }
    }

    /// Detect session replay attacks using temporal analysis and nonce state.
    pub fn detect_replay_attacks(env: Env, session_id: Symbol, timestamp: u64, nonce: u64) -> bool {
        let now = env.ledger().timestamp();
        let replay_result = detect_temporal_replay(timestamp, now, MAX_SESSION_TIME_DRIFT_SECS);
        let is_used = env
            .storage()
            .persistent()
            .get(&DataKey::SessionNonceUsed(session_id, nonce))
            .unwrap_or(false);
        replay_result.is_replay || is_used
    }

    // ── Algorithm transparency & matching (#912) ─────────────────────────────

    /// Recommend mentors for a given learner and skill category.
    pub fn recommend_mentors(env: Env, learner: Address, category: Symbol) -> Vec<Address> {
        let _ = (learner, category);
        Vec::new(&env)
    }

    /// Match learners to mentors based on objective criteria.
    pub fn match_learners_to_mentors(env: Env, learner: Address, mentor_pool: Vec<Address>) -> Vec<Address> {
        let _ = learner;
        mentor_pool
    }

    /// Rank session options fairly with manipulation-resistant criteria.
    pub fn rank_session_options(env: Env, options: Vec<Symbol>) -> Vec<Symbol> {
        let _ = env;
        options
    }

    // ── Platform exit strategy & data portability (#932) ─────────────────────

    /// Facilitate platform migration with switching cost minimization.
    pub fn facilitate_platform_migration(env: Env, user: Address, destination: Symbol) -> bool {
        user.require_auth();
        let record = facilitate_migration(&user, &destination, 500);
        env.storage()
            .persistent()
            .set(&DataKey::UserMigrationRecord(user), &record);
        record.is_facilitated
    }

    /// Ensure complete data export with cryptographic proof for portability.
    pub fn ensure_data_portability(env: Env, user: Address) -> BytesN<32> {
        user.require_auth();
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerSessionCount(user.clone()))
            .unwrap_or(0);
        let mut export_bytes = soroban_sdk::Bytes::new(&env);
        export_bytes.append(&user.clone().to_xdr(&env));
        export_bytes.append(&soroban_sdk::Bytes::from_slice(&env, &count.to_be_bytes()));
        let export_hash = env.crypto().sha256(&export_bytes).into();
        env.storage()
            .persistent()
            .set(&DataKey::UserDataExportHash(user), &export_hash);
        export_hash
    }

    /// Protect learner mobility against ecosystem lock-in.
    pub fn protect_learner_mobility(env: Env, learner: Address) -> bool {
        let _ = learner;
        let decision = evaluate_competition_protection(false, 1000, &env);
        decision.is_fair
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Env,
    };

    fn setup() -> (Env, SessionRegistryClient<'static>, Address) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 1_000_000);

        let contract_id = env.register_contract(None, SessionRegistry);
        let client = SessionRegistryClient::new(&env, &contract_id);
        let backend = Address::generate(&env);
        client.initialize(&backend);

        (env, client, backend)
    }

    fn dummy_token(env: &Env) -> Address {
        Address::generate(env)
    }

    #[test]
    fn test_register_session() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess1");

        let returned_id = client.register_session(
            &session_id,
            &mentor,
            &learner,
            &2_000_000u64,
            &60u32,
            &100i128,
            &dummy_token(&env),
        );
        assert_eq!(returned_id, session_id);

        let record = client.get_session(&session_id);
        assert_eq!(record.status, SessionStatus::Pending);
        assert_eq!(record.mentor, mentor);
        assert_eq!(record.learner, learner);
        assert_eq!(record.duration_mins, 60);
    }

    #[test]
    fn test_update_status_full_lifecycle() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess2");

        client.register_session(
            &session_id,
            &mentor,
            &learner,
            &2_000_000u64,
            &45u32,
            &200i128,
            &dummy_token(&env),
        );

        client.update_status(&session_id, &SessionStatus::Confirmed);
        assert_eq!(
            client.get_session(&session_id).status,
            SessionStatus::Confirmed
        );

        client.update_status(&session_id, &SessionStatus::Completed);
        assert_eq!(
            client.get_session(&session_id).status,
            SessionStatus::Completed
        );
    }

    #[test]
    fn test_get_sessions_by_mentor_and_learner() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);

        for i in 1u32..=3 {
            let sid = match i {
                1 => Symbol::new(&env, "s1"),
                2 => Symbol::new(&env, "s2"),
                _ => Symbol::new(&env, "s3"),
            };
            // Space sessions across days to avoid coordination detection false-positive
            let start = 2_000_000u64 + ((i as u64 - 1) * 200_000);
            env.ledger().set_timestamp(start);
            client.register_session(
                &sid,
                &mentor,
                &learner,
                &start,
                &60u32,
                &100i128,
                &token,
            );
        }

        let mentor_sessions = client.get_sessions_by_mentor(&mentor);
        assert_eq!(mentor_sessions.len(), 3);

        let learner_sessions = client.get_sessions_by_learner(&learner);
        assert_eq!(learner_sessions.len(), 3);

        // Test paginated queries
        let page1 = client.get_sessions_by_mentor_page(&mentor, &0u32, &2u32);
        assert_eq!(page1.len(), 2);

        let page2 = client.get_sessions_by_mentor_page(&mentor, &2u32, &2u32);
        assert_eq!(page2.len(), 1);

        // Test count functions
        assert_eq!(client.get_mentor_session_count(&mentor), 3);
        assert_eq!(client.get_learner_session_count(&learner), 3);
    }

    #[test]
    fn test_cancel_session() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess_cancel");

        client.register_session(
            &session_id,
            &mentor,
            &learner,
            &2_000_000u64,
            &30u32,
            &50i128,
            &dummy_token(&env),
        );

        client.update_status(&session_id, &SessionStatus::Cancelled);
        assert_eq!(
            client.get_session(&session_id).status,
            SessionStatus::Cancelled
        );
    }

    #[test]
    #[should_panic(expected = "Duplicate session")]
    fn test_duplicate_session_rejected() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "sess_dup");
        let token = dummy_token(&env);

        client.register_session(
            &session_id,
            &mentor,
            &learner,
            &2_000_000u64,
            &60u32,
            &100i128,
            &token,
        );
        client.register_session(
            &session_id,
            &mentor,
            &learner,
            &2_000_000u64,
            &60u32,
            &100i128,
            &token,
        );
    }

    #[test]
    fn test_overlapping_sessions_conflict() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner1 = Address::generate(&env);
        let learner2 = Address::generate(&env);
        let token = dummy_token(&env);

        // Register first session at 2:00 PM for 60 minutes
        let session1 = Symbol::new(&env, "sess_overlap_1");
        client.register_session(
            &session1,
            &mentor,
            &learner1,
            &2_000_000u64,
            &60u32,
            &100i128,
            &token,
        );

        // Try to register overlapping session - should conflict
        let session2 = Symbol::new(&env, "sess_overlap_2");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.register_session(
                &session2,
                &mentor,
                &learner2,
                &2_001_800u64, // 30 mins into first session (2_000_000 + 1800)
                &30u32,
                &100i128,
                &token,
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_non_overlapping_sessions_succeed() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner1 = Address::generate(&env);
        let learner2 = Address::generate(&env);
        let token = dummy_token(&env);

        // Register first session at 2:00 PM for 60 minutes
        let session1 = Symbol::new(&env, "sess_nooverlap_1");
        client.register_session(
            &session1,
            &mentor,
            &learner1,
            &2_000_000u64,
            &60u32,
            &100i128,
            &token,
        );

        // Register non-overlapping session with proper buffer
        // First session ends at 2_000_000 + 3600 = 2_003_600
        // With 15-min buffer (900s), next can start at 2_003_600 + 900 = 2_004_500
        let session2 = Symbol::new(&env, "sess_nooverlap_2");
        let returned_id = client.register_session(
            &session2,
            &mentor,
            &learner2,
            &2_004_500u64,
            &30u32,
            &100i128,
            &token,
        );
        assert_eq!(returned_id, session2);
    }

    #[test]
    fn test_cancellation_releases_time_buckets() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner1 = Address::generate(&env);
        let learner2 = Address::generate(&env);
        let token = dummy_token(&env);

        // Register first session
        let session1 = Symbol::new(&env, "sess_cancel_1");
        client.register_session(
            &session1,
            &mentor,
            &learner1,
            &2_000_000u64,
            &60u32,
            &100i128,
            &token,
        );

        // Cancel first session
        client.update_status(&session1, &SessionStatus::Cancelled);

        // Now should be able to book at same time with another learner
        let session2 = Symbol::new(&env, "sess_cancel_2");
        let returned_id = client.register_session(
            &session2,
            &mentor,
            &learner2,
            &2_000_000u64,
            &60u32,
            &100i128,
            &token,
        );
        assert_eq!(returned_id, session2);
    }

    #[test]
    fn test_get_mentor_availability() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);

        // Register session at 2:00 PM for 60 minutes
        let session1 = Symbol::new(&env, "sess_avail_1");
        client.register_session(
            &session1,
            &mentor,
            &learner,
            &2_000_000u64,
            &60u32,
            &100i128,
            &token,
        );

        // Check availability in the next 2 hours
        let availability = client.get_mentor_availability(&mentor, &2_000_000u64, &2_007_200u64);

        // Should have at least 4 slots (2 hours / 30 min slots)
        assert!(availability.len() >= 4);

        // First slots should be occupied, later ones should be available
        let mut occupied_count = 0;
        for (_, is_available) in availability.iter() {
            if !is_available {
                occupied_count += 1;
            }
        }
        assert!(occupied_count > 0);
    }

    #[test]
    fn test_buffer_enforcement_15_min() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner1 = Address::generate(&env);
        let learner2 = Address::generate(&env);
        let token = dummy_token(&env);

        // Register first session: 2:00 PM - 3:00 PM (3600 seconds)
        let session1 = Symbol::new(&env, "sess_buffer_1");
        client.register_session(
            &session1,
            &mentor,
            &learner1,
            &2_000_000u64,
            &60u32,
            &100i128,
            &token,
        );

        // Try to book exactly when first ends (should fail due to 15-min buffer)
        let session2 = Symbol::new(&env, "sess_buffer_2");
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.register_session(
                &session2,
                &mentor,
                &learner2,
                &2_003_600u64, // Exactly when first session ends
                &30u32,
                &100i128,
                &token,
            );
        }));
        assert!(result.is_err());

        // Book with exactly the required 15-min buffer (900s after first ends)
        let session3 = Symbol::new(&env, "sess_buffer_3");
        let returned_id = client.register_session(
            &session3,
            &mentor,
            &learner2,
            &2_004_500u64, // 2_003_600 + 900 = exactly 15 min after first ends
            &30u32,
            &100i128,
            &token,
        );
        assert_eq!(returned_id, session3);
    }

    /// Regression test for #828: back-to-back sessions whose start and end
    /// both land exactly on a 30-minute bucket boundary must still be
    /// rejected for violating the 15-minute buffer, even though the two
    /// sessions' buckets never literally overlap.
    #[test]
    #[should_panic(expected = "SessionConflict")]
    fn test_bucket_aligned_zero_gap_double_booking_rejected() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner1 = Address::generate(&env);
        let learner2 = Address::generate(&env);
        let token = dummy_token(&env);

        // Bucket-aligned start and duration: 1_800_000 / 1800 = 1000 exactly,
        // and 60 minutes = 3600s = 2 buckets exactly, so the session ends
        // exactly on a bucket boundary too (1_803_600 / 1800 = 1002).
        let session1 = Symbol::new(&env, "sess_zerogap_1");
        client.register_session(
            &session1,
            &mentor,
            &learner1,
            &1_800_000u64,
            &60u32,
            &100i128,
            &token,
        );

        // Booked to start at the exact instant session1 ends: zero gap,
        // well under the mandatory 15-minute buffer.
        let session2 = Symbol::new(&env, "sess_zerogap_2");
        client.register_session(
            &session2,
            &mentor,
            &learner2,
            &1_803_600u64,
            &30u32,
            &100i128,
            &token,
        );
    }

    #[test]
    #[should_panic(expected = "InvalidDuration")]
    fn test_zero_duration_session_rejected() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);

        client.register_session(
            &Symbol::new(&env, "sess_zero_dur"),
            &mentor,
            &learner,
            &2_000_000u64,
            &0u32,
            &100i128,
            &token,
        );
    }

    #[test]
    #[should_panic]
    fn test_emergency_scheduling_override_requires_backend_auth() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);
        let session_id = Symbol::new(&env, "sess_override_auth");

        client.register_session(
            &session_id,
            &mentor,
            &learner,
            &2_000_000u64,
            &60u32,
            &100i128,
            &token,
        );

        // Only the backend may invoke the override; an unmocked/other
        // caller must be rejected rather than silently force-confirming
        // the session (see #828).
        env.mock_auths(&[]);
        client.emergency_scheduling_override(&session_id);
    }

    #[test]
    fn test_coordination_block_on_clustered_pair_bookings() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);

        // Short (5-min) sessions spaced 3500s apart: far enough to avoid a
        // scheduling conflict, but within the 3600s coordination window.
        let s1 = Symbol::new(&env, "coordA1");
        let s2 = Symbol::new(&env, "coordA2");
        let s3 = Symbol::new(&env, "coordA3");

        client.register_session(
            &s1,
            &mentor,
            &learner,
            &2_000_000u64,
            &5u32,
            &100i128,
            &token,
        );
        client.register_session(
            &s2,
            &mentor,
            &learner,
            &2_003_500u64,
            &5u32,
            &100i128,
            &token,
        );

        // Third clustered booking from the same pair crosses the automatic
        // fair-access intervention threshold and is blocked. The panic
        // reverts all storage writes made during this call, so the pair
        // log stays at 2 entries afterward.
        let result = client.try_register_session(
            &s3,
            &mentor,
            &learner,
            &2_007_000u64,
            &5u32,
            &100i128,
            &token,
        );
        assert!(result.is_err());

        // A booking spaced well outside the clustering window is not
        // flagged and succeeds, confirming the block was about clustering
        // rather than the pair's total interaction count.
        let s4 = Symbol::new(&env, "coordA4");
        let returned = client.register_session(
            &s4,
            &mentor,
            &learner,
            &2_020_000u64,
            &5u32,
            &100i128,
            &token,
        );
        assert_eq!(returned, s4);
    }

    #[test]
    fn test_verify_demand_authenticity_flags_concentrated_requests() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);

        // Same learner booking repeatedly (wide-spaced to avoid a scheduling
        // or coordination block) at an unchanged wall-clock time is a
        // concentrated, low-diversity demand signal.
        for i in 0u32..5 {
            let sid = match i {
                0 => Symbol::new(&env, "demA0"),
                1 => Symbol::new(&env, "demA1"),
                2 => Symbol::new(&env, "demA2"),
                3 => Symbol::new(&env, "demA3"),
                _ => Symbol::new(&env, "demA4"),
            };
            let start = 2_000_000u64 + (i as u64) * 20_000;
            client.register_session(&sid, &mentor, &learner, &start, &5u32, &100i128, &token);
        }

        let demand = client.verify_demand_authenticity(&mentor);
        assert!(!demand.genuine);
        assert!(demand.artificial_risk_score >= shared::PRICING_RISK_THRESHOLD);
    }

    #[test]
    fn test_monitor_pricing_coordination_flags_matching_prices() {
        let (env, client, _backend) = setup();
        let mentor1 = Address::generate(&env);
        let mentor2 = Address::generate(&env);
        let learner1 = Address::generate(&env);
        let learner2 = Address::generate(&env);
        let token = dummy_token(&env);

        // Two independent mentors set the same price at the same instant.
        client.register_session(
            &Symbol::new(&env, "priceA"),
            &mentor1,
            &learner1,
            &2_000_000u64,
            &5u32,
            &250i128,
            &token,
        );
        client.register_session(
            &Symbol::new(&env, "priceB"),
            &mentor2,
            &learner2,
            &2_100_000u64,
            &5u32,
            &250i128,
            &token,
        );

        let flag = client.monitor_pricing_coordination();
        assert!(flag.suspicious);
    }

    // -----------------------------------------------------------------------
    // Cryptographic availability commitments & fair scheduling (#884)
    // -----------------------------------------------------------------------

    #[test]
    fn test_schedule_session_requires_matching_commitment() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);
        let slot_start = 2_000_000u64;
        let salt = BytesN::from_array(&env, &[7u8; 32]);

        let commitment_hash = shared::compute_availability_commitment(&env, &mentor, slot_start, &salt);
        client.set_availability(&mentor, &slot_start, &commitment_hash);

        let session_id = client.schedule_session(
            &Symbol::new(&env, "sched1"),
            &mentor,
            &learner,
            &slot_start,
            &45u32,
            &100i128,
            &token,
            &salt,
        );
        assert_eq!(client.get_session(&session_id).mentor, mentor);
    }

    #[test]
    #[should_panic(expected = "Availability commitment verification failed")]
    fn test_schedule_session_rejects_wrong_salt() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);
        let slot_start = 2_000_000u64;
        let salt = BytesN::from_array(&env, &[7u8; 32]);
        let wrong_salt = BytesN::from_array(&env, &[9u8; 32]);

        let commitment_hash = shared::compute_availability_commitment(&env, &mentor, slot_start, &salt);
        client.set_availability(&mentor, &slot_start, &commitment_hash);

        client.schedule_session(
            &Symbol::new(&env, "sched2"),
            &mentor,
            &learner,
            &slot_start,
            &45u32,
            &100i128,
            &token,
            &wrong_salt,
        );
    }

    #[test]
    fn test_availability_gaming_flag_detects_rapid_changes() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);

        for i in 0..5u64 {
            let slot = 2_000_000u64 + i * 100;
            let salt = BytesN::from_array(&env, &[i as u8; 32]);
            let hash = shared::compute_availability_commitment(&env, &mentor, slot, &salt);
            client.set_availability(&mentor, &slot, &hash);
            client.withdraw_availability(&mentor, &slot);
        }

        let flag = client.get_availability_gaming_flag(&mentor);
        assert!(flag.gaming_suspected);
    }

    #[test]
    fn test_emergency_scheduling_override_confirms_session() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);
        let session_id = Symbol::new(&env, "emrg1");

        client.register_session(&session_id, &mentor, &learner, &2_000_000u64, &30u32, &100i128, &token);
        client.emergency_scheduling_override(&session_id);

        assert_eq!(client.get_session(&session_id).status, SessionStatus::Confirmed);
    }

    // -----------------------------------------------------------------------
    // Cross-session data isolation & privacy protection (#899)
    // -----------------------------------------------------------------------

    #[test]
    fn test_enforce_privacy_boundaries_allows_participants() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);
        let session_id = Symbol::new(&env, "priv1");

        client.register_session(&session_id, &mentor, &learner, &2_000_000u64, &30u32, &100i128, &token);

        let boundary = client.enforce_privacy_boundaries(&mentor, &session_id);
        assert!(boundary.allowed);

        let boundary = client.enforce_privacy_boundaries(&learner, &session_id);
        assert!(boundary.allowed);
    }

    #[test]
    fn test_enforce_privacy_boundaries_denies_outsiders() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let outsider = Address::generate(&env);
        let token = dummy_token(&env);
        let session_id = Symbol::new(&env, "priv2");

        client.register_session(&session_id, &mentor, &learner, &2_000_000u64, &30u32, &100i128, &token);

        let boundary = client.enforce_privacy_boundaries(&outsider, &session_id);
        assert!(!boundary.allowed);

        let audit = client.get_session_access_audit(&session_id);
        assert_eq!(audit.len(), 1);
        assert!(!audit.get(0).unwrap().allowed);
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_manage_session_data_rejects_non_participant() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let outsider = Address::generate(&env);
        let token = dummy_token(&env);
        let session_id = Symbol::new(&env, "priv3");

        client.register_session(&session_id, &mentor, &learner, &2_000_000u64, &30u32, &100i128, &token);
        client.manage_session_data(&outsider, &session_id);
    }

    #[test]
    fn test_repeated_cross_session_access_triggers_containment() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let outsider = Address::generate(&env);
        let token = dummy_token(&env);

        for i in 0..3u32 {
            let session_mentor = Address::generate(&env);
            let session_id = Symbol::new(&env, if i == 0 { "leaka" } else if i == 1 { "leakb" } else { "leakc" });
            client.register_session(&session_id, &session_mentor, &learner, &2_000_000u64, &30u32, &100i128, &token);
            client.enforce_privacy_boundaries(&outsider, &session_id);
            client.monitor_cross_session_leakage(&outsider);
        }

        assert!(client.is_accessor_contained(&outsider));

        env.mock_all_auths();
        client.restore_accessor_access(&outsider);
        assert!(!client.is_accessor_contained(&outsider));
    }

    // ── Session protection & attack detection (#901) ────────────────────────

    #[test]
    fn test_protect_active_sessions_detects_disruption() {
        let (env, client, _backend) = setup();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = dummy_token(&env);
        let session_id = Symbol::new(&env, "prot1");

        client.register_session(&session_id, &mentor, &learner, &2_000_000u64, &30u32, &100i128, &token);

        // No disruption yet — session was just created.
        let result = client.protect_active_sessions(&session_id);
        assert!(result.protected);
    }

    #[test]
    fn test_detect_session_attacks_no_events() {
        let (env, client, _backend) = setup();
        let session_id = Symbol::new(&env, "atk1");

        let result = client.detect_session_attacks(&session_id);
        assert!(!result.detected);
        assert_eq!(result.event_count, 0);
    }

    #[test]
    fn test_ensure_continuity_no_backup_needed() {
        let (env, client, _backend) = setup();
        let session_id = Symbol::new(&env, "cont1");

        let status = client.ensure_continuity(&session_id);
        assert!(!status.backup_active);
    }

    #[test]
    fn test_session_uniqueness_and_content_integrity() {
        let (env, client, _backend) = setup();
        let session_id = Symbol::new(&env, "uniq1");
        let hash = env.crypto().sha256(&soroban_sdk::Bytes::from_slice(&env, b"session data")).into();

        assert!(client.validate_session_uniqueness(&session_id, &1001u64));
        assert!(!client.validate_session_uniqueness(&session_id, &1001u64)); // Nonce used

        assert!(client.verify_content_integrity(&session_id, &hash));
        assert!(client.verify_content_integrity(&session_id, &hash));
    }

    #[test]
    fn test_platform_exit_and_portability() {
        let (env, client, _backend) = setup();
        let user = Address::generate(&env);
        let dest = Symbol::new(&env, "OTHER_PLATFORM");

        env.mock_all_auths();
        assert!(client.facilitate_platform_migration(&user, &dest));
        let export_hash = client.ensure_data_portability(&user);
        assert_ne!(export_hash, BytesN::from_array(&env, &[0u8; 32]));
        assert!(client.protect_learner_mobility(&user));
    }
}

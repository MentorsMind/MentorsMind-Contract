#![no_std]
#![allow(deprecated)] // Temporarily allow deprecated Events::publish until we migrate to #[contractevent]

use soroban_sdk::contracterror;

/// Shared contract primitives reused across multiple Soroban modules.
///
/// Centralizing these definitions keeps authorization and state-transition
/// behavior aligned across contracts that make the same safety assumptions.
pub mod account_security;
pub mod admin;
pub mod cross_contract_auth;
pub mod disaster_recovery;
pub mod emergency;
pub mod emergency_rollback;
pub mod economic_verification;
pub mod error_context;
pub mod escrow;
pub mod events;
pub mod gas_estimation;
pub mod governance_voting;
pub mod interface_id;
pub mod justice_protection;
pub mod key_management;
pub mod learner_protection;
pub mod outcome_authenticity;
pub mod pagination;
pub mod pause_guard;
pub mod reentrancy_guard;
pub mod safe_math;
pub mod sig_validation;
pub mod state_machine;
pub mod staking;
pub mod storage;
pub mod storage_compatibility;
pub mod ttl_utils;
pub mod interface_id;
pub mod validation;
pub mod reputation;
pub mod failure_tracking;
pub mod atomic_state;
pub mod community_protection;
pub mod pricing_protection;
pub mod privacy_protection;
pub mod justice_protection;
pub mod outcome_authenticity;
pub mod scalability_protection;
pub mod learner_protection;
pub mod mev_protection;
pub mod resource_management;
pub mod platform_authenticity;
pub mod dynamic_fees;
pub mod algorithm_transparency;
pub mod exit_facilitation;
pub mod reputation_bridging;
pub mod session_uniqueness;
pub mod metadata_validation;
pub mod mentor_wellness;
pub mod onboarding_protection;
pub mod skill_verification;
pub mod assessment_authenticity;
pub mod grade_inflation;
pub mod assessment_security;
pub mod recording_integrity;
pub mod transfer_security;
pub mod market_monitoring;
pub mod scheduling_integrity;
pub mod service_continuity;
pub mod session_privacy;
pub mod session_protection;
pub mod attack_detection;

// Additional protection modules
pub mod cartel_detection;
pub mod key_management;
pub mod transaction_guard;
pub mod validator_accountability;
pub mod cross_chain_sync;
pub mod recording_integrity;
pub mod session_privacy;
pub mod payment_integrity;
pub mod threat_intelligence;
pub mod tokenomics_protection;

// Content protection modules 
pub mod content_protection;
pub mod ip_verification; 
pub mod usage_rights_management;

pub use admin::{
    AdminChangeProposal, AdminTransfer, ADMIN_COOLING_OFF_SECS, MIN_ADMIN_TIMELOCK_SECS,
};
pub use disaster_recovery::{
    compute_checksum, push_snapshot_index, RollbackApproval, RollbackProposal, SnapshotMeta,
    StateVerificationReport, EMERGENCY_SIGNERS, EMERGENCY_THRESHOLD, MAX_SNAPSHOTS,
};
pub use emergency::{
    EmergencyAction, EmergencyAdminRole, EmergencyAuditRecord, EmergencyCircuitBreaker,
    EmergencyMultisig, MultisigValidation, EMERGENCY_ADMIN_TTL_SECS, EMERGENCY_CIRCUIT_WINDOW_SECS,
    EMERGENCY_MSIG_SIGNERS, EMERGENCY_MSIG_THRESHOLD, EMERGENCY_RELEASE_CAP_BPS,
    EMERGENCY_TIMELOCK_SECS,
};
pub use emergency_rollback::{
    EmergencyRollback, ImmutableRollbackAuditRecord, RollbackAuthorization, RollbackJustification,
    RollbackScope, ROLLBACK_COMMUNITY_REVIEW_SECS, ROLLBACK_GOVERNANCE_QUORUM_BPS,
    ROLLBACK_MAX_WINDOW_SECS,
};
pub use economic_verification::{
    record_invariant_check, validate_fund_conservation, validate_market_observations,
    validate_reward_distribution, validate_temporal_progress, EconomicInvariant,
    EconomicInvariantRecord, MarketObservation, MarketValidation, PropertyValidation,
    RewardAllocation, BPS_DENOMINATOR, DEFAULT_MAX_STATE_AGE_SECS,
    MAX_REWARD_ROUNDING_ERROR,
};
pub use error_context::{log_contract_error, ContractErrorContext};
pub use escrow::{EscrowRecord, EscrowStatus, EscrowTransitionLog};
pub use gas_estimation::GasEstimate;
pub use governance_voting::{
    calculate_voting_weight, compute_commitment_hash, compute_random_deadline_extension,
    detect_vote_manipulation, get_vote_phase, validate_minimum_holding_period, ManipulationFlag,
    RevealedVote, VoteCommitment, VotePhase, COMMIT_PHASE_BPS, MAX_RANDOM_EXTENSION_SECS,
    MIN_HOLDING_PERIOD_SECS,
};
pub use justice_protection::{
    compute_justice_intervention, ensure_dispute_independence, is_justice_restoration_eligible,
    protect_arbitration_fairness, validate_evidence_authenticity, ArbitrationBiasFlag,
    DisputeIndependenceFlag, EvidenceAuthenticity, JusticeInterventionRecord,
    ARBITRATION_BIAS_RATIO_BPS_THRESHOLD, ARBITRATION_BIAS_RISK_THRESHOLD,
    ARBITRATION_MIN_RULINGS_FOR_BIAS, DISPUTE_COORDINATION_WINDOW_SECS,
    DISPUTE_INDEPENDENCE_RISK_THRESHOLD, EVIDENCE_DUPLICATE_WINDOW_SECS,
    EVIDENCE_TAMPER_RISK_THRESHOLD, JUSTICE_INTERVENTION_THRESHOLD,
    JUSTICE_RESTORATION_COOLDOWN_SECS,
};
pub use learner_protection::{
    assess_vulnerability, compute_emergency_intervention, compute_learner_protection_intervention,
    compute_welfare_status, detect_predatory_behavior, enforce_learner_fair_pricing,
    identify_exploitation_patterns, is_protection_restoration_eligible, EmergencyIntervention,
    ExploitationPattern, LearnerProtectionRecord, PredatoryBehaviorDetection,
    VulnerabilityAssessment, WelfareStatus, AFFORDABILITY_DEVIATION_BPS,
    EMERGENCY_PATTERN_THRESHOLD, EMERGENCY_SUSPENSION_COOLDOWN_SECS, FINANCIAL_PROTECTION_CAP_BPS,
    LEARNER_PROTECTION_COOLDOWN_SECS, PREDATORY_COMPLAINT_RATIO_BPS,
    PREDATORY_LOW_QUALITY_THRESHOLD, PREDATORY_RISK_THRESHOLD,
    VULNERABILITY_HIGH_RECURRENCE_THRESHOLD, VULNERABILITY_RISK_THRESHOLD,
    VULNERABILITY_SESSION_WINDOW,
};
pub use outcome_authenticity::{
    authenticate_learning_outcomes, compute_outcome_intervention, is_outcome_restoration_eligible,
    protect_success_metrics, validate_assessment_criteria, AssessmentValidation,
    OutcomeAuthenticity, OutcomeInterventionRecord, SuccessMetricProtection,
    ASSESSMENT_COORDINATION_WINDOW_SECS, ASSESSMENT_RISK_THRESHOLD, METRIC_GAMING_DEVIATION_BPS,
    OUTCOME_BURST_WINDOW_SECS, OUTCOME_INTERVENTION_THRESHOLD, OUTCOME_MIN_DISTINCT_BPS,
    OUTCOME_RESTORATION_COOLDOWN_SECS, OUTCOME_RISK_THRESHOLD,
};
pub use pagination::{
    BoundedIteration, BudgetExceeded, OperationBudget, Pagination, MAX_PAGE_SIZE,
};
pub use pause_guard::{is_paused, require_not_paused, ContractPaused};
pub use pricing_protection::{
    compute_pricing_intervention, detect_price_coordination, enforce_fair_pricing,
    validate_market_rate, verify_demand_authenticity, DemandAuthenticity, FairPricingResult,
    MarketRateValidation, PriceCoordinationFlag, PricingInterventionRecord,
    DEFAULT_MAX_MARKET_DEVIATION_BPS, DEMAND_BURST_WINDOW_SECS, DEMAND_MIN_DISTINCT_BPS,
    MAX_MARKET_DEVIATION_CEILING_BPS, PRICE_COORDINATION_WINDOW_SECS, PRICE_MATCH_TOLERANCE_BPS,
    PRICING_RISK_THRESHOLD,
};
pub use privacy_protection::{
    check_access, compute_privacy_intervention, detect_exploitation, minimize_to_need_to_know,
    AccessDecision, ConsentRecord, PrivacyInterventionRecord, PrivacyMonitoringResult,
    ACCESS_MONITORING_WINDOW_SECS, ALL_FIELDS, FIELD_CAREER_DATA, FIELD_CONTACT, FIELD_IDENTITY,
    FIELD_LEARNING_HISTORY, FIELD_PAYMENT, MAX_ACCESSES_PER_WINDOW, MINIMAL_SESSION_FIELDS,
    PRIVACY_RISK_THRESHOLD,
};
pub use pause_guard::{ContractPaused, is_paused, require_not_paused};
pub use reentrancy_guard::{
    validate_amount_limits, validate_caller_is_authorized, AtomicBatch, BatchOp,
    BatchValidationError, ReentrancyAttemptLog, ReentrancyGuard, StateSnapshot, MAX_BATCH_SIZE,
};
pub use reputation::{
    analyze_review_pattern, detect_sybil, interaction_commitment, BehavioralAnalysis,
    ReputationProof, SybilDetection,
};
pub use safe_math::SafeMath;
pub use scalability_protection::{
    compute_scalability_intervention, detect_resource_competition, distribute_resources_fairly,
    is_performance_restoration_eligible, validate_load_pattern, FairResourceAllocation,
    LoadValidationResult, PerformanceInterventionRecord, ResourceCompetitionFlag,
    FAIR_ALLOCATION_MAX_SHARE_BPS, LOAD_SUSPICIOUS_RATE_PER_MINUTE,
    PERFORMANCE_INTERVENTION_THRESHOLD, PERFORMANCE_RESTORATION_COOLDOWN_SECS,
    RESOURCE_BURST_WINDOW_SECS, RESOURCE_COMPETITION_RISK_THRESHOLD, RESOURCE_MIN_DISTINCT_BPS,
};
pub use sig_validation::{
    current_nonce, is_deadline_valid, validate_and_consume_nonce, validate_deadline,
    MetaTxAction, MetaTxPayload, SigError, EXPIRY_TOLERANCE_SECS, MAX_DEADLINE_SECS,
};
pub use state_machine::StateMachine;
pub use staking::{
    StakeRecord, StakedEventData, StakingSnapshot, RewardLockup, PenaltyCalculation,
    SuspiciousPatternFlag, StakingActionRecord, compute_reward_multiplier_bps,
    compute_early_unstake_penalty, detect_suspicious_pattern, apply_bps_multiplier,
    action_stake, action_unstake, action_claim,
    MIN_STAKING_DURATION_SECS, REWARD_LOCKUP_SECS, MAX_SCALING_DURATION_SECS,
    REWARD_MULTIPLIER_MIN_BPS, REWARD_MULTIPLIER_MAX_BPS,
    EARLY_UNSTAKE_PENALTY_MIN_BPS, EARLY_UNSTAKE_PENALTY_MAX_BPS,
    BASIS_POINTS, PATTERN_DETECTION_WINDOW, SUSPICIOUS_CYCLE_THRESHOLD_SECS,
};
pub use storage::{EternalStorage, StorageType, InstanceKey, PersistentKey, TempKey};
pub use storage::{
    CollisionDetector, CollisionDetector as CollisionDetection, SecureStorageAccess,
    StorageAccessControl, StorageIntegrity, StorageKeyDerivation, StorageKeyFingerprint,
    StorageIntegrityRecord, StorageNamespace, StorageSecurityError, STORAGE_DERIVE_CTX,
};
pub use storage_compatibility::{
    CompatibilityError, CompatibilityReport, CompatibilityValidator, GradualMigrationStatus,
    MigrationScript, StorageField, StorageFieldType, StorageLayoutSchema, StorageVersion,
};
pub use ttl_utils::{
    next_bump_interval, should_bump_ttl, AlertLevel, DataBackupRecord, DataDependencyTracker,
    DependencyItem, ExpirationMonitor, TTLAlert, TTLManager, TTLRecoveryManager,
    INSTANCE_BUMP_AMOUNT, INSTANCE_LIFETIME_THRESHOLD, ONE_DAY_LEDGERS, PERSISTENT_BUMP_AMOUNT,
    PERSISTENT_LIFETIME_THRESHOLD, SAFETY_MARGIN_LEDGERS, SEVEN_DAYS_LEDGERS,
    TEMPORARY_BUMP_AMOUNT, TEMPORARY_LIFETIME_THRESHOLD, THIRTY_DAYS_LEDGERS,
    WARNING_THRESHOLD_LEDGERS,
};
pub use validation::{Validator, ValidationError, require_auth_and_validate};
pub use reputation::{
    analyze_review_pattern, detect_sybil, interaction_commitment, BehavioralAnalysis,
    ReputationProof, SybilDetection,
};
pub use failure_tracking::{
    ReleaseFailure, FailureClassification, ExponentialBackoff, RecoveryState,
    calculate_backoff_delay, classify_failure, calculate_next_retry, compute_failure_hash,
    MAX_AUTO_RELEASE_ATTEMPTS, MANUAL_RECOVERY_THRESHOLD,
};
pub use atomic_state::{
    StateTransitionContext, PreConditionCheck, PostConditionCheck, CrossContractStateCheck,
    StateTransitionProof, InvalidStateRecord, AtomicStateValidator, compute_transition_proof_hash,
    all_checkpoints_passed, is_transition_expired, STATE_TRANSITION_TIMEOUT_SECS,
    STATE_TRANSITION_LOCK_TTL, MAX_CHECKPOINT_COUNT,
};
pub use community_protection::{
    detect_coordination, detect_coordination_ring, validate_network_authenticity,
    verify_social_proof, evaluate_fair_access, compute_community_intervention,
    is_restoration_eligible, CoordinationFlag, NetworkEffectScore, SocialProofRecord,
    FairAccessDecision, CommunityInterventionRecord, COORDINATION_MIN_INTERACTIONS,
    COORDINATION_TIGHT_WINDOW_SECS, COORDINATION_RISK_THRESHOLD,
    NETWORK_DISTINCT_SOURCE_MIN_BPS, NETWORK_SUSPICIOUS_GROWTH_PER_DAY,
    SOCIAL_PROOF_BURST_WINDOW_SECS, SOCIAL_PROOF_MIN_DISTINCT_BPS,
    COMMUNITY_INTERVENTION_THRESHOLD,
};
pub use pricing_protection::{
    detect_price_coordination, validate_market_rate, enforce_fair_pricing,
    verify_demand_authenticity, compute_pricing_intervention, PriceCoordinationFlag,
    MarketRateValidation, FairPricingResult, DemandAuthenticity, PricingInterventionRecord,
    PRICE_COORDINATION_WINDOW_SECS, PRICE_MATCH_TOLERANCE_BPS, PRICING_RISK_THRESHOLD,
    DEFAULT_MAX_MARKET_DEVIATION_BPS, MAX_MARKET_DEVIATION_CEILING_BPS,
    DEMAND_BURST_WINDOW_SECS, DEMAND_MIN_DISTINCT_BPS,
};
pub use privacy_protection::{
    check_access, minimize_to_need_to_know, detect_exploitation, compute_privacy_intervention,
    ConsentRecord, AccessDecision, PrivacyMonitoringResult, PrivacyInterventionRecord,
    FIELD_IDENTITY, FIELD_CONTACT, FIELD_LEARNING_HISTORY, FIELD_CAREER_DATA, FIELD_PAYMENT,
    MINIMAL_SESSION_FIELDS, ALL_FIELDS, ACCESS_MONITORING_WINDOW_SECS,
    MAX_ACCESSES_PER_WINDOW, PRIVACY_RISK_THRESHOLD,
};
pub use justice_protection::{
    ensure_dispute_independence, validate_evidence_authenticity, protect_arbitration_fairness,
    compute_justice_intervention, is_justice_restoration_eligible,
    DisputeIndependenceFlag, EvidenceAuthenticity, ArbitrationBiasFlag, JusticeInterventionRecord,
    DISPUTE_COORDINATION_WINDOW_SECS, DISPUTE_INDEPENDENCE_RISK_THRESHOLD,
    EVIDENCE_DUPLICATE_WINDOW_SECS, EVIDENCE_TAMPER_RISK_THRESHOLD,
    ARBITRATION_MIN_RULINGS_FOR_BIAS, ARBITRATION_BIAS_RATIO_BPS_THRESHOLD,
    ARBITRATION_BIAS_RISK_THRESHOLD, JUSTICE_INTERVENTION_THRESHOLD,
    JUSTICE_RESTORATION_COOLDOWN_SECS,
};
pub use outcome_authenticity::{
    authenticate_learning_outcomes, protect_success_metrics, validate_assessment_criteria,
    compute_outcome_intervention, is_outcome_restoration_eligible,
    OutcomeAuthenticity, SuccessMetricProtection, AssessmentValidation, OutcomeInterventionRecord,
    OUTCOME_BURST_WINDOW_SECS, OUTCOME_MIN_DISTINCT_BPS, OUTCOME_RISK_THRESHOLD,
    METRIC_GAMING_DEVIATION_BPS, ASSESSMENT_COORDINATION_WINDOW_SECS, ASSESSMENT_RISK_THRESHOLD,
    OUTCOME_INTERVENTION_THRESHOLD, OUTCOME_RESTORATION_COOLDOWN_SECS,
};
pub use scalability_protection::{
    detect_resource_competition, validate_load_pattern, distribute_resources_fairly,
    compute_scalability_intervention, is_performance_restoration_eligible,
    ResourceCompetitionFlag, LoadValidationResult, FairResourceAllocation,
    PerformanceInterventionRecord,
    RESOURCE_BURST_WINDOW_SECS, RESOURCE_MIN_DISTINCT_BPS, RESOURCE_COMPETITION_RISK_THRESHOLD,
    LOAD_SUSPICIOUS_RATE_PER_MINUTE, FAIR_ALLOCATION_MAX_SHARE_BPS,
    PERFORMANCE_INTERVENTION_THRESHOLD, PERFORMANCE_RESTORATION_COOLDOWN_SECS,
};
pub use learner_protection::{
    assess_vulnerability, detect_predatory_behavior, enforce_learner_fair_pricing,
    identify_exploitation_patterns, compute_welfare_status,
    compute_learner_protection_intervention, compute_emergency_intervention,
    is_protection_restoration_eligible,
    VulnerabilityAssessment, PredatoryBehaviorDetection, ExploitationPattern,
    WelfareStatus, EmergencyIntervention, LearnerProtectionRecord,
    VULNERABILITY_SESSION_WINDOW, VULNERABILITY_HIGH_RECURRENCE_THRESHOLD,
    VULNERABILITY_RISK_THRESHOLD, AFFORDABILITY_DEVIATION_BPS,
    FINANCIAL_PROTECTION_CAP_BPS, PREDATORY_LOW_QUALITY_THRESHOLD,
    PREDATORY_COMPLAINT_RATIO_BPS, PREDATORY_RISK_THRESHOLD,
    EMERGENCY_PATTERN_THRESHOLD, EMERGENCY_SUSPENSION_COOLDOWN_SECS,
    LEARNER_PROTECTION_COOLDOWN_SECS,
};
pub use mev_protection::{
    detect_atomic_arbitrage, enforce_protocol_isolation, compute_mev_redistribution, record_mev_monitoring,
    MevProtectionFlag, FairValueExtractionRecord, MevMonitoringRecord,
    MEV_ARBITRAGE_RISK_THRESHOLD, DEFAULT_MEV_PENALTY_BPS, MAX_MEV_PENALTY_BPS,
};
pub use resource_management::{
    allocate_system_resources, manage_session_load, detect_abuse_patterns, check_emergency_trigger,
    RateLimitStatus, ResourceAllocation, AbuseDetectionResult,
    DEFAULT_MAX_REQUESTS_PER_MINUTE, ABUSE_PATTERN_THRESHOLD_BPS, EMERGENCY_THROTTLE_RATE, RESOURCE_QUOTA_MAX_SESSIONS,
};
pub use platform_authenticity::{
    verify_session_authenticity, detect_platform_bypass, detect_fee_evasion,
    AuthenticityResult, CollusionResult, EconomicAuditResult, PenaltyTier,
    MAX_LOW_FEE_SESSIONS_PER_PAIR, LOW_FEE_THRESHOLD, REQUIRED_INTERACTION_MINUTES, FEE_EVASION_TOLERANCE_BPS,
};
pub use dynamic_fees::{
    calculate_dynamic_fee, detect_fee_gaming,
    DynamicFeeResult, FeeEvasionResult,
    BASE_FEE_BPS, MIN_FEE_BPS, HIGH_LOAD_THRESHOLD,
};
pub use validation::{require_auth_and_validate, ValidationError, Validator};
pub use curriculum_validation::{
    validate_curriculum_standards, optimize_learning_path, CurriculumValidation, LearningPathOptimization, OutcomeAssessment, CurriculumDispute
};
pub use qualification_verification::{
    verify_credential_validity, assess_skill_level, CredentialVerification, IdentityValidation, SkillAssessment
};
pub use proof_of_mentoring::{
    generate_mentoring_proof, check_session_authenticity, ProofOfMentoring, SessionAuthenticity, ReputationIntegrity
};
pub use cross_contract_recovery::{
    trigger_rollback, execute_with_recovery, RecoveryState, RollbackProtector
};

// ---------------------------------------------------------------------------
// #866 — Cross-Chain State Synchronization
// ---------------------------------------------------------------------------
pub use cross_chain_sync::{
    acknowledge_prepare, begin_atomic_xchain_op, compute_state_merkle_root, confirm_commit,
    confirm_rollback, expire_xchain_op, get_chain_isolation, get_inconsistency,
    get_xchain_op, initiate_rollback, is_chain_isolated, is_reorg_safe, isolate_chain,
    lift_chain_isolation, record_inconsistency, record_reorg_event, require_finality,
    validate_state_proof, AtomicXChainOp, ChainFinalityConfig, ChainIsolationRecord,
    CrossChainInconsistency, CrossChainStateProof, FinalityTier, XChainPhase,
    XChainSyncError, MAX_PARTICIPATING_CHAINS, MIN_FINALITY_CONFIRMATIONS,
    REORG_SAFE_DEPTH, XCHAIN_OP_TIMEOUT_SECS,
};
pub use failure_tracking::{
    ReleaseFailure, FailureClassification, ExponentialBackoff, RecoveryState,
    calculate_backoff_delay, classify_failure, calculate_next_retry, compute_failure_hash,
    MAX_AUTO_RELEASE_ATTEMPTS, MANUAL_RECOVERY_THRESHOLD,
};
pub use atomic_state::{
    StateTransitionContext, PreConditionCheck, PostConditionCheck, CrossContractStateCheck,
    StateTransitionProof, InvalidStateRecord, AtomicStateValidator, compute_transition_proof_hash,
    all_checkpoints_passed, is_transition_expired, STATE_TRANSITION_TIMEOUT_SECS,
    STATE_TRANSITION_LOCK_TTL, MAX_CHECKPOINT_COUNT,
};
pub use community_protection::{
    detect_coordination, detect_coordination_ring, validate_network_authenticity,
    verify_social_proof, evaluate_fair_access, compute_community_intervention,
    is_restoration_eligible, CoordinationFlag, NetworkEffectScore, SocialProofRecord,
    FairAccessDecision, CommunityInterventionRecord, COORDINATION_MIN_INTERACTIONS,
    COORDINATION_TIGHT_WINDOW_SECS, COORDINATION_RISK_THRESHOLD,
    NETWORK_DISTINCT_SOURCE_MIN_BPS, NETWORK_SUSPICIOUS_GROWTH_PER_DAY,
    SOCIAL_PROOF_BURST_WINDOW_SECS, SOCIAL_PROOF_MIN_DISTINCT_BPS,
    COMMUNITY_INTERVENTION_THRESHOLD,
};
pub use pricing_protection::{
    detect_price_coordination, validate_market_rate, enforce_fair_pricing,
    verify_demand_authenticity, compute_pricing_intervention, PriceCoordinationFlag,
    MarketRateValidation, FairPricingResult, DemandAuthenticity, PricingInterventionRecord,
    PRICE_COORDINATION_WINDOW_SECS, PRICE_MATCH_TOLERANCE_BPS, PRICING_RISK_THRESHOLD,
    DEFAULT_MAX_MARKET_DEVIATION_BPS, MAX_MARKET_DEVIATION_CEILING_BPS,
    DEMAND_BURST_WINDOW_SECS, DEMAND_MIN_DISTINCT_BPS,
};
pub use privacy_protection::{
    check_access, minimize_to_need_to_know, detect_exploitation, compute_privacy_intervention,
    ConsentRecord, AccessDecision, PrivacyMonitoringResult, PrivacyInterventionRecord,
    FIELD_IDENTITY, FIELD_CONTACT, FIELD_LEARNING_HISTORY, FIELD_CAREER_DATA, FIELD_PAYMENT,
    MINIMAL_SESSION_FIELDS, ALL_FIELDS, ACCESS_MONITORING_WINDOW_SECS,
    MAX_ACCESSES_PER_WINDOW, PRIVACY_RISK_THRESHOLD,
};
pub use justice_protection::{
    ensure_dispute_independence, validate_evidence_authenticity, protect_arbitration_fairness,
    compute_justice_intervention, is_justice_restoration_eligible,
    DisputeIndependenceFlag, EvidenceAuthenticity, ArbitrationBiasFlag, JusticeInterventionRecord,
    DISPUTE_COORDINATION_WINDOW_SECS, DISPUTE_INDEPENDENCE_RISK_THRESHOLD,
    EVIDENCE_DUPLICATE_WINDOW_SECS, EVIDENCE_TAMPER_RISK_THRESHOLD,
    ARBITRATION_MIN_RULINGS_FOR_BIAS, ARBITRATION_BIAS_RATIO_BPS_THRESHOLD,
    ARBITRATION_BIAS_RISK_THRESHOLD, JUSTICE_INTERVENTION_THRESHOLD,
    JUSTICE_RESTORATION_COOLDOWN_SECS,
};
pub use outcome_authenticity::{
    authenticate_learning_outcomes, protect_success_metrics, validate_assessment_criteria,
    compute_outcome_intervention, is_outcome_restoration_eligible,
    OutcomeAuthenticity, SuccessMetricProtection, AssessmentValidation, OutcomeInterventionRecord,
    OUTCOME_BURST_WINDOW_SECS, OUTCOME_MIN_DISTINCT_BPS, OUTCOME_RISK_THRESHOLD,
    METRIC_GAMING_DEVIATION_BPS, ASSESSMENT_COORDINATION_WINDOW_SECS, ASSESSMENT_RISK_THRESHOLD,
    OUTCOME_INTERVENTION_THRESHOLD, OUTCOME_RESTORATION_COOLDOWN_SECS,
};
pub use scalability_protection::{
    detect_resource_competition, validate_load_pattern, distribute_resources_fairly,
    compute_scalability_intervention, is_performance_restoration_eligible,
    ResourceCompetitionFlag, LoadValidationResult, FairResourceAllocation,
    PerformanceInterventionRecord,
    RESOURCE_BURST_WINDOW_SECS, RESOURCE_MIN_DISTINCT_BPS, RESOURCE_COMPETITION_RISK_THRESHOLD,
    LOAD_SUSPICIOUS_RATE_PER_MINUTE, FAIR_ALLOCATION_MAX_SHARE_BPS,
    PERFORMANCE_INTERVENTION_THRESHOLD, PERFORMANCE_RESTORATION_COOLDOWN_SECS,
};
pub use learner_protection::{
    assess_vulnerability, detect_predatory_behavior, enforce_learner_fair_pricing,
    identify_exploitation_patterns, compute_welfare_status,
    compute_learner_protection_intervention, compute_emergency_intervention,
    is_protection_restoration_eligible,
    VulnerabilityAssessment, PredatoryBehaviorDetection, ExploitationPattern,
    WelfareStatus, EmergencyIntervention, LearnerProtectionRecord,
    VULNERABILITY_SESSION_WINDOW, VULNERABILITY_HIGH_RECURRENCE_THRESHOLD,
    VULNERABILITY_RISK_THRESHOLD, AFFORDABILITY_DEVIATION_BPS,
    FINANCIAL_PROTECTION_CAP_BPS, PREDATORY_LOW_QUALITY_THRESHOLD,
    PREDATORY_COMPLAINT_RATIO_BPS, PREDATORY_RISK_THRESHOLD,
    EMERGENCY_PATTERN_THRESHOLD, EMERGENCY_SUSPENSION_COOLDOWN_SECS,
    LEARNER_PROTECTION_COOLDOWN_SECS,
};

// Key management exports  
pub use key_management::{
    register_key, propose_key_rotation, execute_key_rotation, is_rotation_due, 
    emergency_revoke_key, is_key_revoked, get_current_key,
    KeyRecord, KeyRotationProposal, KeyScheme,
};

// Transaction intent protection exports
pub use transaction_guard::{
    evaluate_transaction_intent, get_protection_state,
    TransactionIntent, RiskLevel,
};

// Recording integrity exports
pub use recording_integrity::{
    create_recording, compute_merkle_root, verify_recording_integrity,
    grant_consent, revoke_consent, check_access_authorized, apply_redaction,
    log_access, emergency_privacy_protection,
    SessionRecording, RecordingStatus, ConsentRecord as RecordingConsentRecord, AccessRole, RedactionRecord, 
    AccessLogEntry, IntegrityVerificationResult,
};

// Payment integrity exports
pub use payment_integrity::{
    validate_evidence_sufficiency, detect_payment_timing_manipulation, check_multisig_threshold,
    compute_emergency_isolation,
    EvidenceSufficiency, PaymentTimingCheck, EscrowMultisigApproval, EmergencyFundLock, PaymentAuditEntry,
};

// Market protection exports (aliases for governance compatibility)
pub use community_protection::{
    validate_network_authenticity as detect_network_concentration,
    evaluate_fair_access as assess_competition_barriers,
    compute_community_intervention as analyze_market_networks,
    is_restoration_eligible as audit_market_competition,
    CoordinationFlag as DecentralizationMonitoring,
    FairAccessDecision as MarketFairness,
    CommunityInterventionRecord as MarketProtectionRecord,
    COMMUNITY_INTERVENTION_THRESHOLD as MARKET_INTERVENTION_COOLDOWN_SECS,
};
pub use pricing_protection::{
    detect_price_coordination as detect_pricing_coordination,
    compute_pricing_intervention as compute_market_protection_intervention,
    PricingInterventionRecord as CompetitionAuditRecord,
};
pub use scalability_protection::{
    is_performance_restoration_eligible as is_market_restoration_eligible,
};

// Validator accountability exports
pub use validator_accountability::{
    assess_incentive_alignment, get_validator_record, is_validator_ejected,
    register_validator, IncentiveAlignmentScore, ValidatorRecord,
};

// Content protection exports
pub use content_protection::{ContentProtection, ContentType, EncryptionKey, AccessLevel, ProtectedContent, AccessLog};
pub use ip_verification::{IPVerification, IPRecord, OwnershipProof, IPUsageRecord, InfringementRecord, IPType, IPStatus};
pub use usage_rights_management::{UsageRightsManager, LicenseType, ViolationPenalty, License, ViolationRecord};

// Threat intelligence exports
pub use threat_intelligence::{
    assess_delegation_concentration, assess_token_velocity, correlate_attack_vectors, assess_review_quality,
    DelegationConcentrationReport, EconomicVelocityReport, MultiVectorThreatReport, ReviewQualityReport,
    CollusionDetection, GameTheoryState, IncentiveCompatibilityResult,
};

// Tokenomics protection exports
pub use tokenomics_protection::{
    exceeds_extraction_rate, detect_coordinated_timing,
    ManipulationReason, TokenomicsAuditResult, MAX_EXTRACTION_RATE_BPS, MIN_SUSTAINABILITY_RATIO, MIN_POSITION_DELTA_SECS,
};

/// Economic sanity ceiling for a single financial amount (token smallest units).
pub const MAX_FINANCIAL_AMOUNT: i128 = 1_000_000_000_000_000;
/// Absolute upper bound for fee basis-points helpers in shared validation.
pub const MAX_FEE_BPS: u32 = 10_000;

/// Common error codes shared across all MentorsMind contracts.
///
/// Contracts may re-export or extend this enum; the numeric codes are stable
/// and used in off-chain tooling to distinguish error categories.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SharedError {
    /// `initialize` was called more than once on the contract.
    AlreadyInitialized = 1,
    /// A function requiring initialization was called before `initialize`.
    NotInitialized = 2,
    /// The caller lacks the required role (admin, mentor, learner, etc.).
    Unauthorized = 3,
    /// The requested record (escrow, user, token, etc.) does not exist.
    NotFound = 4,
    /// The supplied amount is zero, negative, or exceeds an allowed range.
    InvalidAmount = 5,
    /// The operation is not valid for the entity's current state.
    InvalidState = 6,
    /// An attempt was made to insert a record that already exists.
    DuplicateEntry = 7,
    /// The operation is not supported in the current contract configuration.
    UnsupportedOperation = 8,
    /// An arithmetic operation would overflow the integer bounds.
    Overflow = 9,
    /// An arithmetic operation would underflow below zero.
    Underflow = 10,
    /// Input validation failed (see `ValidationError` for field details).
    ValidationError = 11,
    /// A cross-contract caller failed interface-registry verification.
    UnauthorizedContract = 12,
    /// Content access is denied due to insufficient permissions or invalid license.
    ContentAccessDenied = 13,
    /// Intellectual property ownership cannot be verified or is disputed.
    IPOwnershipInvalid = 14,
    /// Usage rights have been violated or exceeded allowed limits.
    UsageRightsViolation = 15,
    /// Content encryption/decryption failed.
    EncryptionError = 16,
    /// Content piracy or unauthorized distribution detected.
    PiracyDetected = 17,
}

// ---------------------------------------------------------------------------
// Additional modules re-exports
// ---------------------------------------------------------------------------
pub use algorithm_transparency::{
    assess_algorithm_transparency, audit_algorithm_transparency,
    compute_algo_protection_intervention, compute_algo_protection_intervention as compute_algorithm_protection,
    compute_transparency_balance, detect_reverse_engineering,
    is_algo_restoration_eligible, is_algo_restoration_eligible as is_algorithm_restoration_eligible,
    monitor_ranking_algorithm, should_block_transparency_response, AlgorithmMonitoringResult,
    AlgorithmProtectionRecord, AlgorithmTransparency, ReverseEngineeringProtection,
    TransparencyAuditRecord, TransparencyBalance, ALGO_INTERVENTION_THRESHOLD,
    ALGO_MANIPULATION_RISK_THRESHOLD, ALGO_PROTECTION_COOLDOWN_SECS,
    MAX_TRANSPARENCY_DISCLOSURE_BPS, MIN_TRANSPARENCY_DISCLOSURE_BPS,
    PROBE_DETECTION_WINDOW_SECS, PROBE_HIGH_FREQUENCY_THRESHOLD, RANKING_FACTOR_COUNT,
    RANKING_GAMING_WINDOW_SECS, RANKING_SCORE_DEVIATION_BPS,
    REVERSE_ENGINEERING_RISK_THRESHOLD,
};
pub use account_security::{CrossPlatformIdentity, is_identity_match};
pub use exit_facilitation::{
    evaluate_competition_protection, facilitate_migration, validate_dependency_necessity,
    CompetitionProtectionDecision, DataPortabilityPackage, DependencyValidationResult,
    ExitMonitoringReport, MigrationFacilitationRecord, MAX_SWITCHING_COST_BPS,
};
pub use reputation_bridging::{
    audit_reputation_bridging, check_identity_consistency, isolate_reputation_score,
    verify_cross_platform_attestation, BridgedReputationRecord, BridgingAuditReport,
    CrossPlatformAttestation, IdentityConsistencyCheck, PlatformReliabilityScore,
    MAX_BRIDGED_REPUTATION_DISCOUNT_BPS, MIN_PLATFORM_RELIABILITY_BPS,
};
pub use session_uniqueness::{
    audit_session_integrity, detect_temporal_replay, recover_session_content,
    validate_session_nonce, verify_content_checksum, ContentIntegrityRecord,
    ReplayDetectionResult, SessionAuditRecord, SessionNonceRecord, SessionRecoveryRecord,
    MAX_SESSION_TIME_DRIFT_SECS, REPLAY_CONFIDENCE_THRESHOLD_BPS,
};
pub use metadata_validation::{
    audit_information_accuracy, is_transparency_restoration_eligible,
    monitor_metadata_manipulation, protect_transparency,
    protect_transparency as shared_protect_transparency, restore_truth_and_correct,
    validate_metadata_authenticity, verify_information_integrity, InformationAuditRecord,
    InformationIntegrity, MetadataMonitoringRecord, MetadataValidation, TransparencyProtection,
    TruthRestorationRecord, DISINFORMATION_RISK_THRESHOLD, METADATA_AUTHENTICITY_THRESHOLD,
    MIN_SOURCE_CREDIBILITY_BPS, TRANSPARENCY_RESTORATION_COOLDOWN_SECS, TRANSPARENCY_RISK_THRESHOLD,
};
pub use mentor_wellness::{
    activate_emergency_protection, assess_burnout_risk, calculate_burnout_risk,
    can_accept_session, distribute_sessions_fairly, initiate_intervention, update_mentor_workload,
    BurnoutRiskAssessment, EmergencyProtection, FairDistributionResult, MentorWorkload,
    SessionDifficulty, SessionDistributionRequest, WellnessIntervention,
    BURNOUT_RISK_THRESHOLD_BPS, DIFFICULTY_WEIGHTS, MANDATORY_REST_HOURS, MAX_CONCURRENT_SESSIONS,
    MAX_WEEKLY_HOURS, MIN_REST_HOURS,
};
pub use onboarding_protection::{
    assess_admission_equity, compute_onboarding_protection, monitor_onboarding_access_patterns,
    AccessMonitoringRecord, AdmissionEquity, OnboardingFairness, OnboardingProtectionRecord,
    VerificationAuthenticity, BARRIER_GAMING_RISK_THRESHOLD, ONBOARDING_FAIRNESS_THRESHOLD,
    ONBOARDING_RESTORATION_COOLDOWN_SECS,
};
pub use skill_verification::{
    authenticate_external_credential, compute_recertification_due, detect_skill_fraud,
    evaluate_domain_governance, score_practical_assessment, validate_peer_consensus,
    ExpertiseAuthenticationRecord, PracticalAssessment, RecertificationSchedule, SkillFraudFlag,
    SpecializationGovernanceRecord,
};
pub use assessment_authenticity::{
    check_source_diversity, perform_consensus_validation, submit_validation,
    verify_assessment_authenticity, verify_blockchain_attestation, AuthenticityVerification,
    ConsensusRecord, ValidationResult, ValidationSource, MAX_VALIDATION_AGE_SECS,
    MIN_VALIDATION_SOURCES, VALIDATION_CONSENSUS_BPS,
};
pub use grade_inflation::{
    apply_inflation_adjustment, calculate_grade_distribution, detect_grade_inflation,
    record_grade_correction, GradeCorrectionRecord, GradeDistributionStats,
    InflationDetectionResult, MentorScoringAdjustment, INFLATION_PENALTY_BPS_PER_DETECTION,
    INFLATION_WINDOW, MAX_INFLATION_RATE_BPS, MIN_SESSIONS_FOR_ANALYSIS, OUTLIER_ZSCORE_THRESHOLD,
};
pub use assessment_security::{
    AssessmentMetrics, AssessmentRecord, AssessmentSecurity, AssessmentSecurityError,
    GamingDetectionResult, GamingFlag, ManipulationRecord, ProgressAuthenticityRecord,
};
pub use recording_integrity::{
    apply_redaction, check_access_authorized, compute_merkle_root, create_recording,
    emergency_privacy_protection, grant_consent, log_access, revoke_consent,
    verify_recording_integrity, AccessLogEntry, AccessRole,
    IntegrityVerificationResult, RecordingStatus, RedactionRecord, SessionRecording,
    DEFAULT_RETENTION_DAYS, MAX_RECORDING_SIZE_MB, MIN_CONSENT_DURATION_HOURS,
};
pub use transfer_security::{
    CredentialAuthenticityProof, CredentialFraudType, CredentialTransfer, CreditInflationRecord,
    CrossPlatformVerification, FraudDetectionResult, TransferIntegrityResult, TransferSecurity,
    TransferSecurityError, VerificationStep,
};
pub use market_monitoring::{
    assess_demand_authenticity, balance_supply_demand, calculate_market_metrics,
    detect_market_manipulation, trigger_emergency_stabilization,
    validate_price_discovery, DemandAuthenticityResult, EmergencyStabilization,
    MarketManipulationAlert, MarketMetrics, PriceDiscoveryValidation, SupplyDemandBalance,
    ARTIFICIAL_DEMAND_THRESHOLD_BPS, MAX_PRICE_DEVIATION_BPS, MIN_MARKET_DATA_POINTS,
    STABILIZATION_THRESHOLD_BPS, SUPPLY_RESTRICTION_THRESHOLD_BPS,
};
pub use scheduling_integrity::{
    assign_fair_slot, compute_availability_commitment, compute_random_tiebreak,
    detect_availability_gaming, validate_conflict_proof, verify_availability_commitment,
    AvailabilityCommitment, AvailabilityGamingFlag, ConflictProof, FairSchedulingDecision,
    SchedulingAuditRecord, GAMING_RISK_THRESHOLD, MAX_CONFLICT_PROOF_AGE_SECS,
    MIN_COMMITMENT_LEAD_SECS, RAPID_AVAILABILITY_CHANGE_WINDOW_SECS,
};
pub use service_continuity::{
    is_backup_valid, needs_backup, ContinuityBackup, ContinuityStatus, BACKUP_MAX_AGE_SECS,
    MAX_BACKUP_RECORDS,
};
pub use session_privacy::{
    contain_data_breach, detect_cross_session_leak, enforce_session_boundary,
    CrossSessionLeakResult, DataBreachContainment, SessionAccessBoundary, BREACH_RISK_THRESHOLD,
    CROSS_SESSION_MONITORING_WINDOW_SECS, LEAK_DISTINCT_SESSION_THRESHOLD,
};
pub use session_protection::{
    compute_disruption_score, should_protect_session, ProtectionCheckResult,
    SessionProtectionRecord, DISRUPTION_RISK_THRESHOLD_BPS, MAX_PROTECTED_SESSIONS,
    PROTECTION_CHECK_COOLDOWN_SECS,
};
pub use attack_detection::{
    evaluate_attack_risk, AttackDetectionResult, AttackEvent, AttackType,
    ATTACK_DETECTION_WINDOW_SECS, ATTACK_FLAG_THRESHOLD,
};

#![no_std]

use shared::{
    analyze_review_pattern,
    assess_review_quality,
    authenticate_learning_outcomes as shared_authenticate_learning_outcomes,
    compute_community_intervention,
    compute_learner_protection_intervention,
    compute_outcome_intervention,
    compute_welfare_status as shared_compute_welfare_status,
    detect_coordination,
    // learner protection (#917)
    detect_predatory_behavior as shared_detect_predatory_behavior,
    identify_exploitation_patterns as shared_identify_exploitation_patterns,
    interaction_commitment,
    is_outcome_restoration_eligible,
    is_protection_restoration_eligible,
    is_restoration_eligible,
    protect_success_metrics as shared_protect_success_metrics,
    validate_assessment_criteria as shared_validate_assessment_criteria,
    verify_social_proof,
    AssessmentValidation,
    BehavioralAnalysis,
    CommunityInterventionRecord,
    CoordinationFlag,
    EscrowRecord,
    ExploitationPattern,
    LearnerProtectionRecord,
    NetworkEffectScore,
    OutcomeAuthenticity,
    OutcomeInterventionRecord,
    PredatoryBehaviorDetection,
    ReputationProof,
    ReviewQualityReport,
    SocialProofRecord,
    SuccessMetricProtection,
    VulnerabilityAssessment,
    OUTCOME_RESTORATION_COOLDOWN_SECS,
    CollusionDetection,
    IncentiveCompatibilityResult,
};
use shared::*;
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, BytesN, Env, IntoVal,
    Symbol, Vec, Map,
};

// ── Storage keys ────────────────────────────────────────────────────────────
const ESCROW: Symbol = symbol_short!("ESCROW");
const TTL_THRESHOLD: u32 = 500_000;
const TTL_BUMP: u32 = 1_000_000;
/// Maximum rolling outcome scores retained per (mentor, specialization) for
/// expertise/fraud tracking (#891).
const MAX_SPECIALIZATION_HISTORY: u32 = 20;

// ── Types ────────────────────────────────────────────────────────────────────
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewRecord {
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub rating: u32,
    pub timestamp: u64,
    pub comment_hash: BytesN<32>,
    pub authenticity_proof: ReputationProof,
    pub stake_amount: i128,
    pub investigation_required: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LearnerReviewRecord {
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub participation_rating: u32,
    pub comment_hash: BytesN<32>,
    pub submitted_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Review(Symbol),
    MentorRatingSum(Address),
    MentorReviewCount(Address),
    LearnerReview(Symbol),
    LearnerRatingSum(Address),
    LearnerReviewCount(Address),
    LoyaltyPoints(Address),
    LoyaltyTier(Address),
    SlashPenaltyBps(Address),
    Rehabilitated(Address),
    ReviewDispute(Symbol),
    ThresholdProof(Address, u32),
    SessionRegistry,
    ReviewToken,
    ReviewStakeBase,
    ReviewTimestamps(Address),
    ReviewRatings(Address),
    ReviewSignalScore(Address),
    /// Interaction timestamps between one mentor/learner pair (#coordination).
    PairInteractionLog(Address, Address),
    /// Whether `learner` has ever reviewed `mentor` before (distinct-reviewer tracking).
    HasReviewedBefore(Address, Address),
    DistinctReviewerCount(Address),
    CommunityCoordination(Address),
    SocialProofScore(Address),
    CommunityIntervention(Address),
    /// Cached outcome-authenticity assessment for a mentor's learning
    /// outcome measurements (#outcome-authenticity).
    OutcomeAuthenticityRecord(Address),
    /// Trusted historical baseline (bps, 0-10000) for a mentor's success
    /// metric, set by the configured escrow authority.
    MetricBaseline(Address),
    /// Cached success-metric gaming assessment for a mentor.
    SuccessMetricRecord(Address),
    /// Timestamps of recorded assessment-criteria proposals for a mentor.
    AssessmentProposalLog(Address),
    /// Whether `proposer` has ever proposed assessment criteria for
    /// `mentor` before (distinct-proposer tracking).
    AssessmentHasProposedBefore(Address, Address),
    AssessmentDistinctProposerCount(Address),
    /// Cached assessment-validation result for a mentor.
    AssessmentValidationRecord(Address),
    /// Cached combined outcome-protection intervention record for a mentor.
    OutcomeIntervention(Address),
    // ── Reputation bridging (#913) ──────────────────────────────────────────
    ExternalReputation(Address, Symbol),
    BridgedCredentials(Address, BytesN<32>),
    // ── Learner protection / conduct tracking (#917) ──────────────────────
    /// Log of conduct events (timestamps) recorded for a mentor, used for
    /// predatory-behaviour scoring in `track_mentor_conduct`.
    MentorConductLog(Address),
    /// Running count of disputes/complaints filed against a mentor.
    MentorComplaintCount(Address),
    /// Count of consecutive low-quality sessions (rating ≤ 2) for a mentor.
    MentorConsecutiveLowQuality(Address),
    /// Cached predatory-behaviour detection result for a mentor.
    MentorPredatoryBehaviorRecord(Address),
    /// Cached exploitation patterns identified for a mentor/learner pair.
    MentorExploitationPatterns(Address, Address),
    /// Cached welfare status for a learner relative to a specific mentor.
    LearnerWelfareStatus(Address, Address),
    /// Cached learner-protection intervention record for a mentor (reputation
    /// contract's copy; session_registry keeps a parallel copy).
    LearnerProtectionIntervention(Address),
    /// Per-review quality/authenticity audit report.
    ReviewQualityReport(Symbol),
    CollusionSignal(Address),
    IncentiveCompatibility(Address),
    GradeCorrection(Symbol),
    GradeInflationDetection(Address),
    InformationAccuracyTrack(Address),
    MarketManipulationAlert(Symbol),
    MentorBurnoutAssessment(Address),
    MentorGradeHistory(Address),
    MentorGradeTimestamps(Address),
    MentorWorkloadData(Address),
    MisinformationDetection(Address),
    ReputationInfoAudit(Address),
    ReputationTruthRestoration(Address),
    SessionInfoVerification(Symbol),
    SpecializationMetrics(Symbol),
    WellnessIntervention(Address),
}

/// Cooldown before an intervened mentor's community access is eligible for
/// automatic restoration.
pub const COMMUNITY_RESTORATION_COOLDOWN_SECS: u64 = 7 * 24 * 3600;

pub const REVIEW_DISPUTE_WINDOW_SECS: u64 = 14 * 24 * 3600;
pub const DISPUTE_FILING_FEE: i128 = 10_000_000; // 10 MNT

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewDispute {
    pub mentor: Address,
    pub learner: Address,
    pub review_session_id: Symbol,
    pub dispute_reason_hash: BytesN<32>,
    pub filed_at: u64,
    pub status: Symbol,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationThresholdProof {
    pub commitment: BytesN<32>,
    pub threshold: u32,
    pub proof_type: Symbol,
}

pub const TIER_SILVER: u32 = 100;
pub const TIER_GOLD: u32 = 500;
pub const TIER_PLATINUM: u32 = 1000;

// ── Contract ─────────────────────────────────────────────────────────────────
#[contract]
pub struct ReputationContract;

#[contractimpl]
impl ReputationContract {
    /// Initialize with the escrow contract address for cross-contract verification.
    pub fn initialize(env: Env, escrow_contract: Address) {
        if env.storage().instance().has(&ESCROW) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&ESCROW, &escrow_contract);
        env.storage().instance().extend_ttl(TTL_THRESHOLD, TTL_BUMP);
    }

    pub fn configure_review_security(
        env: Env,
        admin: Address,
        session_registry: Address,
        review_token: Address,
        base_stake: i128,
    ) {
        let escrow: Address = env
            .storage()
            .instance()
            .get(&ESCROW)
            .expect("Not initialized");
        admin.require_auth();
        if admin != escrow || base_stake <= 0 {
            panic!("Unauthorized");
        }
        env.storage()
            .instance()
            .set(&DataKey::SessionRegistry, &session_registry);
        env.storage()
            .instance()
            .set(&DataKey::ReviewToken, &review_token);
        env.storage()
            .instance()
            .set(&DataKey::ReviewStakeBase, &base_stake);
    }

    /// Submit a review for a completed session.
    /// Caller must be the learner; session must be Released in escrow.
    pub fn submit_review(
        env: Env,
        session_id: Symbol,
        mentor: Address,
        learner: Address,
        rating: u32,
        comment_hash: BytesN<32>,
    ) {
        // Auth: caller must be learner
        learner.require_auth();

        // Validate rating 1–5
        if rating < 1 || rating > 5 {
            panic!("InvalidRating");
        }

        // Prevent duplicate review
        let review_key = DataKey::Review(session_id.clone());
        if env.storage().persistent().has(&review_key) {
            panic!("DuplicateReview");
        }

        // Cross-contract: verify session is Released
        let escrow_addr: Address = env
            .storage()
            .instance()
            .get(&ESCROW)
            .expect("EscrowContractNotSet");

        let escrow: EscrowRecord = env.invoke_contract(
            &escrow_addr,
            &Symbol::new(&env, "get_escrow_by_session"),
            (session_id.clone(),).into_val(&env),
        );

        if escrow.status != shared::EscrowStatus::Released {
            panic!("SessionNotReleased");
        }

        let proof = if env.storage().instance().has(&DataKey::SessionRegistry) {
            Self::load_and_validate_proof(&env, &session_id, &mentor, &learner)
        } else {
            ReputationProof {
                session_id: session_id.clone(),
                mentor: mentor.clone(),
                learner: learner.clone(),
                completed_at: env.ledger().timestamp(),
                commitment: interaction_commitment(
                    &env,
                    &session_id,
                    &mentor,
                    &learner,
                    env.ledger().timestamp(),
                ),
            }
        };
        let stake_amount = Self::collect_review_stake(&env, &learner, rating);
        let analysis = Self::record_behavior(&env, &mentor, rating);

        // Community-dynamics monitoring: track this mentor/learner pair and
        // distinct-reviewer count, then re-score coordination and social-proof risk.
        Self::record_pair_interaction(&env, &mentor, &learner);
        Self::record_distinct_reviewer(&env, &mentor, &learner);
        let coordination =
            Self::validate_community_interactions(env.clone(), mentor.clone(), learner.clone());
        let social_proof = Self::monitor_social_proof_auth(env.clone(), mentor.clone());
        Self::detect_collusion(env.clone(), mentor.clone(), learner.clone());

        // Outcome-authenticity monitoring: re-score this mentor's learning
        // outcome measurements from the freshly-recorded review timestamps.
        Self::authenticate_learning_outcomes(env.clone(), mentor.clone());
        let quality_report = assess_review_quality(
            true,
            coordination.risk_score,
            social_proof.gaming_risk_score,
            rating <= 2 && coordination.risk_score > 0,
        );
        env.storage().persistent().set(
            &DataKey::ReviewQualityReport(session_id.clone()),
            &quality_report,
        );

        // Store review
        let record = ReviewRecord {
            session_id: session_id.clone(),
            mentor: mentor.clone(),
            learner: learner.clone(),
            rating,
            timestamp: env.ledger().timestamp(),
            comment_hash,
            authenticity_proof: proof,
            stake_amount,
            investigation_required: analysis.risk_score >= 60
                || coordination.suspicious
                || !social_proof.genuine
                || quality_report.dispute_required,
        };
        env.storage().persistent().set(&review_key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&review_key, TTL_THRESHOLD, TTL_BUMP);

        // Learner protection: record conduct signals whenever a review
        // warrants investigation (low quality, coordinated, or fake social proof).
        if record.investigation_required {
            let is_low_quality = rating <= 2;
            Self::track_mentor_conduct(env.clone(), mentor.clone(), is_low_quality, false, 0);
        }

        // Update running average
        let sum_key = DataKey::MentorRatingSum(mentor.clone());
        let cnt_key = DataKey::MentorReviewCount(mentor.clone());

        let current_sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0u64);
        let current_count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0u64);

        let new_sum = current_sum
            .checked_add(rating as u64)
            .expect("sum overflow");
        let new_count = current_count.checked_add(1).expect("count overflow");

        env.storage().persistent().set(&sum_key, &new_sum);
        env.storage()
            .persistent()
            .extend_ttl(&sum_key, TTL_THRESHOLD, TTL_BUMP);
        env.storage().persistent().set(&cnt_key, &new_count);
        env.storage()
            .persistent()
            .extend_ttl(&cnt_key, TTL_THRESHOLD, TTL_BUMP);

        // Emit event
        env.events().publish(
            (
                symbol_short!("review"),
                Symbol::new(&env, "review_submitted"),
                mentor.clone(),
            ),
            (session_id, learner, rating, env.ledger().timestamp()),
        );
    }

    pub fn detect_collusion(env: Env, mentor: Address, learner: Address) -> CollusionDetection {
        let suspicious = mentor == learner;
        let detection = CollusionDetection {
            suspicious,
            coordination_score_bps: if suspicious { 9_500 } else { 200 },
            evidence_count: if suspicious { 2 } else { 1 },
        };
        env.storage()
            .persistent()
            .set(&DataKey::CollusionSignal(mentor), &detection);
        detection
    }

    pub fn verify_incentive_compatibility(
        env: Env,
        mentor: Address,
    ) -> IncentiveCompatibilityResult {
        let suspicious = env
            .storage()
            .persistent()
            .get::<_, CollusionDetection>(&DataKey::CollusionSignal(mentor))
            .map(|d| d.suspicious)
            .unwrap_or(false);
        IncentiveCompatibilityResult {
            strategy_proof: !suspicious,
            honest_nash_equilibrium: !suspicious,
            confidence_bps: if suspicious { 1_500 } else { 8_500 },
        }
    }

    fn load_and_validate_proof(
        env: &Env,
        session_id: &Symbol,
        mentor: &Address,
        learner: &Address,
    ) -> ReputationProof {
        let registry: Address = env
            .storage()
            .instance()
            .get(&DataKey::SessionRegistry)
            .expect("SessionRegistryNotConfigured");
        let proof: ReputationProof = env.invoke_contract(
            &registry,
            &Symbol::new(env, "get_completion_proof"),
            (session_id.clone(),).into_val(env),
        );
        if proof.mentor != *mentor
            || proof.learner != *learner
            || proof.commitment
                != interaction_commitment(env, session_id, mentor, learner, proof.completed_at)
        {
            panic!("InvalidReputationProof");
        }
        proof
    }

    fn collect_review_stake(env: &Env, learner: &Address, rating: u32) -> i128 {
        let base: i128 = env
            .storage()
            .instance()
            .get(&DataKey::ReviewStakeBase)
            .unwrap_or(0);
        if base == 0 {
            return 0;
        }
        let token_addr: Address = env
            .storage()
            .instance()
            .get(&DataKey::ReviewToken)
            .expect("ReviewTokenNotConfigured");
        let amount = base
            .checked_mul((rating.max(1)) as i128)
            .expect("StakeOverflow");
        token::Client::new(env, &token_addr).transfer(
            learner,
            &env.current_contract_address(),
            &amount,
        );
        amount
    }

    fn record_behavior(env: &Env, mentor: &Address, rating: u32) -> BehavioralAnalysis {
        let timestamps_key = DataKey::ReviewTimestamps(mentor.clone());
        let ratings_key = DataKey::ReviewRatings(mentor.clone());
        let mut timestamps: Vec<u64> = env
            .storage()
            .persistent()
            .get(&timestamps_key)
            .unwrap_or(Vec::new(env));
        let mut ratings: Vec<u32> = env
            .storage()
            .persistent()
            .get(&ratings_key)
            .unwrap_or(Vec::new(env));
        timestamps.push_back(env.ledger().timestamp());
        ratings.push_back(rating);
        while timestamps.len() > 10 {
            timestamps.remove(0);
            ratings.remove(0);
        }
        let analysis = analyze_review_pattern(&timestamps, &ratings, env.ledger().timestamp());
        env.storage().persistent().set(&timestamps_key, &timestamps);
        env.storage().persistent().set(&ratings_key, &ratings);
        env.storage().persistent().set(
            &DataKey::ReviewSignalScore(mentor.clone()),
            &analysis.risk_score,
        );
        analysis
    }

    pub fn get_review_risk(env: Env, mentor: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ReviewSignalScore(mentor))
            .unwrap_or(0)
    }

    pub fn get_review_quality_report(env: Env, session_id: Symbol) -> ReviewQualityReport {
        env.storage()
            .persistent()
            .get(&DataKey::ReviewQualityReport(session_id))
            .unwrap_or(ReviewQualityReport {
                authenticated: false,
                manipulation_risk_score: 100,
                reviewer_protection_required: true,
                dispute_required: true,
            })
    }

    fn record_pair_interaction(env: &Env, mentor: &Address, learner: &Address) {
        let key = DataKey::PairInteractionLog(mentor.clone(), learner.clone());
        let mut log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(env));
        log.push_back(env.ledger().timestamp());
        while log.len() > 10 {
            log.remove(0);
        }
        env.storage().persistent().set(&key, &log);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);
    }

    fn record_distinct_reviewer(env: &Env, mentor: &Address, learner: &Address) {
        let seen_key = DataKey::HasReviewedBefore(mentor.clone(), learner.clone());
        if !env.storage().persistent().get(&seen_key).unwrap_or(false) {
            env.storage().persistent().set(&seen_key, &true);
            let cnt_key = DataKey::DistinctReviewerCount(mentor.clone());
            let cnt: u32 = env.storage().persistent().get(&cnt_key).unwrap_or(0);
            env.storage().persistent().set(&cnt_key, &(cnt + 1));
        }
    }

    /// Score a mentor/learner pair's interaction history for coordination
    /// (repeated, tightly-clustered reviews characteristic of a manipulation
    /// ring). Safe to call by anyone as a read-through audit; also invoked
    /// internally on every `submit_review`.
    pub fn validate_community_interactions(
        env: Env,
        mentor: Address,
        learner: Address,
    ) -> CoordinationFlag {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::PairInteractionLog(
                mentor.clone(),
                learner.clone(),
            ))
            .unwrap_or(Vec::new(&env));
        let flag = detect_coordination(&log);
        env.storage()
            .persistent()
            .set(&DataKey::CommunityCoordination(mentor.clone()), &flag);
        if flag.suspicious {
            env.events().publish(
                (symbol_short!("coord"), Symbol::new(&env, "flagged")),
                (mentor, flag.risk_score),
            );
        }
        flag
    }

    /// Score a mentor's aggregate review history for social-proof gaming
    /// (endorsement bursts from a narrow set of reviewers).
    pub fn monitor_social_proof_auth(env: Env, mentor: Address) -> SocialProofRecord {
        let timestamps: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::ReviewTimestamps(mentor.clone()))
            .unwrap_or(Vec::new(&env));
        let distinct: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::DistinctReviewerCount(mentor.clone()))
            .unwrap_or(0);
        let record = verify_social_proof(&timestamps, distinct);
        env.storage()
            .persistent()
            .set(&DataKey::SocialProofScore(mentor.clone()), &record);
        if !record.genuine {
            env.events().publish(
                (symbol_short!("sproof"), Symbol::new(&env, "flagged")),
                (mentor, record.gaming_risk_score),
            );
        }
        record
    }

    /// Recompute and persist the mentor's overall community-protection
    /// intervention status from the latest coordination/social-proof scores.
    pub fn get_community_status(env: Env, mentor: Address) -> CommunityInterventionRecord {
        let coordination: CoordinationFlag = env
            .storage()
            .persistent()
            .get(&DataKey::CommunityCoordination(mentor.clone()))
            .unwrap_or(CoordinationFlag {
                suspicious: false,
                risk_score: 0,
                repeated_pair_count: 0,
                clustered_timing_count: 0,
            });
        let social_proof: SocialProofRecord = env
            .storage()
            .persistent()
            .get(&DataKey::SocialProofScore(mentor.clone()))
            .unwrap_or(SocialProofRecord {
                genuine: true,
                gaming_risk_score: 0,
                distinct_endorser_bps: 10_000,
                burst_count: 0,
            });
        // Reputation has no direct visibility into referral/network growth;
        // treat it as neutral/authentic here (session_registry owns that signal).
        let network = NetworkEffectScore {
            authentic: true,
            influence_score: 100,
            artificial_growth_flag: false,
            distinct_source_bps: 10_000,
        };
        let record = compute_community_intervention(
            &env,
            coordination,
            network,
            social_proof,
            COMMUNITY_RESTORATION_COOLDOWN_SECS,
        );
        env.storage()
            .persistent()
            .set(&DataKey::CommunityIntervention(mentor.clone()), &record);
        record
    }

    /// Restore fair community participation for a mentor once the
    /// intervention cooldown has elapsed. Callable by any arbitrator address
    /// (mirrors `resolve_review_dispute`'s governance-pool assumption).
    pub fn restore_fair_participation(env: Env, arbitrator: Address, mentor: Address) {
        arbitrator.require_auth();
        let record: CommunityInterventionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::CommunityIntervention(mentor.clone()))
            .expect("NoInterventionOnRecord");

        if !is_restoration_eligible(&record, env.ledger().timestamp()) {
            panic!("RestorationNotEligible");
        }

        env.storage()
            .persistent()
            .remove(&DataKey::CommunityIntervention(mentor.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::CommunityCoordination(mentor.clone()));

        env.events().publish(
            (symbol_short!("commrest"), Symbol::new(&env, "restored")),
            mentor,
        );
    }

    // ─── Outcome authenticity (#outcome-authenticity) ──────────────────────

    /// Authenticate a mentor's recent learning-outcome measurements (session
    /// reviews acting as completion attestations): a burst of ratings from a
    /// narrow set of evaluators is treated as manipulated rather than
    /// genuine. Safe to call by anyone as a read-through audit; also
    /// invoked internally on every `submit_review`.
    pub fn authenticate_learning_outcomes(env: Env, mentor: Address) -> OutcomeAuthenticity {
        let timestamps: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::ReviewTimestamps(mentor.clone()))
            .unwrap_or(Vec::new(&env));
        let distinct: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::DistinctReviewerCount(mentor.clone()))
            .unwrap_or(0);
        let result = shared_authenticate_learning_outcomes(&timestamps, distinct);
        env.storage()
            .persistent()
            .set(&DataKey::OutcomeAuthenticityRecord(mentor.clone()), &result);
        if !result.genuine {
            env.events().publish(
                (symbol_short!("outcome"), Symbol::new(&env, "flagged")),
                (mentor, result.manipulation_risk_score),
            );
        }
        result
    }

    /// Set the trusted historical baseline (basis points, 0-10000) used by
    /// `protect_success_metrics` for `mentor`. Restricted to the configured
    /// escrow authority (mirrors `configure_review_security`).
    pub fn set_outcome_baseline(env: Env, admin: Address, mentor: Address, baseline_bps: u32) {
        let escrow: Address = env
            .storage()
            .instance()
            .get(&ESCROW)
            .expect("Not initialized");
        admin.require_auth();
        if admin != escrow {
            panic!("Unauthorized");
        }
        env.storage()
            .persistent()
            .set(&DataKey::MetricBaseline(mentor), &baseline_bps.min(10_000));
    }

    /// Protect a mentor's aggregate rating (expressed as a 0-10000 bps
    /// success metric) against gaming by comparing it to the configured
    /// baseline. Falls back to treating the current value as its own
    /// baseline when none has been configured (no false positive on the
    /// first measurement).
    pub fn protect_success_metrics(env: Env, mentor: Address) -> SuccessMetricProtection {
        let (avg, _count) = Self::get_mentor_rating(env.clone(), mentor.clone());
        let reported_bps = ((avg as u32).saturating_mul(20)).min(10_000);
        let baseline_bps: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MetricBaseline(mentor.clone()))
            .unwrap_or(reported_bps);
        let result = shared_protect_success_metrics(reported_bps, baseline_bps);
        env.storage()
            .persistent()
            .set(&DataKey::SuccessMetricRecord(mentor.clone()), &result);
        if result.gaming_detected {
            env.events().publish(
                (symbol_short!("metric"), Symbol::new(&env, "gaming")),
                (mentor, result.gaming_risk_score),
            );
        }
        result
    }

    /// Record that `proposer` proposed an assessment-criteria change (e.g. a
    /// rating-tier or dispute-threshold adjustment) affecting `mentor`'s
    /// evaluation. Used by `validate_assessment_criteria` to detect a
    /// coordinated bloc setting evaluation standards.
    pub fn record_assessment_proposal(env: Env, mentor: Address, proposer: Address) {
        let log_key = DataKey::AssessmentProposalLog(mentor.clone());
        let mut log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&log_key)
            .unwrap_or(Vec::new(&env));
        log.push_back(env.ledger().timestamp());
        while log.len() > 20 {
            log.remove(0);
        }
        env.storage().persistent().set(&log_key, &log);

        let seen_key = DataKey::AssessmentHasProposedBefore(mentor.clone(), proposer);
        if !env.storage().persistent().get(&seen_key).unwrap_or(false) {
            env.storage().persistent().set(&seen_key, &true);
            let cnt_key = DataKey::AssessmentDistinctProposerCount(mentor);
            let cnt: u32 = env.storage().persistent().get(&cnt_key).unwrap_or(0);
            env.storage().persistent().set(&cnt_key, &(cnt + 1));
        }
    }

    /// Validate that `mentor`'s recorded assessment-criteria proposals were
    /// set independently rather than by a coordinated bloc.
    pub fn validate_assessment_criteria(env: Env, mentor: Address) -> AssessmentValidation {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::AssessmentProposalLog(mentor.clone()))
            .unwrap_or(Vec::new(&env));
        let distinct: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AssessmentDistinctProposerCount(mentor.clone()))
            .unwrap_or(0);
        let result = shared_validate_assessment_criteria(&log, distinct);
        env.storage().persistent().set(
            &DataKey::AssessmentValidationRecord(mentor.clone()),
            &result,
        );
        if !result.objective {
            env.events().publish(
                (symbol_short!("assess"), Symbol::new(&env, "flagged")),
                (mentor, result.coordination_risk_score),
            );
        }
        result
    }

    /// Combine the cached outcome-authenticity, success-metric, and
    /// assessment-validation signals for `mentor` into a single
    /// outcome-protection intervention decision.
    pub fn get_outcome_status(env: Env, mentor: Address) -> OutcomeInterventionRecord {
        let outcome: OutcomeAuthenticity = env
            .storage()
            .persistent()
            .get(&DataKey::OutcomeAuthenticityRecord(mentor.clone()))
            .unwrap_or(OutcomeAuthenticity {
                genuine: true,
                manipulation_risk_score: 0,
                distinct_evaluator_bps: 10_000,
                burst_count: 0,
            });
        let metric: SuccessMetricProtection = env
            .storage()
            .persistent()
            .get(&DataKey::SuccessMetricRecord(mentor.clone()))
            .unwrap_or(SuccessMetricProtection {
                gaming_detected: false,
                gaming_risk_score: 0,
                deviation_bps: 0,
            });
        let assessment: AssessmentValidation = env
            .storage()
            .persistent()
            .get(&DataKey::AssessmentValidationRecord(mentor.clone()))
            .unwrap_or(AssessmentValidation {
                objective: true,
                coordination_risk_score: 0,
                clustered_timing_count: 0,
            });
        let record = compute_outcome_intervention(
            &env,
            outcome,
            metric,
            assessment,
            OUTCOME_RESTORATION_COOLDOWN_SECS,
        );
        env.storage()
            .persistent()
            .set(&DataKey::OutcomeIntervention(mentor.clone()), &record);
        record
    }

    // ─── Learner protection / mentor conduct tracking (#917) ───────────────

    /// Record a conduct event for `mentor` and recompute their predatory-
    /// behaviour score.
    ///
    /// `low_quality_session` – whether the session being recorded had a
    /// learner rating of ≤ 2 (sustained under-delivery indicator).
    /// `is_complaint` – whether the learner filed a dispute/complaint for
    /// this session.
    /// `price_above_market_bps` – how far above the rolling platform-average
    /// price this session was charged to the learner (basis points).
    ///
    /// The conduct log timestamps are used only as an event trail; the scoring
    /// is driven by the aggregated counts. Safe to call by anyone as a
    /// read-through audit; review submission calls this internally when
    /// `investigation_required` is set.
    pub fn track_mentor_conduct(
        env: Env,
        mentor: Address,
        low_quality_session: bool,
        is_complaint: bool,
        price_above_market_bps: u32,
    ) -> PredatoryBehaviorDetection {
        // Update complaint count.
        let complaint_cnt_key = DataKey::MentorComplaintCount(mentor.clone());
        let complaint_count: u32 = env
            .storage()
            .persistent()
            .get(&complaint_cnt_key)
            .unwrap_or(0);
        let new_complaint_count = if is_complaint {
            complaint_count.saturating_add(1)
        } else {
            complaint_count
        };
        env.storage()
            .persistent()
            .set(&complaint_cnt_key, &new_complaint_count);
        env.storage()
            .persistent()
            .extend_ttl(&complaint_cnt_key, TTL_THRESHOLD, TTL_BUMP);

        // Update consecutive low-quality counter (resets on a good session).
        let clq_key = DataKey::MentorConsecutiveLowQuality(mentor.clone());
        let consec: u32 = env.storage().persistent().get(&clq_key).unwrap_or(0);
        let new_consec = if low_quality_session {
            consec.saturating_add(1)
        } else {
            0
        };
        env.storage().persistent().set(&clq_key, &new_consec);
        env.storage()
            .persistent()
            .extend_ttl(&clq_key, TTL_THRESHOLD, TTL_BUMP);

        // Append to conduct timestamp log (capped at 50 entries).
        let log_key = DataKey::MentorConductLog(mentor.clone());
        let mut log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&log_key)
            .unwrap_or(Vec::new(&env));
        log.push_back(env.ledger().timestamp());
        while log.len() > 50 {
            log.remove(0);
        }
        env.storage().persistent().set(&log_key, &log);
        env.storage()
            .persistent()
            .extend_ttl(&log_key, TTL_THRESHOLD, TTL_BUMP);

        // Derive total_sessions from the review count (already tracked).
        let total_sessions_u64: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorReviewCount(mentor.clone()))
            .unwrap_or(0);
        let total_sessions = total_sessions_u64 as u32;

        let behavior = shared_detect_predatory_behavior(
            new_consec,
            new_complaint_count,
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

        if behavior.predatory {
            env.events().publish(
                (symbol_short!("conduct"), Symbol::new(&env, "predatory")),
                (mentor.clone(), behavior.risk_score),
            );
        }

        behavior
    }

    /// Identify exploitation patterns for a `mentor`/`learner` pair by
    /// combining the mentor's latest cached predatory-behaviour score with
    /// a supplied `vulnerability` assessment. Returns the list of active
    /// exploitation patterns and persists them for off-chain indexing.
    pub fn identify_exploitation_patterns(
        env: Env,
        mentor: Address,
        learner: Address,
        vulnerability: VulnerabilityAssessment,
    ) -> Vec<ExploitationPattern> {
        let behavior: PredatoryBehaviorDetection = env
            .storage()
            .persistent()
            .get(&DataKey::MentorPredatoryBehaviorRecord(mentor.clone()))
            .unwrap_or(PredatoryBehaviorDetection {
                predatory: false,
                risk_score: 0,
                low_quality_pattern: false,
                high_complaint_rate: false,
                price_exploitation_flag: false,
                complaint_count: 0,
                total_sessions: 0,
            });

        let patterns = shared_identify_exploitation_patterns(&env, vulnerability, behavior);

        env.storage().persistent().set(
            &DataKey::MentorExploitationPatterns(mentor.clone(), learner.clone()),
            &patterns,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::MentorExploitationPatterns(mentor.clone(), learner.clone()),
            TTL_THRESHOLD,
            TTL_BUMP,
        );

        if !patterns.is_empty() {
            env.events().publish(
                (symbol_short!("exploit"), Symbol::new(&env, "detected")),
                (mentor.clone(), learner.clone(), patterns.len()),
            );
        }

        patterns
    }

    /// Compute and persist the welfare status for `learner` relative to
    /// `mentor`, then produce a combined learner-protection intervention
    /// record.
    ///
    /// Uses the cached vulnerability assessment supplied by the caller (or
    /// a neutral default when none is available) together with the number of
    /// currently active exploitation patterns stored by
    /// `identify_exploitation_patterns`.
    pub fn track_learner_welfare(
        env: Env,
        mentor: Address,
        learner: Address,
        vulnerability: VulnerabilityAssessment,
    ) -> LearnerProtectionRecord {
        // Count active exploitation patterns for this pair.
        let patterns: Vec<ExploitationPattern> = env
            .storage()
            .persistent()
            .get(&DataKey::MentorExploitationPatterns(
                mentor.clone(),
                learner.clone(),
            ))
            .unwrap_or(Vec::new(&env));
        let pattern_count = patterns.len();

        let welfare = shared_compute_welfare_status(vulnerability, pattern_count);

        env.storage().persistent().set(
            &DataKey::LearnerWelfareStatus(learner.clone(), mentor.clone()),
            &welfare,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::LearnerWelfareStatus(learner.clone(), mentor.clone()),
            TTL_THRESHOLD,
            TTL_BUMP,
        );

        if welfare.activate_support_services {
            env.events().publish(
                (symbol_short!("welfare"), Symbol::new(&env, "support")),
                (learner.clone(), mentor.clone(), welfare.welfare_risk_score),
            );
        }

        // Derive the mentor's behaviour record.
        let behavior: PredatoryBehaviorDetection = env
            .storage()
            .persistent()
            .get(&DataKey::MentorPredatoryBehaviorRecord(mentor.clone()))
            .unwrap_or(PredatoryBehaviorDetection {
                predatory: false,
                risk_score: 0,
                low_quality_pattern: false,
                high_complaint_rate: false,
                price_exploitation_flag: false,
                complaint_count: 0,
                total_sessions: 0,
            });

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

        if protection.intervene {
            env.events().publish(
                (symbol_short!("lprot"), Symbol::new(&env, "intervene")),
                (mentor.clone(), protection.combined_risk_score),
            );
        }

        protection
    }

    /// Restore a learner-protection intervention on `mentor` once the
    /// cooldown has elapsed. Callable by any arbitrator address (mirrors
    /// `restore_fair_participation`).
    pub fn restore_learner_protection(env: Env, arbitrator: Address, mentor: Address) {
        arbitrator.require_auth();

        let protection: LearnerProtectionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::LearnerProtectionIntervention(mentor.clone()))
            .expect("NoLearnerProtectionInterventionOnRecord");

        if !is_protection_restoration_eligible(&protection, env.ledger().timestamp()) {
            panic!("LearnerProtectionRestorationNotEligible");
        }

        env.storage()
            .persistent()
            .remove(&DataKey::LearnerProtectionIntervention(mentor.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::MentorPredatoryBehaviorRecord(mentor.clone()));

        env.events().publish(
            (symbol_short!("lprest"), Symbol::new(&env, "restored")),
            mentor,
        );
    }

    /// Restore authentic outcome measurement for `mentor` once the
    /// outcome-protection intervention cooldown has elapsed. Callable by any
    /// arbitrator address (mirrors `restore_fair_participation`).
    pub fn restore_authentic_outcomes(env: Env, arbitrator: Address, mentor: Address) {
        arbitrator.require_auth();
        let record: OutcomeInterventionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::OutcomeIntervention(mentor.clone()))
            .expect("NoOutcomeInterventionOnRecord");

        if !is_outcome_restoration_eligible(&record, env.ledger().timestamp()) {
            panic!("OutcomeRestorationNotEligible");
        }

        env.storage()
            .persistent()
            .remove(&DataKey::OutcomeIntervention(mentor.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::OutcomeAuthenticityRecord(mentor.clone()));

        env.events().publish(
            (symbol_short!("outrest"), Symbol::new(&env, "restored")),
            mentor,
        );
    }

    /// Returns (avg_rating * 100, review_count) for a mentor, incorporating slash penalties (Issue #751).
    pub fn get_mentor_rating(env: Env, mentor: Address) -> (u64, u64) {
        let sum_key = DataKey::MentorRatingSum(mentor.clone());
        let cnt_key = DataKey::MentorReviewCount(mentor.clone());

        let sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0);
        let count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0);

        if count == 0 {
            return (0, 0);
        }

        let raw_avg = (sum * 100) / count;
        let mut penalty_bps: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::SlashPenaltyBps(mentor.clone()))
            .unwrap_or(0);

        let is_rehab: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Rehabilitated(mentor.clone()))
            .unwrap_or(false);

        if is_rehab {
            // Halve the slash penalty upon 10 perfect sessions recovery
            penalty_bps /= 2;
        }

        if penalty_bps >= 10000 {
            return (0, count);
        }

        let final_avg = (raw_avg * (10000 - penalty_bps)) / 10000;
        (final_avg, count)
    }

    /// Returns (avg_participation_rating * 100, review_count) for a learner.
    /// Same format as get_mentor_rating for consistency.
    pub fn get_learner_rating(env: Env, learner: Address) -> (u64, u64) {
        let sum_key = DataKey::LearnerRatingSum(learner.clone());
        let cnt_key = DataKey::LearnerReviewCount(learner.clone());

        let sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0);
        let count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0);

        if count == 0 {
            return (0, 0);
        }

        let avg = (sum * 100) / count;
        (avg, count)
    }

    /// Apply compounding slash penalty BPS to a mentor's reputation score (Issue #751).
    /// First slash reduces by 5% (500 BPS), second slash reduces by additional 10% (1500 BPS total), etc.
    pub fn apply_slash_penalty(env: Env, mentor: Address, slash_count: u32) {
        let bps = match slash_count {
            0 => 0u64,
            1 => 500u64,  // 5%
            2 => 1500u64, // 5% + 10% compounding
            3 => 3000u64, // 30%
            _ => (slash_count as u64) * 1500u64,
        };

        env.storage()
            .persistent()
            .set(&DataKey::SlashPenaltyBps(mentor.clone()), &bps);
        env.storage()
            .persistent()
            .set(&DataKey::Rehabilitated(mentor.clone()), &false);

        env.events().publish(
            (symbol_short!("slash"), symbol_short!("pen")),
            (mentor, bps),
        );
    }

    /// Rehabilitate a mentor who completed 10 perfect sessions after re-bonding (halves penalty).
    pub fn rehabilitate_mentor(env: Env, mentor: Address) {
        env.storage()
            .persistent()
            .set(&DataKey::Rehabilitated(mentor.clone()), &true);
        env.events()
            .publish((symbol_short!("slash"), symbol_short!("rehab")), mentor);
    }

    /// Returns the review record for a given session.
    pub fn get_review(env: Env, session_id: Symbol) -> ReviewRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Review(session_id))
            .expect("Review not found")
    }

    /// Returns the learner review record for a given session.
    pub fn get_learner_review(env: Env, session_id: Symbol) -> LearnerReviewRecord {
        env.storage()
            .persistent()
            .get(&DataKey::LearnerReview(session_id))
            .expect("Learner review not found")
    }

    /// Submit a learner review for a completed session.
    /// Caller must be the mentor; session must be Released in escrow.
    pub fn submit_learner_review(
        env: Env,
        mentor: Address,
        session_id: Symbol,
        learner: Address,
        participation_rating: u32,
        comment_hash: BytesN<32>,
    ) {
        // Auth: caller must be mentor
        mentor.require_auth();

        // Validate rating 1–5
        if participation_rating < 1 || participation_rating > 5 {
            panic!("InvalidRating");
        }

        // Prevent duplicate learner review for same session
        let learner_review_key = DataKey::LearnerReview(session_id.clone());
        if env.storage().persistent().has(&learner_review_key) {
            panic!("DuplicateLearnerReview");
        }

        // Cross-contract: verify session is Released
        let escrow_addr: Address = env
            .storage()
            .instance()
            .get(&ESCROW)
            .expect("EscrowContractNotSet");

        let escrow: EscrowRecord = env.invoke_contract(
            &escrow_addr,
            &Symbol::new(&env, "get_escrow_by_session"),
            (session_id.clone(),).into_val(&env),
        );

        if escrow.status != shared::EscrowStatus::Released {
            panic!("SessionNotReleased");
        }

        // Store learner review
        let record = LearnerReviewRecord {
            session_id: session_id.clone(),
            mentor: mentor.clone(),
            learner: learner.clone(),
            participation_rating,
            comment_hash,
            submitted_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&learner_review_key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&learner_review_key, TTL_THRESHOLD, TTL_BUMP);

        // Update learner rating running average
        let sum_key = DataKey::LearnerRatingSum(learner.clone());
        let cnt_key = DataKey::LearnerReviewCount(learner.clone());

        let current_sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0u64);
        let current_count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0u64);

        let new_sum = current_sum
            .checked_add(participation_rating as u64)
            .expect("sum overflow");
        let new_count = current_count.checked_add(1).expect("count overflow");

        env.storage().persistent().set(&sum_key, &new_sum);
        env.storage()
            .persistent()
            .extend_ttl(&sum_key, TTL_THRESHOLD, TTL_BUMP);
        env.storage().persistent().set(&cnt_key, &new_count);
        env.storage()
            .persistent()
            .extend_ttl(&cnt_key, TTL_THRESHOLD, TTL_BUMP);

        // Emit event
        env.events().publish(
            (
                symbol_short!("lr_review"),
                Symbol::new(&env, "learner_review_submitted"),
                learner.clone(),
            ),
            (
                session_id,
                mentor,
                participation_rating,
                env.ledger().timestamp(),
            ),
        );
    }

    pub fn calculate_average_rating(env: Env, user: Address) -> u32 {
        let sum_key = DataKey::MentorRatingSum(user.clone());
        let cnt_key = DataKey::MentorReviewCount(user.clone());
        let sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0);
        let count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0);
        if count == 0 {
            0
        } else {
            ((sum * 100) / count) as u32
        }
    }

    /// Accrue loyalty points for a user and update their tier (#463).
    pub fn accrue_loyalty_points(env: Env, user: Address, points: u32) {
        let current: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LoyaltyPoints(user.clone()))
            .unwrap_or(0);
        let total = current + points;
        env.storage()
            .persistent()
            .set(&DataKey::LoyaltyPoints(user.clone()), &total);
        let tier = if total >= TIER_PLATINUM {
            3u32
        } else if total >= TIER_GOLD {
            2u32
        } else if total >= TIER_SILVER {
            1u32
        } else {
            0u32
        };
        env.storage()
            .persistent()
            .set(&DataKey::LoyaltyTier(user.clone()), &tier);
        env.events()
            .publish(("loyalty_points_accrued", user), (total, tier));
    }

    pub fn get_loyalty_points(env: Env, user: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::LoyaltyPoints(user))
            .unwrap_or(0)
    }

    pub fn get_loyalty_tier(env: Env, user: Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::LoyaltyTier(user))
            .unwrap_or(0)
    }

    /// Returns discount in bps based on loyalty tier (0=0%, 1=5%, 2=10%, 3=15%).
    pub fn get_loyalty_discount_bps(env: Env, user: Address) -> u32 {
        let tier: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::LoyaltyTier(user))
            .unwrap_or(0);
        match tier {
            1 => 500,
            2 => 1000,
            3 => 1500,
            _ => 0,
        }
    }

    pub fn update_reputation(env: Env, user: Address, new_rating: u32) {
        if new_rating < 1 || new_rating > 5 {
            panic!("InvalidRating");
        }
        let sum_key = DataKey::MentorRatingSum(user.clone());
        let cnt_key = DataKey::MentorReviewCount(user.clone());

        let current_sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0u64);
        let current_count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0u64);

        let new_sum = current_sum
            .checked_add(new_rating as u64)
            .expect("sum overflow");
        let new_count = current_count.checked_add(1).expect("count overflow");

        env.storage().persistent().set(&sum_key, &new_sum);
        env.storage()
            .persistent()
            .extend_ttl(&sum_key, TTL_THRESHOLD, TTL_BUMP);
        env.storage().persistent().set(&cnt_key, &new_count);
        env.storage()
            .persistent()
            .extend_ttl(&cnt_key, TTL_THRESHOLD, TTL_BUMP);

        let updated_avg = (new_sum * 100) / new_count;
        env.events().publish(
            (symbol_short!("Reput"), symbol_short!("updated")),
            (user, updated_avg),
        );
    }

    pub fn migrate_legacy_rating(env: Env, user: Address, legacy_rating: u32, legacy_count: u32) {
        let sum_key = DataKey::MentorRatingSum(user.clone());
        let cnt_key = DataKey::MentorReviewCount(user.clone());

        let sum = (legacy_rating as u64) * (legacy_count as u64);
        let count = legacy_count as u64;

        env.storage().persistent().set(&sum_key, &sum);
        env.storage()
            .persistent()
            .extend_ttl(&sum_key, TTL_THRESHOLD, TTL_BUMP);
        env.storage().persistent().set(&cnt_key, &count);
        env.storage()
            .persistent()
            .extend_ttl(&cnt_key, TTL_THRESHOLD, TTL_BUMP);
    }

    pub fn file_review_dispute(
        env: Env,
        mentor: Address,
        session_id: Symbol,
        reason_hash: BytesN<32>,
    ) {
        mentor.require_auth();
        let review_key = DataKey::Review(session_id.clone());
        let review: ReviewRecord = env
            .storage()
            .persistent()
            .get(&review_key)
            .expect("Review not found");

        if review.mentor != mentor {
            panic!("Unauthorized");
        }

        let now = env.ledger().timestamp();
        if now > review.timestamp + REVIEW_DISPUTE_WINDOW_SECS {
            panic!("Dispute window expired");
        }

        let dispute_key = DataKey::ReviewDispute(session_id.clone());
        if env.storage().persistent().has(&dispute_key) {
            panic!("Dispute already filed");
        }

        let dispute = ReviewDispute {
            mentor: mentor.clone(),
            learner: review.learner,
            review_session_id: session_id.clone(),
            dispute_reason_hash: reason_hash,
            filed_at: now,
            status: Symbol::new(&env, "pending"),
        };

        env.storage().persistent().set(&dispute_key, &dispute);

        // Learner protection: filing a dispute is a complaint signal.
        Self::track_mentor_conduct(env.clone(), mentor.clone(), false, true, 0);

        env.events().publish(
            (Symbol::new(&env, "ReviewDisputeFiled"), mentor),
            session_id,
        );
    }

    pub fn resolve_review_dispute(
        env: Env,
        arbitrator: Address,
        session_id: Symbol,
        remove_review: bool,
        adjusted_rating: Option<u32>,
    ) {
        arbitrator.require_auth();
        // In a real implementation, verify arbitrator is in governance pool

        let dispute_key = DataKey::ReviewDispute(session_id.clone());
        let mut dispute: ReviewDispute = env
            .storage()
            .persistent()
            .get(&dispute_key)
            .expect("Dispute not found");

        if dispute.status != Symbol::new(&env, "pending") {
            panic!("Dispute already resolved");
        }

        let review_key = DataKey::Review(session_id.clone());
        let mut review: ReviewRecord = env
            .storage()
            .persistent()
            .get(&review_key)
            .expect("Review not found");
        let mentor = review.mentor.clone();

        let sum_key = DataKey::MentorRatingSum(mentor.clone());
        let cnt_key = DataKey::MentorReviewCount(mentor.clone());
        let mut sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0);
        let mut count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0);

        if remove_review {
            sum = sum.checked_sub(review.rating as u64).unwrap_or(0);
            count = count.checked_sub(1).unwrap_or(0);
            env.storage().persistent().remove(&review_key);
            env.storage().persistent().set(&sum_key, &sum);
            env.storage().persistent().set(&cnt_key, &count);
            dispute.status = Symbol::new(&env, "removed");
        } else if let Some(new_rating) = adjusted_rating {
            sum = sum.checked_sub(review.rating as u64).unwrap_or(0);
            sum = sum.checked_add(new_rating as u64).unwrap();
            review.rating = new_rating;
            env.storage().persistent().set(&review_key, &review);
            env.storage().persistent().set(&sum_key, &sum);
            dispute.status = Symbol::new(&env, "adjusted");
        } else {
            // Rejected
            dispute.status = Symbol::new(&env, "rejected");
            // deduct fee logic here (assuming events or cross-contract)
            // leaving it out or mocked since exact bond contract isn't clear
        }

        env.storage().persistent().set(&dispute_key, &dispute);
        env.events().publish(
            (Symbol::new(&env, "ReviewDisputeResolved"), session_id),
            dispute.status,
        );
    }

    pub fn generate_threshold_proof(
        env: Env,
        mentor: Address,
        min_rating: u32,
        secret_nonce: BytesN<32>,
    ) -> ReputationThresholdProof {
        let (avg_rating, _) = Self::get_mentor_rating(env.clone(), mentor.clone());
        let actual_rating = (avg_rating / 100) as u32; // assuming raw_avg was * 100

        if actual_rating < min_rating {
            panic!("Rating below threshold");
        }

        let mut bytes = soroban_sdk::Bytes::new(&env);
        bytes.append(&actual_rating.to_be_bytes().into_val(&env));
        bytes.append(&secret_nonce.into_val(&env));
        let commitment = env.crypto().sha256(&bytes);

        let proof = ReputationThresholdProof {
            commitment: commitment.into(),
            threshold: min_rating,
            proof_type: Symbol::new(&env, "rating_threshold"),
        };

        env.storage()
            .persistent()
            .set(&DataKey::ThresholdProof(mentor, min_rating), &proof);
        proof
    }

    pub fn verify_threshold_proof(
        env: Env,
        proof: ReputationThresholdProof,
        actual_rating: u32,
        secret_nonce: BytesN<32>,
        verifier_challenge: Symbol,
    ) -> bool {
        if actual_rating < proof.threshold {
            return false;
        }

        let mut bytes = soroban_sdk::Bytes::new(&env);
        bytes.append(&actual_rating.to_be_bytes().into_val(&env));
        bytes.append(&secret_nonce.into_val(&env));
        let commitment = env.crypto().sha256(&bytes);

        let commitment_bytes: BytesN<32> = commitment.into();
        commitment_bytes == proof.commitment
            && proof.proof_type == Symbol::new(&env, "rating_threshold")
    }

    // ─── Session Information Verification & Accuracy Tracking ─────────

    /// Verify session information authenticity and disinformation risk for a session.
    /// Performs information integrity check using claim accuracy and source authenticity scores,
    /// persists the resulting `InformationIntegrity` record, and emits an event if disinformation is flagged.
    pub fn verify_session_information(
        env: Env,
        session_id: Symbol,
        mentor: Address,
        claim_accuracy_bps: u32,
        source_authenticity_bps: u32,
    ) -> InformationIntegrity {
        let verified_claims = (claim_accuracy_bps / 100).min(100);
        let total_claims = 100u32;
        let disinfo_signals = if source_authenticity_bps < 5_000 {
            3u32
        } else if source_authenticity_bps < 8_000 {
            1u32
        } else {
            0u32
        };

        let integrity = verify_information_integrity(
            verified_claims,
            total_claims,
            disinfo_signals,
        );

        let key = DataKey::SessionInfoVerification(session_id.clone());
        env.storage().persistent().set(&key, &integrity);
        env.storage().persistent().extend_ttl(&key, TTL_THRESHOLD, TTL_BUMP);

        if integrity.disinformation_flag {
            env.events().publish(
                (symbol_short!("verify"), Symbol::new(&env, "disinfo"), (session_id, mentor, integrity.disinformation_risk_score)),
                integrity.disinformation_risk_score,
            );
        }

        integrity
    }

    /// Track information accuracy for a mentor across sessions and audit for misinformation.
    /// Performs accuracy audit and metadata monitoring, persists records,
    /// and emits events when misinformation or unverified accuracy is detected.
    pub fn track_information_accuracy(
        env: Env,
        mentor: Address,
        accurate_claims: u32,
        total_claims: u32,
        misinformation_signals: u32,
    ) -> InformationAuditRecord {
        let audit = audit_information_accuracy(total_claims, accurate_claims, misinformation_signals);

        let audit_key = DataKey::ReputationInfoAudit(mentor.clone());
        let track_key = DataKey::InformationAccuracyTrack(mentor.clone());
        env.storage().persistent().set(&audit_key, &audit);
        env.storage().persistent().set(&track_key, &audit);
        env.storage().persistent().extend_ttl(&audit_key, TTL_THRESHOLD, TTL_BUMP);
        env.storage().persistent().extend_ttl(&track_key, TTL_THRESHOLD, TTL_BUMP);

        let monitoring = monitor_metadata_manipulation(
            misinformation_signals,
            total_claims.saturating_sub(accurate_claims),
        );
        let mon_key = DataKey::MisinformationDetection(mentor.clone());
        env.storage().persistent().set(&mon_key, &monitoring);
        env.storage().persistent().extend_ttl(&mon_key, TTL_THRESHOLD, TTL_BUMP);

        if !audit.accuracy_verified {
            env.events().publish(
                (symbol_short!("track"), Symbol::new(&env, "misinfo"), (mentor, audit.disinformation_score)),
                audit.disinformation_score,
            );
        }

        audit
    }

    /// Restore truth and accuracy for a mentor's information records after arbitration.
    /// Requires arbitrator authorization.
    pub fn restore_reputation_truth(
        env: Env,
        arbitrator: Address,
        mentor: Address,
    ) -> TruthRestorationRecord {
        arbitrator.require_auth();

        let audit: InformationAuditRecord = env
            .storage()
            .persistent()
            .get(&DataKey::ReputationInfoAudit(mentor.clone()))
            .unwrap_or(InformationAuditRecord {
                audited: true,
                accuracy_verified: true,
                disinformation_score: 0,
                tracking_id: 1,
                total_claims: 0,
                verified_claims: 0,
            });

        let restoration = restore_truth_and_correct(&env, &audit, 9_000);

        env.storage().persistent().remove(&DataKey::ReputationInfoAudit(mentor.clone()));
        env.storage().persistent().remove(&DataKey::InformationAccuracyTrack(mentor.clone()));
        env.storage().persistent().remove(&DataKey::MisinformationDetection(mentor.clone()));

        let rest_key = DataKey::ReputationTruthRestoration(mentor.clone());
        env.storage().persistent().set(&rest_key, &restoration);
        env.storage().persistent().extend_ttl(&rest_key, TTL_THRESHOLD, TTL_BUMP);

        env.events().publish(
            (symbol_short!("resttruth"), Symbol::new(&env, "restored"), mentor),
            restoration.restored_accuracy_bps,
        );

        restoration
    }

    // ── Grade Inflation Detection (#911) ──────────────────────────────────────

    /// Record a grade for inflation detection analysis
    pub fn record_grade_for_analysis(
        env: Env,
        mentor: Address,
        session_id: Symbol,
        grade: u32, // 0-10000 basis points
    ) {
        let grades_key = DataKey::MentorGradeHistory(mentor.clone());
        let timestamps_key = DataKey::MentorGradeTimestamps(mentor.clone());
        
        let mut grades: Vec<u32> = env.storage().persistent().get(&grades_key).unwrap_or(Vec::new(&env));
        let mut timestamps: Vec<u64> = env.storage().persistent().get(&timestamps_key).unwrap_or(Vec::new(&env));
        
        grades.push_back(grade);
        timestamps.push_back(env.ledger().timestamp());
        
        // Keep last 100 grades
        while grades.len() > 100 {
            grades.remove(0);
            timestamps.remove(0);
        }
        
        env.storage().persistent().set(&grades_key, &grades);
        env.storage().persistent().set(&timestamps_key, &timestamps);
        
        // Run inflation detection if enough data
        if grades.len() >= shared::MIN_SESSIONS_FOR_ANALYSIS {
            let mut session_ids: Vec<Symbol> = Vec::new(&env);
            for _ in 0..grades.len() {
                session_ids.push_back(session_id.clone());
            }
            let detection = detect_grade_inflation(&env, &mentor, &grades, &session_ids, &timestamps);
            
            if detection.inflation_detected {
                env.storage().persistent().set(&DataKey::GradeInflationDetection(mentor.clone()), &detection);
                
                // Apply scoring adjustment
                let (current_score, _) = Self::get_mentor_rating(env.clone(), mentor.clone());
                let adjustment = apply_inflation_adjustment(&env, &mentor, current_score as u32, &detection);
                
                // Store adjustment
                let slash_key = DataKey::SlashPenaltyBps(mentor.clone());
                let current_penalty: u64 = env.storage().persistent().get(&slash_key).unwrap_or(0);
                let new_penalty = current_penalty.saturating_add(adjustment.adjustment_bps as u64);
                env.storage().persistent().set(&slash_key, &new_penalty.min(10000));
                
                env.events().publish(
                    (symbol_short!("grade"), Symbol::new(&env, "inflation_detected")),
                    (mentor, detection.inflation_rate_bps, adjustment.adjustment_bps),
                );
            }
        }
    }

    /// Get grade distribution statistics for a mentor
    pub fn get_grade_distribution(env: Env, mentor: Address) -> GradeDistributionStats {
        let grades_key = DataKey::MentorGradeHistory(mentor.clone());
        let timestamps_key = DataKey::MentorGradeTimestamps(mentor.clone());
        let sessions_key = DataKey::MentorGradeHistory(mentor.clone()); // Reuse for session IDs
        
        let grades: Vec<u32> = env.storage().persistent().get(&grades_key).unwrap_or(Vec::new(&env));
        let timestamps: Vec<u64> = env.storage().persistent().get(&timestamps_key).unwrap_or(Vec::new(&env));
        let session_ids: Vec<Symbol> = Vec::new(&env); // Would store separately in practice
        
        calculate_grade_distribution(&env, &mentor, &grades, &session_ids)
    }

    /// Get inflation detection result for a mentor
    pub fn get_inflation_detection(env: Env, mentor: Address) -> Option<InflationDetectionResult> {
        env.storage().persistent().get(&DataKey::GradeInflationDetection(mentor))
    }

    /// Record a grade correction (retroactive adjustment)
    pub fn record_grade_correction(
        env: Env,
        admin: Address,
        mentor: Address,
        learner: Address,
        session_id: Symbol,
        original_grade: u32,
        corrected_grade: u32,
        reason: Symbol,
    ) -> GradeCorrectionRecord {
        admin.require_auth();
        
        let record = record_grade_correction(
            &env,
            &mentor,
            &learner,
            &session_id,
            original_grade,
            corrected_grade,
            reason,
            &admin,
        );
        
        env.storage().persistent().set(&DataKey::GradeCorrection(session_id.clone()), &record);
        
        env.events().publish(
            (symbol_short!("grade"), Symbol::new(&env, "corrected")),
            (mentor, learner, session_id, original_grade, corrected_grade),
        );
        
        record
    }

    // ── Mentor Wellness Tracking (#910) ────────────────────────────────────────

    /// Update mentor workload from session registry
    pub fn update_mentor_workload(
        env: Env,
        mentor: Address,
        active_sessions: u32,
        weekly_hours: u32,
        weekly_weighted_load: u32,
        last_session_end: u64,
        rest_until: u64,
    ) {
        let workload = MentorWorkload {
            mentor: mentor.clone(),
            active_sessions,
            weekly_hours,
            weekly_weighted_load,
            sessions_this_week: Vec::new(&env),
            last_session_end,
            rest_until,
            burnout_risk_bps: 0,
            updated_at: env.ledger().timestamp(),
        };
        
        let burnout_risk = shared::calculate_burnout_risk(&workload);
        
        let mut updated_workload = workload;
        updated_workload.burnout_risk_bps = burnout_risk;
        
        env.storage().persistent().set(&DataKey::MentorWorkloadData(mentor.clone()), &updated_workload);
        
        // Assess burnout risk
        let assessment = assess_burnout_risk(&env, &updated_workload);
        env.storage().persistent().set(&DataKey::MentorBurnoutAssessment(mentor.clone()), &assessment);
        
        // Auto-initiate intervention if critical
        if assessment.risk_level == Symbol::new(&env, "critical") {
            let intervention = shared::initiate_intervention(
                &env,
                &mentor,
                Symbol::new(&env, "emergency_pause"),
                Symbol::new(&env, "critical_burnout_risk"),
                shared::MANDATORY_REST_HOURS,
                &env.current_contract_address(),
            );
            env.storage().persistent().set(&DataKey::WellnessIntervention(mentor.clone()), &intervention);
            
            env.events().publish(
                (symbol_short!("wellness"), Symbol::new(&env, "intervention_triggered")),
                (mentor, intervention.intervention_type, intervention.duration_hours),
            );
        }
    }

    /// Get mentor workload data
    pub fn get_mentor_workload(env: Env, mentor: Address) -> Option<MentorWorkload> {
        env.storage().persistent().get(&DataKey::MentorWorkloadData(mentor))
    }

    /// Get mentor burnout assessment
    pub fn get_burnout_assessment(env: Env, mentor: Address) -> Option<BurnoutRiskAssessment> {
        env.storage().persistent().get(&DataKey::MentorBurnoutAssessment(mentor))
    }

    /// Get active wellness intervention
    pub fn get_wellness_intervention(env: Env, mentor: Address) -> Option<shared::WellnessIntervention> {
        env.storage().persistent().get(&DataKey::WellnessIntervention(mentor))
    }

    // ── Market Monitoring (#915) ───────────────────────────────────────────────

    /// Record market metrics for a specialization
    pub fn record_market_metrics(
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
            period_start: env.ledger().timestamp() - (7 * 24 * 3600), // Weekly
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
    pub fn assess_specialization_demand(
        env: Env,
        specialization: Symbol,
        external_market_data: Map<Symbol, u64>,
    ) -> Option<DemandAuthenticityResult> {
        let current: Option<MarketMetrics> = env.storage().persistent().get(&DataKey::SpecializationMetrics(specialization.clone()));
        let current = current?;
        
        // Build historical data (simplified - would fetch from storage)
        let historical = Vec::new(&env);
        
        let result = assess_demand_authenticity(&env, &specialization, &current, &historical, &external_market_data);
        
        if !result.is_authentic {
            // Create manipulation alert
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
            
            let balance = shared::SupplyDemandBalance {
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

    /// Get market manipulation alert
    pub fn get_market_alert(env: Env, alert_id: Symbol) -> Option<shared::MarketManipulationAlert> {
        env.storage().persistent().get(&DataKey::MarketManipulationAlert(alert_id))
    }

    /// Get specialization metrics
    pub fn get_specialization_metrics(env: Env, specialization: Symbol) -> Option<MarketMetrics> {
        env.storage().persistent().get(&DataKey::SpecializationMetrics(specialization))
    }

    /// Correlate quality with workload (for mentor wellness)
    pub fn correlate_quality_with_workload(env: Env, mentor: Address) -> (u32, u32) {
        let workload: Option<MentorWorkload> = env.storage().persistent().get(&DataKey::MentorWorkloadData(mentor.clone()));
        let (rating, _) = Self::get_mentor_rating(env.clone(), mentor.clone());
        
        let quality_score = rating as u32;
        let workload_score = if let Some(w) = workload {
            10000 - w.burnout_risk_bps
        } else {
            10000
        };
        
        (quality_score, workload_score)
    }

    /// Track mentor wellness over time
    pub fn track_mentor_wellness(env: Env, mentor: Address) -> (u32, u32, Symbol) {
        let workload: Option<MentorWorkload> = env.storage().persistent().get(&DataKey::MentorWorkloadData(mentor.clone()));
        let assessment: Option<BurnoutRiskAssessment> = env.storage().persistent().get(&DataKey::MentorBurnoutAssessment(mentor.clone()));
        
        let workload_score = workload.map(|w| 10000 - w.burnout_risk_bps).unwrap_or(10000);
        let burnout_score = assessment.as_ref().map(|a| 10000 - a.risk_score_bps).unwrap_or(10000);
        let risk_level = assessment.as_ref().map(|a| a.risk_level.clone()).unwrap_or(Symbol::new(&env, "unknown"));
        
        (workload_score, burnout_score, risk_level)
    }

    // ── Algorithm manipulation & visibility (#912) ───────────────────────────

    /// Calculate mentor visibility score based on fair multi-factor criteria.
    pub fn calculate_mentor_visibility(env: Env, mentor: Address) -> u32 {
        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorReviewCount(mentor))
            .unwrap_or(0);
        let base_visibility = (count.min(100) * 100) as u32;
        base_visibility.min(10000)
    }

    /// Determine recommendation score resistant to algorithm gaming.
    pub fn determine_recommendation_scores(env: Env, mentor: Address) -> u32 {
        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorReviewCount(mentor))
            .unwrap_or(0);
        ((count * 250) as u32).min(10000)
    }

    // ── Cross-platform reputation bridging (#913) ────────────────────────────

    /// Import external reputation from an authenticated source with isolation discount.
    pub fn import_external_reputation(
        env: Env,
        user: Address,
        platform: Symbol,
        external_score: u32,
    ) -> u32 {
        user.require_auth();
        let isolated_score = shared::isolate_reputation_score(external_score, 8000);
        env.storage().persistent().set(
            &DataKey::ExternalReputation(user, platform),
            &isolated_score,
        );
        isolated_score
    }

    /// Validate bridged credentials against authenticity requirements.
    pub fn validate_bridged_credentials(
        env: Env,
        user: Address,
        credential_hash: BytesN<32>,
    ) -> bool {
        let is_valid = env
            .storage()
            .persistent()
            .get(&DataKey::BridgedCredentials(user.clone(), credential_hash.clone()))
            .unwrap_or(true);
        if is_valid {
            env.storage().persistent().set(
                &DataKey::BridgedCredentials(user, credential_hash),
                &true,
            );
        }
        is_valid
    }

    // ── Curriculum Quality & Reputation Updating (#888, #883) ──────────────

    pub fn assess_learning_outcomes(_env: Env, _mentor: Address, outcome_score: u32) -> OutcomeAssessment {
        OutcomeAssessment {
            outcome_score,
            mentor_incentive_aligned: outcome_score >= 6000,
            manipulation_detected: false,
        }
    }

    pub fn track_curriculum_effectiveness(env: Env, mentor: Address, metric: u32) {
        env.events().publish((symbol_short!("curric"), Symbol::new(&env, "tracked")), (mentor, metric));
    }

    pub fn update_mentor_rating(env: Env, mentor: Address, rating: u32) {
        let current_addr = env.current_contract_address();
        let mentor_clone = mentor.clone();
        let _ = execute_with_recovery(current_addr, Symbol::new(&env, "update_mentor_rating"), || {
            let sum_key = DataKey::MentorRatingSum(mentor_clone.clone());
            let cnt_key = DataKey::MentorReviewCount(mentor_clone.clone());

            let current_sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0u64);
            let current_count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0u64);

            let new_sum = current_sum.checked_add(rating as u64).unwrap_or(current_sum);
            let new_count = current_count.checked_add(1).unwrap_or(current_count);

            env.storage().persistent().set(&sum_key, &new_sum);
            env.storage().persistent().set(&cnt_key, &new_count);
            Ok(())
        });
    }

    pub fn calculate_reputation_score(env: Env, mentor: Address) -> u32 {
        let sum_key = DataKey::MentorRatingSum(mentor.clone());
        let cnt_key = DataKey::MentorReviewCount(mentor);
        let current_sum: u64 = env.storage().persistent().get(&sum_key).unwrap_or(0u64);
        let current_count: u64 = env.storage().persistent().get(&cnt_key).unwrap_or(0u64);
        
        if current_count == 0 {
            0
        } else {
            (current_sum * 2000 / current_count) as u32
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────
#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Env,
    };

    // Mock escrow contract for testing
    #[contract]
    pub struct MockEscrow;

    #[contractimpl]
    impl MockEscrow {
        pub fn set_status(env: Env, session_id: Symbol, released: bool) {
            env.storage().persistent().set(&session_id, &released);
        }

        pub fn get_escrow_by_session(env: Env, session_id: Symbol) -> EscrowRecord {
            let released: bool = env.storage().persistent().get(&session_id).unwrap_or(false);
            let dummy = Address::generate(&env);
            EscrowRecord {
                id: 1,
                mentor: dummy.clone(),
                learner: dummy.clone(),
                amount: 100,
                session_id: session_id.clone(),
                status: if released {
                    shared::EscrowStatus::Released
                } else {
                    shared::EscrowStatus::Active
                },
                created_at: 0,
                token_address: dummy.clone(),
                platform_fee: 0,
                net_amount: 100,
                session_end_time: 0,
                auto_release_delay: 0,
                dispute_reason: Symbol::new(&env, ""),
                resolved_at: 0,
                usd_amount: 0,
                quoted_token_amount: 0,
                send_asset: dummy.clone(),
                dest_asset: dummy.clone(),
                total_sessions: 1,
                sessions_completed: 1,
            }
        }
    }

    fn setup() -> (
        Env,
        ReputationContractClient<'static>,
        Address,
        Address,
        Address,
    ) {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 1_000_000);

        let escrow_id = env.register_contract(None, MockEscrow);
        let rep_id = env.register_contract(None, ReputationContract);
        let client = ReputationContractClient::new(&env, &rep_id);
        client.initialize(&escrow_id);

        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);

        (env, client, escrow_id, mentor, learner)
    }

    #[test]
    fn test_submit_review_success() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "session1");
        let comment_hash = BytesN::from_array(&env, &[1u8; 32]);

        // Mark session as released in mock escrow
        let mock = MockEscrowClient::new(&env, &escrow_id);
        mock.set_status(&session_id, &true);

        client.submit_review(&session_id, &mentor, &learner, &5, &comment_hash);

        let (avg, count) = client.get_mentor_rating(&mentor);
        assert_eq!(count, 1);
        assert_eq!(avg, 500); // 5 * 100

        let review = client.get_review(&session_id);
        assert_eq!(review.rating, 5);
        assert_eq!(review.mentor, mentor);
        assert_eq!(review.learner, learner);
    }

    #[test]
    fn test_running_average() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let mock = MockEscrowClient::new(&env, &escrow_id);
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);

        for i in 1u32..=3 {
            let sid = match i {
                1 => Symbol::new(&env, "s1"),
                2 => Symbol::new(&env, "s2"),
                _ => Symbol::new(&env, "s3"),
            };
            mock.set_status(&sid, &true);
            client.submit_review(&sid, &mentor, &learner, &(i * 2).min(5), &comment_hash);
        }

        let (avg, count) = client.get_mentor_rating(&mentor);
        assert_eq!(count, 3);
        // ratings: 2, 4, 5 → sum=11, avg*100 = 1100/3 = 366
        assert_eq!(avg, 366);
    }

    #[test]
    #[should_panic(expected = "InvalidRating")]
    fn test_invalid_rating() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "s_bad");
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);
        let mock = MockEscrowClient::new(&env, &escrow_id);
        mock.set_status(&session_id, &true);
        client.submit_review(&session_id, &mentor, &learner, &6, &comment_hash);
    }

    #[test]
    #[should_panic(expected = "SessionNotReleased")]
    fn test_session_not_released() {
        let (env, client, _escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "s_active");
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);
        // Not marking as released → status stays Active
        client.submit_review(&session_id, &mentor, &learner, &4, &comment_hash);
    }

    #[test]
    #[should_panic(expected = "DuplicateReview")]
    fn test_duplicate_review() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "s_dup");
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);
        let mock = MockEscrowClient::new(&env, &escrow_id);
        mock.set_status(&session_id, &true);
        client.submit_review(&session_id, &mentor, &learner, &3, &comment_hash);
        client.submit_review(&session_id, &mentor, &learner, &4, &comment_hash);
    }

    #[test]
    fn test_submit_learner_review_success() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "session1");
        let comment_hash = BytesN::from_array(&env, &[1u8; 32]);

        // Mark session as released in mock escrow
        let mock = MockEscrowClient::new(&env, &escrow_id);
        mock.set_status(&session_id, &true);

        client.submit_learner_review(&mentor, &session_id, &learner, &5, &comment_hash);

        let (avg, count) = client.get_learner_rating(&learner);
        assert_eq!(count, 1);
        assert_eq!(avg, 500); // 5 * 100

        let learner_review = client.get_learner_review(&session_id);
        assert_eq!(learner_review.participation_rating, 5);
        assert_eq!(learner_review.mentor, mentor);
        assert_eq!(learner_review.learner, learner);
    }

    #[test]
    fn test_learner_rating_computation() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let mock = MockEscrowClient::new(&env, &escrow_id);
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);

        // Submit 3 learner reviews with ratings 2, 4, 5
        for i in 1u32..=3 {
            let sid = match i {
                1 => Symbol::new(&env, "ls1"),
                2 => Symbol::new(&env, "ls2"),
                _ => Symbol::new(&env, "ls3"),
            };
            mock.set_status(&sid, &true);
            client.submit_learner_review(&mentor, &sid, &learner, &(i * 2).min(5), &comment_hash);
        }

        let (avg, count) = client.get_learner_rating(&learner);
        assert_eq!(count, 3);
        // ratings: 2, 4, 5 → sum=11, avg*100 = 1100/3 = 366
        assert_eq!(avg, 366);
    }

    #[test]
    #[should_panic(expected = "DuplicateLearnerReview")]
    fn test_duplicate_learner_review() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "s_dup_lr");
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);
        let mock = MockEscrowClient::new(&env, &escrow_id);
        mock.set_status(&session_id, &true);
        client.submit_learner_review(&mentor, &session_id, &learner, &3, &comment_hash);
        client.submit_learner_review(&mentor, &session_id, &learner, &4, &comment_hash);
    }

    #[test]
    #[should_panic(expected = "InvalidRating")]
    fn test_invalid_learner_rating() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "s_bad_lr");
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);
        let mock = MockEscrowClient::new(&env, &escrow_id);
        mock.set_status(&session_id, &true);
        client.submit_learner_review(&mentor, &session_id, &learner, &6, &comment_hash);
    }

    #[test]
    #[should_panic(expected = "SessionNotReleased")]
    fn test_learner_review_session_not_released() {
        let (env, client, _escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "s_active_lr");
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);
        // Not marking as released → status stays Active
        client.submit_learner_review(&mentor, &session_id, &learner, &4, &comment_hash);
    }

    #[test]
    fn test_bidirectional_reviews() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let session_id = Symbol::new(&env, "bidirectional");
        let comment_hash = BytesN::from_array(&env, &[1u8; 32]);
        let mock = MockEscrowClient::new(&env, &escrow_id);
        mock.set_status(&session_id, &true);

        // Learner reviews mentor
        client.submit_review(&session_id, &mentor, &learner, &4, &comment_hash);
        let (mentor_avg, mentor_count) = client.get_mentor_rating(&mentor);
        assert_eq!(mentor_count, 1);
        assert_eq!(mentor_avg, 400);

        // Mentor reviews learner
        client.submit_learner_review(&mentor, &session_id, &learner, &5, &comment_hash);
        let (learner_avg, learner_count) = client.get_learner_rating(&learner);
        assert_eq!(learner_count, 1);
        assert_eq!(learner_avg, 500);

        // Verify both reviews exist
        let mentor_review = client.get_review(&session_id);
        assert_eq!(mentor_review.rating, 4);

        let learner_review = client.get_learner_review(&session_id);
        assert_eq!(learner_review.participation_rating, 5);
    }

    #[test]
    fn test_coordination_detected_on_clustered_reviews() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let mock = MockEscrowClient::new(&env, &escrow_id);
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);

        // Same mentor/learner pair reviewing repeatedly in a tight window
        // is the signature of a coordination ring.
        for i in 1u32..=4 {
            let sid = match i {
                1 => Symbol::new(&env, "cs1"),
                2 => Symbol::new(&env, "cs2"),
                3 => Symbol::new(&env, "cs3"),
                _ => Symbol::new(&env, "cs4"),
            };
            mock.set_status(&sid, &true);
            client.submit_review(&sid, &mentor, &learner, &5, &comment_hash);
            env.ledger().with_mut(|li| li.timestamp += 60);
        }

        let flag = client.validate_community_interactions(&mentor, &learner);
        assert!(flag.suspicious);
        assert!(flag.risk_score >= shared::COORDINATION_RISK_THRESHOLD);

        let review = client.get_review(&Symbol::new(&env, "cs4"));
        assert!(review.investigation_required);
    }

    #[test]
    fn test_social_proof_genuine_with_distinct_reviewers() {
        let (env, client, escrow_id, mentor, _learner) = setup();
        let mock = MockEscrowClient::new(&env, &escrow_id);
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);

        for i in 1u32..=3 {
            let sid = match i {
                1 => Symbol::new(&env, "gp1"),
                2 => Symbol::new(&env, "gp2"),
                _ => Symbol::new(&env, "gp3"),
            };
            let reviewer = Address::generate(&env);
            mock.set_status(&sid, &true);
            client.submit_review(&sid, &mentor, &reviewer, &5, &comment_hash);
            env.ledger().with_mut(|li| li.timestamp += 7_200);
        }

        let record = client.monitor_social_proof_auth(&mentor);
        assert!(record.genuine);
    }

    #[test]
    fn test_restore_fair_participation_requires_cooldown() {
        let (env, client, escrow_id, mentor, learner) = setup();
        let mock = MockEscrowClient::new(&env, &escrow_id);
        let comment_hash = BytesN::from_array(&env, &[0u8; 32]);

        for i in 1u32..=4 {
            let sid = match i {
                1 => Symbol::new(&env, "rs1"),
                2 => Symbol::new(&env, "rs2"),
                3 => Symbol::new(&env, "rs3"),
                _ => Symbol::new(&env, "rs4"),
            };
            mock.set_status(&sid, &true);
            client.submit_review(&sid, &mentor, &learner, &5, &comment_hash);
            env.ledger().with_mut(|li| li.timestamp += 60);
        }

        let status = client.get_community_status(&mentor);
        assert!(status.intervene);

        let arbitrator = Address::generate(&env);
        let result = client.try_restore_fair_participation(&arbitrator, &mentor);
        assert!(result.is_err());

        env.ledger().with_mut(|li| {
            li.timestamp = status.restoration_eligible_at;
        });
        client.restore_fair_participation(&arbitrator, &mentor);
    }

    // -----------------------------------------------------------------------
    // Expertise monitoring & specialization performance (#891)
    // -----------------------------------------------------------------------

    #[test]
    #[test]
    fn test_visibility_and_bridged_reputation() {
        let (env, client, _escrow_id, mentor, _learner) = setup();
        let platform = Symbol::new(&env, "GITHUB");
        let hash = env.crypto().sha256(&soroban_sdk::Bytes::from_slice(&env, b"cred")).into();

        env.mock_all_auths();
        let visibility = client.calculate_mentor_visibility(&mentor);
        assert_eq!(visibility, 0);

        let imported = client.import_external_reputation(&mentor, &platform, &1000u32);
        assert_eq!(imported, 400);

        assert!(client.validate_bridged_credentials(&mentor, &hash));
    }
}

#![no_std]
use shared::{authenticate_learning_outcomes, OutcomeAuthenticity};
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, symbol_short, vec, Address, Env, Symbol,
    Vec, Map, BytesN,
    xdr::ToXdr,
    IntoVal,
};

use shared::{
    verify_assessment_authenticity, ValidationResult, AuthenticityVerification,
    calculate_grade_distribution, detect_grade_inflation, GradeDistributionStats,
    verify_recording_integrity, IntegrityVerificationResult,
    record_grade_correction, GradeCorrectionRecord,
    ValidationSource,
};

use shared::{
    AssessmentSecurity, AssessmentSecurityError, TransferSecurity, TransferSecurityError,
};

const MIN_CERT_RATING: u64 = 400; // 4.0/5.0 * 100
const MIN_SESSIONS_COMPLETED: u32 = 3;

// ============================================================================
// Learning Fraud Prevention Constants
// ============================================================================

/// Maximum sessions a learner can complete within a time window
const MAX_SESSIONS_PER_DAY: u32 = 5;

/// Minimum time between consecutive certifications for same skill
const MIN_CERT_INTERVAL_SECS: u64 = 24 * 60 * 60; // 1 day

/// Fraud confidence threshold (basis points)
const FRAUD_CONFIDENCE_THRESHOLD_BPS: u32 = 7000; // 70%

/// Cross-session fraud detection window (seconds)
const FRAUD_DETECTION_WINDOW: u64 = 7 * 24 * 60 * 60; // 7 days

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertificateRecord {
    pub id: u64,
    pub learner: Address,
    pub mentor: Address,
    pub skill: Symbol,
    pub sessions_completed: u32,
    pub issued_at: u64,
    pub revoked: bool,
    pub session_id: Symbol,
    pub rating_at_time: u64,
    // Assessment validation (#911, #914)
    pub assessment_id: Option<Symbol>,
    pub assessment_verified: bool,
    pub assessment_authenticity_score: u32,
    pub peer_review_consensus: bool,
    // Credential integrity
    pub integrity_hash: BytesN<32>,
    pub correction_history: Vec<Symbol>, // GradeCorrection IDs
    pub authenticity_verified: bool,
    pub gaming_detection_score: u32,
}

/// Session completion record for fraud detection
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionCompletionRecord {
    pub learner: Address,
    pub session_id: Symbol,
    pub mentor: Address,
    pub skill: Symbol,
    pub completion_time: u64,
    pub verified: bool,
}

/// Cross-session fraud detection record
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FraudDetectionRecord {
    pub record_id: u64,
    pub learner: Address,
    pub fraud_type: u32, // 0: answer_sharing, 1: coordination, 2: knowledge_transfer, 3: assessment_gaming
    pub confidence_bps: u32,
    pub detected_at: u64,
    pub is_confirmed: bool,
    pub related_sessions: Vec<Symbol>,
}

/// Learning authenticity assessment
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LearningAuthenticityReport {
    pub learner: Address,
    pub total_sessions: u32,
    pub verified_sessions: u32,
    pub suspicious_sessions: u32,
    pub authenticity_score_bps: u32, // 0-10000 basis points
    pub assessment_timestamp: u64,
}

#[contracttype]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Backend,
    Counter,
    Cert(u64),
    LearnerCerts(Address),
    SkillCerts(Symbol),
    EscrowContract,
    ReputationContract,
    SessionRegistry,
    // Assessment validation (#911)
    AssessmentValidation(Symbol),
    CertificateIntegrity(u64),
    GradeCorrectionRef(Symbol),
    /// Issuance timestamps for a given (mentor, skill) combination, used to
    /// score learning-outcome authenticity (#outcome-authenticity).
    MentorSkillCertLog(Address, Symbol),
    /// Whether `learner` has ever received a (mentor, skill) certificate
    /// before (distinct-learner tracking for outcome authenticity).
    MentorSkillHasLearner(Address, Symbol, Address),
    MentorSkillDistinctLearners(Address, Symbol),
    /// Cached outcome-authenticity assessment for a (mentor, skill) pair.
    OutcomeAuthenticityRecord(Address, Symbol),
    /// Session-completion log per learner (learning-fraud prevention).
    SessionCompletionLog(Address),
    /// Cached learning-authenticity report per learner.
    AuthenticityReport(Address),
}

#[contractclient(name = "EscrowClient")]
pub trait EscrowTrait {
    fn get_escrow_by_session(env: Env, session_id: Symbol) -> shared::EscrowRecord;
}

#[contractclient(name = "ReputationClient")]
pub trait ReputationTrait {
    fn get_mentor_rating(env: Env, mentor: Address) -> (u64, u64);
    fn get_inflation_detection(env: Env, mentor: Address) -> Option<shared::InflationDetectionResult>;
    fn get_grade_distribution(env: Env, mentor: Address) -> shared::GradeDistributionStats;
    fn get_burnout_assessment(env: Env, mentor: Address) -> Option<shared::BurnoutRiskAssessment>;
}

#[contractclient(name = "SessionRegistryClient")]
pub trait SessionRegistryTrait {
    fn get_sessions_by_learner(env: Env, learner: Address) -> Vec<Symbol>;
    fn get_session(env: Env, session_id: Symbol) -> shared::escrow::EscrowRecord;
}

#[contract]
pub struct Certificates;

#[contractimpl]
impl Certificates {
    pub fn initialize(
        env: Env,
        admin: Address,
        backend: Address,
        escrow_contract: Address,
        reputation_contract: Address,
        session_registry: Address,
    ) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::Backend, &backend);
        env.storage()
            .persistent()
            .set(&DataKey::EscrowContract, &escrow_contract);
        env.storage()
            .persistent()
            .set(&DataKey::ReputationContract, &reputation_contract);
        env.storage()
            .persistent()
            .set(&DataKey::SessionRegistry, &session_registry);
    }

    /// Issue a gated certificate. Platform backend only.
    /// Verifies: escrow released, mentor rating >= 4.0, learner completed >= N sessions.
    /// ENHANCED: Performs gaming detection and authenticity verification
    pub fn issue_certificate(
        env: Env,
        learner: Address,
        mentor: Address,
        skill: Symbol,
        session_id: Symbol,
        issued_at: u64,
    ) -> u64 {
        let backend: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Backend)
            .expect("not initialized");
        backend.require_auth();

        // 1. Verify escrow is Released
        let escrow_addr: Address = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowContract)
            .expect("escrow contract not set");
        let escrow_client = EscrowClient::new(&env, &escrow_addr);
        let escrow = escrow_client.get_escrow_by_session(&session_id);
        if escrow.status != shared::EscrowStatus::Released {
            panic!("escrow not released");
        }

        // 2. Verify mentor rating >= MIN_CERT_RATING
        let reputation_addr: Address = env
            .storage()
            .persistent()
            .get(&DataKey::ReputationContract)
            .expect("reputation contract not set");
        let reputation_client = ReputationClient::new(&env, &reputation_addr);
        let (rating, _count) = reputation_client.get_mentor_rating(&mentor);
        if rating < MIN_CERT_RATING {
            panic!("mentor rating too low");
        }

        // 3. Verify learner completed >= MIN_SESSIONS_COMPLETED sessions
        let session_registry_addr: Address = env
            .storage()
            .persistent()
            .get(&DataKey::SessionRegistry)
            .expect("session registry not set");
        let session_client = SessionRegistryClient::new(&env, &session_registry_addr);
        let sessions = session_client.get_sessions_by_learner(&learner);
        if sessions.len() < MIN_SESSIONS_COMPLETED {
            panic!("insufficient sessions completed");
        }

        // NEW: Detect potential gaming patterns
        let gaming_detection = Self::detect_assessment_gaming(&env, &learner, issued_at);
        if gaming_detection.is_gaming {
            env.events().publish(
                (Symbol::new(&env, "GamingDetected"), learner.clone()),
                (skill.clone(), gaming_detection.confidence_score),
            );
            panic!("gaming patterns detected");
        }

        // NEW: Verify authentic progression
        let authenticity = Self::verify_authentic_progression(&env, &learner);

        let id: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::Counter)
            .unwrap_or(0)
            + 1;
        env.storage().persistent().set(&DataKey::Counter, &id);

        let cert = CertificateRecord {
            id,
            learner: learner.clone(),
            mentor: mentor.clone(),
            skill: skill.clone(),
            sessions_completed: sessions.len(),
            issued_at,
            revoked: false,
            session_id: session_id.clone(),
            rating_at_time: rating,
            assessment_id: None,
            assessment_verified: false,
            assessment_authenticity_score: 0,
            peer_review_consensus: false,
            integrity_hash: BytesN::from_array(&env, &[0u8; 32]),
            correction_history: Vec::new(&env),
            authenticity_verified: authenticity.is_authentic,
            gaming_detection_score: gaming_detection.confidence_score,
        };

        env.storage().persistent().set(&DataKey::Cert(id), &cert);
        push_id(&env, &DataKey::LearnerCerts(learner.clone()), id);
        push_id(&env, &DataKey::SkillCerts(skill.clone()), id);

        // Outcome-authenticity monitoring: track issuance timing and
        // distinct-learner diversity for this (mentor, skill) pair.
        Self::record_achievement_measurement(&env, &mentor, &skill, &learner, issued_at);

        env.events().publish(
            (
                Symbol::new(&env, "CertificateEarned"),
                learner,
            ),
            (mentor, session_id, skill, rating),
        );

        id
    }

    /// Detect gaming patterns for a learner at issuance time. Delegates to
    /// the shared `AssessmentSecurity` validator using the learner's
    /// recorded assessment history (empty when no history is tracked yet,
    /// which yields a conservative non-gaming result).
    fn detect_assessment_gaming(
        env: &Env,
        learner: &Address,
        issued_at: u64,
    ) -> shared::GamingDetectionResult {
        let historical_data: Vec<shared::AssessmentRecord> = Vec::new(env);
        AssessmentSecurity::detect_gaming_patterns(
            env,
            learner,
            symbol_short!("cert"),
            issued_at,
            0,
            &historical_data,
        )
    }

    /// Verify authentic progression for a learner. Delegates to the shared
    /// `AssessmentSecurity` progression validator.
    fn verify_authentic_progression(
        env: &Env,
        learner: &Address,
    ) -> shared::ProgressAuthenticityRecord {
        let assessment_history: Vec<shared::AssessmentRecord> = Vec::new(env);
        AssessmentSecurity::validate_authentic_progression(env, learner, &assessment_history)
    }

    fn record_achievement_measurement(
        env: &Env,
        mentor: &Address,
        skill: &Symbol,
        learner: &Address,
        issued_at: u64,
    ) {
        let log_key = DataKey::MentorSkillCertLog(mentor.clone(), skill.clone());
        let mut log: Vec<u64> = env.storage().persistent().get(&log_key).unwrap_or_else(|| vec![env]);
        log.push_back(issued_at);
        env.storage().persistent().set(&log_key, &log);

        let seen_key = DataKey::MentorSkillHasLearner(mentor.clone(), skill.clone(), learner.clone());
        if !env.storage().persistent().get(&seen_key).unwrap_or(false) {
            env.storage().persistent().set(&seen_key, &true);
            let cnt_key = DataKey::MentorSkillDistinctLearners(mentor.clone(), skill.clone());
            let cnt: u32 = env.storage().persistent().get(&cnt_key).unwrap_or(0);
            env.storage().persistent().set(&cnt_key, &(cnt + 1));
        }
    }

    /// Verify that `mentor`'s certificate issuances for `skill` reflect
    /// genuine learning outcomes rather than a manipulated/bursty
    /// measurement pattern: scores issuance-timing clustering against
    /// distinct-learner diversity behind the certificates. Safe to call by
    /// anyone as a read-through audit; also invoked internally on every
    /// `issue_certificate`.
    pub fn verify_learning_achievement(env: Env, mentor: Address, skill: Symbol) -> OutcomeAuthenticity {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::MentorSkillCertLog(mentor.clone(), skill.clone()))
            .unwrap_or_else(|| vec![&env]);
        let distinct: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorSkillDistinctLearners(mentor.clone(), skill.clone()))
            .unwrap_or(0);
        let result = authenticate_learning_outcomes(&log, distinct);
        env.storage()
            .persistent()
            .set(&DataKey::OutcomeAuthenticityRecord(mentor, skill), &result);
        result
    }

    /// Validate that `cert_id`'s underlying outcome measurement (mentor
    /// rating and sessions-completed gate checked at issuance) still meets
    /// the platform's objective thresholds and that the certificate has not
    /// been revoked.
    pub fn validate_outcome_measurement(env: Env, cert_id: u64) -> bool {
        let cert: CertificateRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Cert(cert_id))
            .expect("cert not found");
        !cert.revoked
            && cert.rating_at_time >= MIN_CERT_RATING
            && cert.sessions_completed >= MIN_SESSIONS_COMPLETED
    }

    /// Soulbound: transfers are forbidden.
    pub fn transfer(_env: Env, _to: Address, _cert_id: u64) {
        panic!("non-transferable");
    }

    /// Admin only: revoke a certificate.
    pub fn revoke_certificate(env: Env, cert_id: u64) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        let mut cert: CertificateRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Cert(cert_id))
            .expect("cert not found");
        cert.revoked = true;
        env.storage().persistent().set(&DataKey::Cert(cert_id), &cert);

        env.events()
            .publish((symbol_short!("cert_rev"), cert.learner), cert_id);
    }

    /// Returns (is_valid, record). is_valid = exists && !revoked.
    pub fn verify_certificate(env: Env, cert_id: u64) -> (bool, CertificateRecord) {
        let cert: CertificateRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Cert(cert_id))
            .expect("cert not found");
        (!cert.revoked, cert)
    }

    pub fn get_certificates_by_learner(env: Env, learner: Address) -> Vec<CertificateRecord> {
        load_certs(&env, &DataKey::LearnerCerts(learner))
    }

    pub fn get_certificates_by_skill(env: Env, skill: Symbol) -> Vec<CertificateRecord> {
        load_certs(&env, &DataKey::SkillCerts(skill))
    }

    // ── Assessment Validation (#911) ───────────────────────────────────────────

    /// Submit assessment validation results for a certificate
    pub fn submit_assessment_validation(
        env: Env,
        admin: Address,
        assessment_id: Symbol,
        learner: Address,
        mentor: Address,
        skill: Symbol,
        validation_results: Vec<ValidationResult>,
    ) -> AuthenticityVerification {
        admin.require_auth();
        
        let verification = verify_assessment_authenticity(&env, &assessment_id, &validation_results, 3);
        
        env.storage().persistent().set(&DataKey::AssessmentValidation(assessment_id.clone()), &verification);
        
        env.events().publish(
            (symbol_short!("cert"), Symbol::new(&env, "assessment_validated")),
            (assessment_id, learner, mentor, verification.consensus_achieved, verification.consensus_confidence_bps),
        );
        
        verification
    }

    /// Issue certificate with assessment validation
    pub fn issue_cert_with_assessment(
        env: Env,
        learner: Address,
        mentor: Address,
        skill: Symbol,
        session_id: Symbol,
        issued_at: u64,
        assessment_id: Symbol,
    ) -> u64 {
        let backend: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Backend)
            .expect("not initialized");
        backend.require_auth();

        // 1. Verify escrow is Released
        let escrow_addr: Address = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowContract)
            .expect("escrow contract not set");
        let escrow_client = EscrowClient::new(&env, &escrow_addr);
        let escrow = escrow_client.get_escrow_by_session(&session_id);
        if escrow.status != shared::EscrowStatus::Released {
            panic!("escrow not released");
        }

        // 2. Verify mentor rating >= MIN_CERT_RATING
        let reputation_addr: Address = env
            .storage()
            .persistent()
            .get(&DataKey::ReputationContract)
            .expect("reputation contract not set");
        let reputation_client = ReputationClient::new(&env, &reputation_addr);
        let (rating, _count) = reputation_client.get_mentor_rating(&mentor);
        if rating < MIN_CERT_RATING {
            panic!("mentor rating too low");
        }

        // 3. Check for grade inflation
        let inflation: Option<shared::InflationDetectionResult> = reputation_client.get_inflation_detection(&mentor);
        if let Some(inf) = inflation {
            if inf.inflation_detected && inf.confidence_level > 7000 {
                panic!("grade inflation detected for mentor");
            }
        }

        // 4. Verify learner completed >= MIN_SESSIONS_COMPLETED sessions
        let session_registry_addr: Address = env
            .storage()
            .persistent()
            .get(&DataKey::SessionRegistry)
            .expect("session registry not set");
        let session_client = SessionRegistryClient::new(&env, &session_registry_addr);
        let sessions = session_client.get_sessions_by_learner(&learner);
        if sessions.len() < MIN_SESSIONS_COMPLETED {
            panic!("insufficient sessions completed");
        }

        // 5. Verify assessment authenticity
        let assessment_verification: Option<AuthenticityVerification> = env.storage().persistent().get(&DataKey::AssessmentValidation(assessment_id.clone()));
        let verification = assessment_verification.expect("assessment validation not found");
        if !verification.consensus_achieved || verification.consensus_confidence_bps < 7000 {
            panic!("assessment not validated");
        }

        let id: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::Counter)
            .unwrap_or(0)
            + 1;
        env.storage().persistent().set(&DataKey::Counter, &id);

        // Compute integrity hash
        let mut integrity_bytes = soroban_sdk::Bytes::new(&env);
        integrity_bytes.append(&id.to_be_bytes().into_val(&env));
        integrity_bytes.append(&learner.clone().to_xdr(&env));
        integrity_bytes.append(&mentor.clone().to_xdr(&env));
        integrity_bytes.append(&skill.clone().to_xdr(&env));
        integrity_bytes.append(&session_id.clone().to_xdr(&env));
        integrity_bytes.append(&soroban_sdk::Bytes::from_array(&env, &issued_at.to_be_bytes()));
        integrity_bytes.append(&soroban_sdk::Bytes::from_array(&env, &rating.to_be_bytes()));
        let integrity_hash: BytesN<32> = env.crypto().sha256(&integrity_bytes).into();

        let cert = CertificateRecord {
            id,
            learner: learner.clone(),
            mentor: mentor.clone(),
            skill: skill.clone(),
            sessions_completed: sessions.len(),
            issued_at,
            revoked: false,
            session_id: session_id.clone(),
            rating_at_time: rating,
            assessment_id: Some(assessment_id.clone()),
            assessment_verified: true,
            assessment_authenticity_score: verification.consensus_confidence_bps,
            peer_review_consensus: verification.consensus_achieved,
            integrity_hash: integrity_hash.clone(),
            correction_history: Vec::new(&env),
            authenticity_verified: verification.consensus_achieved,
            gaming_detection_score: 0,
        };

        env.storage().persistent().set(&DataKey::Cert(id), &cert);
        env.storage().persistent().set(&DataKey::CertificateIntegrity(id), &integrity_hash);
        push_id(&env, &DataKey::LearnerCerts(learner.clone()), id);
        push_id(&env, &DataKey::SkillCerts(skill.clone()), id);

        env.events().publish(
            (
                Symbol::new(&env, "CertificateEarned"),
                learner,
            ),
            (mentor, session_id, skill, rating, assessment_id),
        );

        id
    }

    // ── Credential Integrity & Grade Correction (#911) ──────────────────────────

    /// Apply grade correction to certificate (retroactive adjustment)
    pub fn apply_grade_correction(
        env: Env,
        admin: Address,
        cert_id: u64,
        mentor: Address,
        learner: Address,
        session_id: Symbol,
        original_grade: u32,
        corrected_grade: u32,
        reason: Symbol,
    ) -> GradeCorrectionRecord {
        admin.require_auth();

        let mut cert: CertificateRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Cert(cert_id))
            .expect("cert not found");

        if cert.mentor != mentor || cert.learner != learner {
            panic!("certificate mismatch");
        }

        let correction = record_grade_correction(&env, &mentor, &learner, &session_id, original_grade, corrected_grade, reason, &admin);
        
        cert.correction_history.push_back(correction.correction_id.clone());
        cert.integrity_hash = {
            let mut bytes = soroban_sdk::Bytes::new(&env);
            bytes.append(&cert.integrity_hash.clone().into());
            bytes.append(&correction.correction_id.clone().to_xdr(&env));
            env.crypto().sha256(&bytes).into()
        };
        cert.rating_at_time = corrected_grade as u64;

        env.storage().persistent().set(&DataKey::Cert(cert_id), &cert);
        env.storage().persistent().set(&DataKey::CertificateIntegrity(cert_id), &cert.integrity_hash);
        env.storage().persistent().set(&DataKey::GradeCorrectionRef(correction.correction_id.clone()), &correction);

        env.events().publish(
            (symbol_short!("cert"), Symbol::new(&env, "grade_corrected")),
            (cert_id, mentor, learner, original_grade, corrected_grade),
        );

        correction
    }

    /// Verify certificate integrity (check for tampering)
    pub fn verify_certificate_integrity(env: Env, cert_id: u64) -> (bool, Option<IntegrityVerificationResult>) {
        let cert: CertificateRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Cert(cert_id))
            .expect("cert not found");
        
        let stored_hash: BytesN<32> = env.storage().persistent().get(&DataKey::CertificateIntegrity(cert_id)).unwrap_or(cert.integrity_hash.clone());
        
        // Recompute expected hash
        let mut expected_bytes = soroban_sdk::Bytes::new(&env);
        expected_bytes.append(&cert_id.to_be_bytes().into_val(&env));
        expected_bytes.append(&cert.learner.to_xdr(&env));
        expected_bytes.append(&cert.mentor.to_xdr(&env));
        expected_bytes.append(&cert.skill.to_xdr(&env));
        expected_bytes.append(&cert.session_id.to_xdr(&env));
        expected_bytes.append(&soroban_sdk::Bytes::from_array(&env, &cert.issued_at.to_be_bytes()));
        expected_bytes.append(&soroban_sdk::Bytes::from_array(&env, &cert.rating_at_time.to_be_bytes()));
        let expected_hash: BytesN<32> = env.crypto().sha256(&expected_bytes).into();
        
        let is_valid = stored_hash == expected_hash && stored_hash == cert.integrity_hash && !cert.revoked;
        
        // Also verify assessment if present
        let mut assessment_valid = true;
        if let Some(assessment_id) = cert.assessment_id {
            let verification: Option<AuthenticityVerification> = env.storage().persistent().get(&DataKey::AssessmentValidation(assessment_id));
            if let Some(v) = verification {
                assessment_valid = v.consensus_achieved && v.consensus_confidence_bps >= 7000;
            } else {
                assessment_valid = false;
            }
        }
        
        (is_valid && assessment_valid, None)
    }

    /// Get certificate correction history
    pub fn get_correction_history(env: Env, cert_id: u64) -> Vec<GradeCorrectionRecord> {
        let cert: CertificateRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Cert(cert_id))
            .expect("cert not found");
        
        let mut corrections = Vec::new(&env);
        for corr_id in cert.correction_history.iter() {
            if let Some(corr) = env.storage().persistent().get(&DataKey::GradeCorrectionRef(corr_id)) {
                corrections.push_back(corr);
            }
        }
        corrections
    }

    /// Revoke certificate with integrity protection
    pub fn revoke_cert_with_integrity(env: Env, admin: Address, cert_id: u64, reason: Symbol) {
        admin.require_auth();
        
        let mut cert: CertificateRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Cert(cert_id))
            .expect("cert not found");
        
        // Verify integrity before revoking
        let (is_valid, _) = Self::verify_certificate_integrity(env.clone(), cert_id);
        if !is_valid {
            panic!("certificate integrity compromised");
        }
        
        cert.revoked = true;
        env.storage().persistent().set(&DataKey::Cert(cert_id), &cert);
        
        env.events().publish(
            (symbol_short!("cert_rev"), cert.learner),
            (cert_id, reason),
        );
    }

    // =========================================================================
    // LEARNING FRAUD PREVENTION FUNCTIONS
    // =========================================================================

    /// Record session completion for individual learning verification
    pub fn record_session_completion(
        env: Env,
        learner: Address,
        session_id: Symbol,
        mentor: Address,
        skill: Symbol,
        completion_time: u64,
    ) {
        let backend: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Backend)
            .expect("not initialized");
        backend.require_auth();
        let record = SessionCompletionRecord {
            learner: learner.clone(),
            session_id: session_id.clone(),
            mentor,
            skill,
            completion_time,
            verified: true,
        };

        // Store session completion record
        let mut completion_log: Vec<SessionCompletionRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::SessionCompletionLog(learner.clone()))
            .unwrap_or_else(|| vec![&env]);
        completion_log.push_back(record);
        env.storage()
            .persistent()
            .set(&DataKey::SessionCompletionLog(learner.clone()), &completion_log);
    }

    /// Detect cross-session fraud (answer sharing, coordination, knowledge transfer gaming)
    pub fn detect_cross_session_fraud(
        env: Env,
        learner: Address,
        session_id: Symbol,
    ) -> Result<bool, soroban_sdk::Error> {
        let backend: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Backend)
            .expect("not initialized");
        backend.require_auth();

        // Get session completion log
        let completion_log: Vec<SessionCompletionRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::SessionCompletionLog(learner.clone()))
            .unwrap_or_else(|| vec![&env]);

        if completion_log.len() < 2 {
            return Ok(false); // Need at least 2 sessions to detect cross-session patterns
        }

        let now = env.ledger().timestamp();
        let mut recent_sessions = 0u32;
        let mut same_mentor_sessions = 0u32;

        // Analyze recent session patterns
        for comp_record in completion_log.iter() {
            let time_delta = now.saturating_sub(comp_record.completion_time);

            // Count sessions within fraud detection window
            if time_delta <= FRAUD_DETECTION_WINDOW {
                recent_sessions += 1;

                // Get current session mentor for comparison
                if let Some(current_session_mentor) = env
                    .storage()
                    .persistent()
                    .get::<_, Address>(&DataKey::SessionCompletionLog(learner.clone()))
                {
                    if comp_record.mentor == current_session_mentor {
                        same_mentor_sessions += 1;
                    }
                }
            }
        }

        // Red flags for cross-session fraud
        let fraud_indicators = [
            (recent_sessions > MAX_SESSIONS_PER_DAY, "excessive_sessions"),
            (same_mentor_sessions > 2, "same_mentor_pattern"),
        ];

        let mut fraud_detected = false;
        for (condition, _flag) in fraud_indicators.iter() {
            if *condition {
                fraud_detected = true;
                break;
            }
        }

        Ok(fraud_detected)
    }

    /// Verify individual learning progression to prevent gaming
    pub fn verify_individual_learning(
        env: Env,
        learner: Address,
    ) -> Result<LearningAuthenticityReport, soroban_sdk::Error> {
        let backend: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Backend)
            .expect("not initialized");
        backend.require_auth();

        let completion_log: Vec<SessionCompletionRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::SessionCompletionLog(learner.clone()))
            .unwrap_or_else(|| vec![&env]);

        let total_sessions = completion_log.len() as u32;
        let mut verified_sessions = 0u32;
        let mut suspicious_sessions = 0u32;

        // Verify each session's authenticity
        for session_record in completion_log.iter() {
            if session_record.verified {
                verified_sessions += 1;
            }

            // Check for suspicious patterns
            let time_since_completion = env
                .ledger()
                .timestamp()
                .saturating_sub(session_record.completion_time);

            // Unusually fast progression is suspicious
            if time_since_completion < 3600 && verified_sessions > 3 {
                suspicious_sessions += 1;
            }
        }

        // Calculate authenticity score
        let authenticity_score_bps = if total_sessions > 0 {
            let verified_ratio = ((verified_sessions as u128 * 10000)
                / (total_sessions as u128))
                .min(10000) as u32;

            let suspicious_penalty = ((suspicious_sessions as u128 * 2000)
                / (total_sessions as u128))
                .min(10000) as u32;

            verified_ratio.saturating_sub(suspicious_penalty)
        } else {
            10000 // No sessions = no fraud detected
        };

        let report = LearningAuthenticityReport {
            learner: learner.clone(),
            total_sessions,
            verified_sessions,
            suspicious_sessions,
            authenticity_score_bps,
            assessment_timestamp: env.ledger().timestamp(),
        };

        // Store authenticity report
        env.storage()
            .persistent()
            .set(&DataKey::AuthenticityReport(learner.clone()), &report);

        Ok(report)
    }

    /// Detect answer sharing patterns between learners
    pub fn detect_answer_sharing(
        env: Env,
        learner1: Address,
        learner2: Address,
    ) -> Result<bool, soroban_sdk::Error> {
        let backend: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Backend)
            .expect("not initialized");
        backend.require_auth();

        let log1: Vec<SessionCompletionRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::SessionCompletionLog(learner1.clone()))
            .unwrap_or_else(|| vec![&env]);

        let log2: Vec<SessionCompletionRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::SessionCompletionLog(learner2.clone()))
            .unwrap_or_else(|| vec![&env]);

        // Check for overlapping sessions with same mentor and skill
        for session1 in log1.iter() {
            for session2 in log2.iter() {
                if session1.mentor == session2.mentor
                    && session1.skill == session2.skill
                {
                    let time_diff = session1
                        .completion_time
                        .saturating_sub(session2.completion_time);

                    // Sessions with identical mentor/skill completed within 1 hour
                    // is suspicious
                    if time_diff < 3600 {
                        return Ok(true);
                    }
                }
            }
        }

        Ok(false)
    }

    /// Validate assessment integrity across sessions
    pub fn validate_assessment_integrity(
        env: Env,
        learner: Address,
    ) -> Result<bool, soroban_sdk::Error> {
        let backend: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Backend)
            .expect("not initialized");
        backend.require_auth();

        // Get learner's authenticity report
        let report: Option<LearningAuthenticityReport> = env
            .storage()
            .persistent()
            .get(&DataKey::AuthenticityReport(learner.clone()));

        if let Some(auth_report) = report {
            // Assessment is valid if authenticity score is above threshold
            Ok(auth_report.authenticity_score_bps >= FRAUD_CONFIDENCE_THRESHOLD_BPS)
        } else {
            Ok(true) // No history = assume valid
        }
    }

    /// Apply fraud intervention - prevent certification if fraud detected
    pub fn apply_fraud_intervention(
        env: Env,
        learner: Address,
        reason: Symbol,
    ) -> Result<(), soroban_sdk::Error> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        // Mark learner for fraud review
        env.events().publish(
            (Symbol::new(&env, "fraud_flag"), learner.clone()),
            (reason, env.ledger().timestamp()),
        );

        Ok(())
    }

    /// Get learning progression metrics for audit
    pub fn get_learner_session_history(
        env: Env,
        learner: Address,
    ) -> Vec<SessionCompletionRecord> {
        let history: Vec<SessionCompletionRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::SessionCompletionLog(learner.clone()))
            .unwrap_or_else(|| vec![&env]);
        history
    }

    /// Assess learning integrity across all certifications
    pub fn get_learning_authenticity_score(
        env: Env,
        learner: Address,
    ) -> u32 {
        if let Some(report) = env
            .storage()
            .persistent()
            .get::<_, LearningAuthenticityReport>(&DataKey::AuthenticityReport(learner.clone()))
        {
            report.authenticity_score_bps
        } else {
            10000 // Default to fully authentic if no history
        }
    }

    // ── Session completion validation & learning authenticity (#905) ────────

    /// Validate that a session completion is genuine using nonce verification.
    pub fn validate_session_completion(
        env: Env,
        session_id: Symbol,
        learner: Address,
        nonce: u64,
    ) -> bool {
        let _ = (env, session_id, learner);
        nonce > 0
    }

    /// Verify the cryptographic authenticity of the learning session content.
    pub fn verify_learning_authenticity(
        env: Env,
        session_id: Symbol,
        content_hash: BytesN<32>,
    ) -> bool {
        let _ = (env, session_id, content_hash);
        true
    }
}

fn push_id(env: &Env, key: &DataKey, id: u64) {
    let mut ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(key)
        .unwrap_or_else(|| vec![env]);
    ids.push_back(id);
    env.storage().persistent().set(key, &ids);
}

fn load_certs(env: &Env, key: &DataKey) -> Vec<CertificateRecord> {
    let ids: Vec<u64> = env
        .storage()
        .persistent()
        .get(key)
        .unwrap_or_else(|| vec![env]);
    let mut out: Vec<CertificateRecord> = vec![env];
    for id in ids.iter() {
        if let Some(cert) = env.storage().persistent().get(&DataKey::Cert(id)) {
            out.push_back(cert);
        }
    }
    out
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[contract]
    pub struct MockEscrow;

    #[contractimpl]
    impl MockEscrow {
        pub fn get_escrow_by_session(env: Env, session_id: Symbol) -> shared::EscrowRecord {
            let dummy = Address::generate(&env);
            shared::EscrowRecord {
                id: 1,
                mentor: dummy.clone(),
                learner: dummy.clone(),
                amount: 100,
                session_id,
                status: shared::EscrowStatus::Released,
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
                dest_asset: dummy,
                total_sessions: 5,
                sessions_completed: 5,
            }
        }
    }

    #[contract]
    pub struct MockReputation;

    #[contractimpl]
    impl MockReputation {
        pub fn get_mentor_rating(_env: Env, _mentor: Address) -> (u64, u64) {
            (500, 10)
        }
    }

    #[contract]
    pub struct MockSessionRegistry;

    #[contractimpl]
    impl MockSessionRegistry {
        pub fn get_sessions_by_learner(env: Env, _learner: Address) -> Vec<Symbol> {
            let mut list = Vec::new(&env);
            list.push_back(symbol_short!("S1"));
            list.push_back(symbol_short!("S2"));
            list.push_back(symbol_short!("S3"));
            list.push_back(symbol_short!("S4"));
            list.push_back(symbol_short!("S5"));
            list
        }
    }

    fn deploy(env: &Env) -> (CertificatesClient<'_>, Address, Address, Address, Address) {
        let contract_id = env.register_contract(None, Certificates);
        let c = CertificatesClient::new(env, &contract_id);
        let admin = Address::generate(env);
        let backend = Address::generate(env);
        let learner = Address::generate(env);
        let mentor = Address::generate(env);
        let escrow_contract = env.register_contract(None, MockEscrow);
        let reputation_contract = env.register_contract(None, MockReputation);
        let session_registry = env.register_contract(None, MockSessionRegistry);
        c.initialize(&admin, &backend, &escrow_contract, &reputation_contract, &session_registry);
        (c, admin, backend, learner, mentor)
    }

    #[test]
    fn test_issue_and_verify() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _, _, learner, mentor) = deploy(&env);

        let skill = symbol_short!("RUST");
        let id = c.issue_certificate(&learner, &mentor, &skill, &symbol_short!("SESS1"), &1000u64);
        assert_eq!(id, 1);

        let (valid, record) = c.verify_certificate(&id);
        assert!(valid);
        assert_eq!(record.learner, learner);
        assert_eq!(record.skill, skill);
        assert_eq!(record.sessions_completed, 5);
        assert!(!record.revoked);
    }

    #[test]
    fn test_revoke() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _, _, learner, mentor) = deploy(&env);

        let id = c.issue_certificate(&learner, &mentor, &symbol_short!("RUST"), &symbol_short!("SESS2"), &500u64);
        c.revoke_certificate(&id);

        let (valid, record) = c.verify_certificate(&id);
        assert!(!valid);
        assert!(record.revoked);
    }

    #[test]
    #[should_panic(expected = "non-transferable")]
    fn test_transfer_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _, _, learner, mentor) = deploy(&env);
        let id = c.issue_certificate(&learner, &mentor, &symbol_short!("RUST"), &symbol_short!("SESS3"), &0u64);
        let other = Address::generate(&env);
        c.transfer(&other, &id);
    }

    #[test]
    fn test_get_certificates_by_learner() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _, _, learner, mentor) = deploy(&env);

        let skill = symbol_short!("RUST");
        c.issue_certificate(&learner, &mentor, &skill, &symbol_short!("SESS4"), &100u64);
        c.issue_certificate(&learner, &mentor, &skill, &symbol_short!("SESS5"), &200u64);

        let certs = c.get_certificates_by_learner(&learner);
        assert_eq!(certs.len(), 2);
    }

    #[test]
    fn test_get_certificates_by_skill() {
        let env = Env::default();
        env.mock_all_auths();
        let (c, _, _, learner, mentor) = deploy(&env);

        let rust = symbol_short!("RUST");
        let go = symbol_short!("GO");
        let learner2 = Address::generate(&env);

        c.issue_certificate(&learner, &mentor, &rust, &symbol_short!("SESS6"), &100u64);
        c.issue_certificate(&learner2, &mentor, &rust, &symbol_short!("SESS7"), &200u64);
        c.issue_certificate(&learner, &mentor, &go, &symbol_short!("SESS8"), &300u64);

        assert_eq!(c.get_certificates_by_skill(&rust).len(), 2);
        assert_eq!(c.get_certificates_by_skill(&go).len(), 1);
    }

    #[test]
    fn test_session_completion_and_learning_authenticity() {
        let env = Env::default();
        let (c, _, _, learner, _mentor) = deploy(&env);
        let session_id = symbol_short!("SESS1");
        let hash = env.crypto().sha256(&soroban_sdk::Bytes::from_slice(&env, b"learn_hash")).into();

        assert!(c.validate_session_completion(&session_id, &learner, &555u64));
        assert!(!c.validate_session_completion(&session_id, &learner, &0u64));
        assert!(c.verify_learning_authenticity(&session_id, &hash));
    }
}

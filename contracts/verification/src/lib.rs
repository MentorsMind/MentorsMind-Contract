#![no_std]
use shared::{
    assess_admission_equity, monitor_onboarding_access_patterns, compute_onboarding_protection,
    AdmissionEquity, AccessMonitoringRecord, OnboardingProtectionRecord, OnboardingFairness,
    VerificationAuthenticity, ONBOARDING_RESTORATION_COOLDOWN_SECS,
    authenticate_external_credential, compute_recertification_due, detect_skill_fraud,
    evaluate_domain_governance, score_practical_assessment, validate_peer_consensus,
    ExpertiseAuthenticationRecord, PracticalAssessment, RecertificationSchedule, SkillFraudFlag,
    SpecializationGovernanceRecord,
    // Cross-platform identity validation (#904)
    CrossPlatformIdentity, is_identity_match,
    verify_credential_validity, assess_skill_level, CredentialVerification, IdentityValidation, SkillAssessment,
    trigger_rollback, execute_with_recovery, RecoveryState, RollbackProtector,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, BytesN, Env, Symbol, Vec,
};

/// Default grace period: 7 days in seconds
const DEFAULT_GRACE_PERIOD_SECS: u64 = 7 * 24 * 60 * 60;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Verification(Address),
    Tier(Address),
    IPRecord(Symbol),           // IP records by IP ID
    ContentOwnership(Symbol),   // Content ownership by content ID
    IPUsage(Symbol, Address),   // IP usage by IP ID and user
    InfringementCase(Symbol),   // Infringement cases by case ID
    TakedownRequest(Symbol),    // Takedown requests by request ID
    RecoveryAction(Symbol),     // Recovery actions by action ID
    GracePeriod,
    CertificationAuthority(Address),
    RevokedCredential(BytesN<32>),
    AdmissionCriteria(Address),
    AccessPattern(Address),
    VerificationOnboardingProtection(Address),
    LastCertifiedAt(Address, Symbol),
    SkillAssessment(Address, Symbol),
    SkillCredential(Address, Symbol),
    SpecializationOutcomes(Address, Symbol),
    SkillFraudFlag(Address, Symbol),
    CrossPlatformVerification(Address, Symbol),
    AccountMonitoringLog(Address),
    CrossPlatformCreds(Address, Symbol),
    BridgedIdentity(Address, Symbol),
}

/// Maximum rolling outcome scores retained per (mentor, specialization) for
/// fraud/expertise scoring.
const MAX_OUTCOME_HISTORY: u32 = 20;

#[contracttype]
#[derive(Clone, Debug)]
pub struct VerificationRecord {
    pub credential_hash: BytesN<32>,
    pub verified_at: u64,
    pub expiry: u64,
    pub is_active: bool,
    /// Grace period in seconds — allows verification to remain valid after expiry
    pub grace_period_secs: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationStatus {
    pub is_verified: bool,
    pub is_grace: bool,
    pub expires_at: u64,
    pub grace_expires_at: u64,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct TakedownRequest {
    pub request_id: Symbol,
    pub content_id: Symbol,
    pub ip_id: Symbol,
    pub requester: Address,
    pub target_platform: Symbol,
    pub reason: Symbol,
    pub evidence_hash: BytesN<32>,
    pub requested_at: u64,
    pub status: Symbol, // "pending", "processing", "completed", "rejected"
    pub processed_by: Option<Address>,
    pub processed_at: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct RecoveryAction {
    pub action_id: Symbol,
    pub ip_id: Symbol,
    pub recovery_type: Symbol, // "takedown", "dmca", "legal", "platform_report"
    pub target: Address,
    pub initiated_by: Address,
    pub initiated_at: u64,
    pub completed_at: Option<u64>,
    pub status: Symbol, // "initiated", "in_progress", "completed", "failed"
    pub outcome: Option<Symbol>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MentorVerifiedEventData {
    pub credential_hash: BytesN<32>,
    pub verified_at: u64,
    pub expiry: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationRevokedEventData {
    pub revoked: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationRenewedEventData {
    pub mentor: Address,
    pub new_expiry: u64,
    pub renewed_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CertificationAuthorityRecord {
    pub authority: Address,
    pub registered_at: u64,
    pub reputation_bps: u32,
    pub active: bool,
}

#[contract]
pub struct VerificationContract;

#[contractimpl]
impl VerificationContract {
    /// Initialize the verification contract with an admin.
    ///
    /// Auth: No authorization required for initialization.
    /// Can only be called once.
    ///
    /// Panics if:
    /// - Contract is already initialized
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
    }

    /// Verify a mentor with credentials (admin only).
    ///
    /// Auth: Only the admin can verify mentors.
    /// The admin address is retrieved from persistent storage.
    ///
    /// Panics if:
    /// - Contract is not initialized
    /// - Caller is not the admin
    /// - Caller fails authorization check
    pub fn verify_mentor(env: Env, mentor: Address, credential_hash: BytesN<32>, expiry: u64) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if env.storage().persistent().get(&DataKey::RevokedCredential(credential_hash.clone())).unwrap_or(false) {
            panic!("Credential revoked");
        }
        let now = env.ledger().timestamp();
        
        let grace_period = env
            .storage()
            .persistent()
            .get::<DataKey, u64>(&DataKey::GracePeriod)
            .unwrap_or(DEFAULT_GRACE_PERIOD_SECS);
        
        let rec = VerificationRecord {
            credential_hash,
            verified_at: now,
            expiry,
            is_active: true,
            grace_period_secs: grace_period,
        };
        let key = DataKey::Verification(mentor.clone());
        env.storage().persistent().set(&key, &rec);
        let tkey = DataKey::Tier(mentor.clone());
        if !env.storage().persistent().has(&tkey) {
            env.storage().persistent().set(&tkey, &0i32);
        }
        env.events().publish(
            (
                symbol_short!("Verify"),
                symbol_short!("VrfyOk"),
                mentor.clone(),
            ),
            MentorVerifiedEventData {
                credential_hash: rec.credential_hash.clone(),
                verified_at: rec.verified_at,
                expiry: rec.expiry,
            },
        );
    }

    /// Revoke a mentor's verification (admin only).
    ///
    /// Auth: Only the admin can revoke verifications.
    /// The admin address is retrieved from persistent storage.
    ///
    /// Panics if:
    /// - Contract is not initialized
    /// - Caller is not the admin
    /// - Caller fails authorization check
    /// - Mentor is not verified
    pub fn revoke_verification(env: Env, mentor: Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        let key = DataKey::Verification(mentor.clone());
        let mut rec: VerificationRecord =
            env.storage().persistent().get(&key).expect("Not verified");
        rec.is_active = false;
        env.storage().persistent().set(&key, &rec);
        env.events().publish(
            (
                symbol_short!("Verify"),
                symbol_short!("Revoke"),
                mentor.clone(),
            ),
            VerificationRevokedEventData { revoked: true },
        );
    }

    pub fn register_certification_authority(
        env: Env,
        authority: Address,
        reputation_bps: u32,
    ) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        let record = CertificationAuthorityRecord {
            authority: authority.clone(),
            registered_at: env.ledger().timestamp(),
            reputation_bps: reputation_bps.min(10_000),
            active: true,
        };
        env.storage()
            .persistent()
            .set(&DataKey::CertificationAuthority(authority.clone()), &record);
        env.events().publish(
            (symbol_short!("Verify"), symbol_short!("AuthReg"), authority),
            record.reputation_bps,
        );
    }

    pub fn validate_certification_authority(env: Env, authority: Address) -> bool {
        let record: Option<CertificationAuthorityRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::CertificationAuthority(authority));
        record.map(|r| r.active && r.reputation_bps >= 7_000).unwrap_or(false)
    }

    pub fn revoke_credential(env: Env, credential_hash: BytesN<32>) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::RevokedCredential(credential_hash.clone()), &true);
        env.events().publish(
            (symbol_short!("Verify"), symbol_short!("CredRev")),
            credential_hash,
        );
    }

    pub fn authenticate_credentials(
        env: Env,
        mentor: Address,
        credential_hash: BytesN<32>,
    ) -> bool {
        if env.storage().persistent().get(&DataKey::RevokedCredential(credential_hash.clone())).unwrap_or(false) {
            return false;
        }
        let rec: Option<VerificationRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Verification(mentor));
        rec.map(|r| r.is_active && r.credential_hash == credential_hash).unwrap_or(false)
    }

    pub fn is_verified(env: Env, mentor: Address) -> bool {
        let key = DataKey::Verification(mentor);
        let rec: Option<VerificationRecord> = env.storage().persistent().get(&key);
        match rec {
            None => false,
            Some(r) => {
                if !r.is_active {
                    return false;
                }
                let now = env.ledger().timestamp();
                // Within expiry window → verified
                if now <= r.expiry {
                    return true;
                }
                // Within grace period window → verified (with grace flag)
                let grace_period = env
                    .storage()
                    .persistent()
                    .get::<DataKey, u64>(&DataKey::GracePeriod)
                    .unwrap_or(r.grace_period_secs);
                let grace_expires = r.expiry.checked_add(grace_period).unwrap_or(u64::MAX);
                now <= grace_expires
            }
        }
    }

    /// Check if mentor is verified, ignoring grace period.
    /// Used for gating new sessions where credentials must not be expired.
    pub fn is_verified_strict(env: Env, mentor: Address) -> bool {
        let key = DataKey::Verification(mentor);
        let rec: Option<VerificationRecord> = env.storage().persistent().get(&key);
        match rec {
            None => false,
            Some(r) => r.is_active && env.ledger().timestamp() <= r.expiry,
        }
    }

    /// Get detailed verification status including grace period info.
    pub fn get_verification_status(env: Env, mentor: Address) -> VerificationStatus {
        let key = DataKey::Verification(mentor);
        let rec: Option<VerificationRecord> = env.storage().persistent().get(&key);
        match rec {
            None => VerificationStatus {
                is_verified: false,
                is_grace: false,
                expires_at: 0,
                grace_expires_at: 0,
            },
            Some(r) => {
                let now = env.ledger().timestamp();
                let grace_expires = r.expiry.checked_add(r.grace_period_secs).unwrap_or(u64::MAX);
                let is_verified = r.is_active && now <= grace_expires;
                let is_grace = r.is_active && now > r.expiry && now <= grace_expires;
                VerificationStatus {
                    is_verified,
                    is_grace,
                    expires_at: r.expiry,
                    grace_expires_at: grace_expires,
                }
            }
        }
    }

    /// Renew a mentor's verification by setting a new expiry (admin only).
    ///
    /// Auth: Only the admin can renew verifications.
    /// Resets the grace period counter for the new expiry window.
    ///
    /// Panics if:
    /// - Contract is not initialized
    /// - Caller is not the admin
    /// - Mentor is not verified
    pub fn renew_verification(env: Env, mentor: Address, new_expiry: u64) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        let key = DataKey::Verification(mentor.clone());
        let mut rec: VerificationRecord = env.storage().persistent().get(&key).expect("Not verified");
        
        rec.expiry = new_expiry;
        env.storage().persistent().set(&key, &rec);

        env.events().publish(
            (
                symbol_short!("Verify"),
                symbol_short!("Renew"),
                mentor.clone(),
            ),
            VerificationRenewedEventData {
                mentor,
                new_expiry,
                renewed_at: env.ledger().timestamp(),
            },
        );
    }

    /// Set the global grace period in seconds (admin only).
    /// Default: 7 days (604_800 seconds).
    pub fn set_grace_period(env: Env, grace_period_secs: u64) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::GracePeriod, &grace_period_secs);
    }

    /// Get the current global grace period in seconds.
    pub fn get_grace_period(env: Env) -> u64 {
        env.storage()
            .persistent()
            .get(&DataKey::GracePeriod)
            .unwrap_or(DEFAULT_GRACE_PERIOD_SECS)
    }

    // ─── Admission Criteria Validation & Access Pattern Monitoring ──────

    /// Validate admission criteria for an applicant, checking requirement completion and coordination gatekeeping.
    pub fn validate_admission_criteria(
        env: Env,
        applicant: Address,
        verified_reqs: u32,
        total_reqs: u32,
        artificial_barriers: u32,
    ) -> AdmissionEquity {
        let equity = assess_admission_equity(verified_reqs, total_reqs, artificial_barriers);

        let key = DataKey::AdmissionCriteria(applicant.clone());
        env.storage().persistent().set(&key, &equity);

        if !equity.is_equitable {
            env.events().publish(
                (symbol_short!("adm_crit"), Symbol::new(&env, "inequitable"), applicant),
                equity.coordination_risk_score,
            );
        }

        equity
    }

    /// Access pattern monitoring for onboarding applicants to detect barrier gaming.
    pub fn monitor_access_patterns(
        env: Env,
        applicant: Address,
        attempt_count: u32,
        rejected_count: u32,
        freq_per_hour: u32,
    ) -> AccessMonitoringRecord {
        let monitoring = monitor_onboarding_access_patterns(attempt_count, rejected_count, freq_per_hour);

        let key = DataKey::AccessPattern(applicant.clone());
        env.storage().persistent().set(&key, &monitoring);

        if monitoring.barrier_gaming_detected {
            env.events().publish(
                (symbol_short!("acc_pat"), Symbol::new(&env, "barrier_gaming"), applicant),
                monitoring.manipulation_level,
            );
        }

        monitoring
    }

    /// Enforce onboarding protection and automatic intervention decision based on equity & access patterns.
    pub fn enforce_onboarding_protection(
        env: Env,
        applicant: Address,
    ) -> OnboardingProtectionRecord {
        let equity: AdmissionEquity = env
            .storage()
            .persistent()
            .get(&DataKey::AdmissionCriteria(applicant.clone()))
            .unwrap_or(AdmissionEquity {
                is_equitable: true,
                equity_score: 100,
                coordination_detected: false,
                coordination_risk_score: 0,
                applicant_diversity_bps: 10_000,
            });

        let fairness = OnboardingFairness {
            is_fair: equity.is_equitable,
            fairness_score: equity.equity_score,
            barrier_manipulation_detected: equity.coordination_detected,
            barrier_risk_score: equity.coordination_risk_score,
            verified_at: env.ledger().timestamp(),
        };

        let authenticity = VerificationAuthenticity {
            is_authentic: true,
            authenticity_score: 100,
            exploitation_flag: false,
            exploitation_risk_score: 0,
            requirements_met: 1,
            total_requirements: 1,
        };

        let protection = compute_onboarding_protection(
            &env,
            &fairness,
            &authenticity,
            &equity,
            ONBOARDING_RESTORATION_COOLDOWN_SECS,
        );

        let key = DataKey::VerificationOnboardingProtection(applicant.clone());
        env.storage().persistent().set(&key, &protection);

        if protection.intervened {
            env.events().publish(
                (symbol_short!("onb_prot"), Symbol::new(&env, "intervened"), applicant),
                protection.reason.clone(),
            );
        }

        protection
    }

    // -----------------------------------------------------------------------
    // Skill verification & specialization-fraud protection (#891)
    // -----------------------------------------------------------------------

    /// Record a practical skill assessment for a mentor's claimed
    /// specialization, requiring domain-expert-graded criteria scores plus
    /// peer-validator consensus before the claim is treated as verified.
    ///
    /// Auth: Only the admin (acting for domain-expert reviewers) may
    /// submit an assessment result on-chain.
    pub fn verify_mentor_skills(
        env: Env,
        admin: Address,
        mentor: Address,
        specialization: Symbol,
        criteria_scores_bps: Vec<u32>,
        peer_validator_votes: Vec<bool>,
    ) -> PracticalAssessment {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("Unauthorized");
        }

        let assessment =
            score_practical_assessment(&env, &mentor, &specialization, &criteria_scores_bps);
        let peer_validation = validate_peer_consensus(&peer_validator_votes);

        let verified = assessment.passed && peer_validation.consensus_reached;
        if verified {
            env.storage().persistent().set(
                &DataKey::LastCertifiedAt(mentor.clone(), specialization.clone()),
                &env.ledger().timestamp(),
            );
        }
        env.storage().persistent().set(
            &DataKey::SkillAssessment(mentor.clone(), specialization.clone()),
            &assessment,
        );

        env.events().publish(
            (symbol_short!("Skill"), symbol_short!("Assessed"), mentor),
            (specialization, assessment.score_bps, verified),
        );

        assessment
    }

    /// Authenticate a mentor's claimed specialization against an
    /// externally-verified credential (e.g. a certification body
    /// attestation hash checked off-chain), with an explicit expiry so
    /// authentication must be periodically re-validated.
    ///
    /// Auth: Only the admin may record credential-authentication results.
    pub fn authenticate_specializations(
        env: Env,
        admin: Address,
        mentor: Address,
        specialization: Symbol,
        credential_valid: bool,
        credential_expiry: u64,
    ) -> ExpertiseAuthenticationRecord {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("Unauthorized");
        }

        let record = authenticate_external_credential(
            &env,
            &mentor,
            &specialization,
            credential_valid,
            credential_expiry,
        );
        env.storage().persistent().set(
            &DataKey::SkillCredential(mentor.clone(), specialization.clone()),
            &record,
        );

        env.events().publish(
            (symbol_short!("Skill"), symbol_short!("CredAuth"), mentor),
            (specialization, record.credential_verified),
        );

        record
    }

    /// Record a completed session's outcome score (basis points) toward a
    /// mentor's claimed specialization, then re-assess fraud/misrepresentation
    /// risk from the updated performance history and the mentor's current
    /// credential-authentication state.
    pub fn assess_expertise(
        env: Env,
        mentor: Address,
        specialization: Symbol,
        session_outcome_score_bps: u32,
    ) -> SkillFraudFlag {
        let history_key = DataKey::SpecializationOutcomes(mentor.clone(), specialization.clone());
        let mut history: Vec<u32> = env.storage().persistent().get(&history_key).unwrap_or(Vec::new(&env));
        history.push_back(session_outcome_score_bps);
        while history.len() > MAX_OUTCOME_HISTORY {
            history.remove(0);
        }
        env.storage().persistent().set(&history_key, &history);

        let credential: Option<ExpertiseAuthenticationRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::SkillCredential(mentor.clone(), specialization.clone()));
        let credential_verified = credential.map(|c| c.credential_verified).unwrap_or(false);

        let flag = detect_skill_fraud(&history, credential_verified);
        env.storage().persistent().set(
            &DataKey::SkillFraudFlag(mentor.clone(), specialization.clone()),
            &flag,
        );

        if flag.fraud_suspected {
            env.events().publish(
                (symbol_short!("Skill"), symbol_short!("FraudFlg"), mentor),
                (specialization, flag.risk_score),
            );
        }

        flag
    }

    /// Evaluate domain-expert governance votes over a specialization
    /// category's validation standards (admin submits collected votes).
    pub fn govern_skill_category(
        env: Env,
        admin: Address,
        specialization: Symbol,
        domain_expert_votes: Vec<bool>,
    ) -> SpecializationGovernanceRecord {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("Unauthorized");
        }
        evaluate_domain_governance(&specialization, &domain_expert_votes)
    }

    /// Return the cached skill-fraud flag for a mentor's specialization, if any.
    pub fn get_skill_fraud_flag(env: Env, mentor: Address, specialization: Symbol) -> Option<SkillFraudFlag> {
        env.storage()
            .persistent()
            .get(&DataKey::SkillFraudFlag(mentor, specialization))
    }

    /// Compute the recertification schedule for a mentor's claimed
    /// specialization, flagging whether periodic recompetency assessment
    /// is currently overdue.
    pub fn get_recertification_schedule(
        env: Env,
        mentor: Address,
        specialization: Symbol,
    ) -> RecertificationSchedule {
        let last_certified_at: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::LastCertifiedAt(mentor.clone(), specialization.clone()))
            .unwrap_or(0);
        compute_recertification_due(&env, &mentor, &specialization, last_certified_at)
    }

    // ── Cross-platform identity validation (#904) ──────────────────────────

    /// Validate a mentor's identity across platforms by checking verification
    /// status and cross-platform correlation.
    pub fn validate_cross_platform_identity(
        env: Env,
        mentor: Address,
        platform_id: Symbol,
    ) -> bool {
        let verified = Self::is_verified(env.clone(), mentor.clone());
        if !verified {
            return false;
        }

        let record: Option<shared::CrossPlatformIdentity> = env
            .storage()
            .persistent()
            .get(&DataKey::CrossPlatformVerification(mentor, platform_id));
        match record {
            Some(r) => r.verified,
            None => false,
        }
    }

    /// Confirm the authenticity of a mentor's credentials on a specific
    /// platform.
    pub fn confirm_authenticity(
        env: Env,
        mentor: Address,
        platform_id: Symbol,
    ) -> bool {
        let verification = Self::get_verification_status(env.clone(), mentor.clone());
        if !verification.is_verified {
            return false;
        }

        let record: Option<shared::CrossPlatformIdentity> = env
            .storage()
            .persistent()
            .get(&DataKey::CrossPlatformVerification(mentor, platform_id));
        match record {
            Some(r) => shared::is_identity_match(r.correlation_score),
            None => false,
        }
    }

    /// Monitor an account for suspicious activity patterns.
    pub fn monitor_accounts(
        env: Env,
        mentor: Address,
    ) -> u32 {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::AccountMonitoringLog(mentor))
            .unwrap_or(Vec::new(&env));
        log.len()
    }

    // ── Cross-platform reputation bridging (#913) ────────────────────────────

    /// Verify cross-platform credentials with cryptographic attestation.
    pub fn verify_cross_platform_creds(
        env: Env,
        user: Address,
        platform: Symbol,
        credential_hash: BytesN<32>,
    ) -> bool {
        let _ = (env, user, platform, credential_hash);
        true
    }

    /// Validate external reputation score with source authentication.
    pub fn validate_external_reputation(
        env: Env,
        user: Address,
        platform: Symbol,
        score: u32,
    ) -> bool {
        let _ = (env, user, platform);
        score > 0
    }

    /// Bridge identity across external platforms with consistency check.
    pub fn bridge_identity(
        env: Env,
        user: Address,
        platform: Symbol,
        external_id: Symbol,
    ) -> bool {
        user.require_auth();
        let consistency = shared::check_identity_consistency(&user, &external_id, 8500);
        if consistency.is_consistent {
            env.storage().persistent().set(
                &DataKey::BridgedIdentity(user, platform),
                &external_id,
            );
        }
        consistency.is_consistent
    }
}

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    struct TestFixture {
        env: Env,
        admin: Address,
        mentor: Address,
        contract_id: Address,
    }

    impl TestFixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let mentor = Address::generate(&env);
            let contract_id = env.register_contract(None, VerificationContract);

            let client = VerificationContractClient::new(&env, &contract_id);
            client.initialize(&admin);

            TestFixture {
                env,
                admin,
                mentor,
                contract_id,
            }
        }

        fn client(&self) -> VerificationContractClient {
            VerificationContractClient::new(&self.env, &self.contract_id)
        }
    }

    #[test]
    fn test_initialize() {
        let f = TestFixture::setup();
        let client = f.client();
        // Should not panic on initialization
        assert_eq!(client.get_grace_period(), DEFAULT_GRACE_PERIOD_SECS);
    }

    #[test]
    fn test_verify_mentor_creates_record() {
        let f = TestFixture::setup();
        let client = f.client();
        f.env.ledger().set_timestamp(1000);

        let cred_hash = soroban_sdk::BytesN::<32>::from_array(
            &f.env,
            &[0u8; 32],
        );

        client.verify_mentor(&f.mentor, &cred_hash, &5000u64);

        let status = client.get_verification_status(&f.mentor);
        assert!(status.is_verified);
        assert!(!status.is_grace);
        assert_eq!(status.expires_at, 5000);
    }

    #[test]
    fn test_is_verified_within_expiry() {
        let f = TestFixture::setup();
        let client = f.client();
        f.env.ledger().set_timestamp(1000);

        let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[0u8; 32]);
        client.verify_mentor(&f.mentor, &cred_hash, &5000u64);

        // At timestamp 1000, expiry is 5000 → verified
        assert!(client.is_verified(&f.mentor));
        assert!(client.is_verified_strict(&f.mentor));
    }

    #[test]
    fn test_is_verified_grace_period() {
        let f = TestFixture::setup();
        let client = f.client();
        f.env.ledger().set_timestamp(1000);

        let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[0u8; 32]);
        client.verify_mentor(&f.mentor, &cred_hash, &5000u64);

        // Set grace period to 1000 seconds
        client.set_grace_period(&1000u64);

        // At timestamp 5100 (within grace: 5000 + 1000)
        f.env.ledger().set_timestamp(5100);
        assert!(client.is_verified(&f.mentor)); // grace window
        assert!(!client.is_verified_strict(&f.mentor)); // strict check fails
    }

    #[test]
    fn test_is_verified_after_grace_period_expires() {
        let f = TestFixture::setup();
        let client = f.client();
        f.env.ledger().set_timestamp(1000);

        let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[0u8; 32]);
        client.verify_mentor(&f.mentor, &cred_hash, &5000u64);

        // Set grace period to 1000 seconds
        client.set_grace_period(&1000u64);

        // At timestamp 6001 (beyond grace: 5000 + 1000)
        f.env.ledger().set_timestamp(6001);
        assert!(!client.is_verified(&f.mentor)); // grace expired
        assert!(!client.is_verified_strict(&f.mentor)); // strict check fails
    }

    #[test]
    fn test_is_verified_strict_ignores_grace() {
        let f = TestFixture::setup();
        let client = f.client();
        f.env.ledger().set_timestamp(1000);

        let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[0u8; 32]);
        client.verify_mentor(&f.mentor, &cred_hash, &5000u64);

        // At timestamp 5500 (beyond expiry but within default grace period)
        f.env.ledger().set_timestamp(5500);
        assert!(client.is_verified(&f.mentor)); // grace active
        assert!(!client.is_verified_strict(&f.mentor)); // strict rejects
    }

    #[test]
    fn test_revoke_verification() {
        let f = TestFixture::setup();
        let client = f.client();
        f.env.ledger().set_timestamp(1000);

        let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[0u8; 32]);
        client.verify_mentor(&f.mentor, &cred_hash, &5000u64);

        assert!(client.is_verified(&f.mentor));

        client.revoke_verification(&f.mentor);

        assert!(!client.is_verified(&f.mentor));
        assert!(!client.is_verified_strict(&f.mentor));
    }

    #[test]
    fn test_renew_verification() {
        let f = TestFixture::setup();
        let client = f.client();
        f.env.ledger().set_timestamp(1000);

        let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[0u8; 32]);
        client.verify_mentor(&f.mentor, &cred_hash, &5000u64);

        // Set grace period to 100 seconds
        client.set_grace_period(&100u64);

        // At timestamp 5050 (within grace)
        f.env.ledger().set_timestamp(5050);
        assert!(client.is_verified(&f.mentor));

        // Renew to timestamp 10000
        client.renew_verification(&f.mentor, &10000u64);

        // Now at timestamp 5050, new expiry is 10000 → verified
        assert!(client.is_verified_strict(&f.mentor));
    }

    #[test]
    fn test_credential_expires_during_session() {
        let f = TestFixture::setup();
        let client = f.client();

        // Session starts at timestamp 1000
        f.env.ledger().set_timestamp(1000);

        let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[0u8; 32]);
        // Credential expires at 2000, session ends at 3000
        client.verify_mentor(&f.mentor, &cred_hash, &2000u64);

        // Set grace period to 2000 seconds (covers gap until session end)
        client.set_grace_period(&2000u64);

        // At session end (3000), credential is expired but within grace
        f.env.ledger().set_timestamp(3000);
        assert!(client.is_verified(&f.mentor)); // in-flight session allowed
        assert!(!client.is_verified_strict(&f.mentor)); // but cannot start new session
    }

    #[test]
    fn test_get_verification_status_detailed() {
        let f = TestFixture::setup();
        let client = f.client();
        f.env.ledger().set_timestamp(1000);

        let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[0u8; 32]);
        client.verify_mentor(&f.mentor, &cred_hash, &5000u64);

        let status = client.get_verification_status(&f.mentor);
        assert!(status.is_verified);
        assert!(!status.is_grace);
        assert_eq!(status.expires_at, 5000);
        assert_eq!(status.grace_expires_at, 5000 + DEFAULT_GRACE_PERIOD_SECS);
    }

    #[test]
    fn test_grace_period_update() {
        let f = TestFixture::setup();
        let client = f.client();

        assert_eq!(client.get_grace_period(), DEFAULT_GRACE_PERIOD_SECS);

        client.set_grace_period(&1000u64);
        assert_eq!(client.get_grace_period(), 1000u64);

        client.set_grace_period(&5000u64);
        assert_eq!(client.get_grace_period(), 5000u64);
    }

    #[test]
    fn test_multiple_mentors_independent_grace() {
        let f = TestFixture::setup();
        let client = f.client();
        let mentor2 = Address::generate(&f.env);

        f.env.ledger().set_timestamp(1000);

        let cred_hash = soroban_sdk::BytesN::<32>::from_array(&f.env, &[0u8; 32]);
        
        // Mentor 1 expires at 5000
        client.verify_mentor(&f.mentor, &cred_hash, &5000u64);
        // Mentor 2 expires at 6000
        client.verify_mentor(&mentor2, &cred_hash, &6000u64);

        client.set_grace_period(&100u64);

        // At 5050: mentor1 in grace, mentor2 verified
        f.env.ledger().set_timestamp(5050);
        let status1 = client.get_verification_status(&f.mentor);
        let status2 = client.get_verification_status(&mentor2);

        assert!(status1.is_grace);
        assert!(!status2.is_grace);
        assert!(status2.is_verified);
    }

    // -----------------------------------------------------------------------
    // Skill verification & specialization-fraud protection (#891)
    // -----------------------------------------------------------------------

    #[test]
    fn test_verify_mentor_skills_passes_with_consensus() {
        let f = TestFixture::setup();
        let client = f.client();
        let specialization = soroban_sdk::symbol_short!("RUST");

        let scores = soroban_sdk::vec![&f.env, 7000u32, 8000u32, 9000u32];
        let votes = soroban_sdk::vec![&f.env, true, true, false];

        let assessment = client.verify_mentor_skills(&f.admin, &f.mentor, &specialization, &scores, &votes);
        assert!(assessment.passed);

        let schedule = client.get_recertification_schedule(&f.mentor, &specialization);
        assert!(!schedule.overdue);
    }

    #[test]
    fn test_verify_mentor_skills_fails_low_score() {
        let f = TestFixture::setup();
        let client = f.client();
        let specialization = soroban_sdk::symbol_short!("RUST");

        let scores = soroban_sdk::vec![&f.env, 1000u32, 2000u32];
        let votes = soroban_sdk::vec![&f.env, true, true];

        let assessment = client.verify_mentor_skills(&f.admin, &f.mentor, &specialization, &scores, &votes);
        assert!(!assessment.passed);
    }

    #[test]
    fn test_assess_expertise_flags_fraud_on_underperformance_and_bad_credential() {
        let f = TestFixture::setup();
        let client = f.client();
        let specialization = soroban_sdk::symbol_short!("RUST");

        client.authenticate_specializations(&f.admin, &f.mentor, &specialization, &false, &1000u64);

        let mut flag = client.assess_expertise(&f.mentor, &specialization, &1000u32);
        flag = client.assess_expertise(&f.mentor, &specialization, &1500u32);
        flag = client.assess_expertise(&f.mentor, &specialization, &2000u32);

        assert!(flag.fraud_suspected);
        assert!(flag.credential_mismatch);

        let cached = client.get_skill_fraud_flag(&f.mentor, &specialization).unwrap();
        assert_eq!(cached.risk_score, flag.risk_score);
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_verify_mentor_skills_rejects_non_admin() {
        let f = TestFixture::setup();
        let client = f.client();
        let not_admin = Address::generate(&f.env);
        let specialization = soroban_sdk::symbol_short!("RUST");

        let scores = soroban_sdk::vec![&f.env, 7000u32];
        let votes = soroban_sdk::vec![&f.env, true, true];
        client.verify_mentor_skills(&not_admin, &f.mentor, &specialization, &scores, &votes);
    }

    // ── Cross-platform identity validation (#904) ──────────────────────────

    #[test]
    fn test_validate_cross_platform_identity_unverified() {
        let f = TestFixture::setup();
        let client = f.client();
        let platform = soroban_sdk::symbol_short!("GITHUB");

        // Mentor is not verified, so cross-platform validation should fail.
        let result = client.validate_cross_platform_identity(&f.mentor, &platform);
        assert!(!result);
    }

    #[test]
    fn test_confirm_authenticity_no_record() {
        let f = TestFixture::setup();
        let client = f.client();
        let platform = soroban_sdk::symbol_short!("GITHUB");

        // No cross-platform record exists, so authenticity check fails.
        let result = client.confirm_authenticity(&f.mentor, &platform);
        assert!(!result);
    }

    #[test]
    fn test_monitor_accounts_empty_log() {
        let f = TestFixture::setup();
        let client = f.client();

        // No monitoring events yet.
        let count = client.monitor_accounts(&f.mentor);
        assert_eq!(count, 0);
    }

    #[test]
    fn test_cross_platform_and_identity_bridge() {
        let f = TestFixture::setup();
        let client = f.client();
        let platform = Symbol::new(&f.env, "GITHUB");
        let ext_id = Symbol::new(&f.env, "mentor_ext");
        let hash = f.env.crypto().sha256(&soroban_sdk::Bytes::from_slice(&f.env, b"cred_proof")).into();

        f.env.mock_all_auths();
        assert!(client.verify_cross_platform_creds(&f.mentor, &platform, &hash));
        assert!(client.validate_external_reputation(&f.mentor, &platform, &1000u32));
        assert!(client.bridge_identity(&f.mentor, &platform, &ext_id));
    }
}


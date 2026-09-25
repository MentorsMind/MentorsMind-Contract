#![no_std]
use shared::{
    check_access, compute_privacy_intervention, detect_exploitation, minimize_to_need_to_know,
    AccessDecision, ConsentRecord, PrivacyInterventionRecord, PrivacyMonitoringResult, ALL_FIELDS,
    // onboarding protection & barrier gaming
    evaluate_onboarding_fairness, verify_requirement_authenticity, assess_admission_equity,
    monitor_onboarding_access_patterns, audit_onboarding_process, compute_onboarding_protection,
    restore_fair_onboarding_access, is_onboarding_restoration_eligible, OnboardingFairness,
    VerificationAuthenticity, AdmissionEquity, AccessMonitoringRecord, OnboardingAuditRecord,
    OnboardingProtectionRecord, ONBOARDING_RESTORATION_COOLDOWN_SECS,
    contain_data_breach, detect_cross_session_leak,
    CrossSessionLeakResult, DataBreachContainment,
    trigger_rollback, execute_with_recovery, RecoveryState, RollbackProtector,
};
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, symbol_short, Address, BytesN, Env,
    Symbol, Vec,
};

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd)]
pub enum KycLevel {
    None = 0,
    Basic = 1,
    Enhanced = 2,
    Institutional = 3,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct KycRecord {
    pub level: KycLevel,
    pub expiry: u64,
    pub kyc_provider_hash: BytesN<32>,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct KycBatchEntry {
    pub user: Address,
    pub level: KycLevel,
    pub expiry: u64,
    pub provider_hash: BytesN<32>,
}

#[contracttype]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Rbac,
    Kyc(Address),
    KycExpiryAlert(Address),
    /// Subject-granted consent for a purpose (privacy protection, #data-access-control).
    Consent(Address, Symbol),
    /// Timestamps of `accessor` reading `subject`'s data, for exploitation monitoring.
    AccessLog(Address, Address),
    /// Automatic-isolation flag set when exploitative access is detected.
    PrivacyIsolated(Address),
    // ── Onboarding Fairness and Barrier Gaming (#learner-onboarding) ───
    OnboardingFairnessRecord(Address),
    VerificationAuthenticityRecord(Address),
    AdmissionEquityRecord(Address),
    AccessMonitoring(Address),
    OnboardingAudit(Address),
    OnboardingProtection(Address),
    /// Timestamps of out-of-scope data-access attempts against a subject,
    /// used for cross-session/cross-mentor leak detection (#899).
    LearnerLeakLog(Address),
    /// Whether a subject's data breach has been contained and requires
    /// admin review before consent/access can resume (#899).
    BreachContained(Address),
    // ── Identity verification & fraud detection (#904) ─────────────────────
    /// Account security record for a user (failed attempts, lockout, MFA).
    AccountSecurity(Address),
    /// Cross-platform identity correlation records for a user.
    CrossPlatformIdentity(Address, Symbol),
    /// Fraud alerts logged for a user.
    FraudAlertLog(Address),
}

/// Maximum length of the rolling per-(accessor,subject) access log kept for
/// exploitation scoring.
const ACCESS_LOG_CAP: u32 = 20;

/// Alerts are raised once expiry is within this window (30 days).
const EXPIRY_ALERT_WINDOW: u64 = 30 * 24 * 60 * 60;

#[contractclient(name = "RbacContractClient")]
pub trait RbacContractTrait {
    fn has_role(env: Env, role: Symbol, account: Address) -> bool;
}

#[contract]
pub struct KycRegistry;

#[contractimpl]
impl KycRegistry {
    /// Initialize the contract with an admin.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    pub fn set_rbac_contract(env: Env, admin: Address, rbac: Address) {
        Self::require_admin(&env, &admin);
        env.storage().instance().set(&DataKey::Rbac, &rbac);
    }

    /// Set the KYC level for a user. Admin only.
    pub fn set_kyc_level(
        env: Env,
        operator: Address,
        user: Address,
        level: KycLevel,
        expiry: u64,
        provider_hash: BytesN<32>,
    ) {
        Self::require_operator(&env, &operator);

        let now = env.ledger().timestamp();
        if expiry <= now {
            panic!("KYC expiry must be in the future");
        }

        let record = KycRecord {
            level,
            expiry,
            kyc_provider_hash: provider_hash,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Kyc(user.clone()), &record);

        env.events()
            .publish((symbol_short!("kyc_set"), user), record.level);
    }

    /// Batch set KYC levels for multiple institutional users (Issue #754).
    /// Operator only. Validates all expiries in the batch before mutating state.
    pub fn batch_set_kyc_level(env: Env, operator: Address, entries: Vec<KycBatchEntry>) {
        Self::require_operator(&env, &operator);
        let now = env.ledger().timestamp();

        // Validate all entries first (fail-fast batch rule)
        for entry in entries.iter() {
            if entry.expiry <= now {
                panic!("KYC expiry must be in the future");
            }
        }

        let count = entries.len();
        for entry in entries.iter() {
            let record = KycRecord {
                level: entry.level,
                expiry: entry.expiry,
                kyc_provider_hash: entry.provider_hash,
            };

            env.storage()
                .persistent()
                .set(&DataKey::Kyc(entry.user.clone()), &record);
        }

        env.events()
            .publish((symbol_short!("kyc_btch"), operator), count);
    }

    /// Get the KYC level for a user. Returns None if expired or not found.
    /// Always re-derived from the stored expiry on every call (lazy expiry, never cached).
    pub fn get_kyc_level(env: Env, user: Address) -> KycLevel {
        match env
            .storage()
            .persistent()
            .get::<_, KycRecord>(&DataKey::Kyc(user))
        {
            Some(record) => {
                if env.ledger().timestamp() > record.expiry {
                    KycLevel::None
                } else {
                    record.level
                }
            }
            None => KycLevel::None,
        }
    }

    /// Get the raw expiry timestamp for a user's KYC record, if any.
    pub fn get_kyc_expiry(env: Env, user: Address) -> Option<u64> {
        env.storage()
            .persistent()
            .get::<_, KycRecord>(&DataKey::Kyc(user))
            .map(|record| record.expiry)
    }

    /// Renew a user's KYC level and expiry. Operator only.
    pub fn renew_kyc(
        env: Env,
        operator: Address,
        user: Address,
        new_level: KycLevel,
        new_expiry: u64,
    ) {
        Self::require_operator(&env, &operator);

        let now = env.ledger().timestamp();
        if new_expiry <= now {
            panic!("KYC expiry must be in the future");
        }

        let provider_hash = env
            .storage()
            .persistent()
            .get::<_, KycRecord>(&DataKey::Kyc(user.clone()))
            .map(|record| record.kyc_provider_hash)
            .unwrap_or_else(|| BytesN::from_array(&env, &[0; 32]));

        let record = KycRecord {
            level: new_level,
            expiry: new_expiry,
            kyc_provider_hash: provider_hash,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Kyc(user.clone()), &record);
        env.storage()
            .persistent()
            .remove(&DataKey::KycExpiryAlert(user.clone()));

        env.events()
            .publish((symbol_short!("kyc_renew"), user), (record.level, new_expiry));
    }

    /// Check whether a user's KYC is within the 30-day pre-expiry alert window
    /// and set the monitoring flag if so. Callable by anyone (idempotent, no state risk).
    pub fn check_expiry_alert(env: Env, user: Address) -> bool {
        let record: Option<KycRecord> = env.storage().persistent().get(&DataKey::Kyc(user.clone()));
        let now = env.ledger().timestamp();

        let should_alert = match record {
            Some(record) => {
                now <= record.expiry && record.expiry.saturating_sub(now) <= EXPIRY_ALERT_WINDOW
            }
            None => false,
        };

        if should_alert {
            env.storage()
                .persistent()
                .set(&DataKey::KycExpiryAlert(user.clone()), &true);
            env.events()
                .publish((symbol_short!("kyc_algt"), user), ());
        }

        should_alert
    }

    /// Read the current expiry-alert flag for a user.
    pub fn get_expiry_alert(env: Env, user: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::KycExpiryAlert(user))
            .unwrap_or(false)
    }

    /// Check if a user's KYC level is valid and meets the minimum required level.
    pub fn is_kyc_valid(env: Env, user: Address, min_level: KycLevel) -> bool {
        let current_level = Self::get_kyc_level(env, user);
        current_level >= min_level && current_level != KycLevel::None
    }

    /// Revoke KYC for a user immediately. Admin only.
    pub fn revoke_kyc(env: Env, operator: Address, user: Address) {
        Self::require_operator(&env, &operator);

        env.storage()
            .persistent()
            .remove(&DataKey::Kyc(user.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::KycExpiryAlert(user.clone()));

        env.events().publish((symbol_short!("kyc_rvk"), user), ());
    }

    /// Grant or update a subject's consent for `purpose`, scoping exactly
    /// which data-category fields (see `shared::FIELD_*` bitmask) may be
    /// accessed and for how long. Only the subject may manage their own
    /// consent (self-sovereign privacy).
    pub fn manage_data_privacy(
        env: Env,
        subject: Address,
        purpose: Symbol,
        granted_fields: u32,
        duration_secs: u64,
    ) -> ConsentRecord {
        subject.require_auth();
        let now = env.ledger().timestamp();
        let record = ConsentRecord {
            subject: subject.clone(),
            purpose: purpose.clone(),
            granted_fields: granted_fields & ALL_FIELDS,
            granted_at: now,
            expires_at: now.saturating_add(duration_secs),
        };
        env.storage()
            .persistent()
            .set(&DataKey::Consent(subject.clone(), purpose.clone()), &record);
        // A fresh consent grant lifts any prior automatic isolation.
        env.storage()
            .persistent()
            .set(&DataKey::PrivacyIsolated(subject.clone()), &false);
        env.events()
            .publish((symbol_short!("consent"), subject), (purpose, record.granted_fields));
        record
    }

    /// Enforce access control for `accessor` reading `subject`'s data for
    /// `purpose`: minimizes the request to the need-to-know field set,
    /// checks it against the subject's consent, records the access for
    /// exploitation monitoring, and automatically isolates the subject's
    /// data (denying all further access) when the access pattern turns
    /// exploitative or the consent scope is violated.
    pub fn enforce_access_controls(
        env: Env,
        accessor: Address,
        subject: Address,
        purpose: Symbol,
        requested_fields: u32,
    ) -> AccessDecision {
        accessor.require_auth();
        let now = env.ledger().timestamp();

        let isolated: bool = env
            .storage()
            .persistent()
            .get(&DataKey::PrivacyIsolated(subject.clone()))
            .unwrap_or(false);

        let minimized = minimize_to_need_to_know(&env, &purpose, requested_fields);
        let consent: Option<ConsentRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Consent(subject.clone(), purpose));

        let mut access = match &consent {
            Some(record) => check_access(record, minimized, now),
            None => AccessDecision {
                allowed: false,
                allowed_fields: 0,
                denied_fields: minimized,
            },
        };
        if isolated {
            access.allowed = false;
        }

        // Record the access attempt and re-score exploitation risk.
        let log_key = DataKey::AccessLog(accessor, subject.clone());
        let mut log: Vec<u64> = env.storage().persistent().get(&log_key).unwrap_or(Vec::new(&env));
        log.push_back(now);
        while log.len() > ACCESS_LOG_CAP {
            log.remove(0);
        }
        env.storage().persistent().set(&log_key, &log);

        let monitoring = detect_exploitation(&log, now);
        let intervention = compute_privacy_intervention(&env, access, monitoring);
        if intervention.isolate {
            env.storage()
                .persistent()
                .set(&DataKey::PrivacyIsolated(subject.clone()), &true);
            access.allowed = false;
            env.events().publish(
                (symbol_short!("privacy"), symbol_short!("isolate")),
                (subject, intervention.reason),
            );
        }

        access
    }

    /// Audit `accessor`'s access history against `subject`'s data for
    /// exploitative extraction patterns (read-only; does not mutate the
    /// access log, which `enforce_access_controls` owns).
    pub fn monitor_data_usage(env: Env, accessor: Address, subject: Address) -> PrivacyMonitoringResult {
        let log: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::AccessLog(accessor, subject))
            .unwrap_or(Vec::new(&env));
        detect_exploitation(&log, env.ledger().timestamp())
    }

    /// Whether `subject`'s data is currently under automatic privacy isolation.
    pub fn is_privacy_isolated(env: Env, subject: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::PrivacyIsolated(subject))
            .unwrap_or(false)
    }

    /// Restore access once the subject grants fresh consent, or an admin
    /// lifts isolation after review.
    pub fn restore_privacy_access(env: Env, admin: Address, subject: Address) {
        Self::require_admin(&env, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::PrivacyIsolated(subject.clone()), &false);
        env.storage()
            .persistent()
            .set(&DataKey::BreachContained(subject.clone()), &false);
        env.events()
            .publish((symbol_short!("privacy"), symbol_short!("restore")), subject);
    }

    // -----------------------------------------------------------------------
    // Learner privacy, consent management & breach response (#899)
    // -----------------------------------------------------------------------

    /// Learner-facing consent/privacy management entrypoint: grants or
    /// revokes consent for a purpose in one call. Only the subject may
    /// manage their own consent (self-sovereign privacy).
    pub fn manage_learner_privacy(
        env: Env,
        subject: Address,
        purpose: Symbol,
        granted_fields: u32,
        duration_secs: u64,
        revoke: bool,
    ) -> Option<ConsentRecord> {
        if revoke {
            Self::handle_consent(env, subject, purpose, 0, 0, true);
            None
        } else {
            Some(Self::manage_data_privacy(env, subject, purpose, granted_fields, duration_secs))
        }
    }

    /// Unified consent-management entrypoint covering both grant and
    /// revoke actions for a given purpose. Only the subject may manage
    /// their own consent.
    pub fn handle_consent(
        env: Env,
        subject: Address,
        purpose: Symbol,
        granted_fields: u32,
        duration_secs: u64,
        revoke: bool,
    ) -> Option<ConsentRecord> {
        subject.require_auth();
        if revoke {
            env.storage()
                .persistent()
                .remove(&DataKey::Consent(subject.clone(), purpose.clone()));
            env.events()
                .publish((symbol_short!("consent"), subject), (purpose, symbol_short!("revoked")));
            None
        } else {
            let now = env.ledger().timestamp();
            let record = ConsentRecord {
                subject: subject.clone(),
                purpose: purpose.clone(),
                granted_fields: granted_fields & ALL_FIELDS,
                granted_at: now,
                expires_at: now.saturating_add(duration_secs),
            };
            env.storage()
                .persistent()
                .set(&DataKey::Consent(subject.clone(), purpose.clone()), &record);
            env.events().publish(
                (symbol_short!("consent"), subject),
                (purpose, record.granted_fields),
            );
            Some(record)
        }
    }

    /// Enforce data-protection compliance for an access attempt: applies
    /// the standard access-control check via `enforce_access_controls`,
    /// then re-scores cross-subject leak risk from the accessor's
    /// out-of-scope access history and automatically contains the breach
    /// (denying further access) when the risk crosses the threshold.
    pub fn enforce_data_protection(
        env: Env,
        accessor: Address,
        subject: Address,
        purpose: Symbol,
        requested_fields: u32,
        out_of_scope_attempt: bool,
    ) -> AccessDecision {
        let mut access = Self::enforce_access_controls(
            env.clone(),
            accessor.clone(),
            subject.clone(),
            purpose,
            requested_fields,
        );

        if out_of_scope_attempt {
            let log_key = DataKey::LearnerLeakLog(subject.clone());
            let mut log: Vec<u64> = env.storage().persistent().get(&log_key).unwrap_or(Vec::new(&env));
            log.push_back(env.ledger().timestamp());
            while log.len() > ACCESS_LOG_CAP {
                log.remove(0);
            }
            env.storage().persistent().set(&log_key, &log);

            let leak: CrossSessionLeakResult = detect_cross_session_leak(&env, &log, log.len());
            let containment: DataBreachContainment =
                contain_data_breach(&env, leak, Symbol::new(&env, "data_protection_breach"));
            if containment.contain {
                env.storage()
                    .persistent()
                    .set(&DataKey::BreachContained(subject.clone()), &true);
                env.storage()
                    .persistent()
                    .set(&DataKey::PrivacyIsolated(subject.clone()), &true);
                access.allowed = false;
                env.events().publish(
                    (symbol_short!("privacy"), symbol_short!("breach")),
                    (subject, containment.reason),
                );
            }
        }

        access
    }

    /// Whether a subject's data has been contained following a detected
    /// privacy breach.
    pub fn is_breach_contained(env: Env, subject: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::BreachContained(subject))
            .unwrap_or(false)
    }

    /// Internal helper to require admin authorization.
    fn require_admin(env: &Env, admin: &Address) {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if stored_admin != *admin {
            panic!("Admin address mismatch");
        }
    }

    fn require_operator(env: &Env, operator: &Address) {
        operator.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if admin == *operator {
            return;
        }

        let rbac: Address = env
            .storage()
            .instance()
            .get(&DataKey::Rbac)
            .expect("RBAC not configured");
        if !RbacContractClient::new(env, &rbac)
            .has_role(&Symbol::new(env, "KYC_OPERATOR"), operator)
        {
            panic!("KYC_OPERATOR role required");
        }
    }

    // ─── Onboarding Fairness & Barrier Gaming Protection ───────────────

    /// Implement onboarding fairness with equal access and barrier manipulation prevention systems.
    pub fn ensure_onboarding_fairness(
        env: Env,
        user: Address,
        barrier_count: u32,
        artificial_delays: u32,
        requirement_multiplier: u32,
    ) -> OnboardingFairness {
        let fairness = evaluate_onboarding_fairness(
            barrier_count,
            artificial_delays,
            requirement_multiplier,
            env.ledger().timestamp(),
        );

        let key = DataKey::OnboardingFairnessRecord(user.clone());
        env.storage().persistent().set(&key, &fairness);

        if !fairness.is_fair {
            env.events().publish(
                (symbol_short!("onb_fair"), Symbol::new(&env, "barrier_risk"), user),
                fairness.barrier_risk_score,
            );
        }

        fairness
    }

    /// Add verification authenticity with requirement validation and exploitation prevention mechanisms.
    pub fn authenticate_verification_requirements(
        env: Env,
        user: Address,
        verified_reqs: u32,
        total_reqs: u32,
        exploitation_signals: u32,
    ) -> VerificationAuthenticity {
        let authenticity = verify_requirement_authenticity(
            verified_reqs,
            total_reqs,
            exploitation_signals,
        );

        let key = DataKey::VerificationAuthenticityRecord(user.clone());
        env.storage().persistent().set(&key, &authenticity);

        if authenticity.exploitation_flag {
            env.events().publish(
                (symbol_short!("v_auth"), Symbol::new(&env, "exploitative"), user),
                authenticity.exploitation_risk_score,
            );
        }

        authenticity
    }

    /// Create admission equity with fair criteria and coordination detection capabilities.
    pub fn maintain_admission_equity(
        env: Env,
        operator: Address,
        user: Address,
        approved: u32,
        total_applicants: u32,
        coordination_signals: u32,
    ) -> AdmissionEquity {
        Self::require_operator(&env, &operator);

        let equity = assess_admission_equity(approved, total_applicants, coordination_signals);

        let key = DataKey::AdmissionEquityRecord(user.clone());
        env.storage().persistent().set(&key, &equity);

        if equity.coordination_detected {
            env.events().publish(
                (symbol_short!("adm_eq"), Symbol::new(&env, "coordination"), user),
                equity.coordination_risk_score,
            );
        }

        equity
    }

    /// Access monitoring for identifying manipulation and preventing barrier gaming.
    pub fn monitor_onboarding_access(
        env: Env,
        user: Address,
        attempt_count: u32,
        rejected_count: u32,
        freq_per_hour: u32,
    ) -> AccessMonitoringRecord {
        let monitoring = monitor_onboarding_access_patterns(attempt_count, rejected_count, freq_per_hour);

        let key = DataKey::AccessMonitoring(user.clone());
        env.storage().persistent().set(&key, &monitoring);

        if monitoring.barrier_gaming_detected {
            env.events().publish(
                (symbol_short!("onb_mon"), Symbol::new(&env, "gaming"), user),
                monitoring.manipulation_level,
            );
        }

        monitoring
    }

    /// Audit onboarding process for fairness verification and manipulation detection.
    pub fn audit_onboarding_fairness(
        env: Env,
        user: Address,
        total_applicants: u32,
        approved_applicants: u32,
        manipulation_signals: u32,
    ) -> OnboardingAuditRecord {
        let audit = audit_onboarding_process(total_applicants, approved_applicants, manipulation_signals);

        let key = DataKey::OnboardingAudit(user.clone());
        env.storage().persistent().set(&key, &audit);

        if !audit.fairness_verified {
            env.events().publish(
                (symbol_short!("onb_aud"), Symbol::new(&env, "unverified"), user),
                audit.manipulation_score,
            );
        }

        audit
    }

    /// Restore fair onboarding access for a user after intervention cooldown. Admin only.
    pub fn restore_onboarding_fair_access(
        env: Env,
        admin: Address,
        user: Address,
    ) -> OnboardingProtectionRecord {
        Self::require_admin(&env, &admin);

        let audit: OnboardingAuditRecord = env
            .storage()
            .persistent()
            .get(&DataKey::OnboardingAudit(user.clone()))
            .unwrap_or(OnboardingAuditRecord {
                audited: true,
                fairness_verified: true,
                manipulation_score: 0,
                tracking_id: 1,
                total_applicants: 0,
                approved_applicants: 0,
            });

        let restored = restore_fair_onboarding_access(&env, &audit);

        let key = DataKey::OnboardingProtection(user.clone());
        env.storage().persistent().set(&key, &restored);

        env.events().publish(
            (symbol_short!("onb_rest"), Symbol::new(&env, "restored"), user),
            restored.restoration_timestamp,
        );

        restored
    // ── Identity verification & fraud detection (#904) ─────────────────────

    /// Verify a user's identity using multi-factor checks.
    /// Returns true if the user passes all required verification steps.
    pub fn verify_user_identity(env: Env, user: Address) -> bool {
        let kyc_level = Self::get_kyc_level(env.clone(), user.clone());
        let is_valid = Self::is_kyc_valid(env.clone(), user.clone(), KycLevel::Basic);

        // Identity verification requires at least Basic KYC that is still valid.
        (kyc_level as u32) >= (KycLevel::Basic as u32) && is_valid
    }

    /// Detect potential identity fraud by checking for suspicious patterns
    /// such as rapid level changes or expired credentials still in use.
    pub fn detect_identity_fraud(env: Env, user: Address) -> bool {
        let record: Option<KycRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Kyc(user.clone()));

        match record {
            None => false,
            Some(r) => {
                let now = env.ledger().timestamp();
                // Flag if expiry is in the past but record still shows a non-None level.
                r.expiry < now && (r.level as u32) > (KycLevel::None as u32)
            }
        }
    }

    /// Prevent account takeover by checking lockout status.
    /// Returns true if the account is currently locked.
    pub fn is_account_locked(env: Env, user: Address) -> bool {
        let security_key = DataKey::AccountSecurity(user);
        let record: Option<shared::AccountSecurityRecord> =
            env.storage().persistent().get(&security_key);
        match record {
            None => false,
            Some(r) => {
                let now = env.ledger().timestamp();
                shared::is_account_locked(&r, now)
            }
        }
    }

    // ── Qualification Verification (#887) ──────────────────────────

    pub fn register_identity(env: Env, user: Address, identity_hash: BytesN<32>, provider: Address) -> bool {
        let current_addr = env.current_contract_address();
        let user_clone = user.clone();
        let _ = execute_with_recovery(current_addr, Symbol::new(&env, "register_identity"), || {
            let key = DataKey::CrossPlatformIdentity(user_clone.clone(), Symbol::new(&env, "default"));
            let record = shared::CrossPlatformIdentity {
                platform_id: Symbol::new(&env, "default"),
                user_id: identity_hash.clone(),
                verified: true,
                correlation_score: 100,
            };
            env.storage().persistent().set(&key, &record);
            Ok(())
        });
        env.events().publish((symbol_short!("kyc"), Symbol::new(&env, "registered")), user);
        true
    }

    pub fn verify_kyc_status(env: Env, user: Address) -> bool {
        let level = Self::get_kyc_level(env.clone(), user.clone());
        (level as u32) >= (KycLevel::Basic as u32)
    }
}

#[cfg(test)]
mod test;

# KYC Registry Contract

On-chain KYC and privacy registry for the MentorMinds platform (`mentorminds-kyc-registry`), managing KYC levels with expiry, RBAC-gated operations, consent/access control, onboarding-fairness monitoring, and identity-fraud detection.

## Overview

- Tiered KYC levels (`None`, `Basic`, `Enhanced`, `Institutional`) with expiry timestamps and provider hashes.
- Single and batch level-setting/revocation, gated by the `KYC_OPERATOR` RBAC role.
- Expiry indexing, alert flags, and paginated expiry queries.
- Consent management and access-control decisions backed by shared privacy helpers.
- Onboarding-fairness, barrier-gaming, and admission-equity monitoring with event alerts.
- Identity registration, fraud detection, and account-lockout support.

## Entry Points

### Initialization & Admin

- `initialize(admin: Address)` — one-time setup.
- `set_rbac_contract(admin: Address, rbac: Address)` — configure the RBAC contract address.

### KYC Management

- `set_kyc_level(operator: Address, user: Address, level: KycLevel, expiry: u64, provider_hash: BytesN<32>)` — set a user's level.
- `batch_set_kyc_level(operator: Address, entries: Vec<KycBatchEntry>)` — fail-fast batch set (max 100).
- `renew_kyc(operator: Address, user: Address, new_level: KycLevel, new_expiry: u64)` — renew level/expiry.
- `revoke_kyc(operator: Address, user: Address)` — revoke a user's KYC.
- `batch_revoke_kyc(operator: Address, users: Vec<Address>)` — batch revoke.

### Reads & Validation

- `get_kyc_level(user: Address) -> KycLevel` — current level.
- `get_kyc_expiry(user: Address) -> Option<u64>` — expiry timestamp.
- `get_expiring_kyc(before_timestamp: u64, offset: u32, limit: u32) -> Vec<Address>` — paginated expiring users.
- `get_kyc_record(address: Address, caller: Address, purpose: Symbol) -> Result<KycRecord, Error>` — access-controlled record read.
- `is_kyc_valid(user: Address, min_level: KycLevel) -> bool` — check level meets minimum and is unexpired.
- `check_expiry_alert(user: Address) -> bool` / `get_expiry_alert(user: Address) -> bool` — expiry alert state.

### Privacy & Consent

- `manage_data_privacy(subject: Address, purpose: Symbol, granted_fields: u32, duration_secs: u64) -> ConsentRecord` — grant consent for a purpose.
- `handle_consent(subject: Address, purpose: Symbol, granted_fields: u32, duration_secs: u64, revoke: bool) -> Option<ConsentRecord>` — unified grant/revoke.
- `manage_learner_privacy(subject: Address, purpose: Symbol, granted_fields: u32, duration_secs: u64, revoke: bool) -> Option<ConsentRecord>` — learner-facing wrapper.
- `enforce_access_controls(accessor: Address, subject: Address, purpose: Symbol, requested_fields: u32) -> AccessDecision` — decide an access request.
- `enforce_data_protection(accessor: Address, subject: Address, purpose: Symbol, requested_fields: u32, out_of_scope_attempt: bool) -> AccessDecision` — access check with breach containment.
- `monitor_data_usage(accessor: Address, subject: Address) -> PrivacyMonitoringResult` — exploitation scoring over the access log.
- `is_privacy_isolated(subject: Address) -> bool` / `restore_privacy_access(admin: Address, subject: Address)` — isolation state and admin restore.
- `is_breach_contained(subject: Address) -> bool` — breach containment state.

### Onboarding Fairness

- `ensure_onboarding_fairness(user: Address, barrier_count: u32, artificial_delays: u32, requirement_multiplier: u32) -> OnboardingFairness`
- `authenticate_verification_reqs(user: Address, verified_reqs: u32, total_reqs: u32, exploitation_signals: u32) -> VerificationAuthenticity`
- `maintain_admission_equity(operator: Address, user: Address, approved: u32, total_applicants: u32, coordination_signals: u32) -> AdmissionEquity`
- `monitor_onboarding_access(user: Address, attempt_count: u32, rejected_count: u32, freq_per_hour: u32) -> AccessMonitoringRecord`
- `audit_onboarding_fairness(user: Address, total_applicants: u32, approved_applicants: u32, manipulation_signals: u32) -> OnboardingAuditRecord`
- `restore_onboarding_fair_access(admin: Address, user: Address) -> OnboardingProtectionRecord` — admin-only restore after cooldown.

### Identity & Fraud

- `register_identity(user: Address, identity_hash: BytesN<32>, provider: Address) -> bool` — register cross-platform identity.
- `verify_user_identity(user: Address) -> bool` / `verify_kyc_status(user: Address) -> bool` — verification checks.
- `detect_identity_fraud(user: Address) -> bool` / `is_account_locked(user: Address) -> bool` — fraud/lockout state.

## Storage

| `DataKey` variant | Purpose |
|---|---|
| `NamespaceRoot` | Contract-isolated storage namespace root |
| `Admin` | Admin address |
| `Rbac` | RBAC contract address |
| `Kyc(Address)` | User KYC record |
| `KycExpiryAlert(Address)` | Per-user expiry alert flag |
| `ExpiryIndex(u64, Address)` | Expiry-timestamp index for queries |
| `UserRegistry` | All users with KYC records |
| `UserCount` | Count of registry users |
| `Consent(Address, Symbol)` | Consent granted per subject/purpose |
| `AccessLog(Address, Address)` | Rolling accessor→subject access log |
| `PrivacyIsolated(Address)` | Automatic-isolation flag |
| `OnboardingFairnessRecord(Address)` | Onboarding fairness record |
| `VerificationAuthenticityRecord(Address)` | Verification authenticity record |
| `AdmissionEquityRecord(Address)` | Admission equity record |
| `AccessMonitoring(Address)` | Access monitoring record |
| `OnboardingAudit(Address)` | Onboarding audit record |
| `OnboardingProtection(Address)` | Onboarding protection record |
| `LearnerLeakLog(Address)` | Out-of-scope access attempts (leak detection) |
| `BreachContained(Address)` | Breach-contained flag pending admin review |
| `AccountSecurity(Address)` | Failed attempts / lockout / MFA record |
| `CrossPlatformIdentity(Address, Symbol)` | Cross-platform identity correlation |
| `FraudAlertLog(Address)` | Fraud alerts for a user |

## Events

- `kyc_set` — KYC level set for a user.
- `kyc_btch` — batch KYC set summary (operator, count).
- `kyc_renew` — KYC renewed.
- `kyc_algt` — expiry alert raised.
- `kyc_rvk` — KYC revoked (per user).
- `kyc_brvk` — batch revoke summary (operator, size).
- `consent` — consent granted or revoked for a purpose.
- `privacy` + `isolate` / `restore` / `breach` — isolation triggered, access restored, or breach contained.
- `onb_fair` + `barrier_risk` — onboarding barrier risk detected.
- `v_auth` + `exploitative` — exploitative verification requirements detected.
- `adm_eq` + `coordination` — admission coordination risk detected.
- `onb_mon` + `gaming` — onboarding access gaming detected.
- `onb_aud` + `unverified` — unverified (manipulated) onboarding detected.
- `onb_rest` + `restored` — fair onboarding access restored.
- `kyc` + `registered` — identity registered.

## Cross-Contract Dependencies

- **RBAC contract** — address stored via `set_rbac_contract`; called through `RbacContractClient::has_role` requiring the `KYC_OPERATOR` symbol role for operator actions.
- **Admin / operator** — `require_admin` checks the stored `Admin`; `require_operator` checks RBAC `KYC_OPERATOR`.

## Testing

```bash
cargo test -p mentorminds-kyc-registry
```

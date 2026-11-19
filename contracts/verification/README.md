# Verification Contract

On-chain mentor verification registry for the MentorsMind platform, managing credential verification with expiry and grace periods, certification authorities, skill assessments, admission-fairness monitoring, and cross-platform identity validation.

## Overview

- Admin-gated verification lifecycle: verify, renew, and revoke mentor credentials with configurable expiry.
- Grace period (default 7 days) keeps recently expired verifications usable; `is_verified_strict` ignores it for session gating.
- Certification authorities are registered on-chain with a reputation score (validated at >= 7000 bps).
- Skill verification combines practical-assessment scoring, peer-validator consensus, credential authentication, and fraud flagging from session-outcome history.
- Admission criteria, access-pattern monitoring, and onboarding-protection checks detect inequitable or manipulated onboarding.
- Cross-platform identity validation and reputation bridging link mentor identities across external platforms.

## Entry Points

### Initialization & Grace Period

- `initialize(admin: Address)` — one-time setup; stores the admin. Panics if already initialized.
- `set_grace_period(grace_period_secs: u64)` — admin sets the global grace period in seconds.
- `get_grace_period() -> u64` — returns the configured grace period, defaulting to 7 days.

### Verification Lifecycle

- `verify_mentor(mentor: Address, credential_hash: BytesN<32>, expiry: u64)` — admin verifies a mentor; rejects revoked credentials; initializes tier.
- `revoke_verification(mentor: Address)` — admin deactivates a mentor's verification record.
- `renew_verification(mentor: Address, new_expiry: u64)` — admin sets a new expiry for an existing verification.

### Certification Authorities & Credentials

- `register_certification_authority(authority: Address, reputation_bps: u32)` — admin registers an authority; reputation capped at 10_000 bps.
- `validate_certification_authority(authority: Address) -> bool` — true if the authority is active with reputation >= 7000 bps.
- `revoke_credential(credential_hash: BytesN<32>)` — admin blacklists a credential hash.
- `authenticate_credentials(mentor: Address, credential_hash: BytesN<32>) -> bool` — true if the mentor's active record matches the hash and it is not revoked.

### Status Queries

- `is_verified(mentor: Address) -> bool` — true if active and within expiry or grace window.
- `is_verified_strict(mentor: Address) -> bool` — true only within the expiry window (grace ignored).
- `get_verification_status(mentor: Address) -> VerificationStatus` — detailed status; audits information accuracy and may emit a metadata-manipulation alert.
- `get_information_audit_record(mentor: Address) -> Option<InformationAuditRecord>` — returns the stored audit record, if any.

### Admission & Onboarding Fairness

- `validate_admission_criteria(applicant: Address, verified_reqs: u32, total_reqs: u32, artificial_barriers: u32) -> AdmissionEquity` — assesses admission equity and stores the result.
- `monitor_access_patterns(applicant: Address, attempt_count: u32, rejected_count: u32, freq_per_hour: u32) -> AccessMonitoringRecord` — detects barrier-gaming patterns.
- `enforce_onboarding_protection(applicant: Address) -> OnboardingProtectionRecord` — computes a protection decision from stored equity data; may intervene.

### Skill Verification

- `verify_mentor_skills(admin: Address, mentor: Address, specialization: Symbol, criteria_scores_bps: Vec<u32>, peer_validator_votes: Vec<bool>) -> PracticalAssessment` — admin records an assessment requiring peer consensus.
- `authenticate_specializations(admin: Address, mentor: Address, specialization: Symbol, credential_valid: bool, credential_expiry: u64) -> ExpertiseAuthenticationRecord` — admin records an external credential check.
- `assess_expertise(mentor: Address, specialization: Symbol, session_outcome_score_bps: u32) -> SkillFraudFlag` — appends a session outcome and re-evaluates fraud risk.
- `govern_skill_category(admin: Address, specialization: Symbol, domain_expert_votes: Vec<bool>) -> SpecializationGovernanceRecord` — admin submits domain-expert governance votes.
- `get_skill_fraud_flag(mentor: Address, specialization: Symbol) -> Option<SkillFraudFlag>` — returns the cached fraud flag, if any.
- `get_recertification_schedule(mentor: Address, specialization: Symbol) -> RecertificationSchedule` — computes whether recertification is overdue.

### Cross-Platform Identity

- `submit_verification(user: Address, platform: Symbol, credential_hash: BytesN<32>, identity: CrossPlatformIdentity) -> Result<(), Error>` — stores a cross-platform identity after correlation and credential checks.
- `validate_cross_platform_identity(mentor: Address, platform_id: Symbol) -> bool` — true if the mentor is verified and the platform record is verified.
- `confirm_authenticity(mentor: Address, platform_id: Symbol) -> bool` — true if the mentor is verified and the identity correlation matches.
- `monitor_accounts(mentor: Address) -> u32` — returns the length of the account monitoring log.

### Cross-Platform Reputation Bridging

- `verify_cross_platform_creds(user: Address, platform: Symbol, credential_hash: BytesN<32>) -> bool` — stub; always returns true.
- `validate_external_reputation(user: Address, platform: Symbol, score: u32) -> bool` — true if score > 0.
- `bridge_identity(user: Address, platform: Symbol, external_id: Symbol) -> bool` — user-authenticated identity consistency check; stores the bridged identity on success.

## Storage

| DataKey Variant | Contents |
| --- | --- |
| `NamespaceRoot` | Contract-isolated storage namespace root |
| `Admin` | Admin address |
| `Verification(Address)` | Verification record (credential hash, timestamps, active flag, grace) |
| `Tier(Address)` | Mentor tier level |
| `IPRecord(Symbol)` | IP record by IP ID |
| `ContentOwnership(Symbol)` | Content ownership by content ID |
| `IPUsage(Symbol, Address)` | IP usage by IP ID and user |
| `InfringementCase(Symbol)` | Infringement case by case ID |
| `TakedownRequest(Symbol)` | Takedown request by request ID |
| `RecoveryAction(Symbol)` | Recovery action by action ID |
| `GracePeriod` | Global grace period in seconds |
| `CertificationAuthority(Address)` | Authority record (registered at, reputation, active) |
| `RevokedCredential(BytesN<32>)` | Revoked-credential flag |
| `AdmissionCriteria(Address)` | Admission equity record |
| `AccessPattern(Address)` | Access monitoring record |
| `VerificationOnboardingProtection(Address)` | Onboarding protection record |
| `LastCertifiedAt(Address, Symbol)` | Last certification timestamp per specialization |
| `SkillAssessment(Address, Symbol)` | Practical assessment record |
| `SkillCredential(Address, Symbol)` | Credential authentication record |
| `SpecializationOutcomes(Address, Symbol)` | Rolling session outcome scores (max 20) |
| `SkillFraudFlag(Address, Symbol)` | Skill fraud flag |
| `CrossPlatformVerification(Address, Symbol)` | Cross-platform identity record |
| `AccountMonitoringLog(Address)` | Account monitoring log |
| `CrossPlatformCreds(Address, Symbol)` | Cross-platform credential record |
| `BridgedIdentity(Address, Symbol)` | Bridged external platform ID |
| `InformationAuditRecord(Address)` | Information accuracy audit record |

## Events

| Topic | Description |
| --- | --- |
| `("Verify", "VrfyOk", mentor)` | Mentor verified; payload carries credential hash and timestamps |
| `("Verify", "Revoke", mentor)` | Verification revoked |
| `("Verify", "AuthReg", authority)` | Certification authority registered; payload is reputation bps |
| `("Verify", "CredRev")` | Credential revoked; payload is the credential hash |
| `("Verify", "Renew", mentor)` | Verification renewed with a new expiry |
| `("adm_crit", "inequitable", applicant)` | Inequitable admission criteria detected |
| `("acc_pat", "barrier_gaming", applicant)` | Barrier-gaming access pattern detected |
| `("onb_prot", "intervened", applicant)` | Onboarding protection intervention triggered |
| `("Skill", "Assessed", mentor)` | Skill assessment recorded; payload is (specialization, score, verified) |
| `("Skill", "CredAuth", mentor)` | Specialization credential authenticated |
| `("Skill", "FraudFlg", mentor)` | Skill fraud flag raised; payload is (specialization, risk score) |
| `MetadataManipulationAlert` (category `meta_alt`) | Published by `get_verification_status` when the information audit fails |

## Cross-Contract Dependencies

- No on-chain contract invocations; the crate depends on the `shared` library for computation helpers.
- Admin address (stored at `initialize`) gates all privileged operations.
- Certification authorities are registered via `register_certification_authority` and validated by `validate_certification_authority`.
- Exposes the `VerificationContractClient` trait (`is_verified`, `get_verification_status`, `submit_verification`) for consumption by other contracts.

## Testing

```bash
cargo test -p mentorminds-verification
```

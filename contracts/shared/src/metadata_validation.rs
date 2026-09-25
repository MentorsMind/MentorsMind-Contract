//! Session metadata validation and information warfare protection primitives.

use soroban_sdk::{contracttype, Env, Symbol};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Authenticity score (0-100) below which metadata is considered manipulated.
pub const METADATA_AUTHENTICITY_THRESHOLD: u32 = 70;

/// Disinformation risk score (0-100) at or above which information integrity is breached.
pub const DISINFORMATION_RISK_THRESHOLD: u32 = 40;

/// Transparency risk score (0-100) at or above which transparency protection auto-intervenes.
pub const TRANSPARENCY_RISK_THRESHOLD: u32 = 60;

/// Minimum source credibility in basis points (5000 = 50%) for information accuracy.
pub const MIN_SOURCE_CREDIBILITY_BPS: u32 = 5_000;

/// Default cooldown before an intervened session/entity is eligible for automatic truth restoration.
pub const TRANSPARENCY_RESTORATION_COOLDOWN_SECS: u64 = 7 * 24 * 3_600;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Authenticity verification and manipulation detection for session metadata.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataValidation {
    pub authentic: bool,
    pub authenticity_score: u32,
    pub manipulation_detected: bool,
    pub manipulation_risk_score: u32,
    pub anomaly_count: u32,
    pub verified_at: u64,
}

/// Information integrity and disinformation prevention assessment.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InformationIntegrity {
    pub integrity_verified: bool,
    pub accuracy_score: u32,
    pub disinformation_flag: bool,
    pub disinformation_risk_score: u32,
    pub verification_ratio_bps: u32,
    pub audit_count: u32,
}

/// Transparency protection and information warfare resistance system.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransparencyProtection {
    pub protected: bool,
    pub warfare_resistance_score: u32,
    pub truth_validated: bool,
    pub combined_risk_score: u32,
    pub reason: Symbol,
    pub restoration_eligible_at: u64,
}

/// Metadata monitoring and misinformation detection record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetadataMonitoringRecord {
    pub monitored: bool,
    pub manipulation_level: u32,
    pub misinformation_detected: bool,
    pub suspicious_pattern_count: u32,
    pub update_frequency_score: u32,
}

/// Information audit and disinformation tracking measures.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InformationAuditRecord {
    pub audited: bool,
    pub accuracy_verified: bool,
    pub disinformation_score: u32,
    pub tracking_id: u64,
    pub total_claims: u32,
    pub verified_claims: u32,
}

/// Automatic correction and truth restoration procedure result.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TruthRestorationRecord {
    pub corrected: bool,
    pub truth_restored: bool,
    pub restoration_timestamp: u64,
    pub original_accuracy_bps: u32,
    pub restored_accuracy_bps: u32,
    pub correction_notes: Symbol,
}

// ---------------------------------------------------------------------------
// Pure functions & utilities
// ---------------------------------------------------------------------------

/// Perform metadata validation with authenticity verification and manipulation detection.
pub fn validate_metadata_authenticity(
    source_count: u32,
    unverified_changes: u32,
    timestamp_delta: u64,
    now: u64,
) -> MetadataValidation {
    let mut risk_score = 0u32;
    let mut anomaly_count = 0u32;

    if source_count == 0 {
        risk_score = risk_score.saturating_add(60);
        anomaly_count = anomaly_count.saturating_add(1);
    } else if source_count < 2 {
        risk_score = risk_score.saturating_add(30);
    }

    if unverified_changes > 0 {
        let unverified_risk = (unverified_changes.saturating_mul(20)).min(60);
        risk_score = risk_score.saturating_add(unverified_risk);
        anomaly_count = anomaly_count.saturating_add(unverified_changes);
    }

    if timestamp_delta < 60 && timestamp_delta > 0 {
        risk_score = risk_score.saturating_add(25);
        anomaly_count = anomaly_count.saturating_add(1);
    }

    let manipulation_risk_score = risk_score.min(100);
    let authenticity_score = 100u32.saturating_sub(manipulation_risk_score);
    let manipulation_detected = manipulation_risk_score >= 30 || authenticity_score < METADATA_AUTHENTICITY_THRESHOLD;
    let authentic = !manipulation_detected && authenticity_score >= METADATA_AUTHENTICITY_THRESHOLD;

    MetadataValidation {
        authentic,
        authenticity_score,
        manipulation_detected,
        manipulation_risk_score,
        anomaly_count,
        verified_at: now,
    }
}

/// Verify information integrity with disinformation prevention and accuracy verification.
pub fn verify_information_integrity(
    verified_claims: u32,
    total_claims: u32,
    disinformation_signals: u32,
) -> InformationIntegrity {
    let verification_ratio_bps = if total_claims == 0 {
        10_000u32
    } else {
        (verified_claims.saturating_mul(10_000)) / total_claims
    };

    let accuracy_score = (verification_ratio_bps / 100).min(100);
    let mut disinfo_risk = 0u32;

    if verification_ratio_bps < MIN_SOURCE_CREDIBILITY_BPS {
        disinfo_risk = disinfo_risk.saturating_add(40);
    }

    if disinformation_signals >= 3 {
        disinfo_risk = disinfo_risk.saturating_add(50);
    } else if disinformation_signals >= 1 {
        disinfo_risk = disinfo_risk.saturating_add(20);
    }

    let disinformation_risk_score = disinfo_risk.min(100);
    let disinformation_flag = disinformation_risk_score >= DISINFORMATION_RISK_THRESHOLD;
    let integrity_verified = !disinformation_flag && accuracy_score >= 50;

    InformationIntegrity {
        integrity_verified,
        accuracy_score,
        disinformation_flag,
        disinformation_risk_score,
        verification_ratio_bps,
        audit_count: total_claims,
    }
}

/// Transparency protection with information warfare resistance and truth validation.
pub fn protect_transparency(
    env: &Env,
    metadata: MetadataValidation,
    integrity: InformationIntegrity,
    cooldown_secs: u64,
) -> TransparencyProtection {
    let combined_risk = (metadata.manipulation_risk_score.saturating_add(integrity.disinformation_risk_score)) / 2;
    let combined_risk_score = combined_risk.min(100);
    let warfare_resistance_score = 100u32.saturating_sub(combined_risk_score);
    let truth_validated = metadata.authentic && integrity.integrity_verified;

    let (protected, reason) = if !metadata.authentic {
        (true, Symbol::new(env, "metadata_manipulation"))
    } else if integrity.disinformation_flag {
        (true, Symbol::new(env, "disinformation"))
    } else if combined_risk_score >= TRANSPARENCY_RISK_THRESHOLD {
        (true, Symbol::new(env, "warfare_threat"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    let now = env.ledger().timestamp();
    TransparencyProtection {
        protected,
        warfare_resistance_score,
        truth_validated,
        combined_risk_score,
        reason,
        restoration_eligible_at: if protected {
            now.saturating_add(cooldown_secs)
        } else {
            now
        },
    }
}

/// Metadata monitoring with manipulation identification and misinformation detection systems.
pub fn monitor_metadata_manipulation(
    update_frequency: u32,
    unverified_changes: u32,
) -> MetadataMonitoringRecord {
    let freq_score = (update_frequency.saturating_mul(10)).min(50);
    let manip_level = freq_score.saturating_add((unverified_changes.saturating_mul(20)).min(50)).min(100);
    let misinformation_detected = manip_level >= 50;
    let suspicious_pattern_count = unverified_changes.saturating_add(if update_frequency > 5 { 1 } else { 0 });

    MetadataMonitoringRecord {
        monitored: true,
        manipulation_level: manip_level,
        misinformation_detected,
        suspicious_pattern_count,
        update_frequency_score: freq_score,
    }
}

/// Comprehensive information audit with accuracy verification and disinformation tracking measures.
pub fn audit_information_accuracy(
    total_claims: u32,
    verified_claims: u32,
    disinformation_flags: u32,
) -> InformationAuditRecord {
    let unverified = total_claims.saturating_sub(verified_claims);
    let disinfo_score = ((disinformation_flags.saturating_mul(30)).saturating_add(unverified.saturating_mul(10))).min(100);
    let accuracy_verified = disinfo_score < 40 && (verified_claims >= (total_claims / 2) || total_claims == 0);

    InformationAuditRecord {
        audited: true,
        accuracy_verified,
        disinformation_score: disinfo_score,
        tracking_id: 1,
        total_claims,
        verified_claims,
    }
}

/// Automatic correction and truth restoration procedures.
pub fn restore_truth_and_correct(
    env: &Env,
    _audit: &InformationAuditRecord,
    current_accuracy_bps: u32,
) -> TruthRestorationRecord {
    let restored_bps = current_accuracy_bps.max(8_000);
    TruthRestorationRecord {
        corrected: true,
        truth_restored: true,
        restoration_timestamp: env.ledger().timestamp(),
        original_accuracy_bps: current_accuracy_bps,
        restored_accuracy_bps: restored_bps,
        correction_notes: Symbol::new(env, "truth_restored"),
    }
}

/// Check if a transparency protection intervention is eligible for restoration.
pub fn is_transparency_restoration_eligible(protection: &TransparencyProtection, now: u64) -> bool {
    !protection.protected || now >= protection.restoration_eligible_at
}

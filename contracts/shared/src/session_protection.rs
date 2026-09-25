use soroban_sdk::{contracttype, Address, Symbol};

/// Maximum number of active sessions a single mentor can protect simultaneously.
pub const MAX_PROTECTED_SESSIONS: u32 = 50;

/// Cooldown (seconds) between consecutive protection checks for the same session.
pub const PROTECTION_CHECK_COOLDOWN_SECS: u64 = 60;

/// Threshold (basis points) above which a session disruption score is flagged.
pub const DISRUPTION_RISK_THRESHOLD_BPS: u32 = 7_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionProtectionRecord {
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub protected_at: u64,
    pub disruption_score: u32,
    pub backup_active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProtectionCheckResult {
    pub session_id: Symbol,
    pub protected: bool,
    pub disruption_score: u32,
    pub backup_activated: bool,
}

/// Compute a disruption-risk score for a session based on frequency of
/// status flips and time since last activity.
///
/// Returns a score in basis points (0–10_000). Higher = more risky.
pub fn compute_disruption_score(
    status_flip_count: u32,
    idle_secs: u64,
    participant_count: u32,
) -> u32 {
    let flip_component = status_flip_count.saturating_mul(500);
    let idle_component = if idle_secs > 3_600 {
        2_000
    } else if idle_secs > 600 {
        1_000
    } else {
        0
    };
    let participant_component = if participant_count > 10 { 1_500 } else { 0 };
    flip_component
        .saturating_add(idle_component)
        .saturating_add(participant_component)
        .min(10_000)
}

/// Check whether a session qualifies for protection based on its disruption
/// score and the current timestamp vs. the last protection check.
pub fn should_protect_session(
    disruption_score: u32,
    last_check_at: u64,
    now: u64,
) -> bool {
    if now.saturating_sub(last_check_at) < PROTECTION_CHECK_COOLDOWN_SECS {
        return false;
    }
    disruption_score >= DISRUPTION_RISK_THRESHOLD_BPS
}

/// Activate backup continuity for a session. Returns the updated record.
pub fn activate_backup(
    record: SessionProtectionRecord,
    now: u64,
) -> SessionProtectionRecord {
    SessionProtectionRecord {
        session_id: record.session_id,
        mentor: record.mentor,
        learner: record.learner,
        protected_at: now,
        disruption_score: record.disruption_score,
        backup_active: true,
    }
}

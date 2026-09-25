use soroban_sdk::{contracttype, Symbol, Vec};

/// Maximum number of detection events retained per session.
pub const DETECTION_LOG_CAP: u32 = 20;

/// Minimum number of suspicious events within the detection window to
/// trigger an attack flag.
pub const ATTACK_FLAG_THRESHOLD: u32 = 3;

/// Detection window in seconds for counting suspicious events.
pub const ATTACK_DETECTION_WINDOW_SECS: u64 = 3_600;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttackEvent {
    pub timestamp: u64,
    pub event_type: AttackType,
    pub severity: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AttackType {
    /// Repeated rapid session creation/deletion.
    SessionFlooding,
    /// Attempting to access sessions the caller does not own.
    UnauthorizedAccess,
    /// Status toggled back-and-forth rapidly.
    StatusFlipAttack,
    /// Excessive resource consumption in a short window.
    ResourceExhaustion,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AttackDetectionResult {
    pub detected: bool,
    pub attack_type: AttackType,
    pub event_count: u32,
    pub risk_score: u32,
}

/// Score a single attack event. Returns a severity in basis points.
pub fn score_attack_event(event_type: &AttackType) -> u32 {
    match event_type {
        AttackType::SessionFlooding => 3_000,
        AttackType::UnauthorizedAccess => 4_000,
        AttackType::StatusFlipAttack => 2_500,
        AttackType::ResourceExhaustion => 3_500,
    }
}

/// Determine whether a series of attack events constitutes a confirmed attack.
pub fn evaluate_attack_risk(events: &Vec<AttackEvent>, now: u64) -> AttackDetectionResult {
    let window_start = now.saturating_sub(ATTACK_DETECTION_WINDOW_SECS);
    let mut recent: u32 = 0;
    let mut total_severity: u32 = 0;
    let mut dominant = AttackType::SessionFlooding;
    let mut max_severity: u32 = 0;

    for i in 0..events.len() {
        if let Some(e) = events.get(i) {
            if e.timestamp >= window_start {
                recent = recent.saturating_add(1);
                total_severity = total_severity.saturating_add(e.severity);
                if e.severity >= max_severity {
                    max_severity = e.severity;
                    dominant = e.event_type.clone();
                }
            }
        }
    }

    let detected = recent >= ATTACK_FLAG_THRESHOLD;

    AttackDetectionResult {
        detected,
        attack_type: dominant,
        event_count: recent,
        risk_score: total_severity.min(10_000),
    }
}

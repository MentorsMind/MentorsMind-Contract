use soroban_sdk::{contracttype, Env};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const DEFAULT_MAX_REQUESTS_PER_MINUTE: u32 = 30;
pub const ABUSE_PATTERN_THRESHOLD_BPS: u32 = 8000; // 80% failure or spam rate
pub const EMERGENCY_THROTTLE_RATE: u32 = 5; // Reduced RPM during emergency
pub const RESOURCE_QUOTA_MAX_SESSIONS: u32 = 100; // Max concurrent

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitStatus {
    pub allowed: bool,
    pub current_rpm: u32,
    pub is_emergency: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResourceAllocation {
    pub granted: bool,
    pub remaining_quota: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AbuseDetectionResult {
    pub is_abusive: bool,
    pub abuse_score: u32,
}

// ---------------------------------------------------------------------------
// Resource Management & Quotas
// ---------------------------------------------------------------------------

pub fn allocate_system_resources(
    _env: &Env,
    current_active: u32,
    requested: u32,
) -> ResourceAllocation {
    let new_total = current_active.saturating_add(requested);
    if new_total > RESOURCE_QUOTA_MAX_SESSIONS {
        return ResourceAllocation {
            granted: false,
            remaining_quota: RESOURCE_QUOTA_MAX_SESSIONS.saturating_sub(current_active),
        };
    }
    
    ResourceAllocation {
        granted: true,
        remaining_quota: RESOURCE_QUOTA_MAX_SESSIONS - new_total,
    }
}

// ---------------------------------------------------------------------------
// Load Balancing & Rate Limiting
// ---------------------------------------------------------------------------

pub fn manage_session_load(
    _env: &Env,
    recent_requests: u32,
    is_emergency: bool,
) -> RateLimitStatus {
    let limit = if is_emergency {
        EMERGENCY_THROTTLE_RATE
    } else {
        DEFAULT_MAX_REQUESTS_PER_MINUTE
    };
    
    RateLimitStatus {
        allowed: recent_requests < limit,
        current_rpm: recent_requests,
        is_emergency,
    }
}

// ---------------------------------------------------------------------------
// Attack Detection
// ---------------------------------------------------------------------------

pub fn detect_abuse_patterns(
    _env: &Env,
    total_requests: u32,
    failed_requests: u32,
) -> AbuseDetectionResult {
    if total_requests < 5 {
        return AbuseDetectionResult {
            is_abusive: false,
            abuse_score: 0,
        };
    }
    
    let fail_rate_bps = (failed_requests.saturating_mul(10000)) / total_requests;
    
    AbuseDetectionResult {
        is_abusive: fail_rate_bps >= ABUSE_PATTERN_THRESHOLD_BPS,
        abuse_score: fail_rate_bps,
    }
}

pub fn check_emergency_trigger(
    global_requests_rpm: u32,
) -> bool {
    global_requests_rpm > 500 // Threshold for global emergency state
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_allocate_resources_at_99_sessions_request_1() {
        let env = Env::default();
        let result = allocate_system_resources(&env, 99, 1);
        
        assert_eq!(result.granted, true);
        assert_eq!(result.remaining_quota, 0);
    }

    #[test]
    fn test_allocate_resources_at_100_sessions_request_1() {
        let env = Env::default();
        let result = allocate_system_resources(&env, 100, 1);
        
        assert_eq!(result.granted, false);
        assert_eq!(result.remaining_quota, 0);
    }

    #[test]
    fn test_allocate_resources_exceeds_quota() {
        let env = Env::default();
        let result = allocate_system_resources(&env, 95, 10);
        
        assert_eq!(result.granted, false);
        assert_eq!(result.remaining_quota, 5);
    }

    #[test]
    fn test_manage_session_load_emergency_throttle() {
        let env = Env::default();
        let result = manage_session_load(&env, 3, true);
        
        assert_eq!(result.allowed, true);
        assert_eq!(result.current_rpm, 3);
        assert_eq!(result.is_emergency, true);
    }

    #[test]
    fn test_manage_session_load_emergency_reduced_rpm() {
        let env = Env::default();
        let result = manage_session_load(&env, 10, true);
        
        // During emergency, RPM should be EMERGENCY_THROTTLE_RATE (5)
        assert_eq!(result.allowed, false); // 10 >= 5
        assert_eq!(result.current_rpm, 10);
        assert_eq!(result.is_emergency, true);
    }

    #[test]
    fn test_manage_session_load_normal_operation() {
        let env = Env::default();
        let result = manage_session_load(&env, 25, false);
        
        assert_eq!(result.allowed, true); // 25 < 30 (DEFAULT_MAX_REQUESTS_PER_MINUTE)
        assert_eq!(result.current_rpm, 25);
        assert_eq!(result.is_emergency, false);
    }
}

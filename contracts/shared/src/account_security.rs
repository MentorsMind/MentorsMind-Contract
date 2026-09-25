use soroban_sdk::{contracttype, Address, Symbol};

/// Maximum number of failed login attempts before lockout.
pub const MAX_FAILED_ATTEMPTS: u32 = 5;

/// Lockout duration (seconds) after exceeding max failed attempts.
pub const LOCKOUT_DURATION_SECS: u64 = 3_600;

/// Maximum number of device signatures retained per user.
pub const MAX_DEVICE_SIGNATURES: u32 = 10;

/// Threshold (basis points) for cross-platform identity correlation.
pub const CROSS_PLATFORM_CORRELATION_THRESHOLD_BPS: u32 = 7_000;

/// Window (seconds) for behavioral analysis.
pub const BEHAVIORAL_WINDOW_SECS: u64 = 86_400;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccountSecurityRecord {
    pub user: Address,
    pub failed_attempts: u32,
    pub locked_until: u64,
    pub last_login_at: u64,
    pub mfa_verified: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossPlatformIdentity {
    pub user: Address,
    pub platform_id: Symbol,
    pub correlation_score: u32,
    pub verified: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FraudAlert {
    pub user: Address,
    pub alert_type: FraudType,
    pub confidence_bps: u32,
    pub detected_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FraudType {
    AccountTakeover,
    IdentitySpoofing,
    MultiAccountAbuse,
    CredentialStuffing,
}

/// Check whether an account is currently locked out.
pub fn is_account_locked(record: &AccountSecurityRecord, now: u64) -> bool {
    record.locked_until > now
}

/// Record a failed login attempt and determine if lockout should trigger.
pub fn record_failed_attempt(record: &mut AccountSecurityRecord, now: u64) {
    record.failed_attempts = record.failed_attempts.saturating_add(1);
    if record.failed_attempts >= MAX_FAILED_ATTEMPTS {
        record.locked_until = now.saturating_add(LOCKOUT_DURATION_SECS);
    }
}

/// Reset failed attempts on successful login.
pub fn record_successful_login(record: &mut AccountSecurityRecord, now: u64) {
    record.failed_attempts = 0;
    record.locked_until = 0;
    record.last_login_at = now;
}

/// Compute a cross-platform correlation score from shared attributes.
pub fn compute_correlation_score(
    shared_attributes: u32,
    total_attributes: u32,
) -> u32 {
    if total_attributes == 0 {
        return 0;
    }
    ((shared_attributes as u64 * 10_000) / total_attributes as u64) as u32
}

/// Determine whether the correlation score warrants an identity match.
pub fn is_identity_match(correlation_score: u32) -> bool {
    correlation_score >= CROSS_PLATFORM_CORRELATION_THRESHOLD_BPS
}

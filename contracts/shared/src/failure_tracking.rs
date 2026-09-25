use soroban_sdk::{contracttype, BytesN, Env};

/// Classification of auto-release failure reasons
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FailureClassification {
    /// Temporary network/contract availability issue (retry recommended)
    Temporary,
    /// Cross-contract verification or authorization failure
    Authorization,
    /// Insurance or reputation checks failed
    PolicyViolation,
    /// State transition or invariant check failed (may be permanent)
    StateError,
    /// Insufficient balance or fund availability
    FundsUnavailable,
    /// Other unclassified error
    Unknown,
}

/// State of escrow in recovery pathway after max failures
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecoveryState {
    /// Active and retrying with backoff
    Retrying,
    /// Manual recovery available via admin
    AwaitingManualRecovery,
    /// Escalated to governance or emergency process
    Escalated,
    /// Successfully recovered
    Recovered,
}

/// Per-escrow failure tracking record with exponential backoff
#[contracttype]
#[derive(Clone, Debug)]
pub struct ReleaseFailure {
    /// Escrow ID
    pub escrow_id: u64,
    /// Current attempt number (1-indexed)
    pub attempt_number: u32,
    /// Maximum retry attempts allowed (typically 10)
    pub max_attempts: u32,
    /// Classification of the failure
    pub classification: FailureClassification,
    /// Timestamp of last failure
    pub last_failure_time: u64,
    /// Next retry timestamp (backoff applied)
    pub next_retry_time: u64,
    /// Hash of error message for tracking patterns
    pub error_hash: BytesN<32>,
    /// Recovery state
    pub recovery_state: RecoveryState,
    /// Admin intervention timestamp (0 if none)
    pub manual_recovery_at: u64,
    /// Reason hash for manual recovery
    pub manual_recovery_reason: BytesN<32>,
}

/// Exponential backoff configuration
#[contracttype]
#[derive(Clone, Debug)]
pub struct ExponentialBackoff {
    /// Initial delay in seconds (1 hour)
    pub initial_delay_secs: u64,
    /// Multiplier applied each attempt (2x)
    pub multiplier: u32,
    /// Maximum delay cap in seconds (8 hours)
    pub max_delay_secs: u64,
    /// Current attempt number
    pub attempt_number: u32,
}

/// Constants for failure tracking and recovery
pub const MAX_AUTO_RELEASE_ATTEMPTS: u32 = 10;
pub const MANUAL_RECOVERY_THRESHOLD: u32 = 5;
pub const EXPONENTIAL_BACKOFF_INITIAL_SECS: u64 = 60 * 60; // 1 hour
pub const EXPONENTIAL_BACKOFF_MULTIPLIER: u32 = 2;
pub const EXPONENTIAL_BACKOFF_MAX_SECS: u64 = 8 * 60 * 60; // 8 hours
pub const FAILURE_PATTERN_ANALYSIS_WINDOW: u64 = 7 * 24 * 60 * 60; // 7 days

/// Calculate the next retry timestamp using exponential backoff
///
/// Formula: next_retry = now + min(initial * (multiplier ^ attempt), max)
pub fn calculate_backoff_delay(
    attempt_number: u32,
    initial_delay_secs: u64,
    multiplier: u32,
    max_delay_secs: u64,
) -> u64 {
    let multiplier_u64 = multiplier as u64;
    
    // Calculate: multiplier ^ (attempt - 1) with overflow protection
    let mut factor = 1u64;
    for _ in 0..attempt_number.saturating_sub(1) {
        factor = factor.saturating_mul(multiplier_u64);
        if factor > max_delay_secs / initial_delay_secs {
            break;
        }
    }
    
    let calculated = initial_delay_secs.saturating_mul(factor);
    calculated.min(max_delay_secs)
}

/// Classify failure based on error characteristics
pub fn classify_failure(error_code: u32) -> FailureClassification {
    match error_code {
        // Network/temporary errors (100-199)
        100..=199 => FailureClassification::Temporary,
        // Authorization/contract errors (200-299)
        200..=299 => FailureClassification::Authorization,
        // Policy/insurance errors (300-399)
        300..=399 => FailureClassification::PolicyViolation,
        // State/logic errors (400-499)
        400..=499 => FailureClassification::StateError,
        // Fund availability errors (500-599)
        500..=599 => FailureClassification::FundsUnavailable,
        // Unknown errors
        _ => FailureClassification::Unknown,
    }
}

/// Calculate next retry timestamp with exponential backoff
pub fn calculate_next_retry(env: &Env, failure: &ReleaseFailure) -> u64 {
    let now = env.ledger().timestamp();
    let backoff = calculate_backoff_delay(
        failure.attempt_number,
        EXPONENTIAL_BACKOFF_INITIAL_SECS,
        EXPONENTIAL_BACKOFF_MULTIPLIER,
        EXPONENTIAL_BACKOFF_MAX_SECS,
    );
    now.saturating_add(backoff)
}

/// Compute a hash of an error message for pattern tracking
///
/// This allows aggregating similar failures without storing full error strings.
pub fn compute_failure_hash(env: &Env, error_msg: &str) -> BytesN<32> {
    use soroban_sdk::Bytes;
    let mut bytes = Bytes::new(env);
    for byte in error_msg.as_bytes() {
        bytes.append(&Bytes::from_slice(env, &[*byte]));
    }
    env.crypto().sha256(&bytes).into()
}

/// Check if manual recovery should be offered (after 5+ consecutive failures)
pub fn should_offer_manual_recovery(attempt_number: u32) -> bool {
    attempt_number >= MANUAL_RECOVERY_THRESHOLD
}

/// Check if the escrow has exceeded max attempts
pub fn is_max_attempts_exceeded(attempt_number: u32) -> bool {
    attempt_number >= MAX_AUTO_RELEASE_ATTEMPTS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_exponential_backoff_calculation() {
        // Attempt 1: 1 hour
        let delay1 = calculate_backoff_delay(1, 3600, 2, 28800);
        assert_eq!(delay1, 3600);

        // Attempt 2: 2 hours
        let delay2 = calculate_backoff_delay(2, 3600, 2, 28800);
        assert_eq!(delay2, 7200);

        // Attempt 3: 4 hours
        let delay3 = calculate_backoff_delay(3, 3600, 2, 28800);
        assert_eq!(delay3, 14400);

        // Attempt 4: 8 hours (capped at max)
        let delay4 = calculate_backoff_delay(4, 3600, 2, 28800);
        assert_eq!(delay4, 28800);

        // Attempt 5+: stays at max
        let delay5 = calculate_backoff_delay(5, 3600, 2, 28800);
        assert_eq!(delay5, 28800);
    }

    #[test]
    fn test_failure_classification() {
        assert_eq!(classify_failure(105), FailureClassification::Temporary);
        assert_eq!(classify_failure(250), FailureClassification::Authorization);
        assert_eq!(classify_failure(350), FailureClassification::PolicyViolation);
        assert_eq!(classify_failure(450), FailureClassification::StateError);
        assert_eq!(classify_failure(550), FailureClassification::FundsUnavailable);
        assert_eq!(classify_failure(999), FailureClassification::Unknown);
    }

    #[test]
    fn test_manual_recovery_threshold() {
        assert!(!should_offer_manual_recovery(1));
        assert!(!should_offer_manual_recovery(4));
        assert!(should_offer_manual_recovery(5));
        assert!(should_offer_manual_recovery(6));
    }

    #[test]
    fn test_max_attempts_exceeded() {
        assert!(!is_max_attempts_exceeded(9));
        assert!(is_max_attempts_exceeded(10));
        assert!(is_max_attempts_exceeded(11));
    }
}

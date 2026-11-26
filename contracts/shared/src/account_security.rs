use soroban_sdk::{contracterror, contracttype, Address, Symbol};

/// Maximum number of failed login attempts before lockout.
pub const MAX_FAILED_ATTEMPTS: u32 = 5;

/// Base lockout duration (seconds) applied when `MAX_FAILED_ATTEMPTS` is first
/// reached. Each further failed attempt doubles it, up to `MAX_LOCKOUT_SECS`.
pub const LOCKOUT_DURATION_SECS: u64 = 3_600;

/// Hard upper bound on any lockout duration (seconds): 24 hours.
///
/// Escalation doubles the lockout with every failed attempt past
/// `MAX_FAILED_ATTEMPTS`, so without a ceiling an attacker who keeps submitting
/// bad credentials could lock the real owner out for an unbounded time (and a
/// large enough attempt count overflows `u64`). Every computed duration is
/// clamped with `min(computed, MAX_LOCKOUT_SECS)`.
///
/// It also sets the appeal window: see `appeal_lockout`.
pub const MAX_LOCKOUT_SECS: u64 = 86_400;

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

/// Clamp a computed lockout duration to `MAX_LOCKOUT_SECS`.
pub fn clamp_lockout_duration(computed: u64) -> u64 {
    computed.min(MAX_LOCKOUT_SECS)
}

/// Lockout duration (seconds) for an account with `failed_attempts` failures.
///
/// * Below `MAX_FAILED_ATTEMPTS`: `0`, no lockout.
/// * At `MAX_FAILED_ATTEMPTS`: `LOCKOUT_DURATION_SECS`.
/// * Each failure beyond that doubles the previous duration.
///
/// The result never exceeds `MAX_LOCKOUT_SECS`, and the doubling cannot
/// overflow: an unrepresentable duration is clamped like any other large one.
pub fn compute_lockout_duration(failed_attempts: u32) -> u64 {
    if failed_attempts < MAX_FAILED_ATTEMPTS {
        return 0;
    }
    let extra_failures = failed_attempts - MAX_FAILED_ATTEMPTS;
    let computed = 1u64
        .checked_shl(extra_failures)
        .and_then(|factor| LOCKOUT_DURATION_SECS.checked_mul(factor))
        .unwrap_or(u64::MAX);
    clamp_lockout_duration(computed)
}

/// Record a failed login attempt and determine if lockout should trigger.
///
/// From `MAX_FAILED_ATTEMPTS` on, every failure (re)starts the lockout from
/// `now` with the escalated, clamped duration from `compute_lockout_duration`.
pub fn record_failed_attempt(record: &mut AccountSecurityRecord, now: u64) {
    record.failed_attempts = record.failed_attempts.saturating_add(1);
    if record.failed_attempts >= MAX_FAILED_ATTEMPTS {
        record.locked_until =
            now.saturating_add(compute_lockout_duration(record.failed_attempts));
    }
}

/// Timestamp at which the current lockout began, or `None` when the account has
/// no lockout.
///
/// The lockout length is a function of `failed_attempts`, which is stored, so
/// the start is `locked_until` minus that length. This holds as long as
/// `locked_until` was set by `record_failed_attempt`.
pub fn lockout_started_at(record: &AccountSecurityRecord) -> Option<u64> {
    if record.locked_until == 0 {
        return None;
    }
    Some(
        record
            .locked_until
            .saturating_sub(compute_lockout_duration(record.failed_attempts)),
    )
}

/// Why an appeal was refused.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum AppealError {
    /// The account is not locked, so there is nothing to appeal.
    NotLocked = 1,
    /// Less than half of `MAX_LOCKOUT_SECS` has passed since the lockout began.
    AppealTooEarly = 2,
}

/// Lift a lockout that has been running for at least half of
/// `MAX_LOCKOUT_SECS`.
///
/// An owner caught in a long, attacker-triggered lockout does not have to wait
/// it out: once the lockout has lasted `MAX_LOCKOUT_SECS / 2` (12 hours) it can
/// be appealed. Shorter lockouts expire on their own before that point, so in
/// practice only escalated lockouts are appealable.
///
/// On success the lockout is cleared and `failed_attempts` is set to
/// `MAX_FAILED_ATTEMPTS - 1`, so the next failed login locks the account again
/// instead of granting a fresh set of attempts.
///
/// This is the eligibility check and state change only. The calling contract
/// decides who may submit and approve an appeal.
///
/// # Errors
/// * `AppealError::NotLocked` if the account is not currently locked.
/// * `AppealError::AppealTooEarly` if the lockout is younger than the window.
pub fn appeal_lockout(record: &mut AccountSecurityRecord, now: u64) -> Result<(), AppealError> {
    if !is_account_locked(record, now) {
        return Err(AppealError::NotLocked);
    }
    let started_at = lockout_started_at(record).unwrap_or(now);
    let appealable_at = started_at.saturating_add(MAX_LOCKOUT_SECS / 2);
    if now < appealable_at {
        return Err(AppealError::AppealTooEarly);
    }
    record.locked_until = 0;
    record.failed_attempts = MAX_FAILED_ATTEMPTS - 1;
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn record(env: &Env) -> AccountSecurityRecord {
        AccountSecurityRecord {
            user: Address::generate(env),
            failed_attempts: 0,
            locked_until: 0,
            last_login_at: 0,
            mfa_verified: false,
        }
    }

    /// Fail `n` times at time `now`.
    fn fail(record: &mut AccountSecurityRecord, n: u32, now: u64) {
        for _ in 0..n {
            record_failed_attempt(record, now);
        }
    }

    #[test]
    fn the_cap_is_documented_as_24_hours() {
        assert_eq!(MAX_LOCKOUT_SECS, 24 * 3_600);
        assert!(MAX_LOCKOUT_SECS > LOCKOUT_DURATION_SECS);
    }

    #[test]
    fn no_lockout_below_the_attempt_limit() {
        assert_eq!(compute_lockout_duration(0), 0);
        assert_eq!(compute_lockout_duration(MAX_FAILED_ATTEMPTS - 1), 0);
    }

    #[test]
    fn first_lockout_uses_the_base_duration() {
        assert_eq!(compute_lockout_duration(MAX_FAILED_ATTEMPTS), LOCKOUT_DURATION_SECS);
    }

    #[test]
    fn each_extra_failure_doubles_the_lockout() {
        assert_eq!(
            compute_lockout_duration(MAX_FAILED_ATTEMPTS + 1),
            2 * LOCKOUT_DURATION_SECS
        );
        assert_eq!(
            compute_lockout_duration(MAX_FAILED_ATTEMPTS + 2),
            4 * LOCKOUT_DURATION_SECS
        );
    }

    #[test]
    fn computed_duration_is_clamped_to_the_maximum() {
        // 3_600 * 2^5 = 115_200 > 86_400.
        assert_eq!(compute_lockout_duration(MAX_FAILED_ATTEMPTS + 5), MAX_LOCKOUT_SECS);
        assert_eq!(compute_lockout_duration(MAX_FAILED_ATTEMPTS + 6), MAX_LOCKOUT_SECS);
        assert_eq!(clamp_lockout_duration(MAX_LOCKOUT_SECS + 1), MAX_LOCKOUT_SECS);
        assert_eq!(clamp_lockout_duration(60), 60);
    }

    #[test]
    fn huge_attempt_counts_cannot_overflow_the_duration() {
        assert_eq!(compute_lockout_duration(MAX_FAILED_ATTEMPTS + 63), MAX_LOCKOUT_SECS);
        assert_eq!(compute_lockout_duration(MAX_FAILED_ATTEMPTS + 64), MAX_LOCKOUT_SECS);
        assert_eq!(compute_lockout_duration(u32::MAX), MAX_LOCKOUT_SECS);
    }

    #[test]
    fn a_lockout_can_never_outlast_the_maximum() {
        let env = Env::default();
        let mut r = record(&env);
        fail(&mut r, 1_000, 10_000);
        assert!(r.locked_until <= 10_000 + MAX_LOCKOUT_SECS);
        assert_eq!(r.locked_until, 10_000 + MAX_LOCKOUT_SECS);
    }

    #[test]
    fn locking_out_starts_at_the_attempt_limit() {
        let env = Env::default();
        let mut r = record(&env);
        fail(&mut r, MAX_FAILED_ATTEMPTS - 1, 1_000);
        assert!(!is_account_locked(&r, 1_000));

        record_failed_attempt(&mut r, 1_000);
        assert!(is_account_locked(&r, 1_000));
        assert_eq!(r.locked_until, 1_000 + LOCKOUT_DURATION_SECS);
    }

    #[test]
    fn a_failure_during_a_lockout_extends_it() {
        let env = Env::default();
        let mut r = record(&env);
        fail(&mut r, MAX_FAILED_ATTEMPTS, 1_000);
        let first = r.locked_until;

        record_failed_attempt(&mut r, 2_000);
        assert_eq!(r.locked_until, 2_000 + 2 * LOCKOUT_DURATION_SECS);
        assert!(r.locked_until > first);
    }

    #[test]
    fn a_successful_login_clears_the_lockout() {
        let env = Env::default();
        let mut r = record(&env);
        fail(&mut r, 10, 1_000);
        record_successful_login(&mut r, 5_000);
        assert_eq!(r.failed_attempts, 0);
        assert!(!is_account_locked(&r, 5_000));
        assert_eq!(lockout_started_at(&r), None);
    }

    #[test]
    fn lockout_start_is_recovered_from_the_record() {
        let env = Env::default();
        let mut r = record(&env);
        assert_eq!(lockout_started_at(&r), None);

        fail(&mut r, MAX_FAILED_ATTEMPTS, 1_000);
        assert_eq!(lockout_started_at(&r), Some(1_000));

        // Escalated and clamped lockouts keep the right start.
        fail(&mut r, 10, 7_000);
        assert_eq!(lockout_started_at(&r), Some(7_000));
    }

    #[test]
    fn appeal_requires_a_lockout() {
        let env = Env::default();
        let mut r = record(&env);
        assert_eq!(appeal_lockout(&mut r, 1_000), Err(AppealError::NotLocked));

        // An expired lockout is no lockout.
        fail(&mut r, MAX_FAILED_ATTEMPTS, 1_000);
        assert_eq!(
            appeal_lockout(&mut r, 1_000 + LOCKOUT_DURATION_SECS),
            Err(AppealError::NotLocked)
        );
    }

    #[test]
    fn appeal_is_refused_before_half_the_maximum_has_passed() {
        let env = Env::default();
        let mut r = record(&env);
        fail(&mut r, MAX_FAILED_ATTEMPTS + 10, 1_000); // clamped to 24h

        let too_early = 1_000 + MAX_LOCKOUT_SECS / 2 - 1;
        assert_eq!(appeal_lockout(&mut r, too_early), Err(AppealError::AppealTooEarly));
        // A refused appeal changes nothing.
        assert!(is_account_locked(&r, too_early));
        assert_eq!(r.locked_until, 1_000 + MAX_LOCKOUT_SECS);
    }

    #[test]
    fn appeal_succeeds_once_half_the_maximum_has_passed() {
        let env = Env::default();
        let mut r = record(&env);
        fail(&mut r, MAX_FAILED_ATTEMPTS + 10, 1_000);

        let now = 1_000 + MAX_LOCKOUT_SECS / 2;
        assert!(is_account_locked(&r, now));
        assert_eq!(appeal_lockout(&mut r, now), Ok(()));

        assert!(!is_account_locked(&r, now));
        assert_eq!(r.locked_until, 0);
    }

    #[test]
    fn a_successful_appeal_leaves_one_strike_before_the_next_lockout() {
        let env = Env::default();
        let mut r = record(&env);
        fail(&mut r, MAX_FAILED_ATTEMPTS + 10, 1_000);
        let now = 1_000 + MAX_LOCKOUT_SECS / 2;
        appeal_lockout(&mut r, now).unwrap();

        assert_eq!(r.failed_attempts, MAX_FAILED_ATTEMPTS - 1);
        assert!(!is_account_locked(&r, now));
        record_failed_attempt(&mut r, now);
        assert!(is_account_locked(&r, now));
    }

    #[test]
    fn short_lockouts_expire_before_they_become_appealable() {
        let env = Env::default();
        let mut r = record(&env);
        fail(&mut r, MAX_FAILED_ATTEMPTS, 1_000); // one hour

        // At the appeal window the lockout is already over.
        let now = 1_000 + MAX_LOCKOUT_SECS / 2;
        assert!(!is_account_locked(&r, now));
        assert_eq!(appeal_lockout(&mut r, now), Err(AppealError::NotLocked));
    }

    #[test]
    fn correlation_helpers_are_unchanged() {
        assert_eq!(compute_correlation_score(0, 0), 0);
        assert_eq!(compute_correlation_score(7, 10), 7_000);
        assert!(is_identity_match(7_000));
        assert!(!is_identity_match(6_999));
    }
}

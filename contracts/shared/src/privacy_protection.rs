//! Learner data privacy protection primitives.
//!
//! Mentors and other participants may try to extract more learner data than
//! a legitimate mentoring interaction requires. These helpers give
//! contracts a deterministic, storage-agnostic way to express consent as a
//! bitmask of data categories, enforce need-to-know minimization, detect
//! excessive/exploitative access patterns, and decide when to automatically
//! isolate a subject's data from further reads. Contracts own the storage
//! of consent records and access logs; these functions are pure
//! scoring/decision logic over data the caller already has on hand.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Data category bitmask
// ---------------------------------------------------------------------------

/// Basic identity fields (name, handle).
pub const FIELD_IDENTITY: u32 = 1 << 0;
/// Contact details (email, phone, socials).
pub const FIELD_CONTACT: u32 = 1 << 1;
/// Learning history / session records.
pub const FIELD_LEARNING_HISTORY: u32 = 1 << 2;
/// Career data (employer, resume, goals).
pub const FIELD_CAREER_DATA: u32 = 1 << 3;
/// Payment / financial information.
pub const FIELD_PAYMENT: u32 = 1 << 4;

/// The minimal field set required to conduct a mentoring session: identity
/// only. Any broader access must be explicitly consented to.
pub const MINIMAL_SESSION_FIELDS: u32 = FIELD_IDENTITY;

/// All known fields, used to validate/clamp requested scopes.
pub const ALL_FIELDS: u32 =
    FIELD_IDENTITY | FIELD_CONTACT | FIELD_LEARNING_HISTORY | FIELD_CAREER_DATA | FIELD_PAYMENT;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Access events within this window are grouped for exploitation scoring.
pub const ACCESS_MONITORING_WINDOW_SECS: u64 = 3_600;

/// Number of accesses to the same subject's data within the monitoring
/// window that is considered excessive.
pub const MAX_ACCESSES_PER_WINDOW: u32 = 5;

/// Risk score (0-100) at or above which access is considered exploitative.
pub const PRIVACY_RISK_THRESHOLD: u32 = 60;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A granular consent grant: which fields a subject has allowed a given
/// purpose to access, and until when.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsentRecord {
    pub subject: Address,
    pub purpose: Symbol,
    pub granted_fields: u32,
    pub granted_at: u64,
    pub expires_at: u64,
}

/// Outcome of an access-control check against a consent record.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct AccessDecision {
    pub allowed: bool,
    pub allowed_fields: u32,
    pub denied_fields: u32,
}

/// Result of scanning access history for exploitative patterns.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct PrivacyMonitoringResult {
    pub exploitative: bool,
    pub risk_score: u32,
    pub accesses_in_window: u32,
}

/// Automatic privacy-protection intervention outcome.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PrivacyInterventionRecord {
    pub isolate: bool,
    pub reason: Symbol,
    pub isolated_at: u64,
}

// ---------------------------------------------------------------------------
// Data access control
// ---------------------------------------------------------------------------

/// Check whether `requested_fields` are covered by an unexpired consent
/// record. Returns which fields are allowed vs. denied.
pub fn check_access(consent: &ConsentRecord, requested_fields: u32, now: u64) -> AccessDecision {
    if now >= consent.expires_at {
        return AccessDecision {
            allowed: false,
            allowed_fields: 0,
            denied_fields: requested_fields & ALL_FIELDS,
        };
    }
    let allowed_fields = requested_fields & consent.granted_fields;
    let denied_fields = requested_fields & !consent.granted_fields & ALL_FIELDS;
    AccessDecision {
        allowed: denied_fields == 0,
        allowed_fields,
        denied_fields,
    }
}

// ---------------------------------------------------------------------------
// Information minimization
// ---------------------------------------------------------------------------

/// Reduce a requested field set to the minimum needed for `purpose`.
/// Unrecognized purposes fall back to the minimal session field set.
pub fn minimize_to_need_to_know(env: &Env, purpose: &Symbol, requested_fields: u32) -> u32 {
    let requested = requested_fields & ALL_FIELDS;

    if *purpose == Symbol::new(env, "session_delivery") {
        requested & MINIMAL_SESSION_FIELDS
    } else if *purpose == Symbol::new(env, "scheduling") {
        requested & (FIELD_IDENTITY | FIELD_CONTACT)
    } else if *purpose == Symbol::new(env, "progress_review") {
        requested & (FIELD_IDENTITY | FIELD_LEARNING_HISTORY)
    } else if *purpose == Symbol::new(env, "billing") {
        requested & (FIELD_IDENTITY | FIELD_PAYMENT)
    } else if *purpose == Symbol::new(env, "career_coaching") {
        requested & (FIELD_IDENTITY | FIELD_LEARNING_HISTORY | FIELD_CAREER_DATA)
    } else {
        requested & MINIMAL_SESSION_FIELDS
    }
}

// ---------------------------------------------------------------------------
// Privacy monitoring / exploitation detection
// ---------------------------------------------------------------------------

/// Score a history of data-access timestamps (by one accessor against one
/// subject) for exploitative extraction patterns.
pub fn detect_exploitation(access_timestamps: &Vec<u64>, now: u64) -> PrivacyMonitoringResult {
    let mut in_window = 0u32;
    for ts in access_timestamps.iter() {
        if now.saturating_sub(ts) <= ACCESS_MONITORING_WINDOW_SECS {
            in_window = in_window.saturating_add(1);
        }
    }

    let mut risk = 0u32;
    if in_window > MAX_ACCESSES_PER_WINDOW {
        let excess = in_window - MAX_ACCESSES_PER_WINDOW;
        risk = risk.saturating_add(40).saturating_add(excess.saturating_mul(10));
    }
    risk = risk.min(100);

    PrivacyMonitoringResult {
        exploitative: risk >= PRIVACY_RISK_THRESHOLD,
        risk_score: risk,
        accesses_in_window: in_window,
    }
}

// ---------------------------------------------------------------------------
// Automatic privacy protection & isolation
// ---------------------------------------------------------------------------

/// Decide whether to automatically isolate a subject's data from further
/// unauthorized access based on the access-control decision and the
/// exploitation-monitoring result.
pub fn compute_privacy_intervention(
    env: &Env,
    access: AccessDecision,
    monitoring: PrivacyMonitoringResult,
) -> PrivacyInterventionRecord {
    let (isolate, reason) = if monitoring.exploitative {
        (true, Symbol::new(env, "excessive_access"))
    } else if !access.allowed && access.denied_fields != 0 {
        (true, Symbol::new(env, "unauthorized_scope"))
    } else {
        (false, Symbol::new(env, "none"))
    };

    PrivacyInterventionRecord {
        isolate,
        reason,
        isolated_at: if isolate { env.ledger().timestamp() } else { 0 },
    }
}

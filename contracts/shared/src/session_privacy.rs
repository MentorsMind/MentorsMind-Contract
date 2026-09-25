//! Cross-session data-isolation & privacy protection primitives (#899).
//!
//! Sensitive session data can leak across sessions, or mentors may access
//! learner data from other mentors' sessions. These helpers give contracts
//! a deterministic, storage-agnostic way to enforce per-session access
//! boundaries, detect cross-session leakage patterns, and contain a
//! detected breach. Contracts own the storage of raw access history;
//! these functions are pure scoring/decision logic over data the caller
//! already has on hand.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Window (seconds) over which cross-session access attempts are
/// monitored for leakage patterns.
pub const CROSS_SESSION_MONITORING_WINDOW_SECS: u64 = 3_600;

/// Number of distinct out-of-scope sessions accessed within the
/// monitoring window at or above which a leak is suspected.
pub const LEAK_DISTINCT_SESSION_THRESHOLD: u32 = 2;

/// Risk score (0-100) at or above which a data-breach containment
/// response is automatically triggered.
pub const BREACH_RISK_THRESHOLD: u32 = 60;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Access-boundary decision for a party requesting a specific session's
/// data.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SessionAccessBoundary {
    pub allowed: bool,
    pub is_participant: bool,
}

/// Result of scanning an accessor's cross-session access history for
/// leakage.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct CrossSessionLeakResult {
    pub leak_suspected: bool,
    pub risk_score: u32,
    pub distinct_out_of_scope_sessions: u32,
}

/// Automatic data-breach containment decision.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataBreachContainment {
    pub contain: bool,
    pub reason: Symbol,
    pub contained_at: u64,
}

// ---------------------------------------------------------------------------
// Session data isolation
// ---------------------------------------------------------------------------

/// Enforce that only the mentor or learner assigned to a session may
/// access that session's data — the fundamental access boundary that
/// prevents mentors from reading other mentors' session data.
pub fn enforce_session_boundary(
    accessor: &Address,
    session_mentor: &Address,
    session_learner: &Address,
) -> SessionAccessBoundary {
    let is_participant = accessor == session_mentor || accessor == session_learner;
    SessionAccessBoundary {
        allowed: is_participant,
        is_participant,
    }
}

// ---------------------------------------------------------------------------
// Leak detection
// ---------------------------------------------------------------------------

/// Detect cross-session leakage by counting how many distinct
/// out-of-scope sessions an accessor attempted to read within the
/// monitoring window.
pub fn detect_cross_session_leak(
    env: &Env,
    out_of_scope_access_timestamps: &Vec<u64>,
    distinct_out_of_scope_sessions: u32,
) -> CrossSessionLeakResult {
    let now = env.ledger().timestamp();
    let mut recent_count = 0u32;
    for ts in out_of_scope_access_timestamps.iter() {
        if now.saturating_sub(ts) <= CROSS_SESSION_MONITORING_WINDOW_SECS {
            recent_count = recent_count.saturating_add(1);
        }
    }

    let mut risk_score = recent_count.saturating_mul(15).min(100);
    if distinct_out_of_scope_sessions >= LEAK_DISTINCT_SESSION_THRESHOLD {
        risk_score = risk_score.saturating_add(30).min(100);
    }

    CrossSessionLeakResult {
        leak_suspected: risk_score >= BREACH_RISK_THRESHOLD,
        risk_score,
        distinct_out_of_scope_sessions,
    }
}

// ---------------------------------------------------------------------------
// Breach containment
// ---------------------------------------------------------------------------

/// Decide whether to automatically contain a detected cross-session
/// data-leak by isolating the offending accessor's further reads.
pub fn contain_data_breach(
    env: &Env,
    leak: CrossSessionLeakResult,
    reason: Symbol,
) -> DataBreachContainment {
    DataBreachContainment {
        contain: leak.leak_suspected,
        reason,
        contained_at: env.ledger().timestamp(),
    }
}

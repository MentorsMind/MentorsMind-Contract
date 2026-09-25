use soroban_sdk::{contracttype, Env, Symbol};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

pub const MAX_LOW_FEE_SESSIONS_PER_PAIR: u32 = 5;
pub const LOW_FEE_THRESHOLD: i128 = 100_000; // Baseline low fee threshold
pub const REQUIRED_INTERACTION_MINUTES: u32 = 10;
pub const FEE_EVASION_TOLERANCE_BPS: u32 = 2000; // 20% deviation allowance

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PenaltyTier {
    None,
    Warning,
    TemporarySuspension,
    PermanentBan,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthenticityResult {
    pub is_authentic: bool,
    pub reason: Symbol,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollusionResult {
    pub is_colluding: bool,
    pub low_fee_count: u32,
    pub penalty_tier: PenaltyTier,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EconomicAuditResult {
    pub is_evading: bool,
    pub deviation_bps: u32,
    pub penalty_tier: PenaltyTier,
}

// ---------------------------------------------------------------------------
// Verification Logic
// ---------------------------------------------------------------------------

pub fn verify_session_authenticity(
    env: &Env,
    interaction_minutes: u32,
    has_valid_signature: bool,
) -> AuthenticityResult {
    if !has_valid_signature {
        return AuthenticityResult {
            is_authentic: false,
            reason: Symbol::new(env, "missing_sig"),
        };
    }
    
    if interaction_minutes < REQUIRED_INTERACTION_MINUTES {
        return AuthenticityResult {
            is_authentic: false,
            reason: Symbol::new(env, "too_short"),
        };
    }

    AuthenticityResult {
        is_authentic: true,
        reason: Symbol::new(env, "ok"),
    }
}

pub fn detect_platform_bypass(
    _env: &Env,
    current_low_fee_count: u32,
    session_price: i128,
) -> CollusionResult {
    let mut new_count = current_low_fee_count;
    if session_price < LOW_FEE_THRESHOLD {
        new_count = new_count.saturating_add(1);
    }
    
    let is_colluding = new_count > MAX_LOW_FEE_SESSIONS_PER_PAIR;
    let penalty = if new_count > MAX_LOW_FEE_SESSIONS_PER_PAIR * 2 {
        PenaltyTier::PermanentBan
    } else if is_colluding {
        PenaltyTier::TemporarySuspension
    } else if new_count == MAX_LOW_FEE_SESSIONS_PER_PAIR {
        PenaltyTier::Warning
    } else {
        PenaltyTier::None
    };

    CollusionResult {
        is_colluding,
        low_fee_count: new_count,
        penalty_tier: penalty,
    }
}

pub fn detect_fee_evasion(
    _env: &Env,
    expected_average_fee: i128,
    actual_average_fee: i128,
) -> EconomicAuditResult {
    if expected_average_fee == 0 || actual_average_fee >= expected_average_fee {
        return EconomicAuditResult {
            is_evading: false,
            deviation_bps: 0,
            penalty_tier: PenaltyTier::None,
        };
    }

    let diff = expected_average_fee - actual_average_fee;
    // Calculate deviation in bps using 10000 limit, carefully handling potential division by zero
    let dev = diff.saturating_mul(10_000) / expected_average_fee;
    let deviation_bps = dev as u32;

    let is_evading = deviation_bps > FEE_EVASION_TOLERANCE_BPS;
    let penalty = if deviation_bps > 5000 {
        PenaltyTier::TemporarySuspension
    } else if is_evading {
        PenaltyTier::Warning
    } else {
        PenaltyTier::None
    };

    EconomicAuditResult {
        is_evading,
        deviation_bps,
        penalty_tier: penalty,
    }
}

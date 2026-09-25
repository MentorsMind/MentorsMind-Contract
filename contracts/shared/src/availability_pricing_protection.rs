/// Availability and Pricing Protection Module
///
/// Implements cryptographic commitment schemes for mentor availability with
/// penalty enforcement, market manipulation detection, and fair pricing validation
/// to prevent artificial scarcity and maintain learner access to quality mentoring.

use soroban_sdk::{symbol_short as symbol, Address, Env, Symbol, Vec};

/// Mentor availability commitment with cryptographic binding
#[derive(Clone, Debug, PartialEq)]
pub struct AvailabilityCommitment {
    pub mentor: Address,
    pub commitment_hash: Symbol,
    pub min_hours_per_week: u32,
    pub min_response_time_secs: u32,
    pub committed_at: u64,
    pub expires_at: u64,
    pub is_binding: bool,
}

/// Price coordination detection record
#[derive(Clone, Debug, PartialEq)]
pub struct PriceCoordinationFlag {
    pub mentor_1: Address,
    pub mentor_2: Address,
    pub same_price: bool,
    pub price_set_time_diff: u64,
    pub correlation_score: u32, // 0-10000 basis points
    pub detected_at: u64,
}

/// Dynamic pricing validation result
#[derive(Clone, Debug, PartialEq)]
pub struct DynamicPricingValidation {
    pub mentor: Address,
    pub price: i128,
    pub market_baseline: i128,
    pub deviation_bps: i32, // basis points, can be negative
    pub is_valid: bool,
    pub manipulation_score: u32, // 0-10000
}

/// Market availability monitoring record
#[derive(Clone, Debug, PartialEq)]
pub struct AvailabilityPattern {
    pub mentor: Address,
    pub total_availability_slots: u32,
    pub actually_booked_slots: u32,
    pub rapid_availability_changes: u32,
    pub analyzed_at: u64,
}

/// Cryptographic commitment for availability
pub fn create_availability_commitment(
    env: &Env,
    mentor: Address,
    min_hours_per_week: u32,
    min_response_time_secs: u32,
    duration_secs: u64,
) -> AvailabilityCommitment {
    let current_time = env.ledger().timestamp();

    // Create cryptographic binding of commitment
    let mut commitment_data: Vec<u8> = env.to_bytes(&mentor).unwrap_or_default();
    commitment_data.append(&mut env.to_bytes(&min_hours_per_week).unwrap_or_default());
    commitment_data.append(&mut env.to_bytes(&min_response_time_secs).unwrap_or_default());
    commitment_data.append(&mut env.to_bytes(&current_time).unwrap_or_default());

    let commitment_hash = Symbol::short(
        &env.compute_hash_sha256(&commitment_data)
            .to_short_string()
            .slice(0..7),
    );

    AvailabilityCommitment {
        mentor,
        commitment_hash,
        min_hours_per_week,
        min_response_time_secs,
        committed_at: current_time,
        expires_at: current_time + duration_secs,
        is_binding: true,
    }
}

/// Verify availability commitment authenticity
pub fn verify_availability_commitment(
    env: &Env,
    commitment: &AvailabilityCommitment,
) -> bool {
    // Check if commitment is still active
    if env.ledger().timestamp() > commitment.expires_at {
        return false;
    }

    if !commitment.is_binding {
        return false;
    }

    // Reconstruct and verify commitment hash
    let mut commitment_data: Vec<u8> =
        env.to_bytes(&commitment.mentor).unwrap_or_default();
    commitment_data.append(&mut env.to_bytes(&commitment.min_hours_per_week).unwrap_or_default());
    commitment_data.append(
        &mut env
            .to_bytes(&commitment.min_response_time_secs)
            .unwrap_or_default(),
    );
    commitment_data.append(&mut env.to_bytes(&commitment.committed_at).unwrap_or_default());

    let expected_hash = Symbol::short(
        &env.compute_hash_sha256(&commitment_data)
            .to_short_string()
            .slice(0..7),
    );

    commitment.commitment_hash == expected_hash
}

/// Detect price coordination between mentors
pub fn detect_price_coordination(
    env: &Env,
    mentor_1_price: i128,
    mentor_2_price: i128,
    mentor_1_price_time: u64,
    mentor_2_price_time: u64,
) -> PriceCoordinationFlag {
    let time_diff = if mentor_1_price_time > mentor_2_price_time {
        mentor_1_price_time - mentor_2_price_time
    } else {
        mentor_2_price_time - mentor_1_price_time
    };

    // Check if prices are exactly the same
    let same_price = mentor_1_price == mentor_2_price;

    // Calculate price difference
    let price_diff = if mentor_1_price > mentor_2_price {
        (mentor_1_price - mentor_2_price) as u128
    } else {
        (mentor_2_price - mentor_1_price) as u128
    };

    let max_price = mentor_1_price.max(mentor_2_price) as u128;
    let price_variance_pct = if max_price > 0 {
        price_diff.saturating_mul(100).saturating_div(max_price)
    } else {
        0
    };

    // Calculate correlation score
    let mut correlation_score: u32 = 0;

    // Identical prices = high correlation
    if same_price {
        correlation_score += 6_000;
    }

    // Very similar prices (within 1%) = suspicious
    if price_variance_pct < 1 && !same_price {
        correlation_score += 4_000;
    }

    // Price set within 5 minutes = suspicious timing
    if time_diff < 300 {
        correlation_score += 3_000;
    }

    correlation_score = correlation_score.min(10_000);

    PriceCoordinationFlag {
        mentor_1: Address::generate(env), // Placeholder - would be actual mentor addresses
        mentor_2: Address::generate(env),
        same_price,
        price_set_time_diff: time_diff,
        correlation_score,
        detected_at: env.ledger().timestamp(),
    }
}

/// Validate pricing against market baseline
pub fn validate_dynamic_pricing(
    env: &Env,
    mentor: Address,
    proposed_price: i128,
    market_baseline: i128,
) -> DynamicPricingValidation {
    let current_time = env.ledger().timestamp();

    // Calculate deviation from market baseline
    let deviation = if proposed_price > market_baseline {
        proposed_price - market_baseline
    } else {
        market_baseline - proposed_price
    };

    let max_baseline = proposed_price.max(market_baseline) as u128;
    let deviation_bps = if max_baseline > 0 {
        (deviation as u128)
            .saturating_mul(10_000)
            .saturating_div(max_baseline) as i32
    } else {
        0
    };

    // Adjust sign based on direction
    let adjusted_deviation = if proposed_price > market_baseline {
        deviation_bps as i32
    } else {
        -(deviation_bps as i32)
    };

    // Check if within acceptable range
    let is_valid = adjusted_deviation.abs() <= MAX_PRICE_DEVIATION_BPS as i32;

    // Calculate manipulation score
    let manipulation_score = if adjusted_deviation.abs() > MAX_PRICE_DEVIATION_BPS as i32 {
        ((adjusted_deviation.abs() as u32) - MAX_PRICE_DEVIATION_BPS).min(10_000)
    } else {
        0
    };

    DynamicPricingValidation {
        mentor,
        price: proposed_price,
        market_baseline,
        deviation_bps: adjusted_deviation,
        is_valid,
        manipulation_score,
    }
}

/// Analyze mentor availability patterns for artificial scarcity
pub fn analyze_availability_pattern(
    env: &Env,
    mentor: Address,
    total_slots: u32,
    booked_slots: u32,
    rapid_changes: u32,
) -> AvailabilityPattern {
    AvailabilityPattern {
        mentor,
        total_availability_slots: total_slots,
        actually_booked_slots: booked_slots,
        rapid_availability_changes: rapid_changes,
        analyzed_at: env.ledger().timestamp(),
    }
}

/// Detect artificial scarcity creation
pub fn detect_artificial_scarcity(
    pattern: &AvailabilityPattern,
) -> bool {
    if pattern.total_availability_slots == 0 {
        return false;
    }

    // Calculate booking rate
    let booking_rate = (pattern.actually_booked_slots as u32)
        .saturating_mul(10_000)
        .saturating_div(pattern.total_availability_slots);

    // High booking rate + rapid changes = artificial scarcity
    booking_rate > SCARCITY_BOOKING_RATE_THRESHOLD && pattern.rapid_availability_changes > 3
}

/// Calculate fair availability requirement
pub fn calculate_fair_availability_requirement(
    mentor_rating: u32,      // 0-10000
    specialization_demand: u32, // 0-10000
) -> u32 {
    // Higher rated mentors with in-demand skills should offer more availability
    // but scaled fairly

    let demand_scaling = (specialization_demand as u128)
        .saturating_mul(specialization_demand as u128)
        .saturating_div(10_000) as u32;

    let rating_scaling = (mentor_rating as u128)
        .saturating_mul(mentor_rating as u128)
        .saturating_div(10_000) as u32;

    // Base requirement plus scaling
    let min_hours = BASE_HOURS_PER_WEEK + ((demand_scaling + rating_scaling) / 2_000) as u32;

    min_hours.min(MAX_REQUIRED_HOURS_PER_WEEK)
}

/// Enforce fair pricing with manipulation prevention
pub fn enforce_fair_pricing(
    env: &Env,
    mentor: Address,
    proposed_price: i128,
    market_baseline: i128,
    recent_price_changes: u32,
) -> bool {
    // Check for rapid price changes (market manipulation)
    if recent_price_changes > MAX_PRICE_CHANGES_PER_WEEK {
        return false;
    }

    // Validate against market baseline
    let validation = validate_dynamic_pricing(env, mentor, proposed_price, market_baseline);
    validation.is_valid
}

/// Emergency market stabilization intervention
pub fn trigger_availability_intervention(
    env: &Env,
    shortage_severity: u32, // 0-10000
) -> bool {
    // If more than X% of mentors are unavailable, trigger intervention
    shortage_severity >= EMERGENCY_INTERVENTION_THRESHOLD_BPS
}

/// Constants for availability and pricing protection
pub const BASE_HOURS_PER_WEEK: u32 = 5;
pub const MAX_REQUIRED_HOURS_PER_WEEK: u32 = 40;
pub const SCARCITY_BOOKING_RATE_THRESHOLD: u32 = 9_000; // 90% booked
pub const MAX_PRICE_DEVIATION_BPS: u32 = 2_500; // 25% max deviation from baseline
pub const MAX_PRICE_CHANGES_PER_WEEK: u32 = 3;
pub const PRICE_COORDINATION_THRESHOLD_BPS: u32 = 7_500; // 75% correlation = coordination
pub const EMERGENCY_INTERVENTION_THRESHOLD_BPS: u32 = 8_000; // 80% shortage = emergency
pub const COMMITMENT_ENFORCEMENT_PENALTY_BPS: u32 = 5_000; // 50% penalty for violation
pub const MIN_COMMITMENT_DURATION_SECS: u64 = 604_800; // 1 week
pub const MAX_COMMITMENT_DURATION_SECS: u64 = 31_536_000; // 1 year

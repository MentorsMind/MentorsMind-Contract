//! Deterministic threat-scoring helpers shared by protection contracts.
//!
//! These helpers are storage-agnostic. Callers provide the local counters
//! they already maintain, then persist or act on the returned report.

use soroban_sdk::{contracttype, Env, Symbol};

pub const DEFAULT_DELEGATION_CAP_BPS: u32 = 10_000;
pub const GOVERNANCE_CONCENTRATION_WARN_BPS: u32 = 3_000;
pub const ECONOMIC_VELOCITY_WARN_BPS: u32 = 2_500;
pub const MULTI_VECTOR_RESPONSE_THRESHOLD: u32 = 70;
pub const REVIEW_MANIPULATION_THRESHOLD: u32 = 60;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DelegationConcentrationReport {
    pub total_power: i128,
    pub delegate_power: i128,
    pub concentration_bps: u32,
    pub cap_bps: u32,
    pub cap_exceeded: bool,
    pub emergency_suspend_recommended: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EconomicVelocityReport {
    pub circulating_supply: i128,
    pub observed_volume: i128,
    pub velocity_bps: u32,
    pub concentration_bps: u32,
    pub health_score: u32,
    pub stabilization_required: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MultiVectorThreatReport {
    pub combined_risk_score: u32,
    pub vectors_triggered: u32,
    pub coordinated_response_required: bool,
    pub recommended_action: Symbol,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewQualityReport {
    pub authenticated: bool,
    pub manipulation_risk_score: u32,
    pub reviewer_protection_required: bool,
    pub dispute_required: bool,
}

pub fn assess_delegation_concentration(
    total_power: i128,
    delegate_power: i128,
    cap_bps: u32,
) -> DelegationConcentrationReport {
    let concentration_bps = if total_power > 0 && delegate_power > 0 {
        ((delegate_power as u128).saturating_mul(10_000) / (total_power as u128)) as u32
    } else {
        0
    };
    let effective_cap = cap_bps.min(10_000);

    DelegationConcentrationReport {
        total_power,
        delegate_power,
        concentration_bps,
        cap_bps: effective_cap,
        cap_exceeded: concentration_bps > effective_cap,
        emergency_suspend_recommended: concentration_bps
            >= GOVERNANCE_CONCENTRATION_WARN_BPS.saturating_mul(2),
    }
}

pub fn assess_token_velocity(
    circulating_supply: i128,
    observed_volume: i128,
    concentration_bps: u32,
) -> EconomicVelocityReport {
    let velocity_bps = if circulating_supply > 0 && observed_volume > 0 {
        ((observed_volume as u128).saturating_mul(10_000) / (circulating_supply as u128)) as u32
    } else {
        0
    };

    let mut risk = 0u32;
    if velocity_bps > ECONOMIC_VELOCITY_WARN_BPS {
        risk = risk.saturating_add(35);
    }
    if concentration_bps > GOVERNANCE_CONCENTRATION_WARN_BPS {
        risk = risk.saturating_add(35);
    }
    if velocity_bps > ECONOMIC_VELOCITY_WARN_BPS.saturating_mul(2) {
        risk = risk.saturating_add(20);
    }
    let risk = risk.min(100);

    EconomicVelocityReport {
        circulating_supply,
        observed_volume,
        velocity_bps,
        concentration_bps,
        health_score: 100u32.saturating_sub(risk),
        stabilization_required: risk >= MULTI_VECTOR_RESPONSE_THRESHOLD,
    }
}

pub fn correlate_attack_vectors(
    env: &Env,
    governance_risk: u32,
    economic_risk: u32,
    technical_risk: u32,
    social_risk: u32,
) -> MultiVectorThreatReport {
    let mut vectors = 0u32;
    let mut total = 0u32;
    for risk in [governance_risk, economic_risk, technical_risk, social_risk] {
        if risk >= REVIEW_MANIPULATION_THRESHOLD {
            vectors = vectors.saturating_add(1);
        }
        total = total.saturating_add(risk.min(100));
    }
    let combined = total / 4;
    let coordinated = vectors >= 2 && combined >= MULTI_VECTOR_RESPONSE_THRESHOLD;
    let action = if coordinated {
        Symbol::new(env, "coordinate_response")
    } else if vectors > 0 {
        Symbol::new(env, "monitor_vectors")
    } else {
        Symbol::new(env, "none")
    };

    MultiVectorThreatReport {
        combined_risk_score: combined,
        vectors_triggered: vectors,
        coordinated_response_required: coordinated,
        recommended_action: action,
    }
}

pub fn assess_review_quality(
    verified_session: bool,
    coordination_risk: u32,
    social_proof_risk: u32,
    low_rating_retaliation_signal: bool,
) -> ReviewQualityReport {
    let mut risk = 0u32;
    if !verified_session {
        risk = risk.saturating_add(50);
    }
    risk = risk.saturating_add(coordination_risk.min(100) / 2);
    risk = risk.saturating_add(social_proof_risk.min(100) / 2);
    if low_rating_retaliation_signal {
        risk = risk.saturating_add(25);
    }
    let risk = risk.min(100);

    ReviewQualityReport {
        authenticated: verified_session,
        manipulation_risk_score: risk,
        reviewer_protection_required: low_rating_retaliation_signal
            || risk >= REVIEW_MANIPULATION_THRESHOLD,
        dispute_required: risk >= REVIEW_MANIPULATION_THRESHOLD,
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn delegation_concentration_flags_cap_excess() {
        let report = assess_delegation_concentration(100, 61, 6_000);
        assert!(report.cap_exceeded);
        assert_eq!(report.concentration_bps, 6_100);
    }

    #[test]
    fn token_velocity_requires_stabilization_for_high_velocity_and_concentration() {
        let report = assess_token_velocity(1_000, 600, 4_000);
        assert!(report.stabilization_required);
        assert!(report.health_score < 50);
    }

    #[test]
    fn multi_vector_report_requires_two_high_risk_vectors() {
        let env = Env::default();
        let report = correlate_attack_vectors(&env, 90, 80, 20, 90);
        assert!(report.coordinated_response_required);
        assert_eq!(report.vectors_triggered, 3);
    }

    #[test]
    fn review_quality_requires_authenticated_session() {
        let report = assess_review_quality(false, 0, 0, false);
        assert!(!report.authenticated);
        assert!(report.reviewer_protection_required);
    }
}

// Additional types for staking contract compatibility
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollusionDetection {
    pub detected: bool,
    pub risk_score: u32,
    pub actors_count: u32,
    pub coordination_patterns: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GameTheoryState {
    pub equilibrium_stable: bool,
    pub defection_risk: u32,
    pub cooperation_incentive: u32,
    pub nash_deviation_risk: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncentiveCompatibilityResult {
    pub compatible: bool,
    pub misalignment_risk: u32,
    pub mechanism_integrity: u32,
    pub welfare_efficiency: u32,
}
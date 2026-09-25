//! Platform Exit Strategy and Ecosystem Openness Primitives (#932)
//!
//! Provides seamless data portability, switching cost minimization,
//! competition protection, and ecosystem lock-in prevention.

use soroban_sdk::{contracttype, Address, BytesN, Env, Symbol, Vec};

/// Maximum acceptable switching cost (basis points, where 10000 = 100%)
pub const MAX_SWITCHING_COST_BPS: u32 = 1500; // 15% max friction

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DataPortabilityPackage {
    pub user: Address,
    pub export_hash: BytesN<32>,
    pub session_count: u32,
    pub reputation_score: u32,
    pub exported_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MigrationFacilitationRecord {
    pub user: Address,
    pub destination_platform: Symbol,
    pub switching_cost_bps: u32,
    pub is_facilitated: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompetitionProtectionDecision {
    pub is_fair: bool,
    pub lock_in_detected: bool,
    pub intervention_required: bool,
    pub remedy: Symbol,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DependencyValidationResult {
    pub dependency_id: Symbol,
    pub is_necessary: bool,
    pub is_artificial_lock_in: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExitMonitoringReport {
    pub user: Address,
    pub exit_impediment_score: u32,
    pub migration_supported: bool,
}

/// Validates whether an ecosystem dependency is legitimate or an artificial lock-in.
pub fn validate_dependency_necessity(
    is_core_protocol: bool,
    has_open_standard: bool,
    switching_cost_bps: u32,
) -> DependencyValidationResult {
    let is_artificial_lock_in = !is_core_protocol && !has_open_standard && switching_cost_bps > MAX_SWITCHING_COST_BPS;
    DependencyValidationResult {
        dependency_id: Symbol::short("DEP_EVAL"),
        is_necessary: is_core_protocol || has_open_standard,
        is_artificial_lock_in,
    }
}

/// Evaluates migration conditions and creates a facilitation record.
pub fn facilitate_migration(
    user: &Address,
    destination: &Symbol,
    assessed_switching_cost_bps: u32,
) -> MigrationFacilitationRecord {
    let is_facilitated = assessed_switching_cost_bps <= MAX_SWITCHING_COST_BPS;
    MigrationFacilitationRecord {
        user: user.clone(),
        destination_platform: destination.clone(),
        switching_cost_bps: assessed_switching_cost_bps,
        is_facilitated,
    }
}

/// Makes a competition protection decision to restore competitive fairness.
pub fn evaluate_competition_protection(
    artificial_lock_in_detected: bool,
    exit_impediment_score: u32,
    env: &Env,
) -> CompetitionProtectionDecision {
    let intervention_required = artificial_lock_in_detected || exit_impediment_score > 6000;
    let remedy = if intervention_required {
        Symbol::new(env, "restore_mobility")
    } else {
        Symbol::new(env, "open_ecosystem")
    };

    CompetitionProtectionDecision {
        is_fair: !intervention_required,
        lock_in_detected: artificial_lock_in_detected,
        intervention_required,
        remedy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[test]
    fn test_validate_dependency_necessity() {
        let valid = validate_dependency_necessity(true, true, 500);
        assert!(valid.is_necessary);
        assert!(!valid.is_artificial_lock_in);

        let lock_in = validate_dependency_necessity(false, false, 2500);
        assert!(!lock_in.is_necessary);
        assert!(lock_in.is_artificial_lock_in);
    }

    #[test]
    fn test_facilitate_migration() {
        let env = Env::default();
        let user = Address::generate(&env);
        let dest = Symbol::new(&env, "PLATFORM_B");

        let rec = facilitate_migration(&user, &dest, 500);
        assert!(rec.is_facilitated);
    }

    #[test]
    fn test_evaluate_competition_protection() {
        let env = Env::default();
        let decision = evaluate_competition_protection(false, 1000, &env);
        assert!(decision.is_fair);
        assert!(!decision.intervention_required);

        let decision_lockin = evaluate_competition_protection(true, 7000, &env);
        assert!(!decision_lockin.is_fair);
        assert!(decision_lockin.intervention_required);
    }
}

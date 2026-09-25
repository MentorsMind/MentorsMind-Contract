//! Validator Accountability and Consensus Attack Resistance (#869)
//!
//! Provides:
//! - Protocol-specific validator slashing with graduated penalties
//! - Validator accountability with performance tracking and reputation scoring
//! - Incentive alignment mechanisms ensuring validators support protocol security
//! - Consensus attack detection with automatic penalty and ejection
//! - Real-time validator behavior monitoring with anomaly detection
//! - Emergency consensus recovery with alternative validator selection

use soroban_sdk::{contracttype, symbol_short, Address, BytesN, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum validator reputation score required to participate (0–10 000 bps).
pub const MIN_REPUTATION_SCORE: u32 = 2_000; // 20%

/// Maximum reputation score.
pub const MAX_REPUTATION_SCORE: u32 = 10_000; // 100%

/// Initial reputation score for new validators.
pub const INITIAL_REPUTATION_SCORE: u32 = 5_000; // 50%

/// Reputation earned per successful epoch (positive reinforcement).
pub const REPUTATION_REWARD_PER_EPOCH: u32 = 50; // 0.5%

/// Reputation lost per missed obligation.
pub const REPUTATION_PENALTY_MISSED: u32 = 200; // 2%

/// Reputation lost per equivocation (double-signing).
pub const REPUTATION_PENALTY_EQUIVOCATION: u32 = 2_000; // 20%

/// Reputation lost per consensus attack attempt.
pub const REPUTATION_PENALTY_ATTACK: u32 = 5_000; // 50%

/// Slash amount (basis points of stake) for minor violations.
pub const SLASH_MINOR_BPS: u32 = 100; // 1%

/// Slash amount for major violations (equivocation).
pub const SLASH_MAJOR_BPS: u32 = 1_000; // 10%

/// Slash amount for critical violations (confirmed consensus attack).
pub const SLASH_CRITICAL_BPS: u32 = 5_000; // 50%

/// Number of consecutive missed epochs before a validator is flagged.
pub const MISSED_EPOCH_FLAG_THRESHOLD: u32 = 3;

/// Number of attack signals within the detection window before ejection.
pub const ATTACK_EJECTION_THRESHOLD: u32 = 2;

/// Sliding window for anomaly scoring (6 hours).
pub const ANOMALY_WINDOW_SECS: u64 = 6 * 60 * 60;

/// Cooling-off period for ejected validators before re-admission (30 days).
pub const EJECTION_COOLDOWN_SECS: u64 = 30 * 24 * 60 * 60;

/// Anomaly score threshold above which emergency consensus recovery is triggered.
pub const EMERGENCY_TRIGGER_SCORE: u32 = 8_000; // 80%

/// Minimum validator set size for emergency alternative selection.
pub const MIN_EMERGENCY_VALIDATOR_SET: u32 = 3;

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// Classification of a validator protocol violation.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum ViolationType {
    /// Missed an obligation (vote/block production) — minor.
    MissedEpoch = 1,
    /// Signed conflicting blocks in the same round — major.
    Equivocation = 2,
    /// Colluded to censor transactions or manipulate ordering — critical.
    TransactionCensorship = 3,
    /// Participated in a known consensus-layer attack — critical.
    ConsensusAttack = 4,
    /// Exceeded permissible stake concentration — major.
    StakeConcentration = 5,
}

/// A slashing event record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashingEvent {
    /// Slashed validator address.
    pub validator: Address,
    /// Violation that triggered slashing.
    pub violation: ViolationType,
    /// Slash amount in basis points of their stake.
    pub slash_bps: u32,
    /// Timestamp of the event.
    pub slashed_at: u64,
    /// Evidence hash (SHA-256 of the violation proof).
    pub evidence_hash: soroban_sdk::BytesN<32>,
}

/// Validator performance and reputation record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorRecord {
    /// Validator address.
    pub validator: Address,
    /// Current reputation score (0–10 000 bps).
    pub reputation_score: u32,
    /// Total epochs participated.
    pub epochs_participated: u64,
    /// Total epochs missed.
    pub epochs_missed: u64,
    /// Consecutive epochs missed in the current streak.
    pub consecutive_missed: u32,
    /// Total violations accumulated.
    pub total_violations: u32,
    /// Whether the validator is currently ejected.
    pub ejected: bool,
    /// Timestamp of ejection (0 if not ejected).
    pub ejected_at: u64,
    /// Timestamp after which the validator can re-apply.
    pub readmission_eligible_at: u64,
    /// Cumulative slashed amount in basis points.
    pub total_slashed_bps: u32,
}

/// A detected consensus anomaly.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsensusAnomalyRecord {
    /// Validator whose behavior triggered the anomaly.
    pub validator: Address,
    /// Type of anomaly detected.
    pub anomaly_type: Symbol,
    /// Risk score for this anomaly (0–10 000 bps).
    pub score: u32,
    /// Timestamp of detection.
    pub detected_at: u64,
    /// Whether automatic action has been taken.
    pub auto_actioned: bool,
}

/// Emergency consensus recovery state.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyConsensusState {
    /// Whether emergency mode is active.
    pub active: bool,
    /// Timestamp emergency mode was activated.
    pub activated_at: u64,
    /// Number of validators involved in the triggering attack.
    pub attack_validator_count: u32,
    /// Cumulative network anomaly score that triggered emergency.
    pub trigger_score: u32,
    /// Alternative validator set selected for emergency operation.
    pub alternative_validators: Vec<Address>,
}

/// Incentive alignment assessment for a validator.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IncentiveAlignmentScore {
    /// Validator address.
    pub validator: Address,
    /// Economic alignment score (0–10 000 bps; higher = better aligned).
    pub alignment_score: u32,
    /// Whether the validator's incentives are considered aligned.
    pub aligned: bool,
    /// Risk factors detected (count).
    pub risk_factors: u32,
}

/// Active validator set for an epoch.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidatorSet {
    /// Epoch this validator set is active for
    pub epoch: u64,
    /// Ordered list of active validators
    pub validators: Vec<Address>,
    /// Stake amounts for each validator (parallel to validators)
    pub stakes: Vec<i128>,
    /// Quorum threshold (2f+1 where f = floor(n/3))
    pub quorum_threshold: u32,
    /// Total stake of all validators in this set
    pub total_stake: i128,
    /// Timestamp when this set was selected
    pub selected_at: u64,
    /// Duration this set is valid (in seconds)
    pub duration_secs: u64,
    /// VRF output hash used for selection
    pub vrf_seed: BytesN<32>,
}

/// Slashing penalty record with full context.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SlashingPenalty {
    /// Validator that was penalized
    pub validator: Address,
    /// The violation type
    pub violation: ViolationType,
    /// Slash amount in basis points
    pub slash_bps: u32,
    /// Estimated slashed amount in token units
    pub slashed_amount: i128,
    /// When the penalty was applied
    pub applied_at: u64,
    /// Evidence hash for the violation
    pub evidence_hash: BytesN<32>,
    /// Whether the validator was ejected
    pub ejected: bool,
    /// Readmission eligible timestamp (if ejected)
    pub readmission_at: Option<u64>,
}

/// Long-range attack protection checkpoint.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LongRangeCheckpoint {
    /// Checkpoint sequence number
    pub sequence: u64,
    /// Ledger sequence at checkpoint
    pub ledger_sequence: u32,
    /// State root hash
    pub state_root: BytesN<32>,
    /// Validator set hash at checkpoint
    pub validator_set_hash: BytesN<32>,
    /// Timestamp
    pub timestamp: u64,
    /// Number of validator attestations
    pub attestation_count: u32,
    /// Whether this checkpoint is finalized (irreversible)
    pub finalized: bool,
}

// ---------------------------------------------------------------------------
// Validator registration and management
// ---------------------------------------------------------------------------

/// Register a new validator.
pub fn register_validator(env: &Env, validator: &Address) -> ValidatorRecord {
    let record = ValidatorRecord {
        validator: validator.clone(),
        reputation_score: INITIAL_REPUTATION_SCORE,
        epochs_participated: 0,
        epochs_missed: 0,
        consecutive_missed: 0,
        total_violations: 0,
        ejected: false,
        ejected_at: 0,
        readmission_eligible_at: 0,
        total_slashed_bps: 0,
    };

    let key = validator_key(validator);
    env.storage().persistent().set(&key, &record);

    env.events().publish(
        (symbol_short!("valacct"), symbol_short!("register")),
        (validator.clone(), INITIAL_REPUTATION_SCORE),
    );

    record
}

/// Record a successful epoch participation.
pub fn record_epoch_participation(env: &Env, validator: &Address) {
    let key = validator_key(validator);
    let mut record: ValidatorRecord = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic!("validator not registered"));

    record.epochs_participated = record.epochs_participated.saturating_add(1);
    record.consecutive_missed = 0;
    record.reputation_score = record
        .reputation_score
        .saturating_add(REPUTATION_REWARD_PER_EPOCH)
        .min(MAX_REPUTATION_SCORE);

    env.storage().persistent().set(&key, &record);
}

/// Record a missed epoch and apply graduated penalty.
pub fn record_missed_epoch(env: &Env, validator: &Address) -> bool {
    let key = validator_key(validator);
    let mut record: ValidatorRecord = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic!("validator not registered"));

    record.epochs_missed = record.epochs_missed.saturating_add(1);
    record.consecutive_missed = record.consecutive_missed.saturating_add(1);
    record.reputation_score = record
        .reputation_score
        .saturating_sub(REPUTATION_PENALTY_MISSED);

    let flagged = record.consecutive_missed >= MISSED_EPOCH_FLAG_THRESHOLD;

    env.storage().persistent().set(&key, &record);

    if flagged {
        env.events().publish(
            (symbol_short!("valacct"), symbol_short!("miss_flag")),
            (validator.clone(), record.consecutive_missed),
        );
    }

    flagged
}

// ---------------------------------------------------------------------------
// Slashing
// ---------------------------------------------------------------------------

/// Apply a slashing event for a protocol violation.
///
/// Returns the applied slash in basis points and the new reputation score.
pub fn apply_slash(
    env: &Env,
    validator: &Address,
    violation: ViolationType,
    evidence_hash: soroban_sdk::BytesN<32>,
) -> (u32, u32) {
    let key = validator_key(validator);
    let mut record: ValidatorRecord = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| panic!("validator not registered"));

    let slash_bps = graduated_slash_bps(violation);
    let reputation_penalty = match violation {
        ViolationType::MissedEpoch => REPUTATION_PENALTY_MISSED,
        ViolationType::Equivocation | ViolationType::StakeConcentration => {
            REPUTATION_PENALTY_EQUIVOCATION
        }
        ViolationType::TransactionCensorship | ViolationType::ConsensusAttack => {
            REPUTATION_PENALTY_ATTACK
        }
    };

    record.reputation_score = record.reputation_score.saturating_sub(reputation_penalty);
    record.total_slashed_bps = record
        .total_slashed_bps
        .saturating_add(slash_bps)
        .min(10_000);
    record.total_violations = record.total_violations.saturating_add(1);

    // Auto-eject for critical violations or reputation below minimum.
    let should_eject = matches!(
        violation,
        ViolationType::ConsensusAttack | ViolationType::Equivocation
    ) || record.reputation_score < MIN_REPUTATION_SCORE;

    if should_eject && !record.ejected {
        record.ejected = true;
        record.ejected_at = env.ledger().timestamp();
        record.readmission_eligible_at =
            env.ledger().timestamp().saturating_add(EJECTION_COOLDOWN_SECS);

        env.events().publish(
            (symbol_short!("valacct"), symbol_short!("ejected")),
            (
                validator.clone(),
                violation as u32,
                record.reputation_score,
                env.ledger().timestamp(),
            ),
        );
    }

    let new_reputation = record.reputation_score;
    env.storage().persistent().set(&key, &record);

    let event = SlashingEvent {
        validator: validator.clone(),
        violation,
        slash_bps,
        slashed_at: env.ledger().timestamp(),
        evidence_hash,
    };
    let slash_key = (
        symbol_short!("vslash"),
        validator.clone(),
        env.ledger().sequence(),
    );
    env.storage().persistent().set(&slash_key, &event);

    env.events().publish(
        (symbol_short!("valacct"), symbol_short!("slashed")),
        (validator.clone(), slash_bps, violation as u32),
    );

    (slash_bps, new_reputation)
}

/// Calculate graduated slash basis points for a violation type.
pub fn graduated_slash_bps(violation: ViolationType) -> u32 {
    match violation {
        ViolationType::MissedEpoch => SLASH_MINOR_BPS,
        ViolationType::StakeConcentration => SLASH_MINOR_BPS * 2,
        ViolationType::Equivocation => SLASH_MAJOR_BPS,
        ViolationType::TransactionCensorship => SLASH_MAJOR_BPS * 2,
        ViolationType::ConsensusAttack => SLASH_CRITICAL_BPS,
    }
}

// ---------------------------------------------------------------------------
// Consensus attack detection and anomaly monitoring
// ---------------------------------------------------------------------------

/// Detect a consensus attack signal from a validator.
///
/// Records the anomaly, applies penalties, and triggers emergency recovery
/// if the aggregate network risk exceeds `EMERGENCY_TRIGGER_SCORE`.
///
/// Returns the new network anomaly score.
pub fn detect_consensus_attack(
    env: &Env,
    validator: &Address,
    attack_type: Symbol,
    evidence_hash: soroban_sdk::BytesN<32>,
    network_validators: &Vec<Address>,
) -> u32 {
    // Record the anomaly.
    let anomaly = ConsensusAnomalyRecord {
        validator: validator.clone(),
        anomaly_type: attack_type.clone(),
        score: REPUTATION_PENALTY_ATTACK,
        detected_at: env.ledger().timestamp(),
        auto_actioned: true,
    };
    let anomaly_key = (symbol_short!("vanom"), validator.clone(), env.ledger().timestamp());
    env.storage().persistent().set(&anomaly_key, &anomaly);

    // Apply critical slashing.
    apply_slash(env, validator, ViolationType::ConsensusAttack, evidence_hash);

    // Compute network anomaly score: fraction of ejected validators * 10_000.
    let network_score = compute_network_anomaly_score(env, network_validators);

    env.events().publish(
        (symbol_short!("valacct"), symbol_short!("atk_det")),
        (
            validator.clone(),
            attack_type,
            network_score,
            env.ledger().timestamp(),
        ),
    );

    // Trigger emergency if network score exceeds threshold.
    if network_score >= EMERGENCY_TRIGGER_SCORE {
        let alternatives = select_healthy_validators(env, network_validators);
        activate_emergency_consensus(env, network_score, &alternatives);
    }

    network_score
}

/// Compute the aggregate anomaly score for the validator network.
///
/// Score is the fraction of ejected/penalized validators weighted by their
/// penalty severity, expressed in basis points (0–10 000).
pub fn compute_network_anomaly_score(env: &Env, validators: &Vec<Address>) -> u32 {
    let total = validators.len();
    if total == 0 {
        return 0;
    }

    let mut risky_weight: u32 = 0;
    for v in validators.iter() {
        if let Some(record) = get_validator_record(env, &v) {
            if record.ejected {
                risky_weight = risky_weight.saturating_add(10_000 / total as u32);
            } else if record.reputation_score < MIN_REPUTATION_SCORE * 2 {
                risky_weight =
                    risky_weight.saturating_add(5_000 / total as u32);
            }
        }
    }

    risky_weight.min(10_000)
}

// ---------------------------------------------------------------------------
// Incentive alignment
// ---------------------------------------------------------------------------

/// Assess whether a validator's economic incentives are aligned with
/// protocol security objectives.
///
/// Computes an alignment score based on:
/// - Reputation score (higher = more aligned)
/// - Slash history (more slashes = less aligned)
/// - Missed epoch ratio (higher miss rate = less aligned)
/// - Whether the validator is ejected (never aligned if ejected)
pub fn assess_incentive_alignment(
    env: &Env,
    validator: &Address,
) -> IncentiveAlignmentScore {
    let record = get_validator_record(env, validator).unwrap_or(ValidatorRecord {
        validator: validator.clone(),
        reputation_score: 0,
        epochs_participated: 0,
        epochs_missed: 0,
        consecutive_missed: 0,
        total_violations: 0,
        ejected: false,
        ejected_at: 0,
        readmission_eligible_at: 0,
        total_slashed_bps: 0,
    });

    if record.ejected {
        return IncentiveAlignmentScore {
            validator: validator.clone(),
            alignment_score: 0,
            aligned: false,
            risk_factors: 3,
        };
    }

    let mut risk_factors: u32 = 0;
    let mut alignment_score: u32 = record.reputation_score;

    // Deduct for slash history.
    alignment_score = alignment_score.saturating_sub(record.total_slashed_bps / 5);
    if record.total_slashed_bps > 0 {
        risk_factors += 1;
    }

    // Deduct for high miss rate.
    let total_epochs = record
        .epochs_participated
        .saturating_add(record.epochs_missed);
    if total_epochs > 0 {
        let miss_rate_bps = (record.epochs_missed as u64 * 10_000 / total_epochs) as u32;
        alignment_score = alignment_score.saturating_sub(miss_rate_bps / 10);
        if miss_rate_bps > 2_000 {
            // > 20% miss rate
            risk_factors += 1;
        }
    }

    // Deduct for repeated violations.
    if record.total_violations >= 3 {
        alignment_score = alignment_score.saturating_sub(1_000);
        risk_factors += 1;
    }

    alignment_score = alignment_score.min(MAX_REPUTATION_SCORE);
    let aligned = alignment_score >= MIN_REPUTATION_SCORE && risk_factors == 0;

    IncentiveAlignmentScore {
        validator: validator.clone(),
        alignment_score,
        aligned,
        risk_factors,
    }
}

// ---------------------------------------------------------------------------
// Emergency consensus recovery
// ---------------------------------------------------------------------------

/// Activate emergency consensus mode, selecting a clean validator set.
pub fn activate_emergency_consensus(
    env: &Env,
    trigger_score: u32,
    alternative_validators: &Vec<Address>,
) {
    let state = EmergencyConsensusState {
        active: true,
        activated_at: env.ledger().timestamp(),
        attack_validator_count: 0,
        trigger_score,
        alternative_validators: alternative_validators.clone(),
    };

    env.storage()
        .persistent()
        .set(&emergency_state_key(), &state);

    env.events().publish(
        (symbol_short!("valacct"), symbol_short!("emer_act")),
        (
            trigger_score,
            alternative_validators.len() as u32,
            env.ledger().timestamp(),
        ),
    );
}

/// Deactivate emergency consensus mode once the threat is resolved.
pub fn deactivate_emergency_consensus(env: &Env) {
    if let Some(mut state) = get_emergency_state(env) {
        state.active = false;
        env.storage()
            .persistent()
            .set(&emergency_state_key(), &state);

        env.events().publish(
            (symbol_short!("valacct"), symbol_short!("emer_off")),
            env.ledger().timestamp(),
        );
    }
}

/// Get the current emergency consensus state.
pub fn get_emergency_state(env: &Env) -> Option<EmergencyConsensusState> {
    env.storage().persistent().get(&emergency_state_key())
}

/// Check if emergency consensus mode is currently active.
pub fn is_emergency_active(env: &Env) -> bool {
    get_emergency_state(env)
        .map(|s| s.active)
        .unwrap_or(false)
}

/// Select validators with sufficient reputation for emergency operation.
pub fn select_healthy_validators(
    env: &Env,
    all_validators: &Vec<Address>,
) -> Vec<Address> {
    let mut healthy = Vec::new(env);
    for v in all_validators.iter() {
        if let Some(record) = get_validator_record(env, &v) {
            if !record.ejected && record.reputation_score >= MIN_REPUTATION_SCORE {
                healthy.push_back(v);
            }
        }
    }
    healthy
}

// ---------------------------------------------------------------------------
// View functions
// ---------------------------------------------------------------------------

/// Get a validator's record.
pub fn get_validator_record(env: &Env, validator: &Address) -> Option<ValidatorRecord> {
    let key = validator_key(validator);
    env.storage().persistent().get(&key)
}

/// Check whether a validator is currently ejected.
pub fn is_validator_ejected(env: &Env, validator: &Address) -> bool {
    get_validator_record(env, validator)
        .map(|r| r.ejected)
        .unwrap_or(false)
}

/// Check whether an ejected validator is eligible for re-admission.
pub fn is_readmission_eligible(env: &Env, validator: &Address) -> bool {
    get_validator_record(env, validator)
        .map(|r| {
            r.ejected && env.ledger().timestamp() >= r.readmission_eligible_at
        })
        .unwrap_or(false)
}

/// Re-admit a validator after the cooling-off period.
///
/// Resets ejection status and restores minimum reputation.
pub fn readmit_validator(env: &Env, validator: &Address) {
    let key = validator_key(validator);
    let mut record: ValidatorRecord = env
        .storage()
        .persistent()
        .get(&key)
        .expect("validator not registered");

    if !record.ejected {
        panic!("validator is not ejected");
    }
    if env.ledger().timestamp() < record.readmission_eligible_at {
        panic!("readmission cooldown not elapsed");
    }

    record.ejected = false;
    record.ejected_at = 0;
    record.readmission_eligible_at = 0;
    record.consecutive_missed = 0;
    // Restore minimum reputation on readmission.
    record.reputation_score = MIN_REPUTATION_SCORE;
    env.storage().persistent().set(&key, &record);

    env.events().publish(
        (symbol_short!("valacct"), symbol_short!("readmit")),
        (validator.clone(), env.ledger().timestamp()),
    );
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn validator_key(validator: &Address) -> (Symbol, Address) {
    (symbol_short!("valrec"), validator.clone())
}

fn emergency_state_key() -> Symbol {
    symbol_short!("val_emer")
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::Address as _;
    extern crate std;
    use super::*;
    use soroban_sdk::{testutils::Ledger, BytesN, Env};

    fn env_at(ts: u64) -> Env {
        let env = Env::default();
        env.ledger().with_mut(|l| l.timestamp = ts);
        env
    }

    fn dummy_evidence(env: &Env) -> BytesN<32> {
        BytesN::from_array(env, &[0xABu8; 32])
    }

    #[test]
    fn test_register_and_participate() {
        let env = env_at(1_000);
        let v = soroban_sdk::Address::generate(&env);
        register_validator(&env, &v);

        let rec = get_validator_record(&env, &v).unwrap();
        assert_eq!(rec.reputation_score, INITIAL_REPUTATION_SCORE);

        record_epoch_participation(&env, &v);
        let rec = get_validator_record(&env, &v).unwrap();
        assert_eq!(
            rec.reputation_score,
            INITIAL_REPUTATION_SCORE + REPUTATION_REWARD_PER_EPOCH
        );
    }

    #[test]
    fn test_missed_epoch_penalty_and_flagging() {
        let env = env_at(1_000);
        let v = soroban_sdk::Address::generate(&env);
        register_validator(&env, &v);

        // 2 misses — not yet flagged.
        assert!(!record_missed_epoch(&env, &v));
        assert!(!record_missed_epoch(&env, &v));

        // 3rd miss — flagged.
        assert!(record_missed_epoch(&env, &v));

        let rec = get_validator_record(&env, &v).unwrap();
        assert_eq!(rec.consecutive_missed, 3);
    }

    #[test]
    fn test_critical_slash_causes_ejection() {
        let env = env_at(1_000);
        let v = soroban_sdk::Address::generate(&env);
        register_validator(&env, &v);

        let (slash_bps, _new_rep) = apply_slash(
            &env,
            &v,
            ViolationType::ConsensusAttack,
            dummy_evidence(&env),
        );

        assert_eq!(slash_bps, SLASH_CRITICAL_BPS);
        assert!(is_validator_ejected(&env, &v));
    }

    #[test]
    fn test_equivocation_slash_and_ejection() {
        let env = env_at(1_000);
        let v = soroban_sdk::Address::generate(&env);
        register_validator(&env, &v);

        let (slash_bps, _) = apply_slash(
            &env,
            &v,
            ViolationType::Equivocation,
            dummy_evidence(&env),
        );

        assert_eq!(slash_bps, SLASH_MAJOR_BPS);
        assert!(is_validator_ejected(&env, &v));
    }

    #[test]
    fn test_readmission_after_cooldown() {
        let env = env_at(1_000);
        let v = soroban_sdk::Address::generate(&env);
        register_validator(&env, &v);

        apply_slash(&env, &v, ViolationType::ConsensusAttack, dummy_evidence(&env));
        assert!(is_validator_ejected(&env, &v));

        // Not yet eligible.
        assert!(!is_readmission_eligible(&env, &v));

        // Advance past cooling-off.
        env.ledger()
            .with_mut(|l| l.timestamp = 1_000 + EJECTION_COOLDOWN_SECS + 1);
        assert!(is_readmission_eligible(&env, &v));

        readmit_validator(&env, &v);
        assert!(!is_validator_ejected(&env, &v));
        let rec = get_validator_record(&env, &v).unwrap();
        assert_eq!(rec.reputation_score, MIN_REPUTATION_SCORE);
    }

    #[test]
    fn test_incentive_alignment_scoring() {
        let env = env_at(1_000);
        let v = soroban_sdk::Address::generate(&env);
        register_validator(&env, &v);

        // Pristine validator → should be aligned.
        let score = assess_incentive_alignment(&env, &v);
        assert!(score.aligned);
        assert_eq!(score.risk_factors, 0);

        // After an attack slash → not aligned.
        apply_slash(&env, &v, ViolationType::ConsensusAttack, dummy_evidence(&env));
        let score_after = assess_incentive_alignment(&env, &v);
        assert!(!score_after.aligned);
    }

    #[test]
    fn test_emergency_activation_and_deactivation() {
        let env = env_at(1_000);

        let alternatives = Vec::new(&env);
        activate_emergency_consensus(&env, 9_000, &alternatives);
        assert!(is_emergency_active(&env));

        deactivate_emergency_consensus(&env);
        assert!(!is_emergency_active(&env));
    }

    #[test]
    fn test_network_anomaly_score_zero_for_clean_set() {
        let env = env_at(1_000);
        let validators: Vec<soroban_sdk::Address> = {
            let mut v = Vec::new(&env);
            for _ in 0..3 {
                let addr = soroban_sdk::Address::generate(&env);
                register_validator(&env, &addr);
                v.push_back(addr);
            }
            v
        };

        let score = compute_network_anomaly_score(&env, &validators);
        assert_eq!(score, 0, "clean validator set should have 0 anomaly score");
    }

    #[test]
    fn test_select_healthy_excludes_ejected() {
        let env = env_at(1_000);
        let v1 = soroban_sdk::Address::generate(&env);
        let v2 = soroban_sdk::Address::generate(&env);
        register_validator(&env, &v1);
        register_validator(&env, &v2);

        // Eject v1.
        apply_slash(&env, &v1, ViolationType::ConsensusAttack, dummy_evidence(&env));

        let mut all = Vec::new(&env);
        all.push_back(v1.clone());
        all.push_back(v2.clone());

        let healthy = select_healthy_validators(&env, &all);
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy.get(0).unwrap(), v2);
    }

    #[test]
    fn test_graduated_slash_amounts() {
        assert_eq!(graduated_slash_bps(ViolationType::MissedEpoch), SLASH_MINOR_BPS);
        assert_eq!(graduated_slash_bps(ViolationType::Equivocation), SLASH_MAJOR_BPS);
        assert_eq!(graduated_slash_bps(ViolationType::ConsensusAttack), SLASH_CRITICAL_BPS);
        assert!(
            graduated_slash_bps(ViolationType::ConsensusAttack)
                > graduated_slash_bps(ViolationType::Equivocation)
        );
    }
}

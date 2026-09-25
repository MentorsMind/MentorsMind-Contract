//! Mentor Wellness Protection with Workload Monitoring and Burnout Prevention
//!
//! Implements mentor capacity tracking, burnout risk assessment, fair session
//! distribution, and emergency protection mechanisms.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec, Map, vec};

/// Maximum concurrent sessions per mentor
pub const MAX_CONCURRENT_SESSIONS: u32 = 5;
/// Maximum weekly hours for mentors
pub const MAX_WEEKLY_HOURS: u32 = 40;
/// Minimum rest period between sessions (hours)
pub const MIN_REST_HOURS: u32 = 1;
/// Burnout risk threshold (basis points)
pub const BURNOUT_RISK_THRESHOLD_BPS: u32 = 7000; // 70%
/// Mandatory rest period after max hours (hours)
pub const MANDATORY_REST_HOURS: u32 = 48;
/// Session difficulty weights
pub const DIFFICULTY_WEIGHTS: [u32; 4] = [10000, 15000, 20000, 30000]; // Easy, Medium, Hard, Expert

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SessionDifficulty {
    Easy = 0,
    Medium = 1,
    Hard = 2,
    Expert = 3,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MentorWorkload {
    pub mentor: Address,
    pub active_sessions: u32,
    pub weekly_hours: u32,
    pub weekly_weighted_load: u32, // Hours * difficulty weight
    pub sessions_this_week: Vec<Symbol>,
    pub last_session_end: u64,
    pub rest_until: u64, // Timestamp when mentor can accept new sessions
    pub burnout_risk_bps: u32,
    pub updated_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BurnoutRiskAssessment {
    pub mentor: Address,
    pub risk_level: Symbol, // "low", "moderate", "high", "critical"
    pub risk_score_bps: u32,
    pub contributing_factors: Vec<Symbol>,
    pub recommended_actions: Vec<Symbol>,
    pub assessment_timestamp: u64,
    pub next_assessment_due: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionDistributionRequest {
    pub session_id: Symbol,
    pub difficulty: SessionDifficulty,
    pub estimated_hours: u32,
    pub preferred_mentors: Vec<Address>,
    pub required_skills: Vec<Symbol>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FairDistributionResult {
    pub session_id: Symbol,
    pub assigned_mentor: Address,
    pub alternative_mentors: Vec<Address>,
    pub fairness_score_bps: u32,
    pub workload_balance_bps: u32,
    pub mentor_preference_met: bool,
    pub assigned_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WellnessIntervention {
    pub mentor: Address,
    pub intervention_type: Symbol, // "rest_mandated", "load_reduced", "support_offered", "emergency"
    pub trigger_reason: Symbol,
    pub duration_hours: u32,
    pub initiated_at: u64,
    pub expires_at: u64,
    pub auto_lift: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyProtection {
    pub mentor: Address,
    pub protection_type: Symbol, // "session_redistribution", "crisis_intervention", "temporary_pause"
    pub affected_sessions: Vec<Symbol>,
    pub redistributed_to: Vec<Address>,
    pub initiated_by: Address,
    pub initiated_at: u64,
    pub resolved_at: Option<u64>,
    pub notes: Symbol,
}

/// Update mentor workload after session registration/completion
pub fn update_mentor_workload(
    env: &Env,
    mentor: &Address,
    session_id: &Symbol,
    difficulty: SessionDifficulty,
    hours: u32,
    is_start: bool, // true = session starting, false = session ending
) -> MentorWorkload {
    let _workload_key = mentor.clone(); // Would use proper storage key in contract
    let mut workload: MentorWorkload = MentorWorkload {
        mentor: mentor.clone(),
        active_sessions: 0,
        weekly_hours: 0,
        weekly_weighted_load: 0,
        sessions_this_week: Vec::new(env),
        last_session_end: 0,
        rest_until: 0,
        burnout_risk_bps: 0,
        updated_at: env.ledger().timestamp(),
    };
    
    // In practice, would load from storage
    // This is the logic for updating
    
    let difficulty_weight = DIFFICULTY_WEIGHTS[difficulty as u32 as usize];
    let weighted_hours = (hours as u64 * difficulty_weight as u64 / 10000) as u32;
    
    if is_start {
        workload.active_sessions = workload.active_sessions.saturating_add(1);
        workload.weekly_hours = workload.weekly_hours.saturating_add(hours);
        workload.weekly_weighted_load = workload.weekly_weighted_load.saturating_add(weighted_hours);
        workload.sessions_this_week.push_back(session_id.clone());
    } else {
        workload.active_sessions = workload.active_sessions.saturating_sub(1);
        workload.last_session_end = env.ledger().timestamp();
        // Enforce minimum rest period
        workload.rest_until = env.ledger().timestamp() + (MIN_REST_HOURS as u64 * 3600);
    }
    
    workload.updated_at = env.ledger().timestamp();
    workload.burnout_risk_bps = calculate_burnout_risk(&workload);
    
    workload
}

/// Calculate burnout risk based on workload metrics
pub fn calculate_burnout_risk(workload: &MentorWorkload) -> u32 {
    let mut risk = 0u32;
    
    // Active sessions factor (0-3000 bps)
    if workload.active_sessions > MAX_CONCURRENT_SESSIONS {
        risk = risk.saturating_add(3000);
    } else {
        risk = risk.saturating_add((workload.active_sessions * 3000) / MAX_CONCURRENT_SESSIONS);
    }
    
    // Weekly hours factor (0-3000 bps)
    if workload.weekly_hours > MAX_WEEKLY_HOURS {
        risk = risk.saturating_add(3000);
    } else {
        risk = risk.saturating_add((workload.weekly_hours * 3000) / MAX_WEEKLY_HOURS);
    }
    
    // Weighted load factor (0-2000 bps)
    let max_weighted = MAX_WEEKLY_HOURS * 3; // Assuming max difficulty
    if workload.weekly_weighted_load > max_weighted {
        risk = risk.saturating_add(2000);
    } else if max_weighted > 0 {
        risk = risk.saturating_add((workload.weekly_weighted_load * 2000) / max_weighted);
    }
    
    // Rest compliance factor (0-2000 bps)
    let now = 0u64; // Would use env.ledger().timestamp() in contract
    if workload.rest_until > now {
        risk = risk.saturating_add(2000); // Currently in mandatory rest
    }
    
    risk.min(10000)
}

/// Assess burnout risk with detailed factors
pub fn assess_burnout_risk(
    env: &Env,
    workload: &MentorWorkload,
) -> BurnoutRiskAssessment {
    let risk_score = workload.burnout_risk_bps;
    let mut factors = Vec::new(env);
    let mut actions = Vec::new(env);
    
    if workload.active_sessions >= MAX_CONCURRENT_SESSIONS {
        factors.push_back(Symbol::new(env, "max_sessions"));
        actions.push_back(Symbol::new(env, "reduce_load"));
    }
    
    if workload.weekly_hours >= MAX_WEEKLY_HOURS {
        factors.push_back(Symbol::new(env, "max_hours"));
        actions.push_back(Symbol::new(env, "mandate_rest"));
    }
    
    if workload.rest_until > env.ledger().timestamp() {
        factors.push_back(Symbol::new(env, "in_rest_period"));
        actions.push_back(Symbol::new(env, "wait_rest"));
    }
    
    let (risk_level, recommended_actions) = if risk_score >= 9000 {
        (Symbol::new(env, "critical"), vec![&env, Symbol::new(env, "emergency_pause"), Symbol::new(env, "redistribute_sessions")])
    } else if risk_score >= BURNOUT_RISK_THRESHOLD_BPS {
        (Symbol::new(env, "high"), vec![&env, Symbol::new(env, "mandate_rest"), Symbol::new(env, "reduce_new_sessions")])
    } else if risk_score >= 5000 {
        (Symbol::new(env, "moderate"), vec![&env, Symbol::new(env, "monitor_closely"), Symbol::new(env, "offer_support")])
    } else {
        (Symbol::new(env, "low"), vec![&env, Symbol::new(env, "continue_monitoring")])
    };
    
    for action in recommended_actions.iter() {
        actions.push_back(action);
    }
    
    BurnoutRiskAssessment {
        mentor: workload.mentor.clone(),
        risk_level,
        risk_score_bps: risk_score,
        contributing_factors: factors,
        recommended_actions: actions,
        assessment_timestamp: env.ledger().timestamp(),
        next_assessment_due: env.ledger().timestamp() + 3600, // 1 hour
    }
}

/// Distribute sessions fairly among mentors
pub fn distribute_sessions_fairly(
    env: &Env,
    request: &SessionDistributionRequest,
    available_mentors: &Vec<Address>,
    mentor_workloads: &Map<Address, MentorWorkload>,
) -> FairDistributionResult {
    let mut best_mentor: Option<Address> = None;
    let mut best_score = 0u32;
    let mut alternatives = Vec::new(env);
    
    for mentor in available_mentors.iter() {
        if let Some(workload) = mentor_workloads.get(mentor.clone()) {
            // Skip if mentor at capacity or in rest period
            if workload.active_sessions >= MAX_CONCURRENT_SESSIONS {
                continue;
            }
            if workload.weekly_hours + request.estimated_hours > MAX_WEEKLY_HOURS {
                continue;
            }
            if workload.rest_until > env.ledger().timestamp() {
                continue;
            }
            
            // Calculate fairness score (lower workload = higher score)
            let capacity_remaining = MAX_CONCURRENT_SESSIONS - workload.active_sessions;
            let hours_remaining = MAX_WEEKLY_HOURS - workload.weekly_hours;
            let difficulty_weight = DIFFICULTY_WEIGHTS[request.difficulty.clone() as u32 as usize];
            let weighted_load_factor = if workload.weekly_weighted_load > 0 {
                10000 - (workload.weekly_weighted_load * 10000 / (MAX_WEEKLY_HOURS * 3)).min(10000)
            } else {
                10000
            };
            
            let score = (capacity_remaining * 2000) + 
                       (hours_remaining * 1000 / MAX_WEEKLY_HOURS) * 3000 +
                       (weighted_load_factor * 5000 / 10000);
            
            // Bonus for mentor preference
            let preference_bonus = if request.preferred_mentors.contains(&mentor) { 2000 } else { 0 };
            let total_score = score + preference_bonus;
            
            if total_score > best_score {
                if let Some(prev_best) = best_mentor {
                    alternatives.push_back(prev_best);
                }
                best_mentor = Some(mentor.clone());
                best_score = total_score;
            } else {
                alternatives.push_back(mentor.clone());
            }
        }
    }
    
    let assigned = best_mentor.unwrap_or_else(|| {
        // Fallback: first available mentor
        available_mentors.get(0).unwrap_or_else(|| Address::from_str(&env, "fallback"))
    });
    
    let mentor_preference_met = request.preferred_mentors.contains(&assigned);
    
    FairDistributionResult {
        session_id: request.session_id.clone(),
        assigned_mentor: assigned.clone(),
        alternative_mentors: alternatives,
        fairness_score_bps: best_score,
        workload_balance_bps: best_score, // Simplified
        mentor_preference_met,
        assigned_at: env.ledger().timestamp(),
    }
}

/// Initiate wellness intervention
pub fn initiate_intervention(
    env: &Env,
    mentor: &Address,
    intervention_type: Symbol,
    trigger_reason: Symbol,
    duration_hours: u32,
    _initiated_by: &Address,
) -> WellnessIntervention {
    let now = env.ledger().timestamp();
    
    WellnessIntervention {
        mentor: mentor.clone(),
        intervention_type,
        trigger_reason,
        duration_hours,
        initiated_at: now,
        expires_at: now + (duration_hours as u64 * 3600),
        auto_lift: true,
    }
}

/// Emergency protection for mentor wellness
pub fn activate_emergency_protection(
    env: &Env,
    mentor: &Address,
    protection_type: Symbol,
    affected_sessions: &Vec<Symbol>,
    redistributed_to: &Vec<Address>,
    initiated_by: &Address,
    notes: Symbol,
) -> EmergencyProtection {
    EmergencyProtection {
        mentor: mentor.clone(),
        protection_type,
        affected_sessions: affected_sessions.clone(),
        redistributed_to: redistributed_to.clone(),
        initiated_by: initiated_by.clone(),
        initiated_at: env.ledger().timestamp(),
        resolved_at: None,
        notes,
    }
}

/// Check if mentor can accept new session
pub fn can_accept_session(
    env: &Env,
    workload: &MentorWorkload,
    additional_hours: u32,
) -> (bool, Symbol) {
    if workload.active_sessions >= MAX_CONCURRENT_SESSIONS {
        return (false, Symbol::new(env, "max_sessions_reached"));
    }
    
    if workload.weekly_hours + additional_hours > MAX_WEEKLY_HOURS {
        return (false, Symbol::new(env, "weekly_hours_exceeded"));
    }
    
    if workload.rest_until > env.ledger().timestamp() {
        return (false, Symbol::new(env, "in_mandatory_rest"));
    }
    
    if workload.burnout_risk_bps >= BURNOUT_RISK_THRESHOLD_BPS {
        return (false, Symbol::new(env, "high_burnout_risk"));
    }
    
    (true, Symbol::new(env, "ok"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env, Symbol};

    #[test]
    fn test_calculate_burnout_risk_low() {
        let env = Env::default();
        let workload = MentorWorkload {
            mentor: Address::generate(&env),
            active_sessions: 1,
            weekly_hours: 10,
            weekly_weighted_load: 10000,
            sessions_this_week: Vec::new(&env),
            last_session_end: 0,
            rest_until: 0,
            burnout_risk_bps: 0,
            updated_at: env.ledger().timestamp(),
        };
        
        let risk = calculate_burnout_risk(&workload);
        assert!(risk < 5000); // Low risk
    }

    #[test]
    fn test_calculate_burnout_risk_high() {
        let env = Env::default();
        let workload = MentorWorkload {
            mentor: Address::generate(&env),
            active_sessions: MAX_CONCURRENT_SESSIONS,
            weekly_hours: MAX_WEEKLY_HOURS,
            weekly_weighted_load: MAX_WEEKLY_HOURS * 3,
            sessions_this_week: Vec::new(&env),
            last_session_end: 0,
            rest_until: env.ledger().timestamp() + 7200, // In rest period
            burnout_risk_bps: 0,
            updated_at: env.ledger().timestamp(),
        };
        
        let risk = calculate_burnout_risk(&workload);
        assert!(risk >= BURNOUT_RISK_THRESHOLD_BPS); // High risk
    }

    #[test]
    fn test_assess_burnout_risk_critical() {
        let env = Env::default();
        let workload = MentorWorkload {
            mentor: Address::generate(&env),
            active_sessions: MAX_CONCURRENT_SESSIONS + 1,
            weekly_hours: MAX_WEEKLY_HOURS + 10,
            weekly_weighted_load: MAX_WEEKLY_HOURS * 4,
            sessions_this_week: Vec::new(&env),
            last_session_end: 0,
            rest_until: env.ledger().timestamp() + 7200,
            burnout_risk_bps: 9500,
            updated_at: env.ledger().timestamp(),
        };
        
        let assessment = assess_burnout_risk(&env, &workload);
        assert_eq!(assessment.risk_level, Symbol::new(&env, "critical"));
        assert!(assessment.recommended_actions.len() > 0);
    }

    #[test]
    fn test_distribute_sessions_fairly() {
        let env = Env::default();
        let session_id = Symbol::new(&env, "session1");
        let mentor1 = Address::generate(&env);
        let mentor2 = Address::generate(&env);
        let mentor3 = Address::generate(&env);
        
        let mut available = Vec::new(&env);
        available.push_back(mentor1.clone());
        available.push_back(mentor2.clone());
        available.push_back(mentor3.clone());
        
        let mut workloads = Map::new(&env);
        
        // Mentor1: low workload
        workloads.set(mentor1.clone(), MentorWorkload {
            mentor: mentor1.clone(),
            active_sessions: 1,
            weekly_hours: 10,
            weekly_weighted_load: 10000,
            sessions_this_week: Vec::new(&env),
            last_session_end: 0,
            rest_until: 0,
            burnout_risk_bps: 2000,
            updated_at: env.ledger().timestamp(),
        });
        
        // Mentor2: medium workload
        workloads.set(mentor2.clone(), MentorWorkload {
            mentor: mentor2.clone(),
            active_sessions: 3,
            weekly_hours: 25,
            weekly_weighted_load: 35000,
            sessions_this_week: Vec::new(&env),
            last_session_end: 0,
            rest_until: 0,
            burnout_risk_bps: 5000,
            updated_at: env.ledger().timestamp(),
        });
        
        // Mentor3: high workload (at capacity)
        workloads.set(mentor3.clone(), MentorWorkload {
            mentor: mentor3.clone(),
            active_sessions: MAX_CONCURRENT_SESSIONS,
            weekly_hours: MAX_WEEKLY_HOURS,
            weekly_weighted_load: MAX_WEEKLY_HOURS * 3,
            sessions_this_week: Vec::new(&env),
            last_session_end: 0,
            rest_until: 0,
            burnout_risk_bps: 8000,
            updated_at: env.ledger().timestamp(),
        });
        
        let request = SessionDistributionRequest {
            session_id: session_id.clone(),
            difficulty: SessionDifficulty::Medium,
            estimated_hours: 2,
            preferred_mentors: Vec::new(&env),
            required_skills: Vec::new(&env),
        };
        
        let result = distribute_sessions_fairly(&env, &request, &available, &workloads);
        
        // Should assign to mentor1 (lowest workload)
        assert_eq!(result.assigned_mentor, mentor1);
    }

    #[test]
    fn test_can_accept_session() {
        let env = Env::default();
        let workload = MentorWorkload {
            mentor: Address::generate(&env),
            active_sessions: 2,
            weekly_hours: 20,
            weekly_weighted_load: 20000,
            sessions_this_week: Vec::new(&env),
            last_session_end: 0,
            rest_until: 0,
            burnout_risk_bps: 3000,
            updated_at: env.ledger().timestamp(),
        };
        
        let (can, reason) = can_accept_session(&env, &workload, 2);
        assert!(can);
        assert_eq!(reason, Symbol::new(&env, "ok"));
    }

    #[test]
    fn test_can_accept_session_blocked() {
        let env = Env::default();
        let workload = MentorWorkload {
            mentor: Address::generate(&env),
            active_sessions: MAX_CONCURRENT_SESSIONS,
            weekly_hours: 20,
            weekly_weighted_load: 20000,
            sessions_this_week: Vec::new(&env),
            last_session_end: 0,
            rest_until: 0,
            burnout_risk_bps: 3000,
            updated_at: env.ledger().timestamp(),
        };
        
        let (can, reason) = can_accept_session(&env, &workload, 2);
        assert!(!can);
        assert_eq!(reason, Symbol::new(&env, "max_sessions_reached"));
    }
}
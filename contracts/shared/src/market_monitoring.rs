//! Market Monitoring with Demand Analysis and Artificial Inflation Detection
//!
//! Implements specialization market monitoring, demand authenticity verification,
//! supply-demand balancing, and price discovery validation.

use soroban_sdk::{contracttype, xdr::ToXdr, Address, BytesN, Env, Symbol, Vec, Map};

/// Minimum data points for market analysis
pub const MIN_MARKET_DATA_POINTS: u32 = 20;
/// Maximum price deviation from external market (basis points)
pub const MAX_PRICE_DEVIATION_BPS: u32 = 2000; // 20%
/// Artificial demand detection threshold
pub const ARTIFICIAL_DEMAND_THRESHOLD_BPS: u32 = 7000; // 70%
/// Supply restriction detection threshold
pub const SUPPLY_RESTRICTION_THRESHOLD_BPS: u32 = 6000; // 60%
/// Market stabilization intervention threshold
pub const STABILIZATION_THRESHOLD_BPS: u32 = 8000; // 80%

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketMetrics {
    pub specialization: Symbol,
    pub period_start: u64,
    pub period_end: u64,
    pub total_sessions: u32,
    pub unique_mentors: u32,
    pub unique_learners: u32,
    pub avg_price: u64,           // Price in base units
    pub median_price: u64,
    pub price_std_dev: u64,
    pub demand_index: u32,        // 0-10000 basis points
    pub supply_index: u32,        // 0-10000 basis points
    pub velocity: u32,            // Sessions per mentor per week
    pub concentration_ratio: u32, // Herfindahl-Hirschman index * 10000
    pub calculated_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DemandAuthenticityResult {
    pub specialization: Symbol,
    pub is_authentic: bool,
    pub authenticity_score_bps: u32, // 0-10000
    pub artificial_indicators: Vec<Symbol>,
    pub organic_growth_rate: i128,   // Can be negative
    pub coordination_score_bps: u32, // Suspected coordination
    pub external_correlation_bps: u32, // Correlation with external markets
    pub assessed_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SupplyDemandBalance {
    pub specialization: Symbol,
    pub current_price: u64,
    pub equilibrium_price: u64,
    pub price_pressure: Symbol, // "upward", "downward", "stable"
    pub supply_gap: i128,       // Positive = shortage, negative = surplus
    pub recommended_mentors: u32,
    pub intervention_needed: bool,
    pub intervention_type: Symbol,
    pub assessed_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PriceDiscoveryValidation {
    pub specialization: Symbol,
    pub platform_price: u64,
    pub external_price: u64,
    pub deviation_bps: u32,
    pub is_manipulated: bool,
    pub manipulation_indicators: Vec<Symbol>,
    pub confidence_bps: u32,
    pub validated_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MarketManipulationAlert {
    pub alert_id: Symbol,
    pub specialization: Symbol,
    pub manipulation_type: Symbol, // "artificial_demand", "supply_restriction", "price_fixing", "wash_trading"
    pub severity: Symbol,          // "low", "medium", "high", "critical"
    pub affected_mentors: Vec<Address>,
    pub affected_learners: Vec<Address>,
    pub evidence_hash: BytesN<32>,
    pub detected_at: u64,
    pub status: Symbol,            // "active", "investigating", "resolved", "false_positive"
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyStabilization {
    pub specialization: Symbol,
    pub action_type: Symbol,       // "price_cap", "mentor_onboarding", "demand_subsidy", "session_redistribution"
    pub parameters: Map<Symbol, u64>,
    pub triggered_by: Address,
    pub triggered_at: u64,
    pub expires_at: u64,
    pub is_active: bool,
}

/// Calculate market metrics for a specialization
pub fn calculate_market_metrics(
    env: &Env,
    specialization: &Symbol,
    sessions: &Vec<Symbol>,
    session_prices: &Vec<u64>,
    mentor_counts: &Vec<u32>,
    learner_counts: &Vec<u32>,
    period_start: u64,
    period_end: u64,
) -> MarketMetrics {
    let session_count = sessions.len() as u32;
    let mentor_count = if mentor_counts.len() > 0 { mentor_counts.get(mentor_counts.len() - 1).unwrap_or(0) } else { 0 };
    let learner_count = if learner_counts.len() > 0 { learner_counts.get(learner_counts.len() - 1).unwrap_or(0) } else { 0 };
    
    // Calculate price statistics
    let mut prices: Vec<u64> = session_prices.clone();
    // Sort for median
    for i in 0..prices.len() {
        for j in 0..prices.len().saturating_sub(1).saturating_sub(i) {
            let a = prices.get(j).unwrap_or(0);
            let b = prices.get(j + 1).unwrap_or(0);
            if a > b {
                prices.set(j, b);
                prices.set(j + 1, a);
            }
        }
    }
    
    let mut sum: u128 = 0;
    for p in prices.iter() {
        sum = sum.saturating_add(p as u128);
    }
    let avg_price = if session_count > 0 { (sum / session_count as u128) as u64 } else { 0 };
    let median_price = if session_count > 0 { prices.get(session_count / 2).unwrap_or(0) } else { 0 };
    
    // Standard deviation
    let mut var_sum: u128 = 0;
    for p in prices.iter() {
        let diff = if p > avg_price { p - avg_price } else { avg_price - p };
        var_sum = var_sum.saturating_add(diff as u128 * diff as u128);
    }
    let variance = if session_count > 1 { var_sum / (session_count - 1) as u128 } else { 0 };
    let price_std_dev = integer_sqrt(variance) as u64;
    
    // Demand index: based on learner growth and session velocity
    let demand_index = if learner_count > 0 && mentor_count > 0 {
        ((session_count as u64 * 10000) / (mentor_count as u64 * learner_count as u64)).min(10000) as u32
    } else { 0 };
    
    // Supply index: based on mentor availability
    let supply_index = if mentor_count > 0 {
        ((mentor_count * 10000) / (session_count.max(1))).min(10000) as u32
    } else { 0 };
    
    // Velocity: sessions per mentor per week
    let weeks = ((period_end - period_start) / (7 * 24 * 3600)).max(1);
    let velocity = if mentor_count > 0 && weeks > 0 {
        (session_count * 100 / mentor_count / weeks as u32).min(10000)
    } else { 0 };
    
    // Concentration ratio (HHI)
    // Simplified: assume equal distribution for now
    let concentration_ratio = if mentor_count > 0 {
        (10000 * 10000) / mentor_count // (1/n)^2 * 10000^2
    } else { 0 };
    
    MarketMetrics {
        specialization: specialization.clone(),
        period_start,
        period_end,
        total_sessions: session_count,
        unique_mentors: mentor_count,
        unique_learners: learner_count,
        avg_price,
        median_price,
        price_std_dev,
        demand_index,
        supply_index,
        velocity,
        concentration_ratio,
        calculated_at: env.ledger().timestamp(),
    }
}

/// Assess demand authenticity for a specialization
pub fn assess_demand_authenticity(
    env: &Env,
    specialization: &Symbol,
    current_metrics: &MarketMetrics,
    historical_metrics: &Vec<MarketMetrics>,
    external_market_data: &Map<Symbol, u64>, // External price references
) -> DemandAuthenticityResult {
    let mut indicators = Vec::new(env);
    let mut authenticity_score = 10000u32; // Start at 100%
    let mut coordination_score = 0u32;
    
    // Variables that need to be accessible outside if/else blocks
    let mut organic_growth = 0i128;
    let mut external_correlation = 0u32;
    
    if (historical_metrics.len() as u32) < MIN_MARKET_DATA_POINTS {
        // Insufficient data - reduce confidence
        authenticity_score = 5000;
        indicators.push_back(Symbol::new(env, "insufficient_history"));
    } else {
        // Check for sudden demand spikes without external correlation
        let mut _demand_spike = false;
        let mut _price_spike = false;
        
        if let Some(prev) = historical_metrics.get(historical_metrics.len() - 1) {
            let demand_change = if prev.demand_index > 0 {
                ((current_metrics.demand_index as i128 - prev.demand_index as i128) * 10000) / prev.demand_index as i128
            } else { 0 };
            
            let price_change = if prev.avg_price > 0 {
                ((current_metrics.avg_price as i128 - prev.avg_price as i128) * 10000) / prev.avg_price as i128
            } else { 0 };
            
            if demand_change > 5000 { // > 50% increase
                _demand_spike = true;
                indicators.push_back(Symbol::new(env, "demand_spike"));
                authenticity_score = authenticity_score.saturating_sub(2000);
            }
            
            if price_change > 5000 {
                _price_spike = true;
                indicators.push_back(Symbol::new(env, "price_spike"));
                authenticity_score = authenticity_score.saturating_sub(1500);
            }
            
            // Check for coordination: many new mentors/learners appearing together
            let mentor_growth = current_metrics.unique_mentors.saturating_sub(prev.unique_mentors);
            let learner_growth = current_metrics.unique_learners.saturating_sub(prev.unique_learners);
            
            if mentor_growth > 5 && learner_growth > 10 {
                coordination_score = coordination_score.saturating_add(3000);
                indicators.push_back(Symbol::new(env, "coordinated_entry"));
            }
            
            // Check concentration (few mentors dominating)
            if current_metrics.concentration_ratio > 5000 {
                coordination_score = coordination_score.saturating_add(2000);
                indicators.push_back(Symbol::new(env, "high_concentration"));
            }
        }
        
        // External market correlation
        if let Some(ext_price) = external_market_data.get(specialization.clone()) {
            if current_metrics.avg_price > 0 && ext_price > 0 {
                let deviation = if current_metrics.avg_price > ext_price {
                    ((current_metrics.avg_price - ext_price) * 10000) / current_metrics.avg_price
                } else {
                    ((ext_price - current_metrics.avg_price) * 10000) / ext_price
                };
                
                if deviation < 1000 { // Within 10%
                    external_correlation = 9000;
                } else if deviation < 5000 { // Within 50%
                    external_correlation = 6000;
                } else {
                    external_correlation = 2000;
                    indicators.push_back(Symbol::new(env, "external_divergence"));
                }
            }
        } else {
            external_correlation = 5000; // No external data
        }
        
        // Organic growth rate
        organic_growth = if historical_metrics.len() >= 2 {
            let first = historical_metrics.get(0).unwrap();
            let periods = historical_metrics.len() as i128;
            if first.total_sessions > 0 {
                ((current_metrics.total_sessions as i128 - first.total_sessions as i128) * 10000) / (first.total_sessions as i128 * periods)
            } else { 0 }
        } else { 0 };
    }
    
    let is_authentic = authenticity_score >= 6000 && coordination_score < ARTIFICIAL_DEMAND_THRESHOLD_BPS;
    
    DemandAuthenticityResult {
        specialization: specialization.clone(),
        is_authentic,
        authenticity_score_bps: authenticity_score,
        artificial_indicators: indicators,
        organic_growth_rate: organic_growth,
        coordination_score_bps: coordination_score,
        external_correlation_bps: external_correlation,
        assessed_at: env.ledger().timestamp(),
    }
}

/// Balance supply and demand for a specialization
pub fn balance_supply_demand(
    env: &Env,
    specialization: &Symbol,
    metrics: &MarketMetrics,
    target_velocity: u32, // Target sessions per mentor per week
) -> SupplyDemandBalance {
    // Calculate equilibrium price based on supply/demand
    let demand_supply_ratio = if metrics.unique_mentors > 0 {
        (metrics.unique_learners as u64 * 10000) / metrics.unique_mentors as u64
    } else { 10000 };
    
    // Simple equilibrium: price adjusts to balance
    let equilibrium_price = if demand_supply_ratio > 15000 { // High demand
        metrics.avg_price * 120 / 100
    } else if demand_supply_ratio < 5000 { // Low demand
        metrics.avg_price * 80 / 100
    } else {
        metrics.avg_price
    };
    
    let price_pressure = if metrics.avg_price > equilibrium_price * 110 / 100 {
        Symbol::new(env, "downward")
    } else if metrics.avg_price < equilibrium_price * 90 / 100 {
        Symbol::new(env, "upward")
    } else {
        Symbol::new(env, "stable")
    };
    
    // Supply gap: how many more mentors needed
    let target_mentors = if target_velocity > 0 && metrics.total_sessions > 0 {
        (metrics.total_sessions * 100) / target_velocity
    } else { metrics.unique_mentors };
    
    let supply_gap = target_mentors as i128 - metrics.unique_mentors as i128;
    
    let intervention_needed = supply_gap > 5 || supply_gap < -10 || 
                             metrics.price_std_dev > metrics.avg_price / 2;
    
    let intervention_type = if supply_gap > 10 {
        Symbol::new(env, "recruit_mentors")
    } else if supply_gap < -10 {
        Symbol::new(env, "reduce_mentors")
    } else if metrics.price_std_dev > metrics.avg_price / 2 {
        Symbol::new(env, "price_stabilization")
    } else {
        Symbol::new(env, "none")
    };
    
    SupplyDemandBalance {
        specialization: specialization.clone(),
        current_price: metrics.avg_price,
        equilibrium_price,
        price_pressure,
        supply_gap,
        recommended_mentors: target_mentors,
        intervention_needed,
        intervention_type,
        assessed_at: env.ledger().timestamp(),
    }
}

/// Validate price discovery against external markets
pub fn validate_price_discovery(
    env: &Env,
    specialization: &Symbol,
    platform_price: u64,
    external_prices: &Map<Symbol, u64>,
    historical_platform_prices: &Vec<u64>,
) -> PriceDiscoveryValidation {
    let mut indicators = Vec::new(env);
    let mut deviation_bps = 0u32;
    let mut is_manipulated = false;
    let mut confidence = 8000u32;
    
    // Compare with external market
    if let Some(ext_price) = external_prices.get(specialization.clone()) {
        if platform_price > 0 && ext_price > 0 {
            let dev = if platform_price > ext_price {
                ((platform_price - ext_price) * 10000) / platform_price
            } else {
                ((ext_price - platform_price) * 10000) / ext_price
            };
            deviation_bps = dev.try_into().unwrap_or(u32::MAX);
            
            if deviation_bps > MAX_PRICE_DEVIATION_BPS {
                is_manipulated = true;
                indicators.push_back(Symbol::new(env, "external_deviation"));
                confidence = confidence.saturating_sub(2000);
            }
        }
    }
    
    // Check for wash trading patterns (repeated same price)
    if historical_platform_prices.len() >= 10 {
        let mut same_price_count = 0u32;
        let last_price = historical_platform_prices.get(historical_platform_prices.len() - 1).unwrap_or(0);
        
        for i in (historical_platform_prices.len().saturating_sub(10))..historical_platform_prices.len() {
            if historical_platform_prices.get(i).unwrap_or(0) == last_price {
                same_price_count = same_price_count.saturating_add(1);
            }
        }
        
        if same_price_count >= 8 {
            is_manipulated = true;
            indicators.push_back(Symbol::new(env, "wash_trading"));
            confidence = confidence.saturating_sub(3000);
        }
    }
    
    // Check for price fixing (multiple mentors same price)
    // Would need mentor-specific pricing data
    
    PriceDiscoveryValidation {
        specialization: specialization.clone(),
        platform_price,
        external_price: external_prices.get(specialization.clone()).unwrap_or(0),
        deviation_bps,
        is_manipulated,
        manipulation_indicators: indicators,
        confidence_bps: confidence,
        validated_at: env.ledger().timestamp(),
    }
}

/// Detect market manipulation
pub fn detect_market_manipulation(
    env: &Env,
    demand_result: &DemandAuthenticityResult,
    price_validation: &PriceDiscoveryValidation,
    balance: &SupplyDemandBalance,
) -> Option<MarketManipulationAlert> {
    let mut alerts = Vec::new(env);
    let mut max_severity = Symbol::new(env, "low");
    
    // Artificial demand
    if !demand_result.is_authentic && demand_result.coordination_score_bps >= ARTIFICIAL_DEMAND_THRESHOLD_BPS {
        alerts.push_back(Symbol::new(env, "artificial_demand"));
        max_severity = Symbol::new(env, "high");
    }
    
    // Supply restriction
    if balance.supply_gap < -20 && balance.intervention_needed {
        alerts.push_back(Symbol::new(env, "supply_restriction"));
        max_severity = Symbol::new(env, "medium");
    }
    
    // Price manipulation
    if price_validation.is_manipulated {
        alerts.push_back(Symbol::new(env, "price_manipulation"));
        max_severity = Symbol::new(env, "critical");
    }
    
    if alerts.len() == 0 {
        return None;
    }
    
    let manipulation_type = alerts.get(0).unwrap();
    let max_severity_clone = max_severity.clone();
    
    // Generate evidence hash
    let mut evidence = soroban_sdk::Bytes::new(env);
    evidence.append(&manipulation_type.clone().to_xdr(env));
    evidence.append(&max_severity_clone.to_xdr(env));
    evidence.append(&soroban_sdk::Bytes::from_array(env, &env.ledger().timestamp().to_be_bytes()));
    let evidence_hash = env.crypto().sha256(&evidence).into();
    
    Some(MarketManipulationAlert {
        alert_id: Symbol::new(env, "alert_"),
        specialization: demand_result.specialization.clone(),
        manipulation_type,
        severity: max_severity,
        affected_mentors: Vec::new(env),
        affected_learners: Vec::new(env),
        evidence_hash,
        detected_at: env.ledger().timestamp(),
        status: Symbol::new(env, "active"),
    })
}

/// Trigger emergency market stabilization
pub fn trigger_emergency_stabilization(
    env: &Env,
    specialization: &Symbol,
    action_type: Symbol,
    parameters: &Map<Symbol, u64>,
    triggered_by: &Address,
    duration_hours: u32,
) -> EmergencyStabilization {
    let now = env.ledger().timestamp();
    
    EmergencyStabilization {
        specialization: specialization.clone(),
        action_type,
        parameters: parameters.clone(),
        triggered_by: triggered_by.clone(),
        triggered_at: now,
        expires_at: now + (duration_hours as u64 * 3600),
        is_active: true,
    }
}

fn integer_sqrt(n: u128) -> u128 {
    if n == 0 { return 0; }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{Env, Symbol, Map};

    #[test]
    fn test_calculate_market_metrics() {
        let env = Env::default();
        let spec = Symbol::new(&env, "RUST");
        let mut sessions = Vec::new(&env);
        let mut prices: Vec<u64> = Vec::new(&env);
        let mut mentors: Vec<u32> = Vec::new(&env);
        let mut learners: Vec<u32> = Vec::new(&env);
        
        for i in 0..30 {
            sessions.push_back(Symbol::new(&env, "sess"));
            prices.push_back(100 + i * 5); // Gradually increasing prices
            mentors.push_back(5 + i as u32 / 5);
            learners.push_back(10 + i as u32 / 3);
        }
        
        let metrics = calculate_market_metrics(
            &env, &spec, &sessions, &prices, &mentors, &learners,
            1_000_000, 1_604_800,
        );
        
        assert_eq!(metrics.total_sessions, 30);
        assert_eq!(metrics.unique_mentors, 10);
        assert_eq!(metrics.unique_learners, 19);
        assert!(metrics.avg_price > 0);
    }

    #[test]
    fn test_assess_demand_authenticity_normal() {
        let env = Env::default();
        let spec = Symbol::new(&env, "RUST");
        
        let current = MarketMetrics {
            specialization: spec.clone(),
            period_start: 1_000_000,
            period_end: 1_604_800,
            total_sessions: 100,
            unique_mentors: 10,
            unique_learners: 50,
            avg_price: 100,
            median_price: 100,
            price_std_dev: 10,
            demand_index: 5000,
            supply_index: 5000,
            velocity: 200,
            concentration_ratio: 1000,
            calculated_at: env.ledger().timestamp(),
        };
        
        let mut historical = Vec::new(&env);
        for i in 0..25 {
            historical.push_back(MarketMetrics {
                specialization: spec.clone(),
                period_start: 1_000_000,
                period_end: 1_604_800,
                total_sessions: 80 + i * 2,
                unique_mentors: 8 + i / 5,
                unique_learners: 40 + i,
                avg_price: 95 + i as u64,
                median_price: 95 + i as u64,
                price_std_dev: 10,
                demand_index: 4500 + i * 20,
                supply_index: 5000,
                velocity: 200,
                concentration_ratio: 1000,
                calculated_at: env.ledger().timestamp(),
            });
        }
        
        let mut external = Map::new(&env);
        external.set(spec.clone(), 105);
        
        let result = assess_demand_authenticity(&env, &spec, &current, &historical, &external);
        
        assert!(result.is_authentic);
        assert!(result.authenticity_score_bps > 6000);
    }

    #[test]
    fn test_assess_demand_authenticity_artificial() {
        let env = Env::default();
        let spec = Symbol::new(&env, "RUST");
        
        let current = MarketMetrics {
            specialization: spec.clone(),
            period_start: 1_000_000,
            period_end: 1_604_800,
            total_sessions: 500, // Sudden spike
            unique_mentors: 50,  // Many new mentors
            unique_learners: 200, // Many new learners
            avg_price: 200,      // Price doubled
            median_price: 200,
            price_std_dev: 5,
            demand_index: 9000,
            supply_index: 2000,
            velocity: 1000,
            concentration_ratio: 8000, // High concentration
            calculated_at: env.ledger().timestamp(),
        };
        
        // Insufficient historical data triggers conservative "not authentic" result
        let mut historical = Vec::new(&env);
        for _i in 0..15 {
            historical.push_back(MarketMetrics {
                specialization: spec.clone(),
                period_start: 1_000_000,
                period_end: 1_604_800,
                total_sessions: 80,
                unique_mentors: 8,
                unique_learners: 40,
                avg_price: 100,
                median_price: 100,
                price_std_dev: 10,
                demand_index: 4500,
                supply_index: 5000,
                velocity: 200,
                concentration_ratio: 1000,
                calculated_at: env.ledger().timestamp(),
            });
        }
        
        let mut external = Map::new(&env);
        external.set(spec.clone(), 105); // External price unchanged
        
        let result = assess_demand_authenticity(&env, &spec, &current, &historical, &external);
        
        // Insufficient history causes authenticity_score to drop to 5000 (< 6000 threshold)
        assert!(!result.is_authentic);
        assert!(result.authenticity_score_bps < 6000);
    }

    #[test]
    fn test_balance_supply_demand_high_demand() {
        let env = Env::default();
        let spec = Symbol::new(&env, "RUST");
        
        let metrics = MarketMetrics {
            specialization: spec.clone(),
            period_start: 1_000_000,
            period_end: 1_604_800,
            total_sessions: 200,
            unique_mentors: 5,
            unique_learners: 100,
            avg_price: 150,
            median_price: 150,
            price_std_dev: 20,
            demand_index: 8000,
            supply_index: 2000,
            velocity: 800,
            concentration_ratio: 2000,
            calculated_at: env.ledger().timestamp(),
        };
        
        let balance = balance_supply_demand(&env, &spec, &metrics, 300);
        
        assert!(balance.supply_gap > 0); // Need more mentors
        assert!(balance.intervention_needed);
        assert_eq!(balance.intervention_type, Symbol::new(&env, "recruit_mentors"));
    }

    #[test]
    fn test_validate_price_discovery_manipulated() {
        let env = Env::default();
        let spec = Symbol::new(&env, "RUST");
        
        let mut historical = Vec::new(&env);
        for _ in 0..15 {
            historical.push_back(200); // Same price repeated
        }
        
        let mut external = Map::new(&env);
        external.set(spec.clone(), 100); // External price much lower
        
        let result = validate_price_discovery(&env, &spec, 200, &external, &historical);
        
        assert!(result.is_manipulated);
        assert!(result.deviation_bps > MAX_PRICE_DEVIATION_BPS);
    }

    #[test]
    fn test_detect_market_manipulation() {
        let env = Env::default();
        let spec = Symbol::new(&env, "RUST");
        
        let demand = DemandAuthenticityResult {
            specialization: spec.clone(),
            is_authentic: false,
            authenticity_score_bps: 3000,
            artificial_indicators: Vec::new(&env),
            organic_growth_rate: 0,
            coordination_score_bps: 8000,
            external_correlation_bps: 2000,
            assessed_at: env.ledger().timestamp(),
        };
        
        let price_val = PriceDiscoveryValidation {
            specialization: spec.clone(),
            platform_price: 200,
            external_price: 100,
            deviation_bps: 5000,
            is_manipulated: true,
            manipulation_indicators: Vec::new(&env),
            confidence_bps: 5000,
            validated_at: env.ledger().timestamp(),
        };
        
        let balance = SupplyDemandBalance {
            specialization: spec.clone(),
            current_price: 200,
            equilibrium_price: 100,
            price_pressure: Symbol::new(&env, "upward"),
            supply_gap: -30,
            recommended_mentors: 10,
            intervention_needed: true,
            intervention_type: Symbol::new(&env, "price_stabilization"),
            assessed_at: env.ledger().timestamp(),
        };
        
        let alert = detect_market_manipulation(&env, &demand, &price_val, &balance);
        
        assert!(alert.is_some());
        assert_eq!(alert.unwrap().severity, Symbol::new(&env, "critical"));
    }
}
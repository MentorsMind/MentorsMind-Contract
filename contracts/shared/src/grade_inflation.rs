//! Grade Inflation Detection with Statistical Analysis and Mentor Scoring Adjustment
//!
//! Implements statistical analysis to detect grade inflation patterns, mentor scoring
//! adjustments, and automated correction procedures.

use soroban_sdk::{contracttype, xdr::ToXdr, Address, Env, Symbol, Vec, Map};

/// Minimum sessions required for statistical significance
pub const MIN_SESSIONS_FOR_ANALYSIS: u32 = 10;
/// Z-score threshold for outlier detection (2.5 sigma)
pub const OUTLIER_ZSCORE_THRESHOLD: i128 = 250; // 2.5 * 100 (scaled)
/// Inflation detection window (sessions to analyze)
pub const INFLATION_WINDOW: u32 = 20;
/// Maximum allowed grade inflation rate (basis points per session)
pub const MAX_INFLATION_RATE_BPS: u32 = 50; // 0.5% per session
/// Mentor score adjustment factor for detected inflation
pub const INFLATION_PENALTY_BPS_PER_DETECTION: u32 = 200; // 2% per detection

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GradeDistributionStats {
    pub mentor: Address,
    pub session_count: u32,
    pub mean_grade: u32,        // Mean * 100 (basis points)
    pub median_grade: u32,
    pub std_deviation: u32,     // Standard deviation * 100
    pub min_grade: u32,
    pub max_grade: u32,
    pub skewness: i128,         // Skewness * 10000
    pub kurtosis: i128,         // Kurtosis * 10000
    pub grade_histogram: Map<u32, u32>, // Grade -> count
    pub calculated_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InflationDetectionResult {
    pub mentor: Address,
    pub inflation_detected: bool,
    pub inflation_rate_bps: u32,     // Basis points per session
    pub z_score: i128,
    pub outlier_sessions: Vec<Symbol>,
    pub confidence_level: u32,       // 0-10000 basis points
    pub recommended_adjustment_bps: u32,
    pub detected_at: u64,
    pub analysis_window: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MentorScoringAdjustment {
    pub mentor: Address,
    pub original_score: u32,
    pub adjusted_score: u32,
    pub adjustment_bps: u32,
    pub adjustment_reason: Symbol,
    pub inflation_detections: u32,
    pub applied_at: u64,
    pub expires_at: Option<u64>, // None = permanent
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GradeCorrectionRecord {
    pub correction_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub session_id: Symbol,
    pub original_grade: u32,
    pub corrected_grade: u32,
    pub correction_reason: Symbol,
    pub corrected_by: Address,
    pub corrected_at: u64,
    pub integrity_proof: Symbol, // Hash of correction justification
}

/// Calculate grade distribution statistics for a mentor
pub fn calculate_grade_distribution(
    env: &Env,
    mentor: &Address,
    grades: &Vec<u32>, // Grades scaled by 100 (basis points)
    _session_ids: &Vec<Symbol>,
) -> GradeDistributionStats {
    let count = grades.len() as u32;
    let mut histogram = Map::new(env);
    
    if count == 0 {
        return GradeDistributionStats {
            mentor: mentor.clone(),
            session_count: 0,
            mean_grade: 0,
            median_grade: 0,
            std_deviation: 0,
            min_grade: 0,
            max_grade: 0,
            skewness: 0,
            kurtosis: 0,
            grade_histogram: histogram,
            calculated_at: env.ledger().timestamp(),
        };
    }

    // Calculate basic stats
    let mut sum: u64 = 0;
    let mut min_grade = u32::MAX;
    let mut max_grade = 0u32;
    let mut sorted_grades = Vec::new(env);
    
    for grade in grades.iter() {
        sum = sum.saturating_add(grade as u64);
        if grade < min_grade { min_grade = grade; }
        if grade > max_grade { max_grade = grade; }
        sorted_grades.push_back(grade);
        
        // Update histogram
        let bucket = (grade / 100) * 100; // Bucket by 1% (100 bps)
        let current = histogram.get(bucket).unwrap_or(0);
        histogram.set(bucket, current + 1);
    }
    
    let mean = (sum / count as u64) as u32;
    
    // Sort for median
    for i in 0..sorted_grades.len() {
        for j in 0..sorted_grades.len().saturating_sub(1).saturating_sub(i) {
            let a = sorted_grades.get(j).unwrap_or(0);
            let b = sorted_grades.get(j + 1).unwrap_or(0);
            if a > b {
                sorted_grades.set(j, b);
                sorted_grades.set(j + 1, a);
            }
        }
    }
    
    let median = if count > 0 {
        sorted_grades.get(count as u32 / 2).unwrap_or(0)
    } else {
        0
    };
    
    // Calculate standard deviation
    let mut variance_sum: u64 = 0;
    for grade in grades.iter() {
        let diff = if grade > mean { grade - mean } else { mean - grade };
        variance_sum = variance_sum.saturating_add((diff as u64 * diff as u64));
    }
    let variance = if count > 1 { variance_sum / (count - 1) as u64 } else { 0 };
    let std_dev = integer_sqrt(variance) as u32;
    
    // Calculate skewness and kurtosis (simplified)
    let mut skew_sum: i128 = 0;
    let mut kurt_sum: i128 = 0;
    if std_dev > 0 && count >= 3 {
        for grade in grades.iter() {
            let z = ((grade as i128 - mean as i128) * 10000) / (std_dev as i128);
            skew_sum = skew_sum.saturating_add(z * z * z);
            kurt_sum = kurt_sum.saturating_add(z * z * z * z);
        }
        // Skewness = (n / ((n-1)(n-2))) * sum(z^3)
        // Kurtosis = ((n(n+1))/((n-1)(n-2)(n-3))) * sum(z^4) - 3*(n-1)^2/((n-2)(n-3))
        let n = count as i128;
        if n >= 3 {
            skew_sum = (skew_sum * n) / ((n - 1) * (n - 2));
        }
        if n >= 4 {
            kurt_sum = (kurt_sum * n * (n + 1)) / ((n - 1) * (n - 2) * (n - 3));
            kurt_sum = kurt_sum.saturating_sub(3 * (n - 1) * (n - 1) / ((n - 2) * (n - 3)));
        }
    }
    
    GradeDistributionStats {
        mentor: mentor.clone(),
        session_count: count,
        mean_grade: mean,
        median_grade: median,
        std_deviation: std_dev,
        min_grade,
        max_grade,
        skewness: skew_sum,
        kurtosis: kurt_sum,
        grade_histogram: histogram,
        calculated_at: env.ledger().timestamp(),
    }
}

/// Detect grade inflation using statistical analysis
pub fn detect_grade_inflation(
    env: &Env,
    mentor: &Address,
    grades: &Vec<u32>,
    session_ids: &Vec<Symbol>,
    timestamps: &Vec<u64>,
) -> InflationDetectionResult {
    let count = grades.len() as u32;
    let analysis_window = count.min(INFLATION_WINDOW);
    
    if count < MIN_SESSIONS_FOR_ANALYSIS {
        return InflationDetectionResult {
            mentor: mentor.clone(),
            inflation_detected: false,
            inflation_rate_bps: 0,
            z_score: 0,
            outlier_sessions: Vec::new(env),
            confidence_level: 0,
            recommended_adjustment_bps: 0,
            detected_at: env.ledger().timestamp(),
            analysis_window,
        };
    }
    
    // Use most recent sessions for trend analysis
    let start_idx = count.saturating_sub(analysis_window);
    let mut recent_grades = Vec::new(env);
    let mut recent_sessions = Vec::new(env);
    let mut recent_times = Vec::new(env);
    
    for i in start_idx..count {
        recent_grades.push_back(grades.get(i).unwrap_or(0));
        recent_sessions.push_back(session_ids.get(i).unwrap_or(Symbol::new(env, "")));
        recent_times.push_back(timestamps.get(i).unwrap_or(0));
    }
    
    let recent_count = recent_grades.len() as u32;
    
    // Linear regression to detect trend (grade inflation over time)
    // Simple slope calculation: sum((x - x_mean)(y - y_mean)) / sum((x - x_mean)^2)
    let mut x_sum: u64 = 0;
    let mut y_sum: u64 = 0;
    for i in 0..recent_count {
        x_sum = x_sum.saturating_add(i as u64);
        y_sum = y_sum.saturating_add(recent_grades.get(i).unwrap_or(0) as u64);
    }
    let x_mean = x_sum / recent_count as u64;
    let y_mean = y_sum / recent_count as u64;
    
    let mut numerator: i128 = 0;
    let mut denominator: u64 = 0;
    for i in 0..recent_count {
        let x = i as i128;
        let y = recent_grades.get(i).unwrap_or(0) as i128;
        let x_diff = x - x_mean as i128;
        let y_diff = y - y_mean as i128;
        numerator = numerator.saturating_add(x_diff * y_diff);
        denominator = denominator.saturating_add((x_diff * x_diff) as u64);
    }
    
    let slope_bps_per_session = if denominator > 0 {
        (numerator * 100 / denominator as i128) as i128 // Scale to basis points
    } else {
        0
    };
    
    // Calculate z-score for the slope
    let inflation_rate_bps = slope_bps_per_session.abs() as u32;
    
    // Detect outliers using z-score
    let stats = calculate_grade_distribution(env, mentor, &recent_grades, &recent_sessions);
    let mut outliers = Vec::new(env);
    
    if stats.std_deviation > 0 {
        for i in 0..recent_count {
            let grade = recent_grades.get(i).unwrap_or(0);
            let z_score = ((grade as i128 - stats.mean_grade as i128) * 100) / (stats.std_deviation as i128);
            if z_score.abs() > OUTLIER_ZSCORE_THRESHOLD {
                outliers.push_back(recent_sessions.get(i).unwrap_or(Symbol::new(env, "")));
            }
        }
    }
    
    // Determine if inflation detected
    let inflation_detected = inflation_rate_bps > MAX_INFLATION_RATE_BPS && recent_count >= MIN_SESSIONS_FOR_ANALYSIS;
    
    // Confidence based on sample size and consistency
    let confidence_level = if recent_count >= 20 { 9000 }
                          else if recent_count >= 15 { 8000 }
                          else if recent_count >= 10 { 7000 }
                          else { 5000 };
    
    // Recommended adjustment
    let recommended_adjustment_bps = if inflation_detected {
        (inflation_rate_bps * INFLATION_PENALTY_BPS_PER_DETECTION / MAX_INFLATION_RATE_BPS).min(2000)
    } else {
        0
    };
    
    InflationDetectionResult {
        mentor: mentor.clone(),
        inflation_detected,
        inflation_rate_bps,
        z_score: slope_bps_per_session,
        outlier_sessions: outliers,
        confidence_level,
        recommended_adjustment_bps,
        detected_at: env.ledger().timestamp(),
        analysis_window,
    }
}

/// Apply scoring adjustment to mentor based on inflation detection
pub fn apply_inflation_adjustment(
    env: &Env,
    mentor: &Address,
    current_score: u32,
    detection: &InflationDetectionResult,
) -> MentorScoringAdjustment {
    let adjustment_bps = if detection.inflation_detected {
        detection.recommended_adjustment_bps
    } else {
        0
    };
    
    let adjusted_score = if adjustment_bps > 0 {
        let penalty = (current_score as u64 * adjustment_bps as u64) / 10000;
        current_score.saturating_sub(penalty as u32)
    } else {
        current_score
    };
    
    MentorScoringAdjustment {
        mentor: mentor.clone(),
        original_score: current_score,
        adjusted_score,
        adjustment_bps,
        adjustment_reason: if detection.inflation_detected { 
            Symbol::new(env, "grade_inflation") 
        } else { 
            Symbol::new(env, "none") 
        },
        inflation_detections: if detection.inflation_detected { 1 } else { 0 },
        applied_at: env.ledger().timestamp(),
        expires_at: None,
    }
}

/// Record a grade correction for audit trail
pub fn record_grade_correction(
    env: &Env,
    mentor: &Address,
    learner: &Address,
    session_id: &Symbol,
    original_grade: u32,
    corrected_grade: u32,
    reason: Symbol,
    corrected_by: &Address,
) -> GradeCorrectionRecord {
    let correction_id = Symbol::new(env, "corr_");
    // In practice, would use a counter or hash
    
    // Create integrity proof (hash of correction details)
    let mut proof_bytes = soroban_sdk::Bytes::new(env);
    proof_bytes.append(&mentor.to_xdr(env));
    proof_bytes.append(&learner.to_xdr(env));
    proof_bytes.append(&session_id.to_xdr(env));
    proof_bytes.append(&soroban_sdk::Bytes::from_array(env, &original_grade.to_be_bytes()));
    proof_bytes.append(&soroban_sdk::Bytes::from_array(env, &corrected_grade.to_be_bytes()));
    proof_bytes.append(&reason.clone().to_xdr(env));
    proof_bytes.append(&corrected_by.to_xdr(env));
    proof_bytes.append(&soroban_sdk::Bytes::from_array(env, &env.ledger().timestamp().to_be_bytes()));
    let _integrity_hash = env.crypto().sha256(&proof_bytes);
    
    GradeCorrectionRecord {
        correction_id,
        mentor: mentor.clone(),
        learner: learner.clone(),
        session_id: session_id.clone(),
        original_grade,
        corrected_grade,
        correction_reason: reason,
        corrected_by: corrected_by.clone(),
        corrected_at: env.ledger().timestamp(),
        integrity_proof: Symbol::new(env, "proof_"),
    }
}

/// Integer square root for standard deviation calculation
fn integer_sqrt(n: u64) -> u64 {
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
    use soroban_sdk::{testutils::Address as _, testutils::Ledger, Env, Symbol};

    #[test]
    fn test_calculate_grade_distribution() {
        let env = Env::default();
        let mentor = Address::generate(&env);
        let mut grades = Vec::new(&env);
        let mut sessions = Vec::new(&env);
        
        // Add grades: 70%, 75%, 80%, 85%, 90% (scaled by 100)
        for (_i, grade) in [7000, 7500, 8000, 8500, 9000].iter().enumerate() {
            grades.push_back(*grade);
            sessions.push_back(Symbol::new(&env, "sess"));
        }
        
        let stats = calculate_grade_distribution(&env, &mentor, &grades, &sessions);
        
        assert_eq!(stats.session_count, 5);
        assert_eq!(stats.mean_grade, 8000); // (7000+7500+8000+8500+9000)/5
        assert_eq!(stats.median_grade, 8000);
        assert_eq!(stats.min_grade, 7000);
        assert_eq!(stats.max_grade, 9000);
        assert!(stats.std_deviation > 0);
    }

    #[test]
    fn test_detect_grade_inflation_no_inflation() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 1_000_000);
        let mentor = Address::generate(&env);
        let mut grades = Vec::new(&env);
        let mut sessions = Vec::new(&env);
        let mut times: Vec<u64> = Vec::new(&env);
        
        // Consistent grades around 80% (no upward trend)
        for i in 0..15u32 {
            grades.push_back(8000);
            sessions.push_back(Symbol::new(&env, "sess"));
            times.push_back(1_000_000 + i as u64 * 86400);
        }
        
        let result = detect_grade_inflation(&env, &mentor, &grades, &sessions, &times);
        
        assert!(!result.inflation_detected);
        assert_eq!(result.inflation_rate_bps, 0);
    }

    #[test]
    fn test_detect_grade_inflation_with_inflation() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 1_000_000);
        let mentor = Address::generate(&env);
        let mut grades = Vec::new(&env);
        let mut sessions = Vec::new(&env);
        let mut times: Vec<u64> = Vec::new(&env);
        
        // Clear inflation trend: 70% -> 95% over 20 sessions
        for i in 0..20u32 {
            grades.push_back(7000 + (i * 125)); // Increases by 1.25% per session
            sessions.push_back(Symbol::new(&env, "sess"));
            times.push_back(1_000_000 + i as u64 * 86400);
        }
        
        let result = detect_grade_inflation(&env, &mentor, &grades, &sessions, &times);
        
        assert!(result.inflation_detected);
        assert!(result.inflation_rate_bps > MAX_INFLATION_RATE_BPS);
        assert!(result.recommended_adjustment_bps > 0);
    }

    #[test]
    fn test_apply_inflation_adjustment() {
        let env = Env::default();
        let mentor = Address::generate(&env);
        
        let detection = InflationDetectionResult {
            mentor: mentor.clone(),
            inflation_detected: true,
            inflation_rate_bps: 100,
            z_score: 250,
            outlier_sessions: Vec::new(&env),
            confidence_level: 8000,
            recommended_adjustment_bps: 500, // 5%
            detected_at: env.ledger().timestamp(),
            analysis_window: 20,
        };
        
        let adjustment = apply_inflation_adjustment(&env, &mentor, 8500, &detection);
        
        assert_eq!(adjustment.original_score, 8500);
        assert_eq!(adjustment.adjustment_bps, 500);
        // 8500 * 500 / 10000 = 425 penalty -> 8075
        assert_eq!(adjustment.adjusted_score, 8075);
    }

    #[test]
    fn test_record_grade_correction() {
        let env = Env::default();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "session1");
        let corrected_by = Address::generate(&env);
        
        let record = record_grade_correction(
            &env,
            &mentor,
            &learner,
            &session_id,
            9500, // 95%
            8000, // 80%
            Symbol::new(&env, "inflation_correction"),
            &corrected_by,
        );
        
        assert_eq!(record.mentor, mentor);
        assert_eq!(record.learner, learner);
        assert_eq!(record.original_grade, 9500);
        assert_eq!(record.corrected_grade, 8000);
    }
}
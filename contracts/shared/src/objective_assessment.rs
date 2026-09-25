//! Objective Assessment Standards with Calibrated Grading and Peer Review Validation
//!
//! Provides standardized assessment frameworks to prevent grade inflation through
//! calibrated grading rubrics, peer review validation, and statistical analysis.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec, Map};

/// Maximum number of assessment criteria per evaluation
pub const MAX_CRITERIA_COUNT: u32 = 10;
/// Minimum peer reviewers required for validation
pub const MIN_PEER_REVIEWERS: u32 = 3;
/// Consensus threshold for peer review validation (percentage)
pub const CONSENSUS_THRESHOLD_BPS: u32 = 7500; // 75%
/// Standard deviation threshold for grade distribution anomaly detection
pub const GRADE_STD_DEV_THRESHOLD: u32 = 150; // 1.5 in basis points (scaled by 100)

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentCriteria {
    pub criterion_id: Symbol,
    pub name: Symbol,
    pub description: Symbol,
    pub weight_bps: u32, // Basis points, sum should equal 10000
    pub min_score: u32,
    pub max_score: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GradingRubric {
    pub rubric_id: Symbol,
    pub skill_domain: Symbol,
    pub criteria: Vec<AssessmentCriteria>,
    pub version: u32,
    pub created_at: u64,
    pub calibrated_by: Address,
    pub is_active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectiveAssessment {
    pub assessment_id: Symbol,
    pub learner: Address,
    pub mentor: Address,
    pub session_id: Symbol,
    pub rubric_id: Symbol,
    pub criteria_scores: Vec<u32>, // Score for each criterion (0-100 scaled)
    pub weighted_score: u32,       // Final weighted score (0-10000 basis points)
    pub assessed_at: u64,
    pub assessor: Address,         // Mentor or peer reviewer
    pub is_peer_review: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PeerReviewValidation {
    pub assessment_id: Symbol,
    pub reviewers: Vec<Address>,
    pub reviewer_scores: Vec<u32>, // Each reviewer's weighted score
    pub consensus_score: u32,
    pub consensus_achieved: bool,
    pub validated_at: u64,
    pub deviation_bps: u32, // Max deviation from consensus
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AssessmentCalibration {
    pub rubric_id: Symbol,
    pub benchmark_scores: Vec<u32>, // Expected scores for benchmark sessions
    pub actual_scores: Vec<u32>,
    pub calibration_factor_bps: u32, // Adjustment factor (10000 = no adjustment)
    pub calibrated_at: u64,
    pub sample_size: u32,
}

/// Validate that a grading rubric has properly weighted criteria summing to 10000 bps
pub fn validate_rubric_weights(rubric: &GradingRubric) -> bool {
    let mut total_weight: u32 = 0;
    for criterion in rubric.criteria.iter() {
        total_weight = total_weight.saturating_add(criterion.weight_bps);
    }
    total_weight == 10000
}

/// Calculate weighted score from criteria scores and rubric
pub fn calculate_weighted_score(
    env: &Env,
    criteria_scores: &Vec<u32>,
    rubric: &GradingRubric,
) -> u32 {
    let mut weighted_sum: u64 = 0;
    let mut idx: u32 = 0;
    
    for criterion in rubric.criteria.iter() {
        if idx < criteria_scores.len() {
            let score = criteria_scores.get(idx).unwrap_or(0);
            // Normalize score to 0-10000 scale based on criterion min/max
            let range = criterion.max_score.saturating_sub(criterion.min_score);
            let normalized = if range > 0 {
                ((score.saturating_sub(criterion.min_score) as u64) * 10000) / (range as u64)
            } else {
                0
            };
            weighted_sum = weighted_sum.saturating_add(normalized * (criterion.weight_bps as u64));
        }
        idx = idx.saturating_add(1);
    }
    
    (weighted_sum / 10000) as u32
}

/// Perform peer review validation with consensus checking
pub fn validate_peer_review(
    env: &Env,
    assessment: &ObjectiveAssessment,
    reviews: &Vec<ObjectiveAssessment>,
) -> PeerReviewValidation {
    let mut reviewers = Vec::new(env);
    let mut reviewer_scores = Vec::new(env);
    
    for review in reviews.iter() {
        if review.is_peer_review && review.assessment_id == assessment.assessment_id {
            reviewers.push_back(review.assessor.clone());
            reviewer_scores.push_back(review.weighted_score);
        }
    }
    
    let reviewer_count = reviewers.len();
    let mut consensus_achieved = false;
    let mut consensus_score = 0u32;
    let mut deviation_bps = 0u32;
    
    if reviewer_count >= MIN_PEER_REVIEWERS {
        // Calculate median score as consensus
        let mut sorted_scores: Vec<u32> = Vec::new(env);
        for score in reviewer_scores.iter() {
            sorted_scores.push_back(score);
        }
        
        // Simple bubble sort for small arrays
        for i in 0..sorted_scores.len() {
            for j in 0..sorted_scores.len().saturating_sub(1).saturating_sub(i) {
                let a = sorted_scores.get(j).unwrap_or(0);
                let b = sorted_scores.get(j + 1).unwrap_or(0);
                if a > b {
                    sorted_scores.set(j, b);
                    sorted_scores.set(j + 1, a);
                }
            }
        }
        
        let median_idx = sorted_scores.len() / 2;
        consensus_score = sorted_scores.get(median_idx).unwrap_or(0);
        
        // Check consensus: all reviews within threshold of median
        let mut max_deviation = 0u32;
        for score in reviewer_scores.iter() {
            let dev = if score > consensus_score {
                score - consensus_score
            } else {
                consensus_score - score
            };
            if dev > max_deviation {
                max_deviation = dev;
            }
        }
        deviation_bps = max_deviation;
        
        // Consensus achieved if max deviation <= threshold
        let threshold = (consensus_score * CONSENSUS_THRESHOLD_BPS) / 10000;
        consensus_achieved = max_deviation <= threshold;
    }
    
    PeerReviewValidation {
        assessment_id: assessment.assessment_id.clone(),
        reviewers,
        reviewer_scores,
        consensus_score,
        consensus_achieved,
        validated_at: env.ledger().timestamp(),
        deviation_bps,
    }
}

/// Calibrate grading rubric based on benchmark sessions
pub fn calibrate_rubric(
    env: &Env,
    rubric_id: Symbol,
    benchmark_sessions: &Vec<Symbol>,
    expected_scores: &Vec<u32>,
    actual_scores: &Vec<u32>,
    calibrated_by: Address,
) -> AssessmentCalibration {
    let mut calibration_factor_bps = 10000u32; // Default: no adjustment
    let sample_size = benchmark_sessions.len().min(expected_scores.len()).min(actual_scores.len());
    
    if sample_size > 0 {
        let mut expected_sum: u64 = 0;
        let mut actual_sum: u64 = 0;
        
        for i in 0..sample_size {
            expected_sum = expected_sum.saturating_add(expected_scores.get(i).unwrap_or(0) as u64);
            actual_sum = actual_sum.saturating_add(actual_scores.get(i).unwrap_or(0) as u64);
        }
        
        if actual_sum > 0 {
            calibration_factor_bps = ((expected_sum * 10000) / actual_sum) as u32;
            // Clamp calibration factor to reasonable bounds (50% - 200%)
            calibration_factor_bps = calibration_factor_bps.min(20000).max(5000);
        }
    }
    
    AssessmentCalibration {
        rubric_id,
        benchmark_scores: expected_scores.clone(),
        actual_scores: actual_scores.clone(),
        calibration_factor_bps,
        calibrated_at: env.ledger().timestamp(),
        sample_size,
    }
}

/// Apply calibration factor to a score
pub fn apply_calibration(score: u32, calibration_factor_bps: u32) -> u32 {
    ((score as u64 * calibration_factor_bps as u64) / 10000) as u32
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env, Symbol};

    fn create_test_rubric(env: &Env) -> GradingRubric {
        let mut criteria = Vec::new(env);
        criteria.push_back(AssessmentCriteria {
            criterion_id: Symbol::new(env, "knowledge"),
            name: Symbol::new(env, "Knowledge"),
            description: Symbol::new(env, "subject_knowledge"),
            weight_bps: 4000,
            min_score: 0,
            max_score: 100,
        });
        criteria.push_back(AssessmentCriteria {
            criterion_id: Symbol::new(env, "application"),
            name: Symbol::new(env, "Application"),
            description: Symbol::new(env, "practical_application"),
            weight_bps: 3500,
            min_score: 0,
            max_score: 100,
        });
        criteria.push_back(AssessmentCriteria {
            criterion_id: Symbol::new(env, "communication"),
            name: Symbol::new(env, "Communication"),
            description: Symbol::new(env, "communication_skills"),
            weight_bps: 2500,
            min_score: 0,
            max_score: 100,
        });

        GradingRubric {
            rubric_id: Symbol::new(env, "rubric_v1"),
            skill_domain: Symbol::new(env, "RUST"),
            criteria,
            version: 1,
            created_at: 1000000,
            calibrated_by: Address::generate(env),
            is_active: true,
        }
    }

    #[test]
    fn test_validate_rubric_weights() {
        let env = Env::default();
        let rubric = create_test_rubric(&env);
        assert!(validate_rubric_weights(&rubric));
    }

    #[test]
    fn test_calculate_weighted_score() {
        let env = Env::default();
        let rubric = create_test_rubric(&env);
        let mut scores = Vec::new(&env);
        scores.push_back(80); // knowledge
        scores.push_back(90); // application
        scores.push_back(70); // communication

        let weighted = calculate_weighted_score(&env, &scores, &rubric);
        // Scores normalized to 0-10000 scale: 80->8000, 90->9000, 70->7000
        // (8000*4000 + 9000*3500 + 7000*2500) / 10000 = 8100
        assert_eq!(weighted, 8100);
    }

    #[test]
    fn test_peer_review_consensus() {
        let env = Env::default();
        let rubric = create_test_rubric(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "session1");
        let assessment_id = Symbol::new(&env, "assess1");

        let mut scores = Vec::new(&env);
        scores.push_back(80);
        scores.push_back(90);
        scores.push_back(70);
        let weighted = calculate_weighted_score(&env, &scores, &rubric);

        let assessment = ObjectiveAssessment {
            assessment_id: assessment_id.clone(),
            learner: learner.clone(),
            mentor: mentor.clone(),
            session_id: session_id.clone(),
            rubric_id: rubric.rubric_id.clone(),
            criteria_scores: scores,
            weighted_score: weighted,
            assessed_at: env.ledger().timestamp(),
            assessor: mentor.clone(),
            is_peer_review: false,
        };

        // Create 3 peer reviews with similar scores
        let mut reviews = Vec::new(&env);
        for i in 1u32..=3 {
            let reviewer = Address::generate(&env);
            let mut peer_scores = Vec::new(&env);
            peer_scores.push_back(82);
            peer_scores.push_back(88);
            peer_scores.push_back(72);
            let peer_weighted = calculate_weighted_score(&env, &peer_scores, &rubric);
            
            reviews.push_back(ObjectiveAssessment {
                assessment_id: assessment_id.clone(),
                learner: learner.clone(),
                mentor: mentor.clone(),
                session_id: session_id.clone(),
                rubric_id: rubric.rubric_id.clone(),
                criteria_scores: peer_scores,
                weighted_score: peer_weighted,
                assessed_at: env.ledger().timestamp(),
                assessor: reviewer,
                is_peer_review: true,
            });
        }

        let validation = validate_peer_review(&env, &assessment, &reviews);
        assert_eq!(validation.reviewers.len(), 3);
        assert!(validation.consensus_achieved);
    }

    #[test]
    fn test_calibrate_rubric() {
        let env = Env::default();
        let rubric_id = Symbol::new(&env, "rubric_v1");
        let mut benchmarks = Vec::new(&env);
        let mut expected = Vec::new(&env);
        let mut actual = Vec::new(&env);
        
        for _ in 0..5 {
            benchmarks.push_back(Symbol::new(&env, "bench"));
            expected.push_back(8000); // Expected 80%
            actual.push_back(9000);   // Actual 90% (grade inflation)
        }

        let calibration = calibrate_rubric(&env, rubric_id, &benchmarks, &expected, &actual, Address::generate(&env));
        // Calibration factor should be ~8889 (8000/9000 * 10000)
        assert!(calibration.calibration_factor_bps < 10000);
        assert!(calibration.calibration_factor_bps > 8000);
    }

    #[test]
    fn test_apply_calibration() {
        let score = 9000; // 90%
        let factor = 8889; // ~88.89%
        let calibrated = apply_calibration(score, factor);
        // 9000 * 8889 / 10000 = 8000
        assert_eq!(calibrated, 8000);
    }
}
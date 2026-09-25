#![no_std]

use soroban_sdk::{contracttype, Address, BytesN, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurriculumValidation {
    pub is_valid: bool,
    pub industry_standard_score: u32, // 0 to 10000 bps
    pub competency_mapped: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LearningPathOptimization {
    pub optimized: bool,
    pub efficiency_metric: u32,
    pub extensions_detected: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OutcomeAssessment {
    pub outcome_score: u32,
    pub mentor_incentive_aligned: bool,
    pub manipulation_detected: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CurriculumDispute {
    pub dispute_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub expert_reviewer: Option<Address>,
    pub outcome_validated: bool,
    pub resolution_reason: BytesN<32>,
}

pub fn validate_curriculum_standards(industry_score: u32, competency: bool) -> CurriculumValidation {
    CurriculumValidation {
        is_valid: industry_score >= 7000 && competency,
        industry_standard_score: industry_score,
        competency_mapped: competency,
    }
}

pub fn optimize_learning_path(extensions: u32) -> LearningPathOptimization {
    LearningPathOptimization {
        optimized: extensions == 0,
        efficiency_metric: 10000 - (extensions.min(10) * 1000),
        extensions_detected: extensions,
    }
}

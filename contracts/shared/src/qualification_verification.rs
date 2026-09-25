#![no_std]

use soroban_sdk::{contracttype, Address, BytesN};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CredentialVerification {
    pub is_verified: bool,
    pub credential_hash: BytesN<32>,
    pub attestation_provider: Option<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityValidation {
    pub kyc_verified: bool,
    pub fraud_risk_score: u32,
    pub identity_hash: BytesN<32>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SkillAssessment {
    pub mentor: Address,
    pub skill_score: u32,
    pub passed_assessment: bool,
    pub monitoring_active: bool,
}

pub fn verify_credential_validity(hash: BytesN<32>, provider: Address) -> CredentialVerification {
    CredentialVerification {
        is_verified: true,
        credential_hash: hash,
        attestation_provider: Some(provider),
    }
}

pub fn assess_skill_level(mentor: Address, score: u32) -> SkillAssessment {
    SkillAssessment {
        mentor,
        skill_score: score,
        passed_assessment: score >= 6000,
        monitoring_active: true,
    }
}

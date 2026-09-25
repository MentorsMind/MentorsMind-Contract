#![no_std]

use soroban_sdk::{contracttype, Address, BytesN, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProofOfMentoring {
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub session_hash: BytesN<32>,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionAuthenticity {
    pub is_authentic: bool,
    pub fraud_probability: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationIntegrity {
    pub mentor: Address,
    pub integrity_score: u32,
    pub manipulations_detected: u32,
}

pub fn generate_mentoring_proof(
    env: &Env,
    session_id: Symbol,
    mentor: Address,
    learner: Address,
    session_hash: BytesN<32>,
) -> ProofOfMentoring {
    ProofOfMentoring {
        session_id,
        mentor,
        learner,
        session_hash,
        timestamp: env.ledger().timestamp(),
    }
}

pub fn check_session_authenticity(fraud_probability: u32) -> SessionAuthenticity {
    SessionAuthenticity {
        is_authentic: fraud_probability < 3000,
        fraud_probability,
    }
}

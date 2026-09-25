use soroban_sdk::{contracttype, xdr::ToXdr, Address, Bytes, BytesN, Env, Symbol, Vec};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReputationProof {
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub completed_at: u64,
    pub commitment: BytesN<32>,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct BehavioralAnalysis {
    pub risk_score: u32,
    pub timing_flag: bool,
    pub frequency_flag: bool,
    pub distribution_flag: bool,
}

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub struct SybilDetection {
    pub suspicious: bool,
    pub risk_score: u32,
}

pub fn interaction_commitment(
    env: &Env,
    session_id: &Symbol,
    mentor: &Address,
    learner: &Address,
    completed_at: u64,
) -> BytesN<32> {
    let mut payload = Bytes::new(env);
    payload.append(&session_id.to_xdr(env));
    payload.append(&mentor.to_xdr(env));
    payload.append(&learner.to_xdr(env));
    payload.append(&Bytes::from_array(env, &completed_at.to_be_bytes()));
    env.crypto().sha256(&payload).into()
}

/// Scores recent review timestamps and ratings. The score is deliberately
/// bounded and deterministic so contracts can use it as a circuit breaker.
pub fn analyze_review_pattern(
    timestamps: &Vec<u64>,
    ratings: &Vec<u32>,
    now: u64,
) -> BehavioralAnalysis {
    let mut risk = 0u32;
    let mut timing_flag = false;
    let mut frequency_flag = false;
    let mut distribution_flag = false;

    if timestamps.len() >= 3 {
        let recent = timestamps.len() - 1;
        let last = timestamps.get(recent).unwrap_or(0);
        let previous = timestamps.get(recent - 1).unwrap_or(last);
        if last.saturating_sub(previous) < 300 || now.saturating_sub(last) < 60 {
            timing_flag = true;
            risk = risk.saturating_add(35);
        }
        if timestamps.len() >= 5 {
            frequency_flag = true;
            risk = risk.saturating_add(25);
        }
    }

    if ratings.len() >= 3 {
        let mut perfect = 0u32;
        for rating in ratings.iter() {
            if rating >= 5 {
                perfect += 1;
            }
        }
        if perfect * 100 >= (ratings.len() as u32) * 80 {
            distribution_flag = true;
            risk = risk.saturating_add(40);
        }
    }

    BehavioralAnalysis {
        risk_score: risk.min(100),
        timing_flag,
        frequency_flag,
        distribution_flag,
    }
}

pub fn detect_sybil(analysis: BehavioralAnalysis, distinct_learners: u32) -> SybilDetection {
    let identity_risk = if distinct_learners == 0 { 30 } else { 0 };
    let risk_score = analysis.risk_score.saturating_add(identity_risk).min(100);
    SybilDetection {
        suspicious: risk_score >= 60,
        risk_score,
    }
}
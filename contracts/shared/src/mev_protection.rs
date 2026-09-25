use soroban_sdk::{contracttype, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MevProtectionFlag {
    pub is_arbitrage: bool,
    pub is_sandwich: bool,
    pub risk_score: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FairValueExtractionRecord {
    pub extracted_amount: i128,
    pub penalty_bps: u32,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MevMonitoringRecord {
    pub protocol: Symbol,
    pub caller: Address,
    pub detected_arbitrage: bool,
    pub value_extracted: i128,
    pub timestamp: u64,
}

pub const MEV_ARBITRAGE_RISK_THRESHOLD: u32 = 75;
pub const DEFAULT_MEV_PENALTY_BPS: u32 = 500;
pub const MAX_MEV_PENALTY_BPS: u32 = 2500;

pub fn detect_atomic_arbitrage(
    _env: &Env,
    _caller: &Address,
    recent_interactions_count: u32,
) -> MevProtectionFlag {
    let is_arbitrage = recent_interactions_count >= 3;
    let is_sandwich = recent_interactions_count >= 5;
    
    let mut risk_score = 0;
    if is_arbitrage {
        risk_score += 50;
    }
    if is_sandwich {
        risk_score += 40;
    }
    
    MevProtectionFlag {
        is_arbitrage,
        is_sandwich,
        risk_score,
    }
}

pub fn enforce_protocol_isolation(flag: &MevProtectionFlag) -> bool {
    flag.risk_score < MEV_ARBITRAGE_RISK_THRESHOLD
}

pub fn compute_mev_redistribution(
    env: &Env,
    transaction_volume: i128,
    flag: &MevProtectionFlag,
) -> FairValueExtractionRecord {
    let mut penalty_bps = 0;
    let mut amount = 0;
    
    if flag.is_arbitrage {
        penalty_bps = DEFAULT_MEV_PENALTY_BPS;
        if flag.is_sandwich {
            penalty_bps = MAX_MEV_PENALTY_BPS;
        }
        
        amount = transaction_volume.checked_mul(penalty_bps as i128).unwrap_or(0) / 10000;
    }
    
    FairValueExtractionRecord {
        extracted_amount: amount,
        penalty_bps,
        timestamp: env.ledger().timestamp(),
    }
}

pub fn record_mev_monitoring(
    env: &Env,
    protocol: Symbol,
    caller: Address,
    flag: &MevProtectionFlag,
    extraction: &FairValueExtractionRecord,
) -> MevMonitoringRecord {
    let record = MevMonitoringRecord {
        protocol: protocol.clone(),
        caller: caller.clone(),
        detected_arbitrage: flag.is_arbitrage,
        value_extracted: extraction.extracted_amount,
        timestamp: env.ledger().timestamp(),
    };
    
    env.events().publish(
        (Symbol::new(env, "mev"), Symbol::new(env, "monitoring"), protocol),
        (caller, flag.is_arbitrage, extraction.extracted_amount)
    );
    
    record
}

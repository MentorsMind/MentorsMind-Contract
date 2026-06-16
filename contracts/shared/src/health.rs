use soroban_sdk::{contracttype, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthMetric {
    pub contract: Symbol,
    pub metric: Symbol,
    pub value: i128,
    pub timestamp: u64,
}

pub trait HealthReporter {
    fn report_metric(env: Env, metric: Symbol, value: i128);
}

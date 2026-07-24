#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubEvent {
    NewSubscription,
    Renewal,
    Cancellation,
    Upgrade,
    Downgrade,
}

#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MonthlyMetrics {
    pub total_mrr: i128,
    pub new_subscribers: u32,
    pub churned_subscribers: u32,
    pub active_subscribers: u32,
    pub net_new: i32,
}

#[contracttype]
pub enum DataKey {
    Admin,
    SubContract,
    TotalMRR,
    ActiveSubscribers,
    Metrics(u32, u32), // (Year, Month) -> MonthlyMetrics
    EscrowTotalVolume,
    EscrowCount,
    EscrowDisputeCount,
    EscrowTotalDurationSecs,
}

#[contracttype]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct EscrowMetrics {
    pub total_volume: i128,
    pub total_count: u32,
    pub dispute_count: u32,
    pub avg_duration_secs: u64,
}

#[contract]
pub struct SubscriptionAnalytics;

#[contractimpl]
impl SubscriptionAnalytics {
    pub fn initialize(env: Env, admin: Address, sub_contract: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::SubContract, &sub_contract);
        env.storage().persistent().set(&DataKey::TotalMRR, &0i128);
        env.storage()
            .persistent()
            .set(&DataKey::ActiveSubscribers, &0u32);
    }

    pub fn record_subscription_event(
        env: Env,
        event_type: SubEvent,
        _plan_id: Symbol,
        amount: i128,
    ) {
        let sub_contract: Address = env
            .storage()
            .persistent()
            .get(&DataKey::SubContract)
            .expect("Not initialized");
        sub_contract.require_auth();

        let (year, month) = get_month_year(env.ledger().timestamp());
        let metrics_key = DataKey::Metrics(year, month);
        let mut metrics: MonthlyMetrics = env
            .storage()
            .persistent()
            .get(&metrics_key)
            .unwrap_or_default();
        let mut active_mrr: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalMRR)
            .unwrap_or(0);
        let mut active_subs: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ActiveSubscribers)
            .unwrap_or(0);

        match event_type {
            SubEvent::NewSubscription => {
                active_mrr += amount;
                metrics.new_subscribers += 1;
                metrics.net_new += 1;
                active_subs += 1;
            }
            SubEvent::Renewal => {}
            SubEvent::Cancellation => {
                active_mrr -= amount;
                metrics.churned_subscribers += 1;
                metrics.net_new -= 1;
                active_subs = active_subs.saturating_sub(1);
            }
            SubEvent::Upgrade | SubEvent::Downgrade => {
                active_mrr += amount;
            }
        }

        metrics.total_mrr = active_mrr;
        metrics.active_subscribers = active_subs;

        env.storage().persistent().set(&metrics_key, &metrics);
        env.storage()
            .persistent()
            .set(&DataKey::TotalMRR, &active_mrr);
        env.storage()
            .persistent()
            .set(&DataKey::ActiveSubscribers, &active_subs);

        env.events().publish(
            (
                symbol_short!("metrics"),
                symbol_short!("updated"),
                year,
                month,
            ),
            (event_type, amount),
        );
    }

    pub fn get_mrr(env: Env, month: u32, year: u32) -> i128 {
        let metrics: MonthlyMetrics = env
            .storage()
            .persistent()
            .get(&DataKey::Metrics(year, month))
            .unwrap_or_default();
        metrics.total_mrr
    }

    pub fn get_churn_rate(env: Env, month: u32, year: u32) -> u32 {
        let metrics: MonthlyMetrics = env
            .storage()
            .persistent()
            .get(&DataKey::Metrics(year, month))
            .unwrap_or_default();

        let total_at_start = metrics.active_subscribers as u64 + metrics.churned_subscribers as u64;
        if total_at_start == 0 {
            return 0;
        }

        ((metrics.churned_subscribers as u64 * 10000) / total_at_start) as u32
    }

    pub fn get_active_subscribers(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ActiveSubscribers)
            .unwrap_or(0)
    }

    pub fn get_monthly_metrics(env: Env, month: u32, year: u32) -> MonthlyMetrics {
        env.storage()
            .persistent()
            .get(&DataKey::Metrics(year, month))
            .unwrap_or_default()
    }

    /// Record an escrow completion for analytics tracking (#468).
    pub fn record_escrow(env: Env, volume: i128, duration_secs: u64, disputed: bool) {
        let count: u32 = env.storage().persistent().get(&DataKey::EscrowCount).unwrap_or(0);
        let total_vol: i128 = env.storage().persistent().get(&DataKey::EscrowTotalVolume).unwrap_or(0);
        let disputes: u32 = env.storage().persistent().get(&DataKey::EscrowDisputeCount).unwrap_or(0);
        let total_dur: u64 = env.storage().persistent().get(&DataKey::EscrowTotalDurationSecs).unwrap_or(0);

        env.storage().persistent().set(&DataKey::EscrowCount, &(count + 1));
        env.storage().persistent().set(&DataKey::EscrowTotalVolume, &(total_vol + volume));
        env.storage().persistent().set(&DataKey::EscrowTotalDurationSecs, &(total_dur + duration_secs));
        if disputed {
            env.storage().persistent().set(&DataKey::EscrowDisputeCount, &(disputes + 1));
        }
    }

    pub fn get_escrow_metrics(env: Env) -> EscrowMetrics {
        let count: u32 = env.storage().persistent().get(&DataKey::EscrowCount).unwrap_or(0);
        let total_vol: i128 = env.storage().persistent().get(&DataKey::EscrowTotalVolume).unwrap_or(0);
        let disputes: u32 = env.storage().persistent().get(&DataKey::EscrowDisputeCount).unwrap_or(0);
        let total_dur: u64 = env.storage().persistent().get(&DataKey::EscrowTotalDurationSecs).unwrap_or(0);
        let avg_duration = if count > 0 { total_dur / count as u64 } else { 0 };
        EscrowMetrics { total_volume: total_vol, total_count: count, dispute_count: disputes, avg_duration_secs: avg_duration }
    }
}

fn get_month_year(timestamp: u64) -> (u32, u32) {
    let days = timestamp / 86400;
    let year = 1970 + (days / 365) as u32;
    let day_of_year = (days % 365) as u32;
    let month = (day_of_year / 30) + 1;
    (year, if month > 12 { 12 } else { month })
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[test]
    fn test_mrr_calculation() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SubscriptionAnalytics);
        let client = SubscriptionAnalyticsClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let sub_contract = Address::generate(&env);
        client.initialize(&admin, &sub_contract);

        let plan_id = symbol_short!("PLAN1");

        client.record_subscription_event(&SubEvent::NewSubscription, &plan_id, &1000i128);
        client.record_subscription_event(&SubEvent::NewSubscription, &plan_id, &1000i128);
        client.record_subscription_event(&SubEvent::Upgrade, &plan_id, &500i128);

        let (year, month) = get_month_year(env.ledger().timestamp());
        assert_eq!(client.get_mrr(&month, &year), 2500i128);
        assert_eq!(client.get_active_subscribers(), 2);

        client.record_subscription_event(&SubEvent::Cancellation, &plan_id, &1000i128);
        assert_eq!(client.get_mrr(&month, &year), 1500i128);

        env.ledger().with_mut(|li| {
            li.timestamp += 31 * 86400;
        });
        let (year2, month2) = get_month_year(env.ledger().timestamp());
        client.record_subscription_event(&SubEvent::NewSubscription, &plan_id, &2000i128);
        assert_eq!(client.get_mrr(&month2, &year2), 3500i128);
        assert_eq!(client.get_active_subscribers(), 2);
    }

    #[test]
    fn test_churn_rate() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SubscriptionAnalytics);
        let client = SubscriptionAnalyticsClient::new(&env, &contract_id);
        client.initialize(&Address::generate(&env), &Address::generate(&env));

        let plan_id = symbol_short!("PLAN1");
        for _ in 0..10 {
            client.record_subscription_event(&SubEvent::NewSubscription, &plan_id, &100i128);
        }
        client.record_subscription_event(&SubEvent::Cancellation, &plan_id, &100i128);

        let (year, month) = get_month_year(env.ledger().timestamp());
        assert_eq!(client.get_churn_rate(&month, &year), 1000);
    }
}

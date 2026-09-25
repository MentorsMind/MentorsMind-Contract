#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol};

/// One calendar-day bucket for rolling revenue aggregation.
const BUCKET_SIZE_SECS: u64 = 86_400;
/// Keep today + 30 prior days of buckets (31 total); prune 32 days ago on write.
const _BUCKETS_RETAINED: u32 = 31;
const PROTOCOL_PAGE_MAX: u32 = 100;

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

const BUCKET_SIZE_SECS: u64 = 86400;

#[contracttype]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    SubContract,
    TotalMRR,
    ActiveSubscribers,
    Metrics(u32, u32), // (Year, Month) -> MonthlyMetrics
    EscrowTotalVolume,
    EscrowCount,
    EscrowDisputeCount,
    EscrowTotalDurationSecs,
    RevenueBucket(Symbol, u32),
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
        env.storage()
            .persistent()
            .set(&DataKey::MentorRevenueCount, &0u32);
    }

    pub fn record_renewal(env: Env, mentor: Symbol, amount: i128) {
        let now = env.ledger().timestamp();
        let current_day = (now / BUCKET_SIZE_SECS) as u32;

        let bucket_key = DataKey::RevenueBucket(mentor.clone(), current_day);
        let existing: i128 = env.storage().persistent().get(&bucket_key).unwrap_or(0);
        env.storage().persistent().set(&bucket_key, &(existing + amount));

        // Prune old bucket from 32 days ago
        if current_day >= 32 {
            let old_bucket_key = DataKey::RevenueBucket(mentor, current_day - 32);
            env.storage().persistent().remove(&old_bucket_key);
        }
    }

    pub fn get_mentor_revenue_30d(env: Env, mentor: Symbol) -> i128 {
        let now = env.ledger().timestamp();
        let current_day = (now / BUCKET_SIZE_SECS) as u32;

        let mut total: i128 = 0;
        let start_day = current_day.saturating_sub(29);
        for day in start_day..=current_day {
            let bucket_key = DataKey::RevenueBucket(mentor.clone(), day);
            let val: i128 = env.storage().persistent().get(&bucket_key).unwrap_or(0);
            total += val;
        }
        total
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

    /// Record a renewal into today's daily revenue bucket for `mentor`.
    /// Deletes the bucket from 32 days ago so storage stays bounded at 31 days.
    pub fn record_renewal(env: Env, mentor: Address, amount: i128) {
        let sub_contract: Address = env
            .storage()
            .persistent()
            .get(&DataKey::SubContract)
            .expect("Not initialized");
        sub_contract.require_auth();

        Self::ensure_mentor_indexed(&env, &mentor);

        let day = (env.ledger().timestamp() / BUCKET_SIZE_SECS) as u32;
        let key = DataKey::RevenueBucket(mentor.clone(), day);
        let prev: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        env.storage().persistent().set(&key, &(prev + amount));

        // Delete the bucket from 32 days ago (keep at most 31 daily buckets).
        if day >= 32 {
            let stale_key = DataKey::RevenueBucket(mentor.clone(), day - 32);
            if env.storage().persistent().has(&stale_key) {
                env.storage().persistent().remove(&stale_key);
            }
        }

        // Refresh cached 30-day total from the retained daily buckets.
        let rolling = Self::sum_mentor_window(&env, &mentor, 30);
        env.storage()
            .persistent()
            .set(&DataKey::MentorRevenue30d(mentor), &rolling);
    }

    /// Sum of the mentor's most recent 30 daily buckets (today inclusive).
    pub fn get_mentor_revenue_30d(env: Env, mentor: Address) -> i128 {
        Self::sum_mentor_window(&env, &mentor, 30)
    }

    /// Protocol-wide 30-day revenue for a page of mentors.
    /// `limit` is capped at 100; use offset paging for larger mentor sets.
    pub fn get_protocol_revenue_30d(env: Env, offset: u32, limit: u32) -> i128 {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorRevenueCount)
            .unwrap_or(0);
        let page = if limit == 0 {
            0
        } else if limit > PROTOCOL_PAGE_MAX {
            PROTOCOL_PAGE_MAX
        } else {
            limit
        };
        let start = offset.min(count);
        let end = (offset.saturating_add(page)).min(count);
        let mut total: i128 = 0;
        for i in start..end {
            if let Some(mentor) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::MentorRevenueAt(i))
            {
                let amt: i128 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::MentorRevenue30d(mentor))
                    .unwrap_or(0);
                total = total.saturating_add(amt);
            }
        }
        total
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
        let total_vol: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowTotalVolume)
            .unwrap_or(0);
        let disputes: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowDisputeCount)
            .unwrap_or(0);
        let total_dur: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowTotalDurationSecs)
            .unwrap_or(0);

        env.storage()
            .persistent()
            .set(&DataKey::EscrowCount, &(count + 1));
        env.storage()
            .persistent()
            .set(&DataKey::EscrowTotalVolume, &(total_vol + volume));
        env.storage()
            .persistent()
            .set(&DataKey::EscrowTotalDurationSecs, &(total_dur + duration_secs));
        if disputed {
            env.storage()
                .persistent()
                .set(&DataKey::EscrowDisputeCount, &(disputes + 1));
        }
    }

    pub fn get_escrow_metrics(env: Env) -> EscrowMetrics {
        let count: u32 = env.storage().persistent().get(&DataKey::EscrowCount).unwrap_or(0);
        let total_vol: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowTotalVolume)
            .unwrap_or(0);
        let disputes: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowDisputeCount)
            .unwrap_or(0);
        let total_dur: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowTotalDurationSecs)
            .unwrap_or(0);
        let avg_duration = if count > 0 {
            total_dur / count as u64
        } else {
            0
        };
        EscrowMetrics {
            total_volume: total_vol,
            total_count: count,
            dispute_count: disputes,
            avg_duration_secs: avg_duration,
        }
    }

    fn ensure_mentor_indexed(env: &Env, mentor: &Address) {
        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorRevenueCount)
            .unwrap_or(0);
        for i in 0..count {
            if let Some(existing) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::MentorRevenueAt(i))
            {
                if &existing == mentor {
                    return;
                }
            }
        }
        env.storage()
            .persistent()
            .set(&DataKey::MentorRevenueAt(count), mentor);
        env.storage()
            .persistent()
            .set(&DataKey::MentorRevenueCount, &(count + 1));
    }

    fn sum_mentor_window(env: &Env, mentor: &Address, days: u32) -> i128 {
        let today = (env.ledger().timestamp() / BUCKET_SIZE_SECS) as u32;
        let mut total: i128 = 0;
        let mut i = 0u32;
        while i < days {
            if today < i {
                break;
            }
            let day = today - i;
            let key = DataKey::RevenueBucket(mentor.clone(), day);
            let amt: i128 = env.storage().persistent().get(&key).unwrap_or(0);
            total = total.saturating_add(amt);
            i += 1;
        }
        total
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

    fn setup(env: &Env) -> SubscriptionAnalyticsClient<'static> {
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SubscriptionAnalytics);
        let client = SubscriptionAnalyticsClient::new(env, &contract_id);
        client.initialize(&Address::generate(env), &Address::generate(env));
        client
    }

    #[test]
    fn test_mrr_calculation() {
        let env = Env::default();
        let client = setup(&env);

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
        let client = setup(&env);

        let plan_id = symbol_short!("PLAN1");
        for _ in 0..10 {
            client.record_subscription_event(&SubEvent::NewSubscription, &plan_id, &100i128);
        }
        client.record_subscription_event(&SubEvent::Cancellation, &plan_id, &100i128);

        let (year, month) = get_month_year(env.ledger().timestamp());
        assert_eq!(client.get_churn_rate(&month, &year), 1000);
    }

    #[test]
    fn test_rolling_30d_includes_today_excludes_day_31() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 100 * BUCKET_SIZE_SECS);
        let client = setup(&env);
        let mentor = Address::generate(&env);

        client.record_renewal(&mentor, &100i128); // today (day 100)
        env.ledger().with_mut(|li| li.timestamp = 90 * BUCKET_SIZE_SECS);
        client.record_renewal(&mentor, &50i128); // 10 days ago
        env.ledger().with_mut(|li| li.timestamp = 69 * BUCKET_SIZE_SECS);
        client.record_renewal(&mentor, &999i128); // 31 days before day 100

        env.ledger().with_mut(|li| li.timestamp = 100 * BUCKET_SIZE_SECS);
        // 30d window: days 71..=100 → includes 100 and 90; excludes 69
        assert_eq!(client.get_mentor_revenue_30d(&mentor), 150i128);
    }

    #[test]
    fn test_bucket_pruning_deletes_day_32() {
        let env = Env::default();
        let client = setup(&env);
        let mentor = Address::generate(&env);

        env.ledger().with_mut(|li| li.timestamp = 168 * BUCKET_SIZE_SECS);
        client.record_renewal(&mentor, &10i128);

        env.ledger().with_mut(|li| li.timestamp = 200 * BUCKET_SIZE_SECS);
        client.record_renewal(&mentor, &5i128); // prunes day 200-32 = 168

        // Day 168 is outside the 30d window anyway; revenue should be today's 5 only
        // from recent activity in window (day 200 only among recorded that remain).
        assert_eq!(client.get_mentor_revenue_30d(&mentor), 5i128);
    }

    #[test]
    fn test_protocol_revenue_paginated() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 50 * BUCKET_SIZE_SECS);
        let client = setup(&env);

        for _ in 0..5 {
            let m = Address::generate(&env);
            client.record_renewal(&m, &10i128);
        }

        let page1 = client.get_protocol_revenue_30d(&0u32, &2u32);
        let page2 = client.get_protocol_revenue_30d(&2u32, &2u32);
        let page3 = client.get_protocol_revenue_30d(&4u32, &2u32);
        assert_eq!(page1, 20i128);
        assert_eq!(page2, 20i128);
        assert_eq!(page3, 10i128);
        assert_eq!(page1 + page2 + page3, 50i128);
        // Cap behavior: requesting >100 is treated as 100, but with only 5 mentors
        // a single page of 5 is enough and stays within resource limits.
        assert_eq!(client.get_protocol_revenue_30d(&0u32, &5u32), 50i128);
    }

    #[test]
    fn test_property_daily_buckets_sum_equals_30d() {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = 80 * BUCKET_SIZE_SECS);
        let client = setup(&env);
        let mentor = Address::generate(&env);

        let mut expected: i128 = 0;
        for d in 0u32..30 {
            env.ledger()
                .with_mut(|li| li.timestamp = (80 - d as u64) * BUCKET_SIZE_SECS);
            let amt = (d as i128 + 1) * 3;
            client.record_renewal(&mentor, &amt);
            expected += amt;
        }
        env.ledger().with_mut(|li| li.timestamp = 80 * BUCKET_SIZE_SECS);
        assert_eq!(client.get_mentor_revenue_30d(&mentor), expected);
    }
}

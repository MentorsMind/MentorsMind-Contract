#![cfg(test)]

use shared::health_reporter::{AlertSeverity, HealthMetric, HealthThresholds, MetricCategory};
use soroban_sdk::{testutils::Address as _, Address, Env, Symbol};

extern crate mentorminds_health_dashboard;
use mentorminds_health_dashboard::HealthDashboardContract;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    let contract_address = env.register_contract(None, HealthDashboardContract);
    let admin = Address::generate(&env);
    let escrow = Address::generate(&env);
    let session_registry = Address::generate(&env);
    let staking = Address::generate(&env);
    let mnt_token = Address::generate(&env);
    let reputation = Address::generate(&env);
    let interface_registry = Address::generate(&env);
    let treasury = Address::generate(&env);
    let insurance = Address::generate(&env);
    let lending_pool = Address::generate(&env);
    let usdc_token = Address::generate(&env);

    env.as_contract(&contract_address, || {
        HealthDashboardContract::initialize(
            env.clone(),
            admin.clone(),
            escrow,
            session_registry,
            staking,
            mnt_token,
            reputation,
            interface_registry,
            treasury,
            insurance,
            lending_pool,
            usdc_token,
        );
    });

    (env, contract_address, admin)
}

fn make_metric(
    env: &Env,
    name: &str,
    category: MetricCategory,
    value: i128,
    alert: AlertSeverity,
) -> HealthMetric {
    HealthMetric {
        name: Symbol::new(env, name),
        category,
        value,
        recorded_at: env.ledger().timestamp(),
        source: Address::generate(env),
        alert,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[test]
fn test_record_and_get_metric() {
    let (env, contract_address, _admin) = setup();
    let metric = make_metric(
        &env,
        "total_tvl",
        MetricCategory::Liquidity,
        1_000_000,
        AlertSeverity::Info,
    );

    env.as_contract(&contract_address, || {
        HealthDashboardContract::record_metric(env.clone(), metric.clone());

        let page = HealthDashboardContract::get_system_health(env.clone(), 0);
        assert_eq!(page.len(), 1);
        assert_eq!(page.get_unchecked(0), metric);

        let page_count = HealthDashboardContract::get_metric_page_count(env.clone());
        assert_eq!(page_count, 1);
    });
}

#[test]
fn test_pagination() {
    let (env, contract_address, _admin) = setup();

    env.as_contract(&contract_address, || {
        // Record 5 metrics (within a single page)
        for i in 0..5 {
            let metric = make_metric(
                &env,
                &format!("metric_{}", i),
                MetricCategory::Throughput,
                i as i128 * 100,
                AlertSeverity::Info,
            );
            HealthDashboardContract::record_metric(env.clone(), metric);
        }

        let page_count = HealthDashboardContract::get_metric_page_count(env.clone());
        assert_eq!(page_count, 1);

        let page = HealthDashboardContract::get_system_health(env.clone(), 0);
        assert_eq!(page.len(), 5);

        // Page 1 doesn't exist yet
        let page1 = HealthDashboardContract::get_system_health(env.clone(), 1);
        assert_eq!(page1.len(), 0);
    });
}

#[test]
fn test_empty_page_returns_empty_vec() {
    let (env, contract_address, _admin) = setup();

    env.as_contract(&contract_address, || {
        let page = HealthDashboardContract::get_system_health(env.clone(), 99);
        assert_eq!(page.len(), 0);
    });
}

#[test]
fn test_is_system_healthy_empty() {
    let (env, contract_address, _admin) = setup();

    env.as_contract(&contract_address, || {
        let health = HealthDashboardContract::is_system_healthy(env.clone());
        assert!(health.is_healthy);
        assert_eq!(health.total_metrics, 0);
        assert_eq!(health.warning_count, 0);
        assert_eq!(health.critical_count, 0);
        assert_eq!(health.total_tvl, 0);
        assert_eq!(health.error_rate_bps, 0);
        assert_eq!(health.active_sources, 0);
    });
}

#[test]
fn test_is_system_healthy_with_metrics() {
    let (env, contract_address, _admin) = setup();

    env.as_contract(&contract_address, || {
        // Add liquidity metric
        let m1 = make_metric(&env, "tvl", MetricCategory::Liquidity, 500_000, AlertSeverity::Info);
        HealthDashboardContract::record_metric(env.clone(), m1);

        // Add warning metric (non-ErrorRate category so error_rate stays 0)
        let m2 = make_metric(&env, "warning_metric", MetricCategory::Availability, 0, AlertSeverity::Warning);
        HealthDashboardContract::record_metric(env.clone(), m2);

        let health = HealthDashboardContract::is_system_healthy(env.clone());
        assert!(health.is_healthy);
        assert_eq!(health.total_metrics, 2);
        assert_eq!(health.warning_count, 1);
        assert_eq!(health.total_tvl, 500_000);
    });
}

#[test]
fn test_is_system_healthy_unhealthy_on_critical() {
    let (env, contract_address, admin) = setup();

    env.mock_all_auths();
    env.as_contract(&contract_address, || {
        // Set strict thresholds
        let thresholds = HealthThresholds {
            max_error_rate_bps: 100,
            min_tvl: 0,
            max_critical_alerts: 1,
            max_warning_alerts: 5,
        };
        HealthDashboardContract::set_health_thresholds(
            env.clone(),
            admin,
            thresholds,
        );

        // Add 2 critical metrics (exceeds max_critical_alerts = 1)
        for i in 0..2 {
            let m = make_metric(
                &env,
                &format!("critical_{}", i),
                MetricCategory::Security,
                0,
                AlertSeverity::Critical,
            );
            HealthDashboardContract::record_metric(env.clone(), m);
        }

        let health = HealthDashboardContract::is_system_healthy(env.clone());
        assert!(!health.is_healthy);
        assert_eq!(health.critical_count, 2);
    });
}

#[test]
fn test_set_health_thresholds() {
    let (env, contract_address, admin) = setup();

    let thresholds = HealthThresholds {
        max_error_rate_bps: 200,
        min_tvl: 10_000,
        max_critical_alerts: 5,
        max_warning_alerts: 20,
    };

    env.mock_all_auths();
    env.as_contract(&contract_address, || {
        HealthDashboardContract::set_health_thresholds(env.clone(), admin, thresholds.clone());
        let stored = HealthDashboardContract::get_health_thresholds(env.clone());
        assert_eq!(stored, thresholds);
    });
}

#[test]
fn test_metric_alert_counts() {
    let (env, contract_address, _admin) = setup();

    env.as_contract(&contract_address, || {
        // Info
        let m1 = make_metric(&env, "ok", MetricCategory::Availability, 100, AlertSeverity::Info);
        HealthDashboardContract::record_metric(env.clone(), m1);

        // Warning
        let m2 = make_metric(&env, "warn", MetricCategory::Availability, 50, AlertSeverity::Warning);
        HealthDashboardContract::record_metric(env.clone(), m2);

        // Critical
        let m3 = make_metric(&env, "crit", MetricCategory::Availability, 10, AlertSeverity::Critical);
        HealthDashboardContract::record_metric(env.clone(), m3);

        let health = HealthDashboardContract::is_system_healthy(env.clone());
        assert_eq!(health.total_metrics, 3);
        assert_eq!(health.warning_count, 1);
        assert_eq!(health.critical_count, 1);
    });
}

#[test]
fn test_error_rate_calculation() {
    let (env, contract_address, _admin) = setup();

    env.as_contract(&contract_address, || {
        // 2 error rate metrics out of 4 total = 50% = 5000 bps
        for i in 0..4 {
            let category = if i < 2 {
                MetricCategory::ErrorRate
            } else {
                MetricCategory::Liquidity
            };
            let m = make_metric(
                &env,
                &format!("metric_{}", i),
                category,
                if i < 2 { 1 } else { 100_000 },
                AlertSeverity::Info,
            );
            HealthDashboardContract::record_metric(env.clone(), m);
        }

        let health = HealthDashboardContract::is_system_healthy(env.clone());
        assert_eq!(health.error_rate_bps, 5000);
    });
}

#![no_std]

use soroban_sdk::{contracterror, contracttype, Address, Env, IntoVal, Symbol, Val, Vec};

// ---------------------------------------------------------------------------
// Health metric types
// ---------------------------------------------------------------------------

/// Categories of health metrics tracked across the platform.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MetricCategory {
    /// TVL and liquidity metrics
    Liquidity,
    /// Transaction throughput and latency
    Throughput,
    /// Error rates and failure counts
    ErrorRate,
    /// Contract uptime and availability
    Availability,
    /// Economic health indicators
    Economic,
    /// Security and fraud detection
    Security,
}

/// Severity levels for health alerts.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AlertSeverity {
    /// Normal operation, no action needed
    Info,
    /// Elevated but within acceptable bounds
    Warning,
    /// Critical threshold breached, action required
    Critical,
}

/// A single health metric data point.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthMetric {
    /// Unique identifier for this metric (e.g. "tvl_usdc", "error_rate_bps")
    pub name: Symbol,
    /// The metric category
    pub category: MetricCategory,
    /// The metric value (interpretation depends on the metric)
    pub value: i128,
    /// When this metric was recorded (ledger timestamp)
    pub recorded_at: u64,
    /// The contract that reported this metric
    pub source: Address,
    /// Optional alert severity if threshold breached
    pub alert: AlertSeverity,
}

/// Aggregated system health status.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SystemHealth {
    /// Whether the system is considered healthy overall
    pub is_healthy: bool,
    /// Total number of metrics recorded
    pub total_metrics: u32,
    /// Number of metrics with Warning severity
    pub warning_count: u32,
    /// Number of metrics with Critical severity
    pub critical_count: u32,
    /// Timestamp of the most recent metric
    pub last_updated: u64,
    /// Aggregated TVL across all contracts
    pub total_tvl: i128,
    /// Overall error rate in basis points
    pub error_rate_bps: u32,
    /// Number of active contracts reporting
    pub active_sources: u32,
}

/// Configurable thresholds for system health determination.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct HealthThresholds {
    /// Maximum acceptable error rate in basis points (default: 500 = 5%)
    pub max_error_rate_bps: u32,
    /// Minimum TVL to consider system solvent (default: 0)
    pub min_tvl: i128,
    /// Maximum number of critical alerts before marking unhealthy
    pub max_critical_alerts: u32,
    /// Maximum number of warning alerts before marking degraded
    pub max_warning_alerts: u32,
}

impl Default for HealthThresholds {
    fn default() -> Self {
        HealthThresholds {
            max_error_rate_bps: 500,
            min_tvl: 0,
            max_critical_alerts: 3,
            max_warning_alerts: 10,
        }
    }
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum HealthReporterError {
    /// Contract not initialized
    NotInitialized = 1,
    /// Metric name is empty
    EmptyMetricName = 2,
    /// Source contract not authorized
    UnauthorizedSource = 3,
    /// Storage error
    StorageError = 4,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum HealthStorageKey {
    /// Global configuration
    Config,
    /// Paginated metrics: (page_index) -> Vec<HealthMetric>
    MetricPage(u32),
    /// Total number of metric pages
    PageCount,
    /// Current page index (for append)
    CurrentPage,
    /// Metrics per page
    PageSize,
    /// Thresholds for health determination
    Thresholds,
    /// Last system health snapshot
    LastHealth,
}

// ---------------------------------------------------------------------------
// Health reporter trait (for cross-contract metric reporting)
// ---------------------------------------------------------------------------

/// Trait that contracts can implement to report health metrics.
/// This is a documentation trait; actual reporting uses the
/// `report_metric` free function which writes to the health dashboard
/// contract's storage via cross-contract invocation.
pub trait HealthReporter {
    /// Called after a significant state change to report a health metric.
    fn report_health_metric(
        env: &Env,
        name: Symbol,
        category: MetricCategory,
        value: i128,
    );
}

/// Report a health metric to the health dashboard contract.
///
/// This function should be called by contracts on relevant state changes.
/// It invokes the health dashboard's `record_metric` function.
pub fn report_metric(
    env: &Env,
    health_dashboard: &Address,
    name: Symbol,
    category: MetricCategory,
    value: i128,
) {
    let source = env.current_contract_address();
    let recorded_at = env.ledger().timestamp();

    let metric = HealthMetric {
        name,
        category,
        value,
        recorded_at,
        source,
        alert: AlertSeverity::Info,
    };

    let mut args: Vec<Val> = Vec::new(env);
    args.push_back(metric.into_val(env));
    let _: () = env.invoke_contract(
        health_dashboard,
        &Symbol::new(env, "record_metric"),
        args,
    );
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[test]
    fn test_health_thresholds_default() {
        let t = HealthThresholds::default();
        assert_eq!(t.max_error_rate_bps, 500);
        assert_eq!(t.min_tvl, 0);
        assert_eq!(t.max_critical_alerts, 3);
        assert_eq!(t.max_warning_alerts, 10);
    }

    #[test]
    fn test_health_metric_creation() {
        let env = Env::default();
        let addr = Address::generate(&env);
        let metric = HealthMetric {
            name: Symbol::new(&env, "tvl_usdc"),
            category: MetricCategory::Liquidity,
            value: 1_000_000,
            recorded_at: 1000,
            source: addr,
            alert: AlertSeverity::Info,
        };
        assert_eq!(metric.name, Symbol::new(&env, "tvl_usdc"));
        assert_eq!(metric.category, MetricCategory::Liquidity);
        assert_eq!(metric.value, 1_000_000);
    }
}

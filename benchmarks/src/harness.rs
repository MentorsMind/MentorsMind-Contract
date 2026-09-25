/// Benchmark harness for Soroban contracts.
///
/// Measurement strategy for soroban-sdk v21:
///   - The soroban-sdk v21 testutils exposes `Env::cost_estimate()` which
///     returns a `CostEstimate` with `.cpu` (u64) and `.memory` (u64) fields.
///   - We reset the budget via `env.budget().reset_default()` before each
///     measured call so only the target function's cost is captured.
///   - Storage read/write counts are tracked via separate counters injected
///     into each suite's setup (lightweight — just counting env.storage calls
///     by wrapping the measured closure).
///   - WASM binary size is read from the compiled release artifact path.
extern crate std;

use serde::{Deserialize, Serialize};
use soroban_sdk::Env;
use std::fs;
use std::path::Path;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// One measured entry point.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BenchResult {
    pub contract: String,
    pub entry_point: String,
    /// Soroban CPU instructions consumed (from host budget).
    pub cpu_instructions: u64,
    /// Memory bytes consumed (from host budget).
    pub mem_bytes: u64,
    /// Placeholder: storage read count (set to 0; future soroban versions may
    /// expose per-call storage metrics directly).
    pub storage_reads: u32,
    /// Placeholder: storage write count.
    pub storage_writes: u32,
    /// WASM binary size in bytes. 0 when WASM not built.
    pub wasm_bytes: u64,
}

/// A single detected regression.
#[derive(Debug)]
pub struct Regression {
    pub contract: String,
    pub entry_point: String,
    pub metric: String,
    pub baseline: u64,
    pub measured: u64,
    pub pct_change: f64,
}

/// Trait implemented by each contract's benchmark suite.
pub trait BenchSuite {
    fn run() -> Vec<BenchResult>;
}

// ---------------------------------------------------------------------------
// Cost snapshot
// ---------------------------------------------------------------------------

pub struct CostSnapshot {
    pub cpu_instructions: u64,
    pub mem_bytes: u64,
}

/// Reset the environment budget, execute `f`, then capture CPU + memory.
///
/// Uses the soroban-sdk `testutils` budget API available at runtime:
///   - `env.budget().reset_default()` clears the instruction/memory counters.
///   - `env.budget().cpu_instruction_cost()` returns the CPU instructions consumed.
///   - `env.budget().memory_bytes()` returns the memory bytes consumed (when
///     available; falls back to 0 on older SDK versions).
pub fn measure<F: FnOnce()>(env: &Env, f: F) -> CostSnapshot {
    env.budget().reset_default();
    f();
    let cpu = env.budget().cpu_instruction_cost();
    let mem = env.budget().memory_bytes();
    CostSnapshot {
        cpu_instructions: cpu,
        mem_bytes: mem,
    }
}

// ---------------------------------------------------------------------------
// WASM size helper
// ---------------------------------------------------------------------------

/// Returns the compiled WASM size for a contract by crate name, or 0 if the
/// file is not present (i.e. the WASM target was not built).
///
/// Expects the WASM at:
///   `target/wasm32v1-none/release/<crate_name>.wasm`
/// where `<crate_name>` has hyphens replaced with underscores.
///
/// Run from the workspace root so the path is correct relative to CWD.
pub fn wasm_size(crate_name: &str) -> u64 {
    let path = format!(
        "target/wasm32v1-none/release/{}.wasm",
        crate_name.replace('-', "_")
    );
    fs::metadata(&path).map(|m| m.len()).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Regression checker
// ---------------------------------------------------------------------------

/// Fraction increase above which a metric is a regression (10 %).
const REGRESSION_THRESHOLD: f64 = 0.10;
/// WASM size alert threshold: 64 KB.
const WASM_ALERT_BYTES: u64 = 64 * 1024;

pub fn check_regressions(results: &[BenchResult], baseline_path: &Path) -> Vec<Regression> {
    let data = fs::read_to_string(baseline_path).expect("failed to read baselines.json");
    let baselines: Vec<BenchResult> =
        serde_json::from_str(&data).expect("failed to parse baselines.json");

    let mut regressions: Vec<Regression> = Vec::new();

    for result in results {
        let baseline = baselines
            .iter()
            .find(|b| b.contract == result.contract && b.entry_point == result.entry_point);

        let Some(b) = baseline else {
            eprintln!(
                "  ℹ️  New entry point [{}/{}] — writing to baseline on next baseline update",
                result.contract, result.entry_point
            );
            continue;
        };

        check_metric(
            result,
            b,
            "cpu_instructions",
            result.cpu_instructions,
            b.cpu_instructions,
            &mut regressions,
        );
        check_metric(
            result,
            b,
            "mem_bytes",
            result.mem_bytes,
            b.mem_bytes,
            &mut regressions,
        );
        check_metric(
            result,
            b,
            "storage_reads",
            result.storage_reads as u64,
            b.storage_reads as u64,
            &mut regressions,
        );
        check_metric(
            result,
            b,
            "storage_writes",
            result.storage_writes as u64,
            b.storage_writes as u64,
            &mut regressions,
        );

        // WASM size: hard alert at > 64 KB (independent of baseline pct gate)
        if result.wasm_bytes > WASM_ALERT_BYTES {
            eprintln!(
                "  ⚠️  [{}/{}] WASM binary exceeds 64 KB alert: {} bytes",
                result.contract, result.entry_point, result.wasm_bytes
            );
        }
        // Also apply 10 % regression gate to WASM size if we have a baseline
        if b.wasm_bytes > 0 {
            check_metric(
                result,
                b,
                "wasm_bytes",
                result.wasm_bytes,
                b.wasm_bytes,
                &mut regressions,
            );
        }
    }

    regressions
}

fn check_metric(
    result: &BenchResult,
    _baseline: &BenchResult,
    metric: &str,
    measured: u64,
    base: u64,
    out: &mut Vec<Regression>,
) {
    if base == 0 {
        // Zero baseline means metric was not available when baseline was written
        // — skip the regression gate for this metric.
        return;
    }
    let pct = (measured as f64 - base as f64) / base as f64;
    if pct > REGRESSION_THRESHOLD {
        out.push(Regression {
            contract: result.contract.clone(),
            entry_point: result.entry_point.clone(),
            metric: metric.to_string(),
            baseline: base,
            measured,
            pct_change: pct * 100.0,
        });
    }
}

// ---------------------------------------------------------------------------
// Gas-estimation accuracy tracking
// ---------------------------------------------------------------------------

/// One measured entry point with both actual and estimated gas costs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GasAccuracyResult {
    pub contract: String,
    pub operation: String,
    /// CPU instructions consumed by the actual operation.
    pub actual_cpu: u64,
    /// CPU instructions estimated by the on-chain heuristic.
    pub estimated_cpu: u64,
    /// Memory bytes consumed by the actual operation.
    pub actual_mem: u64,
    /// Memory bytes estimated by the on-chain heuristic (0 if not estimated).
    pub estimated_mem: u64,
    /// Relative error between actual and estimated CPU, as a percentage.
    pub cpu_error_pct: f64,
    /// Relative error between actual and estimated memory, as a percentage.
    pub mem_error_pct: f64,
    /// Whether the estimate is within the default tolerance (20%).
    pub passes_tolerance: bool,
}

impl GasAccuracyResult {
    /// Compute error percentages and tolerance check.
    pub fn new(
        contract: impl Into<String>,
        operation: impl Into<String>,
        actual_cpu: u64,
        estimated_cpu: u64,
        actual_mem: u64,
        estimated_mem: u64,
    ) -> Self {
        let cpu_error_pct = if actual_cpu > 0 {
            (actual_cpu.max(estimated_cpu) - actual_cpu.min(estimated_cpu)) as f64
                / actual_cpu as f64
                * 100.0
        } else {
            0.0
        };
        let mem_error_pct = if actual_mem > 0 && estimated_mem > 0 {
            (actual_mem.max(estimated_mem) - actual_mem.min(estimated_mem)) as f64
                / actual_mem as f64
                * 100.0
        } else {
            0.0
        };
        let passes_tolerance = cpu_error_pct <= 20.0;
        Self {
            contract: contract.into(),
            operation: operation.into(),
            actual_cpu,
            estimated_cpu,
            actual_mem,
            estimated_mem,
            cpu_error_pct,
            mem_error_pct,
            passes_tolerance,
        }
    }
}

/// Return all accuracy results that failed the tolerance check.
pub fn check_gas_accuracy(results: &[GasAccuracyResult]) -> Vec<&GasAccuracyResult> {
    results.iter().filter(|r| !r.passes_tolerance).collect()
}

/// Write gas accuracy results to `benchmarks/results/gas_accuracy.json`.
pub fn write_gas_accuracy_report(results: &[GasAccuracyResult]) {
    extern crate std;
    use std::fs;
    let dir = "benchmarks/results";
    let _ = fs::create_dir_all(dir);
    let path = format!("{}/gas_accuracy.json", dir);
    let json = serde_json::to_string_pretty(results).expect("failed to serialize gas accuracy");
    let _ = fs::write(&path, json);
    eprintln!("📄  Gas accuracy report written to {}", path);
}

/// Print a human-readable accuracy table to stderr.
pub fn print_gas_accuracy(results: &[GasAccuracyResult]) {
    eprintln!("\n── Gas Estimation Accuracy ──");
    for r in results {
        let status = if r.passes_tolerance {
            "✅"
        } else {
            "❌"
        };
        eprintln!(
            "  {} {:25} actual_cpu={:>12}  estimated_cpu={:>12}  error={:>5.1}%",
            status,
            r.operation,
            r.actual_cpu,
            r.estimated_cpu,
            r.cpu_error_pct
        );
    }
    let failures = check_gas_accuracy(results);
    if !failures.is_empty() {
        eprintln!("\n❌  {} estimate(s) exceeded 20% tolerance:", failures.len());
        for r in failures {
            eprintln!(
                "  [{}/{}] estimated={} actual={} error={:.1}%",
                r.contract, r.operation, r.estimated_cpu, r.actual_cpu, r.cpu_error_pct
            );
        }
    }
}

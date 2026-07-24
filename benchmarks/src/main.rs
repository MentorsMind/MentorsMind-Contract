/// MentorsMind Soroban Benchmark Harness
///
/// Uses soroban-sdk testutils to measure CPU instruction count and storage I/O
/// per contract entry point. Results are compared against `baselines.json`
/// and the process exits with code 1 if any metric regresses more than 10%.
///
/// Output:
///   - benchmarks/results/report.json   — full machine-readable results
///   - benchmarks/results/report.html   — human-readable per-function table
///   - Exit 0 on pass, 1 on regression
extern crate std;

mod harness;
mod report;
mod suites;

use harness::BenchResult;
use std::path::Path;

fn main() {
    let results = run_all_suites();
    report::write_json(&results);
    report::write_html(&results);

    let baseline_path = Path::new("benchmarks/baselines.json");
    if baseline_path.exists() {
        let regressions = harness::check_regressions(&results, baseline_path);
        if !regressions.is_empty() {
            eprintln!("\n❌  REGRESSIONS DETECTED ({} total):", regressions.len());
            for r in &regressions {
                eprintln!(
                    "  [{}] {} — {} exceeded baseline by {:.1}% (baseline={}, measured={})",
                    r.contract, r.entry_point, r.metric, r.pct_change, r.baseline, r.measured
                );
            }
            std::process::exit(1);
        }
        println!("\n✅  All metrics within 10% of baseline.");
    } else {
        println!(
            "\n⚠️   No baselines.json found — writing current results as new baseline."
        );
        report::write_baseline(&results, baseline_path);
    }
}

fn run_all_suites() -> Vec<BenchResult> {
    let mut all: Vec<BenchResult> = Vec::new();
    all.extend(suites::escrow::run());
    all.extend(suites::staking::run());
    all.extend(suites::governance::run());
    all.extend(suites::timelock::run());
    all
}

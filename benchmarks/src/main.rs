#![allow(dead_code, unused_imports)]
/// MentorsMind Soroban Benchmark Harness
///
/// Uses soroban-sdk testutils to measure CPU instruction count and storage I/O
/// per contract entry point. Results are compared against `baselines.json`
/// and the process exits with code 1 if any metric regresses more than 10%.
///
/// Output:
///   - benchmarks/results/report.json   — full machine-readable results
///   - benchmarks/results/report.html   — human-readable per-function table with trends
///   - benchmarks/results/gas_accuracy.json — gas estimation accuracy report
///   - benchmarks/history/<date>_<sha>.json — persisted historical run record
///   - Exit 0 on pass, 1 on regression or estimation failure
extern crate std;

mod harness;
mod history;
mod report;
mod suites;

use harness::BenchResult;
use std::path::Path;

fn main() {
    let results = run_all_suites();
    report::write_json(&results);

    // Load history before saving this run so the HTML can show trends.
    let history = history::load_all();
    report::write_html(&results, &history);

    // Persist this run to the history directory.
    match history::save(&results) {
        Ok(path) => println!("📚  History record written to {}", path),
        Err(e) => eprintln!("⚠️   Could not write history record: {}", e),
    }

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
            // Emit GitHub Actions annotations for each regression.
            emit_annotations(&regressions);
            // Write job summary if running in CI.
            write_job_summary(&results, &regressions);
            std::process::exit(1);
        }
        println!("\n✅  All metrics within 10% of baseline.");
        write_job_summary(&results, &[]);
    } else {
        println!(
            "\n⚠️   No baselines.json found — writing current results as new baseline."
        );
        report::write_baseline(&results, baseline_path);
        write_job_summary(&results, &[]);
    }
}

fn run_all_suites() -> Vec<BenchResult> {
    let mut all: Vec<BenchResult> = Vec::new();
    all.extend(suites::escrow::run());
    all.extend(suites::staking::run());
    all.extend(suites::governance::run());
    all.extend(suites::timelock::run());
    all.extend(suites::upgrade_registry::run());
    all.extend(suites::dispute_evidence::run());
    all.extend(suites::gas_estimation::run());
    all
}

/// Emit GitHub Actions `error` workflow commands so each regression surfaces
/// as an annotation in the PR diff view.
fn emit_annotations(regressions: &[harness::Regression]) {
    for r in regressions {
        // GitHub Actions annotation syntax:
        //   ::error title=<title>::<message>
        println!(
            "::error title=Performance Regression [{}/{}]::Metric `{}` exceeded 10% baseline — baseline={}, measured={}, delta=+{:.1}%",
            r.contract, r.entry_point, r.metric, r.baseline, r.measured, r.pct_change
        );
    }
}

/// Write a Markdown job summary to `$GITHUB_STEP_SUMMARY` when running in CI.
fn write_job_summary(results: &[BenchResult], regressions: &[harness::Regression]) {
    use std::env;
    use std::fs::OpenOptions;
    use std::io::Write;

    let Ok(summary_path) = env::var("GITHUB_STEP_SUMMARY") else {
        return;
    };

    let mut f = match OpenOptions::new().append(true).open(&summary_path) {
        Ok(f) => f,
        Err(_) => return,
    };

    let status = if regressions.is_empty() {
        "✅ All metrics within baseline"
    } else {
        "❌ Performance regressions detected"
    };

    let _ = writeln!(f, "## Soroban Benchmark Results\n");
    let _ = writeln!(f, "**Status:** {}\n", status);

    if !regressions.is_empty() {
        let _ = writeln!(f, "### Regressions\n");
        let _ = writeln!(f, "| Contract | Entry Point | Metric | Baseline | Measured | Delta |");
        let _ = writeln!(f, "|----------|-------------|--------|----------|----------|-------|");
        for r in regressions {
            let _ = writeln!(
                f,
                "| {} | `{}` | {} | {} | {} | **+{:.1}%** |",
                r.contract, r.entry_point, r.metric, r.baseline, r.measured, r.pct_change
            );
        }
        let _ = writeln!(f);
    }

    let _ = writeln!(f, "### All Results\n");
    let _ = writeln!(f, "| Contract | Entry Point | CPU Instructions | Memory (bytes) | WASM Size |");
    let _ = writeln!(f, "|----------|-------------|-----------------|----------------|-----------|");
    let mut prev = "";
    for r in results {
        let contract = if r.contract.as_str() != prev {
            prev = r.contract.as_str();
            r.contract.as_str()
        } else {
            ""
        };
        let wasm = if r.wasm_bytes == 0 {
            "N/A".into()
        } else if r.wasm_bytes > 65536 {
            format!("⚠️ {} KB", r.wasm_bytes / 1024)
        } else {
            format!("{} KB", r.wasm_bytes / 1024)
        };
        let _ = writeln!(
            f,
            "| {} | `{}` | {} | {} | {} |",
            contract,
            r.entry_point,
            r.cpu_instructions,
            r.mem_bytes,
            wasm
        );
    }
}

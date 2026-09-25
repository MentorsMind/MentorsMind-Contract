/// storage-validator — MentorsMind storage migration validation CLI.
///
/// # Commands
///
/// ```
/// # Snapshot current workspace storage schemas
/// storage-validator snapshot [--version <tag>] [--workspace <path>]
///
/// # Compare two snapshots and generate a migration report
/// storage-validator diff --from <version> --to <version>
///
/// # Snapshot current schemas and diff against a baseline version (CI mode)
/// storage-validator check --baseline <version> [--workspace <path>]
///
/// # List all recorded snapshot versions
/// storage-validator list
/// ```
///
/// Exit codes:
///   0 — no breaking changes (or snapshot-only command)
///   1 — breaking changes detected
///   2 — usage / IO error

use std::path::PathBuf;
use std::process;

mod differ;
mod parser;
mod reporter;
mod scanner;
mod types;

fn main() {
    let args: Vec<String> = std::env::args().collect();

    if args.len() < 2 {
        print_usage();
        process::exit(2);
    }

    match args[1].as_str() {
        "snapshot" => cmd_snapshot(&args[2..]),
        "diff" => cmd_diff(&args[2..]),
        "check" => cmd_check(&args[2..]),
        "list" => cmd_list(),
        "--help" | "-h" | "help" => {
            print_usage();
        }
        other => {
            eprintln!("Unknown command: {other}");
            print_usage();
            process::exit(2);
        }
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

/// Snapshot the current workspace schemas.
fn cmd_snapshot(args: &[String]) {
    let version = flag(args, "--version").unwrap_or_else(|| {
        // Default to current git SHA short form or "current".
        std::env::var("GITHUB_SHA")
            .map(|s| s[..s.len().min(8)].to_string())
            .unwrap_or_else(|_| "current".to_string())
    });
    let workspace = flag(args, "--workspace")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));

    println!("🔍  Scanning workspace at {} …", workspace.display());
    let snapshot = scanner::scan_workspace(&workspace, &version);
    println!(
        "    Found {} contracts with contracttype definitions.",
        snapshot.contracts.len()
    );
    for c in &snapshot.contracts {
        println!(
            "    ✓  {} — {} enums, {} structs",
            c.contract,
            c.enums.len(),
            c.structs.len()
        );
    }
    reporter::write_snapshot(&snapshot);
    println!("✅  Snapshot `{version}` saved.");
}

/// Diff two stored snapshots.
fn cmd_diff(args: &[String]) {
    let from = flag(args, "--from").unwrap_or_else(|| {
        eprintln!("--from <version> is required");
        process::exit(2);
    });
    let to = flag(args, "--to").unwrap_or_else(|| {
        eprintln!("--to <version> is required");
        process::exit(2);
    });

    let baseline = scanner::load_snapshot(&from).unwrap_or_else(|| {
        eprintln!("Snapshot `{from}` not found in storage-snapshots/");
        process::exit(2);
    });
    let current = scanner::load_snapshot(&to).unwrap_or_else(|| {
        eprintln!("Snapshot `{to}` not found in storage-snapshots/");
        process::exit(2);
    });

    let report = differ::diff(&baseline, &current);
    reporter::print_summary(&report);
    reporter::write_report_json(&report);
    reporter::write_report_markdown(&report);

    if !report.is_safe {
        process::exit(1);
    }
}

/// Snapshot current workspace and immediately diff against a baseline (CI mode).
fn cmd_check(args: &[String]) {
    let baseline_version = flag(args, "--baseline").unwrap_or_else(|| {
        eprintln!("--baseline <version> is required");
        process::exit(2);
    });
    let workspace = flag(args, "--workspace")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let current_version = flag(args, "--version").unwrap_or_else(|| {
        std::env::var("GITHUB_SHA")
            .map(|s| s[..s.len().min(8)].to_string())
            .unwrap_or_else(|_| "current".to_string())
    });

    let baseline = scanner::load_snapshot(&baseline_version).unwrap_or_else(|| {
        eprintln!(
            "Baseline snapshot `{baseline_version}` not found. \
             Run `storage-validator snapshot --version {baseline_version}` to create it."
        );
        process::exit(2);
    });

    println!("🔍  Scanning current workspace …");
    let current = scanner::scan_workspace(&workspace, &current_version);
    println!(
        "    Found {} contracts.",
        current.contracts.len()
    );

    // Persist the current snapshot so it's available for future diffs.
    reporter::write_snapshot(&current);

    let report = differ::diff(&baseline, &current);
    reporter::print_summary(&report);
    reporter::write_report_json(&report);
    reporter::write_report_markdown(&report);

    // Emit GitHub Actions annotations for each breaking change.
    emit_annotations(&report);
    // Write job summary if running in CI.
    write_job_summary(&report);

    if !report.is_safe {
        process::exit(1);
    }
}

/// List all available snapshot versions.
fn cmd_list() {
    let versions = scanner::list_snapshots();
    if versions.is_empty() {
        println!("No snapshots found in storage-snapshots/");
    } else {
        println!("Available snapshots:");
        for v in versions {
            println!("  - {}", v);
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn flag(args: &[String], name: &str) -> Option<String> {
    for (i, arg) in args.iter().enumerate() {
        if arg == name {
            return args.get(i + 1).cloned();
        }
    }
    None
}

fn emit_annotations(report: &crate::types::MigrationReport) {
    for d in &report.diffs {
        if d.severity == crate::types::Severity::Breaking {
            println!(
                "::error title=Breaking Storage Change [{}/{}]::{} — {}",
                d.contract, d.type_name, format!("{:?}", d.kind), d.description
            );
        } else if d.severity == crate::types::Severity::Warning {
            println!(
                "::warning title=Storage Schema Warning [{}/{}]::{} — {}",
                d.contract, d.type_name, format!("{:?}", d.kind), d.description
            );
        }
    }
}

fn write_job_summary(report: &crate::types::MigrationReport) {
    use std::io::Write;

    let Ok(path) = std::env::var("GITHUB_STEP_SUMMARY") else { return };
    let mut f = match std::fs::OpenOptions::new().append(true).open(&path) {
        Ok(f) => f,
        Err(_) => return,
    };
    let md = reporter::render_markdown(report);
    let _ = f.write_all(md.as_bytes());
}

fn print_usage() {
    println!(
        r#"storage-validator — MentorsMind storage migration validation tool

USAGE:
  storage-validator <COMMAND> [OPTIONS]

COMMANDS:
  snapshot    Scan workspace and write a schema snapshot
  diff        Compare two stored snapshots
  check       Snapshot + diff against a baseline in one step (CI mode)
  list        List all recorded snapshot versions

OPTIONS (snapshot / check):
  --version <tag>        Snapshot version label (default: git SHA short)
  --workspace <path>     Workspace root (default: current directory)

OPTIONS (diff):
  --from <version>       Baseline snapshot version
  --to <version>         New snapshot version

OPTIONS (check):
  --baseline <version>   Baseline snapshot to compare against (required)
  --version <tag>        Label for the new snapshot
  --workspace <path>     Workspace root

EXIT CODES:
  0  No breaking changes
  1  Breaking changes detected
  2  Usage or IO error
"#
    );
}

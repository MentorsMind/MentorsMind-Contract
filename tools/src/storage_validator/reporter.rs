/// Report renderer — produces both JSON and a Markdown migration report.

use crate::types::{MigrationReport, SchemaSnapshot, Severity};
use std::fs;

const REPORTS_DIR: &str = "storage-snapshots";

/// Write the migration report as JSON to `storage-snapshots/migration-report.json`.
pub fn write_report_json(report: &MigrationReport) {
    fs::create_dir_all(REPORTS_DIR).expect("failed to create storage-snapshots dir");
    let path = format!("{}/migration-report.json", REPORTS_DIR);
    let json = serde_json::to_string_pretty(report).expect("failed to serialize report");
    fs::write(&path, json).expect("failed to write migration-report.json");
    println!("📄  Migration report written to {}", path);
}

/// Write the snapshot as JSON to `storage-snapshots/<version>/schema.json`.
pub fn write_snapshot(snapshot: &SchemaSnapshot) {
    let dir = format!("{}/{}", REPORTS_DIR, snapshot.version);
    fs::create_dir_all(&dir).expect("failed to create snapshot dir");
    let path = format!("{}/schema.json", dir);
    let json = serde_json::to_string_pretty(snapshot).expect("failed to serialize snapshot");
    fs::write(&path, json).expect("failed to write schema.json");
    println!("📐  Schema snapshot written to {}", path);
}

/// Render the migration report as Markdown for CI/PR comments.
pub fn render_markdown(report: &MigrationReport) -> String {
    let verdict = if report.is_safe {
        "✅ Safe to upgrade — no breaking storage changes detected"
    } else {
        "❌ Breaking changes detected — migration required before upgrade"
    };

    let mut md = format!(
        "## Storage Migration Validation Report\n\n\
         **Status:** {verdict}\n\n\
         **Comparing:** `{from}` → `{to}`\n\n\
         | Severity | Count |\n\
         |----------|-------|\n\
         | 🔴 Breaking | {breaking} |\n\
         | 🟡 Warning | {warning} |\n\
         | 🟢 Compatible | {compatible} |\n\n",
        from = report.from_version,
        to = report.to_version,
        breaking = report.breaking_count,
        warning = report.warning_count,
        compatible = report.compatible_count,
    );

    let breaking: Vec<_> = report.diffs.iter().filter(|d| d.severity == Severity::Breaking).collect();
    let warnings: Vec<_> = report.diffs.iter().filter(|d| d.severity == Severity::Warning).collect();
    let compatible: Vec<_> = report.diffs.iter().filter(|d| d.severity == Severity::Compatible).collect();

    if !breaking.is_empty() {
        md.push_str("### 🔴 Breaking Changes\n\n");
        md.push_str("These changes will corrupt existing ledger state and **must** be resolved before upgrading.\n\n");
        for d in &breaking {
            md.push_str(&format!(
                "- **[{}]** `{}` — `{}`: {}\n",
                d.contract, d.type_name, format!("{:?}", d.kind), d.description
            ));
        }
        md.push('\n');
    }

    if !warnings.is_empty() {
        md.push_str("### 🟡 Warnings\n\n");
        md.push_str("These changes are technically compatible but require careful migration planning.\n\n");
        for d in &warnings {
            md.push_str(&format!(
                "- **[{}]** `{}` — `{}`: {}\n",
                d.contract, d.type_name, format!("{:?}", d.kind), d.description
            ));
        }
        md.push('\n');
    }

    if !compatible.is_empty() {
        md.push_str("### 🟢 Compatible Changes\n\n");
        for d in &compatible {
            md.push_str(&format!(
                "- **[{}]** `{}`: {}\n",
                d.contract, d.type_name, d.description
            ));
        }
        md.push('\n');
    }

    if report.diffs.is_empty() {
        md.push_str("_No storage schema changes detected between these versions._\n");
    }

    md
}

/// Write the Markdown report to `storage-snapshots/migration-report.md`.
pub fn write_report_markdown(report: &MigrationReport) {
    fs::create_dir_all(REPORTS_DIR).expect("failed to create storage-snapshots dir");
    let path = format!("{}/migration-report.md", REPORTS_DIR);
    let md = render_markdown(report);
    fs::write(&path, &md).expect("failed to write migration-report.md");
    println!("📝  Markdown report written to {}", path);
}

/// Print a compact summary to stdout.
pub fn print_summary(report: &MigrationReport) {
    println!("\n─────────────────────────────────────────");
    println!("  Storage Migration Validation Summary");
    println!("  {} → {}", report.from_version, report.to_version);
    println!("─────────────────────────────────────────");
    println!("  🔴 Breaking : {}", report.breaking_count);
    println!("  🟡 Warnings : {}", report.warning_count);
    println!("  🟢 Safe     : {}", report.compatible_count);
    println!("─────────────────────────────────────────");

    for d in &report.diffs {
        let icon = match d.severity {
            Severity::Breaking => "🔴",
            Severity::Warning => "🟡",
            Severity::Compatible => "🟢",
        };
        println!("  {} [{}] {} — {}", icon, d.contract, d.type_name, d.description);
    }

    if report.is_safe {
        println!("\n✅  Safe to upgrade.");
    } else {
        println!("\n❌  Migration required before upgrade.");
    }
}

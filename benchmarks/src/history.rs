/// Historical benchmark result storage.
///
/// Each CI run appends a timestamped snapshot to `benchmarks/history/`.
/// Files are named `YYYY-MM-DD_<short-sha>.json` so they sort chronologically
/// and are uniquely identified by commit.
///
/// The history directory is committed to the repository so trends persist
/// across CI runs without relying on artifact retention windows.
extern crate std;

use crate::harness::BenchResult;
use serde::{Deserialize, Serialize};
use std::{env, fs, path::Path};

const HISTORY_DIR: &str = "benchmarks/history";

/// A single historical run record.
#[derive(Debug, Serialize, Deserialize)]
pub struct HistoryRecord {
    /// ISO-8601 date string (YYYY-MM-DD), sourced from `BENCH_DATE` env var
    /// or falls back to a placeholder so runs are never silently dropped.
    pub date: String,
    /// Git commit SHA, sourced from `GITHUB_SHA` env var.
    pub sha: String,
    /// Short name for display (branch or tag), sourced from `GITHUB_REF_NAME`.
    pub ref_name: String,
    /// All benchmark results for this run.
    pub results: Vec<BenchResult>,
}

/// Persist the current run as a new history file.
/// Returns the path written, or an error string.
pub fn save(results: &[BenchResult]) -> Result<String, String> {
    fs::create_dir_all(HISTORY_DIR)
        .map_err(|e| format!("failed to create history dir: {e}"))?;

    let date = env::var("BENCH_DATE").unwrap_or_else(|_| "unknown-date".into());
    let sha = env::var("GITHUB_SHA").unwrap_or_else(|_| "local".into());
    let short_sha = &sha[..sha.len().min(8)];
    let ref_name = env::var("GITHUB_REF_NAME").unwrap_or_else(|_| "local".into());

    let record = HistoryRecord {
        date: date.clone(),
        sha: sha.clone(),
        ref_name,
        results: results.to_vec(),
    };

    let filename = format!("{}/{}_{}.json", HISTORY_DIR, date, short_sha);
    let json = serde_json::to_string_pretty(&record)
        .map_err(|e| format!("failed to serialize history record: {e}"))?;
    fs::write(&filename, json)
        .map_err(|e| format!("failed to write history file {filename}: {e}"))?;

    Ok(filename)
}

/// Load all history records, sorted chronologically by filename.
pub fn load_all() -> Vec<HistoryRecord> {
    let dir = Path::new(HISTORY_DIR);
    if !dir.exists() {
        return Vec::new();
    }

    let mut entries: Vec<_> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| {
                    e.path()
                        .extension()
                        .map(|x| x == "json")
                        .unwrap_or(false)
                })
                .collect()
        })
        .unwrap_or_default();

    // Sort by filename so dates order naturally.
    entries.sort_by_key(|e| e.file_name());

    entries
        .into_iter()
        .filter_map(|e| {
            let data = fs::read_to_string(e.path()).ok()?;
            serde_json::from_str(&data).ok()
        })
        .collect()
}

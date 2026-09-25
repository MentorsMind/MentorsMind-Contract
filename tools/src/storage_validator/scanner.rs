/// Workspace scanner — discovers all contract `lib.rs` files to snapshot.
///
/// Walks the workspace looking for directories that contain a `Cargo.toml`
/// with `[lib]` or `[[bin]]` sections, then parses their primary source file.
/// Contract name is derived from the `name` field in `[package]`.

use std::path::{Path, PathBuf};
use std::{env, fs};

use crate::parser::parse_file;
use crate::types::{ContractSchema, SchemaSnapshot};

/// Directories to skip during workspace scanning.
const SKIP_DIRS: &[&str] = &[
    "target",
    ".git",
    ".github",
    "node_modules",
    "benchmarks", // benchmark harness — not contract storage
    "tools",      // this crate — not contract storage
    "tests",      // integration tests
];

/// Scan `workspace_root` recursively for contracts and build a snapshot.
pub fn scan_workspace(workspace_root: &Path, version: &str) -> SchemaSnapshot {
    let contracts = find_contracts(workspace_root);

    let sha = env::var("GITHUB_SHA").unwrap_or_else(|_| "local".into());
    let ref_name = env::var("GITHUB_REF_NAME").unwrap_or_else(|_| "local".into());
    let captured_at = env::var("BENCH_DATE").unwrap_or_else(|_| "unknown-date".into());

    SchemaSnapshot {
        version: version.to_string(),
        captured_at,
        ref_name,
        sha,
        contracts,
    }
}

fn find_contracts(root: &Path) -> Vec<ContractSchema> {
    let mut schemas = Vec::new();
    let mut dirs_to_visit = vec![root.to_path_buf()];

    while let Some(dir) = dirs_to_visit.pop() {
        let cargo_toml = dir.join("Cargo.toml");

        if cargo_toml.exists() {
            let dir_name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();

            if SKIP_DIRS.contains(&dir_name.as_str()) {
                continue;
            }

            if let Some(schema) = try_parse_contract(&dir, &cargo_toml) {
                schemas.push(schema);
                // Don't recurse into crate subdirectories — only the crate root matters.
                continue;
            }
        }

        // Recurse into subdirectories.
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    let name = path
                        .file_name()
                        .map(|n| n.to_string_lossy().into_owned())
                        .unwrap_or_default();
                    if !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                        dirs_to_visit.push(path);
                    }
                }
            }
        }
    }

    // Sort for stable output.
    schemas.sort_by(|a, b| a.contract.cmp(&b.contract));
    schemas
}

/// Attempt to parse a contract crate rooted at `dir`.
/// Returns `None` if this isn't a contract crate (no `src/lib.rs` or `src/main.rs`).
fn try_parse_contract(dir: &Path, cargo_toml: &PathBuf) -> Option<ContractSchema> {
    let contract_name = extract_package_name(cargo_toml)?;

    // Primary source: prefer src/lib.rs (contracts), fall back to src/main.rs.
    let lib_rs = dir.join("src").join("lib.rs");
    let main_rs = dir.join("src").join("main.rs");

    let source_path = if lib_rs.exists() {
        lib_rs
    } else if main_rs.exists() {
        main_rs
    } else {
        return None;
    };

    match parse_file(&contract_name, &source_path) {
        Ok(schema) => {
            // Only include crates that actually have contracttype definitions.
            if schema.enums.is_empty() && schema.structs.is_empty() {
                None
            } else {
                Some(schema)
            }
        }
        Err(e) => {
            eprintln!(
                "  ⚠️  Failed to parse {}: {}",
                source_path.display(),
                e
            );
            None
        }
    }
}

/// Extract the `name` field from `[package]` in a `Cargo.toml`.
fn extract_package_name(cargo_toml: &Path) -> Option<String> {
    let content = fs::read_to_string(cargo_toml).ok()?;
    let mut in_package = false;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed == "[package]" {
            in_package = true;
            continue;
        }
        if trimmed.starts_with('[') {
            in_package = false;
        }
        if in_package && trimmed.starts_with("name") {
            // `name = "mentorminds-escrow"`
            if let Some(val) = trimmed.splitn(2, '=').nth(1) {
                return Some(val.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Load a previously written snapshot from `storage-snapshots/<version>/schema.json`.
pub fn load_snapshot(version: &str) -> Option<SchemaSnapshot> {
    let path = PathBuf::from(format!("storage-snapshots/{}/schema.json", version));
    if !path.exists() {
        return None;
    }
    let data = fs::read_to_string(&path)
        .map_err(|e| eprintln!("Failed to read snapshot {}: {}", path.display(), e))
        .ok()?;
    serde_json::from_str(&data)
        .map_err(|e| eprintln!("Failed to parse snapshot {}: {}", path.display(), e))
        .ok()
}

/// List all available snapshot versions (directory names under `storage-snapshots/`).
pub fn list_snapshots() -> Vec<String> {
    let dir = Path::new("storage-snapshots");
    if !dir.exists() {
        return Vec::new();
    }
    let mut versions: Vec<String> = fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(|e| e.ok())
                .filter(|e| e.path().is_dir())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect()
        })
        .unwrap_or_default();
    versions.sort();
    versions
}

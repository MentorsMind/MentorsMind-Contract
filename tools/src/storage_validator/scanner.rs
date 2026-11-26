/// Workspace scanner — discovers all contract `lib.rs` files to snapshot.
///
/// Walks the workspace looking for directories that contain a `Cargo.toml`
/// with `[lib]` or `[[bin]]` sections, then parses their primary source file.
/// Contract name is derived from the `name` field in `[package]`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::{env, fs};

use crate::parser::{parse_file, parse_source};
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
        Ok(mut schema) => {
            // Only include crates that actually have contracttype definitions.
            if schema.enums.is_empty() && schema.structs.is_empty() {
                return None;
            }
            resolve_nested_types(dir, &mut schema);
            Some(schema)
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

/// Recursively resolve `#[contracttype]` types referenced by struct fields
/// and enum payloads, including types declared in other source files and
/// types nested more than one level deep.
fn resolve_nested_types(dir: &Path, schema: &mut ContractSchema) {
    let sources = rust_sources(&dir.join("src"));

    // Each pass discovers at least one new type; iterate until no new type is
    // found (bounded to guard against pathological cyclic references).
    for _ in 0..16 {
        let mut referenced: HashSet<String> = HashSet::new();
        for s in &schema.structs {
            for field in &s.fields {
                collect_referenced_types(&field.ty, &mut referenced);
            }
        }
        for e in &schema.enums {
            for variant in &e.variants {
                for field in &variant.fields {
                    collect_referenced_types(field, &mut referenced);
                }
            }
        }

        let known_structs: HashSet<String> =
            schema.structs.iter().map(|s| s.name.clone()).collect();
        let known_enums: HashSet<String> = schema.enums.iter().map(|e| e.name.clone()).collect();
        let mut added = false;

        for path in &sources {
            let Ok(source) = fs::read_to_string(path) else { continue };
            let (enums, structs) = parse_source(&source);
            for e in enums {
                if referenced.contains(&e.name) && !known_enums.contains(&e.name) && !known_structs.contains(&e.name) {
                    schema.enums.push(e);
                    added = true;
                }
            }
            for s in structs {
                if referenced.contains(&s.name) && !known_structs.contains(&s.name) && !known_enums.contains(&s.name) {
                    schema.structs.push(s);
                    added = true;
                }
            }
        }

        if !added {
            break;
        }
    }

    schema.enums.sort_by(|a, b| a.name.cmp(&b.name));
    schema.structs.sort_by(|a, b| a.name.cmp(&b.name));
}

/// Collect uppercase type identifiers from a field type such as
/// `Vec<NestedRecord>` or `Option<Inner>`.
fn collect_referenced_types(ty: &str, out: &mut HashSet<String>) {
    let mut start: Option<usize> = None;
    for (idx, ch) in ty.char_indices().chain(std::iter::once((ty.len(), ' '))) {
        if ch.is_alphabetic() || ch == '_' {
            if start.is_none() {
                start = Some(idx);
            }
        } else if let Some(begin) = start.take() {
            let word = &ty[begin..idx];
            if word.chars().next().is_some_and(|c| c.is_uppercase()) {
                out.insert(word.to_string());
            }
        }
    }
}

/// All `.rs` files under a crate's `src` directory.
fn rust_sources(src: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![src.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if let Ok(entries) = fs::read_dir(&dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                } else if path.extension().and_then(|e| e.to_str()) == Some("rs") {
                    files.push(path);
                }
            }
        }
    }
    files
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::differ::diff;
    use crate::types::{DiffKind, SchemaSnapshot};

    fn fixture(name: &str, nested_field_type: &str) -> PathBuf {
        let root = env::temp_dir().join(format!("mm-scanner-{}-{}", std::process::id(), name));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("src")).unwrap();
        fs::write(
            root.join("Cargo.toml"),
            "[package]\nname = \"test-contract\"\nversion = \"0.1.0\"\n",
        )
        .unwrap();
        fs::write(
            root.join("src/lib.rs"),
            "#[contracttype]\npub struct Outer {\n    pub inner: Inner,\n}\n",
        )
        .unwrap();
        fs::write(
            root.join("src/nested.rs"),
            format!(
                "#[contracttype]\npub struct Inner {{\n    pub value: InnerValue,\n}}\n\
                 #[contracttype]\npub struct InnerValue {{\n    pub amount: {nested_field_type},\n}}\n"
            ),
        )
        .unwrap();
        root
    }

    fn snapshot_for(root: &Path) -> SchemaSnapshot {
        let cargo = root.join("Cargo.toml");
        let schema = try_parse_contract(root, &cargo).unwrap();
        SchemaSnapshot {
            version: "v1".into(),
            captured_at: "now".into(),
            ref_name: "main".into(),
            sha: "sha".into(),
            contracts: vec![schema],
        }
    }

    #[test]
    fn resolves_two_levels_of_nested_types() {
        let root = fixture("nested", "u64");
        let mut snapshot = snapshot_for(&root);
        let schema = snapshot.contracts.remove(0);
        let names: Vec<_> = schema.structs.iter().map(|s| s.name.as_str()).collect();
        assert!(names.contains(&"Outer"));
        assert!(names.contains(&"Inner"));
        assert!(names.contains(&"InnerValue"));
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn nested_type_changes_are_reported_by_differ() {
        let old_root = fixture("old", "u64");
        let new_root = fixture("new", "i128");
        let baseline = snapshot_for(&old_root);
        let current = snapshot_for(&new_root);
        let report = diff(&baseline, &current);
        assert!(report.diffs.iter().any(|d| {
            d.kind == DiffKind::FieldTypeChanged && d.description.contains("InnerValue")
        }));
        fs::remove_dir_all(old_root).unwrap();
        fs::remove_dir_all(new_root).unwrap();
    }
}

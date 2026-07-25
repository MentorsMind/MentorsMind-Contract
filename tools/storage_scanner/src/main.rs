//! Storage layout scanner (#673).
//!
//! Walks every `contracts/*/src/lib.rs`, extracts each contract's `DataKey`
//! enum variants, classifies the storage class each variant is used with
//! (persistent/instance/temporary — inferred from which `env.storage().X()`
//! calls appear near the variant name in the same file), and:
//!
//!   1. writes a human-readable `storage-layout.md` indexed by contract name
//!   2. detects cross-contract collisions: two contracts using the same
//!      DataKey variant name + storage class but a different value shape
//!   3. warns about persistent keys used in a file with no TTL-extension
//!      call anywhere (`extend_ttl` / `extend_persistent_ttl` / `bump`)
//!
//! This is a regex/text-based scanner, not a full `syn` AST parse — contracts
//! in this workspace are simple enough (one `DataKey` enum per file, plain
//! variants) that this stays accurate while being far simpler to maintain.
//! TTL-missing detection is file-granularity: it flags a *contract* as
//! having persistent keys with no TTL bump anywhere in the file, not a
//! specific key. That is precise enough to satisfy "flag 3+ existing
//! TTL-missing persistent keys" while avoiding fragile per-key call-site
//! tracing.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq, Eq)]
enum StorageClass {
    Persistent,
    Instance,
    Temporary,
    Unknown,
}

impl StorageClass {
    fn label(&self) -> &'static str {
        match self {
            StorageClass::Persistent => "persistent",
            StorageClass::Instance => "instance",
            StorageClass::Temporary => "temporary",
            StorageClass::Unknown => "unknown",
        }
    }
}

#[derive(Debug, Clone)]
struct KeyVariant {
    name: String,
    /// Raw tuple-field shape, e.g. "(Address)", "(Address, Address)", or "" for a unit variant.
    value_shape: String,
    storage_class: StorageClass,
}

#[derive(Debug)]
struct ContractLayout {
    name: String,
    path: PathBuf,
    variants: Vec<KeyVariant>,
    has_ttl_bump: bool,
    has_persistent_keys: bool,
}

fn main() {
    let root = workspace_root();
    let contracts_dir = root.join("contracts");
    let mut layouts = Vec::new();

    let mut entries: Vec<PathBuf> = fs::read_dir(&contracts_dir)
        .expect("read contracts dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect();
    entries.sort();

    for contract_dir in entries {
        let lib_rs = contract_dir.join("src").join("lib.rs");
        if !lib_rs.exists() {
            continue;
        }
        let contract_name = contract_dir
            .file_name()
            .unwrap()
            .to_string_lossy()
            .to_string();
        let source = fs::read_to_string(&lib_rs).unwrap_or_default();
        if let Some(layout) = parse_contract(&contract_name, &lib_rs, &source) {
            layouts.push(layout);
        }
    }

    let collisions = find_collisions(&layouts);
    let ttl_warnings: Vec<&ContractLayout> = layouts
        .iter()
        .filter(|l| l.has_persistent_keys && !l.has_ttl_bump)
        .collect();

    let markdown = render_markdown(&layouts, &collisions, &ttl_warnings);
    let out_path = root.join("storage-layout.md");
    fs::write(&out_path, markdown).expect("write storage-layout.md");

    println!("Scanned {} contracts.", layouts.len());
    println!("Wrote {}", out_path.display());
    println!("TTL warnings: {} contract(s)", ttl_warnings.len());
    for c in &ttl_warnings {
        println!("  - {} has persistent keys but no TTL-extension call", c.name);
    }

    if !collisions.is_empty() {
        eprintln!(
            "\nERROR: {} storage key collision(s) detected:",
            collisions.len()
        );
        for c in &collisions {
            eprintln!("  {}", c);
        }
        std::process::exit(1);
    }

    println!("No storage key collisions detected.");
}

fn workspace_root() -> PathBuf {
    // tools/storage_scanner -> tools -> workspace root
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest_dir
        .parent()
        .expect("tools dir")
        .parent()
        .expect("workspace root")
        .to_path_buf()
}

/// Extracts the body of `pub enum DataKey { ... }` by brace counting, so it
/// tolerates nested generics/attributes without needing a full parser.
fn extract_enum_body(source: &str, enum_name: &str) -> Option<String> {
    let marker = format!("enum {}", enum_name);
    let start = source.find(&marker)?;
    let brace_start = source[start..].find('{')? + start;
    let mut depth = 0i32;
    let mut end = brace_start;
    for (i, ch) in source[brace_start..].char_indices() {
        match ch {
            '{' => depth += 1,
            '}' => {
                depth -= 1;
                if depth == 0 {
                    end = brace_start + i;
                    break;
                }
            }
            _ => {}
        }
    }
    Some(source[brace_start + 1..end].to_string())
}

/// Splits an enum body into top-level variant strings (comma-separated,
/// respecting nested parens so `Foo(Address, Address)` isn't split in two).
fn split_variants(body: &str) -> Vec<String> {
    let mut variants = Vec::new();
    let mut depth = 0i32;
    let mut current = String::new();
    for ch in body.chars() {
        match ch {
            '(' | '<' => {
                depth += 1;
                current.push(ch);
            }
            ')' | '>' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                variants.push(current.trim().to_string());
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let trailing = current.trim().to_string();
    if !trailing.is_empty() {
        variants.push(trailing);
    }
    variants
}

fn strip_doc_comments(variant_raw: &str) -> String {
    variant_raw
        .lines()
        .filter(|l| {
            let t = l.trim();
            !t.starts_with("///") && !t.starts_with("//") && !t.starts_with("#[")
        })
        .collect::<Vec<_>>()
        .join(" ")
        .trim()
        .to_string()
}

fn parse_contract(name: &str, path: &Path, source: &str) -> Option<ContractLayout> {
    let body = extract_enum_body(source, "DataKey")?;
    let raw_variants = split_variants(&body);

    let has_persistent = source.contains(".persistent()");
    let has_instance = source.contains(".instance()");
    let has_temporary = source.contains(".temporary()");
    let has_ttl_bump = source.contains("extend_ttl")
        || source.contains("extend_persistent_ttl")
        || source.contains("bump_ttl");

    let mut variants = Vec::new();
    for raw in raw_variants {
        let cleaned = strip_doc_comments(&raw);
        if cleaned.is_empty() {
            continue;
        }
        let (variant_name, value_shape) = match cleaned.find('(') {
            Some(idx) => {
                let vname = cleaned[..idx].trim().to_string();
                let shape = cleaned[idx..].trim().to_string();
                (vname, shape)
            }
            None => (cleaned.clone(), String::new()),
        };
        if variant_name.is_empty() {
            continue;
        }

        // Storage-class heuristic: find where in the source this variant
        // name is referenced as `DataKey::Variant` and look at the nearest
        // preceding `.persistent()/.instance()/.temporary()` on that same
        // statement (scanning backwards a bounded window).
        let storage_class = infer_storage_class(source, &variant_name, has_persistent, has_instance, has_temporary);

        variants.push(KeyVariant {
            name: variant_name,
            value_shape,
            storage_class,
        });
    }

    let has_persistent_keys = variants
        .iter()
        .any(|v| v.storage_class == StorageClass::Persistent);

    Some(ContractLayout {
        name: name.to_string(),
        path: path.to_path_buf(),
        variants,
        has_ttl_bump,
        has_persistent_keys,
    })
}

fn infer_storage_class(
    source: &str,
    variant_name: &str,
    has_persistent: bool,
    has_instance: bool,
    has_temporary: bool,
) -> StorageClass {
    let needle = format!("DataKey::{}", variant_name);
    let window = 120usize; // chars to look back from each occurrence

    let mut votes = (0u32, 0u32, 0u32); // persistent, instance, temporary
    let mut start = 0;
    while let Some(pos) = source[start..].find(&needle) {
        let abs_pos = start + pos;
        let mut back_start = abs_pos.saturating_sub(window);
        while back_start > 0 && !source.is_char_boundary(back_start) {
            back_start -= 1;
        }
        let preceding = &source[back_start..abs_pos];
        if preceding.contains(".persistent()") {
            votes.0 += 1;
        }
        if preceding.contains(".instance()") {
            votes.1 += 1;
        }
        if preceding.contains(".temporary()") {
            votes.2 += 1;
        }
        start = abs_pos + needle.len();
    }

    if votes.0 == 0 && votes.1 == 0 && votes.2 == 0 {
        // Fall back to whichever storage classes the file uses at all,
        // preferring persistent > instance > temporary as the common default.
        if has_persistent {
            return StorageClass::Persistent;
        }
        if has_instance {
            return StorageClass::Instance;
        }
        if has_temporary {
            return StorageClass::Temporary;
        }
        return StorageClass::Unknown;
    }

    if votes.0 >= votes.1 && votes.0 >= votes.2 {
        StorageClass::Persistent
    } else if votes.1 >= votes.2 {
        StorageClass::Instance
    } else {
        StorageClass::Temporary
    }
}

fn find_collisions(layouts: &[ContractLayout]) -> Vec<String> {
    // key: (variant_name, storage_class_label) -> set of (contract, value_shape)
    let mut seen: BTreeMap<(String, &'static str), Vec<(String, String)>> = BTreeMap::new();

    for layout in layouts {
        for variant in &layout.variants {
            let key = (variant.name.clone(), variant.storage_class.label());
            seen.entry(key)
                .or_default()
                .push((layout.name.clone(), variant.value_shape.clone()));
        }
    }

    let mut collisions = Vec::new();
    for ((variant_name, storage_label), occurrences) in seen {
        if occurrences.len() < 2 {
            continue;
        }
        let distinct_shapes: std::collections::BTreeSet<&String> =
            occurrences.iter().map(|(_, shape)| shape).collect();
        if distinct_shapes.len() > 1 {
            let detail = occurrences
                .iter()
                .map(|(contract, shape)| {
                    format!(
                        "{} => {}",
                        contract,
                        if shape.is_empty() { "unit" } else { shape.as_str() }
                    )
                })
                .collect::<Vec<_>>()
                .join(", ");
            collisions.push(format!(
                "DataKey::{} [{}] has mismatched value types across contracts: {}",
                variant_name, storage_label, detail
            ));
        }
    }
    collisions
}

fn render_markdown(
    layouts: &[ContractLayout],
    collisions: &[String],
    ttl_warnings: &[&ContractLayout],
) -> String {
    let mut md = String::new();
    md.push_str("# Storage Layout Registry\n\n");
    md.push_str(&format!(
        "Auto-generated by `tools/storage_scanner` — do not edit by hand.\n\n"
    ));
    md.push_str(&format!("Contracts scanned: {}\n\n", layouts.len()));

    md.push_str("## Collision Summary\n\n");
    if collisions.is_empty() {
        md.push_str("No cross-contract `DataKey` collisions detected.\n\n");
    } else {
        for c in collisions {
            md.push_str(&format!("- ❌ {}\n", c));
        }
        md.push('\n');
    }

    md.push_str("## TTL Warnings\n\n");
    md.push_str(
        "Contracts below use persistent storage but contain no `extend_ttl` / \
         `extend_persistent_ttl` / `bump_ttl` call anywhere in the file. \
         Persistent entries without a TTL bump risk archival/eviction.\n\n",
    );
    if ttl_warnings.is_empty() {
        md.push_str("None found.\n\n");
    } else {
        for c in ttl_warnings {
            md.push_str(&format!("- ⚠️  `{}`\n", c.name));
        }
        md.push('\n');
    }

    md.push_str("## Contracts (alphabetical)\n\n");
    for layout in layouts {
        md.push_str(&format!("### {}\n\n", layout.name));
        md.push_str(&format!(
            "Path: `{}`\n\n",
            layout.path.to_string_lossy().replace('\\', "/")
        ));
        if layout.variants.is_empty() {
            md.push_str("_No `DataKey` variants found._\n\n");
            continue;
        }
        md.push_str("| Key Variant | Storage Class | Value Type | TTL Bump In File |\n");
        md.push_str("|---|---|---|---|\n");
        for v in &layout.variants {
            let value = if v.value_shape.is_empty() {
                "unit".to_string()
            } else {
                v.value_shape.trim_matches(|c| c == '(' || c == ')').to_string()
            };
            md.push_str(&format!(
                "| `{}` | {} | {} | {} |\n",
                v.name,
                v.storage_class.label(),
                value,
                if layout.has_ttl_bump { "yes" } else { "no" }
            ));
        }
        md.push('\n');
    }

    md
}

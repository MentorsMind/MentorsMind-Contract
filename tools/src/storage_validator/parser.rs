/// Source-level parser for `#[contracttype]` definitions.
///
/// Uses line-by-line regex matching — no full AST required. The patterns
/// are intentionally simple because the Soroban macro style is very uniform
/// across this codebase:
///
/// ```rust
/// #[contracttype]
/// #[derive(...)]          // optional
/// pub enum DataKey {
///     Admin,
///     Stake(Address),
///     EpochReward(u64),
/// }
///
/// #[contracttype]
/// pub struct StakeRecord {
///     pub mentor: Address,
///     pub amount: i128,
/// }
/// ```
use std::path::Path;

use crate::types::{
    ContractSchema, EnumSchema, FieldDef, StorageTier, StructSchema, VariantDef,
};

/// Parse all `#[contracttype]` definitions from a Rust source file.
pub fn parse_file(contract_name: &str, path: &Path) -> std::io::Result<ContractSchema> {
    let source = std::fs::read_to_string(path)?;
    let (enums, structs) = extract_types(&source);
    Ok(ContractSchema {
        contract: contract_name.to_string(),
        source_path: path.to_string_lossy().into_owned(),
        enums,
        structs,
    })
}

// ---------------------------------------------------------------------------
// Internal extraction logic
// ---------------------------------------------------------------------------

fn extract_types(source: &str) -> (Vec<EnumSchema>, Vec<StructSchema>) {
    let mut enums = Vec::new();
    let mut structs = Vec::new();

    let lines: Vec<&str> = source.lines().collect();
    let mut i = 0;

    while i < lines.len() {
        let trimmed = lines[i].trim();

        if trimmed == "#[contracttype]" {
            // Skip over optional `#[derive(...)]`, `#[repr(...)]`, blank lines.
            let mut j = i + 1;
            while j < lines.len() {
                let next = lines[j].trim();
                if next.starts_with("#[") || next.is_empty() {
                    j += 1;
                } else {
                    break;
                }
            }

            if j >= lines.len() {
                i += 1;
                continue;
            }

            let decl = lines[j].trim();

            if let Some(name) = parse_enum_decl(decl) {
                // Collect the body of the enum (up to the matching `}`).
                let body_start = j + 1;
                let body_end = find_closing_brace(&lines, body_start);
                let body = &lines[body_start..body_end];
                let schema = parse_enum_body(&name, body);
                enums.push(schema);
                i = body_end + 1;
                continue;
            }

            if let Some(name) = parse_struct_decl(decl) {
                let body_start = j + 1;
                let body_end = find_closing_brace(&lines, body_start);
                let body = &lines[body_start..body_end];
                let schema = parse_struct_body(&name, body);
                structs.push(schema);
                i = body_end + 1;
                continue;
            }
        }

        i += 1;
    }

    (enums, structs)
}

/// Extract name from `pub enum FooBar {` or `enum FooBar {`.
fn parse_enum_decl(line: &str) -> Option<String> {
    let line = line.trim_start_matches("pub").trim();
    let line = line.strip_prefix("enum ")?.trim();
    let name = line.split('{').next()?.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Extract name from `pub struct FooBar {` or `struct FooBar {`.
fn parse_struct_decl(line: &str) -> Option<String> {
    let line = line.trim_start_matches("pub").trim();
    let line = line.strip_prefix("struct ")?.trim();
    let name = line.split('{').next()?.trim().to_string();
    if name.is_empty() {
        None
    } else {
        Some(name)
    }
}

/// Scan forward from `start` to find the line index of the closing `}`.
/// Handles nested braces.
fn find_closing_brace(lines: &[&str], start: usize) -> usize {
    let mut depth = 1i32;
    let mut i = start;
    while i < lines.len() {
        let line = lines[i];
        for ch in line.chars() {
            match ch {
                '{' => depth += 1,
                '}' => {
                    depth -= 1;
                    if depth == 0 {
                        return i;
                    }
                }
                _ => {}
            }
        }
        i += 1;
    }
    // Fallback: return end of file if brace never closes.
    lines.len().saturating_sub(1)
}

/// Parse enum body lines into a list of `VariantDef`s.
///
/// Handles:
/// - `Admin,`                    → `VariantDef { name: "Admin", fields: [] }`
/// - `Stake(Address),`           → `VariantDef { name: "Stake", fields: ["Address"] }`
/// - `Approval(u64, Address),`   → `VariantDef { name: "Approval", fields: ["u64", "Address"] }`
/// - Line comments are stripped and preserved as `comment`.
fn parse_enum_body(name: &str, lines: &[&str]) -> EnumSchema {
    let mut variants = Vec::new();
    let mut pending_comment: Option<String> = None;

    for line in lines {
        let line = line.trim();

        // Capture doc/line comments to associate with the next variant.
        if line.starts_with("///") || line.starts_with("//") {
            let comment_text = line.trim_start_matches('/').trim().to_string();
            pending_comment = Some(comment_text);
            continue;
        }

        // Strip inline comment.
        let line = if let Some(pos) = line.find("//") {
            line[..pos].trim()
        } else {
            line
        };

        // Skip blank or purely decorative lines.
        if line.is_empty() || line == "{" || line == "}" || line.starts_with('#') {
            continue;
        }

        // Remove trailing comma.
        let line = line.trim_end_matches(',').trim();

        if let Some(variant) = parse_variant(line, pending_comment.take()) {
            variants.push(variant);
        }
    }

    EnumSchema {
        name: name.to_string(),
        variants,
        inferred_tier: infer_tier(name),
    }
}

/// Parse a single enum variant line.
fn parse_variant(line: &str, comment: Option<String>) -> Option<VariantDef> {
    if line.is_empty() {
        return None;
    }

    // Tuple variant: `Stake(Address)` or `Approval(u64, Address)`.
    if let Some(paren_start) = line.find('(') {
        let name = line[..paren_start].trim().to_string();
        if name.is_empty() {
            return None;
        }
        let inner = line[paren_start + 1..].trim_end_matches(')').trim();
        let fields: Vec<String> = if inner.is_empty() {
            vec![]
        } else {
            split_tuple_fields(inner)
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect()
        };
        return Some(VariantDef { name, fields, comment });
    }

    // Unit variant: `Admin` or `Admin { ... }` (we only handle unit for now).
    let name = line.split('{').next()?.trim().to_string();
    if name.is_empty() || !name.chars().next().map(|c| c.is_uppercase()).unwrap_or(false) {
        return None;
    }
    Some(VariantDef { name, fields: vec![], comment })
}

/// Split `u64, Address` into `["u64", "Address"]`, respecting nested `<>`.
fn split_tuple_fields(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut depth = 0i32;

    for ch in s.chars() {
        match ch {
            '<' | '(' => {
                depth += 1;
                current.push(ch);
            }
            '>' | ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 => {
                let part = current.trim().to_string();
                if !part.is_empty() {
                    parts.push(part);
                }
                current.clear();
            }
            _ => current.push(ch),
        }
    }
    let part = current.trim().to_string();
    if !part.is_empty() {
        parts.push(part);
    }
    parts
}

/// Parse struct body lines into `StructSchema`.
///
/// Handles: `pub field_name: TypeName,`
fn parse_struct_body(name: &str, lines: &[&str]) -> StructSchema {
    let mut fields = Vec::new();

    for line in lines {
        let line = line.trim();

        // Skip comments and decorators.
        if line.starts_with("///") || line.starts_with("//") || line.starts_with('#') || line.is_empty() {
            continue;
        }

        // Strip inline comment.
        let line = if let Some(pos) = line.find("//") {
            line[..pos].trim()
        } else {
            line
        };

        // `pub field_name: TypeName,`
        let line = line.trim_start_matches("pub").trim();
        let line = line.trim_end_matches(',').trim();

        if let Some(colon) = line.find(':') {
            let field_name = line[..colon].trim().to_string();
            let field_ty = line[colon + 1..].trim().to_string();

            if !field_name.is_empty()
                && !field_ty.is_empty()
                && field_name.chars().all(|c| c.is_alphanumeric() || c == '_')
                && !field_name.starts_with(|c: char| c.is_uppercase())
            {
                fields.push(FieldDef { name: field_name, ty: field_ty });
            }
        }
    }

    StructSchema { name: name.to_string(), fields }
}

/// Infer a storage tier hint from the type name.
fn infer_tier(name: &str) -> StorageTier {
    match name {
        "InstanceKey" => StorageTier::Instance,
        "PersistentKey" => StorageTier::Persistent,
        "TempKey" | "TemporaryKey" => StorageTier::Temporary,
        // DataKey is the most common pattern — treat as Persistent by default.
        _ => StorageTier::Persistent,
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_unit_variant() {
        let v = parse_variant("Admin", None).unwrap();
        assert_eq!(v.name, "Admin");
        assert!(v.fields.is_empty());
    }

    #[test]
    fn test_parse_tuple_variant_single() {
        let v = parse_variant("Stake(Address)", None).unwrap();
        assert_eq!(v.name, "Stake");
        assert_eq!(v.fields, vec!["Address"]);
    }

    #[test]
    fn test_parse_tuple_variant_multi() {
        let v = parse_variant("Approval(u64, Address)", None).unwrap();
        assert_eq!(v.name, "Approval");
        assert_eq!(v.fields, vec!["u64", "Address"]);
    }

    #[test]
    fn test_parse_struct_body() {
        let lines = [
            "    pub mentor: Address,",
            "    pub amount: i128,",
            "    pub tier: u32,",
        ];
        let schema = parse_struct_body("StakeRecord", &lines);
        assert_eq!(schema.fields.len(), 3);
        assert_eq!(schema.fields[0].name, "mentor");
        assert_eq!(schema.fields[0].ty, "Address");
        assert_eq!(schema.fields[2].name, "tier");
    }

    #[test]
    fn test_split_nested_generics() {
        let parts = split_tuple_fields("Vec<Address>, u64");
        assert_eq!(parts, vec!["Vec<Address>", "u64"]);
    }
}

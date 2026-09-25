/// Schema diff engine.
///
/// Compares two `SchemaSnapshot` values and classifies every change as
/// `Compatible`, `Warning`, or `Breaking`.
///
/// # Breaking change rules (Soroban Eternal Storage)
///
/// Soroban serialises `#[contracttype]` values via XDR. The on-disk
/// discriminants are positional for enums and field-ordered for structs.
///
/// Breaking changes that will corrupt existing ledger state:
/// - Removing an enum variant (shifts discriminants of subsequent variants)
/// - Reordering enum variants (changes discriminant values)
/// - Removing a struct field (deserialisers expect a fixed field count)
/// - Changing a field's type or a variant's payload types
/// - Renaming a variant that is used as a storage key
///
/// Compatible / safe changes:
/// - Adding a new variant at the *end* of a DataKey enum
/// - Adding a new struct at the end of a file
/// - Adding a new storage key (new variants not present in baseline)
///
/// Warnings (compatible but worth human review):
/// - Adding a field to a struct (old on-disk values cannot deserialise the
///   new struct unless a migration reads and rewrites them)
/// - Renaming a struct or enum (old key strings are still valid, but code
///   must handle both names during transition)

use std::collections::HashMap;

use crate::types::{
    DiffKind, EnumSchema, MigrationReport, SchemaDiff, SchemaSnapshot, Severity, StructSchema,
};

/// Compare `baseline` (old) against `current` (new) and produce a report.
pub fn diff(baseline: &SchemaSnapshot, current: &SchemaSnapshot) -> MigrationReport {
    let mut diffs: Vec<SchemaDiff> = Vec::new();

    // Index baseline contracts by name.
    let baseline_map: HashMap<&str, _> = baseline
        .contracts
        .iter()
        .map(|c| (c.contract.as_str(), c))
        .collect();
    let current_map: HashMap<&str, _> = current
        .contracts
        .iter()
        .map(|c| (c.contract.as_str(), c))
        .collect();

    // Check contracts that existed in baseline.
    for (name, base_contract) in &baseline_map {
        if let Some(curr_contract) = current_map.get(name) {
            // Diff enums.
            diff_enums(name, &base_contract.enums, &curr_contract.enums, &mut diffs);
            // Diff structs.
            diff_structs(name, &base_contract.structs, &curr_contract.structs, &mut diffs);
        } else {
            // Entire contract removed — could be a rename or deletion.
            diffs.push(SchemaDiff {
                contract: name.to_string(),
                type_name: "*".to_string(),
                severity: Severity::Warning,
                kind: DiffKind::TypeRemoved,
                description: format!(
                    "Contract `{name}` is present in baseline but not in current snapshot. \
                     If renamed, update all storage key references."
                ),
            });
        }
    }

    // New contracts — compatible, no existing data to break.
    for name in current_map.keys() {
        if !baseline_map.contains_key(name) {
            diffs.push(SchemaDiff {
                contract: name.to_string(),
                type_name: "*".to_string(),
                severity: Severity::Compatible,
                kind: DiffKind::TypeAdded,
                description: format!("New contract `{name}` added — no existing storage to migrate."),
            });
        }
    }

    let breaking_count = diffs.iter().filter(|d| d.severity == Severity::Breaking).count();
    let warning_count = diffs.iter().filter(|d| d.severity == Severity::Warning).count();
    let compatible_count = diffs.iter().filter(|d| d.severity == Severity::Compatible).count();

    MigrationReport {
        from_version: baseline.version.clone(),
        to_version: current.version.clone(),
        from_sha: baseline.sha.clone(),
        to_sha: current.sha.clone(),
        is_safe: breaking_count == 0,
        breaking_count,
        warning_count,
        compatible_count,
        diffs,
    }
}

// ---------------------------------------------------------------------------
// Enum diffing
// ---------------------------------------------------------------------------

fn diff_enums(
    contract: &str,
    baseline: &[EnumSchema],
    current: &[EnumSchema],
    out: &mut Vec<SchemaDiff>,
) {
    let base_map: HashMap<&str, &EnumSchema> =
        baseline.iter().map(|e| (e.name.as_str(), e)).collect();
    let curr_map: HashMap<&str, &EnumSchema> =
        current.iter().map(|e| (e.name.as_str(), e)).collect();

    for (name, base_enum) in &base_map {
        if let Some(curr_enum) = curr_map.get(name) {
            diff_enum_variants(contract, name, base_enum, curr_enum, out);
        } else {
            out.push(SchemaDiff {
                contract: contract.to_string(),
                type_name: name.to_string(),
                severity: Severity::Breaking,
                kind: DiffKind::TypeRemoved,
                description: format!(
                    "Enum `{name}` removed. Any storage keys using this type will become \
                     inaccessible and existing data will be orphaned."
                ),
            });
        }
    }

    // New enums.
    for name in curr_map.keys() {
        if !base_map.contains_key(name) {
            out.push(SchemaDiff {
                contract: contract.to_string(),
                type_name: name.to_string(),
                severity: Severity::Compatible,
                kind: DiffKind::TypeAdded,
                description: format!("New enum `{name}` added."),
            });
        }
    }
}

fn diff_enum_variants(
    contract: &str,
    enum_name: &str,
    base: &EnumSchema,
    curr: &EnumSchema,
    out: &mut Vec<SchemaDiff>,
) {
    let base_map: HashMap<&str, _> = base.variants.iter().map(|v| (v.name.as_str(), v)).collect();
    let curr_map: HashMap<&str, _> = curr.variants.iter().map(|v| (v.name.as_str(), v)).collect();

    // Check ordering — reordering is breaking because discriminants shift.
    let base_order: Vec<&str> = base.variants.iter().map(|v| v.name.as_str()).collect();
    let curr_order: Vec<&str> = curr.variants.iter().map(|v| v.name.as_str()).collect();
    let common_in_base: Vec<&str> = base_order
        .iter()
        .filter(|n| curr_map.contains_key(*n))
        .copied()
        .collect();
    let common_in_curr: Vec<&str> = curr_order
        .iter()
        .filter(|n| base_map.contains_key(*n))
        .copied()
        .collect();

    if common_in_base != common_in_curr {
        out.push(SchemaDiff {
            contract: contract.to_string(),
            type_name: enum_name.to_string(),
            severity: Severity::Breaking,
            kind: DiffKind::VariantRenamed,
            description: format!(
                "Variant order changed in `{enum_name}`. \
                 Soroban XDR discriminants are positional — reordering variants corrupts \
                 existing storage. Old order: [{base}], new order: [{curr}].",
                base = common_in_base.join(", "),
                curr = common_in_curr.join(", "),
            ),
        });
    }

    // Removed variants.
    for (name, _) in &base_map {
        if !curr_map.contains_key(name) {
            out.push(SchemaDiff {
                contract: contract.to_string(),
                type_name: enum_name.to_string(),
                severity: Severity::Breaking,
                kind: DiffKind::VariantRemoved,
                description: format!(
                    "Variant `{enum_name}::{name}` removed. All storage entries keyed by this \
                     variant become permanently inaccessible after the upgrade."
                ),
            });
        }
    }

    // Changed payload types.
    for (name, base_v) in &base_map {
        if let Some(curr_v) = curr_map.get(name) {
            if base_v.fields != curr_v.fields {
                out.push(SchemaDiff {
                    contract: contract.to_string(),
                    type_name: format!("{enum_name}::{name}"),
                    severity: Severity::Breaking,
                    kind: DiffKind::VariantPayloadChanged,
                    description: format!(
                        "Payload of `{enum_name}::{name}` changed: \
                         was ({old}), now ({new}). \
                         Existing storage entries with this key cannot be decoded.",
                        old = base_v.fields.join(", "),
                        new = curr_v.fields.join(", "),
                    ),
                });
            }
        }
    }

    // New variants appended at the end — compatible.
    for name in curr_map.keys() {
        if !base_map.contains_key(name) {
            out.push(SchemaDiff {
                contract: contract.to_string(),
                type_name: enum_name.to_string(),
                severity: Severity::Compatible,
                kind: DiffKind::VariantAdded,
                description: format!(
                    "New variant `{enum_name}::{name}` added. \
                     Safe if appended at the end (discriminant order preserved)."
                ),
            });
        }
    }
}

// ---------------------------------------------------------------------------
// Struct diffing
// ---------------------------------------------------------------------------

fn diff_structs(
    contract: &str,
    baseline: &[StructSchema],
    current: &[StructSchema],
    out: &mut Vec<SchemaDiff>,
) {
    let base_map: HashMap<&str, &StructSchema> =
        baseline.iter().map(|s| (s.name.as_str(), s)).collect();
    let curr_map: HashMap<&str, &StructSchema> =
        current.iter().map(|s| (s.name.as_str(), s)).collect();

    for (name, base_struct) in &base_map {
        if let Some(curr_struct) = curr_map.get(name) {
            diff_struct_fields(contract, name, base_struct, curr_struct, out);
        } else {
            out.push(SchemaDiff {
                contract: contract.to_string(),
                type_name: name.to_string(),
                severity: Severity::Breaking,
                kind: DiffKind::TypeRemoved,
                description: format!(
                    "Struct `{name}` removed. Any persistent storage values serialised as \
                     `{name}` will fail to deserialise."
                ),
            });
        }
    }

    for name in curr_map.keys() {
        if !base_map.contains_key(name) {
            out.push(SchemaDiff {
                contract: contract.to_string(),
                type_name: name.to_string(),
                severity: Severity::Compatible,
                kind: DiffKind::TypeAdded,
                description: format!("New struct `{name}` added."),
            });
        }
    }
}

fn diff_struct_fields(
    contract: &str,
    struct_name: &str,
    base: &StructSchema,
    curr: &StructSchema,
    out: &mut Vec<SchemaDiff>,
) {
    let base_map: HashMap<&str, &str> =
        base.fields.iter().map(|f| (f.name.as_str(), f.ty.as_str())).collect();
    let curr_map: HashMap<&str, &str> =
        curr.fields.iter().map(|f| (f.name.as_str(), f.ty.as_str())).collect();

    // Removed fields.
    for (name, _) in &base_map {
        if !curr_map.contains_key(name) {
            out.push(SchemaDiff {
                contract: contract.to_string(),
                type_name: struct_name.to_string(),
                severity: Severity::Breaking,
                kind: DiffKind::FieldRemoved,
                description: format!(
                    "Field `{struct_name}.{name}` removed. \
                     Soroban XDR decoding expects a fixed field count — removing a field \
                     will corrupt reads of existing storage values."
                ),
            });
        }
    }

    // Changed types.
    for (name, base_ty) in &base_map {
        if let Some(curr_ty) = curr_map.get(name) {
            if *base_ty != *curr_ty {
                out.push(SchemaDiff {
                    contract: contract.to_string(),
                    type_name: struct_name.to_string(),
                    severity: Severity::Breaking,
                    kind: DiffKind::FieldTypeChanged,
                    description: format!(
                        "Type of `{struct_name}.{name}` changed from `{base_ty}` to `{curr_ty}`. \
                         Existing storage values cannot be decoded with the new type."
                    ),
                });
            }
        }
    }

    // Added fields — warning, not breaking, but requires migration to populate.
    for name in curr_map.keys() {
        if !base_map.contains_key(name) {
            out.push(SchemaDiff {
                contract: contract.to_string(),
                type_name: struct_name.to_string(),
                severity: Severity::Warning,
                kind: DiffKind::FieldAdded,
                description: format!(
                    "Field `{struct_name}.{name}` added. \
                     Old storage entries will fail to deserialise unless a migration \
                     script backfills this field or the code uses a versioned fallback."
                ),
            });
        }
    }

    // Field reordering — breaking (XDR position-sensitive).
    let base_order: Vec<&str> = base.fields.iter().map(|f| f.name.as_str()).collect();
    let curr_order: Vec<&str> = curr.fields.iter().map(|f| f.name.as_str()).collect();
    let common_base: Vec<&str> = base_order.iter().filter(|n| curr_map.contains_key(*n)).copied().collect();
    let common_curr: Vec<&str> = curr_order.iter().filter(|n| base_map.contains_key(*n)).copied().collect();

    if common_base != common_curr {
        out.push(SchemaDiff {
            contract: contract.to_string(),
            type_name: struct_name.to_string(),
            severity: Severity::Breaking,
            kind: DiffKind::FieldTypeChanged,
            description: format!(
                "Field order changed in `{struct_name}`. \
                 Soroban XDR is position-sensitive — reordering fields corrupts reads. \
                 Old order: [{old}], new order: [{new}].",
                old = common_base.join(", "),
                new = common_curr.join(", "),
            ),
        });
    }
}

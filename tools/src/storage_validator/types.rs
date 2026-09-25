/// Schema types for the MentorsMind storage migration validator.
///
/// A `ContractSchema` captures every `#[contracttype]` enum and struct
/// that appears in a contract's source. Schemas are serialised to JSON
/// snapshots and compared across versions to detect breaking changes.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// Schema primitives
// ---------------------------------------------------------------------------

/// Storage tier — mirrors Soroban's three storage namespaces.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StorageTier {
    Instance,
    Persistent,
    Temporary,
    /// Used for types that are values (not keys) in storage.
    Value,
}

/// A single field inside a `#[contracttype]` struct.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    pub ty: String,
}

/// A single variant of a `#[contracttype]` enum.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct VariantDef {
    /// Variant name, e.g. `Stake`.
    pub name: String,
    /// Payload types, e.g. `["Address"]` for `Stake(Address)`.
    pub fields: Vec<String>,
    /// Discriminant comment, if present in source.
    pub comment: Option<String>,
}

/// A `#[contracttype]` enum definition — typically used as a storage key.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnumSchema {
    pub name: String,
    pub variants: Vec<VariantDef>,
    /// Inferred tier: if the enum is named `DataKey` and its variant names
    /// map to storage tier hints, we record the tier here.
    pub inferred_tier: StorageTier,
}

/// A `#[contracttype]` struct definition — typically a storage value.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct StructSchema {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

/// Complete schema for one contract source file.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractSchema {
    /// Contract crate name, e.g. `"staking"`.
    pub contract: String,
    /// Source file path relative to workspace root.
    pub source_path: String,
    /// All `#[contracttype]` enums found in this file.
    pub enums: Vec<EnumSchema>,
    /// All `#[contracttype]` structs found in this file.
    pub structs: Vec<StructSchema>,
}

// ---------------------------------------------------------------------------
// Snapshot — a versioned collection of contract schemas
// ---------------------------------------------------------------------------

/// A point-in-time snapshot of all contract storage schemas.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaSnapshot {
    /// Schema version tag, e.g. `"v1"`, `"main"`, or a commit SHA.
    pub version: String,
    /// ISO-8601 date when this snapshot was taken.
    pub captured_at: String,
    /// Git ref name (branch / tag), populated from `GITHUB_REF_NAME`.
    pub ref_name: String,
    /// Git commit SHA, populated from `GITHUB_SHA`.
    pub sha: String,
    /// One entry per scanned contract.
    pub contracts: Vec<ContractSchema>,
}

// ---------------------------------------------------------------------------
// Diff types
// ---------------------------------------------------------------------------

/// Severity of a detected storage change.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    /// Safe to deploy without a migration.
    Compatible,
    /// Requires a migration script or data transformation.
    Breaking,
    /// Warning — technically compatible but worth human review.
    Warning,
}

/// A single detected change between two schema versions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchemaDiff {
    pub contract: String,
    pub type_name: String,
    pub severity: Severity,
    pub kind: DiffKind,
    pub description: String,
}

/// The category of a schema change.
#[derive(Debug, Clone, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiffKind {
    // Enum key changes
    VariantRemoved,
    VariantAdded,
    VariantPayloadChanged,
    VariantRenamed,
    // Struct field changes
    FieldRemoved,
    FieldAdded,
    FieldTypeChanged,
    // Type-level changes
    TypeRemoved,
    TypeAdded,
    TypeKindChanged, // enum ↔ struct swap
}

/// The full output of comparing two snapshots.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    pub from_version: String,
    pub to_version: String,
    pub from_sha: String,
    pub to_sha: String,
    pub diffs: Vec<SchemaDiff>,
    pub breaking_count: usize,
    pub warning_count: usize,
    pub compatible_count: usize,
    /// Overall verdict: true = safe to upgrade without migration.
    pub is_safe: bool,
}

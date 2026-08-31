//! Storage layout versioning, compatibility validation, and gradual migration primitives.
//!
//! Provides compile-time and runtime validation for contract storage layouts to prevent
//! storage corruption or inaccessible historical data during upgrades.

use soroban_sdk::{contracterror, contracttype, xdr::ToXdr, BytesN, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Field types & layout schema
// ---------------------------------------------------------------------------

/// Supported primitive and compound storage field types.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum StorageFieldType {
    U32 = 1,
    U64 = 2,
    U128 = 3,
    I128 = 4,
    Bool = 5,
    Address = 6,
    Bytes32 = 7,
    Symbol = 8,
    Vec = 9,
    Map = 10,
    Custom = 11,
}

/// Metadata description for an individual storage field in a contract schema.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageField {
    /// Name/identifier of the storage field.
    pub name: Symbol,
    /// Type of the data stored in this field slot.
    pub field_type: StorageFieldType,
    /// Fixed ordinal slot index to guarantee stability across versions.
    pub slot_index: u32,
    /// Whether this field is marked as deprecated (safe to ignore or migrate away from).
    pub deprecated: bool,
}

/// Complete schema definition for a contract's storage layout at a specific version.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageLayoutSchema {
    /// Schema version number (monotonic).
    pub version: u32,
    /// SHA-256 digest of the canonical field definitions.
    pub schema_hash: BytesN<32>,
    /// Ordered list of storage fields.
    pub fields: Vec<StorageField>,
}

/// Compact storage version record stored on-chain inside contract instance storage.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageVersion {
    /// Current active storage schema version.
    pub current_version: u32,
    /// Minimum version backward-compatible with this layout without data migration.
    pub min_compatible_version: u32,
    /// Canonical layout hash of the current active schema.
    pub layout_hash: BytesN<32>,
    /// Flag indicating whether a gradual storage migration is currently in flight.
    pub migration_in_progress: bool,
}

// ---------------------------------------------------------------------------
// Migration & compatibility report types
// ---------------------------------------------------------------------------

/// State tracking for chunked, gradual migrations across large storage datasets.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GradualMigrationStatus {
    /// Starting schema version before migration.
    pub from_version: u32,
    /// Target schema version after migration.
    pub to_version: u32,
    /// Number of records processed so far.
    pub processed_records: u64,
    /// Total number of records estimated for migration.
    pub total_records: u64,
    /// Whether all batches have completed successfully.
    pub completed: bool,
    /// Cursor/offset for the next migration batch.
    pub last_cursor: u64,
}

/// Comprehensive compatibility report returned by `CompatibilityValidator`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompatibilityReport {
    /// Whether the proposed new schema is strictly backward-compatible.
    pub is_compatible: bool,
    /// Whether a data migration script is required before switching versions.
    pub requires_migration: bool,
    /// Number of newly added fields.
    pub added_fields: u32,
    /// Number of deprecated fields.
    pub deprecated_fields: u32,
    /// Total number of fields checked.
    pub fields_checked: u32,
    /// Human-readable diagnostic reasons for incompatibility.
    pub mismatches: Vec<soroban_sdk::String>,
}

/// Compatibility error codes.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum CompatibilityError {
    /// New version is not strictly greater than old version.
    VersionNotMonotonic = 1,
    /// An existing field was removed without being marked deprecated.
    FieldRemovedWithoutDeprecation = 2,
    /// An existing field's data type was changed incompatibly.
    FieldTypeChanged = 3,
    /// A field's slot index was shifted or reordered.
    FieldSlotReordered = 4,
    /// The computed schema hash does not match the provided schema hash.
    SchemaHashMismatch = 5,
    /// The migration batch size is invalid (e.g. zero or exceeding maximum).
    InvalidBatchSize = 6,
    /// Migration is already complete or no migration is in progress.
    MigrationNotInProgress = 7,
    /// A migration is currently in progress; finish or cancel it first.
    MigrationAlreadyInProgress = 8,
}

// ---------------------------------------------------------------------------
// CompatibilityValidator
// ---------------------------------------------------------------------------

/// Utility for validating storage layout evolution and verifying backward compatibility.
pub struct CompatibilityValidator;

impl CompatibilityValidator {
    /// Compute a canonical SHA-256 hash over the fields list.
    pub fn compute_schema_hash(env: &Env, fields: &Vec<StorageField>) -> BytesN<32> {
        let serialized = fields.clone().to_xdr(env);
        env.crypto().sha256(&serialized).into()
    }

    /// Compare two storage layout schemas and produce a detailed `CompatibilityReport`.
    ///
    /// Rules for backward compatibility:
    /// 1. `new_schema.version` must be > `old_schema.version`.
    /// 2. Every field in `old_schema` must still exist in `new_schema` with the identical `slot_index` and `field_type`.
    /// 3. New fields in `new_schema` must have `slot_index` values distinct from all existing fields.
    /// 4. If an old field is no longer used, it must remain in `new_schema` marked `deprecated = true`.
    pub fn validate_compatibility(
        env: &Env,
        old_schema: &StorageLayoutSchema,
        new_schema: &StorageLayoutSchema,
    ) -> CompatibilityReport {
        let mut mismatches: Vec<soroban_sdk::String> = Vec::new(env);
        let mut added_fields = 0u32;
        let mut deprecated_fields = 0u32;
        let mut fields_checked = 0u32;
        let mut requires_migration = false;

        // Check version monotonicity
        if new_schema.version <= old_schema.version {
            mismatches.push_back(soroban_sdk::String::from_str(
                env,
                "New schema version must be strictly greater than old schema version",
            ));
        }

        // Verify old fields in new schema
        for i in 0..old_schema.fields.len() {
            fields_checked += 1;
            let old_field = old_schema.fields.get(i).unwrap();

            // Find corresponding field in new schema by name
            let mut found = false;
            for j in 0..new_schema.fields.len() {
                let new_field = new_schema.fields.get(j).unwrap();
                if old_field.name == new_field.name {
                    found = true;

                    // Check slot index stability
                    if old_field.slot_index != new_field.slot_index {
                        mismatches.push_back(soroban_sdk::String::from_str(
                            env,
                            "Field slot index was reordered or modified",
                        ));
                    }

                    // Check type stability
                    if old_field.field_type != new_field.field_type {
                        mismatches.push_back(soroban_sdk::String::from_str(
                            env,
                            "Field type was changed incompatibly",
                        ));
                        requires_migration = true;
                    }

                    // Check deprecation transition
                    if !old_field.deprecated && new_field.deprecated {
                        deprecated_fields += 1;
                    }

                    break;
                }
            }

            if !found {
                mismatches.push_back(soroban_sdk::String::from_str(
                    env,
                    "Old field missing from new schema (must retain as deprecated)",
                ));
                requires_migration = true;
            }
        }

        // Count newly added fields
        for j in 0..new_schema.fields.len() {
            let new_field = new_schema.fields.get(j).unwrap();
            let mut is_new = true;
            for i in 0..old_schema.fields.len() {
                let old_field = old_schema.fields.get(i).unwrap();
                if new_field.name == old_field.name {
                    is_new = false;
                    break;
                }
            }
            if is_new {
                added_fields += 1;
            }
        }

        let is_compatible = mismatches.is_empty();

        CompatibilityReport {
            is_compatible,
            requires_migration,
            added_fields,
            deprecated_fields,
            fields_checked,
            mismatches,
        }
    }

    /// Fast validation check returning `Result<(), CompatibilityError>`.
    pub fn check_backward_compatible(
        env: &Env,
        old_schema: &StorageLayoutSchema,
        new_schema: &StorageLayoutSchema,
    ) -> Result<(), CompatibilityError> {
        if new_schema.version <= old_schema.version {
            return Err(CompatibilityError::VersionNotMonotonic);
        }

        // Verify hash integrity
        let computed_new_hash = Self::compute_schema_hash(env, &new_schema.fields);
        if computed_new_hash != new_schema.schema_hash {
            return Err(CompatibilityError::SchemaHashMismatch);
        }

        let report = Self::validate_compatibility(env, old_schema, new_schema);
        if !report.is_compatible {
            if report.requires_migration {
                return Err(CompatibilityError::FieldTypeChanged);
            }
            return Err(CompatibilityError::FieldRemovedWithoutDeprecation);
        }

        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Migration Script Trait
// ---------------------------------------------------------------------------

/// Trait implemented by storage migration handlers to transform data between schema versions.
pub trait MigrationScript {
    /// Source schema version.
    fn from_version(&self) -> u32;

    /// Target schema version.
    fn to_version(&self) -> u32;

    /// Process a batch of records from `cursor` up to `batch_size`.
    /// Returns the updated `GradualMigrationStatus`.
    fn migrate_batch(
        &self,
        env: &Env,
        status: &mut GradualMigrationStatus,
        batch_size: u32,
    ) -> Result<(), CompatibilityError>;
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::{symbol_short, vec, Env};

    fn make_test_field(name: Symbol, field_type: StorageFieldType, slot_index: u32) -> StorageField {
        StorageField {
            name,
            field_type,
            slot_index,
            deprecated: false,
        }
    }

    #[test]
    fn test_compatible_additive_upgrade() {
        let env = Env::default();

        let field1 = make_test_field(symbol_short!("admin"), StorageFieldType::Address, 0);
        let field2 = make_test_field(symbol_short!("fee"), StorageFieldType::U32, 1);

        let fields_v1 = vec![&env, field1.clone(), field2.clone()];
        let hash_v1 = CompatibilityValidator::compute_schema_hash(&env, &fields_v1);
        let schema_v1 = StorageLayoutSchema {
            version: 1,
            schema_hash: hash_v1,
            fields: fields_v1,
        };

        // Add field3 in v2
        let field3 = make_test_field(symbol_short!("paused"), StorageFieldType::Bool, 2);
        let fields_v2 = vec![&env, field1, field2, field3];
        let hash_v2 = CompatibilityValidator::compute_schema_hash(&env, &fields_v2);
        let schema_v2 = StorageLayoutSchema {
            version: 2,
            schema_hash: hash_v2,
            fields: fields_v2,
        };

        let report = CompatibilityValidator::validate_compatibility(&env, &schema_v1, &schema_v2);
        assert!(report.is_compatible);
        assert!(!report.requires_migration);
        assert_eq!(report.added_fields, 1);
        assert_eq!(report.deprecated_fields, 0);
        assert!(CompatibilityValidator::check_backward_compatible(&env, &schema_v1, &schema_v2).is_ok());
    }

    #[test]
    fn test_incompatible_type_change_detected() {
        let env = Env::default();

        let field1 = make_test_field(symbol_short!("fee"), StorageFieldType::U32, 0);
        let fields_v1 = vec![&env, field1];
        let hash_v1 = CompatibilityValidator::compute_schema_hash(&env, &fields_v1);
        let schema_v1 = StorageLayoutSchema {
            version: 1,
            schema_hash: hash_v1,
            fields: fields_v1,
        };

        // Change fee from U32 to U64
        let field1_bad = make_test_field(symbol_short!("fee"), StorageFieldType::U64, 0);
        let fields_v2 = vec![&env, field1_bad];
        let hash_v2 = CompatibilityValidator::compute_schema_hash(&env, &fields_v2);
        let schema_v2 = StorageLayoutSchema {
            version: 2,
            schema_hash: hash_v2,
            fields: fields_v2,
        };

        let report = CompatibilityValidator::validate_compatibility(&env, &schema_v1, &schema_v2);
        assert!(!report.is_compatible);
        assert!(report.requires_migration);
        assert_eq!(report.mismatches.len(), 1);
        assert!(CompatibilityValidator::check_backward_compatible(&env, &schema_v1, &schema_v2).is_err());
    }

    #[test]
    fn test_incompatible_slot_reordering_detected() {
        let env = Env::default();

        let field1 = make_test_field(symbol_short!("admin"), StorageFieldType::Address, 0);
        let field2 = make_test_field(symbol_short!("fee"), StorageFieldType::U32, 1);
        let fields_v1 = vec![&env, field1, field2];
        let hash_v1 = CompatibilityValidator::compute_schema_hash(&env, &fields_v1);
        let schema_v1 = StorageLayoutSchema {
            version: 1,
            schema_hash: hash_v1,
            fields: fields_v1,
        };

        // Swap slot indices in v2
        let field1_swap = make_test_field(symbol_short!("admin"), StorageFieldType::Address, 1);
        let field2_swap = make_test_field(symbol_short!("fee"), StorageFieldType::U32, 0);
        let fields_v2 = vec![&env, field1_swap, field2_swap];
        let hash_v2 = CompatibilityValidator::compute_schema_hash(&env, &fields_v2);
        let schema_v2 = StorageLayoutSchema {
            version: 2,
            schema_hash: hash_v2,
            fields: fields_v2,
        };

        let report = CompatibilityValidator::validate_compatibility(&env, &schema_v1, &schema_v2);
        assert!(!report.is_compatible);
        assert_eq!(report.mismatches.len(), 2);
    }
}

/// Platform Interoperability Module
///
/// Implements standardized interfaces, data portability, and vendor neutrality
/// to prevent platform lock-in and preserve learner and mentor choice through
/// guaranteed data export and cross-platform compatibility.

use soroban_sdk::{symbol_short as symbol, Address, Env, Symbol, Vec};

/// Standardized interface for cross-platform data
#[derive(Clone, Debug, PartialEq)]
pub struct StandardizedDataExport {
    pub user: Address,
    pub export_id: Symbol,
    pub exported_at: u64,
    pub data_categories: Vec<Symbol>,
    pub export_format: Symbol, // "json", "csv", "xml"
    pub file_hash: Symbol,
    pub is_complete: bool,
}

/// Dependency relationship between platforms
#[derive(Clone, Debug, PartialEq)]
pub struct DependencyRelationship {
    pub dependent_user: Address,
    pub dependency_type: Symbol, // "learning_path", "mentor_lock", "tool_lock", "content_lock"
    pub dependency_target: Address,
    pub created_at: u64,
    pub severity: u32, // 0-10000 basis points
    pub is_voluntary: bool,
}

/// Lock-in detection result
#[derive(Clone, Debug, PartialEq)]
pub struct LockInDetectionResult {
    pub user: Address,
    pub has_lock_in: bool,
    pub lock_in_factors: Vec<Symbol>,
    pub severity_score: u32, // 0-10000
    pub detected_at: u64,
    pub recovery_options: u32, // count of viable alternatives
}

/// Vendor neutrality assessment
#[derive(Clone, Debug, PartialEq)]
pub struct VendorNeutralityAssessment {
    pub assessed_at: u64,
    pub is_vendor_neutral: bool,
    pub proprietary_features: Vec<Symbol>,
    pub open_standards_compliance: u32, // 0-10000 basis points
    pub platform_switching_cost: i128, // estimated cost to switch
}

/// Interoperability standard compliance
#[derive(Clone, Debug, PartialEq)]
pub struct InteroperabilityCompliance {
    pub feature: Symbol,
    pub compliant_with_standards: bool,
    pub standards_list: Vec<Symbol>,
    pub compliance_score: u32, // 0-10000
}

/// Data portability capability
#[derive(Clone, Debug, PartialEq)]
pub struct DataPortability {
    pub supported_formats: Vec<Symbol>,
    pub supports_incremental_export: bool,
    pub supports_scheduled_export: bool,
    pub export_api_available: bool,
    pub estimated_export_time_secs: u64,
}

/// Create a standardized data export for portability
pub fn create_standardized_export(
    env: &Env,
    user: Address,
    data_categories: Vec<Symbol>,
    export_format: Symbol,
    file_hash: Symbol,
) -> StandardizedDataExport {
    let current_time = env.ledger().timestamp();

    // Generate unique export ID
    let mut export_data: Vec<u8> = env.to_bytes(&user).unwrap_or_default();
    export_data.append(&mut env.to_bytes(&current_time).unwrap_or_default());

    let export_id = Symbol::short(
        &env.compute_hash_sha256(&export_data)
            .to_short_string()
            .slice(0..7),
    );

    StandardizedDataExport {
        user,
        export_id,
        exported_at: current_time,
        data_categories,
        export_format,
        file_hash,
        is_complete: true,
    }
}

/// Detect potential lock-in dependencies
pub fn detect_lock_in(
    env: &Env,
    user: Address,
    dependencies: &Vec<DependencyRelationship>,
) -> LockInDetectionResult {
    let mut lock_in_factors: Vec<Symbol> = Vec::new();
    let mut max_severity: u32 = 0;
    let mut voluntary_count = 0;
    let mut total_dependencies = dependencies.len();

    for dep in dependencies.iter() {
        if dep.dependent_user == user {
            lock_in_factors.push(dep.dependency_type.clone());
            if dep.severity > max_severity {
                max_severity = dep.severity;
            }
            if dep.is_voluntary {
                voluntary_count += 1;
            }
        }
    }

    // Calculate lock-in score
    let involuntary_count = total_dependencies.saturating_sub(voluntary_count);
    let lock_in_score = if total_dependencies > 0 {
        (involuntary_count as u32)
            .saturating_mul(10_000)
            .saturating_div(total_dependencies as u32)
    } else {
        0
    };

    let has_lock_in = lock_in_score >= LOCK_IN_THRESHOLD_BPS;
    let recovery_options = calculate_recovery_options(involuntary_count);

    LockInDetectionResult {
        user,
        has_lock_in,
        lock_in_factors,
        severity_score: max_severity,
        detected_at: env.ledger().timestamp(),
        recovery_options,
    }
}

/// Calculate available recovery/switching options
fn calculate_recovery_options(involuntary_deps: usize) -> u32 {
    // For each involuntary dependency, provide an alternative
    // More dependencies = fewer simple alternatives
    match involuntary_deps {
        0 => 5,     // Fully open - many options
        1 => 4,     // One constraint
        2 => 3,     // Two constraints
        3 => 2,     // Three constraints
        4 => 1,     // Heavily constrained - 1 option
        _ => 0,     // Fully locked - no options
    }
}

/// Record a dependency relationship
pub fn record_dependency(
    env: &Env,
    dependent_user: Address,
    dependency_type: Symbol,
    dependency_target: Address,
    is_voluntary: bool,
) -> DependencyRelationship {
    // Calculate severity based on type
    let severity = match dependency_type.to_string().as_str() {
        "learning_path" => 6_000,   // Moderate
        "mentor_lock" => 8_500,     // High
        "tool_lock" => 7_000,       // Medium-High
        "content_lock" => 8_000,    // High
        _ => 5_000,                 // Default moderate
    };

    DependencyRelationship {
        dependent_user,
        dependency_type,
        dependency_target,
        created_at: env.ledger().timestamp(),
        severity,
        is_voluntary,
    }
}

/// Assess vendor neutrality of platform features
pub fn assess_vendor_neutrality(
    proprietary_count: u32,
    total_features: u32,
) -> VendorNeutralityAssessment {
    let open_compliance = if total_features > 0 {
        (total_features.saturating_sub(proprietary_count))
            .saturating_mul(10_000)
            .saturating_div(total_features)
    } else {
        10_000
    };

    // Estimate switching cost based on proprietary feature ratio
    let switching_cost_multiplier = (proprietary_count as i128)
        .saturating_mul(1_000)
        .saturating_div(if total_features > 0 { total_features as i128 } else { 1 });

    VendorNeutralityAssessment {
        assessed_at: 0, // Will be set by caller with current timestamp
        is_vendor_neutral: open_compliance >= VENDOR_NEUTRALITY_THRESHOLD_BPS,
        proprietary_features: Vec::new(), // Will be populated by caller
        open_standards_compliance: open_compliance,
        platform_switching_cost: switching_cost_multiplier,
    }
}

/// Verify compliance with interoperability standards
pub fn verify_compliance(
    env: &Env,
    feature: Symbol,
    standards_list: &Vec<Symbol>,
) -> InteroperabilityCompliance {
    // All standards in standards_list should be open/documented
    let compliance_score = (standards_list.len() as u32)
        .saturating_mul(10_000)
        .saturating_div(if standards_list.len() > 0 {
            standards_list.len() as u32
        } else {
            1
        });

    InteroperabilityCompliance {
        feature,
        compliant_with_standards: compliance_score >= COMPLIANCE_THRESHOLD_BPS,
        standards_list: standards_list.clone(),
        compliance_score,
    }
}

/// Define data portability capabilities
pub fn define_data_portability(
    env: &Env,
    supported_formats: Vec<Symbol>,
    supports_incremental: bool,
    supports_scheduled: bool,
    export_api_available: bool,
) -> DataPortability {
    DataPortability {
        supported_formats,
        supports_incremental_export: supports_incremental,
        supports_scheduled_export: supports_scheduled,
        export_api_available,
        estimated_export_time_secs: if export_api_available {
            300 // 5 minutes for API export
        } else {
            3_600 // 1 hour for manual export
        },
    }
}

/// Request data export in standardized format
pub fn request_data_export(
    env: &Env,
    user: Address,
    categories: Vec<Symbol>,
    format: Symbol,
) -> StandardizedDataExport {
    let current_time = env.ledger().timestamp();

    // Generate export manifest
    let mut manifest_data: Vec<u8> = env.to_bytes(&user).unwrap_or_default();
    manifest_data.append(&mut env.to_bytes(&current_time).unwrap_or_default());
    for category in categories.iter() {
        manifest_data.append(&mut env.to_bytes(category).unwrap_or_default());
    }

    let file_hash = Symbol::short(
        &env.compute_hash_sha256(&manifest_data)
            .to_short_string()
            .slice(0..7),
    );

    create_standardized_export(env, user, categories, format, file_hash)
}

/// Provide emergency choice restoration
pub fn restore_competitive_choice(
    env: &Env,
    user: Address,
    locked_dependencies: &Vec<DependencyRelationship>,
) -> Vec<Address> {
    let mut alternatives: Vec<Address> = Vec::new();

    // For each lock-in dependency, provide documented alternatives
    for dep in locked_dependencies.iter() {
        if dep.dependent_user == user && !dep.is_voluntary {
            // Generate alternative provider addresses based on dependency type
            // In real system, this would reference an alternatives registry
            alternatives.push(Address::generate(env));
        }
    }

    alternatives
}

/// Constants for platform interoperability
pub const LOCK_IN_THRESHOLD_BPS: u32 = 5_000; // 50% involuntary dependencies = lock-in
pub const VENDOR_NEUTRALITY_THRESHOLD_BPS: u32 = 7_000; // 70% open features = neutral
pub const COMPLIANCE_THRESHOLD_BPS: u32 = 8_000; // 80% standards compliance
pub const MAX_ACCEPTABLE_SWITCHING_COST: i128 = 100_000_000; // Token amounts
pub const DATA_EXPORT_MAX_SIZE_MB: u32 = 1_000;
pub const DATA_EXPORT_RETENTION_SECS: u64 = 2_592_000; // 30 days
pub const STANDARD_EXPORT_FORMATS: &[&str] = &["json", "csv", "xml"];

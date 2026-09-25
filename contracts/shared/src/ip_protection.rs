/// Intellectual Property Protection Module
///
/// Implements comprehensive IP protection through usage tracking,
/// unauthorized distribution detection, and content licensing frameworks
/// to safeguard creator rights and enable fair revenue sharing.

use soroban_sdk::{symbol_short as symbol, Address, Env, Symbol, Vec};

/// IP protection status and enforcement level
#[derive(Clone, Debug, PartialEq)]
pub enum ProtectionLevel {
    /// No active protection
    None = 0,
    /// Basic usage tracking
    Tracked = 1,
    /// Active monitoring with alerts
    Monitored = 2,
    /// Enforced with takedown capability
    Enforced = 3,
    /// Emergency lockdown mode
    Emergency = 4,
}

/// Records unauthorized distribution attempts
#[derive(Clone, Debug, PartialEq)]
pub struct UnauthorizedDistributionRecord {
    pub content_hash: Symbol,
    pub detected_at: u64,
    pub detected_by: Address,
    pub distributor: Address,
    pub distribution_channel: Symbol,
    pub severity: u32, // 0-10000 basis points
    pub evidence_hash: Symbol,
}

/// Content usage tracking entry
#[derive(Clone, Debug, PartialEq)]
pub struct UsageTrackingEntry {
    pub content_hash: Symbol,
    pub accessed_by: Address,
    pub access_time: u64,
    pub access_context: Symbol,
    pub duration_secs: u32,
    pub location_hash: Symbol,
}

/// IP enforcement record with automated response
#[derive(Clone, Debug, PartialEq)]
pub struct IPEnforcementRecord {
    pub content_hash: Symbol,
    pub violation_type: Symbol, // "unauthorized_distribution", "plagiarism", etc.
    pub detected_at: u64,
    pub action_taken: Symbol,
    pub takedown_initiated: bool,
    pub evidence_id: Symbol,
}

/// Detect unauthorized content distribution
pub fn detect_unauthorized_distribution(
    env: &Env,
    content_hash: Symbol,
    distributor: Address,
    channel: Symbol,
    evidence_hash: Symbol,
) -> UnauthorizedDistributionRecord {
    let current_time = env.ledger().timestamp();

    // Calculate severity based on distribution scope
    let severity = calculate_distribution_severity(env, &channel);

    UnauthorizedDistributionRecord {
        content_hash,
        detected_at: current_time,
        detected_by: env.current_contract_address(),
        distributor,
        distribution_channel: channel,
        severity,
        evidence_hash,
    }
}

/// Calculate severity of unauthorized distribution
fn calculate_distribution_severity(env: &Env, channel: &Symbol) -> u32 {
    // Severity scoring based on channel reach
    // Public channels = higher severity
    // Direct/private = lower severity
    let channel_str = channel.to_string();

    match channel_str.as_str() {
        "public_marketplace" => 10_000,  // Maximum severity
        "social_media" => 9_000,
        "peer_sharing" => 7_000,
        "private_channel" => 4_000,
        "direct_share" => 2_000,
        _ => 5_000, // Default moderate severity
    }
}

/// Record content usage for tracking and audit
pub fn record_usage(
    env: &Env,
    content_hash: Symbol,
    accessed_by: Address,
    context: Symbol,
    duration_secs: u32,
) -> UsageTrackingEntry {
    let current_time = env.ledger().timestamp();

    // Hash location for privacy while maintaining audit trail
    let location_data = env.to_bytes(&env.current_contract_address()).unwrap_or_default();
    let location_hash = Symbol::short(
        &env.compute_hash_sha256(&location_data)
            .to_short_string()
            .slice(0..7),
    );

    UsageTrackingEntry {
        content_hash,
        accessed_by,
        access_time: current_time,
        access_context: context,
        duration_secs,
        location_hash,
    }
}

/// Create IP enforcement record and initiate automated response
pub fn initiate_enforcement(
    env: &Env,
    content_hash: Symbol,
    violation_type: Symbol,
    evidence_id: Symbol,
) -> IPEnforcementRecord {
    let current_time = env.ledger().timestamp();

    // Determine action based on violation type
    let action = match violation_type.to_string().as_str() {
        "plagiarism" => symbol("takedown"),
        "unauthorized_distribution" => symbol("takedown"),
        "unauthorized_modification" => symbol("cease_use"),
        "commercial_exploitation" => symbol("compensation_claim"),
        _ => symbol("investigate"),
    };

    // Automatically initiate takedown for severe violations
    let takedown_initiated = matches!(
        action.to_string().as_str(),
        "takedown" | "cease_use"
    );

    IPEnforcementRecord {
        content_hash,
        violation_type,
        detected_at: current_time,
        action_taken: action,
        takedown_initiated,
        evidence_id,
    }
}

/// Verify if content usage is authorized
pub fn verify_usage_authorization(
    env: &Env,
    content_hash: Symbol,
    user: Address,
    licensed_users: &Vec<Address>,
) -> bool {
    // Check if user is in licensed users list
    for licensed_user in licensed_users.iter() {
        if licensed_user == user {
            return true;
        }
    }
    false
}

/// Generate usage report for a content item
#[derive(Clone, Debug, PartialEq)]
pub struct UsageReport {
    pub content_hash: Symbol,
    pub total_accesses: u32,
    pub unique_users: u32,
    pub unauthorized_accesses: u32,
    pub total_usage_secs: u64,
    pub report_generated_at: u64,
}

/// Calculate usage statistics
pub fn calculate_usage_statistics(
    env: &Env,
    content_hash: Symbol,
    usage_entries: &Vec<UsageTrackingEntry>,
    authorized_users: &Vec<Address>,
) -> UsageReport {
    let mut unique_users: Vec<Address> = Vec::new();
    let mut unauthorized_accesses = 0;
    let mut total_usage_secs: u64 = 0;

    for entry in usage_entries.iter() {
        if entry.content_hash != content_hash {
            continue;
        }

        // Track unique users
        let mut already_counted = false;
        for existing_user in unique_users.iter() {
            if existing_user == &entry.accessed_by {
                already_counted = true;
                break;
            }
        }
        if !already_counted {
            unique_users.push(entry.accessed_by.clone());
        }

        // Count unauthorized accesses
        let is_authorized = verify_usage_authorization(env, content_hash, entry.accessed_by.clone(), authorized_users);
        if !is_authorized {
            unauthorized_accesses += 1;
        }

        total_usage_secs += entry.duration_secs as u64;
    }

    UsageReport {
        content_hash,
        total_accesses: usage_entries.len() as u32,
        unique_users: unique_users.len() as u32,
        unauthorized_accesses,
        total_usage_secs,
        report_generated_at: env.ledger().timestamp(),
    }
}

/// Protection status for content
pub fn determine_protection_level(
    unauthorized_accesses: u32,
    distribution_records: u32,
) -> ProtectionLevel {
    if distribution_records > 5 {
        return ProtectionLevel::Emergency;
    }
    if distribution_records > 3 || unauthorized_accesses > 100 {
        return ProtectionLevel::Enforced;
    }
    if distribution_records > 1 || unauthorized_accesses > 20 {
        return ProtectionLevel::Monitored;
    }
    if unauthorized_accesses > 0 {
        return ProtectionLevel::Tracked;
    }
    ProtectionLevel::None
}

/// Constants for IP protection
pub const MAX_UNAUTHORIZED_BEFORE_ESCALATION: u32 = 20;
pub const MAX_DISTRIBUTIONS_BEFORE_LOCKDOWN: u32 = 5;
pub const DISTRIBUTION_SEVERITY_THRESHOLD_BPS: u32 = 7_000;
pub const USAGE_TRACKING_RETENTION_SECS: u64 = 7_776_000; // 90 days
pub const ENFORCEMENT_RESPONSE_TIMEOUT_SECS: u64 = 3_600; // 1 hour

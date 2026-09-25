/// Content Licensing Framework
///
/// Implements flexible permission management with granular access control,
/// revenue sharing mechanisms, and transparent license tracking to enable
/// fair compensation for content creators.

use soroban_sdk::{symbol_short as symbol, Address, Env, Symbol, Vec};

/// License type definitions
#[derive(Clone, Debug, PartialEq)]
pub enum LicenseType {
    /// Creator retains all rights, no sharing
    Proprietary = 0,
    /// Limited use with revenue sharing
    RevenueSharingLicense = 1,
    /// Educational use with credits
    EducationalLicense = 2,
    /// Commercial license with fees
    CommercialLicense = 3,
    /// Open access with attribution
    OpenWithAttribution = 4,
}

/// Granular permission types
#[derive(Clone, Debug, PartialEq)]
pub enum Permission {
    View = 0,
    Download = 1,
    Redistribute = 2,
    Modify = 3,
    Commercial = 4,
}

/// License record with terms and conditions
#[derive(Clone, Debug, PartialEq)]
pub struct License {
    pub content_hash: Symbol,
    pub creator: Address,
    pub license_type: LicenseType,
    pub permissions: Vec<Permission>,
    pub licensee: Address,
    pub issue_date: u64,
    pub expiry_date: Option<u64>,
    pub revenue_share_bps: u32, // basis points (0-10000)
    pub usage_limit: Option<u32>, // max uses, or None for unlimited
    pub is_active: bool,
}

/// Revenue sharing configuration
#[derive(Clone, Debug, PartialEq)]
pub struct RevenueShare {
    pub content_hash: Symbol,
    pub creator: Address,
    pub total_revenue: i128,
    pub creator_share_bps: u32,
    pub platform_share_bps: u32,
    pub other_beneficiaries: Vec<Address>,
    pub other_shares_bps: Vec<u32>,
    pub last_settlement: u64,
}

/// License usage record for auditing
#[derive(Clone, Debug, PartialEq)]
pub struct LicenseUsageRecord {
    pub license_id: Symbol,
    pub used_at: u64,
    pub usage_type: Symbol,
    pub revenue_generated: i128,
}

/// Permission management record
#[derive(Clone, Debug, PartialEq)]
pub struct PermissionGrant {
    pub grantee: Address,
    pub permission: Permission,
    pub granted_at: u64,
    pub expires_at: Option<u64>,
    pub revocable: bool,
}

/// Create a new license agreement
pub fn create_license(
    env: &Env,
    content_hash: Symbol,
    creator: Address,
    licensee: Address,
    license_type: LicenseType,
    permissions: Vec<Permission>,
    revenue_share_bps: u32,
    duration_secs: Option<u64>,
    usage_limit: Option<u32>,
) -> License {
    let current_time = env.ledger().timestamp();
    let expiry_date = duration_secs.map(|d| current_time + d);

    License {
        content_hash,
        creator,
        license_type,
        permissions,
        licensee,
        issue_date: current_time,
        expiry_date,
        revenue_share_bps,
        usage_limit,
        is_active: true,
    }
}

/// Check if a user has specific permission for content
pub fn has_permission(
    env: &Env,
    license: &License,
    permission: Permission,
) -> bool {
    // Check expiry
    if let Some(expiry) = license.expiry_date {
        if env.ledger().timestamp() > expiry {
            return false;
        }
    }

    // Check if not active
    if !license.is_active {
        return false;
    }

    // Check permission list
    for perm in license.permissions.iter() {
        if perm == &permission {
            return true;
        }
    }

    false
}

/// Record license usage for billing purposes
pub fn record_license_usage(
    env: &Env,
    license_id: Symbol,
    usage_type: Symbol,
    revenue_generated: i128,
) -> LicenseUsageRecord {
    LicenseUsageRecord {
        license_id,
        used_at: env.ledger().timestamp(),
        usage_type,
        revenue_generated,
    }
}

/// Calculate revenue shares for a given revenue amount
pub fn calculate_revenue_shares(
    env: &Env,
    revenue_share: &RevenueShare,
    total_revenue: i128,
) -> (i128, i128, Vec<i128>) {
    let creator_amount = (total_revenue as u128)
        .saturating_mul(revenue_share.creator_share_bps as u128)
        .saturating_div(10_000) as i128;

    let platform_amount = (total_revenue as u128)
        .saturating_mul(revenue_share.platform_share_bps as u128)
        .saturating_div(10_000) as i128;

    let mut other_amounts: Vec<i128> = Vec::new();
    for share_bps in revenue_share.other_shares_bps.iter() {
        let amount = (total_revenue as u128)
            .saturating_mul(*share_bps as u128)
            .saturating_div(10_000) as i128;
        other_amounts.push(amount);
    }

    (creator_amount, platform_amount, other_amounts)
}

/// Configure revenue sharing for content
pub fn configure_revenue_share(
    env: &Env,
    content_hash: Symbol,
    creator: Address,
    creator_share_bps: u32,
    platform_share_bps: u32,
    other_beneficiaries: Vec<Address>,
    other_shares_bps: Vec<u32>,
) -> RevenueShare {
    // Validate shares sum to 10000 or less
    let total_bps = creator_share_bps
        .saturating_add(platform_share_bps)
        .saturating_add(other_shares_bps.iter().fold(0u32, |acc, &x| acc.saturating_add(x)));

    let adjusted_platform_share = if total_bps > 10_000 {
        platform_share_bps.saturating_sub(total_bps.saturating_sub(10_000))
    } else {
        platform_share_bps
    };

    RevenueShare {
        content_hash,
        creator,
        total_revenue: 0,
        creator_share_bps,
        platform_share_bps: adjusted_platform_share,
        other_beneficiaries,
        other_shares_bps,
        last_settlement: env.ledger().timestamp(),
    }
}

/// Grant specific permission to an address
pub fn grant_permission(
    env: &Env,
    grantee: Address,
    permission: Permission,
    duration_secs: Option<u64>,
    revocable: bool,
) -> PermissionGrant {
    let current_time = env.ledger().timestamp();
    let expires_at = duration_secs.map(|d| current_time + d);

    PermissionGrant {
        grantee,
        permission,
        granted_at: current_time,
        expires_at,
        revocable,
    }
}

/// Validate license is valid and active
pub fn validate_license(
    env: &Env,
    license: &License,
) -> bool {
    let current_time = env.ledger().timestamp();

    // Check active status
    if !license.is_active {
        return false;
    }

    // Check expiry
    if let Some(expiry) = license.expiry_date {
        if current_time > expiry {
            return false;
        }
    }

    // Check that creator and licensee are valid
    if license.creator == license.licensee {
        return false;
    }

    true
}

/// Track cumulative usage against limits
pub fn check_usage_limit(
    current_usage: u32,
    limit: Option<u32>,
) -> bool {
    if let Some(max_uses) = limit {
        current_usage < max_uses
    } else {
        true // Unlimited
    }
}

/// Constants for licensing framework
pub const MIN_REVENUE_SHARE_BPS: u32 = 0;
pub const MAX_REVENUE_SHARE_BPS: u32 = 10_000;
pub const MIN_LICENSE_DURATION_SECS: u64 = 86_400; // 1 day
pub const MAX_LICENSE_DURATION_SECS: u64 = 315_360_000; // 10 years
pub const LICENSE_GRACE_PERIOD_SECS: u64 = 3_600; // 1 hour
pub const DEFAULT_CREATOR_SHARE_BPS: u32 = 7_000; // 70%
pub const DEFAULT_PLATFORM_SHARE_BPS: u32 = 3_000; // 30%

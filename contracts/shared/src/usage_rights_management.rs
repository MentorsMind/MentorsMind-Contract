use soroban_sdk::{contracttype, Address, Env, Symbol, Vec};
use crate::SharedError;

/// Types of licenses that can be granted
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LicenseType {
    View,           // Can only view content
    Download,       // Can download for personal use
    Share,          // Can share with others
    Modify,         // Can modify content
    Commercial,     // Can use for commercial purposes
    Redistribute,   // Can redistribute to others
    Exclusive,      // Exclusive usage rights
}

/// License status
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum LicenseStatus {
    Active,
    Suspended,
    Revoked,
    Expired,
}

/// Violation penalty types
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ViolationPenalty {
    Warning,
    Suspension(u64),    // Suspension duration in seconds
    Fine(i128),         // Financial penalty amount
    Revocation,         // Permanent revocation
    LegalAction,        // Escalate to legal proceedings
}

/// License record
#[contracttype]
#[derive(Clone, Debug)]
pub struct License {
    pub license_id: Symbol,
    pub licensee: Address,
    pub licensor: Address,
    pub content_id: Symbol,
    pub license_types: Vec<LicenseType>,
    pub granted_at: u64,
    pub expires_at: Option<u64>,
    pub max_usage_count: Option<u32>,
    pub current_usage_count: u32,
    pub status: LicenseStatus,
    pub terms_hash: Option<Symbol>,
    pub payment_required: Option<i128>,
    pub payment_token: Option<Address>,
}

/// Usage rights violation record
#[contracttype]
#[derive(Clone, Debug)]
pub struct ViolationRecord {
    pub violation_id: Symbol,
    pub violator: Address,
    pub content_id: Symbol,
    pub license_id: Option<Symbol>,
    pub violation_type: Symbol,
    pub description: Symbol,
    pub detected_at: u64,
    pub penalty_applied: ViolationPenalty,
    pub resolved: bool,
    pub evidence_hash: Option<Symbol>,
}

/// Rights enforcement action
#[contracttype]
#[derive(Clone, Debug)]
pub struct EnforcementAction {
    pub action_id: Symbol,
    pub target: Address,
    pub content_id: Symbol,
    pub action_type: Symbol, // "takedown", "suspend", "fine", "warn"
    pub reason: Symbol,
    pub executed_at: u64,
    pub executor: Address,
    pub status: Symbol, // "pending", "executed", "failed"
}

/// Usage rights management utilities
pub struct UsageRightsManager;

impl UsageRightsManager {
    /// Create a new license
    pub fn create_license(
        env: &Env,
        license_id: Symbol,
        licensee: Address,
        licensor: Address,
        content_id: Symbol,
        license_types: Vec<LicenseType>,
        expires_at: Option<u64>,
        max_usage_count: Option<u32>,
        payment_required: Option<i128>,
        payment_token: Option<Address>,
    ) -> Result<License, SharedError> {
        let now = env.ledger().timestamp();

        // Validate expiration time if provided
        if let Some(exp_time) = expires_at {
            if exp_time <= now {
                return Err(SharedError::InvalidState);
            }
        }

        Ok(License {
            license_id,
            licensee,
            licensor,
            content_id,
            license_types,
            granted_at: now,
            expires_at,
            max_usage_count,
            current_usage_count: 0,
            status: LicenseStatus::Active,
            terms_hash: None,
            payment_required,
            payment_token,
        })
    }

    /// Check if license permits specific usage type
    pub fn check_usage_permission(
        env: &Env,
        license: &License,
        usage_type: &LicenseType,
    ) -> Result<bool, SharedError> {
        let now = env.ledger().timestamp();

        // Check if license is active
        if license.status != LicenseStatus::Active {
            return Ok(false);
        }

        // Check expiration
        if let Some(expires_at) = license.expires_at {
            if now > expires_at {
                return Ok(false);
            }
        }

        // Check usage count limit
        if let Some(max_count) = license.max_usage_count {
            if license.current_usage_count >= max_count {
                return Ok(false);
            }
        }

        // Check if the specific usage type is permitted
        Ok(license.license_types.contains(usage_type))
    }

    /// Record usage and update license
    pub fn record_usage(
        env: &Env,
        license: &mut License,
        usage_type: &LicenseType,
    ) -> Result<(), SharedError> {
        // First check if usage is permitted
        if !Self::check_usage_permission(env, license, usage_type)? {
            return Err(SharedError::UsageRightsViolation);
        }

        // Update usage count
        license.current_usage_count += 1;

        // Check if license should be expired due to usage limit
        if let Some(max_count) = license.max_usage_count {
            if license.current_usage_count >= max_count {
                license.status = LicenseStatus::Expired;
            }
        }

        Ok(())
    }

    /// Enforce license terms and detect violations
    pub fn enforce_license(
        env: &Env,
        license: &License,
        attempted_usage: &LicenseType,
        user: &Address,
    ) -> Result<bool, SharedError> {
        // Check if user is the licensee
        if license.licensee != *user {
            return Err(SharedError::Unauthorized);
        }

        // Check usage permission
        Self::check_usage_permission(env, license, attempted_usage)
    }

    /// Create violation record
    pub fn create_violation_record(
        env: &Env,
        violation_id: Symbol,
        violator: Address,
        content_id: Symbol,
        license_id: Option<Symbol>,
        violation_type: Symbol,
        description: Symbol,
        penalty: ViolationPenalty,
        evidence_hash: Option<Symbol>,
    ) -> ViolationRecord {
        ViolationRecord {
            violation_id,
            violator,
            content_id,
            license_id,
            violation_type,
            description,
            detected_at: env.ledger().timestamp(),
            penalty_applied: penalty,
            resolved: false,
            evidence_hash,
        }
    }

    /// Apply violation penalty
    pub fn apply_penalty(
        env: &Env,
        license: &mut License,
        penalty: &ViolationPenalty,
    ) -> Result<(), SharedError> {
        let now = env.ledger().timestamp();

        match penalty {
            ViolationPenalty::Warning => {
                // Warning doesn't change license status
                Ok(())
            }
            ViolationPenalty::Suspension(duration) => {
                license.status = LicenseStatus::Suspended;
                // Update expiry to current time + suspension duration
                let suspension_end = now + duration;
                license.expires_at = Some(suspension_end);
                Ok(())
            }
            ViolationPenalty::Fine(_amount) => {
                // Fine doesn't directly affect license but could trigger other actions
                // Implementation would depend on payment system integration
                Ok(())
            }
            ViolationPenalty::Revocation => {
                license.status = LicenseStatus::Revoked;
                Ok(())
            }
            ViolationPenalty::LegalAction => {
                // Mark for legal escalation but don't change license immediately
                Ok(())
            }
        }
    }

    /// Suspend license
    pub fn suspend_license(
        _env: &Env,
        license: &mut License,
        suspension_duration: u64,
        suspender: &Address,
    ) -> Result<(), SharedError> {
        // Only licensor can suspend
        if license.licensor != *suspender {
            return Err(SharedError::Unauthorized);
        }

        let now = _env.ledger().timestamp();
        license.status = LicenseStatus::Suspended;
        
        // Set suspension end time
        let suspension_end = now + suspension_duration;
        if let Some(current_expiry) = license.expires_at {
            // If there's already an expiry, use the earlier of the two
            license.expires_at = Some(suspension_end.min(current_expiry));
        } else {
            license.expires_at = Some(suspension_end);
        }

        Ok(())
    }

    /// Revoke license
    pub fn revoke_license(
        _env: &Env,
        license: &mut License,
        revoker: &Address,
    ) -> Result<(), SharedError> {
        // Only licensor can revoke
        if license.licensor != *revoker {
            return Err(SharedError::Unauthorized);
        }

        license.status = LicenseStatus::Revoked;
        Ok(())
    }

    /// Restore suspended license
    pub fn restore_license(
        _env: &Env,
        license: &mut License,
        restorer: &Address,
    ) -> Result<(), SharedError> {
        // Only licensor can restore
        if license.licensor != *restorer {
            return Err(SharedError::Unauthorized);
        }

        // Can only restore suspended licenses
        if license.status != LicenseStatus::Suspended {
            return Err(SharedError::InvalidState);
        }

        license.status = LicenseStatus::Active;
        Ok(())
    }

    /// Check if license is expired
    pub fn is_license_expired(_env: &Env, license: &License) -> bool {
        if let Some(expires_at) = license.expires_at {
            let now = _env.ledger().timestamp();
            now > expires_at
        } else {
            false
        }
    }

    /// Get remaining usage count
    pub fn get_remaining_usage(license: &License) -> Option<u32> {
        if let Some(max_count) = license.max_usage_count {
            Some(max_count.saturating_sub(license.current_usage_count))
        } else {
            None // Unlimited usage
        }
    }

    /// Create enforcement action
    pub fn create_enforcement_action(
        _env: &Env,
        action_id: Symbol,
        target: Address,
        content_id: Symbol,
        action_type: Symbol,
        reason: Symbol,
        executor: Address,
    ) -> EnforcementAction {
        EnforcementAction {
            action_id,
            target,
            content_id,
            action_type,
            reason,
            executed_at: _env.ledger().timestamp(),
            executor,
            status: Symbol::new(_env, "pending"),
        }
    }

    /// Execute takedown procedure
    pub fn execute_takedown(
        _env: &Env,
        action: &mut EnforcementAction,
        executor: &Address,
    ) -> Result<(), SharedError> {
        // Verify executor authorization (should be licensor or admin)
        if action.executor != *executor {
            return Err(SharedError::Unauthorized);
        }

        // Update action status
        action.status = Symbol::new(_env, "executed");
        action.executed_at = _env.ledger().timestamp();

        Ok(())
    }

    /// Validate license terms compliance
    pub fn validate_compliance(
        _env: &Env,
        license: &License,
        usage_history: &Vec<Symbol>, // Simplified usage history
    ) -> Result<bool, SharedError> {
        let now = _env.ledger().timestamp();

        // Check basic license validity
        if license.status != LicenseStatus::Active {
            return Ok(false);
        }

        // Check expiration
        if let Some(expires_at) = license.expires_at {
            if now > expires_at {
                return Ok(false);
            }
        }

        // Check usage count compliance
        if let Some(max_count) = license.max_usage_count {
            if usage_history.len() as u32 > max_count {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Set license terms
    pub fn set_license_terms(
        _env: &Env,
        license: &mut License,
        terms_hash: Symbol,
        setter: &Address,
    ) -> Result<(), SharedError> {
        // Only licensor can set terms
        if license.licensor != *setter {
            return Err(SharedError::Unauthorized);
        }

        license.terms_hash = Some(terms_hash);
        Ok(())
    }
}
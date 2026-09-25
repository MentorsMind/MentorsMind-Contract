use soroban_sdk::{contracttype, Address, BytesN, Env, Symbol, Vec};
use crate::SharedError;

/// Types of intellectual property that can be verified
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IPType {
    Copyright,
    Trademark,
    Patent,
    TradeSecret,
    Curriculum,
    Methodology,
    CourseContent,
}

/// Status of IP ownership verification
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum IPStatus {
    Pending,
    Verified,
    Disputed,
    Revoked,
    Expired,
}

/// Ownership proof for intellectual property
#[contracttype]
#[derive(Clone, Debug)]
pub struct OwnershipProof {
    pub proof_id: Symbol,
    pub ip_type: IPType,
    pub document_hash: BytesN<32>,
    pub registration_number: Option<Symbol>,
    pub jurisdiction: Symbol,
    pub created_at: u64,
    pub expires_at: Option<u64>,
    pub verifier: Address,
}

/// Intellectual property record
#[contracttype]
#[derive(Clone, Debug)]
pub struct IPRecord {
    pub ip_id: Symbol,
    pub owner: Address,
    pub ip_type: IPType,
    pub title: Symbol,
    pub description_hash: BytesN<32>,
    pub ownership_proof: OwnershipProof,
    pub status: IPStatus,
    pub created_at: u64,
    pub last_verified: u64,
    pub usage_count: u32,
    pub authorized_users: Vec<Address>,
    pub license_terms_hash: Option<BytesN<32>>,
}

/// Usage tracking record for IP
#[contracttype]
#[derive(Clone, Debug)]
pub struct IPUsageRecord {
    pub usage_id: Symbol,
    pub ip_id: Symbol,
    pub user: Address,
    pub usage_type: Symbol, // "display", "copy", "modify", "distribute"
    pub usage_timestamp: u64,
    pub context: Symbol, // session_id, course_id, etc.
    pub authorized: bool,
}

/// Infringement detection record
#[contracttype]
#[derive(Clone, Debug)]
pub struct InfringementRecord {
    pub infringement_id: Symbol,
    pub ip_id: Symbol,
    pub alleged_infringer: Address,
    pub detection_method: Symbol, // "automated", "reported", "manual"
    pub evidence_hash: BytesN<32>,
    pub reported_at: u64,
    pub reporter: Address,
    pub status: Symbol, // "open", "investigating", "resolved", "false_positive"
}

/// Intellectual property verification utilities
pub struct IPVerification;

impl IPVerification {
    /// Create a new IP record with ownership proof
    pub fn create_ip_record(
        env: &Env,
        ip_id: Symbol,
        owner: Address,
        ip_type: IPType,
        title: Symbol,
        description_hash: BytesN<32>,
        ownership_proof: OwnershipProof,
    ) -> Result<IPRecord, SharedError> {
        let now = env.ledger().timestamp();
        
        // Verify ownership proof is not expired
        if let Some(expires_at) = ownership_proof.expires_at {
            if now > expires_at {
                return Err(SharedError::IPOwnershipInvalid);
            }
        }

        Ok(IPRecord {
            ip_id,
            owner,
            ip_type,
            title,
            description_hash,
            ownership_proof,
            status: IPStatus::Pending,
            created_at: now,
            last_verified: now,
            usage_count: 0,
            authorized_users: Vec::new(env),
            license_terms_hash: None,
        })
    }

    /// Verify ownership of intellectual property
    pub fn verify_ownership(
        env: &Env,
        ip_record: &mut IPRecord,
        verifier: Address,
    ) -> Result<(), SharedError> {
        let now = env.ledger().timestamp();
        
        // Check if ownership proof is still valid
        if let Some(expires_at) = ip_record.ownership_proof.expires_at {
            if now > expires_at {
                ip_record.status = IPStatus::Expired;
                return Err(SharedError::IPOwnershipInvalid);
            }
        }

        // Update verification status
        ip_record.status = IPStatus::Verified;
        ip_record.last_verified = now;
        ip_record.ownership_proof.verifier = verifier;

        Ok(())
    }

    /// Add usage tracking for IP
    pub fn track_usage(
        _env: &Env,
        ip_id: Symbol,
        user: Address,
        usage_type: Symbol,
        context: Symbol,
        authorized: bool,
    ) -> IPUsageRecord {
        IPUsageRecord {
            usage_id: Symbol::new(_env, "usage"),
            ip_id,
            user,
            usage_type,
            usage_timestamp: _env.ledger().timestamp(),
            context,
            authorized,
        }
    }

    /// Check if user is authorized to use IP
    pub fn is_authorized_user(
        ip_record: &IPRecord,
        user: &Address,
    ) -> bool {
        // Owner is always authorized
        if ip_record.owner == *user {
            return true;
        }

        // Check if user is in authorized list
        ip_record.authorized_users.contains(user)
    }

    /// Authorize a user to use IP
    pub fn authorize_user(
        _env: &Env,
        ip_record: &mut IPRecord,
        user: Address,
        authorizer: &Address,
    ) -> Result<(), SharedError> {
        // Only IP owner can authorize users
        if ip_record.owner != *authorizer {
            return Err(SharedError::Unauthorized);
        }

        // Check if IP is verified
        if ip_record.status != IPStatus::Verified {
            return Err(SharedError::InvalidState);
        }

        // Check if user is already authorized
        if ip_record.authorized_users.contains(&user) {
            return Err(SharedError::DuplicateEntry);
        }

        ip_record.authorized_users.push_back(user);
        Ok(())
    }

    /// Revoke user authorization for IP
    pub fn revoke_authorization(
        _env: &Env,
        ip_record: &mut IPRecord,
        user: &Address,
        revoker: &Address,
    ) -> Result<(), SharedError> {
        // Only IP owner can revoke authorization
        if ip_record.owner != *revoker {
            return Err(SharedError::Unauthorized);
        }

        // Remove user from authorized list
        let mut new_authorized = Vec::new(_env);
        for authorized_user in ip_record.authorized_users.iter() {
            if authorized_user != *user {
                new_authorized.push_back(authorized_user);
            }
        }

        ip_record.authorized_users = new_authorized;
        Ok(())
    }

    /// Create infringement record
    pub fn report_infringement(
        _env: &Env,
        infringement_id: Symbol,
        ip_id: Symbol,
        alleged_infringer: Address,
        evidence_hash: BytesN<32>,
        reporter: Address,
    ) -> InfringementRecord {
        InfringementRecord {
            infringement_id,
            ip_id,
            alleged_infringer,
            detection_method: Symbol::new(_env, "reported"),
            evidence_hash,
            reported_at: _env.ledger().timestamp(),
            reporter,
            status: Symbol::new(_env, "open"),
        }
    }

    /// Detect potential infringement through usage patterns
    pub fn detect_infringement(
        _env: &Env,
        usage_records: &Vec<IPUsageRecord>,
        time_window: u64,
    ) -> Result<Vec<Address>, SharedError> {
        let now = _env.ledger().timestamp();
        let mut suspicious_users = Vec::new(_env);
        let unauthorized_threshold = 5u32; // Suspicious if more than 5 unauthorized uses

        // Count unauthorized usage per user in time window
        for record in usage_records.iter() {
            if now - record.usage_timestamp <= time_window && !record.authorized {
                let mut count = 0u32;
                for other_record in usage_records.iter() {
                    if other_record.user == record.user && 
                       !other_record.authorized &&
                       now - other_record.usage_timestamp <= time_window {
                        count += 1;
                    }
                }

                if count >= unauthorized_threshold && !suspicious_users.contains(&record.user) {
                    suspicious_users.push_back(record.user.clone());
                }
            }
        }

        Ok(suspicious_users)
    }

    /// Update IP record usage statistics
    pub fn update_usage_stats(
        _env: &Env,
        ip_record: &mut IPRecord,
    ) {
        ip_record.usage_count += 1;
    }

    /// Set license terms for IP
    pub fn set_license_terms(
        _env: &Env,
        ip_record: &mut IPRecord,
        license_terms_hash: BytesN<32>,
        setter: &Address,
    ) -> Result<(), SharedError> {
        // Only IP owner can set license terms
        if ip_record.owner != *setter {
            return Err(SharedError::Unauthorized);
        }

        ip_record.license_terms_hash = Some(license_terms_hash);
        Ok(())
    }

    /// Validate IP claims against ownership proof
    pub fn validate_ip_claims(
        _env: &Env,
        ip_record: &IPRecord,
        claimant: &Address,
    ) -> Result<bool, SharedError> {
        // Check if claimant is the recorded owner
        if ip_record.owner != *claimant {
            return Ok(false);
        }

        // Check if IP status is verified
        if ip_record.status != IPStatus::Verified {
            return Ok(false);
        }

        // Check if ownership proof is still valid
        if let Some(expires_at) = ip_record.ownership_proof.expires_at {
            let now = _env.ledger().timestamp();
            if now > expires_at {
                return Ok(false);
            }
        }

        Ok(true)
    }

    /// Check if IP record is expired
    pub fn is_expired(_env: &Env, ip_record: &IPRecord) -> bool {
        if let Some(expires_at) = ip_record.ownership_proof.expires_at {
            let now = _env.ledger().timestamp();
            now > expires_at
        } else {
            false
        }
    }

    /// Dispute IP ownership
    pub fn dispute_ownership(
        _env: &Env,
        ip_record: &mut IPRecord,
        disputer: &Address,
    ) -> Result<(), SharedError> {
        // Cannot dispute own IP
        if ip_record.owner == *disputer {
            return Err(SharedError::InvalidState);
        }

        // Only verified IP can be disputed
        if ip_record.status != IPStatus::Verified {
            return Err(SharedError::InvalidState);
        }

        ip_record.status = IPStatus::Disputed;
        Ok(())
    }
}
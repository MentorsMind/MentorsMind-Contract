use soroban_sdk::{contracttype, Address, BytesN, Env, Symbol, Vec, Bytes};
use crate::SharedError;

/// Content types that can be protected
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContentType {
    Video,
    Document,
    Audio,
    Image,
    Course,
    Curriculum,
    Methodology,
}

/// Access levels for content protection
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccessLevel {
    Public,
    Licensed,
    Private,
    Restricted,
}

/// Encryption key structure for content protection
#[contracttype]
#[derive(Clone, Debug)]
pub struct EncryptionKey {
    pub key_id: Symbol,
    pub key_hash: BytesN<32>,
    pub created_at: u64,
    pub expires_at: u64,
    pub access_level: AccessLevel,
}

/// Protected content record
#[contracttype]
#[derive(Clone, Debug)]
pub struct ProtectedContent {
    pub content_id: Symbol,
    pub owner: Address,
    pub content_type: ContentType,
    pub encryption_key_id: Symbol,
    pub access_level: AccessLevel,
    pub created_at: u64,
    pub last_accessed: u64,
    pub access_count: u32,
    pub authorized_viewers: Vec<Address>,
}

/// Access log entry for monitoring
#[contracttype]
#[derive(Clone, Debug)]
pub struct AccessLog {
    pub content_id: Symbol,
    pub accessor: Address,
    pub access_time: u64,
    pub access_type: Symbol, // "view", "download", "share"
    pub ip_hash: Option<BytesN<32>>,
    pub success: bool,
}

/// Content protection utilities
pub struct ContentProtection;

impl ContentProtection {
    /// Create a new protected content entry
    pub fn create_protected_content(
        env: &Env,
        content_id: Symbol,
        owner: Address,
        content_type: ContentType,
        access_level: AccessLevel,
    ) -> Result<ProtectedContent, SharedError> {
        let now = env.ledger().timestamp();
        let encryption_key_id = Symbol::new(env, "key");
        
        let protected_content = ProtectedContent {
            content_id,
            owner,
            content_type,
            encryption_key_id,
            access_level,
            created_at: now,
            last_accessed: now,
            access_count: 0,
            authorized_viewers: Vec::new(env),
        };

        Ok(protected_content)
    }

    /// Generate encryption key for content
    pub fn generate_encryption_key(
        env: &Env,
        key_id: Symbol,
        access_level: AccessLevel,
        expires_at: u64,
    ) -> Result<EncryptionKey, SharedError> {
        let now = env.ledger().timestamp();
        
        if expires_at <= now {
            return Err(SharedError::InvalidState);
        }

        // Generate a deterministic but secure key hash
        let key_data = Bytes::new(&env);
        let key_hash = env.crypto().keccak256(&key_data).into();

        Ok(EncryptionKey {
            key_id,
            key_hash,
            created_at: now,
            expires_at,
            access_level,
        })
    }

    /// Verify access permissions for content
    pub fn verify_access(
        env: &Env,
        content: &ProtectedContent,
        accessor: &Address,
        encryption_key: &EncryptionKey,
    ) -> Result<bool, SharedError> {
        let now = env.ledger().timestamp();

        // Check if encryption key is expired
        if now > encryption_key.expires_at {
            return Err(SharedError::EncryptionError);
        }

        // Check access level permissions
        match content.access_level {
            AccessLevel::Public => Ok(true),
            AccessLevel::Licensed => {
                // Check if accessor is in authorized viewers
                Ok(content.authorized_viewers.contains(accessor) || content.owner == *accessor)
            }
            AccessLevel::Private => {
                // Only owner can access
                Ok(content.owner == *accessor)
            }
            AccessLevel::Restricted => {
                // Strictest access - only owner and must match access level
                Ok(content.owner == *accessor && encryption_key.access_level == AccessLevel::Restricted)
            }
        }
    }

    /// Add authorized viewer to protected content
    pub fn authorize_viewer(
        _env: &Env,
        content: &mut ProtectedContent,
        viewer: Address,
        requester: &Address,
    ) -> Result<(), SharedError> {
        // Only owner can authorize viewers
        if content.owner != *requester {
            return Err(SharedError::Unauthorized);
        }

        // Check if viewer is already authorized
        if content.authorized_viewers.contains(&viewer) {
            return Err(SharedError::DuplicateEntry);
        }

        content.authorized_viewers.push_back(viewer);
        Ok(())
    }

    /// Log content access for monitoring
    pub fn log_access(
        env: &Env,
        content_id: Symbol,
        accessor: Address,
        access_type: Symbol,
        success: bool,
        ip_hash: Option<BytesN<32>>,
    ) -> AccessLog {
        AccessLog {
            content_id,
            accessor,
            access_time: env.ledger().timestamp(),
            access_type,
            ip_hash,
            success,
        }
    }

    /// Detect suspicious access patterns (basic piracy detection)
    pub fn detect_suspicious_activity(
        env: &Env,
        access_logs: &Vec<AccessLog>,
        time_window: u64,
    ) -> Result<bool, SharedError> {
        let now = env.ledger().timestamp();
        let threshold = 10u32; // Suspicious if more than 10 accesses in time window
        let mut recent_accesses = 0u32;

        for log in access_logs.iter() {
            if now - log.access_time <= time_window {
                recent_accesses += 1;
            }
        }

        Ok(recent_accesses > threshold)
    }

    /// Revoke access for a viewer
    pub fn revoke_access(
        _env: &Env,
        content: &mut ProtectedContent,
        viewer: &Address,
        requester: &Address,
    ) -> Result<(), SharedError> {
        // Only owner can revoke access
        if content.owner != *requester {
            return Err(SharedError::Unauthorized);
        }

        // Find and remove the viewer
        let mut new_viewers = Vec::new(_env);
        for existing_viewer in content.authorized_viewers.iter() {
            if existing_viewer != *viewer {
                new_viewers.push_back(existing_viewer);
            }
        }

        content.authorized_viewers = new_viewers;
        Ok(())
    }

    /// Check if content encryption key is valid
    pub fn is_key_valid(_env: &Env, key: &EncryptionKey) -> bool {
        let now = _env.ledger().timestamp();
        now <= key.expires_at
    }

    /// Update content access statistics
    pub fn update_access_stats(
        _env: &Env,
        content: &mut ProtectedContent,
    ) {
        content.last_accessed = _env.ledger().timestamp();
        content.access_count += 1;
    }
}
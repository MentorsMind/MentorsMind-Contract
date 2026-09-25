//! Recording Integrity with Tamper-Evident Storage and Privacy Protection
//!
//! Implements cryptographic integrity verification, immutable storage,
//! selective redaction, consent management, and access control.

use soroban_sdk::{contracttype, Address, Env, Symbol, Vec, BytesN};

/// Maximum recording size (MB)
pub const MAX_RECORDING_SIZE_MB: u32 = 500;
/// Minimum consent duration (hours)
pub const MIN_CONSENT_DURATION_HOURS: u32 = 1;
/// Default retention period (days)
pub const DEFAULT_RETENTION_DAYS: u32 = 90;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecordingStatus {
    Recording,
    Completed,
    Processing,
    Verified,
    Redacted,
    Expired,
    Deleted,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AccessRole {
    Participant,      // Mentor or learner in session
    Arbitrator,       // Dispute resolution
    PlatformAdmin,    // Platform administration
    Auditor,          // Compliance audit
    Emergency,        // Emergency access
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecording {
    pub recording_id: Symbol,
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub storage_uri: Symbol,      // IPFS CID or storage reference
    pub content_hash: BytesN<32>, // SHA-256 of original content
    pub merkle_root: BytesN<32>,  // Merkle root of chunked content
    pub chunk_hashes: Vec<BytesN<32>>, // Hash of each chunk
    pub size_bytes: u64,
    pub duration_secs: u32,
    pub status: RecordingStatus,
    pub recorded_at: u64,
    pub verified_at: Option<u64>,
    pub expires_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConsentRecord {
    pub recording_id: Symbol,
    pub grantor: Address,         // Who gave consent (mentor or learner)
    pub grantee: Address,         // Who can access (or Address::zero for public)
    pub role: AccessRole,
    pub granted_at: u64,
    pub expires_at: u64,
    pub scope: Symbol,            // "full", "redacted", "metadata_only"
    pub revoked: bool,
    pub revoked_at: Option<u64>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RedactionRecord {
    pub recording_id: Symbol,
    pub redactor: Address,
    pub redaction_type: Symbol,   // "pii", "sensitive_content", "consent_revoked"
    pub start_timestamp: u32,     // Start time in recording (seconds)
    pub end_timestamp: u32,       // End time in recording (seconds)
    pub reason_hash: BytesN<32>,  // Hash of redaction justification
    pub applied_at: u64,
    pub authorized_by: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AccessLogEntry {
    pub recording_id: Symbol,
    pub accessor: Address,
    pub role: AccessRole,
    pub purpose: Symbol,
    pub accessed_at: u64,
    pub granted_by: Address,
    pub consent_record_id: Option<Symbol>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IntegrityVerificationResult {
    pub recording_id: Symbol,
    pub is_intact: bool,
    pub verified_chunks: u32,
    pub total_chunks: u32,
    pub mismatched_chunks: Vec<u32>,
    pub merkle_root_match: bool,
    pub content_hash_match: bool,
    pub verified_at: u64,
    pub verifier: Address,
}

/// Create a new recording with cryptographic integrity
pub fn create_recording(
    env: &Env,
    session_id: &Symbol,
    mentor: &Address,
    learner: &Address,
    storage_uri: Symbol,
    content_hash: BytesN<32>,
    chunk_hashes: &Vec<BytesN<32>>,
    size_bytes: u64,
    duration_secs: u32,
) -> SessionRecording {
    // Compute merkle root from chunk hashes
    let merkle_root = compute_merkle_root(env, chunk_hashes);
    
    let recording_id = Symbol::new(env, "rec_"); // Would use counter in practice
    let now = env.ledger().timestamp();
    let expires_at = now + (DEFAULT_RETENTION_DAYS as u64 * 24 * 3600);
    
    SessionRecording {
        recording_id,
        session_id: session_id.clone(),
        mentor: mentor.clone(),
        learner: learner.clone(),
        storage_uri,
        content_hash,
        merkle_root,
        chunk_hashes: chunk_hashes.clone(),
        size_bytes,
        duration_secs,
        status: RecordingStatus::Completed,
        recorded_at: now,
        verified_at: None,
        expires_at,
    }
}

/// Compute Merkle root from chunk hashes
pub fn compute_merkle_root(env: &Env, chunk_hashes: &Vec<BytesN<32>>) -> BytesN<32> {
    if chunk_hashes.len() == 0 {
        return BytesN::from_array(env, &[0u8; 32]);
    }
    
    let mut current_level: Vec<BytesN<32>> = chunk_hashes.clone();
    
    while current_level.len() > 1 {
        let mut next_level = Vec::new(env);
        let mut i = 0;
        while i < current_level.len() {
            let left = current_level.get(i).unwrap().clone();
            let right = if i + 1 < current_level.len() {
                current_level.get(i + 1).unwrap().clone()
            } else {
                left.clone() // Duplicate last if odd
            };
            
            let mut combined = soroban_sdk::Bytes::new(env);
            combined.append(&left.into());
            combined.append(&right.into());
            let parent_hash = env.crypto().sha256(&combined).into();
            next_level.push_back(parent_hash);
            i += 2;
        }
        current_level = next_level;
    }
    
    current_level.get(0).unwrap()
}

/// Verify recording integrity
pub fn verify_recording_integrity(
    env: &Env,
    recording: &SessionRecording,
    provided_chunk_hashes: &Vec<BytesN<32>>,
    provided_content_hash: BytesN<32>,
    verifier: &Address,
) -> IntegrityVerificationResult {
    let mut mismatched = Vec::new(env);
    let mut verified = 0u32;
    
    // Verify chunk hashes
    let min_len = recording.chunk_hashes.len().min(provided_chunk_hashes.len());
    for i in 0..min_len {
        if recording.chunk_hashes.get(i).unwrap() == provided_chunk_hashes.get(i).unwrap() {
            verified = verified.saturating_add(1);
        } else {
            mismatched.push_back(i);
        }
    }
    
    // Verify merkle root
    let computed_merkle = compute_merkle_root(env, provided_chunk_hashes);
    let merkle_match = computed_merkle == recording.merkle_root;
    
    // Verify content hash
    let content_match = provided_content_hash == recording.content_hash;
    
    let is_intact = merkle_match && content_match && mismatched.len() == 0 && 
                   min_len == recording.chunk_hashes.len();
    
    IntegrityVerificationResult {
        recording_id: recording.recording_id.clone(),
        is_intact,
        verified_chunks: verified,
        total_chunks: recording.chunk_hashes.len(),
        mismatched_chunks: mismatched,
        merkle_root_match: merkle_match,
        content_hash_match: content_match,
        verified_at: env.ledger().timestamp(),
        verifier: verifier.clone(),
    }
}

/// Grant consent for recording access
pub fn grant_consent(
    env: &Env,
    recording_id: &Symbol,
    grantor: &Address,
    grantee: &Address,
    role: AccessRole,
    duration_hours: u32,
    scope: Symbol,
) -> ConsentRecord {
    let now = env.ledger().timestamp();
    let duration = duration_hours.max(MIN_CONSENT_DURATION_HOURS);
    let expires_at = now + (duration as u64 * 3600);
    
    ConsentRecord {
        recording_id: recording_id.clone(),
        grantor: grantor.clone(),
        grantee: grantee.clone(),
        role,
        granted_at: now,
        expires_at,
        scope,
        revoked: false,
        revoked_at: None,
    }
}

/// Revoke consent
pub fn revoke_consent(
    env: &Env,
    consent: &mut ConsentRecord,
    revoker: &Address,
) -> bool {
    // Only grantor or platform admin can revoke
    if consent.grantor != *revoker {
        return false;
    }
    
    if consent.revoked {
        return false;
    }
    
    consent.revoked = true;
    consent.revoked_at = Some(env.ledger().timestamp());
    true
}

/// Check if access is authorized
pub fn check_access_authorized(
    env: &Env,
    recording: &SessionRecording,
    consent_records: &Vec<ConsentRecord>,
    accessor: &Address,
    role: AccessRole,
) -> bool {
    // Participants always have access to their own recordings
    if recording.mentor == *accessor || recording.learner == *accessor {
        return true;
    }
    
    // Check consent records
    for consent in consent_records.iter() {
        if consent.recording_id == recording.recording_id 
            && consent.grantee == *accessor
            && consent.role == role
            && !consent.revoked
            && consent.expires_at > env.ledger().timestamp() {
            return true;
        }
    }
    
    // Platform admins and auditors have broad access
    if role == AccessRole::PlatformAdmin || role == AccessRole::Auditor {
        return true;
    }
    
    // Emergency access requires special authorization
    if role == AccessRole::Emergency {
        // Would check emergency authorization in practice
        return false;
    }
    
    false
}

/// Apply redaction to recording
pub fn apply_redaction(
    env: &Env,
    recording_id: &Symbol,
    redactor: &Address,
    redaction_type: Symbol,
    start_ts: u32,
    end_ts: u32,
    reason_hash: BytesN<32>,
    authorized_by: &Address,
) -> RedactionRecord {
    RedactionRecord {
        recording_id: recording_id.clone(),
        redactor: redactor.clone(),
        redaction_type,
        start_timestamp: start_ts,
        end_timestamp: end_ts,
        reason_hash,
        applied_at: env.ledger().timestamp(),
        authorized_by: authorized_by.clone(),
    }
}

/// Log access for audit trail
pub fn log_access(
    env: &Env,
    recording_id: &Symbol,
    accessor: &Address,
    role: AccessRole,
    purpose: Symbol,
    granted_by: &Address,
    consent_record_id: Option<Symbol>,
) -> AccessLogEntry {
    AccessLogEntry {
        recording_id: recording_id.clone(),
        accessor: accessor.clone(),
        role,
        purpose,
        accessed_at: env.ledger().timestamp(),
        granted_by: granted_by.clone(),
        consent_record_id,
    }
}

/// Emergency privacy protection - auto-redact and revoke consent
pub fn emergency_privacy_protection(
    env: &Env,
    recording_id: &Symbol,
    reason_hash: BytesN<32>,
    authorized_by: &Address,
) -> (RedactionRecord, Vec<ConsentRecord>) {
    // Full redaction
    let redaction = apply_redaction(
        env,
        recording_id,
        authorized_by,
        Symbol::new(env, "emergency_privacy"),
        0,
        u32::MAX,
        reason_hash,
        authorized_by,
    );
    
    // Revoke all consents
    let revoked_consents = Vec::new(env);
    // In practice, would load and revoke all consents for this recording
    
    (redaction, revoked_consents)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env, Symbol, BytesN};

    fn create_test_chunk_hashes(env: &Env, count: u32) -> Vec<BytesN<32>> {
        let mut chunks = Vec::new(env);
        for i in 0..count {
            let mut bytes = [0u8; 32];
            bytes[0] = i as u8;
            bytes[1] = (i >> 8) as u8;
            chunks.push_back(BytesN::from_array(env, &bytes));
        }
        chunks
    }

    #[test]
    fn test_compute_merkle_root() {
        let env = Env::default();
        let chunks = create_test_chunk_hashes(&env, 4);
        let root = compute_merkle_root(&env, &chunks);
        
        // Root should be deterministic
        let root2 = compute_merkle_root(&env, &chunks);
        assert_eq!(root, root2);
    }

    #[test]
    fn test_merkle_root_different_chunks() {
        let env = Env::default();
        let chunks1 = create_test_chunk_hashes(&env, 4);
        let chunks2 = create_test_chunk_hashes(&env, 4);
        
        // Modify one chunk
        let mut bytes = [0u8; 32];
        bytes[0] = 99;
        let mut modified = Vec::new(&env);
        modified.push_back(BytesN::from_array(&env, &bytes));
        for i in 1..4 {
            modified.push_back(chunks2.get(i).unwrap());
        }
        
        let root1 = compute_merkle_root(&env, &chunks1);
        let root2 = compute_merkle_root(&env, &modified);
        assert_ne!(root1, root2);
    }

    #[test]
    fn test_verify_recording_integrity_valid() {
        let env = Env::default();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "session1");
        let chunks = create_test_chunk_hashes(&env, 4);
        let content_hash = chunks.get(0).unwrap(); // Simplified
        
        let recording = create_recording(
            &env,
            &session_id,
            &mentor,
            &learner,
            Symbol::new(&env, "ipfs_cid_test"),
            content_hash.clone(),
            &chunks,
            1000000,
            3600,
        );
        
        let verifier = Address::generate(&env);
        let result = verify_recording_integrity(&env, &recording, &chunks, content_hash, &verifier);
        
        assert!(result.is_intact);
        assert!(result.merkle_root_match);
        assert!(result.content_hash_match);
        assert_eq!(result.verified_chunks, 4);
    }

    #[test]
    fn test_verify_recording_integrity_tampered() {
        let env = Env::default();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let session_id = Symbol::new(&env, "session2");
        let chunks = create_test_chunk_hashes(&env, 4);
        let content_hash = chunks.get(0).unwrap();
        
        let recording = create_recording(
            &env,
            &session_id,
            &mentor,
            &learner,
            Symbol::new(&env, "ipfs_cid_test"),
            content_hash.clone(),
            &chunks,
            1000000,
            3600,
        );
        
        // Tamper with one chunk
        let mut tampered = chunks.clone();
        let mut bytes = [0u8; 32];
        bytes[0] = 99;
        tampered.set(1, BytesN::from_array(&env, &bytes));
        
        let verifier = Address::generate(&env);
        let result = verify_recording_integrity(&env, &recording, &tampered, content_hash, &verifier);
        
        assert!(!result.is_intact);
        assert!(!result.merkle_root_match);
        assert_eq!(result.mismatched_chunks.len(), 1);
    }

    #[test]
    fn test_grant_and_revoke_consent() {
        let env = Env::default();
        let recording_id = Symbol::new(&env, "rec1");
        let grantor = Address::generate(&env);
        let grantee = Address::generate(&env);
        
        let mut consent = grant_consent(
            &env,
            &recording_id,
            &grantor,
            &grantee,
            AccessRole::Participant,
            24,
            Symbol::new(&env, "full"),
        );
        
        assert!(!consent.revoked);
        assert_eq!(consent.grantor, grantor);
        assert_eq!(consent.grantee, grantee);
        
        // Revoke
        let revoked = revoke_consent(&env, &mut consent, &grantor);
        assert!(revoked);
        assert!(consent.revoked);
        assert!(consent.revoked_at.is_some());
        
        // Can't revoke twice
        let revoked2 = revoke_consent(&env, &mut consent, &grantor);
        assert!(!revoked2);
    }

    #[test]
    fn test_check_access_authorized_participant() {
        let env = Env::default();
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let third_party = Address::generate(&env);
        
        let recording = SessionRecording {
            recording_id: Symbol::new(&env, "rec1"),
            session_id: Symbol::new(&env, "sess1"),
            mentor: mentor.clone(),
            learner: learner.clone(),
            storage_uri: Symbol::new(&env, "ipfs_cid_test"),
            content_hash: BytesN::from_array(&env, &[0u8; 32]),
            merkle_root: BytesN::from_array(&env, &[0u8; 32]),
            chunk_hashes: Vec::new(&env),
            size_bytes: 1000,
            duration_secs: 3600,
            status: RecordingStatus::Verified,
            recorded_at: env.ledger().timestamp(),
            verified_at: Some(env.ledger().timestamp()),
            expires_at: env.ledger().timestamp() + 86400,
        };
        
        let consents = Vec::new(&env);
        
        // Mentor should have access
        assert!(check_access_authorized(&env, &recording, &consents, &mentor, AccessRole::Participant));
        
        // Learner should have access
        assert!(check_access_authorized(&env, &recording, &consents, &learner, AccessRole::Participant));
        
        // Third party should not
        assert!(!check_access_authorized(&env, &recording, &consents, &third_party, AccessRole::Participant));
    }

    #[test]
    fn test_emergency_privacy_protection() {
        let env = Env::default();
        let recording_id = Symbol::new(&env, "rec_emergency");
        let authorized_by = Address::generate(&env);
        let mut reason_bytes = [0u8; 32];
        reason_bytes[0] = 1;
        
        let (redaction, _revoked) = emergency_privacy_protection(
            &env,
            &recording_id,
            BytesN::from_array(&env, &reason_bytes),
            &authorized_by,
        );
        
        assert_eq!(redaction.recording_id, recording_id);
        assert_eq!(redaction.redaction_type, Symbol::new(&env, "emergency_privacy"));
        assert_eq!(redaction.start_timestamp, 0);
        assert_eq!(redaction.end_timestamp, u32::MAX);
    }
}
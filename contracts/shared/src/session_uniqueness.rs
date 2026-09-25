//! Session Uniqueness, Replay Prevention, and Content Integrity (#905)
//!
//! Protects against attackers replaying recorded session data to fake session completion,
//! manipulating session content, or bypassing verification systems.

use soroban_sdk::{contracttype, Address, BytesN, Env, Symbol, Vec};

/// Maximum allowable time drift (seconds) for real-time session verification
pub const MAX_SESSION_TIME_DRIFT_SECS: u64 = 300; // 5 minutes

/// Threshold above which confidence indicates a replay attack (basis points)
pub const REPLAY_CONFIDENCE_THRESHOLD_BPS: u32 = 7500; // 75%

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionNonceRecord {
    pub session_id: Symbol,
    pub nonce: u64,
    pub generated_at: u64,
    pub used: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContentIntegrityRecord {
    pub content_hash: BytesN<32>,
    pub chunk_root: BytesN<32>,
    pub timestamp: u64,
    pub verified: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReplayDetectionResult {
    pub is_replay: bool,
    pub original_session_id: Option<Symbol>,
    pub temporal_delta_secs: u64,
    pub confidence_score: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionAuditRecord {
    pub session_id: Symbol,
    pub integrity_score: u32,
    pub is_tampered: bool,
    pub verified_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SessionRecoveryRecord {
    pub session_id: Symbol,
    pub restored_content_hash: BytesN<32>,
    pub recovery_timestamp: u64,
    pub success: bool,
}

/// Validates session nonce uniqueness and state.
pub fn validate_session_nonce(nonce: u64, expected_nonce: u64, is_used: bool) -> bool {
    !is_used && nonce == expected_nonce && nonce > 0
}

/// Verifies content hash against expected cryptographic checksum.
pub fn verify_content_checksum(hash: &BytesN<32>, expected: &BytesN<32>) -> bool {
    hash == expected
}

/// Analyzes timestamps to detect potential replay attacks based on temporal anomaly.
pub fn detect_temporal_replay(
    session_timestamp: u64,
    current_timestamp: u64,
    max_drift_secs: u64,
) -> ReplayDetectionResult {
    let drift = if current_timestamp >= session_timestamp {
        current_timestamp - session_timestamp
    } else {
        session_timestamp - current_timestamp
    };

    let is_replay = drift > max_drift_secs;
    let confidence_score = if is_replay {
        if drift > max_drift_secs * 5 {
            9500
        } else {
            8000
        }
    } else {
        1000
    };

    ReplayDetectionResult {
        is_replay,
        original_session_id: None,
        temporal_delta_secs: drift,
        confidence_score,
    }
}

/// Audits session integrity combining content hash match and nonce status.
pub fn audit_session_integrity(
    content_hash: &BytesN<32>,
    expected_hash: &BytesN<32>,
    nonce_valid: bool,
    timestamp: u64,
    session_id: Symbol,
) -> SessionAuditRecord {
    let hash_matches = verify_content_checksum(content_hash, expected_hash);
    let is_tampered = !hash_matches || !nonce_valid;
    let integrity_score = if !is_tampered {
        10000
    } else if hash_matches {
        5000
    } else {
        0
    };

    SessionAuditRecord {
        session_id,
        integrity_score,
        is_tampered,
        verified_at: timestamp,
    }
}

/// Recovers session content using a verified backup hash.
pub fn recover_session_content(
    session_id: Symbol,
    backup_hash: BytesN<32>,
    timestamp: u64,
) -> SessionRecoveryRecord {
    SessionRecoveryRecord {
        session_id,
        restored_content_hash: backup_hash,
        recovery_timestamp: timestamp,
        success: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn test_validate_session_nonce() {
        assert!(validate_session_nonce(42, 42, false));
        assert!(!validate_session_nonce(42, 42, true));
        assert!(!validate_session_nonce(42, 99, false));
        assert!(!validate_session_nonce(0, 0, false));
    }

    #[test]
    fn test_verify_content_checksum() {
        let env = Env::default();
        let hash1 = env.crypto().sha256(&soroban_sdk::Bytes::from_slice(&env, b"test content 1")).into();
        let hash2 = env.crypto().sha256(&soroban_sdk::Bytes::from_slice(&env, b"test content 2")).into();
        assert!(verify_content_checksum(&hash1, &hash1));
        assert!(!verify_content_checksum(&hash1, &hash2));
    }

    #[test]
    fn test_detect_temporal_replay() {
        let res_valid = detect_temporal_replay(1000, 1100, 300);
        assert!(!res_valid.is_replay);

        let res_replay = detect_temporal_replay(1000, 2000, 300);
        assert!(res_replay.is_replay);
        assert!(res_replay.confidence_score >= REPLAY_CONFIDENCE_THRESHOLD_BPS);
    }

    #[test]
    fn test_audit_session_integrity() {
        let env = Env::default();
        let hash = env.crypto().sha256(&soroban_sdk::Bytes::from_slice(&env, b"audit")).into();
        let sid = Symbol::new(&env, "session_1");

        let audit = audit_session_integrity(&hash, &hash, true, 1000, sid.clone());
        assert!(!audit.is_tampered);
        assert_eq!(audit.integrity_score, 10000);
    }
}

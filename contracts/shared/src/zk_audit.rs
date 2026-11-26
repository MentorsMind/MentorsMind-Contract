//! Minimal audit-commitment primitive (first increment toward #872).
//!
//! This is deliberately narrow in scope: it provides a cryptographic
//! commitment to an audit entry's data (a SHA-256 hash) so the entry's
//! existence and integrity can be recorded and later verified on-chain
//! without storing the underlying sensitive data itself. It is a real,
//! usable building block for selective disclosure (the pre-image is
//! revealed only to parties who need it, off-chain or via a separate
//! authorized call) — it is NOT a zero-knowledge proof system, threshold
//! decryption scheme, or automated compliance reporting pipeline. Those
//! remain future work; this establishes the commitment primitive they
//! would build on.
//!
//! Security notes:
//! - **Hiding**: a plain `sha256(data)` commitment is not hiding when
//!   `data` is low-entropy or drawn from a small/guessable set (e.g. one
//!   of a handful of audit event types) — it can be brute-forced by
//!   hashing every candidate. `commit_audit_entry` therefore requires a
//!   caller-supplied random blinding `nonce` and hashes `nonce || data`,
//!   so the commitment reveals nothing without the nonce, regardless of
//!   how predictable `data` is. Callers must generate `nonce` with a
//!   real source of randomness and keep it secret until disclosure.
//! - **Binding to context**: the hash also includes `entry_id`, so a
//!   commitment recorded for one audit entry can't be silently replayed
//!   or substituted as the commitment for a different entry — integrity
//!   is bound to a specific identifier, not just to `data` in isolation.

use soroban_sdk::{contracttype, Bytes, BytesN, Env};

/// A commitment to an audit entry: a hash binding `entry_id`, a random
/// `nonce`, and the entry's data together, so the commitment reveals
/// nothing about the underlying data and can't be reattributed to a
/// different entry.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuditCommitment {
    /// Identifier of the audit entry this commitment is bound to.
    pub entry_id: BytesN<32>,
    /// SHA-256 hash of `entry_id || nonce || data`.
    pub commitment: BytesN<32>,
    /// Ledger timestamp the commitment was created.
    pub committed_at: u64,
}

fn commitment_hash(env: &Env, entry_id: &BytesN<32>, nonce: &BytesN<32>, data: &Bytes) -> BytesN<32> {
    let mut preimage = Bytes::new(env);
    preimage.append(&Bytes::from_array(env, &entry_id.to_array()));
    preimage.append(&Bytes::from_array(env, &nonce.to_array()));
    preimage.append(data);
    env.crypto().sha256(&preimage).into()
}

/// Commit to an audit entry's data without revealing it on-chain.
/// `nonce` must be freshly random per commitment and kept secret until
/// disclosure — reusing a nonce across commitments, or deriving it
/// deterministically from `data`, defeats the hiding property.
pub fn commit_audit_entry(
    env: &Env,
    entry_id: &BytesN<32>,
    nonce: &BytesN<32>,
    data: &Bytes,
) -> AuditCommitment {
    AuditCommitment {
        entry_id: entry_id.clone(),
        commitment: commitment_hash(env, entry_id, nonce, data),
        committed_at: env.ledger().timestamp(),
    }
}

/// Verify that `nonce` and `data` are the pre-image of a previously
/// recorded `commitment` for its bound `entry_id` — the selective-
/// disclosure check a party would run once given access to the
/// underlying data and nonce out-of-band.
pub fn verify_audit_commitment(
    env: &Env,
    commitment: &AuditCommitment,
    nonce: &BytesN<32>,
    data: &Bytes,
) -> bool {
    commitment_hash(env, &commitment.entry_id, nonce, data) == commitment.commitment
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Ledger;

    fn filled(env: &Env, byte: u8) -> BytesN<32> {
        BytesN::from_array(env, &[byte; 32])
    }

    fn payload(env: &Env, text: &[u8]) -> Bytes {
        Bytes::from_slice(env, text)
    }

    /// A valid commitment together with the pre-image that opens it.
    fn committed(env: &Env) -> (BytesN<32>, BytesN<32>, Bytes, AuditCommitment) {
        let entry_id = filled(env, 1);
        let nonce = filled(env, 2);
        let data = payload(env, b"session-42:payout-released");
        let commitment = commit_audit_entry(env, &entry_id, &nonce, &data);
        (entry_id, nonce, data, commitment)
    }

    #[test]
    fn valid_preimage_verifies() {
        let env = Env::default();
        let (_, nonce, data, commitment) = committed(&env);
        assert!(verify_audit_commitment(&env, &commitment, &nonce, &data));
    }

    #[test]
    fn commitment_is_deterministic() {
        let env = Env::default();
        let (entry_id, nonce, data, commitment) = committed(&env);
        let again = commit_audit_entry(&env, &entry_id, &nonce, &data);
        assert_eq!(commitment.commitment, again.commitment);
    }

    #[test]
    fn rejects_altered_data() {
        let env = Env::default();
        let (_, nonce, _, commitment) = committed(&env);
        let forged = payload(&env, b"session-42:payout-refunded");
        assert!(!verify_audit_commitment(&env, &commitment, &nonce, &forged));
    }

    #[test]
    fn rejects_wrong_nonce() {
        let env = Env::default();
        let (_, _, data, commitment) = committed(&env);
        assert!(!verify_audit_commitment(&env, &commitment, &filled(&env, 3), &data));
    }

    #[test]
    fn rejects_tampered_commitment() {
        let env = Env::default();
        let (_, nonce, data, commitment) = committed(&env);
        let tampered = AuditCommitment { commitment: filled(&env, 0xFF), ..commitment };
        assert!(!verify_audit_commitment(&env, &tampered, &nonce, &data));
    }

    #[test]
    fn rejects_commitment_reattributed_to_another_entry() {
        let env = Env::default();
        let (_, nonce, data, commitment) = committed(&env);
        let moved = AuditCommitment { entry_id: filled(&env, 9), ..commitment };
        assert!(!verify_audit_commitment(&env, &moved, &nonce, &data));
    }

    #[test]
    fn never_accepts_trivial_forged_commitments() {
        let env = Env::default();
        let (entry_id, nonce, data, _) = committed(&env);
        // An all-zero, all-one or otherwise made-up commitment must not verify,
        // whatever pre-image accompanies it.
        for byte in [0x00u8, 0x01, 0x7F, 0xFF] {
            let forged = AuditCommitment {
                entry_id: entry_id.clone(),
                commitment: filled(&env, byte),
                committed_at: 0,
            };
            assert!(!verify_audit_commitment(&env, &forged, &nonce, &data));
            assert!(!verify_audit_commitment(&env, &forged, &filled(&env, byte), &Bytes::new(&env)));
        }
    }

    #[test]
    fn empty_data_only_opens_its_own_commitment() {
        let env = Env::default();
        let entry_id = filled(&env, 1);
        let nonce = filled(&env, 2);
        let empty = Bytes::new(&env);
        let commitment = commit_audit_entry(&env, &entry_id, &nonce, &empty);
        assert!(verify_audit_commitment(&env, &commitment, &nonce, &empty));
        assert!(!verify_audit_commitment(&env, &commitment, &nonce, &payload(&env, b"x")));
    }

    #[test]
    fn different_nonces_hide_identical_data() {
        let env = Env::default();
        let entry_id = filled(&env, 1);
        let data = payload(&env, b"dispute_opened");
        let first = commit_audit_entry(&env, &entry_id, &filled(&env, 2), &data);
        let second = commit_audit_entry(&env, &entry_id, &filled(&env, 3), &data);
        assert_ne!(first.commitment, second.commitment);
    }

    #[test]
    fn records_the_ledger_timestamp() {
        let env = Env::default();
        env.ledger().with_mut(|l| l.timestamp = 1_700_000_000);
        let (_, _, _, commitment) = committed(&env);
        assert_eq!(commitment.committed_at, 1_700_000_000);
    }
}

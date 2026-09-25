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

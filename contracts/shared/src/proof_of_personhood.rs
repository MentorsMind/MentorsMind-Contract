//! Minimal proof-of-personhood attestation primitive (first increment
//! toward #873).
//!
//! This is not biometric verification or ML-based sybil detection —
//! it's the simplest honest building block those would sit on top of:
//! a time-bounded attestation issued by a trusted attester, which
//! reputation/governance logic can require before granting full
//! voting weight. Replacing the attester with a decentralized
//! biometric/social-graph pipeline is future work.
//!
//! Security note: a `PersonhoodAttestation` is plain data — anyone can
//! construct one off-chain claiming any `attester`. Two things are
//! required to make it trustworthy: (1) attestations must only be
//! created via `create_attestation`, which requires the claimed
//! attester's on-chain authorization at creation time, and the result
//! persisted in contract storage keyed by `subject` so it can't be
//! swapped for a forged copy later; (2) every check of an attestation
//! must call `is_attestation_valid` with the caller's own expected/
//! trusted attester address — checking expiry alone is not sufficient,
//! since that would accept a validly-shaped but untrusted attester.

use soroban_sdk::{contracttype, Address, Env};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PersonhoodAttestation {
    /// The account this attestation vouches for.
    pub subject: Address,
    /// The trusted party that issued the attestation.
    pub attester: Address,
    /// Ledger timestamp the attestation was issued.
    pub issued_at: u64,
    /// Ledger timestamp after which the attestation must be renewed.
    pub expires_at: u64,
}

/// Create an attestation, binding it to the attester's on-chain
/// authorization for this exact call — the only way an attestation
/// should come into existence. Callers must persist the result in
/// contract storage keyed by `subject`; never accept a
/// `PersonhoodAttestation` passed in directly from an untrusted caller
/// as if it were already verified.
pub fn create_attestation(
    env: &Env,
    attester: Address,
    subject: Address,
    ttl_secs: u64,
) -> PersonhoodAttestation {
    attester.require_auth();
    let now = env.ledger().timestamp();
    PersonhoodAttestation {
        subject,
        attester,
        issued_at: now,
        expires_at: now + ttl_secs,
    }
}

/// Whether an attestation is currently valid: not expired, AND issued by
/// the specific `expected_attester` the caller trusts. Checking expiry
/// alone would accept an attestation "issued" by an arbitrary, untrusted
/// address — the caller must always supply the attester it actually
/// trusts for this check to mean anything.
pub fn is_attestation_valid(
    env: &Env,
    attestation: &PersonhoodAttestation,
    expected_attester: &Address,
) -> bool {
    attestation.attester == *expected_attester && env.ledger().timestamp() < attestation.expires_at
}

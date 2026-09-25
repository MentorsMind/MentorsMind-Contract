//! Cryptographic algorithm agility primitive (first increment toward #874).
//!
//! This does not implement post-quantum cryptography itself (lattice/code/
//! isogeny-based schemes are out of scope for a single increment and need
//! dedicated audited libraries). It provides the algorithm-agility
//! mechanism a future migration would plug into: a stable algorithm id
//! stored alongside a signature so verification logic can branch on it,
//! and reject unsupported ids explicitly instead of assuming Ed25519.

#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SignatureAlgorithm {
    /// Current classical scheme in production use.
    Ed25519 = 1,
    /// Reserved id for a future post-quantum scheme; not yet implemented.
    PostQuantumReserved = 2,
}

/// Whether verification logic currently has a real implementation for this
/// algorithm id. Only `Ed25519` is active today; `PostQuantumReserved` is a
/// placeholder slot so callers can be upgraded to check this instead of
/// hardcoding a single scheme.
pub fn is_algorithm_supported(alg: SignatureAlgorithm) -> bool {
    matches!(alg, SignatureAlgorithm::Ed25519)
}

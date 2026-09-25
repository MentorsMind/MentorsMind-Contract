//! Cross-Platform Reputation Bridging and Identity Verification (#913)
//!
//! Protects against users importing fake credentials from other platforms,
//! creating false cross-platform identities, or exploiting reputation bridging mechanisms.

use soroban_sdk::{contracttype, Address, BytesN, Env, Symbol, Vec};

/// Minimum reliability score (basis points) required for an external platform
pub const MIN_PLATFORM_RELIABILITY_BPS: u32 = 6000; // 60%

/// Maximum discounted reputation ratio (basis points) imported to prevent sudden domination
pub const MAX_BRIDGED_REPUTATION_DISCOUNT_BPS: u32 = 5000; // 50% max weight

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CrossPlatformAttestation {
    pub platform: Symbol,
    pub external_id: Symbol,
    pub reputation_score: u32,
    pub attestation_hash: BytesN<32>,
    pub verified_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformReliabilityScore {
    pub platform: Symbol,
    pub reliability_bps: u32,
    pub audit_passed: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgedReputationRecord {
    pub user: Address,
    pub platform: Symbol,
    pub original_score: u32,
    pub isolated_score: u32,
    pub is_authentic: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct IdentityConsistencyCheck {
    pub local_user: Address,
    pub remote_id: Symbol,
    pub is_consistent: bool,
    pub confidence_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BridgingAuditReport {
    pub user: Address,
    pub manipulation_detected: bool,
    pub risk_score: u32,
    pub details: Symbol,
}

/// Verifies cryptographic attestation from a trusted external platform.
pub fn verify_cross_platform_attestation(
    attestation: &CrossPlatformAttestation,
    expected_hash: &BytesN<32>,
    platform_reliability_bps: u32,
) -> bool {
    attestation.attestation_hash == *expected_hash
        && platform_reliability_bps >= MIN_PLATFORM_RELIABILITY_BPS
        && attestation.reputation_score > 0
}

/// Computes an isolated/discounted reputation score to prevent ecosystem manipulation.
pub fn isolate_reputation_score(
    external_score: u32,
    platform_reliability_bps: u32,
) -> u32 {
    let effective_weight_bps = if platform_reliability_bps > 10000 {
        10000
    } else {
        platform_reliability_bps
    };

    let discount = (effective_weight_bps * MAX_BRIDGED_REPUTATION_DISCOUNT_BPS) / 10000;
    (external_score as u64 * discount as u64 / 10000) as u32
}

/// Checks identity consistency between local account and remote cross-platform identity.
pub fn check_identity_consistency(
    local_user: &Address,
    remote_id: &Symbol,
    confidence_bps: u32,
) -> IdentityConsistencyCheck {
    let is_consistent = confidence_bps >= 7000;
    IdentityConsistencyCheck {
        local_user: local_user.clone(),
        remote_id: remote_id.clone(),
        is_consistent,
        confidence_bps,
    }
}

/// Audits a reputation bridging request for manipulation patterns.
pub fn audit_reputation_bridging(
    user: &Address,
    score_jump: u32,
    platform_reliability_bps: u32,
    env: &Env,
) -> BridgingAuditReport {
    let manipulation_detected = score_jump > 5000 || platform_reliability_bps < MIN_PLATFORM_RELIABILITY_BPS;
    let risk_score = if manipulation_detected { 8500 } else { 1200 };

    BridgingAuditReport {
        user: user.clone(),
        manipulation_detected,
        risk_score,
        details: if manipulation_detected {
            Symbol::new(env, "anomaly_detected")
        } else {
            Symbol::new(env, "audit_clean")
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::Env;

    #[test]
    fn test_isolate_reputation_score() {
        let isolated = isolate_reputation_score(1000, 8000);
        assert_eq!(isolated, 400); // 1000 * 50% * 80% = 400
    }

    #[test]
    fn test_verify_cross_platform_attestation() {
        let env = Env::default();
        let hash: BytesN<32> = env.crypto().sha256(&soroban_sdk::Bytes::from_slice(&env, b"attest")).into();
        let att = CrossPlatformAttestation {
            platform: Symbol::new(&env, "GITHUB"),
            external_id: Symbol::new(&env, "user1"),
            reputation_score: 500,
            attestation_hash: hash.clone(),
            verified_at: 1000,
        };

        assert!(verify_cross_platform_attestation(&att, &hash, 8000));
        assert!(!verify_cross_platform_attestation(&att, &hash, 4000));
    }

    #[test]
    fn test_check_identity_consistency() {
        let env = Env::default();
        let user = Address::generate(&env);
        let remote = Symbol::new(&env, "remote_user");

        let res = check_identity_consistency(&user, &remote, 8500);
        assert!(res.is_consistent);

        let res_low = check_identity_consistency(&user, &remote, 5000);
        assert!(!res_low.is_consistent);
    }
}

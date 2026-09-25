//! Emergency rollback authorization, scope limits, and immutable audit helpers.
//!
//! Rollbacks require **both**:
//! 1. Technical 4-of-7 emergency multisig approval, and
//! 2. Governance community review (48-hour minimum) with a passed vote.
//!
//! Rollback scope is limited to snapshots taken within the last 24 hours.
//! Audit and security records are archived before any state restoration so
//! they survive partial rollback execution.

use soroban_sdk::{contracttype, xdr::ToXdr, Address, Bytes, BytesN, Env, Symbol, Vec};

use crate::emergency::{EMERGENCY_MSIG_THRESHOLD, EMERGENCY_MSIG_SIGNERS};
use crate::disaster_recovery::EMERGENCY_THRESHOLD;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum age of a snapshot that may be targeted by emergency rollback.
pub const ROLLBACK_MAX_WINDOW_SECS: u64 = 24 * 60 * 60;

/// Minimum community review period before a rollback may execute (48 hours).
pub const ROLLBACK_COMMUNITY_REVIEW_SECS: u64 = 48 * 60 * 60;

/// Governance quorum (bps) required to approve an emergency rollback review.
pub const ROLLBACK_GOVERNANCE_QUORUM_BPS: u32 = 5_000;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Contract scope targeted by an emergency rollback request.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RollbackScope {
    /// Escrow contract disaster-recovery snapshot.
    Escrow,
    /// Governance / multisig configuration snapshot.
    Governance,
    /// Arbitrary contract address (must match the executing contract).
    Contract(Address),
}

/// Cryptographic justification binding an emergency rollback to off-chain evidence.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RollbackJustification {
    /// SHA-256 digest of the incident report / evidence bundle.
    pub evidence_hash: BytesN<32>,
    /// SHA-256 digest of the signed emergency attestation payload.
    pub incident_hash: BytesN<32>,
    /// Optional human-readable summary digest.
    pub description_hash: BytesN<32>,
}

/// Multi-layer emergency rollback proposal awaiting technical + governance approval.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyRollback {
    pub id: u32,
    pub snapshot_id: u32,
    pub old_wasm_hash: BytesN<32>,
    pub scope: RollbackScope,
    pub justification: RollbackJustification,
    pub proposer: Address,
    pub proposed_at: u64,
    /// Earliest timestamp when governance review may conclude (`proposed_at + 48h`).
    pub review_ends_at: u64,
    /// Distinct technical (4-of-7) approvals accumulated.
    pub technical_approval_count: u32,
    /// Registered emergency signers who approved (for audit).
    pub technical_signers: Vec<Address>,
    /// Linked governance proposal id once community review is opened.
    pub governance_proposal_id: Option<u32>,
    /// Set when governance records a passed rollback review vote.
    pub governance_approved: bool,
    pub executed: bool,
    pub rejected: bool,
}

/// Immutable archive entry written **before** rollback mutates live state.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ImmutableRollbackAuditRecord {
    pub rollback_id: u32,
    pub snapshot_id: u32,
    pub evidence_hash: BytesN<32>,
    pub preserved_emergency_audits: u32,
    pub preserved_transition_logs: u32,
    pub timestamp: u64,
    pub executor: Address,
    pub success: bool,
}

// ---------------------------------------------------------------------------
// Authorization helpers
// ---------------------------------------------------------------------------

pub struct RollbackAuthorization;

impl RollbackAuthorization {
    pub fn zero_hash(env: &Env) -> BytesN<32> {
        BytesN::from_array(env, &[0u8; 32])
    }

    /// Evidence and incident digests must be non-zero.
    pub fn validate_justification(env: &Env, justification: &RollbackJustification) -> bool {
        let zero = Self::zero_hash(env);
        justification.evidence_hash != zero && justification.incident_hash != zero
    }

    /// Snapshot must be within the 24-hour rollback window.
    pub fn validate_scope_window(now: u64, snapshot_created_at: u64) -> bool {
        if snapshot_created_at > now {
            return false;
        }
        now.saturating_sub(snapshot_created_at) <= ROLLBACK_MAX_WINDOW_SECS
    }

    pub fn compute_review_ends_at(proposed_at: u64) -> Option<u64> {
        proposed_at.checked_add(ROLLBACK_COMMUNITY_REVIEW_SECS)
    }

    pub fn community_review_elapsed(now: u64, review_ends_at: u64) -> bool {
        now >= review_ends_at
    }

    pub fn technical_threshold_met(count: u32) -> bool {
        count >= EMERGENCY_THRESHOLD && count <= EMERGENCY_MSIG_SIGNERS
    }

    pub fn exact_technical_threshold_met(count: u32) -> bool {
        count == EMERGENCY_MSIG_THRESHOLD
    }

    pub fn is_registered_signer(signers: &Vec<Address>, candidate: &Address) -> bool {
        signers.iter().any(|s| s == *candidate)
    }

    pub fn aggregate_technical_approval(
        approvals: &mut Vec<Address>,
        signer: Address,
    ) -> u32 {
        if !approvals.iter().any(|s| s == signer) {
            approvals.push_back(signer);
        }
        approvals.len() as u32
    }

    pub fn validate_technical_signatures(
        registered: &Vec<Address>,
        approvals: &Vec<Address>,
    ) -> bool {
        if approvals.len() as u32 != EMERGENCY_MSIG_THRESHOLD {
            return false;
        }
        let n = approvals.len();
        for i in 0..n {
            let a = approvals.get(i).unwrap();
            if !Self::is_registered_signer(registered, &a) {
                return false;
            }
            for j in (i + 1)..n {
                if a == approvals.get(j).unwrap() {
                    return false;
                }
            }
        }
        true
    }

    pub fn compute_rollback_params_hash(
        env: &Env,
        snapshot_id: u32,
        old_wasm_hash: &BytesN<32>,
        scope: &RollbackScope,
        justification: &RollbackJustification,
    ) -> BytesN<32> {
        let mut payload = Bytes::new(env);
        payload.append(&snapshot_id.to_xdr(env));
        payload.append(&old_wasm_hash.clone().to_xdr(env));
        match scope {
            RollbackScope::Escrow => {
                payload.append(&Symbol::new(env, "escrow").to_xdr(env));
            }
            RollbackScope::Governance => {
                payload.append(&Symbol::new(env, "governance").to_xdr(env));
            }
            RollbackScope::Contract(addr) => {
                payload.append(&addr.clone().to_xdr(env));
            }
        }
        payload.append(&justification.evidence_hash.clone().to_xdr(env));
        payload.append(&justification.incident_hash.clone().to_xdr(env));
        env.crypto().sha256(&payload).into()
    }

    /// Returns true when technical multisig, governance approval, and review
    /// period requirements are all satisfied.
    pub fn ready_to_execute(rollback: &EmergencyRollback, now: u64) -> bool {
        !rollback.executed
            && !rollback.rejected
            && rollback.governance_approved
            && Self::exact_technical_threshold_met(rollback.technical_approval_count)
            && Self::community_review_elapsed(now, rollback.review_ends_at)
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::{testutils::Address as _, vec, Env};

    #[test]
    fn justification_rejects_zero_hashes() {
        let env = Env::default();
        let zero = RollbackAuthorization::zero_hash(&env);
        let ok = RollbackJustification {
            evidence_hash: BytesN::from_array(&env, &[1u8; 32]),
            incident_hash: BytesN::from_array(&env, &[2u8; 32]),
            description_hash: zero.clone(),
        };
        assert!(RollbackAuthorization::validate_justification(&env, &ok));

        let bad = RollbackJustification {
            evidence_hash: zero.clone(),
            incident_hash: BytesN::from_array(&env, &[2u8; 32]),
            description_hash: zero,
        };
        assert!(!RollbackAuthorization::validate_justification(&env, &bad));
    }

    #[test]
    fn scope_window_enforces_24h() {
        let created = 1_000u64;
        assert!(RollbackAuthorization::validate_scope_window(
            created + ROLLBACK_MAX_WINDOW_SECS,
            created
        ));
        assert!(!RollbackAuthorization::validate_scope_window(
            created + ROLLBACK_MAX_WINDOW_SECS + 1,
            created
        ));
    }

    #[test]
    fn technical_signatures_require_exact_four() {
        let env = Env::default();
        let mut registered = Vec::new(&env);
        for _ in 0..7 {
            registered.push_back(Address::generate(&env));
        }
        let approvals = vec![
            &env,
            registered.get(0).unwrap(),
            registered.get(1).unwrap(),
            registered.get(2).unwrap(),
            registered.get(3).unwrap(),
        ];
        assert!(RollbackAuthorization::validate_technical_signatures(
            &registered,
            &approvals
        ));
        let too_few = vec![&env, registered.get(0).unwrap(), registered.get(1).unwrap()];
        assert!(!RollbackAuthorization::validate_technical_signatures(
            &registered,
            &too_few
        ));
    }
}

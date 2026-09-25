//! Emergency multisig primitives for escrow and governance break-glass paths.
//!
//! Enforces a 4-of-7 signer model, a minimum 24-hour timelock after proposal,
//! a 10% per-24h circuit breaker on emergency releases, and time-bound
//! emergency-admin roles that expire after 72 hours unless renewed.

use soroban_sdk::{contracttype, xdr::ToXdr, Address, Bytes, BytesN, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum delay between emergency proposal and execution (24 hours).
pub const EMERGENCY_TIMELOCK_SECS: u64 = 24 * 60 * 60;

/// Emergency-admin role lifetime unless renewed (72 hours).
pub const EMERGENCY_ADMIN_TTL_SECS: u64 = 72 * 60 * 60;

/// Rolling window for the emergency-release circuit breaker (24 hours).
pub const EMERGENCY_CIRCUIT_WINDOW_SECS: u64 = 24 * 60 * 60;

/// Maximum share of the active escrow pool releasable via emergency paths
/// inside one circuit-breaker window (1_000 bps = 10%).
pub const EMERGENCY_RELEASE_CAP_BPS: i128 = 1_000;

/// Basis-points denominator.
pub const EMERGENCY_BPS_DENOM: i128 = 10_000;

/// Required approvals for emergency operations (exactly 4-of-7).
pub const EMERGENCY_MSIG_THRESHOLD: u32 = 4;

/// Total emergency signer slots.
pub const EMERGENCY_MSIG_SIGNERS: u32 = 7;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Registered 4-of-7 emergency multisig configuration.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyMultisig {
    /// Exactly seven authorised emergency signers.
    pub signers: Vec<Address>,
    /// Approval threshold (must be 4).
    pub threshold: u32,
}

/// Time-bound emergency admin with a limited operational scope.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyAdminRole {
    /// Address granted the emergency-admin role.
    pub admin: Address,
    /// Ledger timestamp when the role was granted or last renewed.
    pub granted_at: u64,
    /// Absolute expiry (`granted_at + EMERGENCY_ADMIN_TTL_SECS`).
    pub expires_at: u64,
    /// Scope tag limiting which emergency operations this admin may execute
    /// (e.g. `"emergency_release"`).
    pub scope: Symbol,
}

/// Pending emergency action awaiting 4-of-7 approval and the 24h timelock.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyAction {
    /// Auto-incremented action identifier.
    pub id: u32,
    /// Operation discriminant (e.g. `"emergency_release"`).
    pub action_type: Symbol,
    /// Target escrow (0 when not escrow-scoped).
    pub escrow_id: u64,
    /// Amount expected to be released (snapshotted at proposal time).
    pub amount: i128,
    /// Address that opened the proposal (must be an emergency signer).
    pub proposer: Address,
    /// Off-chain justification digest.
    pub reason_hash: BytesN<32>,
    /// Hash of immutable action parameters (blocks retry-after-failure).
    pub params_hash: BytesN<32>,
    /// Proposal creation timestamp.
    pub proposed_at: u64,
    /// Earliest executable timestamp (`proposed_at + EMERGENCY_TIMELOCK_SECS`).
    pub execute_after: u64,
    /// Distinct emergency-signer approvals accumulated so far.
    pub approval_count: u32,
    /// Aggregated unique signer addresses that have approved.
    pub signers: Vec<Address>,
    /// True once successfully executed.
    pub executed: bool,
    /// True once a failed execute attempt was permanently recorded.
    pub failed: bool,
}

/// Immutable audit record for an emergency operation.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyAuditRecord {
    /// Action identifier this audit belongs to.
    pub action_id: u32,
    /// Action type discriminant.
    pub action_type: Symbol,
    /// Target escrow id.
    pub escrow_id: u64,
    /// Amount involved.
    pub amount: i128,
    /// Proposer address.
    pub proposer: Address,
    /// Aggregated participant signatures (signer addresses).
    pub participant_signers: Vec<Address>,
    /// Justification digest.
    pub reason_hash: BytesN<32>,
    /// Parameter hash used for anti-retry binding.
    pub params_hash: BytesN<32>,
    /// Ledger timestamp of the audit write.
    pub timestamp: u64,
    /// Whether the attempt succeeded.
    pub success: bool,
}

/// Rolling circuit-breaker state for emergency releases.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyCircuitBreaker {
    /// Start of the current 24-hour window.
    pub window_start: u64,
    /// Cumulative amount released via emergency paths in this window.
    pub released_in_window: i128,
}

// ---------------------------------------------------------------------------
// MultisigValidation — shared validation utilities
// ---------------------------------------------------------------------------

/// Shared emergency-multisig validation helpers.
///
/// Implemented as free functions so both escrow and `multisig_admin` can
/// call them without trait-object overhead inside the Soroban host.
pub struct MultisigValidation;

impl MultisigValidation {
    /// Returns true iff `signers` contains exactly `EMERGENCY_MSIG_SIGNERS`
    /// distinct addresses and `threshold == EMERGENCY_MSIG_THRESHOLD`.
    pub fn is_valid_emergency_config(signers: &Vec<Address>, threshold: u32) -> bool {
        if signers.len() as u32 != EMERGENCY_MSIG_SIGNERS {
            return false;
        }
        if threshold != EMERGENCY_MSIG_THRESHOLD {
            return false;
        }
        // Reject duplicates.
        let n = signers.len();
        for i in 0..n {
            for j in (i + 1)..n {
                if signers.get(i).unwrap() == signers.get(j).unwrap() {
                    return false;
                }
            }
        }
        true
    }

    /// Returns true iff `signer` is present in the registered emergency set.
    pub fn is_emergency_signer(signers: &Vec<Address>, signer: &Address) -> bool {
        signers.iter().any(|s| s == *signer)
    }

    /// Validates that an aggregated approval set satisfies the exact 4-of-7
    /// requirement: every approval must be a registered signer, there must
    /// be no duplicates, and the count must equal `EMERGENCY_MSIG_THRESHOLD`.
    pub fn validate_emergency_signatures(
        registered: &Vec<Address>,
        approvals: &Vec<Address>,
    ) -> bool {
        if approvals.len() as u32 != EMERGENCY_MSIG_THRESHOLD {
            return false;
        }
        let n = approvals.len();
        for i in 0..n {
            let a = approvals.get(i).unwrap();
            if !Self::is_emergency_signer(registered, &a) {
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

    /// Append `signer` to `approvals` if not already present. Returns the
    /// new approval count. Does **not** authenticate — callers must
    /// `require_auth` before invoking.
    pub fn aggregate_signatures(approvals: &mut Vec<Address>, signer: Address) -> u32 {
        if !approvals.iter().any(|s| s == signer) {
            approvals.push_back(signer);
        }
        approvals.len() as u32
    }

    /// True when `now >= execute_after` (24h timelock elapsed).
    pub fn timelock_elapsed(now: u64, execute_after: u64) -> bool {
        now >= execute_after
    }

    /// Compute `execute_after = proposed_at + EMERGENCY_TIMELOCK_SECS`.
    pub fn compute_execute_after(proposed_at: u64) -> Option<u64> {
        proposed_at.checked_add(EMERGENCY_TIMELOCK_SECS)
    }

    /// True when an emergency-admin role is still within its 72h window.
    pub fn is_emergency_admin_active(role: &EmergencyAdminRole, now: u64) -> bool {
        now < role.expires_at
    }

    /// Compute role expiry from grant/renewal time.
    pub fn compute_admin_expiry(granted_at: u64) -> Option<u64> {
        granted_at.checked_add(EMERGENCY_ADMIN_TTL_SECS)
    }

    /// Maximum releasable amount under the 10% circuit breaker given the
    /// current active escrow pool.
    pub fn max_releasable(pool_total: i128) -> i128 {
        if pool_total <= 0 {
            return 0;
        }
        pool_total
            .saturating_mul(EMERGENCY_RELEASE_CAP_BPS)
            / EMERGENCY_BPS_DENOM
    }

    /// Advance or reset the circuit-breaker window and check whether
    /// `additional` can be released without exceeding the 10% cap.
    ///
    /// Returns `Ok(updated_state)` or `Err(())` if the release would breach
    /// the cap.
    pub fn check_circuit_breaker(
        state: &EmergencyCircuitBreaker,
        now: u64,
        pool_total: i128,
        additional: i128,
    ) -> Result<EmergencyCircuitBreaker, ()> {
        let mut window_start = state.window_start;
        let mut released = state.released_in_window;

        if now.saturating_sub(window_start) >= EMERGENCY_CIRCUIT_WINDOW_SECS {
            window_start = now;
            released = 0;
        }

        let max = Self::max_releasable(pool_total);
        let new_total = released.saturating_add(additional);
        if new_total > max {
            return Err(());
        }

        Ok(EmergencyCircuitBreaker {
            window_start,
            released_in_window: new_total,
        })
    }

    /// Deterministic parameter hash binding an emergency attempt so a
    /// permanently-failed attempt cannot be retried with the same inputs.
    pub fn compute_params_hash(
        env: &Env,
        action_type: &Symbol,
        escrow_id: u64,
        amount: i128,
        reason_hash: &BytesN<32>,
    ) -> BytesN<32> {
        let mut payload = Bytes::new(env);
        payload.append(&action_type.clone().to_xdr(env));
        payload.append(&escrow_id.to_xdr(env));
        payload.append(&amount.to_xdr(env));
        payload.append(&reason_hash.clone().to_xdr(env));
        env.crypto().sha256(&payload).into()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::{testutils::Address as _, vec, Env};

    fn seven_signers(env: &Env) -> Vec<Address> {
        let mut s = Vec::new(env);
        for _ in 0..7 {
            s.push_back(Address::generate(env));
        }
        s
    }

    #[test]
    fn valid_config_requires_exactly_seven_and_threshold_four() {
        let env = Env::default();
        let signers = seven_signers(&env);
        assert!(MultisigValidation::is_valid_emergency_config(&signers, 4));
        assert!(!MultisigValidation::is_valid_emergency_config(&signers, 3));
        let mut six = Vec::new(&env);
        for i in 0..6 {
            six.push_back(signers.get(i).unwrap());
        }
        assert!(!MultisigValidation::is_valid_emergency_config(&six, 4));
    }

    #[test]
    fn validate_signatures_requires_exact_four_unique_registered() {
        let env = Env::default();
        let registered = seven_signers(&env);
        let mut approvals = vec![
            &env,
            registered.get(0).unwrap(),
            registered.get(1).unwrap(),
            registered.get(2).unwrap(),
            registered.get(3).unwrap(),
        ];
        assert!(MultisigValidation::validate_emergency_signatures(
            &registered,
            &approvals
        ));

        // Too few
        approvals.pop_back();
        assert!(!MultisigValidation::validate_emergency_signatures(
            &registered,
            &approvals
        ));

        // Duplicate rejected
        let mut dup = vec![
            &env,
            registered.get(0).unwrap(),
            registered.get(1).unwrap(),
            registered.get(2).unwrap(),
            registered.get(0).unwrap(),
        ];
        assert!(!MultisigValidation::validate_emergency_signatures(
            &registered,
            &dup
        ));
        let _ = &mut dup;
    }

    #[test]
    fn aggregate_signatures_deduplicates() {
        let env = Env::default();
        let a = Address::generate(&env);
        let mut approvals = Vec::new(&env);
        assert_eq!(MultisigValidation::aggregate_signatures(&mut approvals, a.clone()), 1);
        assert_eq!(MultisigValidation::aggregate_signatures(&mut approvals, a), 1);
    }

    #[test]
    fn circuit_breaker_caps_at_ten_percent() {
        let state = EmergencyCircuitBreaker {
            window_start: 0,
            released_in_window: 0,
        };
        let pool = 1_000_000i128;
        // 10% = 100_000
        let ok = MultisigValidation::check_circuit_breaker(&state, 100, pool, 100_000);
        assert!(ok.is_ok());
        let over = MultisigValidation::check_circuit_breaker(&state, 100, pool, 100_001);
        assert!(over.is_err());
    }

    #[test]
    fn circuit_breaker_resets_after_window() {
        let state = EmergencyCircuitBreaker {
            window_start: 0,
            released_in_window: 100_000,
        };
        let pool = 1_000_000i128;
        let reset = MultisigValidation::check_circuit_breaker(
            &state,
            EMERGENCY_CIRCUIT_WINDOW_SECS + 1,
            pool,
            50_000,
        )
        .expect("should reset window");
        assert_eq!(reset.released_in_window, 50_000);
        assert_eq!(reset.window_start, EMERGENCY_CIRCUIT_WINDOW_SECS + 1);
    }

    #[test]
    fn emergency_admin_expires_after_72h() {
        let env = Env::default();
        let role = EmergencyAdminRole {
            admin: Address::generate(&env),
            granted_at: 1_000,
            expires_at: 1_000 + EMERGENCY_ADMIN_TTL_SECS,
            scope: Symbol::new(&env, "emergency_release"),
        };
        assert!(MultisigValidation::is_emergency_admin_active(&role, 1_000));
        assert!(!MultisigValidation::is_emergency_admin_active(
            &role,
            1_000 + EMERGENCY_ADMIN_TTL_SECS
        ));
    }
}

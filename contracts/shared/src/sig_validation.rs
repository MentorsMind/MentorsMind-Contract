/// # Signature Validation Utilities
///
/// Provides nonce-based replay protection and signature expiry enforcement for
/// gasless / meta-transactions on Soroban.
///
/// ## Design
///
/// Soroban does not expose raw ECDSA recovery inside a contract — the platform
/// handles key-pair authentication through `Address::require_auth()`.  For
/// **meta-transactions** (where a relayer submits a transaction on behalf of a
/// user who signed the payload off-chain) we therefore use a *structured
/// authorisation envelope* that the Soroban auth framework can verify:
///
/// ```text
/// MetaTxPayload {
///     contract_id,   // prevents cross-contract replay
///     chain_id,      // prevents cross-network replay
///     nonce,         // per-user monotonic counter — prevents replay
///     deadline,      // ledger timestamp after which the sig is invalid
///     action,        // discriminant identifying the intended operation
///     params_hash,   // SHA-256 of the ABI-encoded call parameters
/// }
/// ```
///
/// The signer's `Address` is passed alongside the payload.  The contract calls
/// `signer.require_auth_for_args(payload_as_val)` which causes the Soroban
/// host to verify that the signer authorised exactly this payload.  This
/// delegates cryptographic verification to the host (Ed25519 / secp256k1 /
/// multisig) while the contract enforces the anti-replay invariants.
///
/// ## Nonce scheme
///
/// Each `(contract_id, signer)` pair has an independent nonce stored in
/// persistent storage under `DataKey::Nonce(signer)`.  The nonce is
/// **monotonically increasing** and is incremented atomically on every
/// accepted meta-transaction.  Gaps are not allowed — the submitted nonce
/// must equal the stored nonce exactly.
///
/// ## Expiry
///
/// Every payload carries a `deadline` (ledger timestamp).  The contract
/// rejects payloads where `deadline < current_time + EXPIRY_TOLERANCE_SECS`
/// to absorb validator clock drift.  Payloads with a `deadline` more than
/// `MAX_DEADLINE_SECS` in the future are also rejected to limit the window
/// during which a stolen signature can be used.
use soroban_sdk::{contracttype, symbol_short, Address, BytesN, Env, IntoVal, Symbol};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum remaining lifetime a deadline must have when the meta-tx is
/// submitted.  Absorbs validator timestamp drift (~30 s on Stellar).
pub const EXPIRY_TOLERANCE_SECS: u64 = 60; // 1 minute

/// Maximum deadline offset from the current ledger time.  Signatures valid
/// for longer than this are rejected to limit the stolen-signature window.
pub const MAX_DEADLINE_SECS: u64 = 24 * 60 * 60; // 24 hours

/// Storage key prefix for per-user nonces.
const NONCE_PREFIX: Symbol = symbol_short!("NONCE");

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Discriminant for the operation being authorised.  Extend this enum as new
/// meta-transaction actions are added; the discriminant is included in the
/// signed payload so a signature for one action cannot be replayed as another.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum MetaTxAction {
    /// Authorise deploying a new escrow on behalf of a learner.
    DeployEscrow = 0,
    /// Authorise releasing escrow funds on behalf of the learner.
    ReleaseEscrow = 1,
    /// Authorise cancelling a subscription on behalf of the learner.
    CancelSubscription = 2,
    /// Authorise claiming vested tokens on behalf of the beneficiary.
    ClaimVested = 3,
}

/// The structured payload that the signer must authorise.
///
/// All fields are included in the authorisation so that:
/// - `contract_id` prevents cross-contract replay
/// - `nonce` prevents same-contract replay
/// - `deadline` limits the validity window
/// - `action` prevents cross-function replay
/// - `params_hash` binds the signature to specific call parameters
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MetaTxPayload {
    /// The contract that will consume this authorisation.
    pub contract_id: Address,
    /// Per-user monotonic nonce.  Must equal the stored nonce exactly.
    pub nonce: u64,
    /// Ledger timestamp after which this payload is invalid.
    pub deadline: u64,
    /// The operation being authorised.
    pub action: MetaTxAction,
    /// SHA-256 (or any 32-byte commitment) of the ABI-encoded call parameters.
    /// Binds the signature to the exact arguments — prevents parameter
    /// substitution attacks.
    pub params_hash: BytesN<32>,
}

/// Errors returned by signature validation.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum SigError {
    /// The submitted nonce does not match the stored nonce.
    InvalidNonce = 1,
    /// The deadline has already passed (or is too close to now).
    Expired = 2,
    /// The deadline is unreasonably far in the future.
    DeadlineTooFar = 3,
    /// The payload's `contract_id` does not match the executing contract.
    ContractMismatch = 4,
}

// ---------------------------------------------------------------------------
// Storage helpers
// ---------------------------------------------------------------------------

/// Returns the storage key for a signer's nonce.
fn nonce_key(signer: &Address) -> (Symbol, Address) {
    (NONCE_PREFIX, signer.clone())
}

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

/// Read the current nonce for `signer`.  Returns 0 if no nonce has been
/// recorded yet (i.e. the signer has never submitted a meta-transaction).
pub fn current_nonce(env: &Env, signer: &Address) -> u64 {
    env.storage()
        .persistent()
        .get(&nonce_key(signer))
        .unwrap_or(0u64)
}

/// Validate a meta-transaction payload and advance the nonce atomically.
///
/// # What this checks
///
/// 1. **Contract binding** — `payload.contract_id` must equal
///    `env.current_contract_address()`.  Prevents a valid payload from being
///    replayed against a different contract.
///
/// 2. **Nonce** — `payload.nonce` must equal `current_nonce(signer)`.
///    Prevents replay of any previously accepted payload.
///
/// 3. **Expiry** — `payload.deadline` must be strictly greater than
///    `now + EXPIRY_TOLERANCE_SECS`.  Prevents submission of an already-
///    expired (or about-to-expire) payload.
///
/// 4. **Deadline ceiling** — `payload.deadline` must be ≤
///    `now + MAX_DEADLINE_SECS`.  Prevents signatures with an unreasonably
///    long validity window.
///
/// 5. **Signer authorisation** — `signer.require_auth_for_args(payload)`
///    delegates cryptographic verification to the Soroban host.  The host
///    checks that the signer's key pair signed exactly this payload value.
///
/// # Panics
///
/// Panics with a descriptive message on any validation failure.  The panic
/// causes the transaction to be rolled back, so no state is mutated on
/// failure.
///
/// # Returns
///
/// The new nonce value (i.e. `old_nonce + 1`) after successful validation.
pub fn validate_and_consume_nonce(
    env: &Env,
    signer: &Address,
    payload: &MetaTxPayload,
) -> u64 {
    // 1. Contract binding
    if payload.contract_id != env.current_contract_address() {
        panic!("sig: contract mismatch");
    }

    // 2. Nonce check
    let stored_nonce = current_nonce(env, signer);
    if payload.nonce != stored_nonce {
        panic!("sig: invalid nonce");
    }

    // 3 & 4. Deadline checks
    let now = env.ledger().timestamp();
    validate_deadline(now, payload.deadline);

    // 5. Delegate cryptographic verification to the Soroban host.
    //    The host verifies that `signer` authorised exactly `payload`.
    signer.require_auth_for_args((payload.clone(),).into_val(env));

    // Advance nonce atomically — must happen after auth so a failed auth
    // does not consume the nonce.
    let new_nonce = stored_nonce
        .checked_add(1)
        .expect("sig: nonce overflow");
    env.storage()
        .persistent()
        .set(&nonce_key(signer), &new_nonce);

    new_nonce
}

/// Validate deadline bounds without consuming a nonce.
///
/// Useful for read-only checks (e.g. in view functions that want to report
/// whether a payload is still valid).
///
/// # Panics
///
/// - `"sig: expired"` if `deadline <= now + EXPIRY_TOLERANCE_SECS`
/// - `"sig: deadline too far"` if `deadline > now + MAX_DEADLINE_SECS`
pub fn validate_deadline(now: u64, deadline: u64) {
    // Must have at least EXPIRY_TOLERANCE_SECS of remaining lifetime.
    if deadline <= now.saturating_add(EXPIRY_TOLERANCE_SECS) {
        panic!("sig: expired");
    }
    // Must not be unreasonably far in the future.
    if deadline > now.saturating_add(MAX_DEADLINE_SECS) {
        panic!("sig: deadline too far");
    }
}

/// Returns `true` if the payload is currently valid (deadline not yet passed,
/// within ceiling).  Does **not** check the nonce or signer — use this only
/// for informational / UI purposes.
pub fn is_deadline_valid(env: &Env, deadline: u64) -> bool {
    let now = env.ledger().timestamp();
    deadline > now.saturating_add(EXPIRY_TOLERANCE_SECS)
        && deadline <= now.saturating_add(MAX_DEADLINE_SECS)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    extern crate std;
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Env,
    };

    // -----------------------------------------------------------------------
    // Helpers
    // -----------------------------------------------------------------------

    fn env_at(ts: u64) -> Env {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = ts);
        env
    }

    fn dummy_hash(env: &Env) -> BytesN<32> {
        BytesN::from_array(env, &[0xABu8; 32])
    }

    fn make_payload(
        env: &Env,
        contract_id: Address,
        nonce: u64,
        deadline: u64,
    ) -> MetaTxPayload {
        MetaTxPayload {
            contract_id,
            nonce,
            deadline,
            action: MetaTxAction::DeployEscrow,
            params_hash: dummy_hash(env),
        }
    }

    // -----------------------------------------------------------------------
    // current_nonce
    // -----------------------------------------------------------------------

    #[test]
    fn test_initial_nonce_is_zero() {
        let env = env_at(1_000);
        let signer = Address::generate(&env);
        assert_eq!(current_nonce(&env, &signer), 0);
    }

    // -----------------------------------------------------------------------
    // validate_deadline
    // -----------------------------------------------------------------------

    #[test]
    fn test_deadline_valid() {
        // deadline = now + 1 hour — well within [TOLERANCE, MAX]
        validate_deadline(1_000, 1_000 + 3_600);
    }

    #[test]
    #[should_panic(expected = "sig: expired")]
    fn test_deadline_exactly_at_tolerance_boundary_rejected() {
        // deadline == now + EXPIRY_TOLERANCE_SECS — not strictly greater, rejected
        validate_deadline(1_000, 1_000 + EXPIRY_TOLERANCE_SECS);
    }

    #[test]
    #[should_panic(expected = "sig: expired")]
    fn test_deadline_in_past_rejected() {
        validate_deadline(10_000, 5_000);
    }

    #[test]
    #[should_panic(expected = "sig: expired")]
    fn test_deadline_zero_rejected() {
        validate_deadline(1_000, 0);
    }

    #[test]
    #[should_panic(expected = "sig: deadline too far")]
    fn test_deadline_beyond_max_rejected() {
        // deadline = now + MAX + 1
        validate_deadline(1_000, 1_000 + MAX_DEADLINE_SECS + 1);
    }

    #[test]
    fn test_deadline_at_max_accepted() {
        // deadline = now + MAX — exactly at ceiling, accepted
        validate_deadline(1_000, 1_000 + MAX_DEADLINE_SECS);
    }

    // -----------------------------------------------------------------------
    // is_deadline_valid
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_deadline_valid_true() {
        let env = env_at(1_000);
        assert!(is_deadline_valid(&env, 1_000 + 3_600));
    }

    #[test]
    fn test_is_deadline_valid_expired_false() {
        let env = env_at(10_000);
        assert!(!is_deadline_valid(&env, 5_000));
    }

    #[test]
    fn test_is_deadline_valid_too_far_false() {
        let env = env_at(1_000);
        assert!(!is_deadline_valid(&env, 1_000 + MAX_DEADLINE_SECS + 1));
    }

    // -----------------------------------------------------------------------
    // validate_and_consume_nonce — contract mismatch
    // -----------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "sig: contract mismatch")]
    fn test_contract_mismatch_rejected() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let wrong_contract = Address::generate(&env);
        let signer = Address::generate(&env);

        let payload = make_payload(&env, wrong_contract, 0, 1_000 + 3_600);

        // Call from within the registered contract context
        env.as_contract(&contract_id, || {
            validate_and_consume_nonce(&env, &signer, &payload);
        });
    }

    // -----------------------------------------------------------------------
    // validate_and_consume_nonce — nonce validation
    // -----------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "sig: invalid nonce")]
    fn test_wrong_nonce_rejected() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let signer = Address::generate(&env);

        // Stored nonce is 0, but we submit nonce = 1
        let payload = make_payload(&env, contract_id.clone(), 1, 1_000 + 3_600);

        env.as_contract(&contract_id, || {
            validate_and_consume_nonce(&env, &signer, &payload);
        });
    }

    #[test]
    #[should_panic(expected = "sig: invalid nonce")]
    fn test_replay_same_nonce_rejected() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let signer = Address::generate(&env);

        let payload0 = make_payload(&env, contract_id.clone(), 0, 1_000 + 3_600);

        env.as_contract(&contract_id, || {
            // First submission — succeeds, nonce advances to 1
            validate_and_consume_nonce(&env, &signer, &payload0);
            // Replay with same nonce=0 — must fail
            validate_and_consume_nonce(&env, &signer, &payload0);
        });
    }

    #[test]
    fn test_sequential_nonces_accepted() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let signer = Address::generate(&env);

        env.as_contract(&contract_id, || {
            for expected_nonce in 0u64..5 {
                assert_eq!(current_nonce(&env, &signer), expected_nonce);
                let payload = make_payload(
                    &env,
                    contract_id.clone(),
                    expected_nonce,
                    1_000 + 3_600,
                );
                let new_nonce = validate_and_consume_nonce(&env, &signer, &payload);
                assert_eq!(new_nonce, expected_nonce + 1);
            }
            assert_eq!(current_nonce(&env, &signer), 5);
        });
    }

    #[test]
    #[should_panic(expected = "sig: invalid nonce")]
    fn test_skipped_nonce_rejected() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let signer = Address::generate(&env);

        // Stored nonce is 0, skip to nonce=2
        let payload = make_payload(&env, contract_id.clone(), 2, 1_000 + 3_600);

        env.as_contract(&contract_id, || {
            validate_and_consume_nonce(&env, &signer, &payload);
        });
    }

    // -----------------------------------------------------------------------
    // validate_and_consume_nonce — expiry
    // -----------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "sig: expired")]
    fn test_expired_payload_rejected() {
        let env = env_at(10_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let signer = Address::generate(&env);

        // deadline is in the past
        let payload = make_payload(&env, contract_id.clone(), 0, 5_000);

        env.as_contract(&contract_id, || {
            validate_and_consume_nonce(&env, &signer, &payload);
        });
    }

    #[test]
    #[should_panic(expected = "sig: deadline too far")]
    fn test_deadline_too_far_rejected() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let signer = Address::generate(&env);

        // deadline = now + 25 hours — exceeds MAX_DEADLINE_SECS (24 h)
        let payload = make_payload(
            &env,
            contract_id.clone(),
            0,
            1_000 + 25 * 60 * 60,
        );

        env.as_contract(&contract_id, || {
            validate_and_consume_nonce(&env, &signer, &payload);
        });
    }

    // -----------------------------------------------------------------------
    // Replay attack scenarios
    // -----------------------------------------------------------------------

    /// A captured payload cannot be replayed after the nonce has advanced.
    #[test]
    #[should_panic(expected = "sig: invalid nonce")]
    fn test_replay_after_nonce_advance() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let signer = Address::generate(&env);

        let captured = make_payload(&env, contract_id.clone(), 0, 1_000 + 3_600);

        env.as_contract(&contract_id, || {
            // Legitimate first use
            validate_and_consume_nonce(&env, &signer, &captured);
            // Attacker replays the captured payload — nonce is now 1, must fail
            validate_and_consume_nonce(&env, &signer, &captured);
        });
    }

    /// A payload signed for contract A cannot be replayed against contract B.
    #[test]
    #[should_panic(expected = "sig: contract mismatch")]
    fn test_cross_contract_replay_rejected() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let contract_a = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let contract_b = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let signer = Address::generate(&env);

        // Payload signed for contract_a
        let payload_for_a = make_payload(&env, contract_a.clone(), 0, 1_000 + 3_600);

        // Attacker submits it to contract_b
        env.as_contract(&contract_b, || {
            validate_and_consume_nonce(&env, &signer, &payload_for_a);
        });
    }

    /// An expired payload cannot be submitted even if the nonce is correct.
    #[test]
    #[should_panic(expected = "sig: expired")]
    fn test_expired_payload_with_correct_nonce_rejected() {
        let env = env_at(50_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let signer = Address::generate(&env);

        // Payload was valid at t=1000 but we're now at t=50000
        let stale_payload = make_payload(&env, contract_id.clone(), 0, 1_000 + 3_600);

        env.as_contract(&contract_id, || {
            validate_and_consume_nonce(&env, &signer, &stale_payload);
        });
    }

    /// Two different signers have independent nonces — one's nonce advancing
    /// does not affect the other's.
    #[test]
    fn test_independent_nonces_per_signer() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        env.as_contract(&contract_id, || {
            // Alice submits nonce=0
            let pa = make_payload(&env, contract_id.clone(), 0, 1_000 + 3_600);
            validate_and_consume_nonce(&env, &alice, &pa);
            assert_eq!(current_nonce(&env, &alice), 1);

            // Bob's nonce is still 0
            assert_eq!(current_nonce(&env, &bob), 0);
            let pb = make_payload(&env, contract_id.clone(), 0, 1_000 + 3_600);
            validate_and_consume_nonce(&env, &bob, &pb);
            assert_eq!(current_nonce(&env, &bob), 1);

            // Alice can now submit nonce=1
            let pa2 = make_payload(&env, contract_id.clone(), 1, 1_000 + 3_600);
            validate_and_consume_nonce(&env, &alice, &pa2);
            assert_eq!(current_nonce(&env, &alice), 2);
        });
    }

    /// Nonce does NOT advance when the deadline check fails — the nonce is
    /// consumed only after all checks pass.
    #[test]
    fn test_nonce_not_consumed_on_deadline_failure() {
        let env = env_at(1_000);
        env.mock_all_auths();

        let contract_id = env.register_contract(None, crate::sig_validation::tests::DummyContract);
        let signer = Address::generate(&env);

        env.as_contract(&contract_id, || {
            assert_eq!(current_nonce(&env, &signer), 0);

            // Attempt with expired deadline — panics before nonce is written
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                let expired = make_payload(&env, contract_id.clone(), 0, 500);
                validate_and_consume_nonce(&env, &signer, &expired);
            }));
            assert!(result.is_err());

            // Nonce must still be 0
            assert_eq!(current_nonce(&env, &signer), 0);
        });
    }

    // -----------------------------------------------------------------------
    // Dummy contract used as execution context in tests
    // -----------------------------------------------------------------------

    use soroban_sdk::{contract, contractimpl};

    #[contract]
    pub struct DummyContract;

    #[contractimpl]
    impl DummyContract {}
}

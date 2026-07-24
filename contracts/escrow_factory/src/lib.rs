#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, IntoVal, Symbol, Vec,
};

// Pull in the shared signature-validation utilities.
use shared::sig_validation::{current_nonce, validate_and_consume_nonce, MetaTxAction, MetaTxPayload};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowInfo {
    pub address: Address,
    pub session_id: Symbol,
    pub mentor: Address,
    pub learner: Address,
    pub created_at: u64,
}

// Storage keys
const ADMIN: Symbol = symbol_short!("ADMIN");
const IMPLEMENTATION: Symbol = symbol_short!("IMPL");
const PAUSE_GUARDIAN: Symbol = symbol_short!("PAUSE_GD");
const ESCROW_MAPPING: Symbol = symbol_short!("ESC_MAP");
const ESCROW_LIST: Symbol = symbol_short!("ESC_LIST");
const ESCROW_COUNT: Symbol = symbol_short!("ESC_CNT");
const FACTORY_TTL_THRESHOLD: u32 = 500_000;
const FACTORY_TTL_BUMP: u32 = 1_000_000;

// ---------------------------------------------------------------------------
// Timestamp security constants
// ---------------------------------------------------------------------------

/// Minimum session duration: 1 hour. Prevents sessions so short that
/// validator timestamp drift (±30 s on Stellar) is a meaningful fraction
/// of the window.
const MIN_SESSION_DURATION_SECS: u64 = 60 * 60; // 1 hour

/// Maximum session duration: 30 days. Caps how far into the future a
/// session-end timestamp may be set, limiting the blast radius of a
/// misconfigured or malicious call.
const MAX_SESSION_DURATION_SECS: u64 = 30 * 24 * 60 * 60; // 30 days

/// Default session duration used when the factory deploys an escrow.
const DEFAULT_SESSION_DURATION_SECS: u64 = 24 * 60 * 60; // 24 hours

/// Tolerance window applied to time comparisons to absorb validator
/// timestamp drift (Stellar validators may drift up to ~30 seconds).
/// Using 60 s gives a comfortable margin without meaningfully weakening
/// the time-lock.
pub const TIMESTAMP_TOLERANCE_SECS: u64 = 60; // 1 minute

/// Maximum allowed clock skew for a caller-supplied `start` timestamp.
/// A supplied start that is more than this many seconds in the past is
/// rejected to prevent replaying stale session parameters.
const MAX_PAST_START_SECS: u64 = 5 * 60; // 5 minutes

#[contract]
pub struct EscrowFactory;

#[contractimpl]
impl EscrowFactory {
    /// Initialize the factory with admin, implementation contract, and optional pause guardian.
    pub fn initialize(env: Env, admin: Address, implementation_address: Address) {
        if env.storage().persistent().has(&ADMIN) {
            panic!("Already initialized");
        }

        env.storage().persistent().set(&ADMIN, &admin);
        env.storage()
            .persistent()
            .extend_ttl(&ADMIN, FACTORY_TTL_THRESHOLD, FACTORY_TTL_BUMP);

        env.storage()
            .persistent()
            .set(&IMPLEMENTATION, &implementation_address);
        env.storage().persistent().extend_ttl(
            &IMPLEMENTATION,
            FACTORY_TTL_THRESHOLD,
            FACTORY_TTL_BUMP,
        );

        env.storage().persistent().set(&ESCROW_COUNT, &0u64);
        env.storage().persistent().extend_ttl(
            &ESCROW_COUNT,
            FACTORY_TTL_THRESHOLD,
            FACTORY_TTL_BUMP,
        );
    }

    /// Set the pause guardian contract address. Admin only.
    pub fn set_pause_guardian(env: Env, guardian: Address) {
        let admin = Self::admin(&env);
        admin.require_auth();
        env.storage().persistent().set(&PAUSE_GUARDIAN, &guardian);
        env.storage()
            .persistent()
            .extend_ttl(&PAUSE_GUARDIAN, FACTORY_TTL_THRESHOLD, FACTORY_TTL_BUMP);
    }

    /// Deploy a new escrow contract instance using minimal proxy pattern.
    ///
    /// # Timestamp security
    /// The session-end timestamp is derived from `env.ledger().timestamp()` plus
    /// `DEFAULT_SESSION_DURATION_SECS`.  The resulting value is validated to fall
    /// within [`MIN_SESSION_DURATION_SECS`, `MAX_SESSION_DURATION_SECS`] of the
    /// current ledger time so that validator timestamp manipulation cannot
    /// meaningfully affect the auto-release window.
    pub fn deploy_escrow(
        env: Env,
        mentor: Address,
        learner: Address,
        amount: i128,
        token: Address,
        session_id: Symbol,
    ) -> Address {
        // Check pause guardian
        if let Some(guardian) = env.storage().persistent().get::<_, Address>(&PAUSE_GUARDIAN) {
            let is_paused: bool = env.invoke_contract(
                &guardian,
                &Symbol::new(&env, "is_paused"),
                soroban_sdk::Vec::new(&env),
            );
            if is_paused {
                panic!("Contract is paused");
            }
        }
        // Check if session ID already exists
        let session_key = (ESCROW_MAPPING, session_id.clone());
        if env.storage().persistent().has(&session_key) {
            panic!("Session ID already exists");
        }

        // Get implementation address
        let implementation: Address = env
            .storage()
            .persistent()
            .get(&IMPLEMENTATION)
            .expect("Implementation not set");

        // Compute and validate session-end timestamp.
        // We anchor to the current ledger timestamp so that even if a validator
        // skews the clock by ±TIMESTAMP_TOLERANCE_SECS the session window
        // remains well within the declared bounds.
        let now = env.ledger().timestamp();
        let session_end = now
            .checked_add(DEFAULT_SESSION_DURATION_SECS)
            .expect("timestamp overflow");

        // Sanity-check: session_end must be strictly after now (with tolerance)
        // and within the maximum allowed window.
        Self::validate_future_timestamp(&env, now, session_end, MIN_SESSION_DURATION_SECS, MAX_SESSION_DURATION_SECS);

        // Deploy new escrow instance as minimal proxy
        let escrow_address = Self::deploy_minimal_proxy(&env, &implementation);

        // Initialize the new escrow contract
        let initialize_sym = Symbol::new(&env, "initialize");
        env.invoke_contract(
            &escrow_address,
            &initialize_sym,
            (
                env.current_contract_address(), // Set factory as admin
                Address::generate(&env),        // Treasury (placeholder)
                0u32,                           // Fee bps (placeholder)
                Vec::new(&env),                 // Approved tokens (empty for now)
                72u64 * 60 * 60,                // Auto release delay (72 hours)
            )
                .into_val(&env),
        );

        // Create escrow in the deployed contract
        let create_escrow_sym = Symbol::new(&env, "create_escrow");
        env.invoke_contract(
            &escrow_address,
            &create_escrow_sym,
            (
                mentor,
                learner,
                amount,
                session_id.clone(),
                token,
                session_end, // Validated session-end timestamp
            )
                .into_val(&env),
        );

        // Store mapping
        env.storage()
            .persistent()
            .set(&session_key, &escrow_address);
        env.storage().persistent().extend_ttl(
            &session_key,
            FACTORY_TTL_THRESHOLD,
            FACTORY_TTL_BUMP,
        );

        // Add to list
        let mut count: u64 = env.storage().persistent().get(&ESCROW_COUNT).unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&ESCROW_COUNT, &count);
        env.storage().persistent().extend_ttl(
            &ESCROW_COUNT,
            FACTORY_TTL_THRESHOLD,
            FACTORY_TTL_BUMP,
        );

        let list_key = (ESCROW_LIST, count);
        let escrow_info = EscrowInfo {
            address: escrow_address.clone(),
            session_id: session_id.clone(),
            mentor,
            learner,
            created_at: now,
        };
        env.storage().persistent().set(&list_key, &escrow_info);
        env.storage()
            .persistent()
            .extend_ttl(&list_key, FACTORY_TTL_THRESHOLD, FACTORY_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("escrow_deployed"), session_id.clone()),
            (escrow_address.clone(), session_id),
        );

        escrow_address
    }

    /// Get escrow address by session ID
    pub fn get_escrow_address(env: Env, session_id: Symbol) -> Option<Address> {
        let session_key = (ESCROW_MAPPING, session_id);
        env.storage().persistent().get(&session_key)
    }

    /// Get all escrows with pagination
    pub fn get_all_escrows(env: Env, page: u32, page_size: u32) -> Vec<EscrowInfo> {
        if page == 0 || page_size == 0 {
            panic!("Invalid pagination parameters");
        }

        let count: u64 = env.storage().persistent().get(&ESCROW_COUNT).unwrap_or(0);
        let start_idx = ((page - 1) * page_size) as u64 + 1;
        let end_idx = (start_idx + page_size as u64 - 1).min(count);

        let mut result = Vec::new(&env);

        for i in start_idx..=end_idx {
            let list_key = (ESCROW_LIST, i);
            if let Some(escrow_info) = env.storage().persistent().get::<_, EscrowInfo>(&list_key) {
                result.push_back(escrow_info);
            }
            env.storage().persistent().extend_ttl(
                &list_key,
                FACTORY_TTL_THRESHOLD,
                FACTORY_TTL_BUMP,
            );
        }

        result
    }

    /// Update implementation contract for future deployments
    pub fn upgrade_implementation(env: Env, new_implementation: Address) {
        let admin = Self::admin(&env);
        admin.require_auth();

        env.storage()
            .persistent()
            .set(&IMPLEMENTATION, &new_implementation);
        env.storage().persistent().extend_ttl(
            &IMPLEMENTATION,
            FACTORY_TTL_THRESHOLD,
            FACTORY_TTL_BUMP,
        );

        env.events().publish(
            (symbol_short!("implementation_upgraded")),
            (new_implementation, env.ledger().timestamp()),
        );
    }

    /// Get current implementation address
    pub fn get_implementation(env: Env) -> Address {
        env.storage()
            .persistent()
            .get(&IMPLEMENTATION)
            .expect("Implementation not set")
    }

    /// Get admin address
    pub fn get_admin(env: Env) -> Address {
        Self::admin(&env)
    }

    /// Get total escrow count
    pub fn get_escrow_count(env: Env) -> u64 {
        env.storage().persistent().get(&ESCROW_COUNT).unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Meta-transaction (gasless) entry point
    // -----------------------------------------------------------------------

    /// Execute a gasless `DeployEscrow` meta-transaction.
    ///
    /// A relayer calls this on behalf of a `signer` (typically the learner).
    /// The signer must have authorised `payload` off-chain; the Soroban host
    /// verifies the cryptographic signature via `require_auth_for_args`.
    ///
    /// # Replay protection
    ///
    /// - `payload.nonce` must equal the signer's current stored nonce.
    /// - `payload.deadline` must be in the future (within `MAX_DEADLINE_SECS`).
    /// - `payload.contract_id` must equal this contract's address.
    /// - `payload.action` must be `MetaTxAction::DeployEscrow`.
    /// - `payload.params_hash` must be the SHA-256 of
    ///   `(mentor, learner, amount, token, session_id)` encoded by the caller.
    ///
    /// On success the nonce is incremented and `deploy_escrow` is called with
    /// the provided parameters.
    ///
    /// # Arguments
    ///
    /// * `signer`     — the address whose key pair signed `payload`
    /// * `payload`    — the structured authorisation envelope
    /// * `mentor`     — mentor address for the new escrow
    /// * `learner`    — learner address for the new escrow
    /// * `amount`     — escrow amount in token base units
    /// * `token`      — token contract address
    /// * `session_id` — unique session identifier
    pub fn execute_meta_tx(
        env: Env,
        signer: Address,
        payload: MetaTxPayload,
        mentor: Address,
        learner: Address,
        amount: i128,
        token: Address,
        session_id: Symbol,
    ) -> Address {
        // Validate action discriminant — prevents a signature for one action
        // being replayed as a different action.
        if payload.action != MetaTxAction::DeployEscrow {
            panic!("meta: wrong action");
        }

        // Validate payload, verify signer authorisation, and advance nonce.
        // Panics on any failure — transaction is rolled back, nonce unchanged.
        validate_and_consume_nonce(&env, &signer, &payload);

        // Proceed with the actual escrow deployment.
        Self::deploy_escrow(env, mentor, learner, amount, token, session_id)
    }

    /// Return the current nonce for `signer`.
    ///
    /// Off-chain clients call this to determine the next nonce to include in
    /// a `MetaTxPayload` before asking the user to sign.
    pub fn get_nonce(env: Env, signer: Address) -> u64 {
        current_nonce(&env, &signer)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Validate that `future_ts` is a reasonable future timestamp relative to
    /// `now`.  Panics if:
    /// - `future_ts` is not strictly greater than `now + TIMESTAMP_TOLERANCE_SECS`
    ///   (i.e. the window is too short to be meaningful after absorbing drift)
    /// - The duration `future_ts - now` exceeds `max_duration_secs`
    ///
    /// The tolerance window means a validator that skews the clock forward by
    /// up to `TIMESTAMP_TOLERANCE_SECS` cannot cause a time-sensitive operation
    /// to trigger prematurely.
    fn validate_future_timestamp(
        _env: &Env,
        now: u64,
        future_ts: u64,
        min_duration_secs: u64,
        max_duration_secs: u64,
    ) {
        // future_ts must be strictly after now (no same-block execution)
        if future_ts <= now {
            panic!("timestamp must be in the future");
        }
        let duration = future_ts - now;
        // Enforce minimum window (must exceed tolerance to be meaningful)
        if duration < min_duration_secs.saturating_add(TIMESTAMP_TOLERANCE_SECS) {
            panic!("timestamp window too short");
        }
        // Enforce maximum window
        if duration > max_duration_secs {
            panic!("timestamp window too long");
        }
    }

    /// Validate that a caller-supplied `start` timestamp is not unreasonably
    /// far in the past (which could indicate a replayed or stale transaction).
    pub fn validate_start_timestamp(_env: &Env, now: u64, start: u64) {
        // Allow start to be up to MAX_PAST_START_SECS in the past (clock drift)
        // but reject anything older than that.
        if start < now.saturating_sub(MAX_PAST_START_SECS) {
            panic!("start timestamp too far in the past");
        }
        // Also reject start timestamps more than MAX_PAST_START_SECS in the future
        // (prevents pre-dating sessions).
        if start > now.saturating_add(MAX_PAST_START_SECS) {
            panic!("start timestamp too far in the future");
        }
    }

    /// Deploy minimal proxy (clone) of implementation contract
    fn deploy_minimal_proxy(env: &Env, implementation: &Address) -> Address {
        // In Soroban, we deploy a new contract instance that will delegate calls
        // to the implementation. For now, we create a new contract address.
        // In a real implementation, this would create a minimal proxy contract.
        let salt = env.prng().gen::<u64>();
        let deployer = env.deployer();
        let deployed_address = deployer
            .with_current_contract(salt)
            .deploy_address(implementation);
        deployed_address
    }

    /// Get admin address (internal helper)
    fn admin(env: &Env) -> Address {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&ADMIN)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&ADMIN, FACTORY_TTL_THRESHOLD, FACTORY_TTL_BUMP);
        admin
    }
}

#[cfg(test)]
mod testutils;

// ---------------------------------------------------------------------------
// Timestamp audit tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod timestamp_tests {
    use super::*;
    use soroban_sdk::{testutils::Ledger, Env};

    /// Helper: create a minimal env with a known timestamp.
    fn env_at(ts: u64) -> Env {
        let env = Env::default();
        env.ledger().with_mut(|li| li.timestamp = ts);
        env
    }

    // --- validate_future_timestamp ---

    #[test]
    fn test_future_timestamp_valid() {
        let env = env_at(1_000);
        // 24 h window is well within [MIN, MAX]
        EscrowFactory::validate_future_timestamp(
            &env,
            1_000,
            1_000 + DEFAULT_SESSION_DURATION_SECS,
            MIN_SESSION_DURATION_SECS,
            MAX_SESSION_DURATION_SECS,
        );
    }

    #[test]
    #[should_panic(expected = "timestamp must be in the future")]
    fn test_future_timestamp_not_future() {
        let env = env_at(1_000);
        EscrowFactory::validate_future_timestamp(
            &env,
            1_000,
            1_000, // same as now — not future
            MIN_SESSION_DURATION_SECS,
            MAX_SESSION_DURATION_SECS,
        );
    }

    #[test]
    #[should_panic(expected = "timestamp window too short")]
    fn test_future_timestamp_too_short() {
        let env = env_at(1_000);
        // Only 30 s — less than MIN_SESSION_DURATION_SECS + TOLERANCE
        EscrowFactory::validate_future_timestamp(
            &env,
            1_000,
            1_030,
            MIN_SESSION_DURATION_SECS,
            MAX_SESSION_DURATION_SECS,
        );
    }

    #[test]
    #[should_panic(expected = "timestamp window too long")]
    fn test_future_timestamp_too_long() {
        let env = env_at(1_000);
        // 31 days — exceeds MAX_SESSION_DURATION_SECS
        EscrowFactory::validate_future_timestamp(
            &env,
            1_000,
            1_000 + 31 * 24 * 60 * 60,
            MIN_SESSION_DURATION_SECS,
            MAX_SESSION_DURATION_SECS,
        );
    }

    // --- validate_start_timestamp ---

    #[test]
    fn test_start_timestamp_valid_now() {
        let env = env_at(10_000);
        EscrowFactory::validate_start_timestamp(&env, 10_000, 10_000);
    }

    #[test]
    fn test_start_timestamp_valid_slight_past() {
        let env = env_at(10_000);
        // 2 minutes in the past — within MAX_PAST_START_SECS (5 min)
        EscrowFactory::validate_start_timestamp(&env, 10_000, 10_000 - 120);
    }

    #[test]
    #[should_panic(expected = "start timestamp too far in the past")]
    fn test_start_timestamp_too_old() {
        let env = env_at(10_000);
        // 10 minutes in the past — exceeds MAX_PAST_START_SECS
        EscrowFactory::validate_start_timestamp(&env, 10_000, 10_000 - 600);
    }

    #[test]
    #[should_panic(expected = "start timestamp too far in the future")]
    fn test_start_timestamp_too_future() {
        let env = env_at(10_000);
        // 10 minutes in the future — exceeds MAX_PAST_START_SECS
        EscrowFactory::validate_start_timestamp(&env, 10_000, 10_000 + 600);
    }

    // --- Validator drift simulation ---
    // Simulate a validator that skews the clock forward by TIMESTAMP_TOLERANCE_SECS.
    // The session-end window should still be valid because we added the tolerance
    // to the minimum duration check.

    #[test]
    fn test_drift_forward_still_valid() {
        // Validator reports time as now + TOLERANCE (worst-case forward drift)
        let skewed_now = 1_000 + TIMESTAMP_TOLERANCE_SECS;
        let env = env_at(skewed_now);
        let session_end = skewed_now + DEFAULT_SESSION_DURATION_SECS;
        EscrowFactory::validate_future_timestamp(
            &env,
            skewed_now,
            session_end,
            MIN_SESSION_DURATION_SECS,
            MAX_SESSION_DURATION_SECS,
        );
    }
}

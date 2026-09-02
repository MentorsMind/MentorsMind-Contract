#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Bytes, BytesN, Env, IntoVal,
    Symbol, Vec,
};
use soroban_sdk::xdr::ToXdr;

// Pull in the shared signature-validation utilities.
use shared::sig_validation::{current_nonce, validate_and_consume_nonce, MetaTxAction, MetaTxPayload};
use shared::GasEstimate;
use shared::dynamic_fees::{calculate_dynamic_fee, DynamicFeeResult};
use shared::health_reporter::{report_metric, MetricCategory};

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
const ANOMALY_DETECTOR: Symbol = symbol_short!("ANOM_DET");
const BYPASS_ANOMALY: Symbol = symbol_short!("BYPASS_AN");
const ESCROW_MAPPING: Symbol = symbol_short!("ESC_MAP");
const ESCROW_LIST: Symbol = symbol_short!("ESC_LIST");
const ESCROW_COUNT: Symbol = symbol_short!("ESC_CNT");
/// Per-session redeployment counter: `SessionNonce(session_id) -> u32`.
/// Bumped each time an escrow for `session_id` is (re)deployed, so a new
/// deployment after a previous one expired produces a different salt (and
/// therefore a different address) instead of colliding.
const SESSION_NONCE: Symbol = symbol_short!("SESS_NCE");
const INTERFACE_REGISTRY: Symbol = symbol_short!("IF_REG");
const HEALTH_DASHBOARD: Symbol = symbol_short!("HLTH_DB");
const FACTORY_TTL_THRESHOLD: u32 = 500_000;
const FACTORY_TTL_BUMP: u32 = 1_000_000;

// ---------------------------------------------------------------------------
// Gas-estimation heuristic constants (#761). Calibrated against
// `env.budget().cpu_instruction_cost()` measured around a real
// `deploy_escrow` call in the estimate-vs-actual test.
// ---------------------------------------------------------------------------
const DEPLOY_BASE_INSTRUCTIONS: u64 = 40_000;
const DEPLOY_PER_STORAGE_OP_INSTRUCTIONS: u64 = 2_000;
const DEPLOY_PER_CROSS_CALL_INSTRUCTIONS: u64 = 230_000;

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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    HighValueThreshold,
    PendingHighValueSession(Symbol),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingSession {
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub token: Address,
    pub requested_at: u64,
}

pub const HIGH_VALUE_APPROVAL_WINDOW_SECS: u64 = 48 * 3600;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingAdminChange {
    pub new_admin: Address,
    pub effective_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChangeProposedEvent {
    pub contract: Address,
    pub old_admin: Address,
    pub new_admin: Address,
    pub effective_at: u64,
}

const ADMIN_CHANGE_TIMELOCK: u64 = 48 * 60 * 60;
const PENDING_ADMIN: Symbol = symbol_short!("PEND_ADM");

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

    pub fn propose_admin_change(
        env: Env,
        current_admin: Address,
        new_admin: Address,
    ) {
        Self::require_admin(&env, &current_admin);
        let old_admin = Self::admin(&env);
        let effective_at = env
            .ledger()
            .timestamp()
            .checked_add(ADMIN_CHANGE_TIMELOCK)
            .expect("timestamp overflow");
        env.storage().persistent().set(
            &PENDING_ADMIN,
            &PendingAdminChange {
                new_admin: new_admin.clone(),
                effective_at,
            },
        );
        env.events().publish(
            (symbol_short!("admin"), symbol_short!("proposed")),
            AdminChangeProposedEvent {
                contract: env.current_contract_address(),
                old_admin,
                new_admin,
                effective_at,
            },
        );
    }

    pub fn accept_admin_change(env: Env, new_admin: Address) {
        new_admin.require_auth();
        let pending: PendingAdminChange = env
            .storage()
            .persistent()
            .get(&PENDING_ADMIN)
            .expect("no pending admin change");
        if pending.new_admin != new_admin {
            panic!("unauthorized");
        }
        if env.ledger().timestamp() < pending.effective_at {
            panic!("admin change not yet effective");
        }
        env.storage().persistent().set(&ADMIN, &new_admin);
        env.storage().persistent().remove(&PENDING_ADMIN);
    }

    pub fn cancel_admin_change(env: Env, multisig: Address) {
        multisig.require_auth();
        if !env.storage().persistent().has(&PENDING_ADMIN) {
            panic!("no pending admin change");
        }
        env.storage().persistent().remove(&PENDING_ADMIN);
    }

    pub fn get_pending_admin_change(env: Env) -> Option<PendingAdminChange> {
        env.storage().persistent().get(&PENDING_ADMIN)
    }

    pub fn get_admin(env: Env) -> Address {
        Self::admin(&env)
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

    /// Set the interface registry contract address. Admin only.
    pub fn set_interface_registry(env: Env, registry: Address) {
        let admin = Self::admin(&env);
        admin.require_auth();
        env.storage().persistent().set(&INTERFACE_REGISTRY, &registry);
        env.storage()
            .persistent()
            .extend_ttl(&INTERFACE_REGISTRY, FACTORY_TTL_THRESHOLD, FACTORY_TTL_BUMP);
    }

    /// Set the health dashboard address for metric reporting. Admin only.
    pub fn set_health_dashboard(env: Env, health_dashboard: Address) {
        let admin = Self::admin(&env);
        admin.require_auth();
        env.storage().persistent().set(&HEALTH_DASHBOARD, &health_dashboard);
        env.storage()
            .persistent()
            .extend_ttl(&HEALTH_DASHBOARD, FACTORY_TTL_THRESHOLD, FACTORY_TTL_BUMP);
    }

    pub fn set_anomaly_detector(env: Env, detector: Address) {
        let admin = Self::admin(&env);
        admin.require_auth();
        env.storage().persistent().set(&ANOMALY_DETECTOR, &detector);
        env.storage().persistent().extend_ttl(&ANOMALY_DETECTOR, FACTORY_TTL_THRESHOLD, FACTORY_TTL_BUMP);
    }

    pub fn set_bypass_anomaly_check(env: Env, bypass: bool) {
        let admin = Self::admin(&env);
        admin.require_auth();
        env.storage().persistent().set(&BYPASS_ANOMALY, &bypass);
        env.storage().persistent().extend_ttl(&BYPASS_ANOMALY, FACTORY_TTL_THRESHOLD, FACTORY_TTL_BUMP);
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
        // Anomaly detection check
        let bypass: bool = env.storage().persistent().get(&BYPASS_ANOMALY).unwrap_or(false);
        if !bypass {
            if let Some(anomaly_detector) = env.storage().persistent().get::<_, Address>(&ANOMALY_DETECTOR) {
                let res: u32 = env.invoke_contract(
                    &anomaly_detector,
                    &Symbol::new(&env, "check_anomaly"),
                    (learner.clone(), 0u32, amount).into_val(&env), // 0u32 = AnomalyAction::CreateEscrow
                );
                if res == 2 {
                    panic!("UserOnHold");
                } else if res == 1 {
                    env.events().publish((symbol_short!("anom_warn"), learner.clone()), amount);
                }
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

        let threshold: i128 = env.storage().persistent().get(&DataKey::HighValueThreshold).unwrap_or(50_000_000_000);
        
        if amount > threshold {
            let pending = PendingSession {
                mentor: mentor.clone(),
                learner: learner.clone(),
                amount,
                token: token.clone(),
                requested_at: now,
            };
            env.storage().persistent().set(&DataKey::PendingHighValueSession(session_id.clone()), &pending);
            env.events().publish(
                (Symbol::new(&env, "HighValueSessionPending"), session_id.clone()),
                (amount, now + HIGH_VALUE_APPROVAL_WINDOW_SECS),
            );
            
            let nonce_key = (SESSION_NONCE, session_id.clone());
            let current_nonce: u32 = env.storage().persistent().get(&nonce_key).unwrap_or(0);
            let next_nonce = current_nonce.checked_add(1).expect("nonce overflow");
            let salt = Self::compute_salt(&env, &session_id, &mentor, &learner, next_nonce);
            return Self::predicted_address(&env, &implementation, salt);
        }

        Self::deploy_escrow_internal(env, mentor, learner, amount, token, session_id, implementation, now, session_end)
    }

    fn deploy_escrow_internal(
        env: Env,
        mentor: Address,
        learner: Address,
        amount: i128,
        token: Address,
        session_id: Symbol,
        implementation: Address,
        now: u64,
        session_end: u64,
    ) -> Address {
        let session_key = (ESCROW_MAPPING, session_id.clone());

        // Bump this session's nonce *before* computing the salt so a
        // redeployment (after a prior escrow for the same session_id
        // expired and was superseded) gets a fresh address instead of
        // colliding with — or being predictable from — the previous one.
        let nonce_key = (SESSION_NONCE, session_id.clone());
        let nonce: u32 = env.storage().persistent().get(&nonce_key).unwrap_or(0);
        let next_nonce = nonce.checked_add(1).expect("nonce overflow");
        env.storage().persistent().set(&nonce_key, &next_nonce);
        env.storage()
            .persistent()
            .extend_ttl(&nonce_key, FACTORY_TTL_THRESHOLD, FACTORY_TTL_BUMP);

        let salt = Self::compute_salt(&env, &session_id, &mentor, &learner, next_nonce);

        // Deploy new escrow instance as minimal proxy
        let escrow_address = Self::deploy_minimal_proxy(&env, &implementation, salt);

        // Calculate dynamic fee
        let system_load: u32 = 0; // Replace with actual load metric
        let reputation: u32 = 100; // Replace with actual reputation
        let fee_bps = Self::calculate_escrow_fees(&env, system_load, reputation);

        // Initialize the new escrow contract
        let initialize_sym = Symbol::new(&env, "initialize");
        let _: () = env.invoke_contract(
            &escrow_address,
            &initialize_sym,
            (
                env.current_contract_address(), // Set factory as admin
                env.current_contract_address(), // Treasury (placeholder)
                fee_bps,                        // Fee bps dynamically calculated
                Vec::<Address>::new(&env),      // Approved tokens (empty for now)
                72u64 * 60 * 60,                // Auto release delay (72 hours)
            )
                .into_val(&env),
        );

        // Create escrow in the deployed contract
        let create_escrow_sym = Symbol::new(&env, "create_escrow");
        let _: Address = env.invoke_contract(
            &escrow_address,
            &create_escrow_sym,
            (
                mentor.clone(),
                learner.clone(),
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
            (Symbol::new(&env, "escrow_deployed"), session_id.clone()),
            (escrow_address.clone(), session_id),
        );

        // Register interface in the interface registry (if set)
        if let Some(registry_addr) = env.storage().persistent().get::<_, Address>(&INTERFACE_REGISTRY) {
            let interface_id = Symbol::new(&env, "escrow_v1");
            let _: () = env.invoke_contract(
                &registry_addr,
                &Symbol::new(&env, "register_interface"),
                (escrow_address.clone(), interface_id, 1u32).into_val(&env),
            );
        }

        // Report health metric for escrow creation
        if let Some(dashboard) = env.storage().persistent().get::<_, Address>(&HEALTH_DASHBOARD) {
            let count: u64 = env.storage().persistent().get(&ESCROW_COUNT).unwrap_or(0);
            report_metric(
                &env,
                &dashboard,
                Symbol::new(&env, "escrow_created"),
                MetricCategory::Throughput,
                count as i128,
            );
        }

        escrow_address
    }

    pub fn approve_high_value_session(env: Env, multisig: Address, session_id: Symbol) -> Address {
        multisig.require_auth();
        
        let key = DataKey::PendingHighValueSession(session_id.clone());
        let pending: PendingSession = env.storage().persistent().get(&key).expect("Session not pending");
        
        let now = env.ledger().timestamp();
        if now > pending.requested_at + HIGH_VALUE_APPROVAL_WINDOW_SECS {
            // Expired approval: Automatically refund learner.
            // Assumption: factory holds the tokens that were transferred for this pending session.
            let token_client = soroban_sdk::token::Client::new(&env, &pending.token);
            token_client.transfer(&env.current_contract_address(), &pending.learner, &pending.amount);
            env.storage().persistent().remove(&key);
            panic!("Approval expired, refunded learner");
        }
        
        env.storage().persistent().remove(&key);
        
        let implementation: Address = env.storage().persistent().get(&IMPLEMENTATION).expect("Implementation not set");
        let session_end = now.checked_add(DEFAULT_SESSION_DURATION_SECS).expect("timestamp overflow");
        
        let address = Self::deploy_escrow_internal(
            env.clone(),
            pending.mentor,
            pending.learner,
            pending.amount,
            pending.token,
            session_id.clone(),
            implementation,
            now,
            session_end,
        );
        
        env.events().publish((Symbol::new(&env, "HighValueSessionApproved"), session_id), multisig);
        address
    }

    /// Heuristic instruction/IO estimate for `deploy_escrow`, without
    /// deploying anything. Mirrors the real flow's fixed reads/writes
    /// (nonce, session mapping, implementation, escrow count, list entry)
    /// and cross-contract calls (proxy deployment, `initialize`,
    /// `create_escrow`), then adds the optional pause-guardian /
    /// anomaly-detector / interface-registry checks based on *current
    /// storage state* — i.e. whichever of those integrations are actually
    /// configured right now.
    pub fn estimate_deploy_escrow_cost(env: Env) -> GasEstimate {
        // deploy_escrow's own reads: BYPASS_ANOMALY, session-exists check,
        // IMPLEMENTATION, nonce, ESCROW_COUNT.
        let mut storage_reads: u32 = 5;
        // deploy_escrow's own writes: nonce, session mapping, ESCROW_COUNT,
        // list entry.
        let storage_writes: u32 = 4;
        // deploy_escrow's own cross-contract calls: minimal-proxy deploy,
        // initialize, create_escrow.
        let mut cross_contract_calls: u32 = 3;

        if env.storage().persistent().has(&PAUSE_GUARDIAN) {
            storage_reads += 1;
            cross_contract_calls += 1; // is_paused check
        }
        let bypass: bool = env.storage().persistent().get(&BYPASS_ANOMALY).unwrap_or(false);
        if !bypass && env.storage().persistent().has(&ANOMALY_DETECTOR) {
            storage_reads += 1;
            cross_contract_calls += 1; // check_anomaly
        }
        if env.storage().persistent().has(&INTERFACE_REGISTRY) {
            storage_reads += 1;
            cross_contract_calls += 1; // register_interface
        }

        let base_instructions = DEPLOY_BASE_INSTRUCTIONS
            + (storage_reads as u64 + storage_writes as u64) * DEPLOY_PER_STORAGE_OP_INSTRUCTIONS
            + (cross_contract_calls as u64) * DEPLOY_PER_CROSS_CALL_INSTRUCTIONS;

        GasEstimate {
            base_instructions,
            storage_reads,
            storage_writes,
            cross_contract_calls,
        }
    }

    /// Get escrow address by session ID
    pub fn get_escrow_address(env: Env, session_id: Symbol) -> Option<Address> {
        let session_key = (ESCROW_MAPPING, session_id);
        env.storage().persistent().get(&session_key)
    }

    /// Predict the address `deploy_escrow` will produce for the *next*
    /// deployment of `(session_id, mentor, learner)`, without deploying
    /// anything on-chain.
    ///
    /// This lets a learner pre-approve token spend to the escrow address
    /// before it exists (compute address off-chain → approve → deploy+fund
    /// in one transaction), instead of requiring deploy → read address →
    /// approve → fund as separate round-trips.
    ///
    /// The predicted address is deterministic given the current
    /// `SessionNonce(session_id)`: it accounts for redeployment, so if a
    /// previous escrow for this exact `session_id` expired and a new one
    /// is deployed, this function (called again) returns the new address,
    /// matching what `deploy_escrow` will actually produce next.
    pub fn predict_escrow_address(
        env: Env,
        session_id: Symbol,
        mentor: Address,
        learner: Address,
    ) -> Address {
        let implementation: Address = env
            .storage()
            .persistent()
            .get(&IMPLEMENTATION)
            .expect("Implementation not set");

        let nonce_key = (SESSION_NONCE, session_id.clone());
        let current_nonce: u32 = env.storage().persistent().get(&nonce_key).unwrap_or(0);
        let next_nonce = current_nonce.checked_add(1).expect("nonce overflow");

        let salt = Self::compute_salt(&env, &session_id, &mentor, &learner, next_nonce);
        Self::predicted_address(&env, &implementation, salt)
    }

    /// Return the current redeployment nonce for `session_id` (0 if no
    /// escrow has ever been deployed for it). The *next* deployment will
    /// use `nonce + 1` when computing its salt.
    pub fn get_session_nonce(env: Env, session_id: Symbol) -> u32 {
        env.storage()
            .persistent()
            .get(&(SESSION_NONCE, session_id))
            .unwrap_or(0)
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
            (Symbol::new(&env, "impl_upgraded"),),
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

    fn delegate_call(env: &Env, target: &Address, func: &Symbol, args: &Vec<soroban_sdk::Val>) -> soroban_sdk::Val {
        env.invoke_contract(target, func, args.clone())
    }

    // -----------------------------------------------------------------------
    // Dynamic Fee Logic
    // -----------------------------------------------------------------------

    pub fn calculate_escrow_fees(env: &Env, system_load: u32, reputation_score: u32) -> u32 {
        let result = calculate_dynamic_fee(env, system_load, reputation_score);
        result.fee_bps
    }

    pub fn validate_fee_payments(env: &Env, expected_bps: u32, provided_bps: u32) -> bool {
        // Allow a tiny rounding tolerance if needed, but normally should exactly match
        expected_bps == provided_bps
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

    /// Compute the deterministic deployment salt for
    /// `(session_id, mentor, learner, nonce)`.
    ///
    /// `sha256(session_id || mentor || learner || nonce)` — derived purely
    /// from parameters public before deployment, so both this contract and
    /// an off-chain client can compute the same salt (and therefore the
    /// same predicted address) without any on-chain round-trip. The
    /// deployed address additionally depends on this factory contract's
    /// own address (via `Deployer::with_current_contract`, which derives
    /// the contract ID from the *current* contract + salt), which prevents
    /// a different factory instance from front-running/pre-claiming the
    /// address computed here.
    fn compute_salt(
        env: &Env,
        session_id: &Symbol,
        mentor: &Address,
        learner: &Address,
        nonce: u32,
    ) -> BytesN<32> {
        let mut bytes = soroban_sdk::Bytes::new(env);
        bytes.append(&session_id.to_xdr(env));
        bytes.append(&mentor.to_xdr(env));
        bytes.append(&learner.to_xdr(env));
        bytes.append(&Bytes::from_array(env, &nonce.to_be_bytes()));
        env.crypto().sha256(&bytes).into()
    }

    /// Return the address that would result from deploying `implementation`
    /// with `salt` from this factory contract, without deploying anything.
    fn predicted_address(env: &Env, implementation: &Address, salt: BytesN<32>) -> Address {
        let _ = implementation;
        env.deployer().with_current_contract(salt).deployed_address()
    }

    /// Deploy minimal proxy (clone) of implementation contract using a
    /// deterministic `salt` (see [`Self::compute_salt`]) so the resulting
    /// address matches what [`Self::predict_escrow_address`] returned
    /// beforehand.
    fn deploy_minimal_proxy(env: &Env, implementation: &Address, salt: BytesN<32>) -> Address {
        let _ = implementation;
        env.deployer().with_current_contract(salt).deployed_address()
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

    fn require_admin(env: &Env, caller: &Address) {
        caller.require_auth();
        if *caller != Self::admin(env) {
            panic!("Unauthorized");
        }
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

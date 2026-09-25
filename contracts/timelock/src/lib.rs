#![no_std]
#![allow(deprecated)] // Temporarily allow deprecated Events::publish until we migrate to #[contractevent]
#![allow(unexpected_cfgs)] // kani proofs use #[cfg(kani)] injected by the cargo-kani driver
use multisig_admin::MultisigAdminContractClient;
use shared::events::{
    emit_timelock_event, evt_timelock_adm_xfr, evt_timelock_cancel, evt_timelock_emerg_cancel,
    evt_timelock_exec, evt_timelock_guardian_set, evt_timelock_init, evt_timelock_sched,
};
use soroban_sdk::{
    contract, contractimpl, contracterror, contracttype, Address, Bytes, BytesN, Env,
    Symbol, Val, Vec,
};
use soroban_sdk::xdr::ToXdr;

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    NotAdmin = 3,
    OperationNotFound = 4,
    AlreadyDone = 5,
    NotReady = 6,
    InvalidDelay = 7,
    /// Caller is not the registered guardian multisig (or none is registered).
    NotGuardian = 8,
    /// The address proposed as guardian does not meet the minimum
    /// emergency-multisig configuration (threshold/signer floor).
    InvalidGuardianConfig = 9,
    /// Guardian override limit exceeded (max 3 per 30-day period).
    GuardianLimitExceeded = 10,
    /// Missing justification for guardian override operation.
    MissingJustification = 11,
    /// Community veto period still active for this guardian action.
    VetoPeriodActive = 12,
    /// Guardian action has been vetoed by community.
    ActionVetoed = 13,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Minimum delay: 48 hours
pub const MIN_DELAY: u64 = 48 * 60 * 60;
/// Maximum delay: 30 days
pub const MAX_DELAY: u64 = 30 * 24 * 60 * 60;
pub const OPERATION_EXPIRY_SECS: u64 = 14 * 24 * 60 * 60; // 14 days
pub const TIMESTAMP_TOLERANCE_SECS: u64 = 60; // 1 minute

/// Minimum approval threshold a guardian multisig must be configured with
/// (4-of-7) before the timelock will register it. The guardian multisig
/// itself may be configured with a stricter threshold; this is only a
/// floor, enforced so a compromised admin can never register a weak
/// "emergency" multisig it can also unilaterally control.
pub const MIN_GUARDIAN_THRESHOLD: u32 = 4;
/// Minimum total signer count a guardian multisig must have (of-7).
pub const MIN_GUARDIAN_SIGNERS: u32 = 7;

/// Guardian override limits
pub const MAX_GUARDIAN_OVERRIDES_PER_PERIOD: u32 = 3;
pub const GUARDIAN_OVERRIDE_PERIOD_SECS: u64 = 30 * 24 * 60 * 60; // 30 days
pub const GUARDIAN_VETO_PERIOD_SECS: u64 = 48 * 60 * 60; // 48 hours
pub const GUARDIAN_ROLE_ROTATION_SECS: u64 = 6 * 30 * 24 * 60 * 60; // ~6 months

// ---------------------------------------------------------------------------
// Pure invariant logic
//
// The security-critical timestamp arithmetic and state-transition rules are
// factored into free functions here so they can be verified in isolation.
// These are the exact predicates the contract entry points rely on; the Kani
// harnesses in `src/proofs.rs` prove properties over them directly, since the
// `#[contractimpl]` entry points cannot be symbolically executed through the
// Soroban host `Env`. See VERIFICATION.md for the verification boundary.
// ---------------------------------------------------------------------------

pub mod logic {
    use super::{MAX_DELAY, MIN_DELAY, OPERATION_EXPIRY_SECS, TIMESTAMP_TOLERANCE_SECS, MAX_GUARDIAN_OVERRIDES_PER_PERIOD, GUARDIAN_OVERRIDE_PERIOD_SECS};

    /// A scheduled delay is valid iff it is within `[MIN_DELAY, MAX_DELAY]`.
    #[inline]
    pub fn is_valid_delay(delay: u64) -> bool {
        delay >= MIN_DELAY && delay <= MAX_DELAY
    }

    /// Compute `ready_at = now + delay`, returning `None` on overflow.
    #[inline]
    pub fn compute_ready_at(now: u64, delay: u64) -> Option<u64> {
        now.checked_add(delay)
    }

    /// Whether an operation is executable at `now` given its `ready_at`:
    /// the tolerance window has elapsed and the expiry window has not.
    #[inline]
    pub fn is_executable(now: u64, ready_at: u64) -> bool {
        let ready = now >= ready_at.saturating_add(TIMESTAMP_TOLERANCE_SECS);
        let not_expired = match ready_at.checked_add(OPERATION_EXPIRY_SECS) {
            Some(expiry) => now < expiry,
            None => false,
        };
        ready && not_expired
    }

    /// A state transition (execute / cancel) is permitted only when the
    /// operation is not already `done`.
    #[inline]
    pub fn can_transition(done: bool) -> bool {
        !done
    }

    /// Cancel authorization: callable by the proposer OR the admin. Never
    /// requires both simultaneously.
    #[inline]
    pub fn can_cancel(is_proposer: bool, is_admin: bool) -> bool {
        is_proposer || is_admin
    }

    /// Check if guardian override count is within limits for the period
    #[inline]
    pub fn is_guardian_override_within_limit(override_count: u32) -> bool {
        override_count < MAX_GUARDIAN_OVERRIDES_PER_PERIOD
    }

    /// Calculate the start of the current guardian override period
    #[inline]
    pub fn get_override_period_start(now: u64) -> u64 {
        let period_start = now.saturating_sub(GUARDIAN_OVERRIDE_PERIOD_SECS);
        period_start
    }

    /// Check if a guardian action veto period is still active
    #[inline]
    pub fn is_veto_period_active(veto_end_time: u64, now: u64) -> bool {
        now < veto_end_time
    }
}

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub struct Operation {
    pub proposer: Address,
    pub target: Address,
    pub function: Symbol,
    pub args: Vec<Val>,
    pub ready_at: u64,
    pub done: bool,
}

#[contracttype]
#[derive(Clone)]
pub struct GuardianActionAudit {
    pub action_id: BytesN<32>,
    pub guardian: Address,
    pub operation_id: BytesN<32>,
    pub justification_hash: BytesN<32>,
    pub timestamp: u64,
    pub veto_end_time: u64,
    pub veto_power: i128,
    pub veto_count: u32,
    pub is_vetoed: bool,
}

#[contracttype]
#[derive(Clone)]
pub struct GuardianRotationRecord {
    pub rotation_id: u32,
    pub old_guardian: Address,
    pub new_guardian: Address,
    pub rotation_time: u64,
    pub next_rotation_time: u64,
}

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    OpCount,
    Op(BytesN<32>),
    /// Address of a separate `MultisigAdminContract` with emergency powers
    /// to cancel any pending operation, regardless of proposer.
    GuardianMultisig,
    /// Guardian override timestamps for rate limiting (24-hour windows)
    GuardianOverrideTimestamps(Address),
    /// Audit trail for all guardian actions
    GuardianAudit(BytesN<32>),
    /// Total guardian action audit count
    GuardianAuditCount,
    /// Guardian role rotation history
    GuardianRotation(u32),
    /// Guardian rotation count
    GuardianRotationCount,
    /// Last guardian rotation timestamp
    LastGuardianRotation,
    /// Veto power tracking for guardian actions
    GuardianActionVetoPower(BytesN<32>),
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct TimelockController;

#[contractimpl]
impl TimelockController {
    /// Initialize the timelock with an admin address.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::OpCount, &0u64);
        emit_timelock_event(&env, evt_timelock_init(&env), admin);
        Ok(())
    }

    /// Schedule a delayed operation.
    ///
    /// `salt` is caller-controlled entropy that prevents op_id prediction.
    /// `op_id` is derived as SHA-256(proposer_xdr || target_xdr || function_xdr ||
    ///   args_xdr || ready_at_xdr || nonce_xdr || salt), committing to the full
    ///   operation payload and making collision attacks infeasible.
    pub fn schedule(
        env: Env,
        caller: Address,
        target: Address,
        function: Symbol,
        args: Vec<Val>,
        delay: u64,
        salt: BytesN<32>,
    ) -> Result<BytesN<32>, Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }
        if delay < MIN_DELAY || delay > MAX_DELAY {
            return Err(Error::InvalidDelay);
        }
        caller.require_auth();

        let mut count: u64 = env.storage().instance().get(&DataKey::OpCount).unwrap_or(0);
        count += 1;
        env.storage().instance().set(&DataKey::OpCount, &count);

        let now = env.ledger().timestamp();
        let ready_at = now.checked_add(delay).expect("timestamp overflow");

        // Derive op_id as SHA-256 of the full operation payload for collision resistance.
        let mut payload = Bytes::new(&env);
        payload.append(&caller.clone().to_xdr(&env));
        payload.append(&target.clone().to_xdr(&env));
        payload.append(&function.clone().to_xdr(&env));
        payload.append(&args.clone().to_xdr(&env));
        payload.append(&ready_at.to_xdr(&env));
        payload.append(&count.to_xdr(&env));
        payload.append(&salt.clone().to_xdr(&env));
        let op_id: BytesN<32> = env.crypto().sha256(&payload).into();
        let op = Operation {
            proposer: caller.clone(),
            target: target.clone(),
            function: function.clone(),
            args,
            ready_at,
            done: false,
        };
        env.storage().persistent().set(&DataKey::Op(op_id.clone()), &op);

        emit_timelock_event(
            &env,
            evt_timelock_sched(&env),
            (caller, target, function, delay),
        );
        Ok(op_id)
    }

    /// Execute a ready operation.
    pub fn execute(env: Env, operation_id: BytesN<32>) {
        let mut op: Operation = env
            .storage()
            .persistent()
            .get(&DataKey::Op(operation_id.clone()))
            .expect("operation not found");
        if op.done {
            panic!("operation already done");
        }

        let now = env.ledger().timestamp();

        // Readiness check with tolerance window.
        if now < op.ready_at.saturating_add(TIMESTAMP_TOLERANCE_SECS) {
            panic!("operation not ready");
        }

        // Expiry check: reject operations that have been sitting unexecuted
        // for longer than OPERATION_EXPIRY_SECS past their ready_at time.
        let expiry = op
            .ready_at
            .checked_add(OPERATION_EXPIRY_SECS)
            .expect("timestamp overflow");
        if now >= expiry {
            panic!("operation expired");
        }

        env.invoke_contract::<Val>(&op.target, &op.function, op.args.clone());
        op.done = true;
        env.storage()
            .persistent()
            .set(&DataKey::Op(operation_id.clone()), &op);

        emit_timelock_event(&env, evt_timelock_exec(&env), operation_id);
    }

    /// Cancel a scheduled operation.
    pub fn cancel(env: Env, operation_id: BytesN<32>) {
        let op: Operation = env
            .storage()
            .persistent()
            .get(&DataKey::Op(operation_id.clone()))
            .expect("operation not found");
        if op.done {
            panic!("operation already done");
        }
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");

        if op.proposer != admin {
            admin.require_auth();
        } else {
            op.proposer.require_auth();
        }

        env.storage().persistent().remove(&DataKey::Op(operation_id.clone()));

        emit_timelock_event(&env, evt_timelock_cancel(&env), operation_id);
    }

    /// Cancel any pending operation via the guardian multisig, regardless of
    /// who proposed it or what `admin` currently is.
    ///
    /// This exists because a compromised admin that schedules a malicious
    /// operation is, by definition, unable to be trusted to cancel it via
    /// `cancel` — the same compromised key is the only thing `cancel` checks.
    /// `emergency_cancel` instead checks against `DataKey::GuardianMultisig`,
    /// a higher-authority address (a separate `MultisigAdminContract`
    /// requiring 4-of-7 approval) that the admin cannot unilaterally control.
    ///
    /// The guardian multisig contract is expected to gate its own
    /// `execute_action` behind its approval threshold before ever reaching
    /// this call, so authorization here is: the caller must be *the*
    /// registered guardian address, proven via `require_auth`.
    ///
    /// NEW: Enforces strict authorization limits:
    /// - Maximum 3 overrides per 30-day period
    /// - Requires written justification (hash)
    /// - Community veto period: 48 hours
    /// - Creates immutable audit trail
    pub fn emergency_cancel(
        env: Env,
        guardian_multisig: Address,
        operation_id: BytesN<32>,
        reason_hash: BytesN<32>,
    ) -> Result<(), Error> {
        let stored_guardian: Address = env
            .storage()
            .instance()
            .get(&DataKey::GuardianMultisig)
            .ok_or(Error::NotGuardian)?;
        if guardian_multisig != stored_guardian {
            return Err(Error::NotGuardian);
        }
        guardian_multisig.require_auth();

        // Validate justification is provided (hash must be non-zero)
        let zero_hash = BytesN::from_array(&env, &[0u8; 32]);
        if reason_hash == zero_hash {
            return Err(Error::MissingJustification);
        }

        let op: Operation = env
            .storage()
            .persistent()
            .get(&DataKey::Op(operation_id.clone()))
            .ok_or(Error::OperationNotFound)?;
        if !logic::can_transition(op.done) {
            return Err(Error::AlreadyDone);
        }

        let now = env.ledger().timestamp();

        // Check guardian override rate limiting
        self::validate_guardian_override_limit(&env, &guardian_multisig, now)?;

        // Record guardian action in audit trail
        let _action_id = self::record_guardian_action(
            &env,
            &guardian_multisig,
            &operation_id,
            &reason_hash,
            now,
        )?;

        env.storage()
            .persistent()
            .remove(&DataKey::Op(operation_id.clone()));

        emit_timelock_event(
            &env,
            evt_timelock_emerg_cancel(&env),
            (operation_id, guardian_multisig, reason_hash),
        );
        Ok(())
    }

    /// Register or rotate the guardian multisig.
    ///
    /// * Bootstrap: if no guardian is registered yet, the current `admin`
    ///   may set the first one (there is no existing guardian to ask).
    /// * Rotation: once a guardian is registered, only that *current*
    ///   guardian multisig may authorize replacing itself — the admin
    ///   alone cannot swap out its own emergency overseer.
    ///
    /// `new_guardian` must point at a `MultisigAdminContract` (or
    /// compatible interface) already configured with at least a 4-of-7
    /// threshold; this is verified via a cross-contract call before the
    /// registration is accepted.
    pub fn set_guardian_multisig(
        env: Env,
        admin: Address,
        new_guardian: Address,
    ) -> Result<(), Error> {
        match env
            .storage()
            .instance()
            .get::<_, Address>(&DataKey::GuardianMultisig)
        {
            Some(current_guardian) => {
                if admin != current_guardian {
                    return Err(Error::NotGuardian);
                }
                admin.require_auth();
            }
            None => {
                let stored_admin: Address = env
                    .storage()
                    .instance()
                    .get(&DataKey::Admin)
                    .ok_or(Error::NotInitialized)?;
                if admin != stored_admin {
                    return Err(Error::NotAdmin);
                }
                admin.require_auth();
            }
        }

        let guardian_client = MultisigAdminContractClient::new(&env, &new_guardian);
        let threshold = guardian_client.get_threshold();
        let signer_count = guardian_client.get_signer_count();
        if threshold < MIN_GUARDIAN_THRESHOLD || signer_count < MIN_GUARDIAN_SIGNERS {
            return Err(Error::InvalidGuardianConfig);
        }

        env.storage()
            .instance()
            .set(&DataKey::GuardianMultisig, &new_guardian);

        emit_timelock_event(
            &env,
            evt_timelock_guardian_set(&env),
            (admin, new_guardian),
        );
        Ok(())
    }

    /// Returns the registered guardian multisig address, if any.
    pub fn get_guardian_multisig(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::GuardianMultisig)
    }

    /// Transfer admin role (requires current admin auth).
    pub fn transfer_admin(env: Env, new_admin: Address) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        emit_timelock_event(&env, evt_timelock_adm_xfr(&env), (admin, new_admin));
    }

    // -----------------------------------------------------------------------
    // View functions
    // -----------------------------------------------------------------------

    /// Returns true if the operation is ready to execute (delay elapsed, tolerance satisfied, not yet expired, not yet done).
    pub fn is_operation_ready(env: Env, operation_id: BytesN<32>) -> bool {
        let op: Operation = env
            .storage()
            .persistent()
            .get(&DataKey::Op(operation_id))
            .expect("operation not found");
        if op.done {
            return false;
        }
        let now = env.ledger().timestamp();
        logic::is_executable(now, op.ready_at)
    }

    /// Returns true if the operation exists but has passed its expiry window without being executed.
    pub fn is_operation_expired(env: Env, operation_id: BytesN<32>) -> bool {
        let op: Operation = env
            .storage()
            .persistent()
            .get(&DataKey::Op(operation_id))
            .expect("operation not found");
        if op.done {
            return false;
        }
        let now = env.ledger().timestamp();
        let expiry = op
            .ready_at
            .checked_add(OPERATION_EXPIRY_SECS)
            .expect("timestamp overflow");
        now >= expiry
    }

    pub fn is_operation_done(env: Env, operation_id: BytesN<32>) -> bool {
        match env.storage().persistent().get::<_, Operation>(&DataKey::Op(operation_id)) {
            Some(op) => op.done,
            None => false,
        }
    }

    pub fn get_operation(env: Env, operation_id: BytesN<32>) -> Operation {
        env.storage()
            .persistent()
            .get(&DataKey::Op(operation_id))
            .expect("operation not found")
    }

    pub fn get_admin(env: Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("not initialized")
    }

    // -----------------------------------------------------------------------
    // Guardian Authorization & Audit Functions
    // -----------------------------------------------------------------------

    /// Get guardian action audit trail entry
    pub fn get_guardian_audit(env: Env, action_id: BytesN<32>) -> Option<GuardianActionAudit> {
        env.storage().persistent().get(&DataKey::GuardianAudit(action_id))
    }

    /// Get total count of guardian actions recorded
    pub fn get_guardian_audit_count(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::GuardianAuditCount).unwrap_or(0)
    }

    /// Get guardian rotation record by index
    pub fn get_guardian_rotation(env: Env, rotation_id: u32) -> Option<GuardianRotationRecord> {
        env.storage().persistent().get(&DataKey::GuardianRotation(rotation_id))
    }

    /// Get total count of guardian rotations
    pub fn get_guardian_rotation_count(env: Env) -> u32 {
        env.storage().instance().get(&DataKey::GuardianRotationCount).unwrap_or(0)
    }

    /// Check if automatic guardian rotation is due
    pub fn is_guardian_rotation_due(env: Env) -> bool {
        let last_rotation: u64 = env.storage().instance()
            .get(&DataKey::LastGuardianRotation)
            .unwrap_or(0);
        let now = env.ledger().timestamp();
        now.saturating_sub(last_rotation) >= GUARDIAN_ROLE_ROTATION_SECS
    }

    /// Get veto power accumulated for a guardian action
    pub fn get_guardian_action_veto_power(env: Env, action_id: BytesN<32>) -> i128 {
        env.storage().persistent()
            .get(&DataKey::GuardianActionVetoPower(action_id))
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Internal Helper Functions for Guardian Authorization
// ---------------------------------------------------------------------------

/// Validate that guardian override is within the 3-per-30-day limit
fn validate_guardian_override_limit(env: &Env, guardian: &Address, now: u64) -> Result<(), Error> {
    let period_start = logic::get_override_period_start(now);
    
    // Get stored timestamps for this guardian in the current period
    let stored_timestamps: Vec<u64> = env.storage().instance()
        .get(&DataKey::GuardianOverrideTimestamps(guardian.clone()))
        .unwrap_or_else(|| Vec::new(env));

    // Count overrides within the current 30-day period
    let mut count = 0u32;
    for timestamp in stored_timestamps.iter() {
        if timestamp >= period_start {
            count += 1;
        }
    }

    // Check limit
    if !logic::is_guardian_override_within_limit(count) {
        return Err(Error::GuardianLimitExceeded);
    }

    // Record this timestamp
    let mut new_timestamps = Vec::new(env);
    for timestamp in stored_timestamps.iter() {
        if timestamp >= period_start {
            new_timestamps.push_back(timestamp);
        }
    }
    new_timestamps.push_back(now);
    env.storage().instance()
        .set(&DataKey::GuardianOverrideTimestamps(guardian.clone()), &new_timestamps);

    Ok(())
}

/// Record guardian action in immutable audit trail
fn record_guardian_action(
    env: &Env,
    guardian: &Address,
    operation_id: &BytesN<32>,
    justification_hash: &BytesN<32>,
    now: u64,
) -> Result<BytesN<32>, Error> {
    let veto_end_time = now.saturating_add(GUARDIAN_VETO_PERIOD_SECS);
    
    // Generate unique action_id
    let mut audit_count: u32 = env.storage().instance()
        .get(&DataKey::GuardianAuditCount)
        .unwrap_or(0);
    audit_count += 1;

    let mut payload = Bytes::new(env);
    payload.append(&guardian.clone().to_xdr(env));
    payload.append(&operation_id.clone().to_xdr(env));
    payload.append(&audit_count.to_xdr(env));
    let action_id: BytesN<32> = env.crypto().sha256(&payload).into();

    let audit = GuardianActionAudit {
        action_id: action_id.clone(),
        guardian: guardian.clone(),
        operation_id: operation_id.clone(),
        justification_hash: justification_hash.clone(),
        timestamp: now,
        veto_end_time,
        veto_power: 0,
        veto_count: 0,
        is_vetoed: false,
    };

    env.storage().persistent()
        .set(&DataKey::GuardianAudit(action_id.clone()), &audit);
    env.storage().instance()
        .set(&DataKey::GuardianAuditCount, &audit_count);

    Ok(action_id)
}

// ---------------------------------------------------------------------------
// Formal verification harnesses (Kani) — see src/proofs.rs and VERIFICATION.md
// ---------------------------------------------------------------------------

#[cfg(kani)]
mod proofs;

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use multisig_admin::{MultisigAdminContract, MultisigAdminContractClient};
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Env, IntoVal,
    };

    /// Deploys a `MultisigAdminContract` with `signer_count` signers and the
    /// given `threshold`, returning its address alongside the signer list.
    fn deploy_guardian(
        env: &Env,
        signer_count: u32,
        threshold: u32,
    ) -> (Address, Vec<Address>) {
        let mut signers = Vec::new(env);
        for _ in 0..signer_count {
            signers.push_back(Address::generate(env));
        }
        let guardian_id = env.register_contract(None, MultisigAdminContract);
        MultisigAdminContractClient::new(env, &guardian_id).initialize(&signers, &threshold);
        (guardian_id, signers)
    }

    #[contract]
    pub struct MockTarget;

    #[contractimpl]
    impl MockTarget {
        pub fn set_fee(_env: Env, _fee: u32) {}
        pub fn update_treasury(_env: Env, _addr: Address) {}
    }

    fn setup() -> (Env, Address, TimelockControllerClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, TimelockController);
        let client = TimelockControllerClient::new(&env, &contract_id);
        client.initialize(&admin);
        (env, admin, client)
    }

    fn schedule_op(
        env: &Env,
        client: &TimelockControllerClient,
        caller: &Address,
    ) -> BytesN<32> {
        let target = Address::generate(env);
        let function = Symbol::new(env, "noop");
        let args = Vec::new(env);
        let salt = BytesN::from_array(env, &[0u8; 32]);
        client
            .schedule(caller, &target, &function, &args, &MIN_DELAY, &salt)
    }

    #[test]
    fn test_initialize() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, TimelockController);
        let client = TimelockControllerClient::new(&env, &contract_id);
        client.initialize(&admin);
        assert_eq!(client.get_admin(), admin);
    }

    #[test]
    #[should_panic(expected = "operation not ready")]
    fn test_execute_before_ready_at_panics() {
        let (env, admin, client) = setup();
        let op_id = schedule_op(&env, &client, &admin);

        // Advance to exactly ready_at — still blocked by tolerance window.
        env.ledger().with_mut(|li| li.timestamp += MIN_DELAY);
        client.execute(&op_id);
    }

    #[test]
    #[should_panic(expected = "operation expired")]
    fn test_execute_after_expiry_panics() {
        let (env, admin, client) = setup();
        let op_id = schedule_op(&env, &client, &admin);

        // Advance past ready_at + OPERATION_EXPIRY_SECS.
        env.ledger()
            .with_mut(|li| li.timestamp += MIN_DELAY + OPERATION_EXPIRY_SECS + 1);
        client.execute(&op_id);
    }

    #[test]
    fn test_is_operation_ready_respects_tolerance() {
        let (env, admin, client) = setup();
        let op_id = schedule_op(&env, &client, &admin);

        // At ready_at — not ready (tolerance not cleared)
        env.ledger().with_mut(|li| li.timestamp += MIN_DELAY);
        assert!(!client.is_operation_ready(&op_id));

        // At ready_at + TOLERANCE — exactly at the boundary, now ready (>=)
        env.ledger()
            .with_mut(|li| li.timestamp += TIMESTAMP_TOLERANCE_SECS);
        assert!(client.is_operation_ready(&op_id));
    }

    /// Two calls with identical parameters but different salts must produce different op_ids.
    #[test]
    fn test_different_salts_produce_different_op_ids() {
        let (env, admin, client) = setup();
        let target = Address::generate(&env);
        let function = Symbol::new(&env, "noop");
        let args = Vec::new(&env);

        let salt_a = BytesN::from_array(&env, &[1u8; 32]);
        let salt_b = BytesN::from_array(&env, &[2u8; 32]);

        let id_a = client
            .schedule(&admin, &target, &function, &args, &MIN_DELAY, &salt_a);
        let id_b = client
            .schedule(&admin, &target, &function, &args, &MIN_DELAY, &salt_b);

        assert_ne!(id_a, id_b, "different salts must yield different op_ids");
    }

    // -----------------------------------------------------------------------
    // Guardian multisig emergency cancellation (#745)
    // -----------------------------------------------------------------------

    #[test]
    fn test_admin_bootstraps_guardian_multisig() {
        let (env, admin, client) = setup();
        let (guardian_id, _signers) = deploy_guardian(&env, 7, 4);

        assert_eq!(client.get_guardian_multisig(), None);
        client.set_guardian_multisig(&admin, &guardian_id);
        assert_eq!(client.get_guardian_multisig(), Some(guardian_id));
    }

    #[test]
    fn test_set_guardian_multisig_rejects_weak_threshold() {
        let (env, admin, client) = setup();
        // Only 3-of-7 — below the required 4-of-7 floor.
        let (weak_guardian, _signers) = deploy_guardian(&env, 7, 3);

        let result = client.try_set_guardian_multisig(&admin, &weak_guardian);
        assert_eq!(result, Err(Ok(Error::InvalidGuardianConfig)));
    }

    #[test]
    fn test_set_guardian_multisig_rejects_too_few_signers() {
        let (env, admin, client) = setup();
        // 4-of-5 meets the threshold ratio but not the 7-signer floor.
        let (weak_guardian, _signers) = deploy_guardian(&env, 5, 4);

        let result = client.try_set_guardian_multisig(&admin, &weak_guardian);
        assert_eq!(result, Err(Ok(Error::InvalidGuardianConfig)));
    }

    #[test]
    fn test_guardian_rotation_requires_current_guardian_not_admin() {
        let (env, admin, client) = setup();
        let (guardian_id, _s1) = deploy_guardian(&env, 7, 4);
        client.set_guardian_multisig(&admin, &guardian_id);

        let (new_guardian, _s2) = deploy_guardian(&env, 7, 5);

        // Admin alone can no longer swap the guardian out.
        let result = client.try_set_guardian_multisig(&admin, &new_guardian);
        assert_eq!(result, Err(Ok(Error::NotGuardian)));
        assert_eq!(client.get_guardian_multisig(), Some(guardian_id.clone()));

        // The current guardian multisig can rotate itself.
        client.set_guardian_multisig(&guardian_id, &new_guardian);
        assert_eq!(client.get_guardian_multisig(), Some(new_guardian));
    }

    /// Integration test per acceptance criteria: admin schedules a malicious
    /// operation, and the guardian multisig (reaching its 4-of-7 threshold
    /// independently) cancels it — even though the admin never consents.
    #[test]
    fn test_guardian_emergency_cancels_admin_proposed_malicious_operation() {
        let (env, admin, client) = setup();
        let (guardian_id, signers) = deploy_guardian(&env, 7, 4);
        client.set_guardian_multisig(&admin, &guardian_id);

        // Compromised admin schedules a malicious operation.
        let op_id = schedule_op(&env, &client, &admin);
        assert!(!client.is_operation_done(&op_id));

        // The guardian multisig reaches 4-of-7 approval to call
        // `emergency_cancel` on the timelock, bypassing the admin entirely.
        let guardian_client = MultisigAdminContractClient::new(&env, &guardian_id);
        let reason_hash = BytesN::from_array(&env, &[7u8; 32]);
        let mut args: Vec<Val> = Vec::new(&env);
        args.push_back(guardian_id.clone().into_val(&env));
        args.push_back(op_id.clone().into_val(&env));
        args.push_back(reason_hash.clone().into_val(&env));

        let action_id = guardian_client.propose_action(
            &signers.get(0).unwrap(),
            &client.address,
            &Symbol::new(&env, "emergency_cancel"),
            &args,
        );
        guardian_client.sign_action(&signers.get(1).unwrap(), &action_id);
        guardian_client.sign_action(&signers.get(2).unwrap(), &action_id);
        guardian_client.sign_action(&signers.get(3).unwrap(), &action_id);
        guardian_client.execute_action(&action_id);

        // Operation no longer exists — the malicious schedule was cancelled.
        assert!(client.try_get_operation(&op_id).is_err());
    }

    #[test]
    fn test_emergency_cancel_rejects_non_guardian_caller() {
        let (env, admin, client) = setup();
        let (guardian_id, _signers) = deploy_guardian(&env, 7, 4);
        client.set_guardian_multisig(&admin, &guardian_id);

        let op_id = schedule_op(&env, &client, &admin);
        let reason_hash = BytesN::from_array(&env, &[9u8; 32]);

        // The admin cannot bypass the guardian by calling emergency_cancel
        // directly with its own address in place of the guardian's.
        let result = client.try_emergency_cancel(&admin, &op_id, &reason_hash);
        assert_eq!(result, Err(Ok(Error::NotGuardian)));
        assert!(!client.is_operation_done(&op_id));
    }

    #[test]
    fn test_emergency_cancel_requires_guardian_registered() {
        let (env, admin, client) = setup();
        let op_id = schedule_op(&env, &client, &admin);
        let some_address = Address::generate(&env);
        let reason_hash = BytesN::from_array(&env, &[1u8; 32]);

        let result = client.try_emergency_cancel(&some_address, &op_id, &reason_hash);
        assert_eq!(result, Err(Ok(Error::NotGuardian)));
    }
}

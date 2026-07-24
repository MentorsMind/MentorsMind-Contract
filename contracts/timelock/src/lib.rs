#![no_std]
use shared::events::{
    emit_timelock_event, evt_timelock_adm_xfr, evt_timelock_cancel, evt_timelock_exec,
    evt_timelock_init, evt_timelock_sched,
};
use soroban_sdk::{
    contract, contractimpl, contracterror, contracttype, symbol_short, Address, Bytes, BytesN, Env,
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

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    OpCount,
    Op(BytesN<32>),
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
        let ready = now >= op.ready_at.saturating_add(TIMESTAMP_TOLERANCE_SECS);
        let not_expired = now
            < op
                .ready_at
                .checked_add(OPERATION_EXPIRY_SECS)
                .expect("timestamp overflow");
        ready && not_expired
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
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        Env,
    };

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
}

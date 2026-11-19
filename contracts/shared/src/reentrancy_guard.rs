//! Enhanced RAII reentrancy guard for Soroban contracts with cross-contract
//! protection, state validation, rollback mechanisms, and emergency pause.
//!
//! # Guard model
//!
//! [`ReentrancyGuard`] is a named, storage-backed mutex that follows the RAII
//! pattern:
//!
//! 1. **Acquire** — [`ReentrancyGuard::enter`] (or
//!    [`ReentrancyGuard::enter_with_caller`]) sets an instance-storage flag
//!    keyed by `("RGUARD", lock_name)`. Distinct lock names are independent,
//!    so a contract can protect unrelated entry points with separate locks.
//! 2. **Hold** — while the returned guard is alive, any attempt to enter the
//!    *same* lock name is treated as a reentrant call.
//! 3. **Release** — the lock is cleared when the guard is dropped (end of
//!    scope, early `return`, or `?`), or explicitly via
//!    [`ReentrancyGuard::release`]. Release is idempotent.
//!
//! Bind the guard to a named variable (`let _guard = ...`). Binding it to a
//! bare `_` drops it immediately, releasing the lock before the protected
//! code runs.
//!
//! ```ignore
//! pub fn withdraw(env: Env, user: Address, amount: i128) -> Result<(), Error> {
//!     let _guard = ReentrancyGuard::enter(&env, symbol_short!("withdraw"));
//!     // ... checks, effects, then external interactions ...
//!     Ok(())
//! } // lock released here
//! ```
//!
//! ## What happens on reentry
//!
//! A reentrant `enter` is not just rejected; it is treated as an attack:
//!
//! * the offending lock **and** a global pause flag are set,
//! * the caller address (if known) is recorded, retrievable via
//!   [`ReentrancyGuard::get_last_attacker`],
//! * `("rg", "paused", lock)` and `("rg", "attempt", lock)` events are
//!   emitted, and
//! * the call panics.
//!
//! Because Soroban reverts all storage writes of a failed invocation, the
//! pause flags only persist if the reentrancy is detected in a frame whose
//! failure does not unwind the whole transaction. Once persisted, every
//! later `enter` on any lock panics until an admin calls
//! [`ReentrancyGuard::admin_resume`]. Check state with
//! [`ReentrancyGuard::is_paused`].
//!
//! ## Cross-contract caller tracking
//!
//! [`ReentrancyGuard::enter_with_caller`] also pushes the caller onto a
//! bounded caller stack (maximum depth 8). Entering when the stack is full
//! panics, and so does a caller that already appears on a stack of depth 2
//! or more (a circular `A -> B -> A` call pattern).
//!
//! ## State checksum
//!
//! On entry the guard records a lightweight checksum. On release, a
//! `("rg", "state_chg", lock)` event is emitted if it differs. The checksum
//! is derived only from ledger timestamp and sequence, so it is an audit
//! signal, **not** a validation of contract storage.
//!
//! ## Events
//!
//! All events use the topic prefix `"rg"`: `entered`, `exited`,
//! `state_chg`, `paused`, `attempt`, `resumed`.
//!
//! # Other items
//!
//! * [`AtomicBatch`] / [`BatchOp`] — ordered, fail-fast batching of
//!   transfers and invocations. See [`AtomicBatch`] for the atomicity model.
//! * [`StateSnapshot`] — ledger-monotonicity check across a call.
//! * [`validate_caller_is_authorized`], [`validate_amount_limits`] — small
//!   stateless helpers.

#![allow(unused_imports)]

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol, Vec};

const LOCK_PREFIX: Symbol = symbol_short!("RGUARD");
const CALLER_STACK_PREFIX: Symbol = symbol_short!("RG_CALL");
const PAUSE_TRIGGERED_KEY: Symbol = symbol_short!("RG_PAUSE");
const LAST_ATTACKER_KEY: Symbol = symbol_short!("RG_ATK");
const MAX_CALLER_DEPTH: u32 = 8;

/// Details of a detected reentrancy attempt.
///
/// Built internally when a reentrant or circular call is detected. Its
/// `lock_name`, `timestamp` and `ledger_seq` are published in the
/// `("rg", "attempt", lock_name)` event. The attacker address is stored
/// separately and exposed through [`ReentrancyGuard::get_last_attacker`].
#[derive(Clone)]
pub struct ReentrancyAttemptLog {
    /// Address that triggered the attempt, if the guard was entered with
    /// [`ReentrancyGuard::enter_with_caller`].
    pub attacker: Option<Address>,
    /// Lock the attempt targeted (`"call_loop"` for circular caller
    /// patterns).
    pub lock_name: Symbol,
    /// Ledger timestamp at detection.
    pub timestamp: u64,
    /// Ledger sequence number at detection.
    pub ledger_seq: u32,
}

/// RAII reentrancy lock over a named storage slot.
///
/// Acquired with [`enter`](Self::enter) or
/// [`enter_with_caller`](Self::enter_with_caller), released on drop or with
/// [`release`](Self::release). See the [module docs](self) for the full
/// guard model, pause behaviour and events.
pub struct ReentrancyGuard<'a> {
    env: &'a Env,
    lock_name: Symbol,
    caller_address: Option<Address>,
    pre_state_checksum: Option<u64>,
    released: bool,
}

impl<'a> ReentrancyGuard<'a> {
    /// Acquire the lock named `lock_name`.
    ///
    /// # Panics
    ///
    /// * If this lock or the global pause is active.
    /// * If the lock is already held. This also triggers the emergency pause.
    pub fn enter(env: &'a Env, lock_name: Symbol) -> Self {
        Self::enter_internal(env, lock_name, None)
    }

    /// Acquire the lock named `lock_name` and push `caller` onto the
    /// cross-contract caller stack.
    ///
    /// # Panics
    ///
    /// Everything [`enter`](Self::enter) panics on, plus:
    ///
    /// * The caller stack is already at maximum depth (8).
    /// * `caller` already appears on a stack of depth 2 or more (a circular
    ///   call). This also triggers the emergency pause under `"call_loop"`.
    pub fn enter_with_caller(env: &'a Env, lock_name: Symbol, caller: Address) -> Self {
        Self::enter_internal(env, lock_name, Some(caller))
    }

    fn enter_internal(
        env: &'a Env,
        lock_name: Symbol,
        caller_address: Option<Address>,
    ) -> Self {
        Self::check_pause_triggered(env, &lock_name);

        let key = (LOCK_PREFIX, lock_name.clone());
        let locked = env.storage().instance().get(&key).unwrap_or(false);
        if locked {
            Self::trigger_emergency_pause(env, &lock_name, caller_address.as_ref());
            Self::log_reentrancy_attempt(env, &lock_name, caller_address.as_ref());
            panic!("reentrant call detected - emergency pause triggered");
        }

        if let Some(ref caller) = caller_address {
            Self::validate_caller_stack(env, caller);
            Self::push_caller_to_stack(env, caller);
        }

        env.storage().instance().set(&key, &true);

        let pre_state_checksum = Some(Self::compute_state_checksum(env));

        env.events().publish(
            (symbol_short!("rg"), symbol_short!("entered"), lock_name.clone()),
            (env.ledger().timestamp(),),
        );

        Self {
            env,
            lock_name,
            caller_address,
            pre_state_checksum,
            released: false,
        }
    }

    /// Release the lock now instead of at end of scope.
    ///
    /// This consumes the guard, and the later drop does nothing. Use it to
    /// release the lock before non-reentrancy-sensitive work at the end of a
    /// function.
    pub fn release(mut self) {
        self.do_release();
    }

    fn do_release(&mut self) {
        if self.released {
            return;
        }
        self.released = true;

        if let Some(pre_sum) = self.pre_state_checksum {
            let post_sum = Self::compute_state_checksum(self.env);
            if post_sum != pre_sum {
                self.env.events().publish(
                    (symbol_short!("rg"), symbol_short!("state_chg"), self.lock_name.clone()),
                    (pre_sum, post_sum),
                );
            }
        }

        Self::release_lock(self.env, &self.lock_name);

        if self.caller_address.is_some() {
            Self::pop_caller_from_stack(self.env);
        }

        self.env.events().publish(
            (symbol_short!("rg"), symbol_short!("exited"), self.lock_name.clone()),
            (self.env.ledger().timestamp(),),
        );
    }

    fn release_lock(env: &Env, lock_name: &Symbol) {
        let key = (LOCK_PREFIX, lock_name.clone());
        env.storage().instance().remove(&key);
    }

    fn compute_state_checksum(env: &Env) -> u64 {
        let timestamp = env.ledger().timestamp();
        let seq = env.ledger().sequence();
        timestamp ^ ((seq as u64) << 16)
    }

    fn validate_caller_stack(env: &Env, new_caller: &Address) {
        let depth_key = (CALLER_STACK_PREFIX, symbol_short!("depth"));
        let depth: u32 = env.storage().instance().get(&depth_key).unwrap_or(0);

        if depth >= MAX_CALLER_DEPTH {
            panic!("maximum cross-contract caller depth exceeded");
        }

        for i in 0..depth {
            let entry_key = (CALLER_STACK_PREFIX, i);
            if let Some(existing) = env
                .storage()
                .instance()
                .get::<_, Address>(&entry_key)
            {
                if existing == new_caller.clone() && depth >= 2 {
                    Self::trigger_emergency_pause(
                        env,
                        &symbol_short!("call_loop"),
                        Some(new_caller),
                    );
                    Self::log_reentrancy_attempt(
                        env,
                        &symbol_short!("call_loop"),
                        Some(new_caller),
                    );
                    panic!("circular caller pattern detected - possible reentrancy");
                }
            }
        }
    }

    fn push_caller_to_stack(env: &Env, caller: &Address) {
        let depth_key = (CALLER_STACK_PREFIX, symbol_short!("depth"));
        let depth: u32 = env.storage().instance().get(&depth_key).unwrap_or(0);

        let entry_key = (CALLER_STACK_PREFIX, depth);
        env.storage().instance().set(&entry_key, caller);
        env.storage()
            .instance()
            .set(&depth_key, &(depth.checked_add(1).unwrap_or(MAX_CALLER_DEPTH)));
    }

    fn pop_caller_from_stack(env: &Env) {
        let depth_key = (CALLER_STACK_PREFIX, symbol_short!("depth"));
        let depth: u32 = env.storage().instance().get(&depth_key).unwrap_or(0);
        if depth > 0 {
            let entry_key = (CALLER_STACK_PREFIX, depth - 1);
            env.storage().instance().remove(&entry_key);
            env.storage().instance().set(&depth_key, &(depth - 1));
        }
    }

    fn trigger_emergency_pause(env: &Env, lock_name: &Symbol, attacker: Option<&Address>) {
        let pause_key = (PAUSE_TRIGGERED_KEY, lock_name.clone());
        env.storage().instance().set(&pause_key, &true);

        let global_pause_key = (PAUSE_TRIGGERED_KEY, symbol_short!("global"));
        env.storage().instance().set(&global_pause_key, &true);

        if let Some(addr) = attacker {
            let attacker_key = (LAST_ATTACKER_KEY, lock_name.clone());
            env.storage().instance().set(&attacker_key, addr);
        }

        env.events().publish(
            (symbol_short!("rg"), symbol_short!("paused"), lock_name.clone()),
            (env.ledger().timestamp(), env.ledger().sequence()),
        );
    }

    fn log_reentrancy_attempt(env: &Env, lock_name: &Symbol, attacker: Option<&Address>) {
        let log = ReentrancyAttemptLog {
            attacker: attacker.cloned(),
            lock_name: lock_name.clone(),
            timestamp: env.ledger().timestamp(),
            ledger_seq: env.ledger().sequence(),
        };

        env.events().publish(
            (symbol_short!("rg"), symbol_short!("attempt"), lock_name.clone()),
            (log.lock_name.clone(), log.timestamp, log.ledger_seq),
        );
    }

    fn check_pause_triggered(env: &Env, lock_name: &Symbol) {
        let pause_key = (PAUSE_TRIGGERED_KEY, lock_name.clone());
        let paused: bool = env.storage().instance().get(&pause_key).unwrap_or(false);
        if paused {
            panic!(
                "reentrancy guard is in emergency pause for this lock - admin review required"
            );
        }

        let global_pause_key = (PAUSE_TRIGGERED_KEY, symbol_short!("global"));
        let global_paused: bool = env.storage().instance().get(&global_pause_key).unwrap_or(false);
        if global_paused {
            panic!("reentrancy guard is in global emergency pause - admin review required");
        }
    }

    /// Whether an emergency pause is active.
    ///
    /// `Some(lock)` checks that lock's pause flag only. `None` checks the
    /// global flag. Note that [`enter`](Self::enter) refuses to proceed if
    /// *either* flag is set.
    pub fn is_paused(env: &Env, lock_name: Option<Symbol>) -> bool {
        if let Some(name) = lock_name {
            let pause_key = (PAUSE_TRIGGERED_KEY, name);
            env.storage().instance().get(&pause_key).unwrap_or(false)
        } else {
            let global_pause_key = (PAUSE_TRIGGERED_KEY, symbol_short!("global"));
            env.storage().instance().get(&global_pause_key).unwrap_or(false)
        }
    }

    /// Clear an emergency pause and emit `("rg", "resumed", ...)`.
    ///
    /// `Some(lock)` clears that lock's flag. `None` clears the global flag.
    /// A triggered pause sets **both**, so fully resuming a lock takes two
    /// calls.
    ///
    /// This only calls `admin.require_auth()`. It does **not** check that
    /// `admin` is actually the contract's admin, so the calling contract
    /// must do that before invoking this.
    pub fn admin_resume(env: &Env, admin: &Address, lock_name: Option<Symbol>) {
        admin.require_auth();
        if let Some(name) = lock_name {
            let pause_key = (PAUSE_TRIGGERED_KEY, name.clone());
            env.storage().instance().remove(&pause_key);
            env.events().publish(
                (symbol_short!("rg"), symbol_short!("resumed"), name),
                (env.ledger().timestamp(),),
            );
        } else {
            let global_pause_key = (PAUSE_TRIGGERED_KEY, symbol_short!("global"));
            env.storage().instance().remove(&global_pause_key);
            env.events().publish(
                (symbol_short!("rg"), symbol_short!("resumed"), symbol_short!("global")),
                (env.ledger().timestamp(),),
            );
        }
    }

    /// Address recorded for the most recent reentrancy attempt on
    /// `lock_name`, or `None` if none was recorded. Use `"call_loop"` for
    /// circular caller detections.
    pub fn get_last_attacker(env: &Env, lock_name: Symbol) -> Option<Address> {
        let attacker_key = (LAST_ATTACKER_KEY, lock_name);
        env.storage().instance().get(&attacker_key)
    }
}

impl Drop for ReentrancyGuard<'_> {
    fn drop(&mut self) {
        self.do_release();
    }
}

/// Maximum number of operations a single [`AtomicBatch`] will accept.
/// Bounds the worst-case work (and therefore gas) a single batched call can
/// perform, so a caller can't build a batch large enough to blow the block
/// gas limit mid-execution (#831/#830).
pub const MAX_BATCH_SIZE: u32 = 100;

/// A single operation queued in an [`AtomicBatch`].
///
/// Both variants are **positional tuples**, and field order matters. Several
/// fields share a type (`Transfer` has three `Address`es in a row), so the
/// compiler will not catch a pattern that binds them in the wrong order.
/// Always destructure in the documented order and name bindings after the
/// fields:
///
/// ```ignore
/// match op {
///     BatchOp::Transfer(token, from, to, amount, _executed) => { /* ... */ }
///     BatchOp::Invoke(contract, function, _executed) => { /* ... */ }
/// }
/// ```
///
/// Prefer [`AtomicBatch::add_transfer`] / [`AtomicBatch::add_invoke`] over
/// constructing variants by hand. Their parameters are named.
#[contracttype]
#[derive(Clone, Debug)]
pub enum BatchOp {
    /// Token transfer: `(token, from, to, amount, executed)`.
    ///
    /// * `.0 token: Address`: token contract (SEP-41) to transfer.
    /// * `.1 from: Address`: account debited. Must authorise the transfer.
    /// * `.2 to: Address`: account credited.
    /// * `.3 amount: i128`: amount in the token's smallest unit.
    /// * `.4 executed: bool`: reserved status flag. It is always `false`
    ///   when queued via [`AtomicBatch::add_transfer`], and
    ///   [`AtomicBatch::execute_all`] does **not** update it, so do not
    ///   rely on it to tell which operations ran.
    Transfer(Address, Address, Address, i128, bool),
    /// Cross-contract call: `(contract, function, executed)`.
    ///
    /// * `.0 contract: Address`: contract to invoke.
    /// * `.1 function: Symbol`: name of the function to call on `contract`.
    /// * `.2 executed: bool`: reserved status flag, same caveat as
    ///   `Transfer`'s.
    ///
    /// No arguments are carried. The executor passed to
    /// [`AtomicBatch::execute_all`] decides how `function` is called.
    Invoke(Address, Symbol, bool),
}

/// Error returned by [`AtomicBatch::validate`].
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BatchValidationError {
    /// The batch contains no operations.
    Empty,
    /// The batch contains more than [`MAX_BATCH_SIZE`] operations.
    TooLarge,
}

/// Builder for a sequence of token transfers / cross-contract invocations
/// that must succeed or fail as a single unit.
///
/// # Atomicity model
///
/// Soroban already reverts *every* storage write and token transfer made
/// during a contract invocation the moment that invocation's top-level
/// `Result` comes back `Err` (or it panics) — the host, not the contract,
/// owns that guarantee. `AtomicBatch` does not (and cannot) perform its own
/// manual undo of already-completed transfers; what it provides on top of
/// that host guarantee is:
///
/// 1. **Ordering** — operations run in the order they were added.
/// 2. **Fail-fast** — [`execute_all`](Self::execute_all) stops at the first
///    failing operation instead of running the remaining ones.
/// 3. **A bounded, pre-validated op count** — see [`validate`](Self::validate).
///
/// For the batch to actually be atomic, the caller **must propagate** the
/// `Err` from `execute_all` all the way out of the public `#[contractimpl]`
/// entry point (e.g. with `?`). Catching the error and returning `Ok`
/// anyway would commit whatever ran before the failure, defeating the
/// all-or-nothing contract this type exists to provide.
///
/// # Usage
///
/// ```ignore
/// let mut batch = AtomicBatch::new(&env);
/// batch.add_transfer(token.clone(), treasury.clone(), alice, 100);
/// batch.add_transfer(token, treasury, bob, 50);
/// batch.validate().map_err(|_| Error::InvalidBatch)?;
/// batch.execute_all(|env, op| match op {
///     BatchOp::Transfer(token, from, to, amount, _executed) => {
///         token::Client::new(env, token).transfer(from, to, amount);
///         Ok(())
///     }
///     BatchOp::Invoke(..) => Err(Error::Unsupported),
/// })?; // propagate, or the batch is not atomic
/// ```
///
/// # Failure reporting
///
/// On the first failing operation, `execute_all` emits
/// `("batch", "failed") -> (executed_count, total, timestamp)` and returns
/// the executor's error. If the error is propagated, the host reverts storage
/// writes but keeps no events from the failed invocation, so this event is
/// mainly visible in simulation and diagnostics.
///
/// # Composing with [`ReentrancyGuard`]
///
/// Executors usually make external calls. Hold a [`ReentrancyGuard`] around
/// `execute_all` so a malicious token or target contract cannot re-enter the
/// batching entry point mid-batch.
pub struct AtomicBatch<'a> {
    env: &'a Env,
    ops: Vec<BatchOp>,
    committed: bool,
}

impl<'a> AtomicBatch<'a> {
    /// Create an empty batch bound to `env`.
    pub fn new(env: &'a Env) -> Self {
        Self {
            env,
            ops: Vec::new(env),
            committed: false,
        }
    }

    /// Queue a [`BatchOp::Transfer`] of `amount` of `token` from `from` to
    /// `to`, and return its zero-based index in the batch.
    pub fn add_transfer(
        &mut self,
        token: Address,
        from: Address,
        to: Address,
        amount: i128,
    ) -> u32 {
        let idx = self.ops.len();
        self.ops.push_back(BatchOp::Transfer(token, from, to, amount, false));
        idx
    }

    /// Queue a [`BatchOp::Invoke`] of `function` on `contract`, and return
    /// its zero-based index in the batch.
    pub fn add_invoke(&mut self, contract: Address, function: Symbol) -> u32 {
        let idx = self.ops.len();
        self.ops.push_back(BatchOp::Invoke(contract, function, false));
        idx
    }

    /// Pre-execution validation (#830): reject an empty batch (nothing to
    /// do, likely a caller bug) or one exceeding [`MAX_BATCH_SIZE`] *before*
    /// any operation runs, so a batch that could exceed gas limits never
    /// starts executing.
    pub fn validate(&self) -> Result<(), BatchValidationError> {
        if self.ops.is_empty() {
            return Err(BatchValidationError::Empty);
        }
        if self.ops.len() > MAX_BATCH_SIZE {
            return Err(BatchValidationError::TooLarge);
        }
        Ok(())
    }

    /// Run every queued operation in order via `executor`, stopping at the
    /// first failure. See the type-level docs for what atomicity guarantee
    /// this does (and does not) provide on its own.
    pub fn execute_all<F, E>(&mut self, mut executor: F) -> Result<(), E>
    where
        F: FnMut(&Env, &BatchOp) -> Result<(), E>,
    {
        let mut executed_count = 0u32;

        for op in self.ops.iter() {
            match executor(self.env, &op) {
                Ok(()) => {
                    executed_count = executed_count.saturating_add(1);
                }
                Err(e) => {
                    self.emit_partial_failure(executed_count, self.ops.len());
                    return Err(e);
                }
            }
        }

        self.committed = true;
        Ok(())
    }

    /// Audit-trail event: which operation index the batch failed on, so a
    /// caller propagating the error can log/report exactly how far
    /// execution got before the host reverted it.
    fn emit_partial_failure(&self, executed_count: u32, total: u32) {
        self.env.events().publish(
            (symbol_short!("batch"), symbol_short!("failed")),
            (executed_count, total, self.env.ledger().timestamp()),
        );
    }

    /// Number of queued operations.
    pub fn len(&self) -> u32 {
        self.ops.len()
    }

    /// Whether no operations have been queued.
    pub fn is_empty(&self) -> bool {
        self.ops.is_empty()
    }
}

/// Ledger timestamp and sequence captured at a point in execution.
///
/// [`verify`](Self::verify) checks that neither value has gone backwards
/// since capture. It does **not** snapshot or compare contract storage.
pub struct StateSnapshot<'a> {
    env: &'a Env,
    timestamp: u64,
    ledger_seq: u64,
}

impl<'a> StateSnapshot<'a> {
    /// Record the current ledger timestamp and sequence.
    pub fn capture(env: &'a Env) -> Self {
        Self {
            env,
            timestamp: env.ledger().timestamp(),
            ledger_seq: env.ledger().sequence() as u64,
        }
    }

    /// Returns `false` if the ledger timestamp or sequence in `env` is lower
    /// than at capture.
    pub fn verify(&self, env: &Env) -> bool {
        let new_ts = env.ledger().timestamp();
        if new_ts < self.timestamp {
            return false;
        }
        let new_seq = env.ledger().sequence() as u64;
        if new_seq < self.ledger_seq {
            return false;
        }
        true
    }

    /// Panics if [`verify`](Self::verify) returns `false`.
    pub fn assert_valid(&self) {
        if !self.verify(self.env) {
            panic!("state validation failed - mid-execution state change detected");
        }
    }
}

/// Returns `true` if `caller` appears in `authorized_contracts`.
///
/// This is a plain membership check. It does not call `require_auth`.
pub fn validate_caller_is_authorized(
    _env: &Env,
    caller: &Address,
    authorized_contracts: &soroban_sdk::Vec<Address>,
) -> bool {
    for auth in authorized_contracts.iter() {
        if auth == caller.clone() {
            return true;
        }
    }
    false
}

/// Returns `true` if `min_amount <= amount <= max_per_tx` (both bounds
/// inclusive).
pub fn validate_amount_limits(
    amount: i128,
    min_amount: i128,
    max_per_tx: i128,
) -> bool {
    amount >= min_amount && amount <= max_per_tx
}

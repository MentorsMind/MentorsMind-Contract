//! RAII reentrancy guard for Soroban contracts.
//!
//! Soroban's single-threaded execution model makes reentrancy unlikely, but
//! cross-contract call chains can still reenter a contract within the same
//! transaction. This guard uses instance storage as a mutex flag to detect
//! and prevent that.
use soroban_sdk::{symbol_short, Env, Symbol};

const LOCK_PREFIX: Symbol = symbol_short!("RGUARD");

/// RAII guard that sets a named lock in instance storage on construction and
/// removes it on drop. Panics immediately if the lock is already held.
///
/// # Usage
/// ```ignore
/// let _guard = ReentrancyGuard::enter(&env, symbol_short!("my_lock"));
/// // protected code here — guard released automatically on scope exit
/// ```
pub struct ReentrancyGuard<'a> {
    env: &'a Env,
    lock_name: Symbol,
}

impl<'a> ReentrancyGuard<'a> {
    /// Acquire the named reentrancy lock.
    ///
    /// Sets `(RGUARD, lock_name) = true` in instance storage.
    ///
    /// # Panics
    /// Panics with `"reentrant call"` if the lock is already held, indicating
    /// that the contract has been reentered within the same transaction.
    pub fn enter(env: &'a Env, lock_name: Symbol) -> Self {
        let key = (LOCK_PREFIX, lock_name.clone());
        let locked = env.storage().instance().get(&key).unwrap_or(false);
        if locked {
            panic!("reentrant call");
        }

        env.storage().instance().set(&key, &true);
        Self { env, lock_name }
    }
}

impl Drop for ReentrancyGuard<'_> {
    /// Release the lock by removing the flag from instance storage.
    fn drop(&mut self) {
        let key = (LOCK_PREFIX, self.lock_name.clone());
        self.env.storage().instance().remove(&key);
    }
}

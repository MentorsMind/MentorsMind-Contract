//! Structured error logging for contract failures (#988).
//!
//! A bare `panic!("Not initialized")` or `panic!("Unauthorized")` tells a
//! caller *that* something failed but not *which* operation, *which*
//! record, or *why* — every occurrence of the same message across a
//! contract is indistinguishable from the outside. This module adds a
//! structured diagnostic event a contract can publish immediately before
//! panicking (or returning a typed `Error`), without changing the typed
//! error code itself — existing `Result<_, Error>` / panic-message call
//! sites and their error codes are unaffected, so this is purely additive.

use soroban_sdk::{contracttype, symbol_short, Address, Env, Symbol};

/// One structured failure record: which operation failed, on what it was
/// operating (if applicable), who called it, and a short reason code.
///
/// `subject_id` is deliberately a plain `i128` rather than a generic `Val`
/// so this type stays simple and `#[contracttype]`-safe; callers encode
/// whatever identifier is relevant (an escrow id, a proposal id, an
/// amount) as an integer. Use `0` when there's no natural subject id.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContractErrorContext {
    /// The public entry point that failed, e.g. `symbol_short!("release")`.
    pub operation: Symbol,
    /// Short machine-readable reason, e.g. `symbol_short!("not_init")`.
    pub reason: Symbol,
    /// The record/entity the operation was acting on (an escrow id, a
    /// proposal id, ...), or `0` if not applicable.
    pub subject_id: i128,
    /// The caller whose call triggered the failure, if known at the point
    /// of failure (auth may not have run yet).
    pub caller: Option<Address>,
    pub timestamp: u64,
    pub ledger_seq: u32,
}

/// Publish a [`ContractErrorContext`] diagnostic event. Call this
/// immediately before a `panic!`/`return Err(...)` so the failure is
/// observable off-chain with full context, even though the panic message
/// or error code alone wouldn't carry it.
///
/// This does not itself abort execution — call it, then panic/return as
/// normal immediately after.
pub fn log_contract_error(
    env: &Env,
    operation: Symbol,
    reason: Symbol,
    subject_id: i128,
    caller: Option<Address>,
) {
    let ctx = ContractErrorContext {
        operation: operation.clone(),
        reason: reason.clone(),
        subject_id,
        caller,
        timestamp: env.ledger().timestamp(),
        ledger_seq: env.ledger().sequence(),
    };
    env.events()
        .publish((symbol_short!("ctr_err"), operation, reason), ctx);
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn log_contract_error_does_not_panic() {
        let env = Env::default();
        let caller = Address::generate(&env);
        log_contract_error(
            &env,
            Symbol::new(&env, "release_funds"),
            Symbol::new(&env, "not_active"),
            42,
            Some(caller),
        );
    }
}

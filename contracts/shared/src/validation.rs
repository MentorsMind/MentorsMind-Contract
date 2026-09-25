//! Shared input validation middleware for all MentorsMind contracts.
//!
//! Provides a [`Validator`] builder pattern for consistent, reusable input
//! validation across contract entry points. Each validation rule returns
//! `Self` for chaining, and `validate()` produces a `Result<(), ValidationError>`.
//!
//! # Example
//! ```ignore
//! Validator::new(&env)
//!     .require_positive(amount, "amount")
//!     .require_future_timestamp(deadline, "deadline")
//!     .require_range(duration_mins, 1, 480, "duration_mins")
//!     .validate()?;
//! ```

use soroban_sdk::{contracttype, Env, Symbol, Vec};

/// Error returned when a validation rule fails.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ValidationError {
    /// The name of the field that failed validation.
    pub field: Symbol,
    /// The constraint that was violated (e.g. "positive", "future", "range").
    pub constraint: Symbol,
    /// The value that was provided (for numeric fields; 0 for non-numeric).
    pub value_provided: i128,
}

/// Builder for validating contract inputs.
///
/// Collects validation errors without short-circuiting, so all failures
/// can be reported at once (useful for batch operations).
pub struct Validator<'a> {
    env: &'a Env,
    errors: Vec<ValidationError>,
}

impl<'a> Validator<'a> {
    pub fn new(env: &'a Env) -> Self {
        Self {
            env,
            errors: Vec::new(env),
        }
    }

    /// Assert that `value` is strictly positive (> 0).
    pub fn require_positive(mut self, value: i128, field: &str) -> Self {
        if value <= 0 {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "positive"),
                value_provided: value,
            });
        }
        self
    }

    /// Assert that `value` is non-negative (>= 0).
    pub fn require_non_negative(mut self, value: i128, field: &str) -> Self {
        if value < 0 {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "non_negative"),
                value_provided: value,
            });
        }
        self
    }

    /// Assert that `timestamp` is in the future relative to the ledger.
    pub fn require_future_timestamp(mut self, timestamp: u64, field: &str) -> Self {
        let now = self.env.ledger().timestamp();
        if timestamp <= now {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "future"),
                value_provided: timestamp as i128,
            });
        }
        self
    }

    /// Assert that `value` is within `[min, max]` (inclusive).
    pub fn require_range(mut self, value: i128, min: i128, max: i128, field: &str) -> Self {
        if value < min || value > max {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "range"),
                value_provided: value,
            });
        }
        self
    }

    /// Assert that `value` is non-zero (for addresses, hashes, etc.).
    pub fn require_nonzero(mut self, value: i128, field: &str) -> Self {
        if value == 0 {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "nonzero"),
                value_provided: 0,
            });
        }
        self
    }

    /// Assert that `value` does not exceed `max`.
    ///
    /// Used to enforce upper economic bounds (e.g. a value must never exceed
    /// the total token supply) so a single oversized amount can't be used to
    /// trigger overflow further down the call chain or drain a pool in one
    /// shot.
    pub fn require_max(mut self, value: i128, max: i128, field: &str) -> Self {
        if value > max {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "max"),
                value_provided: value,
            });
        }
        self
    }

    /// Assert that `value` is at least `min`.
    ///
    /// Used for business-logic minimums (e.g. a stake or escrow amount below
    /// which the operation isn't economically meaningful, or would round to
    /// zero once fees/splits are applied).
    pub fn require_min(mut self, value: i128, min: i128, field: &str) -> Self {
        if value < min {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "min"),
                value_provided: value,
            });
        }
        self
    }

    /// Assert that `amount` is strictly positive and <= MAX_FINANCIAL_AMOUNT.
    pub fn require_valid_amount(mut self, amount: i128, field: &str) -> Self {
        if amount <= 0 {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "positive"),
                value_provided: amount,
            });
        }
        if amount > crate::MAX_FINANCIAL_AMOUNT {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "max"),
                value_provided: amount,
            });
        }
        self
    }

    /// Assert that `bps` is a valid basis-points value (`0..=10_000`).
    pub fn require_valid_bps(mut self, bps: u32, field: &str) -> Self {
        if bps > 10_000 {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "bps_range"),
                value_provided: bps as i128,
            });
        }
        self
    }

    /// Assert that `total` can be split `parts` ways without every share
    /// rounding down to zero (a common source of "precision" exploits where
    /// a caller inflates `parts` so each recipient's cut truncates to
    /// nothing while the remainder is swept elsewhere).
    ///
    /// Also rejects a non-positive `parts`, which would otherwise divide by
    /// zero downstream.
    pub fn require_sufficient_for_division(mut self, total: i128, parts: i128, field: &str) -> Self {
        if parts <= 0 || total < parts {
            self.errors.push_back(ValidationError {
                field: Symbol::new(self.env, field),
                constraint: Symbol::new(self.env, "divisible"),
                value_provided: parts,
            });
        }
        self
    }

    /// Consume the builder and return `Ok(())` if all rules passed, or
    /// `Err(ValidationError)` with the first failure.
    pub fn validate(self) -> Result<(), ValidationError> {
        match self.errors.iter().next() {
            Some(err) => Err(err),
            None => Ok(()),
        }
    }

    /// Consume the builder and return all collected errors (empty = all passed).
    pub fn validate_all(self) -> Vec<ValidationError> {
        self.errors
    }

    /// Consume the builder and panic with a constraint-specific message on
    /// the first failure. Intended for contracts (like `escrow`) that use
    /// panic-based error handling rather than `Result<_, Error>`.
    pub fn validate_or_panic(self) {
        let env = self.env;
        if let Some(err) = self.errors.iter().next() {
            if err.constraint == Symbol::new(env, "positive") {
                panic!("validation failed: value must be greater than zero");
            } else if err.constraint == Symbol::new(env, "non_negative") {
                panic!("validation failed: value must not be negative");
            } else if err.constraint == Symbol::new(env, "future") {
                panic!("validation failed: timestamp must be in the future");
            } else if err.constraint == Symbol::new(env, "range") {
                panic!("validation failed: value is outside the allowed range");
            } else if err.constraint == Symbol::new(env, "nonzero") {
                panic!("validation failed: value must be nonzero");
            } else if err.constraint == Symbol::new(env, "max") {
                panic!("validation failed: value exceeds the maximum allowed amount");
            } else if err.constraint == Symbol::new(env, "min") {
                panic!("validation failed: value is below the minimum required amount");
            } else if err.constraint == Symbol::new(env, "bps_range") {
                panic!("validation failed: basis-points value must be between 0 and 10000");
            } else if err.constraint == Symbol::new(env, "divisible") {
                panic!("validation failed: amount too small to split without rounding a share to zero");
            } else {
                panic!("validation failed");
            }
        }
    }
}

/// Convenience: combine `require_auth()` with `Validator::validate()`.
///
/// Calls `caller.require_auth()` first, then runs the validator. If auth
/// fails, Soroban panics before validation runs (matching existing contract
/// behavior).
pub fn require_auth_and_validate(
    caller: &soroban_sdk::Address,
    validator: Validator<'_>,
) -> Result<(), ValidationError> {
    caller.require_auth();
    validator.validate()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn require_positive_passes_for_positive_value() {
        let env = Env::default();
        let result = Validator::new(&env).require_positive(100, "amount").validate();
        assert!(result.is_ok());
    }

    #[test]
    fn require_positive_fails_for_zero() {
        let env = Env::default();
        let err = Validator::new(&env)
            .require_positive(0, "amount")
            .validate()
            .unwrap_err();
        assert_eq!(err.field, Symbol::new(&env, "amount"));
        assert_eq!(err.constraint, Symbol::new(&env, "positive"));
    }

    #[test]
    fn require_positive_fails_for_negative() {
        let env = Env::default();
        let err = Validator::new(&env)
            .require_positive(-5, "amount")
            .validate()
            .unwrap_err();
        assert_eq!(err.value_provided, -5);
    }

    #[test]
    fn require_range_passes_within_bounds() {
        let env = Env::default();
        let result = Validator::new(&env)
            .require_range(200, 1, 480, "duration")
            .validate();
        assert!(result.is_ok());
    }

    #[test]
    fn require_range_fails_below_min() {
        let env = Env::default();
        let err = Validator::new(&env)
            .require_range(0, 1, 480, "duration")
            .validate()
            .unwrap_err();
        assert_eq!(err.constraint, Symbol::new(&env, "range"));
    }

    #[test]
    fn require_range_fails_above_max() {
        let env = Env::default();
        let err = Validator::new(&env)
            .require_range(500, 1, 480, "duration")
            .validate()
            .unwrap_err();
        assert_eq!(err.value_provided, 500);
    }

    #[test]
    fn chained_validations_all_must_pass() {
        let env = Env::default();
        let result = Validator::new(&env)
            .require_positive(100, "amount")
            .require_range(60, 1, 480, "duration")
            .validate();
        assert!(result.is_ok());
    }

    #[test]
    fn validate_all_returns_multiple_errors() {
        let env = Env::default();
        let errors = Validator::new(&env)
            .require_positive(0, "amount")
            .require_range(0, 1, 480, "duration")
            .validate_all();
        assert_eq!(errors.len(), 2);
    }

    #[test]
    fn require_non_negative_passes_for_zero() {
        let env = Env::default();
        let result = Validator::new(&env)
            .require_non_negative(0, "fee")
            .validate();
        assert!(result.is_ok());
    }

    #[test]
    fn require_non_negative_fails_for_negative() {
        let env = Env::default();
        let err = Validator::new(&env)
            .require_non_negative(-1, "fee")
            .validate()
            .unwrap_err();
        assert_eq!(err.constraint, Symbol::new(&env, "non_negative"));
    }

    #[test]
    fn require_max_passes_at_boundary() {
        let env = Env::default();
        let result = Validator::new(&env).require_max(1_000, 1_000, "amount").validate();
        assert!(result.is_ok());
    }

    #[test]
    fn require_max_fails_above_bound() {
        let env = Env::default();
        let err = Validator::new(&env)
            .require_max(1_001, 1_000, "amount")
            .validate()
            .unwrap_err();
        assert_eq!(err.constraint, Symbol::new(&env, "max"));
        assert_eq!(err.value_provided, 1_001);
    }

    #[test]
    fn require_min_passes_at_boundary() {
        let env = Env::default();
        let result = Validator::new(&env).require_min(10, 10, "amount").validate();
        assert!(result.is_ok());
    }

    #[test]
    fn require_min_fails_below_bound() {
        let env = Env::default();
        let err = Validator::new(&env)
            .require_min(5, 10, "amount")
            .validate()
            .unwrap_err();
        assert_eq!(err.constraint, Symbol::new(&env, "min"));
    }

    #[test]
    fn require_valid_bps_passes_within_range() {
        let env = Env::default();
        let result = Validator::new(&env).require_valid_bps(10_000, "fee_bps").validate();
        assert!(result.is_ok());
    }

    #[test]
    fn require_valid_bps_fails_above_10000() {
        let env = Env::default();
        let err = Validator::new(&env)
            .require_valid_bps(10_001, "fee_bps")
            .validate()
            .unwrap_err();
        assert_eq!(err.constraint, Symbol::new(&env, "bps_range"));
    }

    #[test]
    fn require_sufficient_for_division_passes_when_each_share_nonzero() {
        let env = Env::default();
        let result = Validator::new(&env)
            .require_sufficient_for_division(100, 4, "amount")
            .validate();
        assert!(result.is_ok());
    }

    #[test]
    fn require_sufficient_for_division_fails_when_shares_would_round_to_zero() {
        let env = Env::default();
        let err = Validator::new(&env)
            .require_sufficient_for_division(3, 4, "amount")
            .validate()
            .unwrap_err();
        assert_eq!(err.constraint, Symbol::new(&env, "divisible"));
    }

    #[test]
    fn require_sufficient_for_division_fails_for_zero_parts() {
        let env = Env::default();
        let err = Validator::new(&env)
            .require_sufficient_for_division(100, 0, "amount")
            .validate()
            .unwrap_err();
        assert_eq!(err.constraint, Symbol::new(&env, "divisible"));
    }

    #[test]
    #[should_panic(expected = "value exceeds the maximum allowed amount")]
    fn validate_or_panic_panics_with_constraint_message() {
        let env = Env::default();
        Validator::new(&env).require_max(2_000, 1_000, "amount").validate_or_panic();
    }

    #[test]
    fn validate_or_panic_passes_silently_when_valid() {
        let env = Env::default();
        Validator::new(&env)
            .require_positive(100, "amount")
            .require_max(100, 1_000, "amount")
            .validate_or_panic();
    }
}

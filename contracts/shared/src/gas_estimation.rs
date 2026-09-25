//! Shared gas-estimation type for on-chain view functions.
//!
//! `env.budget()` (soroban-sdk's instruction/memory metering handle) is only
//! available under the `testutils` feature, so it can't be used inside a
//! deployed contract's estimate function to predict the cost of an
//! operation that hasn't run yet. Estimate functions therefore use a
//! heuristic derived from current storage state and the number of
//! cross-contract calls the real operation performs; tests compare the
//! heuristic against `env.budget()` readings taken around the real
//! operation.
use soroban_sdk::contracttype;

/// A heuristic cost estimate for a protocol operation, returned by
/// `estimate_*` view functions (#761). Never mutates state.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GasEstimate {
    /// Estimated CPU instructions the operation will consume.
    pub base_instructions: u64,
    /// Estimated number of storage reads.
    pub storage_reads: u32,
    /// Estimated number of storage writes.
    pub storage_writes: u32,
    /// Estimated number of cross-contract invocations (token transfers,
    /// oracle/reputation/snapshot lookups, etc.).
    pub cross_contract_calls: u32,
}

impl GasEstimate {
    /// Default base instruction cost for a typical contract operation.
    /// Covers dispatch, argument decoding, and basic validation.
    pub const DEFAULT_BASE_INSTRUCTIONS: u64 = 40_000;

    /// Default CPU cost per individual storage read or write.
    pub const DEFAULT_PER_STORAGE_OP_INSTRUCTIONS: u64 = 2_000;

    /// Default CPU cost per cross-contract invocation.
    pub const DEFAULT_PER_CROSS_CALL_INSTRUCTIONS: u64 = 300_000;

    /// Maximum allowed relative error between estimated and actual CPU
    /// instructions for an estimate to be considered accurate, expressed
    /// in basis points (1/100 of a percent). Default: 2_000 bps = 20%.
    pub const DEFAULT_TOLERANCE_BPS: u32 = 2_000;

    /// Compute the total estimated CPU instructions from a base plus
    /// per-operation costs.
    pub fn compute_instructions(
        base: u64,
        storage_ops: u32,
        cross_calls: u32,
    ) -> u64 {
        base + (storage_ops as u64) * Self::DEFAULT_PER_STORAGE_OP_INSTRUCTIONS
            + (cross_calls as u64) * Self::DEFAULT_PER_CROSS_CALL_INSTRUCTIONS
    }

    /// Check whether `actual` is within `tolerance_bps` of `estimated`.
    /// Returns `true` when the relative error does not exceed the tolerance.
    pub fn within_tolerance(estimated: u64, actual: u64, tolerance_bps: u32) -> bool {
        if estimated == 0 && actual == 0 {
            return true;
        }
        if estimated == 0 || actual == 0 {
            return false;
        }
        let larger = u64::max(estimated, actual);
        let smaller = u64::min(estimated, actual);
        let diff = larger - smaller;
        let allowed = (larger * (tolerance_bps as u64)) / 10_000;
        diff <= allowed
    }

    /// Convenience helper using `DEFAULT_TOLERANCE_BPS`.
    pub fn is_accurate(estimated: u64, actual: u64) -> bool {
        Self::within_tolerance(estimated, actual, Self::DEFAULT_TOLERANCE_BPS)
    }
}

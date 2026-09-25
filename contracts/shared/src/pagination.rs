//! Bounded-iteration utilities shared by every contract that enumerates a
//! persistent collection (#831).
//!
//! Soroban contracts run inside a per-invocation CPU/memory budget enforced
//! by the host; a function that loops over an *unbounded* on-chain
//! collection (e.g. "every escrow ever created") can be made to exceed that
//! budget simply by growing the collection, turning a read-only view
//! function into a denial-of-service vector. The fix is always the same
//! shape: never iterate more than a fixed maximum number of items in a
//! single call, and let the caller page through the rest.
//!
//! Note on "gas monitoring": the Soroban guest environment does not expose a
//! way for contract code to introspect its own remaining CPU/memory
//! budget at runtime (that instrumentation, `Env::cost_estimate()`, only
//! exists behind the `testutils` feature used by the host/test harness, not
//! in a deployed contract). So there is no way for a contract to genuinely
//! "monitor gas usage and suspend at 80% of the block limit" from the
//! inside. [`OperationBudget`] is this crate's practical substitute: an
//! explicit *operation count* ceiling, checked before each unit of work, in
//! place of runtime gas introspection that the platform doesn't allow.

use soroban_sdk::Env;

pub const MAX_PAGE_SIZE: u32 = 50;

pub struct Pagination {
    pub offset: u32,
    pub limit: u32,
}

impl Pagination {
    pub fn new(offset: u32, limit: u32) -> Self {
        Self { offset, limit }
    }

/// Hard ceiling on the number of items a single view may return.
///
/// Liquidation bots and governance tooling enumerate positions off-chain, so a
/// page must stay small enough to fit inside a single transaction's budget.
pub const MAX_PAGE_SIZE: u32 = 50;

/// Inclusive/exclusive page window helper shared by every paginated view.
pub struct Pagination;

impl Pagination {
    /// Clamps `limit` and resolves `(offset, offset + limit)` against `total`.
    ///
    /// Returns a half-open range `[start, end)` that is always within
    /// `0..=total`. An out-of-range offset collapses to an empty page rather
    /// than panicking, so callers can page until `start == total`.
    pub fn bounds(total: u32, offset: u32, limit: u32) -> (u32, u32) {
        let limit = Self::clamp_limit(limit);

        if offset >= total {
            return (total, total);
        }

        let remaining = total - offset;
        let end = if limit >= remaining {
            total
        } else {
            offset + limit
        };

        (offset, end)
    }

    /// Enforces `MAX_PAGE_SIZE`, treating a zero limit as a single item so a
    /// caller can never accidentally page the whole collection.
    pub fn clamp_limit(limit: u32) -> u32 {
        if limit == 0 {
            1
        } else if limit > MAX_PAGE_SIZE {
            MAX_PAGE_SIZE
        } else {
            limit
        }
    }
}

/// A trait for enumerable on-chain collections that must expose a bounded
/// view. Implementing this (rather than a bespoke ad hoc loop) documents,
/// per collection, exactly what the page cap and total-count source are.
pub trait BoundedIteration {
    fn total_count(&self, env: &Env) -> u32;

    fn max_page_size(&self) -> u32 {
        MAX_PAGE_SIZE
    }
}

pub struct OperationBudget {
    consumed: u32,
    max: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetExceeded;

impl OperationBudget {
    pub fn new(max: u32) -> Self {
        Self { consumed: 0, max }
    }

    pub fn consume(&mut self) -> Result<(), BudgetExceeded> {
        if self.consumed >= self.max {
            return Err(BudgetExceeded);
        }
        self.consumed = self.consumed.saturating_add(1);
        Ok(())
    }

    pub fn consumed(&self) -> u32 {
        self.consumed
    }

    pub fn remaining(&self) -> u32 {
        self.max.saturating_sub(self.consumed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::Env;

    #[test]
    fn bounds_clamps_limit_to_max_page_size() {
        let _env = Env::default();
        assert_eq!(Pagination::bounds(1_000, 0, 10_000), (0, MAX_PAGE_SIZE));
    }

    #[test]
    fn bounds_clamps_end_to_total() {
        assert_eq!(Pagination::bounds(100, 90, 50), (90, 100));
    }

    #[test]
    fn bounds_offset_past_total_yields_empty_range() {
        assert_eq!(Pagination::bounds(100, 500, 10), (100, 100));
    }

    #[test]
    fn operation_budget_allows_up_to_max() {
        let mut budget = OperationBudget::new(3);
        assert!(budget.consume().is_ok());
        assert!(budget.consume().is_ok());
        assert!(budget.consume().is_ok());
        assert_eq!(budget.consume(), Err(BudgetExceeded));
        assert_eq!(budget.consumed(), 3);
    }
}

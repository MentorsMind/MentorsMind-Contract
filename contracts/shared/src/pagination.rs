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
//! Note on "gas monitoring": the Soroban guest environment does not expose
//! a way for contract code to introspect its own remaining CPU/memory
//! budget at runtime (that instrumentation, `Env::cost_estimate()`, only
//! exists behind the `testutils` feature used by the host/test harness, not
//! in a deployed contract). So there is no way for a contract to genuinely
//! "monitor gas usage and suspend at 80% of the block limit" from the
//! inside. [`OperationBudget`] is this crate's practical substitute: an
//! explicit *operation count* ceiling, checked before each unit of work, in
//! place of runtime gas introspection that the platform doesn't allow.

use soroban_sdk::Env;

/// Hard ceiling on how many items any single paginated call may return or
/// iterate, regardless of what the caller asks for. Mirrors the "maximum
/// 100 items per transaction" requirement from #831.
pub const MAX_PAGE_SIZE: u32 = 100;

/// Requested page bounds for an enumeration call.
pub struct Pagination {
    pub offset: u32,
    pub limit: u32,
}

impl Pagination {
    pub fn new(offset: u32, limit: u32) -> Self {
        Self { offset, limit }
    }

    /// Resolve this request against a collection of `total` items into a
    /// concrete `[start, end)` range, clamping `limit` to [`MAX_PAGE_SIZE`]
    /// and `end` to `total` so callers can never force an out-of-bounds or
    /// oversized scan.
    pub fn bounds(&self, total: u32) -> (u32, u32) {
        let start = self.offset.min(total);
        let capped_limit = self.limit.min(MAX_PAGE_SIZE);
        let end = start.saturating_add(capped_limit).min(total);
        (start, end)
    }
}

/// A trait for enumerable on-chain collections that must expose a bounded
/// view. Implementing this (rather than a bespoke ad hoc loop) documents,
/// per collection, exactly what the page cap and total-count source are.
pub trait BoundedIteration {
    /// Total number of items currently stored.
    fn total_count(&self, env: &Env) -> u32;

    /// Largest page size this collection will ever return in one call.
    /// Defaults to [`MAX_PAGE_SIZE`]; override with a smaller value for a
    /// collection whose per-item work is heavier than a simple read.
    fn max_page_size(&self) -> u32 {
        MAX_PAGE_SIZE
    }
}

/// Explicit operation-count budget, checked before each unit of work in a
/// loop. See the module docs for why this exists instead of true runtime
/// gas introspection.
///
/// ```ignore
/// let mut budget = OperationBudget::new(MAX_PAGE_SIZE);
/// for id in start..end {
///     budget.consume()?; // Err before the (max+1)th iteration runs
///     // ... do bounded work ...
/// }
/// ```
pub struct OperationBudget {
    consumed: u32,
    max: u32,
}

/// Returned by [`OperationBudget::consume`] once the budget is exhausted.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BudgetExceeded;

impl OperationBudget {
    pub fn new(max: u32) -> Self {
        Self { consumed: 0, max }
    }

    /// Record one unit of work. Returns `Err(BudgetExceeded)` (without
    /// incrementing further) once `max` units have already been consumed,
    /// so a caller can abort the loop before doing the over-budget work.
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
        let p = Pagination::new(0, 10_000);
        let (start, end) = p.bounds(1_000);
        assert_eq!(start, 0);
        assert_eq!(end, MAX_PAGE_SIZE);
    }

    #[test]
    fn bounds_clamps_end_to_total() {
        let p = Pagination::new(90, 50);
        let (start, end) = p.bounds(100);
        assert_eq!(start, 90);
        assert_eq!(end, 100);
    }

    #[test]
    fn bounds_offset_past_total_yields_empty_range() {
        let p = Pagination::new(500, 10);
        let (start, end) = p.bounds(100);
        assert_eq!(start, 100);
        assert_eq!(end, 100);
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

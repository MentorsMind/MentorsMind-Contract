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

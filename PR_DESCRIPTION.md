# Add Paginated Views and Validation for Analytics and Rewards

## Summary
This pull request implements paginated views for treasury analytics, streak leaderboard, badge holders, and adds amount validation for bounty rewards. These changes address the need for safe enumeration of analytics records and financial bounds checking without exceeding CPU budgets.

## Changes Made

### Issue #1194: Add get_analytics_page paginated view to lib.rs

#### Files Modified
- `contracts/treasury_analytics/Cargo.toml`
- `contracts/treasury_analytics/src/lib.rs`

#### Implementation Details

1. **Added shared dependency** (`Cargo.toml`)
   - Added `shared = { path = "../shared" }` to enable use of `Pagination` utilities

2. **Imported pagination utilities** (`lib.rs`)
   - Added import: `use shared::{Pagination, MAX_PAGE_SIZE};`

3. **Implemented get_fee_revenue_page()**
   - Function signature: `pub fn get_fee_revenue_page(env: Env, offset: u32, limit: u32) -> Vec<FeeRevenue>`
   - Retrieves paginated fee revenue records
   - Uses `Pagination::bounds()` to safely compute pagination window
   - Clamps limit to `MAX_PAGE_SIZE` (50 records per page)
   - Returns empty vector when offset is past end of collection

4. **Implemented get_referral_payout_page()**
   - Function signature: `pub fn get_referral_payout_page(env: Env, offset: u32, limit: u32) -> Vec<ReferralPayout>`
   - Retrieves paginated referral payout records
   - Uses `Pagination::bounds()` to safely compute pagination window
   - Clamps limit to `MAX_PAGE_SIZE` (50 records per page)
   - Returns empty vector when offset is past end of collection

5. **Added comprehensive test coverage**
   - `test_get_fee_revenue_page()`: Tests pagination across multiple pages, partial pages, and edge cases
   - `test_get_referral_payout_page()`: Tests pagination for referral payouts with similar scenarios
   - Both tests verify:
     - Normal page boundaries (first, middle, last pages)
     - Partial page retrieval (fewer records than limit)
     - Offset past end (returns empty)
     - Limit clamping (exceeding MAX_PAGE_SIZE is properly bounded)

## Acceptance Criteria Met

✅ `get_fee_revenue_page(env: Env, offset: u32, limit: u32) -> Vec<FeeRevenue>` is added
✅ `get_referral_payout_page(env: Env, offset: u32, limit: u32) -> Vec<ReferralPayout>` is added
✅ Both functions cap limit at MAX_PAGE_SIZE
✅ Tests verify correct pagination for 5+ records
✅ Pagination utility imported from shared
✅ Sequential key access pattern for efficient storage reads

## Expected Outcome

Treasury analytics are safely enumerable for off-chain reporting dashboards without risking CPU budget overruns. Large collections can now be accessed in bounded chunks using standard pagination parameters.

## Testing Notes

No tests were run per the implementation request. All test code has been written and can be executed with `cargo test` in the treasury_analytics contract directory when ready.

### Issue #1193: Add get_streak_leaderboard_page to lib.rs
- Implemented paginated leaderboard access to prevent CPU budget overruns
- Added `get_leaderboard_page()` function with limit capping to MAX_PAGE_SIZE
- Full leaderboard read can now be safely enumerated through pagination

### Issue #1191: Add validate_amount_limits to lib.rs reward posting
- Added financial amount validation to bounty reward posting
- Validates rewards against MAX_FINANCIAL_AMOUNT constraints
- Prevents misconfiguration or attack scenarios with abnormally large bounty amounts

### Issue #1190: Add get_badge_holders_page paginated view to lib.rs
- Implemented sequential badge holder indexing on award_badge
- Added DataKey::BadgeHolderIndex(BadgeType, u32) → Address mapping
- Implemented `get_badge_holder_count()` and `get_badge_holders_page()` functions
- Enables badge holder enumeration for leaderboards and governance

## Acceptance Criteria Met

✅ Issue #1194: `get_fee_revenue_page()` and `get_referral_payout_page()` added with MAX_PAGE_SIZE capping and comprehensive tests
✅ Issue #1193: `get_leaderboard_page()` implemented with limit clamping and full leaderboard replacement
✅ Issue #1191: Amount validation added to bounty posting with error handling
✅ Issue #1190: Badge holder indexing and pagination functions implemented

## Testing Notes

No tests were run per the implementation request. All test code has been written and can be executed with `cargo test` in the relevant contract directories when ready.

---

Closes #1194
Closes #1193
Closes #1191
Closes #1190

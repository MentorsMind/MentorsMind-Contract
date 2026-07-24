# Treasury Analytics Contract

## Overview

The Treasury Analytics Contract provides comprehensive visibility into protocol revenue flows and risk exposure for the MentorsMind platform. It tracks fee revenue, referral payouts, insurance reserves, and treasury growth metrics with full historical reporting capabilities.

## Features

### 1. Fee Revenue Tracking
- Records all fee revenue from various protocol sources (escrow, sessions, lending, etc.)
- Stores token address, amount, source identifier, and timestamp
- Supports pagination for efficient data retrieval
- Historical querying by time period

### 2. Referral Payout Analysis
- Tracks all referral reward distributions
- Records referrer address, token, amount, and timestamp
- Paginated access to payout history
- Time-based filtering for analysis

### 3. Insurance Reserve Analysis
- Snapshots of insurance reserve state over time
- Tracks total balance, allocated amount, and available reserves
- Historical view of reserve health
- Support for multiple token types

### 4. Treasury Growth Metrics
- Period-based revenue and payout tracking
- Calculates net treasury growth
- Configurable time windows for analysis
- Growth trend visualization data

### 5. Historical Reporting
- Comprehensive analytics reports for any time period
- Aggregates data across all tracking categories
- Net growth calculations
- Event-based audit trail

## Data Structures

### FeeRevenue
```rust
pub struct FeeRevenue {
    pub token: Address,        // Token address
    pub amount: i128,          // Revenue amount
    pub source: Symbol,        // Source identifier (e.g., "escrow", "session")
    pub timestamp: u64,        // Block timestamp
}
```

### ReferralPayout
```rust
pub struct ReferralPayout {
    pub referrer: Address,     // Referrer receiving payout
    pub token: Address,        // Token paid
    pub amount: i128,          // Payout amount
    pub timestamp: u64,        // Block timestamp
}
```

### InsuranceReserve
```rust
pub struct InsuranceReserve {
    pub token: Address,        // Reserve token
    pub total_balance: i128,   // Total reserve balance
    pub allocated: i128,       // Amount allocated/locked
    pub available: i128,       // Available for claims
    pub timestamp: u64,        // Snapshot timestamp
}
```

### TreasuryMetrics
```rust
pub struct TreasuryMetrics {
    pub total_revenue: i128,   // Total revenue in period
    pub total_payouts: i128,   // Total payouts in period
    pub net_growth: i128,      // Net growth (revenue - payouts)
    pub period_start: u64,     // Period start timestamp
    pub period_end: u64,       // Period end timestamp
}
```

### AnalyticsReport
```rust
pub struct AnalyticsReport {
    pub total_fee_revenue: i128,           // Aggregated fee revenue
    pub total_referral_payouts: i128,      // Aggregated referral payouts
    pub total_insurance_reserves: i128,    // Latest insurance reserve total
    pub net_treasury_growth: i128,         // Net growth calculation
    pub report_timestamp: u64,             // Report generation time
    pub period_start: u64,                 // Analysis period start
    pub period_end: u64,                   // Analysis period end
}
```

## Contract Functions

### Initialization

#### `initialize(admin: Address)`
Initializes the contract with an admin address. Can only be called once.

**Parameters:**
- `admin`: Address with permission to record analytics data

**Returns:** `Result<(), Error>`

**Errors:**
- `AlreadyInitialized`: Contract already initialized

---

### Fee Revenue Tracking

#### `record_fee_revenue(token: Address, amount: i128, source: Symbol)`
Records fee revenue from a protocol source. Admin only.

**Parameters:**
- `token`: Token address of the revenue
- `amount`: Revenue amount
- `source`: Source identifier (e.g., "escrow", "session", "lending")

**Returns:** `Result<(), Error>`

**Errors:**
- `NotInitialized`: Contract not initialized
- `Unauthorized`: Caller is not admin

**Events:**
- `("fee_rev", source, token)` → amount

#### `get_fee_revenue(offset: u32, limit: u32)`
Retrieves fee revenue records with pagination.

**Parameters:**
- `offset`: Starting index
- `limit`: Maximum records to return

**Returns:** `Vec<FeeRevenue>`

#### `get_fee_revenue_count()`
Returns total number of fee revenue records.

**Returns:** `u32`

#### `get_historical_fee_revenue(period_start: u64, period_end: u64)`
Retrieves fee revenue records within a specific time period.

**Parameters:**
- `period_start`: Start timestamp (inclusive)
- `period_end`: End timestamp (inclusive)

**Returns:** `Vec<FeeRevenue>`

---

### Referral Payout Analysis

#### `record_referral_payout(referrer: Address, token: Address, amount: i128)`
Records a referral payout. Admin only.

**Parameters:**
- `referrer`: Address receiving the payout
- `token`: Token address
- `amount`: Payout amount

**Returns:** `Result<(), Error>`

**Errors:**
- `NotInitialized`: Contract not initialized
- `Unauthorized`: Caller is not admin

**Events:**
- `("ref_pay", referrer, token)` → amount

#### `get_referral_payouts(offset: u32, limit: u32)`
Retrieves referral payout records with pagination.

**Parameters:**
- `offset`: Starting index
- `limit`: Maximum records to return

**Returns:** `Vec<ReferralPayout>`

#### `get_referral_payout_count()`
Returns total number of referral payout records.

**Returns:** `u32`

#### `get_historical_referral_payouts(period_start: u64, period_end: u64)`
Retrieves referral payout records within a specific time period.

**Parameters:**
- `period_start`: Start timestamp (inclusive)
- `period_end`: End timestamp (inclusive)

**Returns:** `Vec<ReferralPayout>`

---

### Insurance Reserve Analysis

#### `record_insurance_reserve(token: Address, total_balance: i128, allocated: i128, available: i128)`
Records an insurance reserve snapshot. Admin only.

**Parameters:**
- `token`: Reserve token address
- `total_balance`: Total reserve balance
- `allocated`: Amount allocated/locked
- `available`: Available for claims

**Returns:** `Result<(), Error>`

**Errors:**
- `NotInitialized`: Contract not initialized
- `Unauthorized`: Caller is not admin

**Events:**
- `("ins_rsv", token)` → total_balance

#### `get_insurance_reserves(offset: u32, limit: u32)`
Retrieves insurance reserve records with pagination.

**Parameters:**
- `offset`: Starting index
- `limit`: Maximum records to return

**Returns:** `Vec<InsuranceReserve>`

#### `get_insurance_reserve_count()`
Returns total number of insurance reserve records.

**Returns:** `u32`

---

### Treasury Growth Metrics

#### `record_treasury_metrics(total_revenue: i128, total_payouts: i128, period_start: u64, period_end: u64)`
Records treasury metrics for a specific period. Admin only.

**Parameters:**
- `total_revenue`: Total revenue in the period
- `total_payouts`: Total payouts in the period
- `period_start`: Period start timestamp
- `period_end`: Period end timestamp

**Returns:** `Result<(), Error>`

**Errors:**
- `NotInitialized`: Contract not initialized
- `Unauthorized`: Caller is not admin
- `InvalidTimestamp`: period_start >= period_end

**Events:**
- `("trs_mtrc", "growth")` → net_growth

#### `get_treasury_metrics(offset: u32, limit: u32)`
Retrieves treasury metrics records with pagination.

**Parameters:**
- `offset`: Starting index
- `limit`: Maximum records to return

**Returns:** `Vec<TreasuryMetrics>`

#### `get_treasury_metrics_count()`
Returns total number of treasury metrics records.

**Returns:** `u32`

---

### Historical Reporting

#### `generate_report(period_start: u64, period_end: u64)`
Generates a comprehensive analytics report for a time period.

**Parameters:**
- `period_start`: Analysis period start timestamp
- `period_end`: Analysis period end timestamp

**Returns:** `Result<AnalyticsReport, Error>`

**Errors:**
- `InvalidTimestamp`: period_start >= period_end

**Events:**
- `("report", "gen")` → net_treasury_growth

**Description:**
Aggregates all tracked metrics within the specified time period and generates a comprehensive report including:
- Total fee revenue
- Total referral payouts
- Latest insurance reserve total
- Net treasury growth (revenue - payouts)

---

## Error Codes

| Error | Code | Description |
|-------|------|-------------|
| `AlreadyInitialized` | 1 | Contract already initialized |
| `NotInitialized` | 2 | Contract not initialized |
| `Unauthorized` | 3 | Caller lacks required permission |
| `InvalidTimestamp` | 4 | Invalid timestamp parameters |
| `RecordNotFound` | 5 | Requested record does not exist |

## Events

### Fee Revenue
- **Topic:** `("fee_rev", source: Symbol, token: Address)`
- **Data:** `amount: i128`

### Referral Payout
- **Topic:** `("ref_pay", referrer: Address, token: Address)`
- **Data:** `amount: i128`

### Insurance Reserve
- **Topic:** `("ins_rsv", token: Address)`
- **Data:** `total_balance: i128`

### Treasury Metrics
- **Topic:** `("trs_mtrc", "growth")`
- **Data:** `net_growth: i128`

### Report Generation
- **Topic:** `("report", "gen")`
- **Data:** `net_treasury_growth: i128`

## Usage Examples

### Initialize Contract
```rust
let admin = Address::generate(&env);
contract.initialize(&admin);
```

### Record Fee Revenue
```rust
let token = token_address;
let amount = 1000;
let source = symbol_short!("escrow");

contract.record_fee_revenue(&token, &amount, &source);
```

### Record Referral Payout
```rust
let referrer = referrer_address;
let token = token_address;
let amount = 50;

contract.record_referral_payout(&referrer, &token, &amount);
```

### Record Insurance Reserve Snapshot
```rust
let token = token_address;
let total = 100000;
let allocated = 30000;
let available = 70000;

contract.record_insurance_reserve(&token, &total, &allocated, &available);
```

### Record Treasury Metrics
```rust
let revenue = 5000;
let payouts = 2000;
let start = 1000;
let end = 2000;

contract.record_treasury_metrics(&revenue, &payouts, &start, &end);
```

### Generate Analytics Report
```rust
let start_time = 1000;
let end_time = 2000;

let report = contract.generate_report(&start_time, &end_time);

// Access report data
let net_growth = report.net_treasury_growth;
let total_revenue = report.total_fee_revenue;
let total_payouts = report.total_referral_payouts;
```

### Query Historical Data
```rust
// Get fee revenue in a time period
let fee_history = contract.get_historical_fee_revenue(&start_time, &end_time);

// Get referral payouts in a time period
let payout_history = contract.get_historical_referral_payouts(&start_time, &end_time);

// Paginated access
let page_1 = contract.get_fee_revenue(&0, &10);
let page_2 = contract.get_fee_revenue(&10, &10);
```

## Integration Guide

### 1. Deploy Contract
Deploy the treasury analytics contract and initialize with admin address.

### 2. Grant Admin Access
Ensure the admin address is set to an authorized treasury management account or governance contract.

### 3. Integrate with Revenue Sources
Update fee-generating contracts (escrow, sessions, lending) to call `record_fee_revenue` when fees are collected.

### 4. Integrate with Referral System
Update referral contract to call `record_referral_payout` when rewards are distributed.

### 5. Schedule Reserve Snapshots
Implement periodic calls to `record_insurance_reserve` to track reserve health over time.

### 6. Calculate Periodic Metrics
Implement scheduled jobs to aggregate data and call `record_treasury_metrics` for each reporting period.

### 7. Generate Reports
Call `generate_report` on-demand or periodically for dashboard displays and governance reporting.

## Testing

The contract includes comprehensive unit tests covering:
- ✅ Contract initialization
- ✅ Fee revenue recording and retrieval
- ✅ Referral payout tracking
- ✅ Insurance reserve snapshots
- ✅ Treasury metrics recording
- ✅ Report generation with time periods
- ✅ Historical data querying
- ✅ Pagination functionality
- ✅ Invalid timestamp handling
- ✅ Comprehensive analytics workflow

Run tests:
```bash
cargo test --package mentorminds-treasury-analytics
```

## Security Considerations

1. **Admin-Only Recording**: All data recording functions require admin authentication to prevent unauthorized data manipulation.

2. **Timestamp Validation**: Period-based functions validate that start timestamps are before end timestamps.

3. **Data Immutability**: Once recorded, analytics data cannot be modified, ensuring audit trail integrity.

4. **Pagination**: Large datasets use pagination to prevent excessive storage reads and gas consumption.

5. **Event Emission**: All significant actions emit events for off-chain monitoring and alerting.

## Gas Optimization

- Counter-based indexing for efficient storage access
- Pagination to limit per-call gas costs
- Minimal storage operations in report generation
- Efficient iteration over time-filtered data

## Future Enhancements

- Multi-token aggregation in reports
- On-chain trend analysis (moving averages, growth rates)
- Automated alerting for anomalous patterns
- Role-based access control for different analytics consumers
- Data archival for very old records
- Cross-contract analytics integration

## License

This contract is part of the MentorsMind protocol.

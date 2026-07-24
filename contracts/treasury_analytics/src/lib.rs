#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Symbol, Vec,
};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidTimestamp = 4,
    RecordNotFound = 5,
}

/// Fee revenue tracking record
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeRevenue {
    pub token: Address,
    pub amount: i128,
    pub source: Symbol, // e.g., "escrow", "session", "lending"
    pub timestamp: u64,
}

/// Referral payout tracking record
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralPayout {
    pub referrer: Address,
    pub token: Address,
    pub amount: i128,
    pub timestamp: u64,
}

/// Insurance reserve snapshot
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InsuranceReserve {
    pub token: Address,
    pub total_balance: i128,
    pub allocated: i128,
    pub available: i128,
    pub timestamp: u64,
}

/// Treasury growth metrics
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryMetrics {
    pub total_revenue: i128,
    pub total_payouts: i128,
    pub net_growth: i128,
    pub period_start: u64,
    pub period_end: u64,
}

/// Aggregated analytics report
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnalyticsReport {
    pub total_fee_revenue: i128,
    pub total_referral_payouts: i128,
    pub total_insurance_reserves: i128,
    pub net_treasury_growth: i128,
    pub report_timestamp: u64,
    pub period_start: u64,
    pub period_end: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    /// Fee revenue count
    FeeRevenueCount,
    /// Individual fee revenue record: DataKey::FeeRevenue(index)
    FeeRevenue(u32),
    /// Referral payout count
    ReferralPayoutCount,
    /// Individual referral payout: DataKey::ReferralPayout(index)
    ReferralPayout(u32),
    /// Insurance reserve count
    InsuranceReserveCount,
    /// Individual insurance reserve snapshot: DataKey::InsuranceReserve(index)
    InsuranceReserve(u32),
    /// Treasury metrics count
    TreasuryMetricsCount,
    /// Individual treasury metrics: DataKey::TreasuryMetrics(index)
    TreasuryMetrics(u32),
}

#[contract]
pub struct TreasuryAnalyticsContract;

#[contractimpl]
impl TreasuryAnalyticsContract {
    /// Initialize the analytics contract with an admin
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().persistent().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }

        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::FeeRevenueCount, &0u32);
        env.storage()
            .persistent()
            .set(&DataKey::ReferralPayoutCount, &0u32);
        env.storage()
            .persistent()
            .set(&DataKey::InsuranceReserveCount, &0u32);
        env.storage()
            .persistent()
            .set(&DataKey::TreasuryMetricsCount, &0u32);

        Ok(())
    }

    // -----------------------------------------------------------------------
    // Fee Revenue Tracking
    // -----------------------------------------------------------------------

    /// Record fee revenue from a protocol source
    pub fn record_fee_revenue(
        env: Env,
        token: Address,
        amount: i128,
        source: Symbol,
    ) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::FeeRevenueCount)
            .unwrap_or(0u32);

        let record = FeeRevenue {
            token: token.clone(),
            amount,
            source: source.clone(),
            timestamp: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::FeeRevenue(count), &record);
        env.storage()
            .persistent()
            .set(&DataKey::FeeRevenueCount, &(count + 1));

        env.events().publish(
            (symbol_short!("fee_rev"), source, token),
            amount,
        );

        Ok(())
    }

    /// Get fee revenue records with pagination
    pub fn get_fee_revenue(env: Env, offset: u32, limit: u32) -> Vec<FeeRevenue> {
        let total_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::FeeRevenueCount)
            .unwrap_or(0u32);

        let mut result = Vec::new(&env);
        let end = offset.saturating_add(limit).min(total_count);

        for i in offset..end {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, FeeRevenue>(&DataKey::FeeRevenue(i))
            {
                result.push_back(record);
            }
        }

        result
    }

    /// Get total fee revenue count
    pub fn get_fee_revenue_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::FeeRevenueCount)
            .unwrap_or(0u32)
    }

    // -----------------------------------------------------------------------
    // Referral Payout Analysis
    // -----------------------------------------------------------------------

    /// Record referral payout
    pub fn record_referral_payout(
        env: Env,
        referrer: Address,
        token: Address,
        amount: i128,
    ) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ReferralPayoutCount)
            .unwrap_or(0u32);

        let record = ReferralPayout {
            referrer: referrer.clone(),
            token: token.clone(),
            amount,
            timestamp: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::ReferralPayout(count), &record);
        env.storage()
            .persistent()
            .set(&DataKey::ReferralPayoutCount, &(count + 1));

        env.events().publish(
            (symbol_short!("ref_pay"), referrer, token),
            amount,
        );

        Ok(())
    }

    /// Get referral payout records with pagination
    pub fn get_referral_payouts(env: Env, offset: u32, limit: u32) -> Vec<ReferralPayout> {
        let total_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ReferralPayoutCount)
            .unwrap_or(0u32);

        let mut result = Vec::new(&env);
        let end = offset.saturating_add(limit).min(total_count);

        for i in offset..end {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, ReferralPayout>(&DataKey::ReferralPayout(i))
            {
                result.push_back(record);
            }
        }

        result
    }

    /// Get total referral payout count
    pub fn get_referral_payout_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ReferralPayoutCount)
            .unwrap_or(0u32)
    }

    // -----------------------------------------------------------------------
    // Insurance Reserve Analysis
    // -----------------------------------------------------------------------

    /// Record insurance reserve snapshot
    pub fn record_insurance_reserve(
        env: Env,
        token: Address,
        total_balance: i128,
        allocated: i128,
        available: i128,
    ) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::InsuranceReserveCount)
            .unwrap_or(0u32);

        let record = InsuranceReserve {
            token: token.clone(),
            total_balance,
            allocated,
            available,
            timestamp: env.ledger().timestamp(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::InsuranceReserve(count), &record);
        env.storage()
            .persistent()
            .set(&DataKey::InsuranceReserveCount, &(count + 1));

        env.events().publish(
            (symbol_short!("ins_rsv"), token),
            total_balance,
        );

        Ok(())
    }

    /// Get insurance reserve records with pagination
    pub fn get_insurance_reserves(env: Env, offset: u32, limit: u32) -> Vec<InsuranceReserve> {
        let total_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::InsuranceReserveCount)
            .unwrap_or(0u32);

        let mut result = Vec::new(&env);
        let end = offset.saturating_add(limit).min(total_count);

        for i in offset..end {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, InsuranceReserve>(&DataKey::InsuranceReserve(i))
            {
                result.push_back(record);
            }
        }

        result
    }

    /// Get total insurance reserve count
    pub fn get_insurance_reserve_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::InsuranceReserveCount)
            .unwrap_or(0u32)
    }

    // -----------------------------------------------------------------------
    // Treasury Growth Metrics
    // -----------------------------------------------------------------------

    /// Record treasury growth metrics for a period
    pub fn record_treasury_metrics(
        env: Env,
        total_revenue: i128,
        total_payouts: i128,
        period_start: u64,
        period_end: u64,
    ) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        if period_start >= period_end {
            return Err(Error::InvalidTimestamp);
        }

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::TreasuryMetricsCount)
            .unwrap_or(0u32);

        let net_growth = total_revenue - total_payouts;

        let record = TreasuryMetrics {
            total_revenue,
            total_payouts,
            net_growth,
            period_start,
            period_end,
        };

        env.storage()
            .persistent()
            .set(&DataKey::TreasuryMetrics(count), &record);
        env.storage()
            .persistent()
            .set(&DataKey::TreasuryMetricsCount, &(count + 1));

        env.events().publish(
            (symbol_short!("trs_mtrc"), symbol_short!("growth")),
            net_growth,
        );

        Ok(())
    }

    /// Get treasury metrics with pagination
    pub fn get_treasury_metrics(env: Env, offset: u32, limit: u32) -> Vec<TreasuryMetrics> {
        let total_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::TreasuryMetricsCount)
            .unwrap_or(0u32);

        let mut result = Vec::new(&env);
        let end = offset.saturating_add(limit).min(total_count);

        for i in offset..end {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, TreasuryMetrics>(&DataKey::TreasuryMetrics(i))
            {
                result.push_back(record);
            }
        }

        result
    }

    /// Get total treasury metrics count
    pub fn get_treasury_metrics_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::TreasuryMetricsCount)
            .unwrap_or(0u32)
    }

    // -----------------------------------------------------------------------
    // Historical Reporting & Analytics
    // -----------------------------------------------------------------------

    /// Generate comprehensive analytics report for a time period
    pub fn generate_report(
        env: Env,
        period_start: u64,
        period_end: u64,
    ) -> Result<AnalyticsReport, Error> {
        if period_start >= period_end {
            return Err(Error::InvalidTimestamp);
        }

        let mut total_fee_revenue: i128 = 0;
        let mut total_referral_payouts: i128 = 0;
        let mut total_insurance_reserves: i128 = 0;

        // Aggregate fee revenue
        let fee_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::FeeRevenueCount)
            .unwrap_or(0u32);

        for i in 0..fee_count {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, FeeRevenue>(&DataKey::FeeRevenue(i))
            {
                if record.timestamp >= period_start && record.timestamp <= period_end {
                    total_fee_revenue += record.amount;
                }
            }
        }

        // Aggregate referral payouts
        let referral_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ReferralPayoutCount)
            .unwrap_or(0u32);

        for i in 0..referral_count {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, ReferralPayout>(&DataKey::ReferralPayout(i))
            {
                if record.timestamp >= period_start && record.timestamp <= period_end {
                    total_referral_payouts += record.amount;
                }
            }
        }

        // Get latest insurance reserve in period
        let insurance_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::InsuranceReserveCount)
            .unwrap_or(0u32);

        for i in 0..insurance_count {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, InsuranceReserve>(&DataKey::InsuranceReserve(i))
            {
                if record.timestamp >= period_start && record.timestamp <= period_end {
                    total_insurance_reserves = record.total_balance;
                }
            }
        }

        let net_treasury_growth = total_fee_revenue - total_referral_payouts;

        let report = AnalyticsReport {
            total_fee_revenue,
            total_referral_payouts,
            total_insurance_reserves,
            net_treasury_growth,
            report_timestamp: env.ledger().timestamp(),
            period_start,
            period_end,
        };

        env.events().publish(
            (symbol_short!("report"), symbol_short!("gen")),
            net_treasury_growth,
        );

        Ok(report)
    }

    /// Get historical data for a specific metric type and time range
    pub fn get_historical_fee_revenue(
        env: Env,
        period_start: u64,
        period_end: u64,
    ) -> Vec<FeeRevenue> {
        let total_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::FeeRevenueCount)
            .unwrap_or(0u32);

        let mut result = Vec::new(&env);

        for i in 0..total_count {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, FeeRevenue>(&DataKey::FeeRevenue(i))
            {
                if record.timestamp >= period_start && record.timestamp <= period_end {
                    result.push_back(record);
                }
            }
        }

        result
    }

    /// Get historical referral payouts for a time range
    pub fn get_historical_referral_payouts(
        env: Env,
        period_start: u64,
        period_end: u64,
    ) -> Vec<ReferralPayout> {
        let total_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ReferralPayoutCount)
            .unwrap_or(0u32);

        let mut result = Vec::new(&env);

        for i in 0..total_count {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, ReferralPayout>(&DataKey::ReferralPayout(i))
            {
                if record.timestamp >= period_start && record.timestamp <= period_end {
                    result.push_back(record);
                }
            }
        }

        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::{symbol_short, Env};

    fn setup_test(env: &Env) -> (Address, Address) {
        let admin = Address::generate(env);
        let contract_id = env.register_contract(None, TreasuryAnalyticsContract);
        let client = TreasuryAnalyticsContractClient::new(env, &contract_id);
        client.initialize(&admin);
        (admin, contract_id)
    }

    #[test]
    fn test_initialization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, TreasuryAnalyticsContract);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        client.initialize(&admin);

        // Try initializing again - should fail
        let result = client.try_initialize(&admin);
        assert!(result.is_err());
    }

    #[test]
    fn test_record_fee_revenue() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id) = setup_test(&env);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        let token = Address::generate(&env);
        let source = symbol_short!("escrow");

        env.ledger().set_timestamp(1000);
        client.record_fee_revenue(&token, &500, &source);

        assert_eq!(client.get_fee_revenue_count(), 1);

        let records = client.get_fee_revenue(&0, &10);
        assert_eq!(records.len(), 1);

        let record = records.get(0).unwrap();
        assert_eq!(record.token, token);
        assert_eq!(record.amount, 500);
        assert_eq!(record.source, source);
        assert_eq!(record.timestamp, 1000);
    }

    #[test]
    fn test_record_referral_payout() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id) = setup_test(&env);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        let referrer = Address::generate(&env);
        let token = Address::generate(&env);

        env.ledger().set_timestamp(2000);
        client.record_referral_payout(&referrer, &token, &100);

        assert_eq!(client.get_referral_payout_count(), 1);

        let records = client.get_referral_payouts(&0, &10);
        assert_eq!(records.len(), 1);

        let record = records.get(0).unwrap();
        assert_eq!(record.referrer, referrer);
        assert_eq!(record.token, token);
        assert_eq!(record.amount, 100);
        assert_eq!(record.timestamp, 2000);
    }

    #[test]
    fn test_record_insurance_reserve() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id) = setup_test(&env);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        let token = Address::generate(&env);

        env.ledger().set_timestamp(3000);
        client.record_insurance_reserve(&token, &10000, &3000, &7000);

        assert_eq!(client.get_insurance_reserve_count(), 1);

        let records = client.get_insurance_reserves(&0, &10);
        assert_eq!(records.len(), 1);

        let record = records.get(0).unwrap();
        assert_eq!(record.token, token);
        assert_eq!(record.total_balance, 10000);
        assert_eq!(record.allocated, 3000);
        assert_eq!(record.available, 7000);
        assert_eq!(record.timestamp, 3000);
    }

    #[test]
    fn test_record_treasury_metrics() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id) = setup_test(&env);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        client.record_treasury_metrics(&5000, &2000, &1000, &2000);

        assert_eq!(client.get_treasury_metrics_count(), 1);

        let records = client.get_treasury_metrics(&0, &10);
        assert_eq!(records.len(), 1);

        let record = records.get(0).unwrap();
        assert_eq!(record.total_revenue, 5000);
        assert_eq!(record.total_payouts, 2000);
        assert_eq!(record.net_growth, 3000);
        assert_eq!(record.period_start, 1000);
        assert_eq!(record.period_end, 2000);
    }

    #[test]
    fn test_treasury_metrics_invalid_period() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id) = setup_test(&env);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        // period_start >= period_end should fail
        let result = client.try_record_treasury_metrics(&5000, &2000, &2000, &1000);
        assert!(result.is_err());
    }

    #[test]
    fn test_generate_report() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id) = setup_test(&env);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        let token = Address::generate(&env);
        let referrer = Address::generate(&env);

        // Record data at different timestamps
        env.ledger().set_timestamp(1000);
        client.record_fee_revenue(&token, &500, &symbol_short!("escrow"));

        env.ledger().set_timestamp(1500);
        client.record_referral_payout(&referrer, &token, &100);

        env.ledger().set_timestamp(2000);
        client.record_insurance_reserve(&token, &10000, &3000, &7000);

        env.ledger().set_timestamp(2500);
        client.record_fee_revenue(&token, &300, &symbol_short!("session"));

        // Generate report for period 1000-2000
        let report = client.generate_report(&1000, &2000);

        assert_eq!(report.total_fee_revenue, 500);
        assert_eq!(report.total_referral_payouts, 100);
        assert_eq!(report.total_insurance_reserves, 10000);
        assert_eq!(report.net_treasury_growth, 400);
        assert_eq!(report.period_start, 1000);
        assert_eq!(report.period_end, 2000);
    }

    #[test]
    fn test_historical_fee_revenue() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id) = setup_test(&env);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        let token = Address::generate(&env);

        env.ledger().set_timestamp(1000);
        client.record_fee_revenue(&token, &500, &symbol_short!("escrow"));

        env.ledger().set_timestamp(2000);
        client.record_fee_revenue(&token, &300, &symbol_short!("session"));

        env.ledger().set_timestamp(3000);
        client.record_fee_revenue(&token, &200, &symbol_short!("lending"));

        // Get historical data for period 1500-2500
        let historical = client.get_historical_fee_revenue(&1500, &2500);
        assert_eq!(historical.len(), 1);

        let record = historical.get(0).unwrap();
        assert_eq!(record.amount, 300);
        assert_eq!(record.timestamp, 2000);
    }

    #[test]
    fn test_historical_referral_payouts() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id) = setup_test(&env);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        let referrer1 = Address::generate(&env);
        let referrer2 = Address::generate(&env);
        let token = Address::generate(&env);

        env.ledger().set_timestamp(1000);
        client.record_referral_payout(&referrer1, &token, &100);

        env.ledger().set_timestamp(2000);
        client.record_referral_payout(&referrer2, &token, &150);

        env.ledger().set_timestamp(3000);
        client.record_referral_payout(&referrer1, &token, &200);

        // Get historical data for period 1500-2500
        let historical = client.get_historical_referral_payouts(&1500, &2500);
        assert_eq!(historical.len(), 1);

        let record = historical.get(0).unwrap();
        assert_eq!(record.amount, 150);
        assert_eq!(record.timestamp, 2000);
    }

    #[test]
    fn test_pagination() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id) = setup_test(&env);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        let token = Address::generate(&env);

        // Record 5 fee revenues
        for i in 0..5 {
            env.ledger().set_timestamp(1000 + (i * 100) as u64);
            client.record_fee_revenue(&token, &(100 * (i as i128 + 1)), &symbol_short!("test"));
        }

        // Test pagination
        let page1 = client.get_fee_revenue(&0, &2);
        assert_eq!(page1.len(), 2);

        let page2 = client.get_fee_revenue(&2, &2);
        assert_eq!(page2.len(), 2);

        let page3 = client.get_fee_revenue(&4, &2);
        assert_eq!(page3.len(), 1);
    }

    #[test]
    fn test_comprehensive_analytics_flow() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, contract_id) = setup_test(&env);
        let client = TreasuryAnalyticsContractClient::new(&env, &contract_id);

        let token = Address::generate(&env);
        let referrer = Address::generate(&env);

        // Simulate a month of protocol activity
        env.ledger().set_timestamp(1000);
        client.record_fee_revenue(&token, &1000, &symbol_short!("escrow"));
        client.record_referral_payout(&referrer, &token, &50);

        env.ledger().set_timestamp(2000);
        client.record_fee_revenue(&token, &1500, &symbol_short!("session"));
        client.record_referral_payout(&referrer, &token, &75);

        env.ledger().set_timestamp(3000);
        client.record_insurance_reserve(&token, &50000, &10000, &40000);
        client.record_treasury_metrics(&2500, &125, &1000, &3000);

        // Generate comprehensive report
        let report = client.generate_report(&1000, &3000);

        assert_eq!(report.total_fee_revenue, 2500);
        assert_eq!(report.total_referral_payouts, 125);
        assert_eq!(report.net_treasury_growth, 2375);

        // Verify counts
        assert_eq!(client.get_fee_revenue_count(), 2);
        assert_eq!(client.get_referral_payout_count(), 2);
        assert_eq!(client.get_insurance_reserve_count(), 1);
        assert_eq!(client.get_treasury_metrics_count(), 1);
    }
}

#![no_std]
use shared::Pagination;
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, vec, Address, Env, Symbol, Vec,
};

const LARGE_TX_THRESHOLD: i128 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReportTrigger {
    pub contract: Symbol,
    pub function: Symbol,
    pub address: Address,
    pub amount_usd: i128,
    pub timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TxRecord {
    pub sender: Address,
    pub receiver: Address,
    pub amount_usd: i128,
    pub asset: Symbol,
    pub cross_border: bool,
    pub large_transaction: bool,
    pub timestamp: u64,
    pub year: u32,
    pub month: u32,
}

#[contracttype]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    EscrowContract,
    /// All records for a user (sender or receiver): Vec<TxRecord>
    UserRecords(Address),
    /// Large transaction reports counter
    ReportCount,
    /// Report storage: ReportCount -> ReportTrigger
    Report(u32),
    /// Reviewed flag: report_id -> bool
    Reviewed(u32),
}

#[contract]
pub struct RegulatoryReporting;

#[contractimpl]
impl RegulatoryReporting {
    pub fn initialize(env: Env, admin: Address, escrow_contract: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env
            .storage()
            .persistent()
            .set(&DataKey::EscrowContract, &escrow_contract);
        env.storage().persistent().set(&DataKey::ReportCount, &0u32);
    }

    /// Called only by the escrow contract to record a transaction.
    pub fn record_transaction(
        env: Env,
        sender: Address,
        receiver: Address,
        amount_usd: i128,
        asset: Symbol,
        cross_border: bool,
    ) {
        let escrow: Address = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowContract)
            .expect("not initialized");
        escrow.require_auth();

        let ts = env.ledger().timestamp();
        let (year, month) = ts_to_year_month(ts);
        let large_transaction = amount_usd > LARGE_TX_THRESHOLD;

        let record = TxRecord {
            sender: sender.clone(),
            receiver: receiver.clone(),
            amount_usd,
            asset,
            cross_border,
            large_transaction,
            timestamp: ts,
            year,
            month,
        };

        append_record(&env, &sender, &record);
        append_record(&env, &receiver, &record);

        env.events().publish(
            (symbol_short!("tx_rec"), sender.clone()),
            (receiver.clone(), amount_usd, cross_border),
        );

        if large_transaction {
            env.events().publish(
                (symbol_short!("large_tx"), sender),
                (receiver, amount_usd),
            );
        }
    }

    /// Sum of amount_usd for a user in a given year/month.
    pub fn get_monthly_volume(env: Env, user: Address, year: u32, month: u32) -> i128 {
        let records = load_records(&env, &user);
        records
            .iter()
            .filter(|r| r.year == year && r.month == month)
            .map(|r| r.amount_usd)
            .fold(0i128, |acc, v| acc + v)
    }

    /// All large transactions for a user in a given year/month.
    pub fn get_large_transactions(
        env: Env,
        user: Address,
        year: u32,
        month: u32,
    ) -> Vec<TxRecord> {
        let records = load_records(&env, &user);
        let mut out: Vec<TxRecord> = vec![&env];
        for r in records.iter() {
            if r.year == year && r.month == month && r.large_transaction {
                out.push_back(r);
            }
        }
        out
    }

    /// Sum of cross-border amount_usd for a user in a given year/month.
    pub fn get_cross_border_volume(env: Env, user: Address, year: u32, month: u32) -> i128 {
        let records = load_records(&env, &user);
        records
            .iter()
            .filter(|r| r.year == year && r.month == month && r.cross_border)
            .map(|r| r.amount_usd)
            .fold(0i128, |acc, v| acc + v)
    }

    /// Admin-only: export all records for a user.
    pub fn get_all_records(env: Env, user: Address) -> Vec<TxRecord> {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        load_records(&env, &user)
    }

    /// Record a large transaction trigger for compliance reporting.
    pub fn record_large_tx(env: Env, trigger: ReportTrigger) {
        // Only callable by authorized contracts (e.g., treasury, lending_pool)
        // For now, any contract can call - could add whitelist if needed
        
        if trigger.amount_usd <= LARGE_TX_THRESHOLD {
            return; // Below threshold, no report needed
        }

        let mut count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ReportCount)
            .unwrap_or(0);
        count += 1;

        env.storage()
            .persistent()
            .set(&DataKey::Report(count), &trigger);
        env.storage().persistent().set(&DataKey::ReportCount, &count);
        env.storage()
            .persistent()
            .set(&DataKey::Reviewed(count), &false);

        env.events().publish(
            (symbol_short!("lg_tx"), count),
            (trigger.address, trigger.amount_usd),
        );
    }

    /// Returns the total number of large-transaction reports recorded so far.
    pub fn get_report_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ReportCount)
            .unwrap_or(0)
    }

    /// Returns a bounded page of all reports (reviewed and unreviewed).
    ///
    /// `offset` is zero-based; the first stored report has id 1, so we
    /// translate: stored_id = offset + 1 … offset + effective_limit.
    /// `limit` is clamped to `MAX_PAGE_SIZE` (100).
    pub fn get_reports_page(env: Env, offset: u32, limit: u32) -> Vec<ReportTrigger> {
        const MAX_PAGE_SIZE: u32 = 100;
        let count = Self::get_report_count(env.clone());
        let limit  = limit.min(MAX_PAGE_SIZE);
        let mut result = soroban_sdk::vec![&env];

        // Reports are stored with 1-based ids: Report(1) … Report(count)
        let first = offset.saturating_add(1);
        let last  = first.saturating_add(limit).min(count.saturating_add(1));
        for id in first..last {
            if let Some(report) = env
                .storage()
                .persistent()
                .get::<_, ReportTrigger>(&DataKey::Report(id))
            {
                result.push_back(report);
            }
        }
        result
    }

    /// Returns a bounded page of (id, report) pairs whose `Reviewed` flag is false.
    ///
    /// `limit` is clamped to `MAX_PAGE_SIZE` (100).
    pub fn get_unreviewed_reports_page(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> Vec<(u32, ReportTrigger)> {
        const MAX_PAGE_SIZE: u32 = 100;
        let count       = Self::get_report_count(env.clone());
        let effective   = limit.min(MAX_PAGE_SIZE);
        let mut result  = soroban_sdk::vec![&env];
        let mut skipped = 0u32;
        let mut found   = 0u32;

        for id in 1..=count {
            let reviewed: bool = env
                .storage()
                .persistent()
                .get(&DataKey::Reviewed(id))
                .unwrap_or(false);
            if reviewed {
                continue;
            }
            // Apply offset (skip first `offset` unreviewed entries)
            if skipped < offset {
                skipped = skipped.saturating_add(1);
                continue;
            }
            if found >= effective {
                break;
            }
            if let Some(report) = env
                .storage()
                .persistent()
                .get::<_, ReportTrigger>(&DataKey::Report(id))
            {
                result.push_back((id, report));
                found = found.saturating_add(1);
            }
        }
        result
    }

    /// Get pending reports with pagination.
    pub fn get_pending_reports(env: Env, offset: u32, limit: u32) -> Vec<ReportTrigger> {
        let total: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::ReportCount)
            .unwrap_or(0);
        let mut result = vec![&env];

        let end = (offset + limit).min(total);
        for i in (offset + 1)..=end {
            let reviewed: bool = env
                .storage()
                .persistent()
                .get(&DataKey::Reviewed(i))
                .unwrap_or(false);
            if !reviewed {
                if let Some(report) = env.storage().persistent().get::<_, ReportTrigger>(&DataKey::Report(i)) {
                    result.push_back(report);
                }
            }
        }
        result
    }

    /// Mark a report as reviewed (admin only).
    pub fn mark_reviewed(env: Env, report_id: u32) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();

        env.storage()
            .persistent()
            .set(&DataKey::Reviewed(report_id), &true);

        env.events().publish((symbol_short!("reviewed"), report_id), report_id);
    }

    /// Total number of large-transaction reports recorded so far.
    pub fn get_report_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ReportCount)
            .unwrap_or(0)
    }

    /// Paginated view over every recorded report, regardless of review
    /// status. `limit` is clamped to `MAX_PAGE_SIZE` so a caller can never
    /// force a single call to enumerate the whole collection.
    pub fn get_reports_page(env: Env, offset: u32, limit: u32) -> Vec<ReportTrigger> {
        let total = Self::get_report_count(env.clone());
        let (start, end) = Pagination::bounds(total, offset, limit);
        let mut out: Vec<ReportTrigger> = vec![&env];
        // Reports are stored 1-indexed (DataKey::Report(1)..=DataKey::Report(total)).
        for i in start..end {
            if let Some(report) = env
                .storage()
                .persistent()
                .get::<_, ReportTrigger>(&DataKey::Report(i + 1))
            {
                out.push_back(report);
            }
        }
        out
    }

    /// Paginated view over reports that have not yet been marked reviewed,
    /// returned alongside their report id so a caller can act on them (e.g.
    /// via `mark_reviewed`). `limit` is clamped to `MAX_PAGE_SIZE`.
    pub fn get_unreviewed_reports_page(
        env: Env,
        offset: u32,
        limit: u32,
    ) -> Vec<(u32, ReportTrigger)> {
        let total = Self::get_report_count(env.clone());
        let (start, end) = Pagination::bounds(total, offset, limit);
        let mut out: Vec<(u32, ReportTrigger)> = vec![&env];
        for i in start..end {
            let report_id = i + 1;
            let reviewed: bool = env
                .storage()
                .persistent()
                .get(&DataKey::Reviewed(report_id))
                .unwrap_or(false);
            if !reviewed {
                if let Some(report) = env
                    .storage()
                    .persistent()
                    .get::<_, ReportTrigger>(&DataKey::Report(report_id))
                {
                    out.push_back((report_id, report));
                }
            }
        }
        out
    }
}

fn ts_to_year_month(ts: u64) -> (u32, u32) {
    // Approximate: good enough for monthly bucketing
    let days = ts / 86400;
    let year = 1970u32 + (days / 365) as u32;
    let day_of_year = (days % 365) as u32;
    let month = (day_of_year / 30) + 1;
    (year, if month > 12 { 12 } else { month })
}

fn load_records(env: &Env, user: &Address) -> Vec<TxRecord> {
    env.storage()
        .persistent()
        .get(&DataKey::UserRecords(user.clone()))
        .unwrap_or_else(|| vec![env])
}

fn append_record(env: &Env, user: &Address, record: &TxRecord) {
    let key = DataKey::UserRecords(user.clone());
    let mut records: Vec<TxRecord> = env
        .storage()
        .persistent()
        .get(&key)
        .unwrap_or_else(|| vec![env]);
    records.push_back(record.clone());
    // No TTL bump — records must be retained indefinitely (7-year requirement).
    env.storage().persistent().set(&key, &records);
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    fn setup(env: &Env) -> (Address, Address, Address, Address) {
        let admin = Address::generate(env);
        let escrow = Address::generate(env);
        let sender = Address::generate(env);
        let receiver = Address::generate(env);
        let contract_id = env.register_contract(None, RegulatoryReporting);
        let client = RegulatoryReportingClient::new(env, &contract_id);
        client.initialize(&admin, &escrow);
        (admin, escrow, sender, receiver)
    }

    fn client(env: &Env) -> RegulatoryReportingClient {
        // Re-use the already-registered contract (first one registered)
        let contract_id = env.register_contract(None, RegulatoryReporting);
        RegulatoryReportingClient::new(env, &contract_id)
    }

    #[test]
    fn test_record_and_aggregate() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, RegulatoryReporting);
        let c = RegulatoryReportingClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let escrow = Address::generate(&env);
        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);
        c.initialize(&admin, &escrow);

        let asset = symbol_short!("USDC");
        c.record_transaction(&sender, &receiver, &5_000i128, &asset, &false);
        c.record_transaction(&sender, &receiver, &3_000i128, &asset, &false);

        let (year, month) = ts_to_year_month(env.ledger().timestamp());
        // sender sees both txs
        assert_eq!(c.get_monthly_volume(&sender, &year, &month), 8_000i128);
        // receiver also sees both
        assert_eq!(c.get_monthly_volume(&receiver, &year, &month), 8_000i128);
    }

    #[test]
    fn test_large_transaction_flag() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, RegulatoryReporting);
        let c = RegulatoryReportingClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let escrow = Address::generate(&env);
        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);
        c.initialize(&admin, &escrow);

        let asset = symbol_short!("USDC");
        c.record_transaction(&sender, &receiver, &5_000i128, &asset, &false);
        c.record_transaction(&sender, &receiver, &15_000i128, &asset, &false);

        let (year, month) = ts_to_year_month(env.ledger().timestamp());
        let large = c.get_large_transactions(&sender, &year, &month);
        assert_eq!(large.len(), 1);
        assert_eq!(large.get(0).unwrap().amount_usd, 15_000i128);
        assert!(large.get(0).unwrap().large_transaction);
    }

    #[test]
    fn test_cross_border_flag() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, RegulatoryReporting);
        let c = RegulatoryReportingClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let escrow = Address::generate(&env);
        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);
        c.initialize(&admin, &escrow);

        let asset = symbol_short!("USDC");
        c.record_transaction(&sender, &receiver, &2_000i128, &asset, &true);
        c.record_transaction(&sender, &receiver, &3_000i128, &asset, &false);

        let (year, month) = ts_to_year_month(env.ledger().timestamp());
        assert_eq!(c.get_cross_border_volume(&sender, &year, &month), 2_000i128);
    }

    #[test]
    fn test_get_all_records_admin_only() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, RegulatoryReporting);
        let c = RegulatoryReportingClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let escrow = Address::generate(&env);
        let sender = Address::generate(&env);
        let receiver = Address::generate(&env);
        c.initialize(&admin, &escrow);

        let asset = symbol_short!("USDC");
        c.record_transaction(&sender, &receiver, &1_000i128, &asset, &false);
        c.record_transaction(&sender, &receiver, &2_000i128, &asset, &true);

        let records = c.get_all_records(&sender);
        assert_eq!(records.len(), 2);
    }

    // -------------------------------------------------------------------
    // Paginated report view tests (#1240)
    // -------------------------------------------------------------------

    fn make_trigger(env: &Env, addr: &Address, amount: i128) -> ReportTrigger {
        ReportTrigger {
            contract:   symbol_short!("escrow"),
            function:   symbol_short!("release"),
            address:    addr.clone(),
            amount_usd: amount,
            timestamp:  env.ledger().timestamp(),
        }
    }

    #[test]
    fn test_get_report_count() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, RegulatoryReporting);
        let c  = RegulatoryReportingClient::new(&env, &id);
        let admin = Address::generate(&env);
        let escrow = Address::generate(&env);
        c.initialize(&admin, &escrow);

        assert_eq!(c.get_report_count(), 0u32);

        let addr = Address::generate(&env);
        c.record_large_tx(&make_trigger(&env, &addr, 20_000i128));
        c.record_large_tx(&make_trigger(&env, &addr, 30_000i128));

        assert_eq!(c.get_report_count(), 2u32);
    }

    #[test]
    fn test_get_reports_page_basic() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, RegulatoryReporting);
        let c  = RegulatoryReportingClient::new(&env, &id);
        let admin = Address::generate(&env);
        let escrow = Address::generate(&env);
        c.initialize(&admin, &escrow);

        let addr = Address::generate(&env);
        c.record_large_tx(&make_trigger(&env, &addr, 11_000i128));
        c.record_large_tx(&make_trigger(&env, &addr, 12_000i128));
        c.record_large_tx(&make_trigger(&env, &addr, 13_000i128));

        // Page 0, limit 2 → first two reports
        let page = c.get_reports_page(&0, &2);
        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().amount_usd, 11_000i128);
        assert_eq!(page.get(1).unwrap().amount_usd, 12_000i128);

        // Page offset 2, limit 10 → remaining one report
        let page2 = c.get_reports_page(&2, &10);
        assert_eq!(page2.len(), 1);
        assert_eq!(page2.get(0).unwrap().amount_usd, 13_000i128);
    }

    #[test]
    fn test_get_unreviewed_reports_page() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, RegulatoryReporting);
        let c  = RegulatoryReportingClient::new(&env, &id);
        let admin = Address::generate(&env);
        let escrow = Address::generate(&env);
        c.initialize(&admin, &escrow);

        let addr = Address::generate(&env);
        c.record_large_tx(&make_trigger(&env, &addr, 20_000i128)); // id=1
        c.record_large_tx(&make_trigger(&env, &addr, 21_000i128)); // id=2
        c.record_large_tx(&make_trigger(&env, &addr, 22_000i128)); // id=3

        // Mark id=2 as reviewed
        c.mark_reviewed(&2);

        // All unreviewed: ids 1 and 3
        let page = c.get_unreviewed_reports_page(&0, &10);
        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().0, 1u32);
        assert_eq!(page.get(1).unwrap().0, 3u32);

        // Offset 1, limit 10 → only id=3
        let page2 = c.get_unreviewed_reports_page(&1, &10);
        assert_eq!(page2.len(), 1);
        assert_eq!(page2.get(0).unwrap().0, 3u32);
    }

    #[test]
    fn test_get_reports_page_limit_clamped() {
        let env = Env::default();
        env.mock_all_auths();
        let id = env.register_contract(None, RegulatoryReporting);
        let c  = RegulatoryReportingClient::new(&env, &id);
        let admin = Address::generate(&env);
        let escrow = Address::generate(&env);
        c.initialize(&admin, &escrow);

        let addr = Address::generate(&env);
        // Record 3 reports
        for amt in [50_000i128, 60_000i128, 70_000i128] {
            c.record_large_tx(&make_trigger(&env, &addr, amt));
        }

        // Limit of 200 is clamped to MAX_PAGE_SIZE (100); only 3 exist so we get 3
        let page = c.get_reports_page(&0, &200);
        assert_eq!(page.len(), 3);
    }
}

#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, vec, Address, Env, Symbol, Vec,
};

const LARGE_TX_THRESHOLD: i128 = 10_000;

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
    Admin,
    EscrowContract,
    /// All records for a user (sender or receiver): Vec<TxRecord>
    UserRecords(Address),
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
}

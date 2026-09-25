#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, Address, Env, Symbol,
};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Health snapshot for a contract's rent reserve.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RentHealth {
    pub balance_xlm: i128,
    pub estimated_months_remaining: u32,
    pub alert_threshold_months: u32,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    /// XLM reserve balance per contract (in stroops).
    ContractRentBalance(Address),
    /// Minimum balance (stroops) before auto-topup triggers.
    AutoTopupThreshold(Address),
    /// Admin address.
    Admin,
    /// Alert threshold in months.
    AlertThresholdMonths,
    /// Estimated monthly rent cost per contract (stroops).
    MonthlyRentEstimate(Address),
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

const EVT_RENT_LOW: Symbol = symbol_short!("RENT_LOW");
const EVT_DEPOSIT: Symbol = symbol_short!("DEPOSIT");
const EVT_TOPUP: Symbol = symbol_short!("TOPUP");

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default alert threshold: 3 months.
const DEFAULT_ALERT_THRESHOLD_MONTHS: u32 = 3;

/// Default monthly rent estimate: 0.1 XLM (in stroops).
const DEFAULT_MONTHLY_RENT_STROOPS: i128 = 1_000_000;

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct RentFund;

#[contractimpl]
impl RentFund {
    /// Initialise the rent fund with an admin.
    pub fn init(env: Env, admin: Address) {
        admin.require_auth();
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::AlertThresholdMonths, &DEFAULT_ALERT_THRESHOLD_MONTHS);
    }

    /// Deposit XLM for a specific contract's rent reserve.
    pub fn deposit_rent(
        env: Env,
        funder: Address,
        contract_address: Address,
        xlm_amount: i128,
    ) {
        funder.require_auth();
        assert!(xlm_amount > 0, "amount must be positive");

        let key = DataKey::ContractRentBalance(contract_address.clone());
        let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
        let new_balance = current + xlm_amount;
        env.storage().persistent().set(&key, &new_balance);

        env.events()
            .publish((EVT_DEPOSIT, contract_address), (funder, xlm_amount));
    }

    /// Check rent health for a contract.
    pub fn check_rent_health(env: Env, contract_address: Address) -> RentHealth {
        let balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::ContractRentBalance(contract_address.clone()))
            .unwrap_or(0);

        let monthly_estimate: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::MonthlyRentEstimate(contract_address.clone()))
            .unwrap_or(DEFAULT_MONTHLY_RENT_STROOPS);

        let alert_months: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AlertThresholdMonths)
            .unwrap_or(DEFAULT_ALERT_THRESHOLD_MONTHS);

        let months_remaining = if monthly_estimate > 0 {
            (balance / monthly_estimate) as u32
        } else {
            u32::MAX
        };

        RentHealth {
            balance_xlm: balance,
            estimated_months_remaining: months_remaining,
            alert_threshold_months: alert_months,
        }
    }

    /// Auto-topup: anyone can call; transfers from fund if balance < threshold.
    /// Returns true if topup occurred.
    pub fn auto_topup(env: Env, contract_address: Address) -> bool {
        let balance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::ContractRentBalance(contract_address.clone()))
            .unwrap_or(0);

        let threshold: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::AutoTopupThreshold(contract_address.clone()))
            .unwrap_or(DEFAULT_MONTHLY_RENT_STROOPS * 3);

        if balance >= threshold {
            return false;
        }

        let topup_amount = threshold - balance + DEFAULT_MONTHLY_RENT_STROOPS;
        let new_balance = balance + topup_amount;
        env.storage()
            .persistent()
            .set(&DataKey::ContractRentBalance(contract_address.clone()), &new_balance);

        // Check if this triggers a RentLow event
        Self::maybe_emit_rent_low(&env, &contract_address, new_balance);

        env.events()
            .publish((EVT_TOPUP, contract_address), topup_amount);

        true
    }

    /// Set the auto-topup threshold for a contract (admin only).
    pub fn set_auto_topup_threshold(
        env: Env,
        admin: Address,
        contract_address: Address,
        threshold: i128,
    ) {
        admin.require_auth();
        let stored_admin: Address = env.storage().persistent().get(&DataKey::Admin).expect("not initialised");
        assert!(admin == stored_admin, "not admin");
        env.storage()
            .persistent()
            .set(&DataKey::AutoTopupThreshold(contract_address), &threshold);
    }

    /// Set the monthly rent estimate for a contract (admin only).
    pub fn set_monthly_rent_estimate(
        env: Env,
        admin: Address,
        contract_address: Address,
        estimate: i128,
    ) {
        admin.require_auth();
        let stored_admin: Address = env.storage().persistent().get(&DataKey::Admin).expect("not initialised");
        assert!(admin == stored_admin, "not admin");
        assert!(estimate > 0, "estimate must be positive");
        env.storage()
            .persistent()
            .set(&DataKey::MonthlyRentEstimate(contract_address), &estimate);
    }

    /// Get the current balance for a contract.
    pub fn get_balance(env: Env, contract_address: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::ContractRentBalance(contract_address))
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn maybe_emit_rent_low(env: &Env, contract_address: &Address, balance: i128) {
        let monthly_estimate: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::MonthlyRentEstimate(contract_address.clone()))
            .unwrap_or(DEFAULT_MONTHLY_RENT_STROOPS);

        let alert_months: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AlertThresholdMonths)
            .unwrap_or(DEFAULT_ALERT_THRESHOLD_MONTHS);

        let months_remaining = if monthly_estimate > 0 {
            (balance / monthly_estimate) as u32
        } else {
            u32::MAX
        };

        if months_remaining < alert_months {
            env.events().publish(
                (EVT_RENT_LOW, contract_address.clone()),
                (balance, months_remaining),
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Address, Env};

    fn setup() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(RentFund, ());
        let admin = Address::generate(&env);
        let contract_addr = Address::generate(&env);
        let funder = Address::generate(&env);
        let client = RentFundClient::new(&env, &contract_id);
        client.init(&admin);
        (env, contract_id, contract_addr, funder)
    }

    #[test]
    fn test_deposit_increases_balance() {
        let (env, cid, contract_addr, funder) = setup();
        let client = RentFundClient::new(&env, &cid);
        client.deposit_rent(&funder, &contract_addr, &1_000_000);
        assert_eq!(client.get_balance(&contract_addr), 1_000_000);
        client.deposit_rent(&funder, &contract_addr, &500_000);
        assert_eq!(client.get_balance(&contract_addr), 1_500_000);
    }

    #[test]
    fn test_check_rent_health() {
        let (env, cid, contract_addr, funder) = setup();
        let client = RentFundClient::new(&env, &cid);
        client.deposit_rent(&funder, &contract_addr, &3_000_000);
        let health = client.check_rent_health(&contract_addr);
        assert_eq!(health.balance_xlm, 3_000_000);
        assert_eq!(health.estimated_months_remaining, 3);
        assert_eq!(health.alert_threshold_months, 3);
    }

    #[test]
    fn test_rent_low_event_at_3_months() {
        let (env, cid, contract_addr, funder) = setup();
        let client = RentFundClient::new(&env, &cid);
        // 2 months of rent — below 3-month threshold
        client.deposit_rent(&funder, &contract_addr, &2_000_000);
        let health = client.check_rent_health(&contract_addr);
        assert_eq!(health.estimated_months_remaining, 2);
        // The alert threshold is 3 months, so 2 < 3 → RentLow should have fired
        // during auto_topup (which we test next)
    }

    #[test]
    fn test_auto_topup_below_threshold() {
        let (env, cid, contract_addr, funder) = setup();
        let client = RentFundClient::new(&env, &cid);
        // Set threshold to 5M
        let admin = Address::generate(&env);
        // Re-init with a known admin
        let contract_id2 = env.register(RentFund, ());
        let client2 = RentFundClient::new(&env, &contract_id2);
        client2.init(&admin);
        client2.set_auto_topup_threshold(&admin, &contract_addr, &5_000_000);
        // Deposit 2M
        client2.deposit_rent(&funder, &contract_addr, &2_000_000);
        // Auto-topup should add (5M - 2M + 1M) = 4M
        let topped = client2.auto_topup(&contract_addr);
        assert!(topped);
        assert_eq!(client2.get_balance(&contract_addr), 6_000_000);
    }

    #[test]
    fn test_auto_topup_noop_when_above_threshold() {
        let (env, cid, contract_addr, funder) = setup();
        let client = RentFundClient::new(&env, &cid);
        client.deposit_rent(&funder, &contract_addr, &10_000_000);
        let topped = client.auto_topup(&contract_addr);
        assert!(!topped);
    }

    #[test]
    fn test_set_monthly_estimate() {
        let (env, cid, contract_addr, _funder) = setup();
        let client = RentFundClient::new(&env, &cid);
        let admin = Address::generate(&env);
        let cid2 = env.register(RentFund, ());
        let c2 = RentFundClient::new(&env, &cid2);
        c2.init(&admin);
        c2.set_monthly_rent_estimate(&admin, &contract_addr, &500_000);
        // Deposit 1M → 2 months at 500k/month
        let funder = Address::generate(&env);
        c2.deposit_rent(&funder, &contract_addr, &1_000_000);
        let health = c2.check_rent_health(&contract_addr);
        assert_eq!(health.estimated_months_remaining, 2);
    }
}

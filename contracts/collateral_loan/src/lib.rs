#![no_std]

use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, token, Address, Env, Symbol,
};

const MIN_COLLATERAL_RATIO_BPS: i128 = 15_000; // 150%
const LIQUIDATION_THRESHOLD_BPS: i128 = 12_000; // 120%
const LIQUIDATOR_BONUS_BPS: i128 = 500; // 5%
const BPS_DENOMINATOR: i128 = 10_000;
const PRICE_SCALE: i128 = 10_000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Loan {
    pub collateral_amount: i128,
    pub debt_amount: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    MntToken,
    UsdcToken,
    Oracle,
    MntAsset,
    Loan(Address),
}

#[contractclient(name = "OracleClient")]
pub trait OracleTrait {
    fn get_price(env: Env, asset: Symbol) -> (i128, u64);
}

#[contract]
pub struct CollateralLoanContract;

#[contractimpl]
impl CollateralLoanContract {
    pub fn initialize(
        env: Env,
        admin: Address,
        mnt_token: Address,
        usdc_token: Address,
        oracle: Address,
        mnt_asset: Symbol,
    ) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }

        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MntToken, &mnt_token);
        env.storage()
            .instance()
            .set(&DataKey::UsdcToken, &usdc_token);
        env.storage().instance().set(&DataKey::Oracle, &oracle);
        env.storage().instance().set(&DataKey::MntAsset, &mnt_asset);
    }

    pub fn open_loan(env: Env, borrower: Address, collateral_amount: i128, borrow_amount: i128) {
        Self::require_initialized(&env);
        borrower.require_auth();

        if collateral_amount <= 0 || borrow_amount <= 0 {
            panic!("invalid amount");
        }

        let loan_key = DataKey::Loan(borrower.clone());
        if env.storage().persistent().has(&loan_key) {
            panic!("loan already exists");
        }

        let price = Self::get_mnt_price(&env);
        let ratio_bps = Self::compute_ratio_bps(collateral_amount, borrow_amount, price);
        if ratio_bps < MIN_COLLATERAL_RATIO_BPS {
            panic!("insufficient collateralization");
        }

        let mnt = Self::mnt_token(&env);
        let usdc = Self::usdc_token(&env);

        let mnt_client = token::Client::new(&env, &mnt);
        mnt_client.transfer(
            &borrower,
            &env.current_contract_address(),
            &collateral_amount,
        );

        let usdc_client = token::Client::new(&env, &usdc);
        usdc_client.transfer(&env.current_contract_address(), &borrower, &borrow_amount);

        let loan = Loan {
            collateral_amount,
            debt_amount: borrow_amount,
        };
        env.storage().persistent().set(&loan_key, &loan);

        env.events().publish(
            (Symbol::new(&env, "loan_opened"), borrower),
            (collateral_amount, borrow_amount),
        );
    }

    pub fn repay_loan(env: Env, borrower: Address, amount: i128) {
        Self::require_initialized(&env);
        borrower.require_auth();

        if amount <= 0 {
            panic!("invalid amount");
        }

        let loan_key = DataKey::Loan(borrower.clone());
        let mut loan: Loan = env
            .storage()
            .persistent()
            .get(&loan_key)
            .expect("loan not found");

        if loan.debt_amount <= 0 {
            panic!("loan already repaid");
        }

        let pay_amount = if amount > loan.debt_amount {
            loan.debt_amount
        } else {
            amount
        };

        let usdc = Self::usdc_token(&env);
        let usdc_client = token::Client::new(&env, &usdc);
        usdc_client.transfer(&borrower, &env.current_contract_address(), &pay_amount);

        loan.debt_amount -= pay_amount;

        if loan.debt_amount == 0 {
            let mnt = Self::mnt_token(&env);
            let mnt_client = token::Client::new(&env, &mnt);
            mnt_client.transfer(
                &env.current_contract_address(),
                &borrower,
                &loan.collateral_amount,
            );
            env.storage().persistent().remove(&loan_key);
        } else {
            env.storage().persistent().set(&loan_key, &loan);
        }

        env.events().publish(
            (Symbol::new(&env, "repaid"), borrower),
            (pay_amount, loan.debt_amount),
        );
    }

    pub fn add_collateral(env: Env, borrower: Address, amount: i128) {
        Self::require_initialized(&env);
        borrower.require_auth();

        if amount <= 0 {
            panic!("invalid amount");
        }

        let loan_key = DataKey::Loan(borrower.clone());
        let mut loan: Loan = env
            .storage()
            .persistent()
            .get(&loan_key)
            .expect("loan not found");

        let mnt = Self::mnt_token(&env);
        let mnt_client = token::Client::new(&env, &mnt);
        mnt_client.transfer(&borrower, &env.current_contract_address(), &amount);

        loan.collateral_amount += amount;
        env.storage().persistent().set(&loan_key, &loan);

        env.events().publish(
            (Symbol::new(&env, "collateral_added"), borrower),
            (amount, loan.collateral_amount),
        );
    }

    pub fn liquidate(env: Env, borrower: Address, liquidator: Address) {
        Self::require_initialized(&env);
        liquidator.require_auth();

        let loan_key = DataKey::Loan(borrower.clone());
        let loan: Loan = env
            .storage()
            .persistent()
            .get(&loan_key)
            .expect("loan not found");

        if loan.debt_amount <= 0 {
            panic!("loan already repaid");
        }

        let ratio_bps = Self::get_health_factor(env.clone(), borrower.clone()) as i128;
        if ratio_bps >= LIQUIDATION_THRESHOLD_BPS {
            panic!("loan healthy");
        }

        let bonus = (loan.collateral_amount * LIQUIDATOR_BONUS_BPS) / BPS_DENOMINATOR;

        let mnt = Self::mnt_token(&env);
        let mnt_client = token::Client::new(&env, &mnt);
        mnt_client.transfer(&env.current_contract_address(), &liquidator, &bonus);

        env.storage().persistent().remove(&loan_key);

        env.events().publish(
            (Symbol::new(&env, "liquidated"), borrower, liquidator),
            (loan.collateral_amount, loan.debt_amount, bonus),
        );
    }

    pub fn get_health_factor(env: Env, borrower: Address) -> u32 {
        Self::require_initialized(&env);

        let loan: Loan = match env.storage().persistent().get(&DataKey::Loan(borrower)) {
            Some(l) => l,
            None => return 0,
        };

        if loan.debt_amount <= 0 {
            return u32::MAX;
        }

        let price = Self::get_mnt_price(&env);
        let ratio = Self::compute_ratio_bps(loan.collateral_amount, loan.debt_amount, price);

        if ratio < 0 {
            0
        } else if ratio > u32::MAX as i128 {
            u32::MAX
        } else {
            ratio as u32
        }
    }

    pub fn get_loan(env: Env, borrower: Address) -> Option<Loan> {
        env.storage().persistent().get(&DataKey::Loan(borrower))
    }

    fn compute_ratio_bps(collateral_amount: i128, debt_amount: i128, price: i128) -> i128 {
        if debt_amount <= 0 {
            return i128::MAX;
        }
        let collateral_value = collateral_amount
            .checked_mul(price)
            .expect("overflow")
            .checked_div(PRICE_SCALE)
            .expect("invalid price scale");

        collateral_value
            .checked_mul(BPS_DENOMINATOR)
            .expect("overflow")
            .checked_div(debt_amount)
            .expect("division by zero")
    }

    fn get_mnt_price(env: &Env) -> i128 {
        let oracle = Self::oracle(env);
        let asset = Self::mnt_asset(env);
        let (price, _) = OracleClient::new(env, &oracle).get_price(&asset);
        if price <= 0 {
            panic!("invalid oracle price");
        }
        price
    }

    fn require_initialized(env: &Env) {
        if !env.storage().instance().has(&DataKey::Admin) {
            panic!("not initialized");
        }
    }

    fn mnt_token(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::MntToken)
            .expect("not initialized")
    }

    fn usdc_token(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::UsdcToken)
            .expect("not initialized")
    }

    fn oracle(env: &Env) -> Address {
        env.storage()
            .instance()
            .get(&DataKey::Oracle)
            .expect("not initialized")
    }

    fn mnt_asset(env: &Env) -> Symbol {
        env.storage()
            .instance()
            .get(&DataKey::MntAsset)
            .expect("not initialized")
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{contract, contractimpl, symbol_short};

    #[contracttype]
    #[derive(Clone)]
    enum MockTokenDataKey {
        Balance(Address),
    }

    #[contract]
    struct MockToken;

    #[contractimpl]
    impl MockToken {
        pub fn mint(env: Env, to: Address, amount: i128) {
            let current = Self::balance(env.clone(), to.clone());
            env.storage()
                .persistent()
                .set(&MockTokenDataKey::Balance(to), &(current + amount));
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&MockTokenDataKey::Balance(id))
                .unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            let from_bal = Self::balance(env.clone(), from.clone());
            assert!(amount >= 0, "negative transfer");
            assert!(from_bal >= amount, "insufficient balance");
            let to_bal = Self::balance(env.clone(), to.clone());

            env.storage()
                .persistent()
                .set(&MockTokenDataKey::Balance(from), &(from_bal - amount));
            env.storage()
                .persistent()
                .set(&MockTokenDataKey::Balance(to), &(to_bal + amount));
        }
    }

    #[contract]
    struct MockOracle;

    #[contractimpl]
    impl MockOracle {
        pub fn set_price(env: Env, asset: Symbol, price: i128) {
            env.storage().persistent().set(&asset, &price);
        }

        pub fn get_price(env: Env, asset: Symbol) -> (i128, u64) {
            (
                env.storage()
                    .persistent()
                    .get(&asset)
                    .expect("price not set"),
                0,
            )
        }
    }

    struct Fixture {
        env: Env,
        contract_id: Address,
        borrower: Address,
        liquidator: Address,
        mnt_token_id: Address,
        usdc_token_id: Address,
        oracle_id: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let borrower = Address::generate(&env);
            let liquidator = Address::generate(&env);

            let mnt_token_id = env.register_contract(None, MockToken);
            let usdc_token_id = env.register_contract(None, MockToken);
            let oracle_id = env.register_contract(None, MockOracle);
            let contract_id = env.register_contract(None, CollateralLoanContract);

            let contract = CollateralLoanContractClient::new(&env, &contract_id);
            let oracle = MockOracleClient::new(&env, &oracle_id);
            let mnt = MockTokenClient::new(&env, &mnt_token_id);
            let usdc = MockTokenClient::new(&env, &usdc_token_id);

            contract.initialize(
                &admin,
                &mnt_token_id,
                &usdc_token_id,
                &oracle_id,
                &symbol_short!("MNT"),
            );

            // Price = 2.0 USDC per MNT.
            oracle.set_price(&symbol_short!("MNT"), &20_000);

            // Borrower starts with MNT collateral inventory.
            mnt.mint(&borrower, &1_000);

            // Loan contract starts with USDC liquidity for disbursement.
            usdc.mint(&contract_id, &10_000);

            Self {
                env,
                contract_id,
                borrower,
                liquidator,
                mnt_token_id,
                usdc_token_id,
                oracle_id,
            }
        }

        fn contract(&self) -> CollateralLoanContractClient {
            CollateralLoanContractClient::new(&self.env, &self.contract_id)
        }

        fn mnt(&self) -> MockTokenClient {
            MockTokenClient::new(&self.env, &self.mnt_token_id)
        }

        fn usdc(&self) -> MockTokenClient {
            MockTokenClient::new(&self.env, &self.usdc_token_id)
        }

        fn oracle(&self) -> MockOracleClient {
            MockOracleClient::new(&self.env, &self.oracle_id)
        }
    }

    #[test]
    fn test_open_loan() {
        let f = Fixture::setup();
        let contract = f.contract();

        contract.open_loan(&f.borrower, &100, &120);

        let loan = contract.get_loan(&f.borrower).unwrap();
        assert_eq!(loan.collateral_amount, 100);
        assert_eq!(loan.debt_amount, 120);

        assert_eq!(f.mnt().balance(&f.borrower), 900);
        assert_eq!(f.mnt().balance(&f.contract_id), 100);

        assert_eq!(f.usdc().balance(&f.borrower), 120);
        assert_eq!(f.usdc().balance(&f.contract_id), 9_880);

        assert_eq!(contract.get_health_factor(&f.borrower), 16_666);
    }

    #[test]
    fn test_repay_partial_and_full() {
        let f = Fixture::setup();
        let contract = f.contract();

        contract.open_loan(&f.borrower, &100, &120);
        contract.repay_loan(&f.borrower, &20);

        let partial = contract.get_loan(&f.borrower).unwrap();
        assert_eq!(partial.debt_amount, 100);
        assert_eq!(f.usdc().balance(&f.borrower), 100);

        // Overpay request only repays outstanding debt.
        contract.repay_loan(&f.borrower, &200);

        assert_eq!(contract.get_loan(&f.borrower), None);
        assert_eq!(f.mnt().balance(&f.borrower), 1_000);
        assert_eq!(f.mnt().balance(&f.contract_id), 0);
    }

    #[test]
    fn test_add_collateral() {
        let f = Fixture::setup();
        let contract = f.contract();

        contract.open_loan(&f.borrower, &100, &120);
        contract.add_collateral(&f.borrower, &50);

        let loan = contract.get_loan(&f.borrower).unwrap();
        assert_eq!(loan.collateral_amount, 150);
        assert_eq!(contract.get_health_factor(&f.borrower), 25_000);
    }

    #[test]
    fn test_liquidation_trigger() {
        let f = Fixture::setup();
        let contract = f.contract();

        contract.open_loan(&f.borrower, &100, &120);

        // Drop MNT price from 2.0 to 1.0 USDC so health becomes 83.33%.
        f.oracle().set_price(&symbol_short!("MNT"), &10_000);

        assert_eq!(contract.get_health_factor(&f.borrower), 8_333);
        contract.liquidate(&f.borrower, &f.liquidator);

        assert_eq!(contract.get_loan(&f.borrower), None);
    }

    #[test]
    fn test_liquidator_bonus() {
        let f = Fixture::setup();
        let contract = f.contract();

        contract.open_loan(&f.borrower, &100, &120);
        f.oracle().set_price(&symbol_short!("MNT"), &10_000);

        let before = f.mnt().balance(&f.liquidator);
        contract.liquidate(&f.borrower, &f.liquidator);
        let after = f.mnt().balance(&f.liquidator);

        // 5% of 100 collateral.
        assert_eq!(after - before, 5);
    }
}

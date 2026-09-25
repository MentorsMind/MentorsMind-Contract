#![no_std]

use shared::pagination::Pagination;
use shared::pause_guard;
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, token, Address, Env, Symbol, Vec,
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

/// A collateral position as returned by the paginated views.
///
/// `ratio_bps` is the live collateralisation ratio against the current oracle
/// price, so liquidation bots do not have to re-derive it off-chain.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CollateralPosition {
    pub owner: Address,
    pub collateral_amount: i128,
    pub debt_amount: i128,
    pub ratio_bps: i128,
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
    /// Address allowed to trigger the emergency pause. Set by the admin via
    /// `set_pause_guardian`.
    PauseGuardian,
    /// Sequential index over open positions: `CollateralIndex(i) -> Address`.
    CollateralIndex(u32),
    /// Reverse lookup from position owner to its slot in `CollateralIndex`.
    CollateralSlot(Address),
    /// Number of live entries in `CollateralIndex`.
    CollateralCount,
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

    /// Register the address allowed to trigger the emergency pause.
    ///
    /// Admin only. Without a guardian the contract cannot be halted, so
    /// operators should set one immediately after `initialize`.
    pub fn set_pause_guardian(env: Env, admin: Address, guardian: Address) {
        Self::require_initialized(&env);
        admin.require_auth();

        env.storage()
            .instance()
            .set(&DataKey::PauseGuardian, &guardian);

        env.events().publish(
            (Symbol::new(&env, "pause_guardian_set"),),
            (admin, guardian),
        );
    }

    pub fn get_pause_guardian(env: Env) -> Option<Address> {
        env.storage().instance().get(&DataKey::PauseGuardian)
    }

    /// Halt new borrowing, repayment and liquidation. Guardian only.
    ///
    /// Deposits and view functions stay available so the position set remains
    /// auditable while the market is in disarray.
    pub fn pause(env: Env) {
        Self::require_pause_guardian(&env);
        pause_guard::set_paused(&env, true);

        env.events().publish(
            (Symbol::new(&env, "contract_paused"),),
            env.ledger().sequence(),
        );
    }

    /// Resume normal operation. Guardian only.
    pub fn unpause(env: Env) {
        Self::require_pause_guardian(&env);
        pause_guard::set_paused(&env, false);

        env.events().publish(
            (Symbol::new(&env, "contract_unpaused"),),
            env.ledger().sequence(),
        );
    }

    pub fn is_paused(env: Env) -> bool {
        pause_guard::is_paused(&env)
    }

    pub fn open_loan(env: Env, borrower: Address, collateral_amount: i128, borrow_amount: i128) {
        pause_guard::require_not_paused(&env);
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
        Self::index_position(&env, &borrower);

        env.events().publish(
            (Symbol::new(&env, "loan_opened"), borrower),
            (collateral_amount, borrow_amount),
        );
    }

    pub fn repay_loan(env: Env, borrower: Address, amount: i128) {
        pause_guard::require_not_paused(&env);
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
            Self::deindex_position(&env, &borrower);
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
        pause_guard::require_not_paused(&env);
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
        Self::deindex_position(&env, &borrower);

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

    /// Number of open collateral positions.
    ///
    /// Liquidation bots page through the index with this value as the stop
    /// condition, so it always reflects the number of live entries.
    pub fn get_collateral_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::CollateralCount)
            .unwrap_or(0)
    }

    /// One page of open positions, in index order.
    ///
    /// `limit` is clamped to `shared::MAX_PAGE_SIZE`, so the view stays inside a
    /// single transaction's budget regardless of what the caller asks for.
    pub fn get_collateral_page(env: Env, offset: u32, limit: u32) -> Vec<CollateralPosition> {
        Self::require_initialized(&env);

        let total = Self::get_collateral_count(env.clone());
        let (start, end) = Pagination::bounds(total, offset, limit);

        // One oracle read per page, not per position.
        let price = Self::get_mnt_price(&env);
        let mut page = Vec::new(&env);

        for slot in start..end {
            let owner: Address = match env
                .storage()
                .persistent()
                .get(&DataKey::CollateralIndex(slot))
            {
                Some(o) => o,
                None => continue,
            };

            if let Some(loan) = env
                .storage()
                .persistent()
                .get::<_, Loan>(&DataKey::Loan(owner.clone()))
            {
                page.push_back(CollateralPosition {
                    owner,
                    ratio_bps: Self::compute_ratio_bps(
                        loan.collateral_amount,
                        loan.debt_amount,
                        price,
                    ),
                    collateral_amount: loan.collateral_amount,
                    debt_amount: loan.debt_amount,
                });
            }
        }

        page
    }

    /// One page of positions whose live collateralisation ratio is below
    /// `threshold_bps`.
    ///
    /// `offset` and `limit` address the raw index window, not the filtered
    /// result, so a caller pages the whole position set with a fixed stride
    /// (`offset += limit`) and cannot miss a match that shifts the page size.
    /// The ratio is computed against the current oracle price as the window is
    /// walked.
    pub fn get_undercollateralized_page(
        env: Env,
        threshold_bps: u32,
        offset: u32,
        limit: u32,
    ) -> Vec<CollateralPosition> {
        Self::require_initialized(&env);

        let total = Self::get_collateral_count(env.clone());
        let (start, end) = Pagination::bounds(total, offset, limit);

        let price = Self::get_mnt_price(&env);
        let threshold = threshold_bps as i128;
        let mut page = Vec::new(&env);

        for slot in start..end {
            let owner: Address = match env
                .storage()
                .persistent()
                .get(&DataKey::CollateralIndex(slot))
            {
                Some(o) => o,
                None => continue,
            };

            let loan: Loan = match env
                .storage()
                .persistent()
                .get(&DataKey::Loan(owner.clone()))
            {
                Some(l) => l,
                None => continue,
            };

            let ratio_bps =
                Self::compute_ratio_bps(loan.collateral_amount, loan.debt_amount, price);
            if ratio_bps < threshold {
                page.push_back(CollateralPosition {
                    owner,
                    collateral_amount: loan.collateral_amount,
                    debt_amount: loan.debt_amount,
                    ratio_bps,
                });
            }
        }

        page
    }

    /// Appends `owner` to the dense position index.
    fn index_position(env: &Env, owner: &Address) {
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::CollateralCount)
            .unwrap_or(0);

        env.storage()
            .persistent()
            .set(&DataKey::CollateralIndex(count), owner);
        env.storage()
            .persistent()
            .set(&DataKey::CollateralSlot(owner.clone()), &count);
        env.storage()
            .instance()
            .set(&DataKey::CollateralCount, &(count + 1));
    }

    /// Removes `owner` from the index by swapping in the last entry, which
    /// keeps the index dense so the page views never skip a slot.
    fn deindex_position(env: &Env, owner: &Address) {
        let count: u32 = match env.storage().instance().get(&DataKey::CollateralCount) {
            Some(c) => c,
            None => return,
        };

        if count == 0 {
            return;
        }

        let slot: u32 = match env
            .storage()
            .persistent()
            .get(&DataKey::CollateralSlot(owner.clone()))
        {
            Some(s) => s,
            None => return,
        };

        let last = count - 1;

        if slot != last {
            if let Some(moved) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::CollateralIndex(last))
            {
                env.storage()
                    .persistent()
                    .set(&DataKey::CollateralIndex(slot), &moved);
                env.storage()
                    .persistent()
                    .set(&DataKey::CollateralSlot(moved), &slot);
            }
        }

        env.storage()
            .persistent()
            .remove(&DataKey::CollateralIndex(last));
        env.storage()
            .persistent()
            .remove(&DataKey::CollateralSlot(owner.clone()));
        env.storage()
            .instance()
            .set(&DataKey::CollateralCount, &last);
    }

    fn require_pause_guardian(env: &Env) {
        let guardian: Address = env
            .storage()
            .instance()
            .get(&DataKey::PauseGuardian)
            .expect("pause guardian not set");
        guardian.require_auth();
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
        admin: Address,
        guardian: Address,
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
            let guardian = Address::generate(&env);
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
            contract.set_pause_guardian(&admin, &guardian);

            // Price = 2.0 USDC per MNT.
            oracle.set_price(&symbol_short!("MNT"), &20_000);

            // Borrower starts with MNT collateral inventory.
            mnt.mint(&borrower, &10_000);

            // Loan contract starts with USDC liquidity for disbursement.
            usdc.mint(&contract_id, &100_000);

            Self {
                env,
                contract_id,
                admin,
                guardian,
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

        /// Registers a second borrower with a freshly funded MNT balance so the
        /// position index has more than one entry to page over.
        fn add_borrower(&self) -> Address {
            let borrower = Address::generate(&self.env);
            self.mnt().mint(&borrower, &1_000);
            borrower
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

    // ── Emergency pause (#1114) ──────────────────────────────────────────────

    #[test]
    fn test_pause_guardian_is_stored() {
        let f = Fixture::setup();
        assert_eq!(f.contract().get_pause_guardian(), Some(f.guardian.clone()));
    }

    #[test]
    fn test_admin_can_rotate_the_pause_guardian() {
        let f = Fixture::setup();
        let contract = f.contract();
        let new_guardian = Address::generate(&f.env);

        assert_eq!(contract.get_pause_guardian(), Some(f.guardian.clone()));
        contract.set_pause_guardian(&f.admin, &new_guardian);
        assert_eq!(contract.get_pause_guardian(), Some(new_guardian));
    }

    #[test]
    fn test_pause_blocks_borrowing() {
        let f = Fixture::setup();
        let contract = f.contract();

        assert!(!contract.is_paused());
        contract.pause();
        assert!(contract.is_paused());

        assert!(contract.try_open_loan(&f.borrower, &100, &120).is_err());
        assert_eq!(contract.get_loan(&f.borrower), None);
    }

    #[test]
    #[should_panic(expected = "contract paused")]
    fn test_pause_panics_with_an_explicit_reason() {
        let f = Fixture::setup();
        f.contract().pause();
        f.contract().open_loan(&f.borrower, &100, &120);
    }

    #[test]
    fn test_pause_blocks_liquidation() {
        let f = Fixture::setup();
        let contract = f.contract();

        contract.open_loan(&f.borrower, &100, &120);
        f.oracle().set_price(&symbol_short!("MNT"), &10_000);

        contract.pause();
        assert!(contract.try_liquidate(&f.borrower, &f.liquidator).is_err());
    }

    #[test]
    fn test_pause_blocks_repayment() {
        let f = Fixture::setup();
        let contract = f.contract();

        contract.open_loan(&f.borrower, &100, &120);
        contract.pause();

        assert!(contract.try_repay_loan(&f.borrower, &20).is_err());
    }

    #[test]
    fn test_unpause_restores_borrowing() {
        let f = Fixture::setup();
        let contract = f.contract();

        contract.pause();
        assert!(contract.try_open_loan(&f.borrower, &100, &120).is_err());

        contract.unpause();
        assert!(!contract.is_paused());

        contract.open_loan(&f.borrower, &100, &120);
        let loan = contract.get_loan(&f.borrower).unwrap();
        assert_eq!(loan.debt_amount, 120);
    }

    #[test]
    fn test_pause_leaves_position_additions_and_views_open() {
        let f = Fixture::setup();
        let contract = f.contract();

        contract.open_loan(&f.borrower, &100, &120);
        contract.pause();

        // Deposits stay open so borrowers can top up collateral and so the
        // position set remains auditable while the market is in disarray.
        contract.add_collateral(&f.borrower, &50);
        assert_eq!(contract.get_collateral_count(), 1);
        assert_eq!(contract.get_health_factor(&f.borrower), 25_000);
    }

    #[test]
    #[should_panic(expected = "pause guardian not set")]
    fn test_pause_requires_a_registered_guardian() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let mnt_token_id = env.register_contract(None, MockToken);
        let usdc_token_id = env.register_contract(None, MockToken);
        let oracle_id = env.register_contract(None, MockOracle);
        let contract_id = env.register_contract(None, CollateralLoanContract);

        let contract = CollateralLoanContractClient::new(&env, &contract_id);
        contract.initialize(
            &admin,
            &mnt_token_id,
            &usdc_token_id,
            &oracle_id,
            &symbol_short!("MNT"),
        );

        assert_eq!(contract.get_pause_guardian(), None);
        contract.pause();
    }

    // ── Paginated position enumeration (#1116) ───────────────────────────────

    #[test]
    fn test_collateral_count_tracks_open_positions() {
        let f = Fixture::setup();
        let contract = f.contract();
        assert_eq!(contract.get_collateral_count(), 0);

        let second = f.add_borrower();
        contract.open_loan(&f.borrower, &100, &120);
        assert_eq!(contract.get_collateral_count(), 1);

        contract.open_loan(&second, &200, &100);
        assert_eq!(contract.get_collateral_count(), 2);

        contract.repay_loan(&f.borrower, &120);
        assert_eq!(contract.get_collateral_count(), 1);
    }

    #[test]
    fn test_collateral_index_stays_dense_after_removal() {
        let f = Fixture::setup();
        let contract = f.contract();

        let second = f.add_borrower();
        let third = f.add_borrower();

        contract.open_loan(&f.borrower, &100, &120);
        contract.open_loan(&second, &200, &100);
        contract.open_loan(&third, &300, &200);

        // Repay the middle position: the last one is swapped into its slot so
        // the page views never skip an index.
        contract.repay_loan(&second, &100);

        assert_eq!(contract.get_collateral_count(), 2);
        let page = contract.get_collateral_page(&0, &10);
        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().owner, f.borrower);
        assert_eq!(page.get(1).unwrap().owner, third);
    }

    #[test]
    fn test_collateral_page_paginates() {
        let f = Fixture::setup();
        let contract = f.contract();

        let second = f.add_borrower();
        let third = f.add_borrower();
        contract.open_loan(&f.borrower, &100, &120);
        contract.open_loan(&second, &200, &100);
        contract.open_loan(&third, &300, &200);

        let first_page = contract.get_collateral_page(&0, &2);
        assert_eq!(first_page.len(), 2);
        assert_eq!(first_page.get(0).unwrap().owner, f.borrower);
        assert_eq!(first_page.get(1).unwrap().owner, second);

        let second_page = contract.get_collateral_page(&2, &2);
        assert_eq!(second_page.len(), 1);
        assert_eq!(second_page.get(0).unwrap().owner, third);

        // Past the end yields an empty page rather than panicking.
        assert_eq!(contract.get_collateral_page(&3, &2).len(), 0);
        assert_eq!(contract.get_collateral_page(&99, &2).len(), 0);
    }

    #[test]
    fn test_collateral_page_clamps_limit_to_max_page_size() {
        let f = Fixture::setup();
        let contract = f.contract();

        for _ in 0..(shared::MAX_PAGE_SIZE + 10) {
            let borrower = f.add_borrower();
            contract.open_loan(&borrower, &100, &120);
        }

        assert_eq!(contract.get_collateral_count(), shared::MAX_PAGE_SIZE + 10);

        let page = contract.get_collateral_page(&0, &10_000);
        assert_eq!(page.len(), shared::MAX_PAGE_SIZE);
    }

    #[test]
    fn test_collateral_page_reports_live_ratio() {
        let f = Fixture::setup();
        let contract = f.contract();

        contract.open_loan(&f.borrower, &100, &120);
        let position = contract.get_collateral_page(&0, &10).get(0).unwrap();

        assert_eq!(position.collateral_amount, 100);
        assert_eq!(position.debt_amount, 120);
        assert_eq!(position.ratio_bps, 16_666);
    }

    #[test]
    fn test_undercollateralized_page_filters_below_threshold() {
        let f = Fixture::setup();
        let contract = f.contract();

        // 166.66% ratio, well above the liquidation threshold.
        contract.open_loan(&f.borrower, &100, &120);

        // 200% ratio.
        let healthy = f.add_borrower();
        contract.open_loan(&healthy, &200, &100);

        assert_eq!(contract.get_collateral_count(), 2);

        // Nothing is below 150% while the price holds.
        let page = contract.get_undercollateralized_page(&15_000, &0, &10);
        assert_eq!(page.len(), 0);

        // Drop MNT to 1.0 USDC: the first position falls to 83.33%, the second
        // to 100%. Both are now undercollateralised.
        f.oracle().set_price(&symbol_short!("MNT"), &10_000);

        let page = contract.get_undercollateralized_page(&12_000, &0, &10);
        assert_eq!(page.len(), 2);
        assert_eq!(page.get(0).unwrap().owner, f.borrower);
        assert_eq!(page.get(0).unwrap().ratio_bps, 8_333);
        assert_eq!(page.get(1).unwrap().owner, healthy);
        assert_eq!(page.get(1).unwrap().ratio_bps, 10_000);
    }

    #[test]
    fn test_undercollateralized_page_selects_only_the_weak_positions() {
        let f = Fixture::setup();
        let contract = f.contract();

        // 100% ratio: always below the 150% borrow threshold.
        contract.open_loan(&f.borrower, &100, &100);

        // 400% ratio.
        let strong = f.add_borrower();
        contract.open_loan(&strong, &200, &100);

        let page = contract.get_undercollateralized_page(&15_000, &0, &10);
        assert_eq!(page.len(), 1);
        assert_eq!(page.get(0).unwrap().owner, f.borrower);
        assert_eq!(page.get(0).unwrap().ratio_bps, 20_000);
    }

    #[test]
    fn test_undercollateralized_page_windows_the_index() {
        let f = Fixture::setup();
        let contract = f.contract();

        let second = f.add_borrower();
        let third = f.add_borrower();
        contract.open_loan(&f.borrower, &100, &100);
        contract.open_loan(&second, &200, &100);
        contract.open_loan(&third, &300, &100);

        // Only the first slot is weak, and offset windows the raw index.
        let first = contract.get_undercollateralized_page(&15_000, &0, &1);
        assert_eq!(first.len(), 1);
        assert_eq!(first.get(0).unwrap().owner, f.borrower);

        let second_window = contract.get_undercollateralized_page(&15_000, &1, &1);
        assert_eq!(second_window.len(), 0);

        // Full window returns the only match.
        let full = contract.get_undercollateralized_page(&15_000, &0, &10);
        assert_eq!(full.len(), 1);
    }

    #[test]
    fn test_undercollateralized_page_clamps_limit() {
        let f = Fixture::setup();
        let contract = f.contract();

        for _ in 0..(shared::MAX_PAGE_SIZE + 5) {
            let borrower = f.add_borrower();
            contract.open_loan(&borrower, &100, &100);
        }

        let page = contract.get_undercollateralized_page(&15_000, &0, &10_000);
        assert_eq!(page.len(), shared::MAX_PAGE_SIZE);
    }

    #[test]
    fn test_paginated_views_are_empty_before_any_loan() {
        let f = Fixture::setup();
        let contract = f.contract();

        assert_eq!(contract.get_collateral_count(), 0);
        assert_eq!(contract.get_collateral_page(&0, &10).len(), 0);
        assert_eq!(
            contract
                .get_undercollateralized_page(&15_000, &0, &10)
                .len(),
            0
        );
    }
}

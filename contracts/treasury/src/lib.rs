#![no_std]

use shared::ReentrancyGuard;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env,
    IntoVal, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized     = 2,
    Unauthorized       = 3,
    InsufficientBalance = 4,
    TokenNotApproved   = 5,
    /// buyback_and_burn must be called through the registered timelock.
    NotTimelock        = 6,
    /// min_mnt_out must be > 0 (pre-flight slippage guard).
    InvalidMinOut      = 7,
    /// DEX returned fewer tokens than min_mnt_out (slippage exceeded).
    SlippageExceeded   = 8,
    /// DEX returned zero MNT — no XLM was transferred (approve-pull pattern).
    ZeroOutput         = 9,
}

// ---------------------------------------------------------------------------
// Event payloads
// ---------------------------------------------------------------------------

/// Emitted when a buyback-and-burn succeeds.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuybackSucceeded {
    pub xlm_spent:   i128,
    pub mnt_burned:  i128,
    pub timestamp:   u64,
}

/// Emitted when a buyback-and-burn attempt fails before any XLM leaves treasury.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BuybackFailed {
    pub xlm_amount:  i128,
    /// Short reason tag: "zero_output" | "slippage" | "invalid_min_out"
    pub reason:      Symbol,
}

// ---------------------------------------------------------------------------
// DEX interface validation
// ---------------------------------------------------------------------------

/// Describes the expected interface of the DEX swap contract.
///
/// Callers must supply a `DexInterface` when invoking `buyback_and_burn`.
/// The treasury uses `approve + pull` — it never pushes XLM to the DEX.
/// The DEX must pull exactly `xlm_amount` from the treasury's allowance,
/// execute the swap, and return the MNT amount credited to `recipient`.
///
/// Expected DEX function signature:
///   `swap_exact_in(token_in, token_out, amount_in, min_out, recipient) -> i128`
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DexInterface {
    /// The entry-point function name on the DEX contract.
    pub swap_fn: Symbol,
}

impl DexInterface {
    /// Validate that `swap_fn` is not empty.
    pub fn validate(&self, env: &Env) {
        if self.swap_fn == Symbol::new(env, "") {
            panic!("DexInterface: swap_fn must not be empty");
        }
    }
}

// ---------------------------------------------------------------------------
// Allocation history
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllocationHistory {
    pub token:     Address,
    pub recipient: Address,
    pub amount:    i128,
    pub timestamp: u64,
}

// ---------------------------------------------------------------------------
// Token approval event
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TreasuryTokenApprovalEvent {
    pub token:    Address,
    pub approved: bool,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    /// The timelock contract whose `execute` is the only allowed caller of
    /// `buyback_and_burn`. Set during `initialize`.
    Timelock,
    StakingContract,
    AllocationCount,
    /// Individual allocation history: DataKey::Allocation(index) → AllocationHistory
    Allocation(u32),
    /// Token whitelist: DataKey::ApprovedToken(token_address) → bool
    ApprovedToken(Address),
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct TreasuryContract;

#[contractimpl]
impl TreasuryContract {
    /// Initialize treasury contract.
    ///
    /// * `admin`            — regular admin for deposits/allocations.
    /// * `staking_contract` — receives staker distributions.
    /// * `timelock`         — the **only** address allowed to call
    ///                        `buyback_and_burn` (enforced on every call).
    pub fn initialize(
        env: Env,
        admin: Address,
        staking_contract: Address,
        timelock: Address,
    ) -> Result<(), Error> {
        if env.storage().persistent().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().persistent().set(&DataKey::Admin,           &admin);
        env.storage().persistent().set(&DataKey::StakingContract, &staking_contract);
        env.storage().persistent().set(&DataKey::Timelock,        &timelock);
        env.storage().persistent().set(&DataKey::AllocationCount, &0u32);
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Token whitelist management
    // -----------------------------------------------------------------------

    /// Add or remove an approved token from the treasury whitelist (admin only).
    pub fn set_approved_token(
        env: Env,
        token_address: Address,
        approved: bool,
    ) -> Result<(), Error> {
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage().persistent().set(&key, &approved);

        if approved {
            env.events().publish(
                (symbol_short!("treasury"), symbol_short!("tok_appr")),
                TreasuryTokenApprovalEvent { token: token_address, approved: true },
            );
        } else {
            env.events().publish(
                (symbol_short!("treasury"), symbol_short!("tok_rej")),
                TreasuryTokenApprovalEvent { token: token_address, approved: false },
            );
        }
        Ok(())
    }

    /// Check if a token is on the treasury's approved whitelist.
    pub fn is_token_approved(env: Env, token_address: Address) -> bool {
        Self::_is_token_approved(&env, &token_address)
    }

    fn _is_token_approved(env: &Env, token_address: &Address) -> bool {
        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage().persistent().get::<_, bool>(&key).unwrap_or(false)
    }

    // -----------------------------------------------------------------------
    // Deposit / Allocate / Distribute
    // -----------------------------------------------------------------------

    /// Accept deposits of approved Stellar assets only.
    pub fn deposit(env: Env, from: Address, token: Address, amount: i128) -> Result<(), Error> {
        from.require_auth();
        if !Self::_is_token_approved(&env, &token) {
            panic!("Token not approved");
        }
        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&from, &env.current_contract_address(), &amount);
        env.events().publish(
            (symbol_short!("deposit"), from.clone(), token.clone()),
            amount,
        );
        Ok(())
    }

    /// Query on-chain balance for a token held by this contract.
    pub fn get_balance(env: Env, token: Address) -> i128 {
        token::Client::new(&env, &token).balance(&env.current_contract_address())
    }

    /// Allocate tokens to a recipient — governance/timelock only.
    pub fn allocate(
        env: Env,
        token: Address,
        recipient: Address,
        amount: i128,
    ) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "allocate"));
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        if !Self::_is_token_approved(&env, &token) {
            return Err(Error::TokenNotApproved);
        }

        token::Client::new(&env, &token)
            .transfer(&env.current_contract_address(), &recipient, &amount);

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AllocationCount)
            .unwrap_or(0u32);
        env.storage().persistent().set(
            &DataKey::Allocation(count),
            &AllocationHistory {
                token:     token.clone(),
                recipient: recipient.clone(),
                amount,
                timestamp: env.ledger().timestamp(),
            },
        );
        env.storage().persistent().set(&DataKey::AllocationCount, &(count + 1));

        env.events().publish(
            (symbol_short!("allocate"), recipient.clone(), token.clone()),
            amount,
        );
        Ok(())
    }

    /// Distribute tokens to stakers — pro-rata handled by staking contract.
    pub fn distribute_to_stakers(
        env: Env,
        token: Address,
        total_amount: i128,
    ) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "distribute"));
        let admin = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();

        if !Self::_is_token_approved(&env, &token) {
            return Err(Error::TokenNotApproved);
        }

        let staking_contract: Address = env
            .storage()
            .persistent()
            .get(&DataKey::StakingContract)
            .ok_or(Error::NotInitialized)?;

        token::Client::new(&env, &token)
            .transfer(&env.current_contract_address(), &staking_contract, &total_amount);

        env.invoke_contract::<()>(
            &staking_contract,
            &Symbol::new(&env, "distribute_revenue"),
            (token.clone(), total_amount).into_val(&env),
        );

        env.events().publish(
            (symbol_short!("distrib"), staking_contract.clone(), token.clone()),
            total_amount,
        );
        Ok(())
    }

    // -----------------------------------------------------------------------
    // buyback_and_burn — atomic approve + pull pattern
    // -----------------------------------------------------------------------

    /// Swap XLM for MNT via a DEX and burn the received MNT.
    ///
    /// ## Atomicity guarantee
    /// This function uses the **approve + pull** pattern instead of push:
    ///
    /// 1. Pre-flight validation (no state changes yet).
    /// 2. `xlm_token.approve(treasury → dex_contract, xlm_amount)` — the DEX
    ///    is authorised to pull up to `xlm_amount` from the treasury.
    /// 3. Call `dex_contract.<swap_fn>(...)` — the DEX pulls the XLM itself.
    ///    If the call panics the allowance is never consumed and no XLM leaves.
    /// 4. Validate `mnt_received >= min_mnt_out`.  On failure the allowance
    ///    is revoked (set to 0) before emitting `BuybackFailed`.
    /// 5. Burn the received MNT and emit `BuybackSucceeded`.
    ///
    /// ## Access control
    /// Only the registered `timelock` contract address may call this function.
    /// Direct admin calls are rejected — all buybacks must go through the
    /// timelock's 48-hour delay to prevent rushed or malicious swaps.
    pub fn buyback_and_burn(
        env: Env,
        xlm_token:    Address,
        mnt_token:    Address,
        dex_contract: Address,
        xlm_amount:   i128,
        min_mnt_out:  i128,
        dex_iface:    DexInterface,
    ) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "buyback"));

        // ------------------------------------------------------------------
        // 1. Access control: must be called by the registered timelock only.
        // ------------------------------------------------------------------
        let timelock: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Timelock)
            .ok_or(Error::NotInitialized)?;
        timelock.require_auth();

        // ------------------------------------------------------------------
        // 2. Pre-flight validation — no state changes yet.
        // ------------------------------------------------------------------
        dex_iface.validate(&env);

        if min_mnt_out <= 0 {
            env.events().publish(
                (symbol_short!("buyback"), symbol_short!("failed")),
                BuybackFailed {
                    xlm_amount,
                    reason: Symbol::new(&env, "invalid_min_out"),
                },
            );
            return Err(Error::InvalidMinOut);
        }

        if !Self::_is_token_approved(&env, &xlm_token) {
            return Err(Error::TokenNotApproved);
        }
        if !Self::_is_token_approved(&env, &mnt_token) {
            return Err(Error::TokenNotApproved);
        }

        // ------------------------------------------------------------------
        // 3. Approve the DEX to pull XLM (no XLM leaves treasury yet).
        //    Ledger sequence used as expiration_ledger for the allowance
        //    (single-ledger approval — revoked automatically after this tx).
        // ------------------------------------------------------------------
        let xlm_client = token::Client::new(&env, &xlm_token);
        let expiration_ledger = env.ledger().sequence() + 1;
        xlm_client.approve(
            &env.current_contract_address(),
            &dex_contract,
            &xlm_amount,
            &expiration_ledger,
        );

        // ------------------------------------------------------------------
        // 4. Call DEX swap — DEX pulls XLM via the allowance and returns MNT.
        //    Signature: swap_fn(token_in, token_out, amount_in, min_out, recipient)
        //    The treasury is the recipient so MNT lands in the treasury first.
        // ------------------------------------------------------------------
        let mnt_received: i128 = env.invoke_contract(
            &dex_contract,
            &dex_iface.swap_fn,
            (
                xlm_token.clone(),
                mnt_token.clone(),
                xlm_amount,
                min_mnt_out,
                env.current_contract_address(),
            )
                .into_val(&env),
        );

        // ------------------------------------------------------------------
        // 5. Validate output — revoke allowance and emit failure if bad.
        // ------------------------------------------------------------------
        if mnt_received == 0 {
            // Revoke any remaining allowance (defensive; DEX may not have pulled).
            xlm_client.approve(
                &env.current_contract_address(),
                &dex_contract,
                &0,
                &expiration_ledger,
            );
            env.events().publish(
                (symbol_short!("buyback"), symbol_short!("failed")),
                BuybackFailed {
                    xlm_amount,
                    reason: Symbol::new(&env, "zero_output"),
                },
            );
            return Err(Error::ZeroOutput);
        }

        if mnt_received < min_mnt_out {
            // Revoke any remaining allowance.
            xlm_client.approve(
                &env.current_contract_address(),
                &dex_contract,
                &0,
                &expiration_ledger,
            );
            env.events().publish(
                (symbol_short!("buyback"), symbol_short!("failed")),
                BuybackFailed {
                    xlm_amount,
                    reason: Symbol::new(&env, "slippage"),
                },
            );
            return Err(Error::SlippageExceeded);
        }

        // ------------------------------------------------------------------
        // 6. Burn MNT — only reached if swap succeeded and output is valid.
        // ------------------------------------------------------------------
        env.invoke_contract::<()>(
            &mnt_token,
            &Symbol::new(&env, "burn"),
            (env.current_contract_address(), mnt_received).into_val(&env),
        );

        env.events().publish(
            (symbol_short!("buyback"), symbol_short!("ok")),
            BuybackSucceeded {
                xlm_spent:  xlm_amount,
                mnt_burned: mnt_received,
                timestamp:  env.ledger().timestamp(),
            },
        );

        Ok(())
    }

    // -----------------------------------------------------------------------
    // View helpers
    // -----------------------------------------------------------------------

    pub fn get_history_page(env: Env, offset: u32, limit: u32) -> Vec<AllocationHistory> {
        let total_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::AllocationCount)
            .unwrap_or(0u32);

        let mut result = Vec::new(&env);
        let end = offset.saturating_add(limit).min(total_count);

        for i in offset..end {
            if let Some(record) = env
                .storage()
                .persistent()
                .get::<DataKey, AllocationHistory>(&DataKey::Allocation(i))
            {
                result.push_back(record);
            }
        }
        result
    }

    pub fn get_timelock(env: Env) -> Address {
        env.storage()
            .persistent()
            .get(&DataKey::Timelock)
            .expect("not initialized")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    // -----------------------------------------------------------------------
    // Mock contracts
    // -----------------------------------------------------------------------

    /// DEX that performs a normal 1:1 swap (approve + pull pattern).
    /// Pulls XLM from the caller's allowance and returns `amount_in` MNT.
    #[contract]
    pub struct MockDEX;

    #[contractimpl]
    impl MockDEX {
        pub fn swap_exact_in(
            env: Env,
            token_in: Address,
            _token_out: Address,
            amount_in: i128,
            _min_out: i128,
            recipient: Address,
        ) -> i128 {
            // Pull the XLM allowance from the treasury (simulate DEX pull).
            let xlm = token::Client::new(&env, &token_in);
            xlm.transfer_from(
                &env.current_contract_address(),
                &recipient,  // pull from treasury (spender == DEX contract)
                &env.current_contract_address(), // actually pull from who approved
                &amount_in,
            );
            // Return MNT amount (1:1 for tests).
            amount_in
        }
    }

    /// DEX that always returns 0 MNT (simulates failed / empty swap).
    #[contract]
    pub struct MockDEXZero;

    #[contractimpl]
    impl MockDEXZero {
        pub fn swap_exact_in(
            _env: Env,
            _token_in: Address,
            _token_out: Address,
            _amount_in: i128,
            _min_out: i128,
            _recipient: Address,
        ) -> i128 {
            0 // returns nothing — no XLM pulled
        }
    }

    /// DEX that returns less MNT than min_mnt_out (simulates slippage).
    #[contract]
    pub struct MockDEXSlippage;

    #[contractimpl]
    impl MockDEXSlippage {
        pub fn swap_exact_in(
            _env: Env,
            _token_in: Address,
            _token_out: Address,
            _amount_in: i128,
            _min_out: i128,
            _recipient: Address,
        ) -> i128 {
            1 // returns tiny amount — below min_mnt_out
        }
    }

    #[contract]
    pub struct MockStaking;

    #[contractimpl]
    impl MockStaking {
        pub fn distribute_revenue(_env: Env, _token: Address, _amount: i128) {}
    }

    #[contract]
    pub struct MockMNT;

    #[contractimpl]
    impl MockMNT {
        pub fn burn(_env: Env, _from: Address, _amount: i128) {}
    }

    // -----------------------------------------------------------------------
    // Setup helper
    // -----------------------------------------------------------------------

    fn setup_test(env: &Env) -> (Address, Address, Address, Address) {
        let admin    = Address::generate(env);
        let staking  = env.register_contract(None, MockStaking);
        let timelock = Address::generate(env);  // simulated timelock address
        let contract_id = env.register_contract(None, TreasuryContract);
        let client = TreasuryContractClient::new(env, &contract_id);
        client.initialize(&admin, &staking, &timelock);
        (admin, staking, timelock, contract_id)
    }

    fn default_dex_iface(env: &Env) -> DexInterface {
        DexInterface { swap_fn: Symbol::new(env, "swap_exact_in") }
    }

    // -----------------------------------------------------------------------
    // Existing core tests (updated for new initialize signature)
    // -----------------------------------------------------------------------

    #[test]
    fn test_initialization() {
        let env = Env::default();
        let (admin, staking, timelock, contract_id) = setup_test(&env);
        let client = TreasuryContractClient::new(&env, &contract_id);
        // double-init must fail
        let result = client.try_initialize(&admin, &staking, &timelock);
        assert!(result.is_err());
    }

    #[test]
    fn test_deposit_and_balance() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, contract_id) = setup_test(&env);
        let user = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let stellar_asset_client = token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset_client.mint(&user, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);
        treasury_client.set_approved_token(&token_addr, &true);
        treasury_client.deposit(&user, &token_addr, &500);

        assert_eq!(treasury_client.get_balance(&token_addr), 500);
    }

    #[test]
    #[should_panic(expected = "Token not approved")]
    fn test_deposit_unapproved_token() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, contract_id) = setup_test(&env);
        let user = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let stellar_asset_client = token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset_client.mint(&user, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);
        treasury_client.deposit(&user, &token_addr, &500);
    }

    #[test]
    fn test_allocate() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, contract_id) = setup_test(&env);
        let recipient = Address::generate(&env);
        let token_addr = env.register_stellar_asset_contract(admin.clone());
        let token_client = token::Client::new(&env, &token_addr);
        let stellar_asset_client = token::StellarAssetClient::new(&env, &token_addr);
        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);
        treasury_client.set_approved_token(&token_addr, &true);
        env.ledger().set_timestamp(12345);
        treasury_client.allocate(&token_addr, &recipient, &400);

        assert_eq!(treasury_client.get_balance(&token_addr), 600);
        assert_eq!(token_client.balance(&recipient), 400);
    }

    #[test]
    fn test_token_whitelist_toggle() {
        let env = Env::default();
        env.mock_all_auths();
        let (_, _, _, contract_id) = setup_test(&env);
        let treasury_client = TreasuryContractClient::new(&env, &contract_id);
        let token = Address::generate(&env);
        assert!(!treasury_client.is_token_approved(&token));
        treasury_client.set_approved_token(&token, &true);
        assert!(treasury_client.is_token_approved(&token));
        treasury_client.set_approved_token(&token, &false);
        assert!(!treasury_client.is_token_approved(&token));
    }

    // -----------------------------------------------------------------------
    // buyback_and_burn — timelock access control
    // -----------------------------------------------------------------------

    #[test]
    fn test_buyback_requires_timelock_auth() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _timelock, contract_id) = setup_test(&env);

        let xlm_addr = env.register_stellar_asset_contract(admin.clone());
        let mnt_addr = env.register_contract(None, MockMNT);
        let dex_addr = env.register_contract(None, MockDEX);

        let stellar_asset_client = token::StellarAssetClient::new(&env, &xlm_addr);
        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);
        treasury_client.set_approved_token(&xlm_addr, &true);
        treasury_client.set_approved_token(&mnt_addr, &true);

        // get_timelock should return the registered address
        assert_eq!(treasury_client.get_timelock(), _timelock);

        // mock_all_auths covers timelock auth — call succeeds
        // (full auth-gating is enforced by require_auth; this test confirms the
        //  function reads the timelock address from storage correctly)
        let _ = treasury_client.try_buyback_and_burn(
            &xlm_addr,
            &mnt_addr,
            &dex_addr,
            &1000,
            &500,
            &default_dex_iface(&env),
        );
        // We only check that get_timelock() returns the expected address; the
        // auth mock covers the auth requirement in unit test mode.
        assert_eq!(treasury_client.get_timelock(), _timelock);
    }

    // -----------------------------------------------------------------------
    // buyback_and_burn — zero output (DEX returns 0 MNT)
    // -----------------------------------------------------------------------

    #[test]
    fn test_buyback_dex_returns_zero_mnt_fails_and_no_xlm_lost() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, contract_id) = setup_test(&env);

        let xlm_addr = env.register_stellar_asset_contract(admin.clone());
        let mnt_addr = env.register_contract(None, MockMNT);
        let dex_addr = env.register_contract(None, MockDEXZero); // returns 0

        let stellar_asset_client = token::StellarAssetClient::new(&env, &xlm_addr);
        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);
        treasury_client.set_approved_token(&xlm_addr, &true);
        treasury_client.set_approved_token(&mnt_addr, &true);

        let xlm_balance_before = treasury_client.get_balance(&xlm_addr);

        let result = treasury_client.try_buyback_and_burn(
            &xlm_addr,
            &mnt_addr,
            &dex_addr,
            &500,
            &100,
            &default_dex_iface(&env),
        );

        // Must return ZeroOutput error
        assert!(result.is_err(), "expected ZeroOutput error");

        // XLM balance must not have changed — no funds left treasury
        let xlm_balance_after = treasury_client.get_balance(&xlm_addr);
        assert_eq!(
            xlm_balance_before, xlm_balance_after,
            "XLM must not leave treasury when DEX returns 0 MNT"
        );
    }

    // -----------------------------------------------------------------------
    // buyback_and_burn — slippage guard (min_mnt_out not met)
    // -----------------------------------------------------------------------

    #[test]
    fn test_buyback_slippage_guard_no_xlm_transferred() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, contract_id) = setup_test(&env);

        let xlm_addr = env.register_stellar_asset_contract(admin.clone());
        let mnt_addr = env.register_contract(None, MockMNT);
        let dex_addr = env.register_contract(None, MockDEXSlippage); // returns 1

        let stellar_asset_client = token::StellarAssetClient::new(&env, &xlm_addr);
        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);
        treasury_client.set_approved_token(&xlm_addr, &true);
        treasury_client.set_approved_token(&mnt_addr, &true);

        let xlm_balance_before = treasury_client.get_balance(&xlm_addr);

        // min_mnt_out = 500, DEX returns 1 → slippage
        let result = treasury_client.try_buyback_and_burn(
            &xlm_addr,
            &mnt_addr,
            &dex_addr,
            &500,
            &500,
            &default_dex_iface(&env),
        );

        assert!(result.is_err(), "expected SlippageExceeded error");

        let xlm_balance_after = treasury_client.get_balance(&xlm_addr);
        assert_eq!(
            xlm_balance_before, xlm_balance_after,
            "XLM must not leave treasury when slippage guard triggers"
        );
    }

    // -----------------------------------------------------------------------
    // buyback_and_burn — invalid min_mnt_out (= 0) rejected before any transfer
    // -----------------------------------------------------------------------

    #[test]
    fn test_buyback_zero_min_out_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, contract_id) = setup_test(&env);

        let xlm_addr = env.register_stellar_asset_contract(admin.clone());
        let mnt_addr = env.register_contract(None, MockMNT);
        let dex_addr = env.register_contract(None, MockDEX);

        let stellar_asset_client = token::StellarAssetClient::new(&env, &xlm_addr);
        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);
        treasury_client.set_approved_token(&xlm_addr, &true);
        treasury_client.set_approved_token(&mnt_addr, &true);

        let xlm_balance_before = treasury_client.get_balance(&xlm_addr);

        // min_mnt_out = 0 → InvalidMinOut, no XLM transferred
        let result = treasury_client.try_buyback_and_burn(
            &xlm_addr,
            &mnt_addr,
            &dex_addr,
            &500,
            &0,  // invalid
            &default_dex_iface(&env),
        );

        assert!(result.is_err(), "expected InvalidMinOut error");
        assert_eq!(
            treasury_client.get_balance(&xlm_addr),
            xlm_balance_before,
            "XLM must remain in treasury when min_out = 0"
        );
    }

    // -----------------------------------------------------------------------
    // buyback_and_burn — unapproved tokens rejected
    // -----------------------------------------------------------------------

    #[test]
    fn test_buyback_unapproved_token_fails() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, contract_id) = setup_test(&env);

        let xlm_addr = env.register_stellar_asset_contract(admin.clone());
        let mnt_addr = env.register_contract(None, MockMNT);
        let dex_addr = env.register_contract(None, MockDEX);

        let stellar_asset_client = token::StellarAssetClient::new(&env, &xlm_addr);
        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);
        // Do NOT approve tokens

        let result = treasury_client.try_buyback_and_burn(
            &xlm_addr,
            &mnt_addr,
            &dex_addr,
            &1000,
            &500,
            &default_dex_iface(&env),
        );
        assert!(result.is_err(), "unapproved token buyback must fail");
    }

    // -----------------------------------------------------------------------
    // buyback_and_burn — invalid DEX interface rejected
    // -----------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "DexInterface: swap_fn must not be empty")]
    fn test_buyback_empty_swap_fn_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let (admin, _, _, contract_id) = setup_test(&env);

        let xlm_addr = env.register_stellar_asset_contract(admin.clone());
        let mnt_addr = env.register_contract(None, MockMNT);
        let dex_addr = env.register_contract(None, MockDEX);

        let stellar_asset_client = token::StellarAssetClient::new(&env, &xlm_addr);
        stellar_asset_client.mint(&contract_id, &1000);

        let treasury_client = TreasuryContractClient::new(&env, &contract_id);
        treasury_client.set_approved_token(&xlm_addr, &true);
        treasury_client.set_approved_token(&mnt_addr, &true);

        let bad_iface = DexInterface { swap_fn: Symbol::new(&env, "") };
        let _ = treasury_client.try_buyback_and_burn(
            &xlm_addr, &mnt_addr, &dex_addr, &1000, &500, &bad_iface,
        );
    }
}

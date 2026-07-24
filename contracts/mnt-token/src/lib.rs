#![no_std]

use soroban_sdk::token::TokenInterface;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, String, Symbol,
};
use soroban_token_sdk::metadata::TokenMetadata;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InsufficientBalance = 3,
    InsufficientAllowance = 4,
    Unauthorized = 5,
    SupplyCapExceeded = 6,
}

// ---------------------------------------------------------------------------
// Storage keys
//
// Storage layout — MNTToken
// ─────────────────────────────────────────────────────────────────────────
// All keys use `persistent()` storage so they survive ledger archival.
//
// Singleton keys:
//   DataKey::Admin              → Address          (set once at initialize)
//   DataKey::TotalSupply        → i128             (updated on mint/burn)
//   DataKey::Metadata           → TokenMetadata    (name, symbol, decimals)
//
// Per-account keys:
//   DataKey::Balance(Address)   → i128             (token balance)
//   DataKey::Allowance(Address, Address) → i128    (owner → spender allowance)
//
// No two keys share the same discriminant.  Because each contract has its
// own isolated storage namespace, there is no collision risk with the
// escrow or verification contracts even if they use the same variant names.
// ─────────────────────────────────────────────────────────────────────────
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MintEventData {
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BurnEventData {
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveEventData {
    pub spender: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferEventData {
    pub to: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Allowance(Address, Address), // (owner, spender)
    Balance(Address),
    TotalSupply,
    Metadata,
    PauseGuardian,
}

const SUPPLY_CAP: i128 = 100_000_000 * 10_000_000; // 100M with 7 decimals

#[contract]
pub struct MNTToken;

#[contractimpl]
impl MNTToken {
    /// Initialize the token contract with an admin.
    ///
    /// Auth: No authorization required for initialization.
    /// Can only be called once.
    ///
    /// Panics if:
    /// - Contract is already initialized
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);

        let metadata = TokenMetadata {
            decimal: 7,
            name: String::from_str(&env, "MentorMinds Token"),
            symbol: String::from_str(&env, "MNT"),
        };
        env.storage()
            .persistent()
            .set(&DataKey::Metadata, &metadata);
        env.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &0i128);
    }

    /// Mint new tokens (admin only).
    ///
    /// Auth: Only the admin can mint tokens.
    /// The admin address is retrieved from persistent storage.
    ///
    /// Panics if:
    /// - Contract is not initialized
    /// - Caller is not the admin
    /// - Caller fails authorization check
    /// - Amount is not positive
    /// - Minting would exceed supply cap
    pub fn mint(env: Env, to: Address, amount: i128) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        Self::assert_not_paused(&env);

        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let total_supply: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);
        let new_total_supply = total_supply.checked_add(amount).expect("Overflow");

        if new_total_supply > SUPPLY_CAP {
            panic!("Supply cap exceeded");
        }

        let balance = Self::balance(env.clone(), to.clone());
        env.storage()
            .persistent()
            .set(&DataKey::Balance(to.clone()), &(balance + amount));
        env.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &new_total_supply);

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Mint"),
                to.clone(),
            ),
            MintEventData { amount },
        );
    }

    /// Burn tokens from an account.
    ///
    /// Auth: Only the token holder can burn their own tokens.
    /// The 'from' address must provide valid authorization.
    ///
    /// Panics if:
    /// - Caller is not the 'from' address
    /// - Caller fails authorization check
    /// - Amount is not positive
    /// - Insufficient balance
    pub fn do_burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        Self::assert_not_paused(&env);

        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let balance = Self::balance(env.clone(), from.clone());
        if balance < amount {
            panic!("Insufficient balance");
        }

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Burn"),
                from.clone(),
            ),
            BurnEventData { amount },
        );
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }

    /// Set the pause guardian contract address. Admin only.
    pub fn set_pause_guardian(env: Env, guardian: Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        env.storage().persistent().set(&DataKey::PauseGuardian, &guardian);
    }

    /// Panics if the pause guardian reports the system is paused.
    fn assert_not_paused(env: &Env) {
        if let Some(guardian) = env
            .storage()
            .persistent()
            .get::<_, Address>(&DataKey::PauseGuardian)
        {
            let is_paused: bool = env.invoke_contract(
                &guardian,
                &soroban_sdk::Symbol::new(env, "is_paused"),
                soroban_sdk::Vec::new(env),
            );
            if is_paused {
                panic!("Contract is paused");
            }
        }
    }
}

#[contractimpl]
impl TokenInterface for MNTToken {
    fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Allowance(from, spender))
            .unwrap_or(0)
    }

    /// Approve a spender to use tokens on behalf of the owner.
    ///
    /// Auth: Only the token owner can approve spenders.
    /// The 'from' address must provide valid authorization.
    ///
    /// Panics if:
    /// - Caller is not the 'from' address
    /// - Caller fails authorization check
    /// - Amount is negative
    fn approve(env: Env, from: Address, spender: Address, amount: i128, _expiration_ledger: u32) {
        from.require_auth();
        if amount < 0 {
            panic!("Amount must be non-negative");
        }
        env.storage()
            .persistent()
            .set(&DataKey::Allowance(from.clone(), spender.clone()), &amount);

        // Note: Simple implementation, expiration_ledger is usually used for TTL in Soroban
        // but for simplicity in this MVP we just store the amount.

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Approve"),
                from.clone(),
            ),
            ApproveEventData { spender, amount },
        );
    }

    fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(id))
            .unwrap_or(0)
    }

    /// Transfer tokens from one account to another.
    ///
    /// Auth: Only the token owner can transfer their tokens.
    /// The 'from' address must provide valid authorization.
    ///
    /// Panics if:
    /// - Caller is not the 'from' address
    /// - Caller fails authorization check
    /// - Amount is not positive
    /// - Insufficient balance
    fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        Self::assert_not_paused(&env);
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let from_balance = Self::balance(env.clone(), from.clone());
        if from_balance < amount {
            panic!("Insufficient balance");
        }

        let to_balance = Self::balance(env.clone(), to.clone());

        env.storage()
            .persistent()
            .set(&DataKey::Balance(from.clone()), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&DataKey::Balance(to.clone()), &(to_balance + amount));

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Transfer"),
                from.clone(),
            ),
            TransferEventData { to, amount },
        );
    }

    /// Transfer tokens using an allowance.
    ///
    /// Auth: Only the approved spender can transfer tokens on behalf of the owner.
    /// The spender address must provide valid authorization.
    ///
    /// Panics if:
    /// - Caller is not the spender
    /// - Caller fails authorization check
    /// - Amount is not positive
    /// - Insufficient allowance
    /// - Insufficient balance
    fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        Self::assert_not_paused(&env);
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let allowance = Self::allowance(env.clone(), from.clone(), spender.clone());
        if allowance < amount {
            panic!("Insufficient allowance");
        }

        let from_balance = Self::balance(env.clone(), from.clone());
        if from_balance < amount {
            panic!("Insufficient balance");
        }

        let to_balance = Self::balance(env.clone(), to.clone());

        env.storage().persistent().set(
            &DataKey::Allowance(from.clone(), spender.clone()),
            &(allowance - amount),
        );
        env.storage()
            .persistent()
            .set(&DataKey::Balance(from.clone()), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&DataKey::Balance(to.clone()), &(to_balance + amount));

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Transfer"),
                from.clone(),
            ),
            TransferEventData {
                to: to.clone(),
                amount,
            },
        );
        env.events()
            .publish((symbol_short!("transfer"), from, to), amount);
    }

    fn burn(env: Env, from: Address, amount: i128) {
        Self::do_burn(env, from, amount)
    }

    fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        Self::assert_not_paused(&env);
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let allowance = Self::allowance(env.clone(), from.clone(), spender.clone());
        if allowance < amount {
            panic!("Insufficient allowance");
        }

        let from_balance = Self::balance(env.clone(), from.clone());
        if from_balance < amount {
            panic!("Insufficient balance");
        }

        let total_supply: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Burn"),
                from.clone(),
            ),
            BurnEventData { amount },
        );
        env.storage()
            .persistent()
            .set(&DataKey::Balance(from.clone()), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &(total_supply - amount));

        env.events().publish((symbol_short!("burn"), from), amount);
    }

    fn decimals(_env: Env) -> u32 {
        7
    }

    fn name(env: Env) -> String {
        let metadata: TokenMetadata = env
            .storage()
            .persistent()
            .get(&DataKey::Metadata)
            .expect("Not initialized");
        metadata.name
    }

    fn symbol(env: Env) -> String {
        let metadata: TokenMetadata = env
            .storage()
            .persistent()
            .get(&DataKey::Metadata)
            .expect("Not initialized");
        metadata.symbol
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events, MockAuth, MockAuthInvoke};
    use soroban_sdk::{vec, Env, IntoVal, Symbol, TryFromVal};

    #[test]
    fn test_initialization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register_contract(None, MNTToken);
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);

        assert_eq!(client.name(), String::from_str(&env, "MentorMinds Token"));
        assert_eq!(client.symbol(), String::from_str(&env, "MNT"));
        assert_eq!(client.decimals(), 7);
    }

    #[test]
    fn test_mint_and_burn() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let contract_id = env.register_contract(None, MNTToken);
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);

        client.mint(&user, &1000);
        assert_eq!(client.balance(&user), 1000);

        let events = env.events().all();
        let last_event = events.last().unwrap();
        assert_eq!(last_event.0, contract_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Mint"),
                user.clone()
            )
                .into_val(&env)
        );
        let mint_data = MintEventData::try_from_val(&env, &last_event.2).unwrap();
        assert_eq!(mint_data.amount, 1000);

        client.burn(&user, &400);
        assert_eq!(client.balance(&user), 600);

        let events = env.events().all();
        let last_event = events.last().unwrap();

        assert_eq!(last_event.0, contract_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Burn"),
                user.clone()
            )
                .into_val(&env)
        );
        let burn_data = BurnEventData::try_from_val(&env, &last_event.2).unwrap();
        assert_eq!(burn_data.amount, 400);
    }

    #[test]
    fn test_transfer_flow() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        let contract_id = env.register_contract(None, MNTToken);
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);
        client.mint(&user1, &1000);

        client.transfer(&user1, &user2, &300);
        assert_eq!(client.balance(&user1), 700);
        assert_eq!(client.balance(&user2), 300);

        let events = env.events().all();
        let last_event = events.last().unwrap();

        assert_eq!(last_event.0, contract_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Transfer"),
                user1.clone()
            )
                .into_val(&env)
        );
        let transfer_data = TransferEventData::try_from_val(&env, &last_event.2).unwrap();
        assert_eq!(transfer_data.to, user2.clone());
        assert_eq!(transfer_data.amount, 300);
    }

    #[test]
    fn test_allowance_flow() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        let contract_id = env.register_contract(None, MNTToken);
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);
        client.mint(&user1, &1000);

        client.approve(&user1, &user2, &500, &100);
        assert_eq!(client.allowance(&user1, &user2), 500);

        let events = env.events().all();
        let mut last_event = events.last().unwrap();

        assert_eq!(last_event.0, contract_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Approve"),
                user1.clone()
            )
                .into_val(&env)
        );
        let approve_data = ApproveEventData::try_from_val(&env, &last_event.2).unwrap();
        assert_eq!(approve_data.spender, user2.clone());
        assert_eq!(approve_data.amount, 500);

        client.transfer_from(&user2, &user1, &user2, &200);
        assert_eq!(client.balance(&user1), 800);
        assert_eq!(client.balance(&user2), 200);
        assert_eq!(client.allowance(&user1, &user2), 300);

        let events2 = env.events().all();
        last_event = events2.last().unwrap();

        assert_eq!(last_event.0, contract_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Transfer"),
                user1.clone()
            )
                .into_val(&env)
        );
        let transfer_from_data = TransferEventData::try_from_val(&env, &last_event.2).unwrap();
        assert_eq!(transfer_from_data.to, user2.clone());
        assert_eq!(transfer_from_data.amount, 200);
    }

    #[test]
    #[should_panic(expected = "Supply cap exceeded")]
    fn test_supply_cap() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let contract_id = env.register_contract(None, MNTToken);
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);

        // Mints nearly up to cap
        client.mint(&user, &SUPPLY_CAP);

        // This should fail
        client.mint(&user, &1);
    }

    // -----------------------------------------------------------------------
    // Storage layout readback — full lifecycle
    // Verifies every stored value is readable after a complete lifecycle run.
    // -----------------------------------------------------------------------

    #[test]
    fn test_storage_readback_full_lifecycle() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        let contract_id = env.register_contract(None, MNTToken);
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);

        // Metadata readable after init
        assert_eq!(client.name(), String::from_str(&env, "MentorMinds Token"));
        assert_eq!(client.symbol(), String::from_str(&env, "MNT"));
        assert_eq!(client.decimals(), 7);

        // Balances start at zero
        assert_eq!(client.balance(&user1), 0);
        assert_eq!(client.balance(&user2), 0);

        // Mint → balance readable
        client.mint(&user1, &1_000);
        assert_eq!(client.balance(&user1), 1_000);

        // Approve → allowance readable
        client.approve(&user1, &user2, &400, &100);
        assert_eq!(client.allowance(&user1, &user2), 400);

        // transfer_from → balances and allowance updated
        client.transfer_from(&user2, &user1, &user2, &150);
        assert_eq!(client.balance(&user1), 850);
        assert_eq!(client.balance(&user2), 150);
        assert_eq!(client.allowance(&user1, &user2), 250);

        // transfer → balances updated
        client.transfer(&user1, &user2, &100);
        assert_eq!(client.balance(&user1), 750);
        assert_eq!(client.balance(&user2), 250);

        // burn → balance and supply updated
        client.burn(&user1, &250);
        assert_eq!(client.balance(&user1), 500);

        // burn_from → allowance, balance, supply updated
        client.burn_from(&user2, &user1, &200);
        assert_eq!(client.balance(&user1), 300);
        assert_eq!(client.allowance(&user1, &user2), 50);
    }
}

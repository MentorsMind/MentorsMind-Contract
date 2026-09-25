
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Bytes, BytesN, Env, IntoVal,
    String, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Unified MockMNT Token
// ---------------------------------------------------------------------------

#[contract]
pub struct MockMNT;

#[contractimpl]
impl MockMNT {
    const TOTAL_SUPPLY_KEY: Symbol = symbol_short!("TOTAL");
    const BALANCE_PREFIX: Symbol = symbol_short!("BAL");

    pub fn initialize(env: Env, admin: Address, total_supply: i128) {
        if env.storage().instance().has(&Self::TOTAL_SUPPLY_KEY) {
            panic!("already initialized");
        }
        env.storage()
            .instance()
            .set(&Self::TOTAL_SUPPLY_KEY, &total_supply);
        env.storage()
            .persistent()
            .set(&Self::balance_key(&admin), &total_supply);
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        let mut total = env.storage().instance().get(&Self::TOTAL_SUPPLY_KEY).unwrap_or(0);
        let mut balance = Self::balance(env.clone(), to.clone());
        
        total += amount;
        balance += amount;
        
        env.storage().instance().set(&Self::TOTAL_SUPPLY_KEY, &total);
        env.storage().persistent().set(&to, &balance);
    }

    pub fn burn(env: Env, from: Address, amount: i128) {
        let mut total = env.storage().instance().get(&Self::TOTAL_SUPPLY_KEY).unwrap_or(0);
        let mut balance = Self::balance(env.clone(), from.clone());
        
        if balance < amount {
            panic!("insufficient balance to burn");
        }
        
        total -= amount;
        balance -= amount;
        
        env.storage().instance().set(&Self::TOTAL_SUPPLY_KEY, &total);
        env.storage().persistent().set(&from, &balance);
    }

    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&id)
            .unwrap_or(0)
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        
        let mut from_bal = Self::balance(env.clone(), from.clone());
        let mut to_bal = Self::balance(env.clone(), to.clone());
        
        if from_bal < amount {
            panic!("insufficient balance");
        }
        
        from_bal -= amount;
        to_bal += amount;
        
        env.storage().persistent().set(&from, &from_bal);
        env.storage().persistent().set(&to, &to_bal);
    }

    pub fn name(env: Env) -> String {
        String::from_str(&env, "MentorsMind Token")
    }

    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "MNT")
    }

    pub fn decimals(_env: Env) -> u32 {
        7
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage().instance().get(&Self::TOTAL_SUPPLY_KEY).unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Mock Snapshot Contract for Governance
// ---------------------------------------------------------------------------

#[contract]
pub struct MockSnapshot;

#[contractimpl]
impl MockSnapshot {
    const TOKEN_KEY: Symbol = symbol_short!("TOKEN");
    const SNAPSHOT_KEY: Symbol = symbol_short!("SNAP");

    pub fn set_token(env: Env, token: Address) {
        env.storage().instance().set(&Self::TOKEN_KEY, &token);
    }

    pub fn capture(env: Env, ledger: u32) -> i128 {
        let token = env.storage().instance().get(&Self::TOKEN_KEY).unwrap();
        // Return a mock total supply
        1_000_000_000
    }

    pub fn get_balance_at(env: Env, addr: Address, ledger: u32) -> i128 {
        let token = env.storage().instance().get(&Self::TOKEN_KEY).unwrap();
        // Mock implementation - return actual balance
        env.invoke_contract::<i128>(
            &token,
            &symbol_short!("balance"),
            (addr,).into_val(&env),
        )
    }
}

// ---------------------------------------------------------------------------
// Mock KYC Registry
// ---------------------------------------------------------------------------

#[contract]
pub struct MockKYCRegistry;

#[contractimpl]
impl MockKYCRegistry {
    pub fn set_kyc(env: Env, user: Address, approved: bool) {
        env.storage().persistent().set(&user, &approved);
    }

    pub fn is_kyc(env: Env, user: Address) -> bool {
        env.storage().persistent().get(&user).unwrap_or(true)
    }
}

// ---------------------------------------------------------------------------
// Mock Sanctions
// ---------------------------------------------------------------------------

#[contract]
pub struct MockSanctions;

#[contractimpl]
impl MockSanctions {
    pub fn set_sanctioned(env: Env, user: Address, sanctioned: bool) {
        env.storage().persistent().set(&user, &sanctioned);
    }

    pub fn is_sanctioned(env: Env, user: Address) -> bool {
        env.storage().persistent().get(&user).unwrap_or(false)
    }
}

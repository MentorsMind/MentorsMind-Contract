use soroban_sdk::{contract, contractimpl, symbol_short, Address, Env, IntoVal, String};

#[contract]
pub struct MockToken;
#[contractimpl]
impl MockToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let mut total: i128 = env.storage().persistent().get(&to).unwrap_or(0);
        total += amount;
        env.storage().persistent().set(&to, &total);
    }
    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage().persistent().get(&id).unwrap_or(0)
    }
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        let mut f_bal = Self::balance(env.clone(), from.clone());
        let mut t_bal = Self::balance(env.clone(), to.clone());
        if f_bal < amount {
            panic!("Insuff balance");
        }
        f_bal -= amount;
        t_bal += amount;
        env.storage().persistent().set(&from, &f_bal);
        env.storage().persistent().set(&to, &t_bal);
    }
    pub fn name(env: Env) -> String {
        String::from_str(&env, "Mock")
    }
    pub fn symbol(env: Env) -> String {
        String::from_str(&env, "MCK")
    }
    pub fn decimals(_env: Env) -> u32 {
        7
    }
    pub fn total_supply(_env: Env) -> i128 {
        1_000_000_000
    }
}

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

#[contract]
pub struct MockVelocityLimits;
#[contractimpl]
impl MockVelocityLimits {
    pub fn set_fail(env: Env, fail: bool) {
        env.storage()
            .persistent()
            .set(&soroban_sdk::symbol_short!("fail"), &fail);
    }
    pub fn check_and_record(env: Env, _user: Address, _amount: i128) -> bool {
        let fail: bool = env
            .storage()
            .persistent()
            .get(&soroban_sdk::symbol_short!("fail"))
            .unwrap_or(false);
        !fail
    }
}

#[soroban_sdk::contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeRecord {
    pub mentor: Address,
    pub amount: i128,
    pub staked_at: u64,
    pub unlock_at: u64,
    pub tier: u32,
}

#[contract]
pub struct MockStaking;
#[contractimpl]
impl MockStaking {
    pub fn stake(env: Env, user: Address) -> StakeRecord {
        StakeRecord {
            mentor: user,
            amount: 1000,
            staked_at: env.ledger().timestamp(),
            unlock_at: env.ledger().timestamp() + 86400,
            tier: 1,
        }
    }
}

#[contract]
pub struct MockLendingPool;
#[contractimpl]
impl MockLendingPool {
    pub fn borrow(env: Env, user: Address, _amount: i128, credit_score_addr: Address) -> bool {
        let score: u32 = env.invoke_contract(
            &credit_score_addr,
            &symbol_short!("get_score"),
            (user,).into_val(&env),
        );
        if score < 500 {
            panic!("Credit score too low");
        }
        true
    }
}

#[contract]
pub struct MockHealthDashboard;
#[contractimpl]
impl MockHealthDashboard {
    pub fn get_summary(env: Env, escrow_addr: Address) -> u64 {
        env.invoke_contract::<u64>(
            &escrow_addr,
            &soroban_sdk::Symbol::new(&env, "get_escrow_count"),
            soroban_sdk::Vec::new(&env),
        )
    }
}

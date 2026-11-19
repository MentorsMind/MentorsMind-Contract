#![no_std]

use shared::events::{
    emit_allowance_event, evt_allowance_authorized, evt_allowance_pulled, evt_allowance_revoked,
};
use shared::pause_guard::require_not_paused;
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, vec, Address, Env, IntoVal, Symbol,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AllowanceRecord {
    pub owner: Address,
    pub spender: Address,
    pub token: Address,
    pub amount_per_period: i128,
    pub max_periods: u32,
    pub period_seconds: u64,
    pub periods_used: u32,
    pub last_pull_timestamp: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    /// Contract administrator, set once at `initialize`.
    Admin,
    /// Address of the pause-guardian contract used as an emergency stop.
    PauseGuardian,
    Allowance(Address, Address, Address), // (owner, spender, token)
}

#[contract]
pub struct RecurringAllowanceContract;

#[contractimpl]
impl RecurringAllowanceContract {
    /// Initialise the contract with its admin. Can only be called once.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
    }

    /// Admin-only: set or update the pause-guardian contract address used as
    /// the emergency stop for [`Self::pull_payment`].
    pub fn set_pause_guardian(env: Env, admin: Address, guardian: Address) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        if stored_admin != admin {
            panic!("Unauthorized");
        }
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::PauseGuardian, &guardian);
    }

    pub fn get_pause_guardian(env: Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::PauseGuardian)
    }

    pub fn authorize(
        env: Env,
        owner: Address,
        spender: Address,
        token: Address,
        amount_per_period: i128,
        max_periods: u32,
        period_seconds: u64,
    ) {
        owner.require_auth();

        if amount_per_period <= 0 {
            panic!("amount_per_period must be positive");
        }
        if max_periods == 0 {
            panic!("max_periods must be positive");
        }
        if period_seconds == 0 {
            panic!("period_seconds must be positive");
        }

        let key = DataKey::Allowance(owner.clone(), spender.clone(), token.clone());
        let record = AllowanceRecord {
            owner: owner.clone(),
            spender: spender.clone(),
            token: token.clone(),
            amount_per_period,
            max_periods,
            period_seconds,
            periods_used: 0,
            last_pull_timestamp: 0,
        };

        env.storage().persistent().set(&key, &record);

        emit_allowance_event(
            &env,
            evt_allowance_authorized(&env),
            (spender, token, amount_per_period, max_periods, period_seconds),
        );
    }

    pub fn pull_payment(env: Env, spender: Address, owner: Address, token: Address, amount: i128) {
        if let Some(guardian) = env
            .storage()
            .persistent()
            .get::<DataKey, Address>(&DataKey::PauseGuardian)
        {
            require_not_paused(&env, &guardian);
        }

        spender.require_auth();

        if amount <= 0 {
            panic!("amount must be positive");
        }

        let key = DataKey::Allowance(owner.clone(), spender.clone(), token.clone());
        let mut record: AllowanceRecord = env
            .storage()
            .persistent()
            .get(&key)
            .expect("allowance not found");

        if record.periods_used >= record.max_periods {
            panic!("allowance exhausted");
        }

        if amount > record.amount_per_period {
            panic!("amount exceeds allowance");
        }

        let now = env.ledger().timestamp();
        if record.periods_used > 0 {
            let earliest_next_pull = record
                .last_pull_timestamp
                .checked_add(record.period_seconds)
                .expect("timestamp overflow");
            if now < earliest_next_pull {
                panic!("pull too early");
            }
        }

        let transfer_from_fn = Symbol::new(&env, "transfer_from");
        let args = vec![
            &env,
            spender.clone().into_val(&env),
            owner.clone().into_val(&env),
            spender.clone().into_val(&env),
            amount.into_val(&env),
        ];
        env.invoke_contract::<()>(&token, &transfer_from_fn, args);

        record.periods_used = record
            .periods_used
            .checked_add(1)
            .expect("periods_used overflow");
        record.last_pull_timestamp = now;
        env.storage().persistent().set(&key, &record);

        emit_allowance_event(
            &env,
            evt_allowance_pulled(&env),
            (spender, token, amount, record.periods_used),
        );
    }

    pub fn revoke(env: Env, owner: Address, spender: Address, token: Address) {
        owner.require_auth();

        let key = DataKey::Allowance(owner.clone(), spender.clone(), token.clone());
        if !env.storage().persistent().has(&key) {
            panic!("allowance not found");
        }

        env.storage().persistent().remove(&key);
        emit_allowance_event(&env, evt_allowance_revoked(&env), (spender, token));
    }

    pub fn get_allowance(
        env: Env,
        owner: Address,
        spender: Address,
        token: Address,
    ) -> AllowanceRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Allowance(owner, spender, token))
            .expect("allowance not found")
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[contract]
    pub struct MockToken;

    #[contractimpl]
    impl MockToken {
        pub fn mint(env: Env, to: Address, amount: i128) {
            let key = (symbol_short!("BAL"), to);
            let current: i128 = env.storage().persistent().get(&key).unwrap_or(0);
            env.storage().persistent().set(&key, &(current + amount));
        }

        pub fn balance(env: Env, addr: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&(symbol_short!("BAL"), addr))
                .unwrap_or(0)
        }

        pub fn transfer_from(
            env: Env,
            _spender: Address,
            from: Address,
            to: Address,
            amount: i128,
        ) {
            let from_key = (symbol_short!("BAL"), from.clone());
            let to_key = (symbol_short!("BAL"), to.clone());
            let from_balance: i128 = env.storage().persistent().get(&from_key).unwrap_or(0);
            let to_balance: i128 = env.storage().persistent().get(&to_key).unwrap_or(0);

            if from_balance < amount {
                panic!("insufficient balance");
            }

            env.storage()
                .persistent()
                .set(&from_key, &(from_balance - amount));
            env.storage()
                .persistent()
                .set(&to_key, &(to_balance + amount));
        }
    }

    #[contract]
    pub struct MockPauseGuardian;

    #[contractimpl]
    impl MockPauseGuardian {
        pub fn set_paused(env: Env, paused: bool) {
            env.storage()
                .persistent()
                .set(&symbol_short!("PAUSED"), &paused);
        }

        pub fn is_paused(env: Env) -> bool {
            env.storage()
                .persistent()
                .get(&symbol_short!("PAUSED"))
                .unwrap_or(false)
        }
    }

    #[test]
    fn test_pause_blocks_pull_payment() {
        let env = Env::default();
        env.mock_all_auths();

        let allowance_id = env.register_contract(None, RecurringAllowanceContract);
        let token_id = env.register_contract(None, MockToken);
        let guardian_id = env.register_contract(None, MockPauseGuardian);
        let allowance = RecurringAllowanceContractClient::new(&env, &allowance_id);
        let token = MockTokenClient::new(&env, &token_id);
        let guardian = MockPauseGuardianClient::new(&env, &guardian_id);

        let admin = Address::generate(&env);
        let owner = Address::generate(&env);
        let spender = Address::generate(&env);
        token.mint(&owner, &1_000);

        allowance.initialize(&admin);
        allowance.authorize(&owner, &spender, &token_id, &100, &5, &60);
        allowance.set_pause_guardian(&admin, &guardian_id);

        guardian.set_paused(&true);
        let result = allowance.try_pull_payment(&spender, &owner, &token_id, &100);
        assert!(result.is_err(), "paused contract must block pull_payment");
        assert_eq!(token.balance(&spender), 0);

        guardian.set_paused(&false);
        allowance.pull_payment(&spender, &owner, &token_id, &100);
        assert_eq!(token.balance(&spender), 100);
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_set_pause_guardian_rejects_non_admin() {
        let env = Env::default();
        env.mock_all_auths();

        let allowance_id = env.register_contract(None, RecurringAllowanceContract);
        let guardian_id = env.register_contract(None, MockPauseGuardian);
        let allowance = RecurringAllowanceContractClient::new(&env, &allowance_id);

        let admin = Address::generate(&env);
        let stranger = Address::generate(&env);
        allowance.initialize(&admin);
        allowance.set_pause_guardian(&stranger, &guardian_id);
    }

    #[test]
    fn test_authorize_and_pull_on_time() {
        let env = Env::default();
        env.mock_all_auths();

        let allowance_id = env.register_contract(None, RecurringAllowanceContract);
        let token_id = env.register_contract(None, MockToken);
        let allowance = RecurringAllowanceContractClient::new(&env, &allowance_id);
        let token = MockTokenClient::new(&env, &token_id);

        let owner = Address::generate(&env);
        let spender = Address::generate(&env);
        token.mint(&owner, &1_000);

        allowance.authorize(&owner, &spender, &token_id, &200, &3, &100);
        allowance.pull_payment(&spender, &owner, &token_id, &150);

        let owner_balance = token.balance(&owner);
        let spender_balance = token.balance(&spender);
        assert_eq!(owner_balance, 850);
        assert_eq!(spender_balance, 150);

        env.ledger().set_timestamp(env.ledger().timestamp() + 101);
        allowance.pull_payment(&spender, &owner, &token_id, &200);

        let record = allowance.get_allowance(&owner, &spender, &token_id);
        assert_eq!(record.periods_used, 2);
        assert_eq!(record.last_pull_timestamp, env.ledger().timestamp());
    }

    #[test]
    #[should_panic(expected = "pull too early")]
    fn test_early_pull_rejection() {
        let env = Env::default();
        env.mock_all_auths();

        let allowance_id = env.register_contract(None, RecurringAllowanceContract);
        let token_id = env.register_contract(None, MockToken);
        let allowance = RecurringAllowanceContractClient::new(&env, &allowance_id);

        let owner = Address::generate(&env);
        let spender = Address::generate(&env);
        let token = MockTokenClient::new(&env, &token_id);
        token.mint(&owner, &1000);

        allowance.authorize(&owner, &spender, &token_id, &100, &5, &60);
        allowance.pull_payment(&spender, &owner, &token_id, &100);

        env.ledger().set_timestamp(env.ledger().timestamp() + 30);
        allowance.pull_payment(&spender, &owner, &token_id, &100);
    }

    #[test]
    #[should_panic(expected = "allowance not found")]
    fn test_revoke() {
        let env = Env::default();
        env.mock_all_auths();

        let allowance_id = env.register_contract(None, RecurringAllowanceContract);
        let token_id = env.register_contract(None, MockToken);
        let allowance = RecurringAllowanceContractClient::new(&env, &allowance_id);

        let owner = Address::generate(&env);
        let spender = Address::generate(&env);

        allowance.authorize(&owner, &spender, &token_id, &120, &2, &60);
        allowance.revoke(&owner, &spender, &token_id);

        allowance.get_allowance(&owner, &spender, &token_id);
    }
}

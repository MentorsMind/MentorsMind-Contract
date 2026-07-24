#![no_std]

use shared::events::{emit_staking_event, evt_staking_staked, evt_staking_unstaked};
use shared::ReentrancyGuard;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, Env, Symbol,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidAmount = 3,
    AlreadyStaked = 4,
    NoStakeFound = 5,
    StillLocked = 6,
}

// ---------------------------------------------------------------------------
// Storage types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeRecord {
    pub mentor: Address,
    pub amount: i128,
    pub staked_at: u64,
    pub unlock_at: u64,
    pub unlock_cooldown_until: Option<u64>,
    pub tier: u32,
}

// ---------------------------------------------------------------------------
// Event data types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakedEventData {
    pub mentor: Address,
    pub amount: i128,
    pub unlock_at: u64,
    pub unlock_cooldown_until: Option<u64>,
    pub tier: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnstakedEventData {
    pub mentor: Address,
    pub amount: i128,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    MNTToken,
    Stake(Address),
    StakerAt(u32),
    StakerCount,
    StakerIndex(Address),
    Stakers,
    TotalStaked,
    PendingRewards(Address),
}

// ---------------------------------------------------------------------------
// Tier thresholds (raw i128, no decimals assumed — callers pass raw amounts)
// Thresholds: Bronze ≥ 100, Silver ≥ 500, Gold ≥ 2000
// ---------------------------------------------------------------------------

const TIER_BRONZE: i128 = 100;
const TIER_SILVER: i128 = 500;
const TIER_GOLD: i128 = 2_000;

fn compute_tier(amount: i128) -> u32 {
    if amount >= TIER_GOLD {
        3
    } else if amount >= TIER_SILVER {
        2
    } else if amount >= TIER_BRONZE {
        1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct StakingContract;

#[contractimpl]
impl StakingContract {
    /// Initialize the staking contract.
    /// Must be called once before any other function.
    pub fn initialize(env: Env, admin: Address, mnt_token: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MNTToken, &mnt_token);
        Ok(())
    }

    /// Stake MNT tokens for a given lock period.
    ///
    /// - Transfers `amount` MNT from `mentor` to this contract.
    /// - Stores a StakeRecord with tier derived from amount.
    /// - A mentor can only have one active stake at a time.
    ///
    /// Auth: `mentor` must authorize this call.
    pub fn stake(
        env: Env,
        mentor: Address,
        amount: i128,
        lock_period_days: u32,
    ) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "stake"));
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if env
            .storage()
            .persistent()
            .has(&DataKey::Stake(mentor.clone()))
        {
            return Err(Error::AlreadyStaked);
        }

        mentor.require_auth();

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .ok_or(Error::NotInitialized)?;

        // Transfer MNT from mentor to this contract
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&mentor, &env.current_contract_address(), &amount);

        let now = env.ledger().timestamp();
        let lock_seconds = (lock_period_days as u64).checked_mul(86_400u64).expect("Overflow");
        let unlock_at = now.checked_add(lock_seconds).expect("Timestamp overflow");
        let tier = compute_tier(amount);

        let record = StakeRecord {
            mentor: mentor.clone(),
            amount,
            staked_at: now,
            unlock_at,
            unlock_cooldown_until: None,
            tier,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Stake(mentor.clone()), &record);

        // Update stakers list and total staked
        let key = DataKey::StakerIndex(mentor.clone());
        if !env.storage().persistent().has(&key) {
            let count: u32 = env.storage().persistent().get(&DataKey::StakerCount).unwrap_or(0);
            env.storage().persistent().set(&DataKey::StakerAt(count), &mentor);
            env.storage().persistent().set(&key, &count);
            env.storage().persistent().set(&DataKey::StakerCount, &(count + 1));
        }

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::TotalStaked, &(total_staked.checked_add(amount).expect("Overflow")));

        emit_staking_event(
            &env,
            evt_staking_staked(&env),
            StakedEventData {
                mentor,
                amount,
                unlock_at,
                unlock_cooldown_until: None,
                tier,
            },
        );

        Ok(())
    }

    /// Unstake MNT tokens after the lock period has expired.
    ///
    /// - Returns the full staked amount back to `mentor`.
    /// - Removes the StakeRecord.
    ///
    /// Auth: `mentor` must authorize this call.
    pub fn unstake(env: Env, mentor: Address) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "unstake"));
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        let record: StakeRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Stake(mentor.clone()))
            .ok_or(Error::NoStakeFound)?;

        let now = env.ledger().timestamp();
        if now < record.unlock_at {
            return Err(Error::StillLocked);
        }

        mentor.require_auth();

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .ok_or(Error::NotInitialized)?;

        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&env.current_contract_address(), &mentor, &record.amount);

        env.storage()
            .persistent()
            .remove(&DataKey::Stake(mentor.clone()));

        // Update stakers list and total staked
        let key = DataKey::StakerIndex(mentor.clone());
        if let Some(index) = env.storage().persistent().get::<_, u32>(&key) {
            let count: u32 = env.storage().persistent().get(&DataKey::StakerCount).unwrap_or(0);
            let last_index = count - 1;
            
            if index != last_index {
                let last_mentor: Address = env.storage().persistent().get(&DataKey::StakerAt(last_index)).unwrap();
                env.storage().persistent().set(&DataKey::StakerAt(index), &last_mentor);
                env.storage().persistent().set(&DataKey::StakerIndex(last_mentor), &index);
            }
            
            env.storage().persistent().remove(&DataKey::StakerAt(last_index));
            env.storage().persistent().remove(&key);
            env.storage().persistent().set(&DataKey::StakerCount, &last_index);
        }

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::TotalStaked, &(total_staked.checked_sub(record.amount).expect("Underflow")));

        emit_staking_event(
            &env,
            evt_staking_unstaked(&env),
            UnstakedEventData {
                mentor,
                amount: record.amount,
            },
        );

        Ok(())
    }

    /// Return the StakeRecord for a mentor, or an error if none exists.
    pub fn get_stake(env: Env, mentor: Address) -> Result<StakeRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Stake(mentor))
            .ok_or(Error::NoStakeFound)
    }

    /// Return the tier for a mentor.
    /// 0 = None, 1 = Bronze, 2 = Silver, 3 = Gold
    pub fn get_tier(env: Env, mentor: Address) -> u32 {
        env.storage()
            .persistent()
            .get::<DataKey, StakeRecord>(&DataKey::Stake(mentor))
            .map(|r| r.tier)
            .unwrap_or(0)
    }

    pub fn get_staker_count(env: Env) -> u32 {
        env.storage().persistent().get(&DataKey::StakerCount).unwrap_or(0)
    }

    /// Return all current stakers (Paginated).
    pub fn get_stakers(env: Env) -> soroban_sdk::Vec<Address> {
        let count = Self::get_staker_count(env.clone());
        let mut out = soroban_sdk::Vec::new(&env);
        for i in 0..count {
            if let Some(addr) = env.storage().persistent().get::<_, Address>(&DataKey::StakerAt(i)) {
                out.push_back(addr);
            }
        }
        out
    }

    /// Return the total amount staked in the contract.
    pub fn get_total_staked(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0)
    }

    /// Distribute rewards to stakers pro-rata based on their stake amounts.
    /// Processes a window of stakers.
    pub fn distribute_revenue_batch(env: Env, token: Address, amount: i128, offset: u32, limit: u32) {
        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);

        if total_staked == 0 {
            return;
        }

        let count = Self::get_staker_count(env.clone());
        let end = (offset + limit).min(count);

        for i in offset..end {
            if let Some(staker) = env.storage().persistent().get::<_, Address>(&DataKey::StakerAt(i)) {
                let record: StakeRecord = env
                    .storage()
                    .persistent()
                    .get(&DataKey::Stake(staker.clone()))
                    .unwrap();

                let share = (record.amount * amount) / total_staked;

                if share > 0 {
                    let pending: i128 = env
                        .storage()
                        .persistent()
                        .get(&DataKey::PendingRewards(staker.clone()))
                        .unwrap_or(0);
                    env.storage()
                        .persistent()
                        .set(&DataKey::PendingRewards(staker.clone()), &(pending + share));
                }
            }
        }
    }

    pub fn migrate_stakers(env: Env) {
        if !env.storage().instance().has(&DataKey::Admin) {
            panic!("Not initialized");
        }
        if let Some(list) = env.storage().persistent().get::<_, soroban_sdk::Vec<Address>>(&DataKey::Stakers) {
            let mut count: u32 = env.storage().persistent().get(&DataKey::StakerCount).unwrap_or(0);
            for staker in list.iter() {
                if !env.storage().persistent().has(&DataKey::StakerIndex(staker.clone())) {
                    env.storage().persistent().set(&DataKey::StakerAt(count), &staker);
                    env.storage().persistent().set(&DataKey::StakerIndex(staker.clone()), &count);
                    count += 1;
                }
            }
            env.storage().persistent().set(&DataKey::StakerCount, &count);
            env.storage().persistent().remove(&DataKey::Stakers);
        }
    }

    /// Claim pending rewards for a staker.
    /// Transfers the pending rewards to the staker's address.
    pub fn claim_rewards(env: Env, staker: Address, token: Address) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "claim_rewards"));
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        staker.require_auth();

        let pending: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PendingRewards(staker.clone()))
            .unwrap_or(0);

        if pending == 0 {
            return Err(Error::InvalidAmount);
        }

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&env.current_contract_address(), &staker, &pending);

        env.storage()
            .persistent()
            .remove(&DataKey::PendingRewards(staker.clone()));

        Ok(())
    }

    /// Get the pending rewards for a staker.
    pub fn get_pending_rewards(env: Env, staker: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::PendingRewards(staker))
            .unwrap_or(0)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    // ---------------------------------------------------------------------------
    // Minimal mock MNT token — mirrors the real token's storage pattern so that
    // token::Client calls (transfer / balance) work correctly in tests.
    // ---------------------------------------------------------------------------

    #[contracttype]
    #[derive(Clone)]
    pub enum MockDataKey {
        Balance(Address),
    }

    #[contract]
    pub struct MockMNT;

    #[contractimpl]
    impl MockMNT {
        pub fn mint(env: Env, to: Address, amount: i128) {
            let bal: i128 = env
                .storage()
                .persistent()
                .get(&MockDataKey::Balance(to.clone()))
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(to), &(bal + amount));
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&MockDataKey::Balance(id))
                .unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            let from_bal = Self::balance(env.clone(), from.clone());
            assert!(from_bal >= amount, "Insufficient balance");
            let to_bal = Self::balance(env.clone(), to.clone());
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(from), &(from_bal - amount));
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(to), &(to_bal + amount));
        }
    }

    struct Fixture {
        env: Env,
        staking_id: Address,
        mnt_id: Address,
        admin: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let mnt_id = env.register_contract(None, MockMNT);

            let staking_id = env.register_contract(None, StakingContract);
            StakingContractClient::new(&env, &staking_id).initialize(&admin, &mnt_id);

            Fixture {
                env,
                staking_id,
                mnt_id,
                admin,
            }
        }

        fn client(&self) -> StakingContractClient {
            StakingContractClient::new(&self.env, &self.staking_id)
        }

        fn mnt(&self) -> MockMNTClient {
            MockMNTClient::new(&self.env, &self.mnt_id)
        }

        fn fund(&self, addr: &Address, amount: i128) {
            self.mnt().mint(addr, &amount);
        }
    }

    // -----------------------------------------------------------------------
    // stake / tier assignment
    // -----------------------------------------------------------------------

    #[test]
    fn test_stake_assigns_no_tier_below_bronze() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 50);

        f.client().stake(&mentor, &50, &30);

        assert_eq!(f.client().get_tier(&mentor), 0);
        let record = f.client().get_stake(&mentor);
        assert_eq!(record.amount, 50);
        assert_eq!(record.tier, 0);
    }

    #[test]
    fn test_stake_assigns_bronze_tier() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 100);

        f.client().stake(&mentor, &100, &30);

        assert_eq!(f.client().get_tier(&mentor), 1);
    }

    #[test]
    fn test_stake_assigns_silver_tier() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &30);

        assert_eq!(f.client().get_tier(&mentor), 2);
    }

    #[test]
    fn test_stake_assigns_gold_tier() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 2_000);

        f.client().stake(&mentor, &2_000, &30);

        assert_eq!(f.client().get_tier(&mentor), 3);
    }

    #[test]
    fn test_stake_stores_correct_unlock_at() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(1_000_000);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &10);

        let record = f.client().get_stake(&mentor);
        // 10 days * 86400 seconds
        assert_eq!(record.unlock_at, 1_000_000 + 10 * 86_400);
    }

    #[test]
    fn test_stake_transfers_tokens_to_contract() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 1_000);

        f.client().stake(&mentor, &1_000, &30);

        assert_eq!(f.mnt().balance(&mentor), 0);
        assert_eq!(f.mnt().balance(&f.staking_id), 1_000);
    }

    #[test]
    fn test_stake_rejects_duplicate() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 2_000);

        f.client().stake(&mentor, &500, &30);

        let result = f.client().try_stake(&mentor, &500, &30);
        assert_eq!(result, Err(Ok(Error::AlreadyStaked)));
    }

    #[test]
    fn test_stake_rejects_zero_amount() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);

        let result = f.client().try_stake(&mentor, &0, &30);
        assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    }

    // -----------------------------------------------------------------------
    // unstake
    // -----------------------------------------------------------------------

    #[test]
    fn test_unstake_after_lock_returns_tokens() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &30);

        // Advance past lock period
        f.env.ledger().set_timestamp(30 * 86_400 + 1);

        f.client().unstake(&mentor);

        assert_eq!(f.mnt().balance(&mentor), 500);
        assert_eq!(f.mnt().balance(&f.staking_id), 0);

        // Stake record should be gone
        let result = f.client().try_get_stake(&mentor);
        assert_eq!(result, Err(Ok(Error::NoStakeFound)));
    }

    #[test]
    fn test_unstake_rejects_early_unlock() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &30);

        // Only 1 day has passed — still locked
        f.env.ledger().set_timestamp(86_400);

        let result = f.client().try_unstake(&mentor);
        assert_eq!(result, Err(Ok(Error::StillLocked)));
    }

    #[test]
    fn test_unstake_rejects_no_stake() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);

        let result = f.client().try_unstake(&mentor);
        assert_eq!(result, Err(Ok(Error::NoStakeFound)));
    }

    // -----------------------------------------------------------------------
    // get_tier for unstaked mentor
    // -----------------------------------------------------------------------

    #[test]
    fn test_get_tier_returns_zero_when_no_stake() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        assert_eq!(f.client().get_tier(&mentor), 0);
    }

    // -----------------------------------------------------------------------
    // double-initialize guard
    // -----------------------------------------------------------------------

    #[test]
    fn test_initialize_rejects_double_init() {
        let f = Fixture::setup();
        let result = f.client().try_initialize(&f.admin, &f.mnt_id);
        assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
    }

    #[test]
    fn test_distribute_revenue_batch_benchmark() {
        let f = Fixture::setup();
        // create 50 stakers to fit within normal instruction budget if iterating, but we batch 10
        for i in 0..50 {
            let mentor = Address::generate(&f.env);
            f.fund(&mentor, 1000);
            f.client().stake(&mentor, &100, &30);
        }
        
        f.env.budget().reset_unlimited();
        f.client().distribute_revenue_batch(&f.mnt_id, &10000, &0, &10);
        f.env.budget().print();
    }
}

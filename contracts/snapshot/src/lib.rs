#![no_std]

use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, Symbol, Vec, IntoVal, FromVal,
};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    StakingContract,
    Snapshot(u32, Address), // (snapshot_id, voter)
    SnapshotTotalSupply(u32), // snapshot_id
}

#[contract]
pub struct SnapshotContract;

#[contractimpl]
impl SnapshotContract {
    /// Initialize the snapshot contract.
    pub fn initialize(env: Env, admin: Address, staking_contract: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::StakingContract, &staking_contract);
    }

    /// records all staked MNT balances at current ledger
    pub fn record_snapshot(env: Env, snapshot_id: u32) {
        let staking_contract: Address = env.storage().persistent().get(&DataKey::StakingContract).expect("not initialized");
        
        // 1. Get total supply at this snapshot
        let total_supply: i128 = env.invoke_contract(&staking_contract, &Symbol::new(&env, "get_total_staked"), Vec::new(&env));
        env.storage().persistent().set(&DataKey::SnapshotTotalSupply(snapshot_id), &total_supply);

        // 2. Get all stakers and record their balances
        let stakers: Vec<Address> = env.invoke_contract(&staking_contract, &Symbol::new(&env, "get_stakers"), Vec::new(&env));
        
        let thirty_days_ledgers = 30 * 24 * 60 * 60 / 5; // Approx 5s per ledger

        for staker in stakers.iter() {
            // Get stake record (amount is what we need)
            // StakeRecord is struct { mentor, amount, staked_at, unlock_at, tier }
            // We can invoke get_stake
            let stake_record: soroban_sdk::Val = env.invoke_contract(&staking_contract, &Symbol::new(&env, "get_stake"), (staker.clone(),).into_val(&env));
            
            // Extract amount from StakeRecord (it's the second field)
            // Instead of parsing the struct here, we can just use a helper or assume the structure
            // Actually, we can define the struct here too.
            
            #[contracttype]
            #[derive(Clone, Debug, Eq, PartialEq)]
            pub struct StakeRecord {
                pub mentor: Address,
                pub amount: i128,
                pub staked_at: u64,
                pub unlock_at: u64,
                pub tier: u8,
            }
            
            let record: StakeRecord = env.from_val(stake_record);
            let key = DataKey::Snapshot(snapshot_id, staker.clone());
            env.storage().persistent().set(&key, &record.amount);
            
            // Auto-expire: extend TTL for 30 days
            env.storage().persistent().extend_ttl(&key, thirty_days_ledgers, thirty_days_ledgers);
        }
        
        // Also extend TTL for total supply
        let ts_key = DataKey::SnapshotTotalSupply(snapshot_id);
        env.storage().persistent().extend_ttl(&ts_key, thirty_days_ledgers, thirty_days_ledgers);
    }

    /// returns the voting power for a voter at a specific snapshot
    pub fn get_voting_power(env: Env, snapshot_id: u32, voter: Address) -> i128 {
        env.storage().persistent().get(&DataKey::Snapshot(snapshot_id, voter)).unwrap_or(0)
    }

    /// returns the total supply at a specific snapshot for quorum calculation
    pub fn get_total_supply_at(env: Env, snapshot_id: u32) -> i128 {
        env.storage().persistent().get(&DataKey::SnapshotTotalSupply(snapshot_id)).unwrap_or(0)
    }
}

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::{Address as _};
    use soroban_sdk::{Env, IntoVal, symbol_short};

    #[contracttype]
    #[derive(Clone, Debug, Eq, PartialEq)]
    pub struct StakeRecord {
        pub mentor: Address,
        pub amount: i128,
        pub staked_at: u64,
        pub unlock_at: u64,
        pub tier: u8,
    }

    #[contract]
    pub struct MockStaking;

    #[contractimpl]
    impl MockStaking {
        pub fn get_total_staked(env: Env) -> i128 {
            env.storage().persistent().get(&symbol_short!("TOT_STK")).unwrap_or(0)
        }
        pub fn set_total_staked(env: Env, amount: i128) {
            env.storage().persistent().set(&symbol_short!("TOT_STK"), &amount);
        }
        pub fn get_stakers(env: Env) -> Vec<Address> {
            env.storage().persistent().get(&symbol_short!("STAKERS")).unwrap_or_else(|| Vec::new(&env))
        }
        pub fn set_stakers(env: Env, stakers: Vec<Address>) {
            env.storage().persistent().set(&symbol_short!("STAKERS"), &stakers);
        }
        pub fn get_stake(env: Env, mentor: Address) -> StakeRecord {
            env.storage().persistent().get(&(symbol_short!("STAKE"), mentor)).unwrap()
        }
        pub fn set_stake(env: Env, mentor: Address, amount: i128) {
            let record = StakeRecord {
                mentor: mentor.clone(),
                amount,
                staked_at: 0,
                unlock_at: 100,
                tier: 1,
            };
            env.storage().persistent().set(&(symbol_short!("STAKE"), mentor), &record);
        }
    }

    #[test]
    fn test_snapshot_logic() {
        let env = Env::default();
        env.mock_all_auths();

        let snapshot_id = env.register_contract(None, SnapshotContract);
        let staking_id = env.register_contract(None, MockStaking);
        let client = SnapshotContractClient::new(&env, &snapshot_id);
        let staking = MockStakingClient::new(&env, &staking_id);

        let admin = Address::generate(&env);
        client.initialize(&admin, &staking_id);

        let voter1 = Address::generate(&env);
        let voter2 = Address::generate(&env);
        
        staking.set_total_staked(&1000);
        staking.set_stakers(&Vec::from_array(&env, [voter1.clone(), voter2.clone()]));
        staking.set_stake(&voter1, &400);
        staking.set_stake(&voter2, &600);

        // Record snapshot 1
        client.record_snapshot(&1);

        assert_eq!(client.get_total_supply_at(&1), 1000);
        assert_eq!(client.get_voting_power(&1, &voter1), 400);
        assert_eq!(client.get_voting_power(&1, &voter2), 600);

        // Change balances
        staking.set_total_staked(&1500);
        staking.set_stake(&voter1, &900);

        // Snapshot 1 should still show old balances
        assert_eq!(client.get_voting_power(&1, &voter1), 400);

        // Record snapshot 2
        client.record_snapshot(&2);
        assert_eq!(client.get_voting_power(&2, &voter1), 900);
    }
}

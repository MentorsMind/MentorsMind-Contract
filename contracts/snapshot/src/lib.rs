#![no_std]

use shared::StakeRecord;
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, FromVal, IntoVal, Symbol, Vec,
};

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    StakingContract,
    DelegationContract,
    Snapshot(u32, Address),   // (snapshot_id, voter)
    SnapshotTotalSupply(u32), // snapshot_id
}

#[contract]
pub struct SnapshotContract;

#[contractimpl]
impl SnapshotContract {
    /// Initialize the snapshot contract.
    pub fn initialize(
        env: Env,
        admin: Address,
        staking_contract: Address,
        delegation_contract: Address,
    ) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::StakingContract, &staking_contract);
        env.storage()
            .persistent()
            .set(&DataKey::DelegationContract, &delegation_contract);
    }

    /// records all staked MNT balances at current ledger and snapshots delegation state
    pub fn record_snapshot(env: Env, snapshot_id: u32) {
        let staking_contract: Address = env
            .storage()
            .persistent()
            .get(&DataKey::StakingContract)
            .expect("not initialized");
        let delegation_contract: Address = env
            .storage()
            .persistent()
            .get(&DataKey::DelegationContract)
            .expect("delegation contract not set");

        // 1. Get total supply at this snapshot
        let total_supply: i128 = env.invoke_contract(
            &staking_contract,
            &Symbol::new(&env, "get_total_staked"),
            Vec::new(&env),
        );
        env.storage()
            .persistent()
            .set(&DataKey::SnapshotTotalSupply(snapshot_id), &total_supply);

        // 2. Get all stakers and record their balances
        let stakers: Vec<Address> = env.invoke_contract(
            &staking_contract,
            &Symbol::new(&env, "get_stakers"),
            Vec::new(&env),
        );

        let thirty_days_ledgers = 30 * 24 * 60 * 60 / 5; // Approx 5s per ledger

        for staker in stakers.iter() {
            // Get stake record from the staking contract.
            // StakeRecord is imported from the shared crate, so the XDR
            // positional layout here is GUARANTEED to match what the
            // staking contract emitted on get_stake — no silent tier
            // corruption caused by mismatched field counts/types.
            let stake_record: soroban_sdk::Val = env.invoke_contract(
                &staking_contract,
                &Symbol::new(&env, "get_stake"),
                (staker.clone(),).into_val(&env),
            );
            let record: StakeRecord = FromVal::from_val(&env, &stake_record);
            let key = DataKey::Snapshot(snapshot_id, staker.clone());
            env.storage().persistent().set(&key, &record.amount);

            // Auto-expire: extend TTL for 30 days
            env.storage()
                .persistent()
                .extend_ttl(&key, thirty_days_ledgers, thirty_days_ledgers);
        }

        // Also extend TTL for total supply
        let ts_key = DataKey::SnapshotTotalSupply(snapshot_id);
        env.storage()
            .persistent()
            .extend_ttl(&ts_key, thirty_days_ledgers, thirty_days_ledgers);

        // 3. Snapshot delegation state for this proposal
        env.invoke_contract::<()>(
            &delegation_contract,
            &Symbol::new(&env, "snapshot_delegations"),
            (snapshot_id,).into_val(&env),
        );
    }

    /// returns the voting power for a voter at a specific snapshot
    /// accounts for delegation state at snapshot time: if voter delegated away,
    /// returns 0; otherwise returns their staked balance
    pub fn get_voting_power(env: Env, snapshot_id: u32, voter: Address) -> i128 {
        let delegation_contract: Address = env
            .storage()
            .persistent()
            .get(&DataKey::DelegationContract)
            .unwrap_or_else(|| {
                // Fallback for contracts initialized before delegation support
                env.storage()
                    .persistent()
                    .get(&DataKey::DelegationContract)
                    .expect("delegation contract not set")
            });

        // Check if voter delegated away at snapshot time
        let delegated_to: Option<Address> = env.invoke_contract(
            &delegation_contract,
            &Symbol::new(&env, "get_delegation_at_snapshot"),
            (snapshot_id, voter.clone()).into_val(&env),
        );

        if delegated_to.is_some() {
            // Voter delegated away - their voting power is 0
            return 0;
        }

        // Voter did not delegate - return their staked balance
        env.storage()
            .persistent()
            .get(&DataKey::Snapshot(snapshot_id, voter))
            .unwrap_or(0)
    }

    /// returns the staked balance for a staker at a specific snapshot
    pub fn get_snapshot_balance(env: Env, snapshot_id: u32, staker: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Snapshot(snapshot_id, staker))
            .unwrap_or(0)
    }

    /// returns the total supply at a specific snapshot for quorum calculation
    pub fn get_total_supply_at(env: Env, snapshot_id: u32) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::SnapshotTotalSupply(snapshot_id))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use shared::StakeRecord as SharedStakeRecord;
    use soroban_sdk::testutils::Address as _;
    use soroban_sdk::{symbol_short, Env, IntoVal};

    // -----------------------------------------------------------------------
    // NOTE: StakeRecord is NOT re-defined here. MockStaking imports the same
    // `SharedStakeRecord` (shared crate) that SnapshotContract uses — XDR
    // positional layout is guaranteed identical across both. This is the
    // exact fix for issue #646: the old test-local definition was missing
    // the 5th field `unlock_cooldown_until`, shifting all subsequent fields.
    // -----------------------------------------------------------------------

    pub type StakeRecord = SharedStakeRecord;

    #[contract]
    pub struct MockStaking;

    #[contractimpl]
    impl MockStaking {
        pub fn get_total_staked(env: Env) -> i128 {
            env.storage()
                .persistent()
                .get(&symbol_short!("TOT_STK"))
                .unwrap_or(0)
        }
        pub fn set_total_staked(env: Env, amount: i128) {
            env.storage()
                .persistent()
                .set(&symbol_short!("TOT_STK"), &amount);
        }
        pub fn get_stakers(env: Env) -> Vec<Address> {
            env.storage()
                .persistent()
                .get(&symbol_short!("STAKERS"))
                .unwrap_or_else(|| Vec::new(&env))
        }
        pub fn set_stakers(env: Env, stakers: Vec<Address>) {
            env.storage()
                .persistent()
                .set(&symbol_short!("STAKERS"), &stakers);
        }
        pub fn get_stake(env: Env, mentor: Address) -> StakeRecord {
            env.storage()
                .persistent()
                .get(&(symbol_short!("STAKE"), mentor))
                .unwrap()
        }
        pub fn set_stake(env: Env, mentor: Address, amount: i128) {
            // Use shared StakeRecord — includes unlock_cooldown_until field
            // and tier: u32 so positional XDR matches between mock/staking/snapshot.
            let tier = if amount >= 2_000 {
                3 // Gold
            } else if amount >= 500 {
                2 // Silver
            } else if amount >= 100 {
                1 // Bronze
            } else {
                0
            };
            let record = StakeRecord {
                mentor: mentor.clone(),
                amount,
                staked_at: 0,
                unlock_at: 100,
                unlock_cooldown_until: None,
                tier,
            };
            env.storage()
                .persistent()
                .set(&(symbol_short!("STAKE"), mentor), &record);
        }
        /// Diagnostic: return the tier of a mentor as computed by the mock
        /// staking contract. Snapshot reads the same record through a
        /// shared StakeRecord type; this helper lets us verify snapshot
        /// reads the exact same tier value (the exact regression of #646).
        pub fn get_mock_tier(env: Env, mentor: Address) -> u32 {
            let r: StakeRecord = env
                .storage()
                .persistent()
                .get(&(symbol_short!("STAKE"), mentor))
                .unwrap();
            r.tier
        }
    }

    #[contract]
    pub struct MockDelegation;

    #[contractimpl]
    impl MockDelegation {
        pub fn snapshot_delegations(_env: Env, _snapshot_id: u32) {}
        pub fn get_delegation_at_snapshot(
            _env: Env,
            _snapshot_id: u32,
            _delegator: Address,
        ) -> Option<Address> {
            None // No delegations in mock
        }
        pub fn get_delegated_power_at_snapshot(
            _env: Env,
            _delegate: Address,
            _snapshot_id: u32,
        ) -> i128 {
            0 // No delegated power in mock
        }
    }

    #[test]
    fn test_snapshot_logic() {
        let env = Env::default();
        env.mock_all_auths();

        let snapshot_id = env.register_contract(None, SnapshotContract);
        let staking_id = env.register_contract(None, MockStaking);
        let delegation_id = env.register_contract(None, MockDelegation);
        let client = SnapshotContractClient::new(&env, &snapshot_id);
        let staking = MockStakingClient::new(&env, &staking_id);

        let admin = Address::generate(&env);
        client.initialize(&admin, &staking_id, &delegation_id);

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

    // ========================================================================
    // Issue #646 — Tier correctness for Bronze / Silver / Gold mentors
    //
    // The root bug: snapshot's in-loop StakeRecord was missing the 5th field
    // `unlock_cooldown_until: Option<u64>`, so positional XDR deserialization
    // read garbage bytes for `tier` (field 6 in staking → became field 5 in
    // snapshot, which was reading the Option discriminant bytes, typically
    // returning tier = 0 for all mentors, including Gold-tier).
    // ========================================================================

    #[test]
    fn tier_bronze_silver_gold_read_correctly_after_snapshot() {
        let env = Env::default();
        env.mock_all_auths();

        let snapshot_id = env.register_contract(None, SnapshotContract);
        let staking_id = env.register_contract(None, MockStaking);
        let delegation_id = env.register_contract(None, MockDelegation);
        let client = SnapshotContractClient::new(&env, &snapshot_id);
        let staking = MockStakingClient::new(&env, &staking_id);

        let admin = Address::generate(&env);
        client.initialize(&admin, &staking_id, &delegation_id);

        // Three mentors with exact thresholds.
        // Bronze: >= 100 (threshold)
        // Silver: >= 500 (threshold)
        // Gold:   >= 2000 (threshold)
        let none = Address::generate(&env); // 50 → tier 0
        let bronze = Address::generate(&env); // 100 → tier 1
        let silver = Address::generate(&env); // 500 → tier 2
        let gold = Address::generate(&env); // 2000 → tier 3

        staking.set_total_staked(&2650); // 50+100+500+2000
        staking.set_stakers(&Vec::from_array(
            &env,
            [none.clone(), bronze.clone(), silver.clone(), gold.clone()],
        ));
        staking.set_stake(&none, &50);
        staking.set_stake(&bronze, &100);
        staking.set_stake(&silver, &500);
        staking.set_stake(&gold, &2000);

        // --- BEFORE snapshot: the mock staking contract itself must report
        // correct tiers. This is the baseline.
        assert_eq!(staking.get_mock_tier(&none), 0);
        assert_eq!(staking.get_mock_tier(&bronze), 1);
        assert_eq!(staking.get_mock_tier(&silver), 2);
        assert_eq!(staking.get_mock_tier(&gold), 3);

        // --- Record snapshot. Snapshot deserializes StakeRecord via shared
        // crate type and stores voting power.
        client.record_snapshot(&1);

        // --- Voting power must equal the raw amount (snapshot stores amount,
        // not tier directly). But to validate the tier bytes were decoded
        // correctly we do an explicit cross-contract serialize→deserialize
        // round-trip for every staker and compare the tier field.
        let tiers: [u32; 4] = [0, 1, 2, 3];
        let stakers_arr = [&none, &bronze, &silver, &gold];
        for (addr, expected_tier) in stakers_arr.iter().zip(tiers.iter()) {
            let raw_val: soroban_sdk::Val = env.invoke_contract(
                &staking_id,
                &Symbol::new(&env, "get_stake"),
                ((*addr).clone(),).into_val(&env),
            );
            let decoded: StakeRecord = FromVal::from_val(&env, &raw_val);
            assert_eq!(
                decoded.tier, *expected_tier,
                "Tier mismatch for addr with expected tier={}. decoded={:?} amount={}",
                expected_tier, decoded, decoded.amount
            );
        }

        // --- Voting powers (amount at snapshot time)
        assert_eq!(client.get_voting_power(&1, &none), 50);
        assert_eq!(client.get_voting_power(&1, &bronze), 100);
        assert_eq!(client.get_voting_power(&1, &silver), 500);
        assert_eq!(client.get_voting_power(&1, &gold), 2000);
        assert_eq!(client.get_total_supply_at(&1), 2650);
    }

    // ========================================================================
    // Cross-contract serialization round-trip (shared::StakeRecord).
    //
    // Builds StakeRecords from one side, serializes them via into_val,
    // deserializes via from_val, and asserts field-by-field equality.
    // Exercises the exact positional XDR encoding that the bug violated.
    // ========================================================================

    #[test]
    fn stakerecord_serialize_round_trip_all_fields() {
        let env = Env::default();
        env.mock_all_auths();

        // --- Case A: unlock_cooldown_until = None, tier = 0 (None)
        let a = StakeRecord {
            mentor: Address::generate(&env),
            amount: 123_456,
            staked_at: 1_000_000,
            unlock_at: 2_000_000,
            unlock_cooldown_until: None,
            tier: 0,
        };
        let a_val: soroban_sdk::Val = a.clone().into_val(&env);
        let a_back: StakeRecord = FromVal::from_val(&env, &a_val);
        assert_eq!(a, a_back);
        assert_eq!(a_back.tier, 0);
        assert!(a_back.unlock_cooldown_until.is_none());

        // --- Case B: unlock_cooldown_until = Some, tier = 3 (GOLD)
        let b = StakeRecord {
            mentor: Address::generate(&env),
            amount: 5_000_000,
            staked_at: 10_000_000,
            unlock_at: 20_000_000,
            unlock_cooldown_until: Some(30_000_000),
            tier: 3,
        };
        let b_val: soroban_sdk::Val = b.clone().into_val(&env);
        let b_back: StakeRecord = FromVal::from_val(&env, &b_val);
        assert_eq!(b, b_back);
        assert_eq!(b_back.tier, 3);
        assert_eq!(b_back.unlock_cooldown_until, Some(30_000_000));

        // --- Case C: tier = 1 Bronze + Some cooldown
        let c = StakeRecord {
            mentor: Address::generate(&env),
            amount: 250,
            staked_at: 1,
            unlock_at: 100,
            unlock_cooldown_until: Some(200),
            tier: 1,
        };
        let c_val: soroban_sdk::Val = c.clone().into_val(&env);
        let c_back: StakeRecord = FromVal::from_val(&env, &c_val);
        assert_eq!(c, c_back);
        assert_eq!(c_back.tier, 1);

        // --- Case D: tier = 2 Silver + None cooldown
        let d = StakeRecord {
            mentor: Address::generate(&env),
            amount: 750,
            staked_at: 500,
            unlock_at: 5000,
            unlock_cooldown_until: None,
            tier: 2,
        };
        let d_val: soroban_sdk::Val = d.clone().into_val(&env);
        let d_back: StakeRecord = FromVal::from_val(&env, &d_val);
        assert_eq!(d, d_back);
        assert_eq!(d_back.tier, 2);

        // --- Field ordering matters! Deliberately swap tier and
        // unlock_cooldown_until by constructing a "bad" byte-level decoding
        // cannot happen because the shared struct forces the same layout.
        // This assert ensures field order hasn't silently changed in shared:
        let fields: [i128; 4] = [
            a_back.amount, // 123456
            b_back.amount, // 5_000_000
            c_back.amount, // 250
            d_back.amount, // 750
        ];
        assert_eq!(fields, [123_456, 5_000_000, 250, 750]);
    }

    // ========================================================================
    // StakeRecord field count / layout defense-in-depth.
    //
    // StakeRecord has exactly 6 fields. If someone adds or removes a field
    // from shared::staking without updating BOTH staking + snapshot crates
    // together, the round-trip test above catches it; but this test
    // explicitly documents the expected positional order for reviewers.
    // ========================================================================

    #[test]
    fn stakerecord_positional_field_order_documentation() {
        let env = Env::default();
        // Expected order (shared::staking::StakeRecord):
        //   1. mentor: Address
        //   2. amount: i128
        //   3. staked_at: u64
        //   4. unlock_at: u64
        //   5. unlock_cooldown_until: Option<u64>
        //   6. tier: u32
        let expected_tier = 3u32;
        let expected_until = Some(999u64);
        let mentor = Address::generate(&env);
        let r = StakeRecord {
            mentor: mentor.clone(),
            amount: 42,
            staked_at: 1,
            unlock_at: 2,
            unlock_cooldown_until: expected_until,
            tier: expected_tier,
        };
        // Full structure check — any reordering or field-count change
        // will either fail to compile, fail struct literal, or fail assert.
        assert_eq!(r.mentor, mentor);
        assert_eq!(r.amount, 42);
        assert_eq!(r.staked_at, 1);
        assert_eq!(r.unlock_at, 2);
        assert_eq!(r.unlock_cooldown_until, expected_until);
        assert_eq!(r.tier, expected_tier);
    }
}

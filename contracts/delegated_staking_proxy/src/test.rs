#![cfg(test)]

use crate::{DelegatedStakingProxy, DelegatedStakingProxyClient};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    Address, Env,
};

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
    proxy_id: Address,
    mnt_id: Address,
    admin: Address,
}

impl Fixture {
    fn setup(lock_period_days: u32) -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let mnt_id = env.register_contract(None, MockMNT);
        let proxy_id = env.register_contract(None, DelegatedStakingProxy);
        DelegatedStakingProxyClient::new(&env, &proxy_id).initialize(
            &admin,
            &mnt_id,
            &lock_period_days,
        );

        Fixture {
            env,
            proxy_id,
            mnt_id,
            admin,
        }
    }

    fn client(&self) -> DelegatedStakingProxyClient {
        DelegatedStakingProxyClient::new(&self.env, &self.proxy_id)
    }

    fn mnt(&self) -> MockMNTClient {
        MockMNTClient::new(&self.env, &self.mnt_id)
    }

    fn fund(&self, who: &Address, amount: i128) {
        self.mnt().mint(who, &amount);
    }
}

#[test]
fn two_owners_reach_gold_tier() {
    let fx = Fixture::setup(30);
    let owner_a = Address::generate(&fx.env);
    let owner_b = Address::generate(&fx.env);
    fx.fund(&owner_a, 600);
    fx.fund(&owner_b, 1_400);

    fx.client().deposit_and_stake(&owner_a, &600);
    fx.client().deposit_and_stake(&owner_b, &1_400);

    assert_eq!(fx.client().get_total_deposited(), 2_000);
    let record = fx.client().get_proxy_stake().unwrap();
    assert_eq!(record.amount, 2_000);
    assert_eq!(record.tier, 3); // Gold
}

#[test]
fn rewards_distributed_pro_rata() {
    let fx = Fixture::setup(30);
    let owner_a = Address::generate(&fx.env);
    let owner_b = Address::generate(&fx.env);
    fx.fund(&owner_a, 600);
    fx.fund(&owner_b, 1_400);
    fx.client().deposit_and_stake(&owner_a, &600);
    fx.client().deposit_and_stake(&owner_b, &1_400);

    // Simulate the proxy receiving 1000 MNT of staking rewards.
    fx.fund(&fx.admin, 1_000);
    fx.client().distribute_rewards(&fx.admin, &1_000);

    // Pro-rata on deposits of 600 / 1400 out of 2000 total => 30% / 70%.
    assert_eq!(fx.client().get_pending_rewards(&owner_a), 300);
    assert_eq!(fx.client().get_pending_rewards(&owner_b), 700);

    let claimed_a = fx.client().claim_rewards_for(&owner_a);
    let claimed_b = fx.client().claim_rewards_for(&owner_b);
    assert_eq!(claimed_a, 300);
    assert_eq!(claimed_b, 700);
    assert_eq!(fx.mnt().balance(&owner_a), 300);
    assert_eq!(fx.mnt().balance(&owner_b), 700);
}

#[test]
fn withdrawal_queues_and_executes_after_lock() {
    let fx = Fixture::setup(10);
    let owner = Address::generate(&fx.env);
    fx.fund(&owner, 1_000);
    fx.client().deposit_and_stake(&owner, &1_000);

    fx.client().withdraw_request(&owner, &400);
    // Still locked: executing before the lock period ends must fail.
    let res = fx.client().try_execute_withdrawal(&owner);
    assert!(res.is_err());

    fx.env.ledger().with_mut(|l| {
        l.timestamp += 10 * 86_400 + 1;
    });

    fx.client().execute_withdrawal(&owner);
    assert_eq!(fx.mnt().balance(&owner), 400);
    assert_eq!(fx.client().get_beneficial_owner(&owner), 600);
    assert_eq!(fx.client().get_total_deposited(), 600);
}

#[test]
fn new_deposit_increases_stake_and_recomputes_tier() {
    let fx = Fixture::setup(30);
    let owner = Address::generate(&fx.env);
    fx.fund(&owner, 2_000);

    fx.client().deposit_and_stake(&owner, &400);
    assert_eq!(fx.client().get_proxy_stake().unwrap().tier, 1); // Bronze

    fx.client().deposit_and_stake(&owner, &600);
    assert_eq!(fx.client().get_proxy_stake().unwrap().tier, 2); // Silver (1000)

    fx.client().deposit_and_stake(&owner, &1_000);
    assert_eq!(fx.client().get_proxy_stake().unwrap().tier, 3); // Gold (2000)
}

#[test]
fn integration_three_owners_stake_earn_claim() {
    let fx = Fixture::setup(5);
    let owner_a = Address::generate(&fx.env);
    let owner_b = Address::generate(&fx.env);
    let owner_c = Address::generate(&fx.env);
    fx.fund(&owner_a, 1_000);
    fx.fund(&owner_b, 2_000);
    fx.fund(&owner_c, 3_000);

    fx.client().deposit_and_stake(&owner_a, &1_000);
    fx.client().deposit_and_stake(&owner_b, &2_000);
    fx.client().deposit_and_stake(&owner_c, &3_000);

    assert_eq!(fx.client().get_total_deposited(), 6_000);
    assert_eq!(fx.client().get_proxy_stake().unwrap().tier, 3); // Gold

    fx.fund(&fx.admin, 600);
    fx.client().distribute_rewards(&fx.admin, &600);

    // 1000/6000, 2000/6000, 3000/6000 of 600 => 100, 200, 300.
    assert_eq!(fx.client().claim_rewards_for(&owner_a), 100);
    assert_eq!(fx.client().claim_rewards_for(&owner_b), 200);
    assert_eq!(fx.client().claim_rewards_for(&owner_c), 300);

    fx.client().withdraw_request(&owner_a, &1_000);
    fx.env.ledger().with_mut(|l| {
        l.timestamp += 5 * 86_400 + 1;
    });
    fx.client().execute_withdrawal(&owner_a);

    assert_eq!(fx.mnt().balance(&owner_a), 1_000 + 100);
    assert_eq!(fx.client().get_total_deposited(), 5_000);
    // Tier recomputed down from Gold once owner_a's 1000 leaves.
    assert_eq!(fx.client().get_proxy_stake().unwrap().tier, 3); // still >= 2000
}

#[test]
fn min_deposit_rejects_dust() {
    let fx = Fixture::setup(30);
    fx.client().set_min_deposit(&fx.admin, &50);

    let owner = Address::generate(&fx.env);
    fx.fund(&owner, 10);
    let res = fx.client().try_deposit_and_stake(&owner, &10);
    assert!(res.is_err());
}

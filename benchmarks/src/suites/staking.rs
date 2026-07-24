/// Staking benchmark suite.
///
/// Covers: stake, unstake, distribute_revenue_batch, claim_rewards.
extern crate std;

use crate::harness::{measure, wasm_size, BenchResult};
use mentorminds_staking::{StakingContract, StakingContractClient};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, Symbol,
};

const CONTRACT: &str = "staking";
const WASM_CRATE: &str = "mentorminds_staking";

// ---------------------------------------------------------------------------
// Minimal mock MNT token (mirrors staking test mock)
// ---------------------------------------------------------------------------

use soroban_sdk::contracttype;

#[contracttype]
enum MockTokKey {
    Bal(Address),
    Total,
}

#[contract]
pub struct MockMntToken;

#[contractimpl]
impl MockMntToken {
    pub fn initialize(_env: Env, _admin: Address) {}
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        let from_bal: i128 = env
            .storage()
            .persistent()
            .get(&MockTokKey::Bal(from.clone()))
            .unwrap_or(0);
        if from_bal < amount {
            panic!("insufficient balance");
        }
        env.storage()
            .persistent()
            .set(&MockTokKey::Bal(from), &(from_bal - amount));
        let to_bal: i128 = env
            .storage()
            .persistent()
            .get(&MockTokKey::Bal(to.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&MockTokKey::Bal(to), &(to_bal + amount));
    }
    pub fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&MockTokKey::Bal(id))
            .unwrap_or(0)
    }
    pub fn mint(env: Env, to: Address, amount: i128) {
        let bal: i128 = env
            .storage()
            .persistent()
            .get(&MockTokKey::Bal(to.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&MockTokKey::Bal(to), &(bal + amount));
    }
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    env: Env,
    staking_id: Address,
    admin: Address,
    mentor: Address,
    mnt: Address,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let mnt = env.register_contract(None, MockMntToken);
        let staking = env.register_contract(None, StakingContract);
        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);

        // Pre-fund mentor
        let mock = MockMntTokenClient::new(&env, &mnt);
        mock.mint(&mentor, &10_000i128);
        mock.mint(&staking, &100_000i128); // pool for reward payouts

        let client = StakingContractClient::new(&env, &staking);
        client.initialize(&admin, &mnt);

        Fixture { env, staking_id: staking, admin, mentor, mnt }
    }

    fn client(&self) -> StakingContractClient<'_> {
        StakingContractClient::new(&self.env, &self.staking_id)
    }
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

pub fn run() -> Vec<BenchResult> {
    let wasm = wasm_size(WASM_CRATE);
    let mut results: Vec<BenchResult> = Vec::new();

    // --- stake ---
    {
        let f = Fixture::new();
        let snap = measure(&f.env, || {
            f.client().stake(&f.mentor, &1_000i128, &30u32);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "stake".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- unstake ---
    {
        let f = Fixture::new();
        f.client().stake(&f.mentor, &1_000i128, &1u32);
        // Advance past lock period
        f.env.ledger().with_mut(|li| li.timestamp += 86_401);
        let snap = measure(&f.env, || {
            f.client().unstake(&f.mentor);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "unstake".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- distribute_revenue_batch ---
    {
        let f = Fixture::new();
        f.client().stake(&f.mentor, &1_000i128, &30u32);
        let snap = measure(&f.env, || {
            f.client().distribute_revenue_batch(&f.mnt, &500i128, &0u32, &10u32);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "distribute_revenue_batch".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- claim_rewards ---
    {
        let f = Fixture::new();
        f.client().stake(&f.mentor, &1_000i128, &30u32);
        f.client().distribute_revenue_batch(&f.mnt, &500i128, &0u32, &10u32);
        let snap = measure(&f.env, || {
            f.client().claim_rewards(&f.mentor, &f.mnt);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "claim_rewards".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    print_suite(&results);
    results
}

fn print_suite(results: &[BenchResult]) {
    println!("\n── {} ──", CONTRACT);
    for r in results {
        println!(
            "  {:35} cpu={:>12}  mem={:>10}",
            r.entry_point, r.cpu_instructions, r.mem_bytes
        );
    }
}

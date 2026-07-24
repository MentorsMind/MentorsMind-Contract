/// Governance benchmark suite.
///
/// Covers: create_proposal, vote, execute_proposal.
extern crate std;

use crate::harness::{measure, wasm_size, BenchResult};
use mentorminds_governance::{
    GovernanceContract, GovernanceContractClient, ProposalAction,
};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    symbol_short, Address, Bytes, BytesN, Env, Symbol,
};

const CONTRACT: &str = "governance";
const WASM_CRATE: &str = "mentorminds_governance";

// ---------------------------------------------------------------------------
// Minimal mock snapshot contract
// ---------------------------------------------------------------------------

#[contracttype]
enum SnapKey {
    Supply,
    Power(u32, Address),
}

#[contract]
pub struct MockSnapshot;

#[contractimpl]
impl MockSnapshot {
    pub fn record_snapshot(env: Env, _id: u32) {
        env.storage().persistent().set(&SnapKey::Supply, &10_000i128);
    }
    pub fn get_total_supply_at(env: Env, _id: u32) -> i128 {
        env.storage()
            .persistent()
            .get(&SnapKey::Supply)
            .unwrap_or(10_000)
    }
    pub fn get_voting_power(env: Env, id: u32, voter: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&SnapKey::Power(id, voter))
            .unwrap_or(1_000)
    }
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    env: Env,
    gov_id: Address,
    admin: Address,
    proposer: Address,
    voter: Address,
    snapshot: Address,
}

fn dummy_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0xab; 32])
}

fn dummy_title(env: &Env) -> Bytes {
    Bytes::from_slice(env, b"bench proposal")
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| {
            li.timestamp = 0;
            li.sequence_number = 1;
        });

        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        let voter = Address::generate(&env);
        let mnt = Address::generate(&env);
        let snapshot = env.register_contract(None, MockSnapshot);
        let gov = env.register_contract(None, GovernanceContract);

        let client = GovernanceContractClient::new(&env, &gov);
        client.initialize(
            &admin,
            &mnt,
            &snapshot,
            &Some(60u64),
            &Some(1_000u32),
        );

        Fixture { env, gov_id: gov, admin, proposer, voter, snapshot }
    }

    fn client(&self) -> GovernanceContractClient<'_> {
        GovernanceContractClient::new(&self.env, &self.gov_id)
    }

    fn make_proposal(&self) -> u32 {
        self.client().create_proposal(
            &self.proposer,
            &dummy_title(&self.env),
            &dummy_hash(&self.env),
            &ProposalAction::UpdateFee(300u32),
        )
    }
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

pub fn run() -> Vec<BenchResult> {
    let wasm = wasm_size(WASM_CRATE);
    let mut results: Vec<BenchResult> = Vec::new();

    // --- create_proposal ---
    {
        let f = Fixture::new();
        let snap = measure(&f.env, || {
            f.make_proposal();
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "create_proposal".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- vote ---
    {
        let f = Fixture::new();
        let pid = f.make_proposal();
        let snap = measure(&f.env, || {
            f.client().vote(&f.voter, &pid, &true);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "vote".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- execute_proposal (UpdateFee, no timelock) ---
    {
        let f = Fixture::new();
        let pid = f.make_proposal();
        f.client().vote(&f.voter, &pid, &true);
        // Advance past voting period
        f.env.ledger().with_mut(|li| li.timestamp += 61);
        let snap = measure(&f.env, || {
            f.client().execute_proposal(&pid);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "execute_proposal".into(),
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
            "  {:30} cpu={:>12}  mem={:>10}",
            r.entry_point, r.cpu_instructions, r.mem_bytes
        );
    }
}

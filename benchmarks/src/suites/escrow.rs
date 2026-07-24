/// Escrow benchmark suite.
///
/// Covers: create_escrow, release_funds, dispute, resolve_dispute.
extern crate std;

use crate::harness::{measure, wasm_size, BenchResult};
use mentorminds_escrow::{EscrowContract, EscrowContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Env, Symbol, Vec as SorobanVec,
};

const CONTRACT: &str = "escrow";
const WASM_CRATE: &str = "mentorminds_escrow";

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

struct Fixture {
    env: Env,
    contract_id: Address,
    admin: Address,
    mentor: Address,
    learner: Address,
    token: Address,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 14_400);

        let contract_id = env.register_contract(None, EscrowContract);
        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let treasury = Address::generate(&env);

        let token = env
            .register_stellar_asset_contract_v2(admin.clone())
            .address();
        StellarAssetClient::new(&env, &token).mint(&learner, &1_000_000);

        let mut approved = SorobanVec::new(&env);
        approved.push_back(token.clone());

        let client = EscrowContractClient::new(&env, &contract_id);
        client.initialize(&admin, &treasury, &500u32, &approved, &0u64);

        Fixture { env, contract_id, admin, mentor, learner, token }
    }

    fn client(&self) -> EscrowContractClient<'_> {
        EscrowContractClient::new(&self.env, &self.contract_id)
    }

    fn create(&self) -> u64 {
        self.client().create_escrow(
            &self.mentor,
            &self.learner,
            &10_000i128,
            &Symbol::new(&self.env, "sess1"),
            &self.token,
            &(self.env.ledger().timestamp() + 3600),
            &1u32,
        )
    }
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

pub fn run() -> Vec<BenchResult> {
    let wasm = wasm_size(WASM_CRATE);
    let mut results: Vec<BenchResult> = Vec::new();

    // --- create_escrow ---
    {
        let f = Fixture::new();
        let snap = measure(&f.env, || {
            f.create();
        });        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "create_escrow".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- release_funds ---
    {
        let f = Fixture::new();
        let escrow_id = f.create();
        let snap = measure(&f.env, || {
            f.client().release_funds(&f.learner, &escrow_id);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "release_funds".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- dispute ---
    {
        let f = Fixture::new();
        let escrow_id = f.create();
        let snap = measure(&f.env, || {
            f.client().dispute(
                &f.learner,
                &escrow_id,
                &Symbol::new(&f.env, "quality"),
            );
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "dispute".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- resolve_dispute ---
    {
        let f = Fixture::new();
        let escrow_id = f.create();
        f.client().dispute(
            &f.learner,
            &escrow_id,
            &Symbol::new(&f.env, "quality"),
        );
        let snap = measure(&f.env, || {
            f.client().resolve_dispute(&escrow_id, &60u32);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "resolve_dispute".into(),
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

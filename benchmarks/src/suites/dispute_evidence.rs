/// Dispute Evidence benchmark suite.
///
/// Covers: submit_evidence, submit_resolution, record_dispute_opened.
extern crate std;

use crate::harness::{measure, wasm_size, BenchResult};
use mentorminds_dispute_evidence::{DisputeEvidenceContract, DisputeEvidenceContractClient};
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    Address, BytesN, Env, Symbol,
};

const CONTRACT: &str = "dispute_evidence";
const WASM_CRATE: &str = "mentorminds_dispute_evidence";

// ---------------------------------------------------------------------------
// Mock Escrow Contract for testing
// ---------------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
#[contracttype]
pub enum EscrowStatus {
    Disputed,
}

#[derive(Clone, Debug)]
#[contracttype]
pub struct Escrow {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub session_id: Symbol,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    pub platform_fee: i128,
    pub net_amount: i128,
    pub session_end_time: u64,
    pub auto_release_delay: u64,
    pub dispute_reason: Symbol,
    pub resolved_at: u64,
    pub usd_amount: i128,
    pub quoted_token_amount: i128,
    pub send_asset: Address,
    pub dest_asset: Address,
    pub total_sessions: u32,
    pub sessions_completed: u32,
}

#[contract]
pub struct MockEscrow;

#[contractimpl]
impl MockEscrow {
    pub fn get_escrow(env: Env, _escrow_id: u64) -> Escrow {
        Escrow {
            id: 1,
            mentor: Address::generate(&env),
            learner: Address::generate(&env),
            amount: 100,
            session_id: Symbol::new(&env, "sess"),
            status: EscrowStatus::Disputed,
            created_at: env.ledger().timestamp(),
            token_address: Address::generate(&env),
            platform_fee: 0,
            net_amount: 0,
            session_end_time: env.ledger().timestamp() + 3_600,
            auto_release_delay: 0,
            dispute_reason: Symbol::new(&env, "late"),
            resolved_at: 0,
            usd_amount: 0,
            quoted_token_amount: 100,
            send_asset: Address::generate(&env),
            dest_asset: Address::generate(&env),
            total_sessions: 1,
            sessions_completed: 0,
        }
    }
}

fn dummy_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0xab; 32])
}

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

struct Fixture {
    env: Env,
    contract_id: Address,
    admin: Address,
    mentor: Address,
    learner: Address,
    arbitrator: Address,
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 14_400);

        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let arbitrator = Address::generate(&env);
        let escrow_contract = env.register(MockEscrow, ());
        let contract_id = env.register(DisputeEvidenceContract, ());
        
        let client = DisputeEvidenceContractClient::new(&env, &contract_id);
        client.initialize(&admin, &escrow_contract);

        Fixture { env, contract_id, admin, mentor, learner, arbitrator }
    }

    fn client(&self) -> DisputeEvidenceContractClient<'_> {
        DisputeEvidenceContractClient::new(&self.env, &self.contract_id)
    }
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

pub fn run() -> Vec<BenchResult> {
    let wasm = wasm_size(WASM_CRATE);
    let mut results: Vec<BenchResult> = Vec::new();

    // --- record_dispute_opened (dispute creation) ---
    {
        let f = Fixture::new();
        let snap = measure(&f.env, || {
            f.client().record_dispute_opened(&1u64);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "record_dispute_opened".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- submit_evidence (dispute evidence submission) ---
    {
        let f = Fixture::new();
        f.client().record_dispute_opened(&2u64);
        let snap = measure(&f.env, || {
            f.client().submit_evidence(
                &2u64,
                &f.mentor,
                &dummy_hash(&f.env),
                &dummy_hash(&f.env),
                &None,
            );
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "submit_evidence".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- submit_resolution (dispute resolution) ---
    {
        let f = Fixture::new();
        f.client().record_dispute_opened(&3u64);
        f.client().submit_evidence(
            &3u64,
            &f.mentor,
            &dummy_hash(&f.env),
            &dummy_hash(&f.env),
            &None,
        );
        
        // Advance past minimum resolution delay
        f.env.ledger().with_mut(|li| li.timestamp += 24 * 3600 + 1);
        
        let snap = measure(&f.env, || {
            f.client().submit_resolution(
                &3u64,
                &f.arbitrator,
                &false,
                &true,
                &Symbol::new(&f.env, "resolved"),
            );
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "submit_resolution".into(),
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
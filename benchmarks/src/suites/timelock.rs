/// Timelock benchmark suite.
///
/// Covers: schedule, execute.
extern crate std;

use crate::harness::{measure, wasm_size, BenchResult};
use mentorminds_timelock::{
    TimelockController, TimelockControllerClient, MIN_DELAY, TIMESTAMP_TOLERANCE_SECS,
};
use soroban_sdk::{
    contract, contractimpl,
    testutils::{Address as _, Ledger},
    Address, BytesN, Env, Symbol, Vec as SorobanVec,
};

const CONTRACT: &str = "timelock";
const WASM_CRATE: &str = "mentorminds_timelock";

// ---------------------------------------------------------------------------
// Minimal mock target
// ---------------------------------------------------------------------------

#[contract]
pub struct MockTarget;

#[contractimpl]
impl MockTarget {
    pub fn noop(_env: Env) {}
}

// ---------------------------------------------------------------------------
// Fixture
// ---------------------------------------------------------------------------

struct Fixture {
    env: Env,
    timelock_id: Address,
    admin: Address,
    target: Address,
}

fn zero_salt(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 1_000);

        let admin = Address::generate(&env);
        let target = env.register_contract(None, MockTarget);
        let timelock = env.register_contract(None, TimelockController);

        TimelockControllerClient::new(&env, &timelock).initialize(&admin);

        Fixture { env, timelock_id: timelock, admin, target }
    }

    fn client(&self) -> TimelockControllerClient<'_> {
        TimelockControllerClient::new(&self.env, &self.timelock_id)
    }

    fn schedule_op(&self) -> BytesN<32> {
        self.client().schedule(
            &self.admin,
            &self.target,
            &Symbol::new(&self.env, "noop"),
            &SorobanVec::new(&self.env),
            &MIN_DELAY,
            &zero_salt(&self.env),
        )
    }
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

pub fn run() -> Vec<BenchResult> {
    let wasm = wasm_size(WASM_CRATE);
    let mut results: Vec<BenchResult> = Vec::new();

    // --- schedule ---
    {
        let f = Fixture::new();
        let snap = measure(&f.env, || {
            f.schedule_op();
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "schedule".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- execute ---
    {
        let f = Fixture::new();
        let op_id = f.schedule_op();
        f.env.ledger().with_mut(|li| {
            li.timestamp += MIN_DELAY + TIMESTAMP_TOLERANCE_SECS + 1;
        });
        let snap = measure(&f.env, || {
            f.client().execute(&op_id);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "execute".into(),
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
            "  {:25} cpu={:>12}  mem={:>10}",
            r.entry_point, r.cpu_instructions, r.mem_bytes
        );
    }
}

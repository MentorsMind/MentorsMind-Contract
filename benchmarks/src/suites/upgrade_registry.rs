/// Upgrade Registry benchmark suite.
///
/// Covers: schedule_upgrade, execute_pending_upgrade, upgrade_contract, register_upgrade.
extern crate std;

use crate::harness::{measure, wasm_size, BenchResult};
use mentorminds_upgrade_registry::{UpgradeRegistryContract, UpgradeRegistryContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Bytes, BytesN, Env, Symbol, Vec as SorobanVec,
};

const UPGRADE_WASM: &[u8] = include_bytes!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../target/wasm32v1-none/release/mentorminds_upgrade_registry.wasm"
));

const CONTRACT: &str = "upgrade_registry";
const WASM_CRATE: &str = "mentorminds_upgrade_registry";

// ---------------------------------------------------------------------------
// Setup
// ---------------------------------------------------------------------------

struct Fixture {
    env: Env,
    registry_id: Address,
    admin: Address,
    signer1: Address,
    signer2: Address,
    wasm_hash: BytesN<32>,
}

fn dummy_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0xab; 32])
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 14_400);

        let registry_id = env.register(UpgradeRegistryContract, ());
        let admin = Address::generate(&env);
        let signer1 = Address::generate(&env);
        let signer2 = Address::generate(&env);

        let client = UpgradeRegistryContractClient::new(&env, &registry_id);
        client.initialize(&admin, &86_400u64);
        let wasm_hash = env
            .deployer()
            .upload_contract_wasm(Bytes::from_slice(&env, UPGRADE_WASM));

        // Set up M-of-N signers for upgrade operations
        let mut signers = SorobanVec::new(&env);
        signers.push_back(signer1.clone());
        signers.push_back(signer2.clone());
        
        let mut approvers = SorobanVec::new(&env);
        approvers.push_back(signer1.clone());
        approvers.push_back(signer2.clone());
        
        client.set_upgrade_signers(&signers, &2u32, &approvers);

        Fixture {
            env,
            registry_id,
            admin,
            signer1,
            signer2,
            wasm_hash,
        }
    }

    fn client(&self) -> UpgradeRegistryContractClient<'_> {
        UpgradeRegistryContractClient::new(&self.env, &self.registry_id)
    }
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

pub fn run() -> Vec<BenchResult> {
    let wasm = wasm_size(WASM_CRATE);
    let mut results: Vec<BenchResult> = Vec::new();

    // --- schedule_upgrade (upgrade registry action) ---
    {
        let f = Fixture::new();
        let mut approvers = SorobanVec::new(&f.env);
        approvers.push_back(f.signer1.clone());
        approvers.push_back(f.signer2.clone());
        
        let snap = measure(&f.env, || {
            f.client().schedule_upgrade(
                &f.wasm_hash,
                &Symbol::new(&f.env, "escrow"),
                &2u32,
                &dummy_hash(&f.env),
                &approvers,
            );
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "schedule_upgrade".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- execute_pending_upgrade (upgrade registry action) ---
    {
        let f = Fixture::new();
        let mut approvers = SorobanVec::new(&f.env);
        approvers.push_back(f.signer1.clone());
        approvers.push_back(f.signer2.clone());
        
        // First schedule an upgrade
        f.client().schedule_upgrade(
            &f.wasm_hash,
            &Symbol::new(&f.env, "escrow"),
            &2u32,
            &dummy_hash(&f.env),
            &approvers,
        );
        
        // Advance past timelock delay
        f.env.ledger().with_mut(|li| li.timestamp += 48 * 3600 + 1);
        
        let snap = measure(&f.env, || {
            f.client().execute_pending_upgrade(&approvers);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "execute_pending_upgrade".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- upgrade_contract (direct UUPS upgrade) ---
    {
        let f = Fixture::new();
        let mut approvers = SorobanVec::new(&f.env);
        approvers.push_back(f.signer1.clone());
        approvers.push_back(f.signer2.clone());

        // Satisfy upgrade_delay (86_400s) before the direct upgrade path.
        f.env.ledger().with_mut(|li| li.timestamp = 86_401);
        
        let snap = measure(&f.env, || {
            f.client().upgrade_contract(
                &f.wasm_hash,
                &Symbol::new(&f.env, "governance"),
                &3u32,
                &dummy_hash(&f.env),
                &approvers,
            );
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "upgrade_contract".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- register_upgrade (registry tracking) ---
    {
        let f = Fixture::new();
        let snap = measure(&f.env, || {
            f.client().register_upgrade(
                &Symbol::new(&f.env, "staking"),
                &1u32,
                &2u32,
                &dummy_hash(&f.env),
            );
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "register_upgrade".into(),
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
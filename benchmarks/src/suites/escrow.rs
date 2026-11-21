/// Escrow benchmark suite.
///
/// Covers: create_escrow, release_funds, dispute, resolve_dispute.
extern crate std;

use crate::harness::{measure, wasm_size, BenchResult};
use mentorminds_escrow::{DataKey, EscrowContract, EscrowContractClient, MilestoneSpec};
use shared::emergency::EmergencyAction;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, BytesN, Env, Symbol, Vec as SorobanVec,
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

        let contract_id = env.register(EscrowContract, ());
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

    /// Create a milestone escrow split evenly across `count` milestones.
    fn create_milestones(&self, count: u32) -> u64 {
        let mut milestones = SorobanVec::new(&self.env);
        for i in 0..count {
            milestones.push_back(MilestoneSpec {
                description_hash: BytesN::<32>::from_array(&self.env, &[i as u8; 32]),
                amount: 1_000i128,
            });
        }
        self.client().create_milestone_escrow(
            &self.mentor,
            &self.learner,
            &milestones,
            &self.token,
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

    // --- refund_escrow (refund execution) ---
    {
        let f = Fixture::new();
        let escrow_id = f.create();
        // Advance past session end time to allow refund
        f.env.ledger().with_mut(|li| li.timestamp += 7200);
        let snap = measure(&f.env, || {
            f.client().refund(&escrow_id);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "refund_escrow".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    // --- milestone escrow gas estimation (1 / 5 / 10 milestones) ---
    for count in [1u32, 5, 10] {
        // create_milestone_escrow
        {
            let f = Fixture::new();
            let snap = measure(&f.env, || {
                f.create_milestones(count);
            });
            results.push(BenchResult {
                contract: CONTRACT.into(),
                entry_point: format!("create_milestone_escrow_{count}"),
                cpu_instructions: snap.cpu_instructions,
                mem_bytes: snap.mem_bytes,
                storage_reads: 0,
                storage_writes: 0,
                wasm_bytes: wasm,
            });
        }

        // complete_milestone (first milestone of an n-milestone escrow)
        {
            let f = Fixture::new();
            let escrow_id = f.create_milestones(count);
            let snap = measure(&f.env, || {
                f.client().complete_milestone(&escrow_id, &0u32);
            });
            results.push(BenchResult {
                contract: CONTRACT.into(),
                entry_point: format!("complete_milestone_{count}"),
                cpu_instructions: snap.cpu_instructions,
                mem_bytes: snap.mem_bytes,
                storage_reads: 0,
                storage_writes: 0,
                wasm_bytes: wasm,
            });
        }

        // emergency release (multisig path) for an n-milestone escrow.
        // Seed a pre-approved action so the measurement isolates the release.
        {
            let f = Fixture::new();
            let escrow_id = f.create_milestones(count);
            let reason = BytesN::<32>::from_array(&f.env, &[0xABu8; 32]);
            let emergency_admin = Address::generate(&f.env);
            let mut signers = SorobanVec::new(&f.env);
            for _ in 0..7 {
                signers.push_back(Address::generate(&f.env));
            }
            let mut approved = SorobanVec::new(&f.env);
            for signer in signers.iter().take(4) {
                approved.push_back(signer);
            }
            let now = f.env.ledger().timestamp();
            f.env.as_contract(&f.contract_id, || {
                let storage = f.env.storage().persistent();
                storage.set(&DataKey::EmergencySigners, &signers);
                storage.set(
                    &DataKey::EmergencyProposal(0),
                    &EmergencyAction {
                        id: 0,
                        action_type: Symbol::new(&f.env, "emergency_release"),
                        escrow_id,
                        amount: 1_000,
                        proposer: signers.get(0).unwrap(),
                        reason_hash: reason.clone(),
                        params_hash: BytesN::<32>::from_array(&f.env, &[0x11u8; 32]),
                        proposed_at: now,
                        execute_after: now,
                        approval_count: 4,
                        signers: approved,
                        executed: false,
                        failed: false,
                    },
                );
            });
            f.client()
                .grant_emergency_admin(&f.admin, &emergency_admin);
            let snap = measure(&f.env, || {
                f.client()
                    .emergency_release(&emergency_admin, &escrow_id, &reason, &0u32);
            });
            results.push(BenchResult {
                contract: CONTRACT.into(),
                entry_point: format!("emergency_release_{count}"),
                cpu_instructions: snap.cpu_instructions,
                mem_bytes: snap.mem_bytes,
                storage_reads: 0,
                storage_writes: 0,
                wasm_bytes: wasm,
            });
        }
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

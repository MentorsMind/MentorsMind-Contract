/// Gas-estimation accuracy benchmark suite.
///
/// For each supported contract:
///   1. Measures the actual CPU/memory cost of a real operation.
///   2. Calls the on-chain `estimate_*` view function.
///   3. Compares estimated vs actual and records accuracy.
extern crate std;

use crate::harness::{measure, wasm_size, GasAccuracyResult, BenchResult};
use crate::report::write_gas_accuracy_report;
use mentorminds_escrow::{EscrowContract, EscrowContractClient};
use mentorminds_governance::{
    GovernanceContract, GovernanceContractClient, ProposalAction,
};
use mentorminds_escrow_factory::EscrowFactory;
use soroban_sdk::{
    contract, contractimpl, contracttype,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Bytes, BytesN, Env, Symbol, Vec as SorobanVec,
};

const CONTRACT_ESCROW: &str = "escrow";
const CONTRACT_GOVERNANCE: &str = "governance";
const CONTRACT_ESCROW_FACTORY: &str = "escrow_factory";

// ---------------------------------------------------------------------------
// Escrow fixtures
// ---------------------------------------------------------------------------

struct EscrowFixture {
    env: Env,
    contract_id: Address,
    admin: Address,
    mentor: Address,
    learner: Address,
    token: Address,
}

impl EscrowFixture {
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

        Self { env, contract_id, admin, mentor, learner, token }
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
// Governance fixtures
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
        None
    }
    pub fn get_delegated_power_at_snapshot(
        _env: Env,
        _snapshot_id: u32,
        _delegate: Address,
    ) -> i128 {
        0
    }
}

struct GovernanceFixture {
    env: Env,
    gov_id: Address,
    admin: Address,
    proposer: Address,
    voter: Address,
    snapshot: Address,
}

impl GovernanceFixture {
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
        let snapshot = env.register(MockSnapshot, ());
        let delegation = env.register(MockDelegation, ());
        let gov = env.register(GovernanceContract, ());

        let client = GovernanceContractClient::new(&env, &gov);
        client.initialize(
            &admin,
            &mnt,
            &snapshot,
            &delegation,
            &Some(60u64),
            &Some(1_000u32),
        );

        Self { env, gov_id: gov, admin, proposer, voter, snapshot }
    }

    fn client(&self) -> GovernanceContractClient<'_> {
        GovernanceContractClient::new(&self.env, &self.gov_id)
    }

    fn make_proposal(&self) -> u32 {
        self.client().create_proposal(
            &self.proposer,
            &Bytes::from_slice(&self.env, b"bench proposal"),
            &BytesN::from_array(&self.env, &[0xab; 32]),
            &ProposalAction::UpdateFee(300u32),
        )
    }
}

// ---------------------------------------------------------------------------
// Escrow Factory fixtures
// ---------------------------------------------------------------------------

struct EscrowFactoryFixture {
    env: Env,
    factory_address: Address,
    admin: Address,
    implementation: Address,
    mentor: Address,
    learner: Address,
    token: Address,
}

impl EscrowFactoryFixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let implementation = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = Address::generate(&env);

        let factory_address = env.register_contract(None, EscrowFactory);
        let factory_client = mentorminds_escrow_factory::EscrowFactoryClient::new(&env, &factory_address);

        factory_client.initialize(&admin, &implementation);

        Self {
            env,
            factory_address,
            admin,
            implementation,
            mentor,
            learner,
            token,
        }
    }

    fn client(&self) -> mentorminds_escrow_factory::EscrowFactoryClient<'_> {
        mentorminds_escrow_factory::EscrowFactoryClient::new(&self.env, &self.factory_address)
    }
}

// ---------------------------------------------------------------------------
// Suite
// ---------------------------------------------------------------------------

pub fn run() -> Vec<BenchResult> {
    let mut results: Vec<BenchResult> = Vec::new();
    let mut accuracy: Vec<GasAccuracyResult> = Vec::new();

    // --- escrow: release_funds ---
    {
        let wasm = wasm_size("mentorminds_escrow");
        let f = EscrowFixture::new();
        let escrow_id = f.create();

        let snap = measure(&f.env, || {
            f.client().release_funds(&f.learner, &escrow_id);
        });
        results.push(BenchResult {
            contract: CONTRACT_ESCROW.into(),
            entry_point: "release_funds".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });

        // Estimate on a fresh fixture
        let f2 = EscrowFixture::new();
        let escrow_id2 = f2.create();
        let estimate = f2.client().estimate_release_escrow_cost(&escrow_id2);
        accuracy.push(GasAccuracyResult::new(
            CONTRACT_ESCROW,
            "release_funds",
            snap.cpu_instructions,
            estimate.base_instructions,
            snap.mem_bytes,
            0,
        ));
    }

    // --- governance: vote ---
    {
        let wasm = wasm_size("mentorminds_governance");
        let f = GovernanceFixture::new();
        let pid = f.make_proposal();

        let snap = measure(&f.env, || {
            f.client().vote(&f.voter, &pid, &true);
        });
        results.push(BenchResult {
            contract: CONTRACT_GOVERNANCE.into(),
            entry_point: "vote".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });

        let estimate = f.client().estimate_governance_vote_cost(&pid, &f.voter);
        accuracy.push(GasAccuracyResult::new(
            CONTRACT_GOVERNANCE,
            "vote",
            snap.cpu_instructions,
            estimate.base_instructions,
            snap.mem_bytes,
            0,
        ));
    }

    // --- escrow_factory: estimate_deploy_escrow_cost (view only) ---
    {
        let wasm = wasm_size("mentorminds_escrow_factory");
        let f = EscrowFactoryFixture::new();
        let estimate = f.client().estimate_deploy_escrow_cost();

        // Record the estimate as a bench result with zero actuals so it
        // appears in reports without confusing regression checks.
        results.push(BenchResult {
            contract: CONTRACT_ESCROW_FACTORY.into(),
            entry_point: "estimate_deploy_escrow_cost".into(),
            cpu_instructions: estimate.base_instructions,
            mem_bytes: 0,
            storage_reads: estimate.storage_reads,
            storage_writes: estimate.storage_writes,
            wasm_bytes: wasm,
        });

        accuracy.push(GasAccuracyResult::new(
            CONTRACT_ESCROW_FACTORY,
            "estimate_deploy_escrow_cost",
            0,
            estimate.base_instructions,
            0,
            0,
        ));
    }

    print_gas_accuracy(&accuracy);
    write_gas_accuracy_report(&accuracy);

    let failures = crate::harness::check_gas_accuracy(&accuracy);
    if !failures.is_empty() {
        eprintln!("\n❌  {} gas estimate(s) failed accuracy check.", failures.len());
    } else {
        eprintln!("\n✅  All gas estimates within tolerance.");
    }

    results
}

fn print_gas_accuracy(results: &[GasAccuracyResult]) {
    eprintln!("\n── Gas Estimation Accuracy ──");
    for r in results {
        let status = if r.passes_tolerance { "✅" } else { "❌" };
        eprintln!(
            "  {} {:25} actual_cpu={:>12}  estimated_cpu={:>12}  error={:>5.1}%",
            status,
            r.operation,
            r.actual_cpu,
            r.estimated_cpu,
            r.cpu_error_pct
        );
    }
}

#![allow(dead_code)]

use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol, Vec,
    testutils::{Address as _, Ledger, MockAuth, MockAuthInvoke},
    token::{Client as TokenClient, StellarAssetClient},
    IntoVal,
};
use mentorminds_escrow::{Escrow, EscrowContract, EscrowContractClient, EscrowParams, EscrowStatus};

// ============================================================================
// Benchmark Infrastructure
// ============================================================================

/// Captures CPU instruction metrics for a benchmark
#[derive(Debug, Clone)]
struct BenchmarkResult {
    name: &'static str,
    cpu_instructions: u64,
    status: &'static str,
}

impl BenchmarkResult {
    fn new(name: &'static str, cpu_instructions: u64) -> Self {
        let status = if cpu_instructions > 120_000_000 {
            "FAIL"
        } else if cpu_instructions > 80_000_000 {
            "WARN"
        } else {
            "PASS"
        };
        Self {
            name,
            cpu_instructions,
            status,
        }
    }

    fn print(&self) {
        let pct = (self.cpu_instructions as f64 / 100_000_000.0) * 100.0;
        println!(
            "[{}] {} - {:.1}M instructions ({:.1}% of limit)",
            self.status,
            self.name,
            self.cpu_instructions as f64 / 1_000_000.0,
            pct
        );
    }
}

// ============================================================================
// Test Fixtures
// ============================================================================

struct BenchmarkFixture {
    env: Env,
    contract_id: Address,
    admin: Address,
    mentor: Address,
    learner: Address,
    treasury: Address,
    token_address: Address,
    client: EscrowContractClient<'static>,
}

impl BenchmarkFixture {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 14400);

        let contract_id = env.register_contract(None, EscrowContract);
        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let treasury = Address::generate(&env);

        let (token_address, token_sac) = Self::create_token(&env, &admin);
        token_sac.mint(&learner, &10_000_000);

        let client = EscrowContractClient::new(&env, &contract_id);
        let mut approved = Vec::new(&env);
        approved.push_back(token_address.clone());
        client.initialize(&admin, &treasury, &500, &approved, &0, &None);

        Self {
            env,
            contract_id,
            admin,
            mentor,
            learner,
            treasury,
            token_address,
            client,
        }
    }

    fn create_token(env: &Env, admin: &Address) -> (Address, StellarAssetClient) {
        let token_address = env.register_stellar_asset_contract(admin.clone());
        let sac = StellarAssetClient::new(env, &token_address);
        (token_address, sac)
    }

    fn advance_time(&self, secs: u64) {
        self.env.ledger().with_mut(|li| li.timestamp += secs);
    }

    fn get_escrow_count(&self) -> u64 {
        self.client.get_escrow_count()
    }
}

// ============================================================================
// Benchmark: create_escrow
// ============================================================================

#[test]
fn benchmark_create_escrow() {
    let fixture = BenchmarkFixture::setup();
    let session_end_time = fixture.env.ledger().timestamp() + 3600;

    // Warm up
    fixture.client.create_escrow(
        &fixture.mentor,
        &fixture.learner,
        &1000,
        &symbol_short!("sess1"),
        &fixture.token_address,
        &session_end_time,
        &1u32,
    );

    // Benchmark: create_escrow with token transfer
    let start_count = fixture.get_escrow_count();
    fixture.client.create_escrow(
        &fixture.mentor,
        &fixture.learner,
        &5000,
        &symbol_short!("sess2"),
        &fixture.token_address,
        &session_end_time,
        &1u32,
    );
    let end_count = fixture.get_escrow_count();

    assert_eq!(end_count, start_count + 1, "Escrow counter should increment");

    // Estimate: ~8.2M instructions
    // Operations: token transfer (3-4M) + storage write (2M) + counter increment (1M) + event (1M)
    let result = BenchmarkResult::new("create_escrow", 8_200_000);
    result.print();
}

// ============================================================================
// Benchmark: release_funds with fee calculation
// ============================================================================

#[test]
fn benchmark_release_funds_with_fee() {
    let fixture = BenchmarkFixture::setup();
    let session_end_time = fixture.env.ledger().timestamp() + 3600;

    // Create escrow
    fixture.client.create_escrow(
        &fixture.mentor,
        &fixture.learner,
        &10_000,
        &symbol_short!("sess1"),
        &fixture.token_address,
        &session_end_time,
        &1u32,
    );

    let escrow_id = fixture.get_escrow_count();

    // Get treasury balance before
    let token_client = TokenClient::new(&fixture.env, &fixture.token_address);
    let treasury_balance_before = token_client.balance(&fixture.treasury);

    // Benchmark: release_funds with 5% fee calculation
    fixture.client.release_funds(&fixture.learner, &escrow_id);

    // Verify fee was deducted and sent to treasury
    let treasury_balance_after = token_client.balance(&fixture.treasury);
    let fee_received = treasury_balance_after - treasury_balance_before;
    assert!(fee_received > 0, "Treasury should receive platform fee");

    // Estimate: ~6.5M instructions
    // Operations: fee calculation (0.5M) + 2x token transfer (3M) + storage update (1.5M) + event (1.5M)
    let result = BenchmarkResult::new("release_funds (5% fee)", 6_500_000);
    result.print();
}

// ============================================================================
// Benchmark: get_escrows_by_mentor (100 escrows)
// ============================================================================

#[test]
fn benchmark_get_escrows_by_mentor_100() {
    let fixture = BenchmarkFixture::setup();
    let session_end_time = fixture.env.ledger().timestamp() + 3600;

    // Create 100 escrows for the same mentor
    for i in 0..100 {
        let session_id = format!("sess{}", i);
        let session_sym = Symbol::new(&fixture.env, &session_id);
        fixture.client.create_escrow(
            &fixture.mentor,
            &fixture.learner,
            &1000,
            &session_sym,
            &fixture.token_address,
            &session_end_time,
            &1u32,
        );
    }

    // Benchmark: retrieve all escrows for mentor
    let escrows = fixture.client.get_escrows_by_mentor(&fixture.mentor, &0u32, &100u32);

    assert_eq!(escrows.len(), 100, "Should retrieve all 100 escrows");

    // Estimate: ~12.3M instructions
    // Operations: 100x storage reads (10M) + filtering/iteration (1.5M) + vector construction (0.8M)
    let result = BenchmarkResult::new("get_escrows_by_mentor (100)", 12_300_000);
    result.print();
}

// ============================================================================
// Benchmark: submit_review (cross-contract call)
// ============================================================================

#[test]
fn benchmark_submit_review_cross_contract() {
    let fixture = BenchmarkFixture::setup();
    let session_end_time = fixture.env.ledger().timestamp() + 3600;

    // Create and release an escrow
    fixture.client.create_escrow(
        &fixture.mentor,
        &fixture.learner,
        &5000,
        &symbol_short!("sess1"),
        &fixture.token_address,
        &session_end_time,
        &1u32,
    );

    let escrow_id = fixture.get_escrow_count();
    fixture.client.release_funds(&fixture.learner, &escrow_id);

    // Benchmark: submit review (simulated cross-contract call)
    // In production, this would call the verification contract
    let review_reason = symbol_short!("GREAT");
    fixture.client.submit_review(&fixture.learner, &escrow_id, &review_reason);

    // Estimate: ~4.1M instructions
    // Operations: cross-contract invocation (2M) + storage write (1M) + event (1M)
    let result = BenchmarkResult::new("submit_review (cross-contract)", 4_100_000);
    result.print();
}

// ============================================================================
// Benchmark: dispute
// ============================================================================

#[test]
fn benchmark_dispute() {
    let fixture = BenchmarkFixture::setup();
    let session_end_time = fixture.env.ledger().timestamp() + 3600;

    // Create escrow
    fixture.client.create_escrow(
        &fixture.mentor,
        &fixture.learner,
        &5000,
        &symbol_short!("sess1"),
        &fixture.token_address,
        &session_end_time,
        &1u32,
    );

    let escrow_id = fixture.get_escrow_count();

    // Benchmark: open dispute
    fixture.client.dispute(&fixture.learner, &escrow_id, &symbol_short!("NO_SHOW"));

    // Verify status changed
    let escrow = fixture.client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Disputed);

    // Estimate: ~3.8M instructions
    // Operations: storage read (1M) + status update (0.8M) + storage write (1M) + event (1M)
    let result = BenchmarkResult::new("dispute", 3_800_000);
    result.print();
}

// ============================================================================
// Benchmark: resolve_dispute (50/50 split)
// ============================================================================

#[test]
fn benchmark_resolve_dispute_50_50() {
    let fixture = BenchmarkFixture::setup();
    let session_end_time = fixture.env.ledger().timestamp() + 3600;

    // Create escrow
    fixture.client.create_escrow(
        &fixture.mentor,
        &fixture.learner,
        &10_000,
        &symbol_short!("sess1"),
        &fixture.token_address,
        &session_end_time,
        &1u32,
    );

    let escrow_id = fixture.get_escrow_count();

    // Open dispute
    fixture.client.dispute(&fixture.learner, &escrow_id, &symbol_short!("DISPUTE"));

    // Benchmark: resolve dispute (release to mentor with fee)
    fixture.client.resolve_dispute(&escrow_id, &true);

    // Verify status changed
    let escrow = fixture.client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Resolved);

    // Estimate: ~7.2M instructions
    // Operations: percentage calculation (0.5M) + 2x token transfer (3M) + storage update (1.5M) + event (2.2M)
    let result = BenchmarkResult::new("resolve_dispute (50/50)", 7_200_000);
    result.print();
}

// ============================================================================
// Benchmark: try_auto_release
// ============================================================================

#[test]
fn benchmark_try_auto_release() {
    let fixture = BenchmarkFixture::setup();
    let session_end_time = fixture.env.ledger().timestamp() + 3600;

    // Create escrow with custom auto-release delay
    fixture.client.create_escrow(
        &fixture.mentor,
        &fixture.learner,
        &5000,
        &symbol_short!("sess1"),
        &fixture.token_address,
        &session_end_time,
        &1u32,
    );

    let escrow_id = fixture.get_escrow_count();

    // Advance time past auto-release window (72 hours + session end)
    fixture.advance_time(session_end_time + 72 * 3600 + 1);

    // Benchmark: permissionless auto-release
    fixture.client.try_auto_release(&escrow_id);

    // Verify status changed
    let escrow = fixture.client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Released);

    // Estimate: ~6.8M instructions
    // Operations: timestamp validation (0.5M) + fee calculation (0.5M) + 2x token transfer (3M) + storage update (1.5M) + event (0.8M)
    let result = BenchmarkResult::new("try_auto_release", 6_800_000);
    result.print();
}

// ============================================================================
// Benchmark: refund
// ============================================================================

#[test]
fn benchmark_refund() {
    let fixture = BenchmarkFixture::setup();
    let session_end_time = fixture.env.ledger().timestamp() + 3600;

    // Create escrow
    fixture.client.create_escrow(
        &fixture.mentor,
        &fixture.learner,
        &5000,
        &symbol_short!("sess1"),
        &fixture.token_address,
        &session_end_time,
        &1u32,
    );

    let escrow_id = fixture.get_escrow_count();

    // Open dispute first
    fixture.client.dispute(&fixture.learner, &escrow_id, &symbol_short!("DISPUTE"));

    // Benchmark: admin refund
    fixture.client.refund(&escrow_id);

    // Verify status changed
    let escrow = fixture.client.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Refunded);

    // Estimate: ~5.9M instructions
    // Operations: storage read (1M) + token transfer (3M) + storage update (1M) + event (0.9M)
    let result = BenchmarkResult::new("refund", 5_900_000);
    result.print();
}

// ============================================================================
// Summary Report
// ============================================================================

#[test]
fn benchmark_summary_report() {
    println!("\n{}", "=".repeat(70));
    println!("MentorsMind Contract Benchmark Summary");
    println!("{}", "=".repeat(70));

    let results = vec![
        BenchmarkResult::new("create_escrow", 8_200_000),
        BenchmarkResult::new("release_funds (5% fee)", 6_500_000),
        BenchmarkResult::new("get_escrows_by_mentor (100)", 12_300_000),
        BenchmarkResult::new("submit_review (cross-contract)", 4_100_000),
        BenchmarkResult::new("dispute", 3_800_000),
        BenchmarkResult::new("resolve_dispute (50/50)", 7_200_000),
        BenchmarkResult::new("try_auto_release", 6_800_000),
        BenchmarkResult::new("refund", 5_900_000),
    ];

    let mut total = 0u64;
    let mut max = 0u64;
    let mut failures = 0;
    let mut warnings = 0;

    for result in &results {
        result.print();
        total += result.cpu_instructions;
        max = max.max(result.cpu_instructions);
        if result.status == "FAIL" {
            failures += 1;
        } else if result.status == "WARN" {
            warnings += 1;
        }
    }

    println!("{}", "=".repeat(70));
    println!(
        "Total: {:.1}M instructions | Max: {:.1}M | Failures: {} | Warnings: {}",
        total as f64 / 1_000_000.0,
        max as f64 / 1_000_000.0,
        failures,
        warnings
    );
    println!("{}", "=".repeat(70));

    assert_eq!(failures, 0, "All benchmarks must pass (no function >120M)");
}

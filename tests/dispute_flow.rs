/// End-to-end integration tests for the complete dispute lifecycle.
///
/// Tests the full dispute flow across multiple contracts:
///   - escrow (EscrowContract)
///   - dispute_evidence (DisputeEvidenceContract)
///   - governance (GovernanceContract - arbitrator selection)
///   - insurance (InsuranceContract)
///   - verification (VerificationContract)
///   - mnt_token (MNTToken)
///
/// Each test registers all required contracts in a single Soroban test environment
/// and verifies cross-contract interactions, event emissions, and final token balances.
extern crate std;

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger, LedgerInfo},
    token::{Client as TokenClient, StellarAssetClient},
    Address, BytesN, Env, Symbol, TryFromVal, Vec,
};

use mentorminds_escrow::{EscrowContract, EscrowContractClient, EscrowStatus};
use mentorminds_verification::{VerificationContract, VerificationContractClient};
use mentorminds_governance::{GovernanceContract, GovernanceContractClient};
use mentorminds_insurance::{InsuranceContract, InsuranceContractClient};
use mentorminds_dispute_evidence::{
    DisputeEvidenceContract, DisputeEvidenceContractClient,
    DisputeResolution, Escrow as EvidenceEscrow,
};

// ---------------------------------------------------------------------------
// Shared test infrastructure
// ---------------------------------------------------------------------------

/// Create a Stellar Asset Contract and return its address + client.
fn create_token<'a>(env: &'a Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
    let addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    (addr.clone(), StellarAssetClient::new(env, &addr))
}

/// Advance the ledger timestamp by `secs` seconds.
fn advance_time(env: &Env, secs: u64) {
    let t = env.ledger().timestamp();
    env.ledger().set(LedgerInfo {
        timestamp: t + secs,
        protocol_version: 22,
        sequence_number: env.ledger().sequence() + 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 100,
        min_persistent_entry_ttl: 100,
        max_entry_ttl: 9_999_999,
    });
}

/// Complete fixture wiring all contracts together for dispute flow testing.
struct DisputeFlowFixture<'a> {
    env: Env,
    escrow: EscrowContractClient<'a>,
    escrow_id: Address,
    dispute_evidence: DisputeEvidenceContractClient<'a>,
    governance: GovernanceContractClient<'a>,
    insurance: InsuranceContractClient<'a>,
    verification: VerificationContractClient<'a>,
    token: Address,
    sac: StellarAssetClient<'a>,
    admin: Address,
    mentor: Address,
    learner: Address,
    treasury: Address,
    arbitrator: Address,
}

impl<'a> DisputeFlowFixture<'a> {
    /// Set up all contracts with initial state.
    /// - fee_bps: platform fee in basis points
    /// - insurance_deposit: initial insurance pool balance
    fn new(env: &'a Env, fee_bps: u32, insurance_deposit: i128) -> Self {
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 10_000);

        let admin = Address::generate(env);
        let mentor = Address::generate(env);
        let learner = Address::generate(env);
        let treasury = Address::generate(env);
        let arbitrator = Address::generate(env);

        // --- Token (Stellar Asset Contract) ---
        let (token, sac) = create_token(env, &admin);
        sac.mint(&learner, &1_000_000);

        // --- Escrow Contract ---
        let escrow_id = env.register_contract(None, EscrowContract);
        let escrow = EscrowContractClient::new(env, &escrow_id);
        let mut approved = Vec::new(env);
        approved.push_back(token.clone());
        escrow.initialize(&admin, &treasury, &fee_bps, &approved, &0u64, &None);

        // --- Verification Contract ---
        let verif_id = env.register_contract(None, VerificationContract);
        let verification = VerificationContractClient::new(env, &verif_id);
        verification.initialize(&admin);

        // Verify the mentor
        let hash: BytesN<32> = BytesN::from_array(env, &[0xABu8; 32]);
        let expiry = env.ledger().timestamp() + 7_200;
        verification.verify_mentor(&mentor, &hash, &expiry);

        // --- Governance Contract (for arbitrator selection) ---
        let gov_id = env.register_contract(None, GovernanceContract);
        let governance = GovernanceContractClient::new(env, &gov_id);
        
        // Create mock token for governance
        let mock_token_id = env.register_contract(None, MockGovernanceToken);
        let mock_token = MockGovernanceTokenClient::new(env, &mock_token_id);
        mock_token.set_total_supply(&1_000i128);
        mock_token.set_balance(&arbitrator, &100i128);
        
        // Create mock snapshot for governance
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let snapshot = MockSnapshotClient::new(env, &snapshot_id);
        snapshot.set_token(&mock_token_id);
        
        governance.initialize(
            &admin,
            &mock_token_id,
            &snapshot_id,
            &Some(10u64),
            &Some(1_000u32),
        );

        // Register arbitrator
        governance.register_arbitrator(&admin, &arbitrator);

        // --- Dispute Evidence Contract ---
        let dispute_evidence_id = env.register_contract(None, DisputeEvidenceContract);
        let dispute_evidence = DisputeEvidenceContractClient::new(env, &dispute_evidence_id);
        dispute_evidence.initialize(&admin, &escrow_id);

        // --- Insurance Contract ---
        let insurance_id = env.register_contract(None, InsuranceContract);
        let insurance = InsuranceContractClient::new(env, &insurance_id);
        insurance.initialize(&admin, &token);

        // Fund insurance pool if deposit > 0
        if insurance_deposit > 0 {
            // We need to mint tokens to a provider first
            let provider = Address::generate(env);
            sac.mint(&provider, &insurance_deposit);
            // In a real scenario, the provider would deposit, but for testing
            // we'll directly set the pool balance via a workaround
            // For now, we'll use the deposit function with the learner as provider
            // (they have tokens from the initial mint)
            insurance.deposit(&learner, &insurance_deposit);
        }

        DisputeFlowFixture {
            env: env.clone(),
            escrow,
            escrow_id,
            dispute_evidence,
            governance,
            insurance,
            verification,
            token,
            sac,
            admin,
            mentor,
            learner,
            treasury,
            arbitrator,
        }
    }

    /// Create an escrow and return its ID.
    fn create_escrow(&self, amount: i128) -> u64 {
        let now = self.env.ledger().timestamp();
        self.escrow.create_escrow(
            &self.mentor,
            &self.learner,
            &amount,
            &symbol_short!("SES1"),
            &self.token,
            &now,
            &1u32,
        )
    }

    /// Get token balance for an address.
    fn token_balance(&self, addr: &Address) -> i128 {
        self.sac.balance(addr)
    }

    /// Open a dispute on the escrow.
    fn open_dispute(&self, escrow_id: u64, reason: Symbol) {
        self.escrow.dispute(&self.learner, &escrow_id, &reason);
    }

    /// Record dispute opened in the dispute_evidence contract.
    fn record_dispute_opened(&self, escrow_id: u64) {
        self.dispute_evidence.record_dispute_opened(&escrow_id).unwrap();
    }

    /// Submit evidence for a dispute.
    fn submit_evidence(&self, escrow_id: u64, submitter: &Address, evidence_ref: Symbol) {
        self.dispute_evidence
            .submit_evidence(&escrow_id, submitter, &evidence_ref)
            .unwrap();
    }

    /// Submit resolution (arbitrator decides).
    fn submit_resolution(&self, escrow_id: u64, release_to_mentor: bool, note: Symbol) {
        self.dispute_evidence
            .submit_resolution(&escrow_id, &self.arbitrator, &release_to_mentor, &note)
            .unwrap();
    }

    /// Resolve the escrow dispute (admin only).
    fn resolve_escrow_dispute(&self, escrow_id: u64, mentor_pct: u32) {
        self.escrow.resolve_dispute(&escrow_id, &mentor_pct);
    }

    /// Make an insurance claim for the learner.
    fn make_insurance_claim(&self, escrow_id: Symbol, learner: &Address, amount: i128) {
        self.insurance.claim(&escrow_id, learner, &amount).unwrap();
    }

    /// Get the resolution from dispute_evidence contract.
    fn get_resolution(&self, escrow_id: u64) -> DisputeResolution {
        self.dispute_evidence.get_resolution(&escrow_id)
    }

    /// Count events matching a topic.
    fn count_events(&self, topic: &Symbol) -> usize {
        let events = self.env.events().all();
        events.iter().filter(|(_, topics, _)| {
            topics.iter().any(|t| {
                if let Ok(s) = Symbol::try_from_val(&self.env, &t) {
                    s == *topic
                } else {
                    false
                }
            })
        }).count()
    }

    /// Verify event order: event_a must appear before event_b.
    fn verify_event_order(&self, event_a: Symbol, event_b: Symbol) {
        let events = self.env.events().all();
        let mut pos_a = None;
        let mut pos_b = None;

        for (i, (_, topics, _)) in events.iter().enumerate() {
            for j in 0..topics.len() {
                let v = topics.get(j).unwrap();
                if let Ok(s) = Symbol::try_from_val(&self.env, &v) {
                    if s == event_a && pos_a.is_none() {
                        pos_a = Some(i);
                    }
                    if s == event_b && pos_b.is_none() {
                        pos_b = Some(i);
                    }
                }
            }
        }

        assert!(pos_a.is_some(), "event {} must be emitted", event_a);
        assert!(pos_b.is_some(), "event {} must be emitted", event_b);
        assert!(
            pos_a.unwrap() < pos_b.unwrap(),
            "event {} must appear before {}",
            event_a,
            event_b
        );
    }
}

// Mock contracts for governance testing
#[contract]
pub struct MockGovernanceToken;

#[contractimpl]
impl MockGovernanceToken {
    pub fn set_total_supply(env: Env, amount: i128) {
        env.storage().persistent().set(&symbol_short!("TOT_SUP"), &amount);
    }
    pub fn set_balance(env: Env, addr: Address, amount: i128) {
        env.storage().persistent().set(&(symbol_short!("BAL"), addr), &amount);
    }
    pub fn balance(env: Env, addr: Address) -> i128 {
        env.storage().persistent().get(&(symbol_short!("BAL"), addr)).unwrap_or(0)
    }
    pub fn total_supply(env: Env) -> i128 {
        env.storage().persistent().get(&symbol_short!("TOT_SUP")).unwrap_or(0)
    }
}

#[contract]
pub struct MockSnapshot;

#[contractimpl]
impl MockSnapshot {
    pub fn record_snapshot(env: Env, _id: u32) {
        env.storage().persistent().set(&symbol_short!("TOT_SUP"), &1000i128);
    }
    pub fn get_total_supply_at(env: Env, _id: u32) -> i128 {
        env.storage().persistent().get(&symbol_short!("TOT_SUP")).unwrap_or(0)
    }
    pub fn get_voting_power(env: Env, _id: u32, voter: Address) -> i128 {
        let token: Address = env.storage().persistent().get(&symbol_short!("TOKEN")).unwrap();
        let args = vec![&env, voter.into_val(&env)];
        env.invoke_contract::<i128>(&token, &Symbol::new(&env, "balance"), args)
    }
    pub fn set_token(env: Env, token: Address) {
        env.storage().persistent().set(&symbol_short!("TOKEN"), &token);
    }
}

// ---------------------------------------------------------------------------
// Test Scenario A: Happy Path
// Dispute opens, evidence submitted by both parties, resolution in mentor favor,
// insurance not triggered
// ---------------------------------------------------------------------------

#[test]
fn test_scenario_a_happy_path_mentor_wins() {
    let env = Env::default();
    let f = DisputeFlowFixture::new(&env, 500, 500_000); // 5% fee, 500k insurance pool

    let escrow_amount = 10_000;
    let escrow_id = f.create_escrow(escrow_amount);

    // Record initial balances
    let mentor_initial = f.token_balance(&f.mentor);
    let learner_initial = f.token_balance(&f.learner);
    let treasury_initial = f.token_balance(&f.treasury);
    let insurance_initial = f.insurance.get_pool_balance();

    // Step 1: Open dispute
    f.open_dispute(escrow_id, symbol_short!("LATE_DELIVERY"));
    let escrow = f.escrow.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Disputed);

    // Step 2: Record dispute opened in dispute_evidence contract
    f.record_dispute_opened(escrow_id);

    // Step 3: Both parties submit evidence
    f.submit_evidence(escrow_id, &f.mentor, symbol_short!("MENTOR_PROOF"));
    f.submit_evidence(escrow_id, &f.learner, symbol_short!("LEARNER_PROOF"));

    let evidence_count = f.dispute_evidence.get_evidence_count(&escrow_id);
    assert_eq!(evidence_count, 2);

    // Step 4: Advance past resolution timelock (24 hours)
    advance_time(&env, 24 * 60 * 60 + 1);

    // Step 5: Arbitrator submits resolution (mentor wins 100%)
    f.submit_resolution(escrow_id, true, symbol_short!("MENTOR_WINS"));

    let resolution = f.get_resolution(escrow_id);
    assert!(resolution.release_to_mentor);
    assert_eq!(resolution.arbitrator, f.arbitrator);

    // Step 6: Admin resolves escrow dispute (100% to mentor)
    f.resolve_escrow_dispute(escrow_id, 100);

    // Verify final state
    let final_escrow = f.escrow.get_escrow(&escrow_id);
    assert_eq!(final_escrow.status, EscrowStatus::Resolved);

    // Verify token balances
    // Mentor gets full amount (no fee in dispute resolution)
    let mentor_final = f.token_balance(&f.mentor);
    assert_eq!(mentor_final, mentor_initial + escrow_amount);

    // Learner gets nothing
    let learner_final = f.token_balance(&f.learner);
    assert_eq!(learner_final, learner_initial);

    // Treasury gets nothing (no fee in dispute resolution)
    let treasury_final = f.token_balance(&f.treasury);
    assert_eq!(treasury_final, treasury_initial);

    // Insurance pool unchanged
    let insurance_final = f.insurance.get_pool_balance();
    assert_eq!(insurance_final, insurance_initial);

    // Verify event emissions in correct order
    f.verify_event_order(symbol_short!("DisputeOpened"), symbol_short!("evidence_submitted"));
    f.verify_event_order(symbol_short!("evidence_submitted"), symbol_short!("dispute_resolved"));
    f.verify_event_order(symbol_short!("dispute_resolved"), symbol_short!("DisputeResolved"));
}

// ---------------------------------------------------------------------------
// Test Scenario B: Learner Wins with Insurance
// Resolution releases to learner, insurance claim covers shortfall
// ---------------------------------------------------------------------------

#[test]
fn test_scenario_b_learner_wins_with_insurance() {
    let env = Env::default();
    // 0% fee for simplicity, 500k insurance pool
    let f = DisputeFlowFixture::new(&env, 0, 500_000);

    let escrow_amount = 10_000;
    let escrow_id = f.create_escrow(escrow_amount);

    // Record initial balances
    let mentor_initial = f.token_balance(&f.mentor);
    let learner_initial = f.token_balance(&f.learner);
    let insurance_initial = f.insurance.get_pool_balance();

    // Step 1: Open dispute
    f.open_dispute(escrow_id, symbol_short!("POOR_QUALITY"));

    // Step 2: Record dispute opened
    f.record_dispute_opened(escrow_id);

    // Step 3: Submit evidence
    f.submit_evidence(escrow_id, &f.mentor, symbol_short!("MENTOR_EVIDENCE"));
    f.submit_evidence(escrow_id, &f.learner, symbol_short!("LEARNER_EVIDENCE"));

    // Step 4: Advance past timelock
    advance_time(&env, 24 * 60 * 60 + 1);

    // Step 5: Arbitrator rules in favor of learner (0% to mentor)
    f.submit_resolution(escrow_id, false, symbol_short!("LEARNER_WINS"));

    let resolution = f.get_resolution(escrow_id);
    assert!(!resolution.release_to_mentor);

    // Step 6: Resolve escrow (0% to mentor, 100% to learner)
    f.resolve_escrow_dispute(escrow_id, 0);

    // Verify escrow state
    let final_escrow = f.escrow.get_escrow(&escrow_id);
    assert_eq!(final_escrow.status, EscrowStatus::Resolved);

    // Learner gets full amount from escrow
    let learner_after_escrow = f.token_balance(&f.learner);
    assert_eq!(learner_after_escrow, learner_initial + escrow_amount);

    // Mentor gets nothing
    let mentor_after_escrow = f.token_balance(&f.mentor);
    assert_eq!(mentor_after_escrow, mentor_initial);

    // Step 7: Learner files insurance claim for additional coverage
    // (Simulating that learner expected more than the escrow amount)
    let insurance_claim_amount = 2_000;
    f.make_insurance_claim(
        Symbol::new(&env, &format!("escrow_{}", escrow_id)),
        &f.learner,
        insurance_claim_amount,
    );

    // Verify insurance pool decreased
    let insurance_final = f.insurance.get_pool_balance();
    assert_eq!(insurance_final, insurance_initial - insurance_claim_amount);

    // Verify learner received insurance payout
    let learner_final = f.token_balance(&f.learner);
    assert_eq!(learner_final, learner_after_escrow + insurance_claim_amount);

    // Verify total claims paid
    assert_eq!(
        f.insurance.get_total_claims_paid(),
        insurance_claim_amount
    );

    // Verify event order
    f.verify_event_order(symbol_short!("DisputeOpened"), symbol_short!("dispute_resolved"));
    f.verify_event_order(symbol_short!("dispute_resolved"), symbol_short!("claim_paid"));
}

// ---------------------------------------------------------------------------
// Test Scenario C: Evidence Window Expires
// Verify EvidenceWindowClosed is enforced
// ---------------------------------------------------------------------------

#[test]
fn test_scenario_c_evidence_window_expires() {
    let env = Env::default();
    let f = DisputeFlowFixture::new(&env, 500, 0);

    let escrow_amount = 10_000;
    let escrow_id = f.create_escrow(escrow_amount);

    // Step 1: Open dispute
    f.open_dispute(escrow_id, symbol_short!("LATE_DELIVERY"));
    f.record_dispute_opened(escrow_id);

    // Step 2: Submit evidence immediately (should succeed)
    f.submit_evidence(escrow_id, &f.mentor, symbol_short!("EARLY_EVIDENCE"));
    assert_eq!(f.dispute_evidence.get_evidence_count(&escrow_id), 1);

    // Step 3: Advance past evidence window (48 hours default)
    advance_time(&env, 48 * 60 * 60 + 1);

    // Step 4: Try to submit more evidence (should fail)
    let result = f.dispute_evidence.try_submit_evidence(
        &escrow_id,
        &f.learner,
        &Symbol::new(&env, "LATE_EVIDENCE"),
    );
    assert!(result.is_err(), "EvidenceWindowClosed should be enforced");

    // Verify evidence count unchanged
    assert_eq!(f.dispute_evidence.get_evidence_count(&escrow_id), 1);

    // Step 5: Resolution should still work (evidence window doesn't block resolution)
    advance_time(&env, 24 * 60 * 60 + 1); // Past timelock
    f.submit_resolution(escrow_id, true, symbol_short!("MENTOR_WINS"));

    let resolution = f.get_resolution(escrow_id);
    assert!(resolution.release_to_mentor);

    // Verify event order
    f.verify_event_order(symbol_short!("DisputeOpened"), symbol_short!("evidence_submitted"));
}

// ---------------------------------------------------------------------------
// Test Scenario D: Resolution Timelock Bypass (Should Fail)
// ---------------------------------------------------------------------------

#[test]
fn test_scenario_d_timelock_bypass_fails() {
    let env = Env::default();
    let f = DisputeFlowFixture::new(&env, 500, 0);

    let escrow_amount = 10_000;
    let escrow_id = f.create_escrow(escrow_amount);

    // Step 1: Open dispute
    f.open_dispute(escrow_id, symbol_short!("LATE_DELIVERY"));
    f.record_dispute_opened(escrow_id);

    // Step 2: Try to submit resolution immediately (should fail - timelock active)
    let result = f.dispute_evidence.try_submit_resolution(
        &escrow_id,
        &f.arbitrator,
        &true,
        &Symbol::new(&env, "EARLY_RESOLUTION"),
    );
    assert!(result.is_err(), "ResolutionTimelockActive should be enforced");

    // Step 3: Advance only 1 hour (still before timelock)
    advance_time(&env, 60 * 60);

    let result2 = f.dispute_evidence.try_submit_resolution(
        &escrow_id,
        &f.arbitrator,
        &true,
        &Symbol::new(&env, "STILL_EARLY"),
    );
    assert!(result2.is_err(), "Resolution should still fail before timelock");

    // Step 4: Advance to exactly the timelock boundary (should succeed)
    advance_time(&env, 23 * 60 * 60); // Total: 24 hours
    f.submit_resolution(escrow_id, true, symbol_short!("MENTOR_WINS"));

    let resolution = f.get_resolution(escrow_id);
    assert!(resolution.release_to_mentor);

    // Verify escrow can be resolved
    f.resolve_escrow_dispute(escrow_id, 100);
    let final_escrow = f.escrow.get_escrow(&escrow_id);
    assert_eq!(final_escrow.status, EscrowStatus::Resolved);

    // Verify event order
    f.verify_event_order(symbol_short!("DisputeOpened"), symbol_short!("dispute_resolved"));
}

// ---------------------------------------------------------------------------
// Test Scenario E: Full Integration with Governance Arbitrator Selection
// ---------------------------------------------------------------------------

#[test]
fn test_scenario_e_governance_arbitrator_selection() {
    let env = Env::default();
    let f = DisputeFlowFixture::new(&env, 500, 0);

    let escrow_amount = 10_000;
    let escrow_id = f.create_escrow(escrow_amount);

    // Step 1: Open dispute
    f.open_dispute(escrow_id, symbol_short!("LATE_DELIVERY"));
    f.record_dispute_opened(escrow_id);

    // Step 2: Use governance to select arbitrator
    let selected_arbitrator = f.governance.select_arbitrator(&escrow_id);
    assert_eq!(selected_arbitrator, f.arbitrator);

    // Step 3: Submit evidence
    f.submit_evidence(escrow_id, &f.mentor, symbol_short!("MENTOR_PROOF"));

    // Step 4: Advance past timelock
    advance_time(&env, 24 * 60 * 60 + 1);

    // Step 5: Selected arbitrator submits resolution
    f.dispute_evidence
        .submit_resolution(&escrow_id, &selected_arbitrator, &true, &symbol_short!("OK"))
        .unwrap();

    let resolution = f.get_resolution(escrow_id);
    assert_eq!(resolution.arbitrator, f.arbitrator);

    // Step 6: Resolve escrow
    f.resolve_escrow_dispute(escrow_id, 100);

    let final_escrow = f.escrow.get_escrow(&escrow_id);
    assert_eq!(final_escrow.status, EscrowStatus::Resolved);

    // Verify complete event chain
    f.verify_event_order(symbol_short!("DisputeOpened"), symbol_short!("evidence_submitted"));
    f.verify_event_order(symbol_short!("evidence_submitted"), symbol_short!("dispute_resolved"));
    f.verify_event_order(symbol_short!("dispute_resolved"), symbol_short!("DisputeResolved"));
}

// ---------------------------------------------------------------------------
// Test Scenario F: Insurance Coverage Ratio Check
// ---------------------------------------------------------------------------

#[test]
fn test_scenario_f_insurance_coverage_ratio() {
    let env = Env::default();
    // Small insurance pool relative to escrow value
    let f = DisputeFlowFixture::new(&env, 500, 50_000); // 50k pool for 10k escrow

    let escrow_amount = 10_000;
    let escrow_id = f.create_escrow(escrow_amount);

    // Set active escrow value
    f.insurance.set_active_escrow_value(&escrow_amount);

    // Check coverage ratio: 50,000 / 10,000 = 500% = 5000 bps
    let coverage_ratio = f.insurance.get_coverage_ratio();
    assert_eq!(coverage_ratio, 5000);
    assert!(!f.insurance.is_coverage_low());

    // Open dispute and resolve in learner's favor
    f.open_dispute(escrow_id, symbol_short!("LATE_DELIVERY"));
    f.record_dispute_opened(escrow_id);
    advance_time(&env, 24 * 60 * 60 + 1);
    f.submit_resolution(escrow_id, false, symbol_short!("LEARNER_WINS"));
    f.resolve_escrow_dispute(escrow_id, 0);

    // Learner gets escrow amount
    let learner_escrow_amount = f.token_balance(&f.learner);

    // File insurance claim for shortfall
    let claim_amount = 5_000;
    f.make_insurance_claim(
        Symbol::new(&env, &format!("escrow_{}", escrow_id)),
        &f.learner,
        claim_amount,
    );

    // Verify insurance pool decreased
    let pool_after = f.insurance.get_pool_balance();
    assert_eq!(pool_after, 50_000 - claim_amount);

    // Verify total claims paid
    assert_eq!(f.insurance.get_total_claims_paid(), claim_amount);
}

// ---------------------------------------------------------------------------
// Performance Test: All scenarios complete in < 10s
// ---------------------------------------------------------------------------

#[test]
fn test_performance_all_scenarios_complete_quickly() {
    let start = std::time::Instant::now();

    // Scenario A
    {
        let env = Env::default();
        let f = DisputeFlowFixture::new(&env, 500, 500_000);
        let escrow_id = f.create_escrow(10_000);
        f.open_dispute(escrow_id, symbol_short!("TEST"));
        f.record_dispute_opened(escrow_id);
        f.submit_evidence(escrow_id, &f.mentor, symbol_short!("E1"));
        advance_time(&env, 24 * 60 * 60 + 1);
        f.submit_resolution(escrow_id, true, symbol_short!("WIN"));
        f.resolve_escrow_dispute(escrow_id, 100);
    }

    // Scenario B
    {
        let env = Env::default();
        let f = DisputeFlowFixture::new(&env, 0, 500_000);
        let escrow_id = f.create_escrow(10_000);
        f.open_dispute(escrow_id, symbol_short!("TEST"));
        f.record_dispute_opened(escrow_id);
        advance_time(&env, 24 * 60 * 60 + 1);
        f.submit_resolution(escrow_id, false, symbol_short!("LOSE"));
        f.resolve_escrow_dispute(escrow_id, 0);
        f.make_insurance_claim(
            Symbol::new(&env, "escrow_1"),
            &f.learner,
            2_000,
        );
    }

    // Scenario C
    {
        let env = Env::default();
        let f = DisputeFlowFixture::new(&env, 500, 0);
        let escrow_id = f.create_escrow(10_000);
        f.open_dispute(escrow_id, symbol_short!("TEST"));
        f.record_dispute_opened(escrow_id);
        f.submit_evidence(escrow_id, &f.mentor, symbol_short!("E1"));
        advance_time(&env, 48 * 60 * 60 + 1);
        let result = f.dispute_evidence.try_submit_evidence(
            &escrow_id,
            &f.learner,
            &Symbol::new(&env, "LATE"),
        );
        assert!(result.is_err());
    }

    // Scenario D
    {
        let env = Env::default();
        let f = DisputeFlowFixture::new(&env, 500, 0);
        let escrow_id = f.create_escrow(10_000);
        f.open_dispute(escrow_id, symbol_short!("TEST"));
        f.record_dispute_opened(escrow_id);
        let result = f.dispute_evidence.try_submit_resolution(
            &escrow_id,
            &f.arbitrator,
            &true,
            &Symbol::new(&env, "EARLY"),
        );
        assert!(result.is_err());
    }

    let elapsed = start.elapsed();
    assert!(
        elapsed.as_secs() < 10,
        "All scenarios must complete in < 10s, took {}s",
        elapsed.as_secs()
    );
}

// ---------------------------------------------------------------------------
// Documentation Test: Verify workflow matches dispute_evidence module docs
// ---------------------------------------------------------------------------

#[test]
fn test_workflow_matches_module_documentation() {
    // This test verifies the exact workflow documented in dispute_evidence::lib.rs:
    //
    // 1. Learner or mentor opens a dispute on the escrow contract.
    // 2. Either party calls submit_evidence with a Symbol pointing to off-chain document.
    // 3. An arbitrator calls submit_resolution after MIN_RESOLUTION_DELAY_SECS (24h).
    // 4. The admin uses the on-chain resolution record to call resolve_dispute on escrow.

    let env = Env::default();
    let f = DisputeFlowFixture::new(&env, 500, 0);

    let escrow_id = f.create_escrow(10_000);

    // Step 1: Open dispute (learner calls escrow.dispute)
    f.open_dispute(escrow_id, symbol_short!("WORKFLOW_TEST"));
    let escrow = f.escrow.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Disputed, "Step 1: Escrow should be Disputed");

    // Step 2: Record dispute opened (links escrow to dispute_evidence)
    f.record_dispute_opened(escrow_id);
    let opened_at = f.dispute_evidence.get_dispute_opened_at(&escrow_id);
    assert!(opened_at.is_some(), "Step 2: Dispute opened timestamp should be recorded");

    // Step 3: Submit evidence (both parties)
    let evidence_ref = symbol_short!("IPFS_QmHash");
    f.submit_evidence(escrow_id, &f.mentor, evidence_ref);
    let evidence = f.dispute_evidence.get_evidence(&escrow_id);
    assert_eq!(evidence.len(), 1, "Step 3: Evidence should be recorded");
    assert_eq!(evidence.get(0).unwrap().submitter, f.mentor);

    // Step 4: Wait for resolution timelock
    advance_time(&env, 24 * 60 * 60 + 1);

    // Step 5: Arbitrator submits resolution
    f.submit_resolution(escrow_id, true, symbol_short!("RESOLVED"));
    let resolution = f.get_resolution(escrow_id);
    assert!(resolution.release_to_mentor, "Step 5: Resolution should favor mentor");

    // Step 6: Admin resolves escrow using resolution
    f.resolve_escrow_dispute(escrow_id, 100);
    let final_escrow = f.escrow.get_escrow(&escrow_id);
    assert_eq!(final_escrow.status, EscrowStatus::Resolved, "Step 6: Escrow should be Resolved");

    // Verify complete event chain
    let events = env.events().all();
    assert!(
        events.len() >= 5,
        "Should have at least 5 events in the workflow"
    );
}
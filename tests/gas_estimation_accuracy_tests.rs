//! Gas estimation accuracy integration tests.
//!
//! Verifies that on-chain `estimate_*` view functions return values within
//! a reasonable tolerance of the actual gas consumed by the corresponding
//! real operation. Also validates that estimates correctly reflect
//! configuration changes (fees, integrations, etc.).

extern crate alloc;

use mentorminds_escrow::{EscrowContract, EscrowContractClient};
use mentorminds_escrow_factory::EscrowFactory;
use mentorminds_governance::{
    GovernanceContract, GovernanceContractClient, ProposalAction,
};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Bytes, BytesN, Env, Symbol, Vec as SorobanVec,
};
use shared::GasEstimate;

// ---------------------------------------------------------------------------
// Escrow test fixture
// ---------------------------------------------------------------------------

struct TestFixture {
    pub env: Env,
    pub contract_id: Address,
    pub admin: Address,
    pub mentor: Address,
    pub learner: Address,
    pub treasury: Address,
    pub token: Address,
}

impl TestFixture {
    fn setup_with_fee(fee_bps: u32) -> Self {
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
        client.initialize(&admin, &treasury, &fee_bps, &approved, &0u64);

        Self { env, contract_id, admin, mentor, learner, treasury, token }
    }

    fn client(&self) -> EscrowContractClient<'_> {
        EscrowContractClient::new(&self.env, &self.contract_id)
    }

    fn create_escrow_at(&self, amount: i128, fee_bps: u32, session: &str) -> u64 {
        self.client().create_escrow(
            &self.mentor,
            &self.learner,
            &amount,
            &Symbol::new(&self.env, session),
            &self.token,
            &(self.env.ledger().timestamp() + 3600),
            &fee_bps,
        )
    }
}

// ---------------------------------------------------------------------------
// Governance test helper
// ---------------------------------------------------------------------------

fn setup_governance(env: &Env) -> (GovernanceContractClient, Address, Address) {
    let admin = Address::generate(env);
    let proposer = Address::generate(env);
    let voter = Address::generate(env);
    let mnt = Address::generate(env);

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
            env.storage().persistent().get(&SnapKey::Supply).unwrap_or(10_000)
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

    let snapshot = env.register(MockSnapshot, ());
    let delegation = env.register(MockDelegation, ());
    let gov = env.register(GovernanceContract, ());

    let client = GovernanceContractClient::new(env, &gov);
    client.initialize(
        &admin,
        &mnt,
        &snapshot,
        &delegation,
        &Some(60u64),
        &Some(1_000u32),
    );

    (client, proposer, voter)
}

// ---------------------------------------------------------------------------
// #761 Escrow gas estimation tests
// ---------------------------------------------------------------------------

#[test]
fn test_estimate_release_escrow_cost_is_nonzero_and_view_only() {
    let f = TestFixture::setup_with_fee(500);
    let id = f.create_escrow_at(1_000, 0, "GAS1");

    let estimate = f.client().estimate_release_escrow_cost(&id);
    assert!(estimate.base_instructions > 0);
    assert!(estimate.storage_reads > 0);
    assert!(estimate.storage_writes > 0);
    assert!(estimate.cross_contract_calls > 0);

    f.client().release_funds(&f.learner, &id);
    assert_eq!(f.client().get_escrow(&id).status, mentorminds_escrow::EscrowStatus::Released);
}

#[test]
fn test_estimate_release_escrow_cost_accounts_for_fee_transfer() {
    let f_with_fee = TestFixture::setup_with_fee(500);
    let id_with_fee = f_with_fee.create_escrow_at(1_000, 0, "GAS2");
    let with_fee = f_with_fee.client().estimate_release_escrow_cost(&id_with_fee);

    let f_no_fee = TestFixture::setup_with_fee(0);
    let id_no_fee = f_no_fee.create_escrow_at(1_000, 0, "GAS3");
    let no_fee = f_no_fee.client().estimate_release_escrow_cost(&id_no_fee);

    assert!(with_fee.cross_contract_calls > no_fee.cross_contract_calls);
    assert!(with_fee.base_instructions > no_fee.base_instructions);
}

#[test]
fn test_estimate_release_escrow_cost_within_tolerance_of_actual() {
    let f = TestFixture::setup_with_fee(500);
    let id = f.create_escrow_at(1_000, 0, "GAS4");

    let estimate = f.client().estimate_release_escrow_cost(&id);

    f.env.budget().reset_default();
    f.client().release_funds(&f.learner, &id);
    let actual = f.env.budget().cpu_instruction_cost();

    let diff = if actual > estimate.base_instructions {
        actual - estimate.base_instructions
    } else {
        estimate.base_instructions - actual
    };
    let tolerance = actual / 5;
    assert!(
        diff <= tolerance,
        "estimate {} vs actual {} exceeds 20% tolerance",
        estimate.base_instructions,
        actual
    );
}

#[test]
fn test_estimate_release_escrow_cost_increases_with_fee_and_integrations() {
    let f_base = TestFixture::setup_with_fee(0);
    let id_base = f_base.create_escrow_at(1_000, 0, "BASE");
    let base_est = f_base.client().estimate_release_escrow_cost(&id_base);

    let f_fee = TestFixture::setup_with_fee(500);
    let id_fee = f_fee.create_escrow_at(1_000, 0, "FEE");
    let fee_est = f_fee.client().estimate_release_escrow_cost(&id_fee);

    assert!(fee_est.cross_contract_calls >= base_est.cross_contract_calls);
    assert!(fee_est.base_instructions >= base_est.base_instructions);
}

// ---------------------------------------------------------------------------
// #761 Governance gas estimation tests
// ---------------------------------------------------------------------------

#[test]
fn test_estimate_governance_vote_cost_is_nonzero_and_view_only() {
    let env = Env::default();
    env.mock_all_auths();
    let (gov, _admin, voter, _token_id, _snapshot_id) = setup_governance(&env);

    let title = Bytes::from_slice(&env, b"Proposal");
    let description_hash = BytesN::from_array(&env, &[1u8; 32]);
    let proposal_id = gov.create_proposal(
        &voter,
        &title,
        &description_hash,
        &ProposalAction::UpdateFee(300),
    );

    let estimate = gov.estimate_governance_vote_cost(&proposal_id, &voter);
    assert!(estimate.base_instructions > 0);
    assert!(estimate.storage_reads > 0);
    assert!(estimate.storage_writes > 0);
    assert!(estimate.cross_contract_calls > 0);

    gov.vote(&voter, &proposal_id, &true);
}

#[test]
fn test_estimate_governance_vote_cost_within_tolerance_of_actual() {
    let env = Env::default();
    env.mock_all_auths();
    let (gov, _admin, voter, _token_id, _snapshot_id) = setup_governance(&env);

    let title = Bytes::from_slice(&env, b"Proposal");
    let description_hash = BytesN::from_array(&env, &[2u8; 32]);
    let proposal_id = gov.create_proposal(
        &voter,
        &title,
        &description_hash,
        &ProposalAction::UpdateFee(300),
    );

    let estimate = gov.estimate_governance_vote_cost(&proposal_id, &voter);

    env.budget().reset_default();
    gov.vote(&voter, &proposal_id, &true);
    let actual = env.budget().cpu_instruction_cost();

    let diff = if actual > estimate.base_instructions {
        actual - estimate.base_instructions
    } else {
        estimate.base_instructions - actual
    };
    let tolerance = actual / 5;
    assert!(
        diff <= tolerance,
        "estimate {} vs actual {} exceeds 20% tolerance",
        estimate.base_instructions,
        actual
    );
}

// ---------------------------------------------------------------------------
// #761 Escrow Factory gas estimation tests
// ---------------------------------------------------------------------------

#[test]
fn test_estimate_deploy_escrow_cost_is_nonzero_and_view_only() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let implementation = Address::generate(&env);
    let factory_address = env.register_contract(None, EscrowFactory);
    let client = mentorminds_escrow_factory::EscrowFactoryClient::new(&env, &factory_address);
    client.initialize(&admin, &implementation);

    let estimate = client.estimate_deploy_escrow_cost();
    assert!(estimate.base_instructions > 0);
    assert!(estimate.storage_reads > 0);
    assert!(estimate.storage_writes > 0);
    assert!(estimate.cross_contract_calls > 0);
}

#[test]
fn test_estimate_deploy_escrow_cost_reflects_configured_integrations() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let implementation = Address::generate(&env);
    let factory_address = env.register_contract(None, EscrowFactory);
    let client = mentorminds_escrow_factory::EscrowFactoryClient::new(&env, &factory_address);
    client.initialize(&admin, &implementation);

    let baseline = client.estimate_deploy_escrow_cost();

    let guardian = Address::generate(&env);
    client.set_pause_guardian(&guardian);
    let with_guardian = client.estimate_deploy_escrow_cost();
    assert!(with_guardian.cross_contract_calls > baseline.cross_contract_calls);
    assert!(with_guardian.base_instructions > baseline.base_instructions);

    let detector = Address::generate(&env);
    client.set_anomaly_detector(&detector);
    let with_detector = client.estimate_deploy_escrow_cost();
    assert!(with_detector.cross_contract_calls > with_guardian.cross_contract_calls);

    client.set_bypass_anomaly_check(&true);
    let bypassed = client.estimate_deploy_escrow_cost();
    assert_eq!(bypassed.cross_contract_calls, with_guardian.cross_contract_calls);
}

// ---------------------------------------------------------------------------
// Cross-cutting accuracy tests
// ---------------------------------------------------------------------------

#[test]
fn test_shared_gas_estimate_constants_are_sensible() {
    assert!(GasEstimate::DEFAULT_BASE_INSTRUCTIONS > 0);
    assert!(GasEstimate::DEFAULT_PER_STORAGE_OP_INSTRUCTIONS > 0);
    assert!(GasEstimate::DEFAULT_PER_CROSS_CALL_INSTRUCTIONS > 0);
    assert!(GasEstimate::DEFAULT_TOLERANCE_BPS > 0);
    assert!(GasEstimate::DEFAULT_TOLERANCE_BPS <= 10_000);
}

#[test]
fn test_gas_estimate_within_tolerance_helper() {
    assert!(GasEstimate::within_tolerance(100, 100, 1000));
    assert!(GasEstimate::within_tolerance(100, 110, 1000)); // 10% diff, 10% tolerance
    assert!(!GasEstimate::within_tolerance(100, 111, 1000)); // 11% diff, 10% tolerance
    assert!(GasEstimate::within_tolerance(0, 0, 1000));
    assert!(!GasEstimate::within_tolerance(0, 10, 1000));
    assert!(!GasEstimate::within_tolerance(10, 0, 1000));
}

#[test]
fn test_gas_estimate_compute_instructions() {
    let total = GasEstimate::compute_instructions(40_000, 5, 2);
    assert_eq!(total, 40_000 + 5 * 2_000 + 2 * 300_000);
}

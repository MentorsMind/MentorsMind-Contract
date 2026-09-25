//! Deterministic Test Fixture Factory
//!
//! Provides a unified, deterministic way to set up cross-contract integration tests.
//! Eliminates duplication of mock implementations and ensures consistent initialization
//! across all integration tests.

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, BytesN, Env, Vec,
};

// Import contract types
use mentorminds_escrow::{EscrowContract, EscrowContractClient};
use mentorminds_governance::{GovernanceContract, GovernanceContractClient};
use mentorminds_staking::{StakingContract, StakingContractClient};
use mentorminds_delegation::{DelegationContract, DelegationContractClient};
use mentorminds_verification::{VerificationContract, VerificationContractClient};
use mentorminds_timelock::{TimelockController, TimelockControllerClient};
use mentorminds_reputation::{ReputationContract, ReputationContractClient};

// Import unified mocks from shared
use shared::{MockMNT, MockSnapshot, MockKYCRegistry, MockSanctions};

// ---------------------------------------------------------------------------
// Test Addresses - Deterministic generation
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct TestAddresses {
    pub admin: Address,
    pub treasury: Address,
    pub mentor: Address,
    pub learner: Address,
    pub voter: Address,
    pub arbitrator: Address,
}

impl TestAddresses {
    pub fn new(env: &Env) -> Self {
        Self {
            admin: Address::generate(env),
            treasury: Address::generate(env),
            mentor: Address::generate(env),
            learner: Address::generate(env),
            voter: Address::generate(env),
            arbitrator: Address::generate(env),
        }
    }
}

// ---------------------------------------------------------------------------
// Contract Addresses - All deployed contracts
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct ContractAddresses {
    pub mnt_token: Address,
    pub escrow: Address,
    pub governance: Option<Address>,
    pub staking: Option<Address>,
    pub delegation: Option<Address>,
    pub verification: Option<Address>,
    pub timelock: Option<Address>,
    pub reputation: Option<Address>,
    pub snapshot: Option<Address>,
    pub kyc_registry: Option<Address>,
    pub sanctions: Option<Address>,
}

// ---------------------------------------------------------------------------
// Fixture Builder - Deterministic contract setup
// ---------------------------------------------------------------------------

pub struct FixtureBuilder<'a> {
    env: &'a Env,
    addresses: TestAddresses,
    contracts: ContractAddresses,
    config: FixtureConfig,
}

#[derive(Clone, Default)]
pub struct FixtureConfig {
    pub fee_bps: u32,
    pub voting_period_secs: u64,
    pub quorum_bps: u32,
    pub auto_release_delay_secs: u64,
    pub initial_token_supply: i128,
}

impl Default for FixtureConfig {
    fn default() -> Self {
        Self {
            fee_bps: 500, // 5%
            voting_period_secs: 7 * 24 * 60 * 60, // 7 days
            quorum_bps: 1_000, // 10%
            auto_release_delay_secs: 72 * 60 * 60, // 72 hours
            initial_token_supply: 1_000_000_000,
        }
    }
}

impl<'a> FixtureBuilder<'a> {
    pub fn new(env: &'a Env) -> Self {
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 10_000);

        let addresses = TestAddresses::new(env);
        
        Self {
            env,
            addresses,
            contracts: ContractAddresses {
                mnt_token: Address::generate(env),
                escrow: Address::generate(env),
                governance: None,
                staking: None,
                delegation: None,
                verification: None,
                timelock: None,
                reputation: None,
                snapshot: None,
                kyc_registry: None,
                sanctions: None,
            },
            config: FixtureConfig::default(),
        }
    }

    pub fn with_config(mut self, config: FixtureConfig) -> Self {
        self.config = config;
        self
    }

    pub fn with_addresses(mut self, addresses: TestAddresses) -> Self {
        self.addresses = addresses;
        self
    }

    /// Deploy the unified MockMNT token
    pub fn deploy_mnt_token(mut self) -> Self {
        let token_id = self.env.register_contract(None, MockMNT);
        self.contracts.mnt_token = token_id;
        
        let token_client = MockMNTClient::new(self.env, &token_id);
        token_client.initialize(&self.addresses.admin, &self.config.initial_token_supply);
        
        // Fund test addresses
        token_client.mint(&self.addresses.learner, &1_000_000);
        token_client.mint(&self.addresses.mentor, &100_000);
        token_client.mint(&self.addresses.voter, &100_000_000); // For governance quorum
        
        self
    }

    /// Deploy Escrow contract
    pub fn deploy_escrow(mut self) -> Self {
        let escrow_id = self.env.register_contract(None, EscrowContract);
        self.contracts.escrow = escrow_id;
        
        let escrow_client = EscrowContractClient::new(self.env, &escrow_id);
        let mut approved = Vec::new(self.env);
        approved.push_back(self.contracts.mnt_token.clone());
        
        escrow_client.initialize(
            &self.addresses.admin,
            &self.addresses.treasury,
            &self.config.fee_bps,
            &approved,
            &self.config.auto_release_delay_secs,
            &None,
        );
        
        self
    }

    /// Deploy Governance contract
    pub fn deploy_governance(mut self) -> Self {
        let gov_id = self.env.register_contract(None, GovernanceContract);
        self.contracts.governance = Some(gov_id);
        
        let gov_client = GovernanceContractClient::new(self.env, &gov_id);
        gov_client.initialize(
            &self.addresses.admin,
            &self.contracts.mnt_token,
            &self.contracts.snapshot.unwrap_or_else(|| {
                let snap_id = self.env.register_contract(None, MockSnapshot);
                let snap_client = MockSnapshotClient::new(self.env, &snap_id);
                snap_client.set_token(&self.contracts.mnt_token);
                snap_id
            }),
            &Some(self.config.voting_period_secs),
            &Some(self.config.quorum_bps),
        );
        
        self
    }

    /// Deploy Staking contract
    pub fn deploy_staking(mut self) -> Self {
        let staking_id = self.env.register_contract(None, StakingContract);
        self.contracts.staking = Some(staking_id);
        
        let staking_client = StakingContractClient::new(self.env, &staking_id);
        staking_client.initialize(&self.addresses.admin, &self.contracts.mnt_token);
        
        self
    }

    /// Deploy Delegation contract
    pub fn deploy_delegation(mut self) -> Self {
        let del_id = self.env.register_contract(None, DelegationContract);
        self.contracts.delegation = Some(del_id);
        
        let del_client = DelegationContractClient::new(self.env, &del_id);
        del_client.initialize(&self.addresses.admin, &self.contracts.mnt_token);
        
        self
    }

    /// Deploy Verification contract
    pub fn deploy_verification(mut self) -> Self {
        let verif_id = self.env.register_contract(None, VerificationContract);
        self.contracts.verification = Some(verif_id);
        
        let verif_client = VerificationContractClient::new(self.env, &verif_id);
        verif_client.initialize(&self.addresses.admin);
        
        self
    }

    /// Deploy Timelock contract
    pub fn deploy_timelock(mut self) -> Self {
        let tl_id = self.env.register_contract(None, TimelockController);
        self.contracts.timelock = Some(tl_id);
        
        let tl_client = TimelockControllerClient::new(self.env, &tl_id);
        tl_client.initialize(&self.contracts.governance.unwrap());
        
        self
    }

    /// Deploy Reputation contract
    pub fn deploy_reputation(mut self) -> Self {
        let rep_id = self.env.register_contract(None, ReputationContract);
        self.contracts.reputation = Some(rep_id);
        
        let rep_client = ReputationContractClient::new(self.env, &rep_id);
        rep_client.initialize(&self.addresses.admin);
        
        self
    }

    /// Deploy Snapshot contract
    pub fn deploy_snapshot(mut self) -> Self {
        let snap_id = self.env.register_contract(None, MockSnapshot);
        self.contracts.snapshot = Some(snap_id);
        
        let snap_client = MockSnapshotClient::new(self.env, &snap_id);
        snap_client.set_token(&self.contracts.mnt_token);
        
        self
    }

    /// Deploy KYC Registry
    pub fn deploy_kyc_registry(mut self) -> Self {
        let kyc_id = self.env.register_contract(None, MockKYCRegistry);
        self.contracts.kyc_registry = Some(kyc_id);
        
        self
    }

    /// Deploy Sanctions contract
    pub fn deploy_sanctions(mut self) -> Self {
        let sanctions_id = self.env.register_contract(None, MockSanctions);
        self.contracts.sanctions = Some(sanctions_id);
        
        self
    }

    /// Build the complete fixture
    pub fn build(self) -> Fixture<'a> {
        Fixture {
            env: self.env,
            addresses: self.addresses,
            contracts: self.contracts,
        }
    }
}

// ---------------------------------------------------------------------------
// Fixture - Provides access to all deployed contracts
// ---------------------------------------------------------------------------

pub struct Fixture<'a> {
    pub env: &'a Env,
    pub addresses: TestAddresses,
    pub contracts: ContractAddresses,
}

impl<'a> Fixture<'a> {
    /// Get MNT token client
    pub fn mnt_client(&self) -> MockMNTClient<'a> {
        MockMNTClient::new(self.env, &self.contracts.mnt_token)
    }

    /// Get Escrow client
    pub fn escrow_client(&self) -> EscrowContractClient<'a> {
        EscrowContractClient::new(self.env, &self.contracts.escrow)
    }

    /// Get Governance client
    pub fn governance_client(&self) -> Option<GovernanceContractClient<'a>> {
        self.contracts.governance
            .map(|id| GovernanceContractClient::new(self.env, &id))
    }

    /// Get Staking client
    pub fn staking_client(&self) -> Option<StakingContractClient<'a>> {
        self.contracts.staking
            .map(|id| StakingContractClient::new(self.env, &id))
    }

    /// Get Delegation client
    pub fn delegation_client(&self) -> Option<DelegationContractClient<'a>> {
        self.contracts.delegation
            .map(|id| DelegationContractClient::new(self.env, &id))
    }

    /// Get Verification client
    pub fn verification_client(&self) -> Option<VerificationContractClient<'a>> {
        self.contracts.verification
            .map(|id| VerificationContractClient::new(self.env, &id))
    }

    /// Get Timelock client
    pub fn timelock_client(&self) -> Option<TimelockControllerClient<'a>> {
        self.contracts.timelock
            .map(|id| TimelockControllerClient::new(self.env, &id))
    }

    /// Get Reputation client
    pub fn reputation_client(&self) -> Option<ReputationContractClient<'a>> {
        self.contracts.reputation
            .map(|id| ReputationContractClient::new(self.env, &id))
    }

    /// Get Snapshot client
    pub fn snapshot_client(&self) -> Option<MockSnapshotClient<'a>> {
        self.contracts.snapshot
            .map(|id| MockSnapshotClient::new(self.env, &id))
    }

    /// Get KYC Registry client
    pub fn kyc_client(&self) -> Option<MockKYCRegistryClient<'a>> {
        self.contracts.kyc_registry
            .map(|id| MockKYCRegistryClient::new(self.env, &id))
    }

    /// Get Sanctions client
    pub fn sanctions_client(&self) -> Option<MockSanctionsClient<'a>> {
        self.contracts.sanctions
            .map(|id| MockSanctionsClient::new(self.env, &id))
    }

    /// Helper: Verify a mentor (if verification contract is deployed)
    pub fn verify_mentor(&self) {
        if let Some(verif) = self.verification_client() {
            let hash: BytesN<32> = BytesN::from_array(self.env, &[0xABu8; 32]);
            let expiry = self.env.ledger().timestamp() + 3_600;
            verif.verify_mentor(&self.addresses.mentor, &hash, &expiry);
        }
    }

    /// Helper: Create an escrow
    pub fn create_escrow(&self, amount: i128) -> u64 {
        let escrow = self.escrow_client();
        let now = self.env.ledger().timestamp();
        escrow.create_escrow(
            &self.addresses.mentor,
            &self.addresses.learner,
            &amount,
            &symbol_short!("SES1"),
            &self.contracts.mnt_token,
            &now,
            &1u32,
        )
    }

    /// Helper: Advance ledger time
    pub fn advance_time(&self, secs: u64) {
        self.env.ledger().with_mut(|li| li.timestamp += secs);
    }
}

// ---------------------------------------------------------------------------
// Mock Clients (generated by soroban-sdk)
// ---------------------------------------------------------------------------

// These would normally be generated by soroban-sdk. For now, we'll use
// direct contract invocation or the existing client patterns.

pub struct MockMNTClient<'a> {
    env: &'a Env,
    address: &'a Address,
}

impl<'a> MockMNTClient<'a> {
    pub fn new(env: &'a Env, address: &'a Address) -> Self {
        Self { env, address }
    }

    pub fn initialize(&self, admin: &Address, total_supply: &i128) {
        self.env.invoke_contract(
            self.address,
            &symbol_short!("initialize"),
            (admin, total_supply).into_val(self.env),
        );
    }

    pub fn mint(&self, to: &Address, amount: &i128) {
        self.env.invoke_contract(
            self.address,
            &symbol_short!("mint"),
            (to, amount).into_val(self.env),
        );
    }

    pub fn balance(&self, id: &Address) -> i128 {
        self.env.invoke_contract::<i128>(
            self.address,
            &symbol_short!("balance"),
            (id,).into_val(self.env),
        )
    }

    pub fn transfer(&self, from: &Address, to: &Address, amount: &i128) {
        self.env.invoke_contract(
            self.address,
            &symbol_short!("transfer"),
            (from, to, amount).into_val(self.env),
        );
    }
}

pub struct MockSnapshotClient<'a> {
    env: &'a Env,
    address: &'a Address,
}

impl<'a> MockSnapshotClient<'a> {
    pub fn new(env: &'a Env, address: &'a Address) -> Self {
        Self { env, address }
    }

    pub fn set_token(&self, token: &Address) {
        self.env.invoke_contract(
            self.address,
            &symbol_short!("set_token"),
            (token,).into_val(self.env),
        );
    }
}

pub struct MockKYCRegistryClient<'a> {
    env: &'a Env,
    address: &'a Address,
}

impl<'a> MockKYCRegistryClient<'a> {
    pub fn new(env: &'a Env, address: &'a Address) -> Self {
        Self { env, address }
    }

    pub fn set_kyc(&self, user: &Address, approved: &bool) {
        self.env.invoke_contract(
            self.address,
            &symbol_short!("set_kyc"),
            (user, approved).into_val(self.env),
        );
    }

    pub fn is_kyc(&self, user: &Address) -> bool {
        self.env.invoke_contract::<bool>(
            self.address,
            &symbol_short!("is_kyc"),
            (user,).into_val(self.env),
        )
    }
}

pub struct MockSanctionsClient<'a> {
    env: &'a Env,
    address: &'a Address,
}

impl<'a> MockSanctionsClient<'a> {
    pub fn new(env: &'a Env, address: &'a Address) -> Self {
        Self { env, address }
    }

    pub fn set_sanctioned(&self, user: &Address, sanctioned: &bool) {
        self.env.invoke_contract(
            self.address,
            &symbol_short!("set_sanctioned"),
            (user, sanctioned).into_val(self.env),
        );
    }

    pub fn is_sanctioned(&self, user: &Address) -> bool {
        self.env.invoke_contract::<bool>(
            self.address,
            &symbol_short!("is_sanctioned"),
            (user,).into_val(self.env),
        )
    }
}

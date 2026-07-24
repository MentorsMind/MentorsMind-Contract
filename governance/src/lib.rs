#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, Address, Env, Symbol, symbol_short,
};
use mentorminds_shared::ROLE_GOVERNANCE_ADMIN;
use soroban_sdk::Vec;

// Helper function to call RBAC contract's has_role via cross-contract call
fn check_rbac_role(env: &Env, rbac_address: &Address, address: &Address, role: &Symbol) -> bool {
    let fn_name = soroban_sdk::symbol_short!("has_role");
    let args = Vec::new(env);
    args.push_back(address.clone());
    args.push_back(role.clone());
    
    let result: bool = env.invoke_contract(rbac_address, &fn_name, &args).unwrap();
    result
}

/// Proposal status enum
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalStatus {
    Active,
    Passed,
    Rejected,
    Executed,
}

/// Proposal data structure
#[contracttype]
#[derive(Clone, Debug)]
pub struct Proposal {
    pub id: u64,
    pub proposer: Address,
    pub title: Symbol,
    pub description: Symbol,
    pub votes_for: u64,
    pub votes_against: u64,
    pub status: ProposalStatus,
    pub created_at: u64,
    pub voting_end_time: u64,
}

/// Vote data structure
#[contracttype]
#[derive(Clone, Debug)]
pub struct Vote {
    pub voter: Address,
    pub proposal_id: u64,
    pub support: bool,
    pub voted_at: u64,
}

/// Storage keys
const RBAC_ADDRESS: Symbol = symbol_short!("RBAC_ADDR");
const PROPOSALS: Symbol = symbol_short!("PROP");
const PROPOSAL_COUNT: Symbol = symbol_short!("PROP_CNT");
const VOTES: Symbol = symbol_short!("VOTE");
const INITIALIZED: Symbol = symbol_short!("INIT");

/// TTL constants
const STORAGE_TTL_THRESHOLD: u32 = 500_000;
const STORAGE_TTL_BUMP: u32 = 1_000_000;

/// Voting period in seconds (default 7 days)
const DEFAULT_VOTING_PERIOD: u64 = 604_800;

#[contract]
pub struct GovernanceContract;

#[contractimpl]
impl GovernanceContract {
    /// Initialize the governance contract with RBAC contract address
    pub fn initialize(env: Env, rbac_address: Address) {
        // Ensure not already initialized
        if env.storage().persistent().has(&INITIALIZED) {
            panic!("Governance already initialized");
        }

        // Store RBAC contract address
        env.storage().persistent().set(&RBAC_ADDRESS, &rbac_address);
        env.storage().persistent().extend_ttl(&RBAC_ADDRESS, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Initialize proposal count
        env.storage().persistent().set(&PROPOSAL_COUNT, &0u64);
        env.storage().persistent().extend_ttl(&PROPOSAL_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Mark as initialized
        env.storage().persistent().set(&INITIALIZED, &true);
        env.storage().persistent().extend_ttl(&INITIALIZED, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
    }

    /// Create a new proposal (requires ROLE_GOVERNANCE_ADMIN)
    pub fn create_proposal(
        env: Env,
        caller: Address,
        title: Symbol,
        description: Symbol,
        voting_period: Option<u64>,
    ) -> u64 {
        // Verify caller has governance admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_GOVERNANCE_ADMIN) {
            panic!("Caller does not have GOVERNANCE_ADMIN role");
        }

        // Get and increment proposal count
        let mut count: u64 = env.storage().persistent().get(&PROPOSAL_COUNT).unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&PROPOSAL_COUNT, &count);
        env.storage().persistent().extend_ttl(&PROPOSAL_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Calculate voting end time
        let period = voting_period.unwrap_or(DEFAULT_VOTING_PERIOD);
        let voting_end_time = env.ledger().timestamp() + period;

        // Create proposal
        let proposal = Proposal {
            id: count,
            proposer: caller.clone(),
            title: title.clone(),
            description: description.clone(),
            votes_for: 0,
            votes_against: 0,
            status: ProposalStatus::Active,
            created_at: env.ledger().timestamp(),
            voting_end_time,
        };

        // Store proposal
        let key = (PROPOSALS, count);
        env.storage().persistent().set(&key, &proposal);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("PROP_CRT"), count),
            (caller, title, description, voting_end_time),
        );

        count
    }

    /// Vote on a proposal (public function)
    pub fn vote(env: Env, voter: Address, proposal_id: u64, support: bool) {
        // Get proposal
        let key = (PROPOSALS, proposal_id);
        let mut proposal: Proposal = env.storage().persistent().get(&key)
            .expect("Proposal not found");

        // Check if proposal is active
        if proposal.status != ProposalStatus::Active {
            panic!("Proposal is not active");
        }

        // Check if voting period has ended
        if env.ledger().timestamp() >= proposal.voting_end_time {
            panic!("Voting period has ended");
        }

        // Check if already voted
        let vote_key = (VOTES, proposal_id, voter.clone());
        if env.storage().persistent().has(&vote_key) {
            panic!("Already voted on this proposal");
        }

        // Require voter authorization
        voter.require_auth();

        // Record vote
        let vote = Vote {
            voter: voter.clone(),
            proposal_id,
            support,
            voted_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&vote_key, &vote);
        env.storage().persistent().extend_ttl(&vote_key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Update proposal vote counts
        if support {
            proposal.votes_for += 1;
        } else {
            proposal.votes_against += 1;
        }

        // Store updated proposal
        env.storage().persistent().set(&key, &proposal);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("VOTED"), proposal_id),
            (voter, support),
        );
    }

    /// Execute a proposal (requires ROLE_GOVERNANCE_ADMIN)
    pub fn execute_proposal(env: Env, caller: Address, proposal_id: u64) {
        // Verify caller has governance admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_GOVERNANCE_ADMIN) {
            panic!("Caller does not have GOVERNANCE_ADMIN role");
        }

        // Get proposal
        let key = (PROPOSALS, proposal_id);
        let mut proposal: Proposal = env.storage().persistent().get(&key)
            .expect("Proposal not found");

        // Check if proposal can be executed
        if proposal.status != ProposalStatus::Passed {
            panic!("Proposal has not passed");
        }

        // Mark as executed
        proposal.status = ProposalStatus::Executed;
        env.storage().persistent().set(&key, &proposal);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("PROP_EXEC"), proposal_id),
            (caller, proposal.title),
        );
    }

    /// Finalize proposal (requires ROLE_GOVERNANCE_ADMIN)
    pub fn finalize_proposal(env: Env, caller: Address, proposal_id: u64) {
        // Verify caller has governance admin role via RBAC
        let rbac_address: Address = env.storage().persistent().get(&RBAC_ADDRESS)
            .expect("RBAC address not set");
        
        if !check_rbac_role(&env, &rbac_address, &caller, &ROLE_GOVERNANCE_ADMIN) {
            panic!("Caller does not have GOVERNANCE_ADMIN role");
        }

        // Get proposal
        let key = (PROPOSALS, proposal_id);
        let mut proposal: Proposal = env.storage().persistent().get(&key)
            .expect("Proposal not found");

        // Check if proposal is active
        if proposal.status != ProposalStatus::Active {
            panic!("Proposal is not active");
        }

        // Check if voting period has ended
        if env.ledger().timestamp() < proposal.voting_end_time {
            panic!("Voting period has not ended");
        }

        // Determine outcome (simple majority)
        if proposal.votes_for > proposal.votes_against {
            proposal.status = ProposalStatus::Passed;
        } else {
            proposal.status = ProposalStatus::Rejected;
        }

        // Store updated proposal
        env.storage().persistent().set(&key, &proposal);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);

        // Emit event
        env.events().publish(
            (symbol_short!("PROP_FIN"), proposal_id),
            (proposal.status.clone(), proposal.votes_for, proposal.votes_against),
        );
    }

    /// Get proposal details
    pub fn get_proposal(env: Env, proposal_id: u64) -> Proposal {
        let key = (PROPOSALS, proposal_id);
        env.storage().persistent().extend_ttl(&key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&key).expect("Proposal not found")
    }

    /// Get total proposal count
    pub fn get_proposal_count(env: Env) -> u64 {
        env.storage().persistent().extend_ttl(&PROPOSAL_COUNT, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&PROPOSAL_COUNT).unwrap_or(0)
    }

    /// Check if an address has voted on a proposal
    pub fn has_voted(env: Env, voter: Address, proposal_id: u64) -> bool {
        let vote_key = (VOTES, proposal_id, voter);
        env.storage().persistent().extend_ttl(&vote_key, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().has(&vote_key)
    }

    /// Get RBAC contract address
    pub fn get_rbac_address(env: Env) -> Address {
        env.storage().persistent().extend_ttl(&RBAC_ADDRESS, STORAGE_TTL_THRESHOLD, STORAGE_TTL_BUMP);
        env.storage().persistent().get(&RBAC_ADDRESS).expect("RBAC address not set")
    }
}

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{testutils::{Address as _, Ledger}, Address, Env, Symbol};
    use mentorminds_shared::ROLE_SUPER_ADMIN;

    fn setup_env() -> (Env, Address, Address, Address, Address) {
        let env = Env::default();
        let rbac_contract_id = env.register_contract(None, mentorminds_rbac::RbacContract);
        let governance_contract_id = env.register_contract(None, GovernanceContract);
        
        let super_admin = Address::generate(&env);
        let governance_admin = Address::generate(&env);
        let voter = Address::generate(&env);
        
        (env, rbac_contract_id, governance_contract_id, super_admin, governance_admin, voter)
    }

    #[test]
    fn test_initialize() {
        let (env, rbac_id, governance_id, _, _, _) = setup_env();
        let governance_client = GovernanceContractClient::new(&env, &governance_id);
        
        governance_client.initialize(&rbac_id);
        
        assert_eq!(governance_client.get_rbac_address(), rbac_id);
    }

    #[test]
    fn test_create_proposal_with_role() {
        let (env, rbac_id, governance_id, super_admin, governance_admin, _) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let governance_client = GovernanceContractClient::new(&env, &governance_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Governance
        governance_client.initialize(&rbac_id);
        
        // Grant governance admin role
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &governance_admin, &ROLE_GOVERNANCE_ADMIN);
        
        // Create proposal
        env.mock_all_auths();
        let proposal_id = governance_client.create_proposal(
            &governance_admin,
            &symbol_short!("TEST"),
            &symbol_short!("Description"),
            None,
        );
        
        assert_eq!(proposal_id, 1);
        
        let proposal = governance_client.get_proposal(&proposal_id);
        assert_eq!(proposal.title, symbol_short!("TEST"));
        assert_eq!(proposal.status, ProposalStatus::Active);
    }

    #[test]
    fn test_create_proposal_without_role() {
        let (env, rbac_id, governance_id, super_admin, governance_admin, _) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let governance_client = GovernanceContractClient::new(&env, &governance_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Governance
        governance_client.initialize(&rbac_id);
        
        // Try to create proposal without role - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            governance_client.create_proposal(
                &governance_admin,
                &symbol_short!("TEST"),
                &symbol_short!("Description"),
                None,
            );
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_vote() {
        let (env, rbac_id, governance_id, super_admin, governance_admin, voter) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let governance_client = GovernanceContractClient::new(&env, &governance_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Governance
        governance_client.initialize(&rbac_id);
        
        // Grant governance admin role
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &governance_admin, &ROLE_GOVERNANCE_ADMIN);
        
        // Create proposal
        env.mock_all_auths();
        let proposal_id = governance_client.create_proposal(
            &governance_admin,
            &symbol_short!("TEST"),
            &symbol_short!("Description"),
            None,
        );
        
        // Vote
        env.mock_all_auths();
        governance_client.vote(&voter, &proposal_id, true);
        
        assert!(governance_client.has_voted(&voter, &proposal_id));
        
        let proposal = governance_client.get_proposal(&proposal_id);
        assert_eq!(proposal.votes_for, 1);
    }

    #[test]
    fn test_finalize_proposal() {
        let (env, rbac_id, governance_id, super_admin, governance_admin, voter) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let governance_client = GovernanceContractClient::new(&env, &governance_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Governance
        governance_client.initialize(&rbac_id);
        
        // Grant governance admin role
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &governance_admin, &ROLE_GOVERNANCE_ADMIN);
        
        // Create proposal with short voting period
        env.mock_all_auths();
        let proposal_id = governance_client.create_proposal(
            &governance_admin,
            &symbol_short!("TEST"),
            &symbol_short!("Description"),
            Some(1),
        );
        
        // Vote
        env.mock_all_auths();
        governance_client.vote(&voter, &proposal_id, true);
        
        // Advance time past voting period
        env.ledger().set(env.ledger().seq() + 10, env.ledger().timestamp() + 10);
        
        // Finalize proposal
        env.mock_all_auths();
        governance_client.finalize_proposal(&governance_admin, &proposal_id);
        
        let proposal = governance_client.get_proposal(&proposal_id);
        assert_eq!(proposal.status, ProposalStatus::Passed);
    }

    #[test]
    fn test_execute_proposal() {
        let (env, rbac_id, governance_id, super_admin, governance_admin, voter) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let governance_client = GovernanceContractClient::new(&env, &governance_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Governance
        governance_client.initialize(&rbac_id);
        
        // Grant governance admin role
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &governance_admin, &ROLE_GOVERNANCE_ADMIN);
        
        // Create proposal with short voting period
        env.mock_all_auths();
        let proposal_id = governance_client.create_proposal(
            &governance_admin,
            &symbol_short!("TEST"),
            &symbol_short!("Description"),
            Some(1),
        );
        
        // Vote
        env.mock_all_auths();
        governance_client.vote(&voter, &proposal_id, true);
        
        // Advance time past voting period
        env.ledger().set(env.ledger().seq() + 10, env.ledger().timestamp() + 10);
        
        // Finalize proposal
        env.mock_all_auths();
        governance_client.finalize_proposal(&governance_admin, &proposal_id);
        
        // Execute proposal
        env.mock_all_auths();
        governance_client.execute_proposal(&governance_admin, &proposal_id);
        
        let proposal = governance_client.get_proposal(&proposal_id);
        assert_eq!(proposal.status, ProposalStatus::Executed);
    }

    #[test]
    fn test_execute_without_role() {
        let (env, rbac_id, governance_id, super_admin, governance_admin, voter) = setup_env();
        let rbac_client = mentorminds_rbac::RbacContractClient::new(&env, &rbac_id);
        let governance_client = GovernanceContractClient::new(&env, &governance_id);
        
        // Initialize RBAC
        rbac_client.initialize(&super_admin);
        
        // Initialize Governance
        governance_client.initialize(&rbac_id);
        
        // Grant governance admin role
        env.mock_all_auths();
        rbac_client.grant_role(&super_admin, &governance_admin, &ROLE_GOVERNANCE_ADMIN);
        
        // Create proposal with short voting period
        env.mock_all_auths();
        let proposal_id = governance_client.create_proposal(
            &governance_admin,
            &symbol_short!("TEST"),
            &symbol_short!("Description"),
            Some(1),
        );
        
        // Vote
        env.mock_all_auths();
        governance_client.vote(&voter, &proposal_id, true);
        
        // Advance time past voting period
        env.ledger().set(env.ledger().seq() + 10, env.ledger().timestamp() + 10);
        
        // Finalize proposal
        env.mock_all_auths();
        governance_client.finalize_proposal(&governance_admin, &proposal_id);
        
        // Revoke role
        env.mock_all_auths();
        rbac_client.revoke_role(&super_admin, &governance_admin, &ROLE_GOVERNANCE_ADMIN);
        
        // Try to execute without role - should panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            env.mock_all_auths();
            governance_client.execute_proposal(&governance_admin, &proposal_id);
        }));
        assert!(result.is_err());
    }
}

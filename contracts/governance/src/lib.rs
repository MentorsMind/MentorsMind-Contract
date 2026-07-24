#![no_std]

use shared::events::{
    emit_governance_event,
    evt_gov_appeal_resolved, evt_gov_appeal_submitted, evt_gov_arb_registered,
    evt_gov_arb_unregistered, evt_gov_call_allowed, evt_gov_proposal_cancelled,
    evt_gov_proposal_created, evt_gov_proposal_executed, evt_gov_proposal_failed,
    evt_gov_proposal_passed, evt_gov_proposal_queued, evt_gov_timelock_set, evt_gov_vote_cast,
};
use shared::StateMachine;
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, vec, Address, Bytes, BytesN, Env, IntoVal,
    Symbol, Vec,
};

// Instance storage: frequently read config
const ADMIN: Symbol = symbol_short!("ADMIN");
const TOKEN: Symbol = symbol_short!("TOKEN");
const SNAPSHOT: Symbol = symbol_short!("SNAPSHOT");
const PROPOSAL_COUNT: Symbol = symbol_short!("PROP_CNT");
const VOTING_PERIOD_SECS: Symbol = symbol_short!("VOT_PER");
const QUORUM_BPS: Symbol = symbol_short!("QRM_BPS");
const CURRENT_FEE_BPS: Symbol = symbol_short!("FEE_BPS");
const CURRENT_AUTO_RELEASE_SECS: Symbol = symbol_short!("AUTO_REL");

const DEFAULT_VOTING_PERIOD_SECS: u64 = 7 * 24 * 60 * 60;
const DEFAULT_QUORUM_BPS: u32 = 1_000; // 10%
const EXECUTE_CALL_TIMELOCK_SECS: u64 = 7 * 24 * 60 * 60; // 7-day mandatory delay

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalAction {
    UpdateFee(u32),
    UpdateAutoRelease(u64),
    AddAsset(Address),
    UpdateAdmin(Address),
    ExecuteCall(Address, Symbol),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ProposalStatus {
    Active,
    Passed,
    Queued,
    Failed,
    Executed,
    Cancelled,
}

impl StateMachine for ProposalStatus {
    type State = ProposalStatus;

    fn is_valid_transition(_env: &Env, from: &Self::State, to: &Self::State) -> bool {
        if *from == ProposalStatus::Active && *to == ProposalStatus::Passed { return true; }
        if *from == ProposalStatus::Active && *to == ProposalStatus::Failed { return true; }
        if *from == ProposalStatus::Active && *to == ProposalStatus::Cancelled { return true; }
        if *from == ProposalStatus::Passed && *to == ProposalStatus::Queued { return true; }
        if *from == ProposalStatus::Passed && *to == ProposalStatus::Executed { return true; }
        if *from == ProposalStatus::Queued && *to == ProposalStatus::Executed { return true; }
        false
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Proposal {
    pub id: u32,
    pub proposer: Address,
    pub title: Bytes,
    pub description_hash: BytesN<32>,
    pub action: ProposalAction,
    pub status: ProposalStatus,
    pub created_at: u64,
    pub voting_ends_at: u64,
    pub snapshot_ledger: u32,
    pub total_supply_snapshot: i128,
    pub votes_for: i128,
    pub votes_against: i128,
    pub timelock_op_id: BytesN<32>,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Proposal(u32),
    Vote(u32, Address),
    VoteWeight(u32, Address),
    ApprovedAsset(Address),
    Timelock,
    Arbitrator(Address),
    ArbitratorAt(u32),
    ArbitratorCount,
    ArbitratorIndex(Address),
    ArbitratorList,
    ArbitratorCompensation,
    Appeal(u32),
    AllowedCall(Address, Symbol),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArbitratorRecord {
    pub address: Address,
    pub active: bool,
    pub cases_handled: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AppealRecord {
    pub proposal_id: u32,
    pub appellant: Address,
    pub reason: soroban_sdk::String,
    pub submitted_at: u64,
    pub resolved: bool,
}

#[contract]
pub struct GovernanceContract;

#[contractimpl]
impl GovernanceContract {
    fn transition_proposal_status(env: &Env, proposal: &mut Proposal, to: ProposalStatus) {
        let from = proposal.status.clone();
        soroban_sdk::log!(env, "transitioning from {:?} to {:?}", from, to);
        if !ProposalStatus::is_valid_transition(env, &from, &to) {
            panic!("invalid transition from {:?} to {:?}", from, to);
        }
        proposal.status = to;
    }

    pub fn initialize(
        env: Env,
        admin: Address,
        mnt_token: Address,
        snapshot_contract: Address,
        voting_period_secs: Option<u64>,
        quorum_bps: Option<u32>,
    ) {
        if env.storage().instance().has(&ADMIN) {
            panic!("already initialized");
        }

        let period = voting_period_secs.unwrap_or(DEFAULT_VOTING_PERIOD_SECS);
        if period == 0 {
            panic!("invalid voting period");
        }

        let quorum = quorum_bps.unwrap_or(DEFAULT_QUORUM_BPS);
        if quorum == 0 || quorum > 10_000 {
            panic!("invalid quorum bps");
        }

        env.storage().instance().set(&ADMIN, &admin);
        env.storage().instance().set(&TOKEN, &mnt_token);
        env.storage().instance().set(&SNAPSHOT, &snapshot_contract);
        env.storage().instance().set(&VOTING_PERIOD_SECS, &period);
        env.storage().instance().set(&QUORUM_BPS, &quorum);
        env.storage().instance().set(&PROPOSAL_COUNT, &0u32);
        env.storage().persistent().set(&ADMIN, &admin);
        env.storage().persistent().set(&TOKEN, &mnt_token);

        env.storage()
            .persistent()
            .set(&SNAPSHOT, &snapshot_contract);
        env.storage().persistent().set(&VOTING_PERIOD_SECS, &period);

        env.storage().persistent().set(&QUORUM_BPS, &quorum);
        env.storage().persistent().set(&PROPOSAL_COUNT, &0u32);

    }

    pub fn set_timelock(env: Env, timelock: Address) {
        let admin: Address = env.storage().instance().get(&ADMIN).unwrap();
        admin.require_auth();
        env.storage()
            .persistent()
            .set(&DataKey::Timelock, &timelock);

        emit_governance_event(&env, evt_gov_timelock_set(&env), timelock);
    }

    /// Add a (target, function) pair to the ExecuteCall allowlist. Admin only.
    pub fn add_allowed_call(env: Env, admin: Address, target: Address, function: Symbol) {
        Self::assert_admin(&env, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::AllowedCall(target.clone(), function.clone()), &true);
        emit_governance_event(&env, evt_gov_call_allowed(&env), (target, function));
    }

    pub fn is_call_allowed(env: Env, target: Address, function: Symbol) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::AllowedCall(target, function))
            .unwrap_or(false)
    }

    pub fn create_proposal(
        env: Env,
        proposer: Address,
        title: Bytes,
        description_hash: BytesN<32>,
        action: ProposalAction,
    ) -> u32 {
        proposer.require_auth();
        Self::require_initialized(&env);

        // ExecuteCall proposals must target an allowlisted (contract, function) pair
        if let ProposalAction::ExecuteCall(ref target, ref function) = action {
            if !env.storage().persistent().get::<_, bool>(
                &DataKey::AllowedCall(target.clone(), function.clone())
            ).unwrap_or(false) {
                panic!("call not allowlisted");
            }
        }

        let mut count: u32 = env.storage().instance().get(&PROPOSAL_COUNT).unwrap_or(0);
        let mut count: u32 = env
            .storage()
            .persistent()
            .get(&PROPOSAL_COUNT)
            .unwrap_or(0);
        count = count.checked_add(1).expect("proposal overflow");

        let now = env.ledger().timestamp();
        let voting_period_secs: u64 = env
            .storage()
            .instance()
            .get(&VOTING_PERIOD_SECS)
            .unwrap_or(DEFAULT_VOTING_PERIOD_SECS);

        let snapshot_contract: Address = env
            .storage()
            .instance()
            .get(&SNAPSHOT)
            .expect("snapshot not set");
        env.invoke_contract::<()>(
            &snapshot_contract,
            &Symbol::new(&env, "record_snapshot"),
            (count,).into_val(&env),
        );

        let total_supply_snapshot: i128 = env.invoke_contract(
            &snapshot_contract,
            &Symbol::new(&env, "get_total_supply_at"),
            (count,).into_val(&env),
        );

        let proposal = Proposal {
            id: count,
            proposer: proposer.clone(),
            title,
            description_hash,
            action,
            status: ProposalStatus::Active,
            created_at: now,
            voting_ends_at: now
                .checked_add(voting_period_secs)
                .expect("voting end overflow"),
            snapshot_ledger: env.ledger().sequence(),
            total_supply_snapshot,
            votes_for: 0,
            votes_against: 0,
            timelock_op_id: BytesN::from_array(&env, &[0; 32]),
        };

        env.storage().instance().set(&PROPOSAL_COUNT, &count);
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(count), &proposal);

        emit_governance_event(
            &env,
            evt_gov_proposal_created(&env),
            (proposer, proposal.snapshot_ledger, proposal.voting_ends_at),
        );

        count
    }

    pub fn vote(env: Env, voter: Address, proposal_id: u32, support: bool) {
        voter.require_auth();
        let mut proposal = Self::get_proposal(env.clone(), proposal_id);
        Self::require_active_proposal(&env, &proposal);

        let key = DataKey::Vote(proposal_id, voter.clone());
        if env.storage().persistent().has(&key) {
            panic!("already voted");
        }

        let snapshot_contract: Address = env
            .storage()
            .instance()
            .get(&SNAPSHOT)
            .expect("snapshot not set");
        let weight: i128 = env.invoke_contract(
            &snapshot_contract,
            &Symbol::new(&env, "get_voting_power"),
            (proposal_id, voter.clone()).into_val(&env),
        );

        if weight <= 0 {
            panic!("no voting power");
        }

        if support {
            proposal.votes_for = proposal
                .votes_for
                .checked_add(weight)
                .expect("votes for overflow");
        } else {
            proposal.votes_against = proposal
                .votes_against
                .checked_add(weight)
                .expect("votes against overflow");
        }

        env.storage().persistent().set(&key, &support);
        env.storage()
            .persistent()
            .set(&DataKey::VoteWeight(proposal_id, voter.clone()), &weight);
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);

        emit_governance_event(&env, evt_gov_vote_cast(&env), (voter, support, weight));
    }

    pub fn execute_proposal(env: Env, proposal_id: u32) {
        let mut proposal = Self::get_proposal(env.clone(), proposal_id);

        if proposal.status == ProposalStatus::Executed || proposal.status == ProposalStatus::Queued {
            panic!("proposal already executed or queued");
        }

        if env.ledger().timestamp() < proposal.voting_ends_at {
            panic!("voting period not ended");
        }

        if proposal.status == ProposalStatus::Cancelled || proposal.status == ProposalStatus::Failed
        {
            panic!("proposal not executable");
        }

        let quorum_bps: u32 = env
            .storage()
            .instance()
            .get(&QUORUM_BPS)
            .unwrap_or(DEFAULT_QUORUM_BPS);
        let total_votes = proposal
            .votes_for
            .checked_add(proposal.votes_against)
            .expect("vote overflow");

        let quorum_met = if proposal.total_supply_snapshot <= 0 {
            false
        } else {
            total_votes.checked_mul(10_000).expect("quorum overflow")
                >= proposal
                    .total_supply_snapshot
                    .checked_mul(quorum_bps as i128)
                    .expect("quorum threshold overflow")
        };

        let passed = quorum_met && proposal.votes_for > proposal.votes_against;

        if !passed {
            Self::transition_proposal_status(&env, &mut proposal, ProposalStatus::Failed);
            env.storage()
                .persistent()
                .set(&DataKey::Proposal(proposal_id), &proposal);
            emit_governance_event(
                &env,
                evt_gov_proposal_failed(&env),
                (proposal.votes_for, proposal.votes_against, quorum_met),
            );
            return;
        }

        Self::transition_proposal_status(&env, &mut proposal, ProposalStatus::Passed);
        emit_governance_event(
            &env,
            evt_gov_proposal_passed(&env),
            (proposal.votes_for, proposal.votes_against, quorum_met),
        );

        // ExecuteCall requires an additional 7-day delay after voting ends
        if let ProposalAction::ExecuteCall(_, _) = &proposal.action {
            let earliest_execute = proposal
                .voting_ends_at
                .checked_add(EXECUTE_CALL_TIMELOCK_SECS)
                .expect("timelock overflow");
            if env.ledger().timestamp() < earliest_execute {
                panic!("ExecuteCall timelock not elapsed");
            }
            // Get timelock contract
            let timelock: Address = env.storage().persistent().get(&DataKey::Timelock).expect("timelock not set");
            
            // Use the governance contract address as the caller for the timelock schedule
            let gov_address = env.current_contract_address();

            // Schedule the action to be executed by the timelock
            let delay: u64 = 48 * 60 * 60; // 48 hours, as per timelock's MIN_DELAY
            let mut args: Vec<soroban_sdk::Val> = Vec::new(&env);
            args.push_back(proposal_id.into_val(&env));
            let op_id: BytesN<32> = env.invoke_contract::<BytesN<32>>(
                &timelock,
                &Symbol::new(&env, "schedule"),
                (
                    gov_address.clone(),
                    gov_address,
                    Symbol::new(&env, "execute_queued_proposal"),
                    args,
                    delay,
                ).into_val(&env),
            );

            proposal.timelock_op_id = op_id.clone();
            Self::transition_proposal_status(&env, &mut proposal, ProposalStatus::Queued);
            
            env.storage()
                .persistent()
                .set(&DataKey::Proposal(proposal_id), &proposal);

            emit_governance_event(&env, evt_gov_proposal_queued(&env), op_id);
        } else {
            Self::apply_action(&env, &proposal.action);
            Self::transition_proposal_status(&env, &mut proposal, ProposalStatus::Executed);
            
            env.storage()
                .persistent()
                .set(&DataKey::Proposal(proposal_id), &proposal);

            emit_governance_event(&env, evt_gov_proposal_executed(&env), true);
        }
    }

    /// Execute a queued proposal after timelock delay. Can only be called by the timelock.
    pub fn execute_queued_proposal(env: Env, proposal_id: u32) {
        // Check that caller is the timelock
        let timelock: Address = env.storage().persistent().get(&DataKey::Timelock).expect("timelock not set");
        timelock.require_auth();

        let mut proposal = Self::get_proposal(env.clone(), proposal_id);

        if proposal.status != ProposalStatus::Queued {
            panic!("proposal not queued");
        }

        Self::apply_action(&env, &proposal.action);
        Self::transition_proposal_status(&env, &mut proposal, ProposalStatus::Executed);
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);

        emit_governance_event(&env, evt_gov_proposal_executed(&env), true);
    }

    pub fn cancel_proposal(env: Env, proposal_id: u32) {
        let admin: Address = env
            .storage()
            .instance()
            .get(&ADMIN)
            .expect("not initialized");
        admin.require_auth();

        let mut proposal = Self::get_proposal(env.clone(), proposal_id);

        match proposal.status {
            ProposalStatus::Executed => panic!("cannot cancel executed proposal"),
            ProposalStatus::Failed => panic!("cannot cancel failed proposal"),
            ProposalStatus::Cancelled => panic!("proposal already cancelled"),
            _ => {}
        }

        proposal.status = ProposalStatus::Cancelled;
        env.storage()
            .persistent()
            .set(&DataKey::Proposal(proposal_id), &proposal);

        emit_governance_event(&env, evt_gov_proposal_cancelled(&env), proposal.proposer.clone());
    }

    /// Register an arbitrator for dispute resolution (#470).
    pub fn register_arbitrator(env: Env, admin: Address, arbitrator: Address) {
        Self::assert_admin(&env, &admin);
        let record = ArbitratorRecord { address: arbitrator.clone(), active: true, cases_handled: 0 };
        let key = DataKey::Arbitrator(arbitrator.clone());
        let is_new = !env.storage().persistent().has(&key);
        env.storage().persistent().set(&key, &record);

        if is_new {
            let count: u32 = env.storage().persistent().get(&DataKey::ArbitratorCount).unwrap_or(0);
            env.storage().persistent().set(&DataKey::ArbitratorAt(count), &arbitrator);
            env.storage().persistent().set(&DataKey::ArbitratorIndex(arbitrator.clone()), &count);
            env.storage().persistent().set(&DataKey::ArbitratorCount, &(count + 1));
        }

        emit_governance_event(&env, evt_gov_arb_registered(&env), arbitrator);
    }

    pub fn unregister_arbitrator(env: Env, admin: Address, arbitrator: Address) {
        Self::assert_admin(&env, &admin);
        let key = DataKey::Arbitrator(arbitrator.clone());
        if !env.storage().persistent().has(&key) {
            panic!("arbitrator not found");
        }
        
        let count: u32 = env.storage().persistent().get(&DataKey::ArbitratorCount).unwrap_or(0);
        if count == 0 {
            panic!("no arbitrators");
        }
        
        let index: u32 = env.storage().persistent().get(&DataKey::ArbitratorIndex(arbitrator.clone())).expect("arbitrator index not found");
        let last_index = count - 1;
        
        if index != last_index {
            let last_arbitrator: Address = env.storage().persistent().get(&DataKey::ArbitratorAt(last_index)).expect("last arbitrator not found");
            env.storage().persistent().set(&DataKey::ArbitratorAt(index), &last_arbitrator);
            env.storage().persistent().set(&DataKey::ArbitratorIndex(last_arbitrator), &index);
        }
        
        env.storage().persistent().remove(&DataKey::ArbitratorAt(last_index));
        env.storage().persistent().remove(&DataKey::ArbitratorIndex(arbitrator.clone()));
        env.storage().persistent().remove(&key);
        env.storage().persistent().set(&DataKey::ArbitratorCount, &last_index);
        
        emit_governance_event(&env, evt_gov_arb_unregistered(&env), arbitrator);
    }

    pub fn get_arbitrator_count(env: Env) -> u32 {
        env.storage().persistent().get(&DataKey::ArbitratorCount).unwrap_or(0)
    }

    pub fn list_arbitrators_page(env: Env, offset: u32, limit: u32) -> Vec<Address> {
        let count = Self::get_arbitrator_count(env.clone());
        let mut out = Vec::new(&env);
        let end = (offset + limit).min(count);
        for i in offset..end {
            if let Some(addr) = env.storage().persistent().get::<_, Address>(&DataKey::ArbitratorAt(i)) {
                out.push_back(addr);
            }
        }
        out
    }

    pub fn select_arbitrator(env: Env, dispute_id: u64) -> Address {
        let count = Self::get_arbitrator_count(env.clone());
        if count == 0 {
            panic!("no arbitrators");
        }
        
        let start_idx = (dispute_id % (count as u64)) as u32;
        let mut idx = start_idx;
        loop {
            let addr: Address = env.storage().persistent().get(&DataKey::ArbitratorAt(idx)).expect("invalid arbitrator index");
            let record: ArbitratorRecord = env.storage().persistent().get(&DataKey::Arbitrator(addr.clone())).expect("arbitrator record not found");
            if record.active {
                return addr;
            }
            idx = (idx + 1) % count;
            if idx == start_idx {
                panic!("no active arbitrators");
            }
        }
    }

    pub fn migrate_arbitrators(env: Env, admin: Address) {
        Self::assert_admin(&env, &admin);
        if let Some(list) = env.storage().persistent().get::<_, Vec<Address>>(&DataKey::ArbitratorList) {
            let mut count: u32 = env.storage().persistent().get(&DataKey::ArbitratorCount).unwrap_or(0);
            for addr in list.iter() {
                if !env.storage().persistent().has(&DataKey::ArbitratorIndex(addr.clone())) {
                    env.storage().persistent().set(&DataKey::ArbitratorAt(count), &addr);
                    env.storage().persistent().set(&DataKey::ArbitratorIndex(addr.clone()), &count);
                    count += 1;
                }
            }
            env.storage().persistent().set(&DataKey::ArbitratorCount, &count);
            env.storage().persistent().remove(&DataKey::ArbitratorList);
        }
    }

    pub fn set_arbitration_compensation(env: Env, admin: Address, amount: i128) {
        admin.require_auth();
        env.storage().instance().set(&DataKey::ArbitratorCompensation, &amount);
    }

    pub fn get_arbitration_compensation(env: Env) -> i128 {
        env.storage().instance().get(&DataKey::ArbitratorCompensation).unwrap_or(0)
    }

    pub fn get_arbitrator(env: Env, arbitrator: Address) -> ArbitratorRecord {
        env.storage().persistent().get(&DataKey::Arbitrator(arbitrator)).expect("arbitrator not found")
    }

    /// Submit an appeal for a resolved proposal (#469).
    pub fn submit_appeal(env: Env, appellant: Address, proposal_id: u32, reason: soroban_sdk::String) {
        appellant.require_auth();
        let appeal = AppealRecord {
            proposal_id,
            appellant,
            reason,
            submitted_at: env.ledger().timestamp(),
            resolved: false,
        };
        env.storage().persistent().set(&DataKey::Appeal(proposal_id), &appeal);
        emit_governance_event(&env, evt_gov_appeal_submitted(&env), (appellant, proposal_id));
    }

    pub fn resolve_appeal(env: Env, arbitrator: Address, proposal_id: u32) {
        arbitrator.require_auth();
        let record_check: ArbitratorRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Arbitrator(arbitrator.clone()))
            .expect("arbitrator not found");
        if !record_check.active {
            panic!("arbitrator inactive");
        }
        let mut appeal: AppealRecord = env.storage().persistent()
            .get(&DataKey::Appeal(proposal_id)).expect("appeal not found");
        appeal.resolved = true;
        env.storage().persistent().set(&DataKey::Appeal(proposal_id), &appeal);
        let mut record: ArbitratorRecord = env.storage().persistent()
            .get(&DataKey::Arbitrator(arbitrator.clone())).expect("arbitrator not found");
        record.cases_handled += 1;
        env.storage().persistent().set(&DataKey::Arbitrator(arbitrator.clone()), &record);
        emit_governance_event(&env, evt_gov_appeal_resolved(&env), (arbitrator, proposal_id));
    }

    pub fn get_appeal(env: Env, proposal_id: u32) -> AppealRecord {
        env.storage().persistent().get(&DataKey::Appeal(proposal_id)).expect("appeal not found")
    }

    pub fn get_proposal(env: Env, id: u32) -> Proposal {
        env.storage()
            .persistent()
            .get(&DataKey::Proposal(id))
            .expect("proposal not found")
    }

    pub fn get_vote(env: Env, id: u32, voter: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Vote(id, voter))
            .unwrap_or(false)
    }

    pub fn get_vote_weight(env: Env, id: u32, voter: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::VoteWeight(id, voter))
            .unwrap_or(0)
    }

    fn require_initialized(env: &Env) {
        if !env.storage().instance().has(&ADMIN) {
            panic!("not initialized");
        }
    }

    fn assert_admin(env: &Env, admin: &Address) {
        admin.require_auth();
        let stored: Address = env
            .storage()
            .instance()
            .get(&ADMIN)
            .expect("not initialized");
        if &stored != admin {
            panic!("unauthorized");
        }
    }

    fn require_active_proposal(env: &Env, proposal: &Proposal) {
        if proposal.status != ProposalStatus::Active {
            panic!("proposal not active");
        }

        if env.ledger().timestamp() >= proposal.voting_ends_at {
            panic!("voting period ended");
        }
    }

    #[allow(dead_code)]
    fn token_address(env: &Env) -> Address {
        env.storage().instance().get(&TOKEN).expect("token not set")
    }

    #[allow(dead_code)]
    fn get_balance(env: &Env, addr: &Address) -> i128 {
        let token = Self::token_address(env);
        let fn_name = Symbol::new(env, "balance");
        let args = vec![env, addr.clone().into_val(env)];
        env.invoke_contract::<i128>(&token, &fn_name, args)
    }

    #[allow(dead_code)]
    fn get_total_supply(env: &Env) -> i128 {
        let token = Self::token_address(env);
        let fn_name = Symbol::new(env, "total_supply");
        let args = vec![env];
        env.invoke_contract::<i128>(&token, &fn_name, args)
    }

    fn apply_action(env: &Env, action: &ProposalAction) {
        match action {
            ProposalAction::UpdateFee(new_fee_bps) => {
                env.storage().instance().set(&CURRENT_FEE_BPS, new_fee_bps);
            }
            ProposalAction::UpdateAutoRelease(new_delay) => {
                env.storage()
                    .instance()
                    .set(&CURRENT_AUTO_RELEASE_SECS, new_delay);
            }
            ProposalAction::AddAsset(asset) => {
                env.storage()
                    .persistent()
                    .set(&DataKey::ApprovedAsset(asset.clone()), &true);
            }
            ProposalAction::UpdateAdmin(new_admin) => {
                env.storage().instance().set(&ADMIN, new_admin);
            }
            ProposalAction::ExecuteCall(target, function) => {
                env.invoke_contract::<soroban_sdk::Val>(target, function, vec![env]);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    extern crate std;

    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};

    #[contract]
    pub struct MockMntToken;

    #[contractimpl]
    impl MockMntToken {
        pub fn set_total_supply(env: Env, amount: i128) {
            env.storage()
                .persistent()
                .set(&symbol_short!("TOT_SUP"), &amount);
        }

        pub fn set_balance(env: Env, addr: Address, amount: i128) {
            env.storage()
                .persistent()
                .set(&(symbol_short!("BAL"), addr), &amount);
        }

        pub fn balance(env: Env, addr: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&(symbol_short!("BAL"), addr))
                .unwrap_or(0)
        }

        pub fn total_supply(env: Env) -> i128 {
            env.storage()
                .persistent()
                .get(&symbol_short!("TOT_SUP"))
                .unwrap_or(0)
        }
    }

    #[contract]
    pub struct MockSnapshot;

    #[contractimpl]
    impl MockSnapshot {
        pub fn record_snapshot(env: Env, _id: u32) {
            env.storage()
                .persistent()
                .set(&symbol_short!("TOT_SUP"), &1000i128);
        }
        pub fn get_total_supply_at(env: Env, _id: u32) -> i128 {
            env.storage()
                .persistent()
                .get(&symbol_short!("TOT_SUP"))
                .unwrap_or(0)
        }
        pub fn get_voting_power(env: Env, _id: u32, voter: Address) -> i128 {
            let token: Address = env
                .storage()
                .persistent()
                .get(&symbol_short!("TOKEN"))
                .unwrap();
            let args = vec![&env, voter.into_val(&env)];
            env.invoke_contract::<i128>(&token, &Symbol::new(&env, "balance"), args)
        }
        pub fn set_token(env: Env, token: Address) {
            env.storage()
                .persistent()
                .set(&symbol_short!("TOKEN"), &token);
        }
    }

    #[test]
    fn test_full_proposal_lifecycle() {
        let env = Env::default();
        env.mock_all_auths();

        let gov_id = env.register_contract(None, GovernanceContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let gov = GovernanceContractClient::new(&env, &gov_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let snapshot = MockSnapshotClient::new(&env, &snapshot_id);
        snapshot.set_token(&token_id);

        let admin = Address::generate(&env);
        let voter = Address::generate(&env);
        gov.initialize(
            &admin,
            &token_id,
            &snapshot_id,
            &Some(10u64),
            &Some(1_000u32),
        );
        token.set_total_supply(&1_000i128);
        token.set_balance(&voter, &200i128);

        let title = Bytes::from_slice(&env, b"Update fee");
        let description_hash = BytesN::from_array(&env, &[1u8; 32]);
        let proposal_id = gov.create_proposal(
            &voter,
            &title,
            &description_hash,
            &ProposalAction::UpdateFee(300),
        );

        gov.vote(&voter, &proposal_id, &true);
        assert!(gov.get_vote(&proposal_id, &voter));

        env.ledger().set_timestamp(env.ledger().timestamp() + 11);
        gov.execute_proposal(&proposal_id);

        let proposal = gov.get_proposal(&proposal_id);
        assert_eq!(proposal.status, ProposalStatus::Executed);
    }

    #[test]
    fn test_quorum_failure() {
        let env = Env::default();
        env.mock_all_auths();

        let gov_id = env.register_contract(None, GovernanceContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let gov = GovernanceContractClient::new(&env, &gov_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let snapshot = MockSnapshotClient::new(&env, &snapshot_id);
        snapshot.set_token(&token_id);

        let admin = Address::generate(&env);
        let voter = Address::generate(&env);
        gov.initialize(
            &admin,
            &token_id,
            &snapshot_id,
            &Some(10u64),
            &Some(1_000u32),
        );

        token.set_total_supply(&10_000i128);
        token.set_balance(&voter, &50i128);

        let title = Bytes::from_slice(&env, b"Raise delay");
        let description_hash = BytesN::from_array(&env, &[2u8; 32]);
        let proposal_id = gov.create_proposal(
            &voter,
            &title,
            &description_hash,
            &ProposalAction::UpdateAutoRelease(86_400),
        );

        gov.vote(&voter, &proposal_id, &true);
        env.ledger().set_timestamp(env.ledger().timestamp() + 11);
        gov.execute_proposal(&proposal_id);

        let proposal = gov.get_proposal(&proposal_id);
        assert_eq!(proposal.status, ProposalStatus::Failed);
    }

    #[test]
    #[should_panic(expected = "already voted")]
    fn test_double_vote_prevention() {
        let env = Env::default();
        env.mock_all_auths();

        let gov_id = env.register_contract(None, GovernanceContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let gov = GovernanceContractClient::new(&env, &gov_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let snapshot = MockSnapshotClient::new(&env, &snapshot_id);
        snapshot.set_token(&token_id);

        let admin = Address::generate(&env);
        let voter = Address::generate(&env);
        gov.initialize(
            &admin,
            &token_id,
            &snapshot_id,
            &Some(10u64),
            &Some(1_000u32),
        );
        token.set_total_supply(&1_000i128);
        token.set_balance(&voter, &200i128);

        let title = Bytes::from_slice(&env, b"Asset listing");
        let description_hash = BytesN::from_array(&env, &[3u8; 32]);
        let proposal_id = gov.create_proposal(
            &voter,
            &title,
            &description_hash,
            &ProposalAction::AddAsset(Address::generate(&env)),
        );

        gov.vote(&voter, &proposal_id, &true);
        gov.vote(&voter, &proposal_id, &false);
    }

    #[contract]
    pub struct MockTarget;

    #[contractimpl]
    impl MockTarget {
        pub fn do_thing(_env: Env) {}
    }

    fn setup(env: &Env) -> (
        GovernanceContractClient,
        Address, // admin
        Address, // voter
        Address, // token_id
        Address, // snapshot_id
    ) {
        let gov_id = env.register_contract(None, GovernanceContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let gov = GovernanceContractClient::new(env, &gov_id);
        let token = MockMntTokenClient::new(env, &token_id);
        let snapshot = MockSnapshotClient::new(env, &snapshot_id);
        snapshot.set_token(&token_id);
        let admin = Address::generate(env);
        let voter = Address::generate(env);
        gov.initialize(&admin, &token_id, &snapshot_id, &Some(10u64), &Some(1_000u32));
        token.set_total_supply(&1_000i128);
        token.set_balance(&voter, &600i128);
        (gov, admin, voter, token_id, snapshot_id)
    }

    #[test]
    #[should_panic(expected = "call not allowlisted")]
    fn test_execute_call_rejected_if_not_allowlisted() {
        let env = Env::default();
        env.mock_all_auths();
        let (gov, _admin, voter, _, _) = setup(&env);

        let target = Address::generate(&env);
        let title = Bytes::from_slice(&env, b"Exec call");
        let description_hash = BytesN::from_array(&env, &[9u8; 32]);
        gov.create_proposal(
            &voter,
            &title,
            &description_hash,
            &ProposalAction::ExecuteCall(target, Symbol::new(&env, "do_thing")),
        );
    }

    #[test]
    #[should_panic(expected = "ExecuteCall timelock not elapsed")]
    fn test_execute_call_timelock_enforced() {
        let env = Env::default();
        env.mock_all_auths();
        let (gov, admin, voter, _, _) = setup(&env);

        let target_id = env.register_contract(None, MockTarget);
        let fn_name = Symbol::new(&env, "do_thing");
        gov.add_allowed_call(&admin, &target_id, &fn_name);

        let title = Bytes::from_slice(&env, b"Exec call");
        let description_hash = BytesN::from_array(&env, &[10u8; 32]);
        let proposal_id = gov.create_proposal(
            &voter,
            &title,
            &description_hash,
            &ProposalAction::ExecuteCall(target_id, fn_name),
        );

        gov.vote(&voter, &proposal_id, &true);
        // Advance past voting period but NOT past the 7-day timelock
        env.ledger().set_timestamp(env.ledger().timestamp() + 11);
        gov.execute_proposal(&proposal_id); // should panic
    }

    #[test]
    fn test_execute_call_succeeds_after_timelock() {
        let env = Env::default();
        env.mock_all_auths();
        let (gov, admin, voter, _, _) = setup(&env);

        let target_id = env.register_contract(None, MockTarget);
        let timelock_id = env.register_contract(None, MockTimelock);
        let fn_name = Symbol::new(&env, "do_thing");
        gov.add_allowed_call(&admin, &target_id, &fn_name);
        gov.set_timelock(&timelock_id);

        let title = Bytes::from_slice(&env, b"Exec call");
        let description_hash = BytesN::from_array(&env, &[11u8; 32]);
        let proposal_id = gov.create_proposal(
            &voter,
            &title,
            &description_hash,
            &ProposalAction::ExecuteCall(target_id, fn_name),
        );

        gov.vote(&voter, &proposal_id, &true);
        // Advance past voting period AND 7-day timelock
        env.ledger().set_timestamp(env.ledger().timestamp() + 10 + 7 * 24 * 60 * 60 + 1);
        gov.execute_proposal(&proposal_id);

        let proposal = gov.get_proposal(&proposal_id);
        assert_eq!(proposal.status, ProposalStatus::Queued);
    }

    #[test]
    fn test_arbitrator_registry_and_selection() {
        let env = Env::default();
        env.mock_all_auths();

        let gov_id = env.register_contract(None, GovernanceContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let gov = GovernanceContractClient::new(&env, &gov_id);

        let admin = Address::generate(&env);
        gov.initialize(&admin, &token_id, &snapshot_id, &Some(10u64), &Some(1_000u32));

        let a1 = Address::generate(&env);
        let a2 = Address::generate(&env);
        gov.register_arbitrator(&admin, &a1);
        gov.register_arbitrator(&admin, &a2);

        let list = gov.list_arbitrators_page(&0, &10);
        assert_eq!(list.len(), 2);

        let selected = gov.select_arbitrator(&7u64);
        assert!(selected == a1 || selected == a2);
    }

    #[test]
    #[should_panic(expected = "cannot cancel failed proposal")]
    fn test_cancel_failed_proposal_panics() {
        let env = Env::default();
        env.mock_all_auths();

        let gov_id = env.register_contract(None, GovernanceContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let gov = GovernanceContractClient::new(&env, &gov_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let snapshot = MockSnapshotClient::new(&env, &snapshot_id);
        snapshot.set_token(&token_id);

        let admin = Address::generate(&env);
        let voter = Address::generate(&env);
        gov.initialize(
            &admin,
            &token_id,
            &snapshot_id,
            &Some(10u64),
            &Some(1_000u32),
        );

        // Make quorum fail => proposal transitions to Failed
        token.set_total_supply(&10_000i128);
        token.set_balance(&voter, &50i128);

        let title = Bytes::from_slice(&env, b"Raise delay");
        let description_hash = BytesN::from_array(&env, &[9u8; 32]);
        let proposal_id = gov.create_proposal(
            &voter,
            &title,
            &description_hash,
            &ProposalAction::UpdateAutoRelease(86_400),
        );

        gov.vote(&voter, &proposal_id, &true);
        env.ledger().set_timestamp(env.ledger().timestamp() + 11);
        gov.execute_proposal(&proposal_id);

        let proposal = gov.get_proposal(&proposal_id);
        assert_eq!(proposal.status, ProposalStatus::Failed);

        // Now cancel should panic
        gov.cancel_proposal(&proposal_id);
    }

    #[test]
    #[should_panic(expected = "proposal already cancelled")]
    fn test_cancel_cancelled_proposal_panics() {
        let env = Env::default();
        env.mock_all_auths();

        let gov_id = env.register_contract(None, GovernanceContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let gov = GovernanceContractClient::new(&env, &gov_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let snapshot = MockSnapshotClient::new(&env, &snapshot_id);
        snapshot.set_token(&token_id);

        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        gov.initialize(
            &admin,
            &token_id,
            &snapshot_id,
            &Some(10u64),
            &Some(1_000u32),
        );

        // Token values aren't used for cancellation of an Active proposal.
        token.set_total_supply(&1_000i128);
        token.set_balance(&proposer, &200i128);

        let title = Bytes::from_slice(&env, b"Update fee");
        let description_hash = BytesN::from_array(&env, &[10u8; 32]);
        let proposal_id = gov.create_proposal(
            &proposer,
            &title,
            &description_hash,
            &ProposalAction::UpdateFee(300),
        );

        // First cancel succeeds
        gov.cancel_proposal(&proposal_id);
        let proposal = gov.get_proposal(&proposal_id);
        assert_eq!(proposal.status, ProposalStatus::Cancelled);

        // Second cancel panics
        gov.cancel_proposal(&proposal_id);
    }

    #[contract]
    pub struct MockTimelock;

    #[contractimpl]
    impl MockTimelock {
        pub fn schedule(
            env: Env,
            _target: Address,
            _caller: Address,
            _function: Symbol,
            _args: Vec<soroban_sdk::Val>,
            _delay: u64,
        ) -> BytesN<32> {
            BytesN::from_array(&env, &[7u8; 32])
        }
    }

    #[test]
    fn test_update_fee_proposal_executes_immediately() {
        let env = Env::default();
        env.mock_all_auths();

        let gov_id = env.register_contract(None, GovernanceContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let gov = GovernanceContractClient::new(&env, &gov_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let snapshot = MockSnapshotClient::new(&env, &snapshot_id);
        snapshot.set_token(&token_id);

        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        gov.initialize(
            &admin,
            &token_id,
            &snapshot_id,
            &Some(10u64),
            &Some(1_000u32),
        );

        token.set_total_supply(&1_000i128);
        token.set_balance(&proposer, &200i128);

        let title = Bytes::from_slice(&env, b"Update fee to 500");
        let description_hash = BytesN::from_array(&env, &[12u8; 32]);
        let proposal_id = gov.create_proposal(
            &proposer,
            &title,
            &description_hash,
            &ProposalAction::UpdateFee(500),
        );

        gov.vote(&proposer, &proposal_id, &true);
        env.ledger().set_timestamp(env.ledger().timestamp() + 11);
        gov.execute_proposal(&proposal_id);

        let proposal = gov.get_proposal(&proposal_id);
        assert_eq!(proposal.status, ProposalStatus::Executed);
        
        let current_fee: u32 = env.as_contract(&gov_id, || {
            env.storage().instance().get(&symbol_short!("FEE_BPS")).unwrap_or(0)
        });
        assert_eq!(current_fee, 500);
    }

    #[test]
    fn test_execute_call_transitions_to_queued() {
        let env = Env::default();
        env.mock_all_auths();

        let gov_id = env.register_contract(None, GovernanceContract);
        let token_id = env.register_contract(None, MockMntToken);
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let timelock_id = env.register_contract(None, MockTimelock);
        let gov = GovernanceContractClient::new(&env, &gov_id);
        let token = MockMntTokenClient::new(&env, &token_id);
        let snapshot = MockSnapshotClient::new(&env, &snapshot_id);
        snapshot.set_token(&token_id);

        let admin = Address::generate(&env);
        let proposer = Address::generate(&env);
        gov.initialize(
            &admin,
            &token_id,
            &snapshot_id,
            &Some(10u64),
            &Some(1_000u32),
        );
        
        // Admin needs auth to set timelock
        gov.set_timelock(&timelock_id);

        token.set_total_supply(&1_000i128);
        token.set_balance(&proposer, &200i128);

        let title = Bytes::from_slice(&env, b"Execute External Call");
        let description_hash = BytesN::from_array(&env, &[13u8; 32]);
        let dummy_target = Address::generate(&env);
        gov.add_allowed_call(&admin, &dummy_target, &Symbol::new(&env, "some_func"));
        let proposal_id = gov.create_proposal(
            &proposer,
            &title,
            &description_hash,
            &ProposalAction::ExecuteCall(dummy_target, Symbol::new(&env, "some_func")),
        );

        gov.vote(&proposer, &proposal_id, &true);
        
        let voting_ends_at = gov.get_proposal(&proposal_id).voting_ends_at;
        // Advance time to just after voting ends, PLUS the EXECUTE_CALL_TIMELOCK_SECS
        // since ExecuteCall enforces a 7-day delay (EXECUTE_CALL_TIMELOCK_SECS) before executing
        // wait, I need to look up EXECUTE_CALL_TIMELOCK_SECS from lib.rs
        let execute_call_delay = 7 * 24 * 60 * 60; 
        env.ledger().set_timestamp(voting_ends_at + execute_call_delay + 1);
        
        gov.execute_proposal(&proposal_id);

        let proposal = gov.get_proposal(&proposal_id);
        assert_eq!(proposal.status, ProposalStatus::Queued);
    }
}


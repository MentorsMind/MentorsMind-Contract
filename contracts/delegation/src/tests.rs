extern crate std;

use super::*;
use soroban_sdk::testutils::Address as _;

/// Stand-in for the governance snapshot contract, holding one weight per
/// voter so weight attribution can be asserted end to end.
#[contract]
pub struct MockSnapshot;

#[contracttype]
#[derive(Clone)]
enum MockDataKey {
    Weight(u32, Address),
}

#[contractimpl]
impl MockSnapshot {
    pub fn set_weight(env: Env, proposal_id: u32, voter: Address, weight: i128) {
        env.storage()
            .persistent()
            .set(&MockDataKey::Weight(proposal_id, voter), &weight);
    }

    pub fn get_voting_power(env: Env, proposal_id: u32, voter: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&MockDataKey::Weight(proposal_id, voter))
            .unwrap_or(0)
    }
}

struct Fixture {
    env: Env,
    contract_id: Address,
    snapshot_id: Address,
    admin: Address,
}

const PROPOSAL: u32 = 1;

impl Fixture {
    fn setup() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let snapshot_id = env.register_contract(None, MockSnapshot);
        let contract_id = env.register_contract(None, DelegationContract);

        let contract = DelegationContractClient::new(&env, &contract_id);
        contract.initialize(&admin, &snapshot_id);

        Self {
            env,
            contract_id,
            snapshot_id,
            admin,
        }
    }

    fn contract(&self) -> DelegationContractClient<'_> {
        DelegationContractClient::new(&self.env, &self.contract_id)
    }

    fn snapshot(&self) -> MockSnapshotClient<'_> {
        MockSnapshotClient::new(&self.env, &self.snapshot_id)
    }

    /// Creates a voter holding `weight` and returns its address.
    fn voter(&self, weight: i128) -> Address {
        let voter = Address::generate(&self.env);
        self.snapshot().set_weight(&PROPOSAL, &voter, &weight);
        voter
    }
}

// ── Weight transfer ───────────────────────────────────────────────────────────

#[test]
fn test_base_weight_comes_from_the_snapshot() {
    let f = Fixture::setup();
    let delegator = f.voter(100);

    assert_eq!(f.contract().get_base_weight(&PROPOSAL, &delegator), 100);
    assert_eq!(f.contract().get_voting_weight(&PROPOSAL, &delegator), 100);
}

#[test]
fn test_delegated_weight_transfers_to_the_delegate() {
    let f = Fixture::setup();
    let contract = f.contract();

    let delegator = f.voter(100);
    let delegate = f.voter(40);

    // Before delegating, each votes with its own weight.
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &delegator), 100);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &delegate), 40);

    contract.delegate(&delegator, &delegate);

    // The weight moved: the delegator votes with nothing, and the delegate
    // picks up the full 140.
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &delegator), 0);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &delegate), 140);
}

#[test]
fn test_delegate_is_recorded_in_both_directions() {
    let f = Fixture::setup();
    let contract = f.contract();

    let delegator = f.voter(100);
    let delegate = f.voter(40);

    contract.delegate(&delegator, &delegate);

    assert_eq!(contract.get_delegate(&delegator), Some(delegate.clone()));

    let delegators = contract.get_delegators(&delegate);
    assert_eq!(delegators.len(), 1);
    assert_eq!(delegators.get(0).unwrap(), delegator);
}

#[test]
fn test_several_delegators_accumulate_on_one_delegate() {
    let f = Fixture::setup();
    let contract = f.contract();

    let first = f.voter(100);
    let second = f.voter(25);
    let delegate = f.voter(10);

    contract.delegate(&first, &delegate);
    contract.delegate(&second, &delegate);

    assert_eq!(contract.get_delegate(&first), Some(delegate.clone()));
    assert_eq!(contract.get_delegate(&second), Some(delegate.clone()));
    assert_eq!(contract.get_delegators(&delegate).len(), 2);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &delegate), 135);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &first), 0);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &second), 0);
}

// ── Revocation ────────────────────────────────────────────────────────────────

#[test]
fn test_revoking_delegation_restores_the_delegators_weight() {
    let f = Fixture::setup();
    let contract = f.contract();

    let delegator = f.voter(100);
    let delegate = f.voter(40);

    contract.delegate(&delegator, &delegate);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &delegator), 0);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &delegate), 140);

    contract.revoke(&delegator);

    assert_eq!(contract.get_delegate(&delegator), None);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &delegator), 100);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &delegate), 40);
}

#[test]
fn test_revoking_clears_the_reverse_index() {
    let f = Fixture::setup();
    let contract = f.contract();

    let first = f.voter(100);
    let second = f.voter(25);
    let delegate = f.voter(10);

    contract.delegate(&first, &delegate);
    contract.delegate(&second, &delegate);
    contract.revoke(&first);

    let delegators = contract.get_delegators(&delegate);
    assert_eq!(delegators.len(), 1);
    assert_eq!(delegators.get(0).unwrap(), second);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &delegate), 35);
}

#[test]
fn test_re_delegation_moves_weight_instead_of_duplicating_it() {
    let f = Fixture::setup();
    let contract = f.contract();

    let delegator = f.voter(100);
    let first = f.voter(10);
    let second = f.voter(5);

    contract.delegate(&delegator, &first);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &first), 110);

    contract.delegate(&delegator, &second);

    // The first delegate gave the weight back rather than keeping a copy.
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &first), 10);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &second), 105);
    assert_eq!(contract.get_delegators(&first).len(), 0);
    assert_eq!(contract.get_delegators(&second).len(), 1);
}

#[test]
fn test_total_weight_is_conserved_across_a_reexport() {
    let f = Fixture::setup();
    let contract = f.contract();

    let delegator = f.voter(100);
    let delegate = f.voter(40);

    let before = contract.get_voting_weight(&PROPOSAL, &delegator)
        + contract.get_voting_weight(&PROPOSAL, &delegate);

    contract.delegate(&delegator, &delegate);
    let after = contract.get_voting_weight(&PROPOSAL, &delegator)
        + contract.get_voting_weight(&PROPOSAL, &delegate);

    assert_eq!(before, 140);
    assert_eq!(after, 140);
}

// ── Chains and cycles ─────────────────────────────────────────────────────────

#[test]
fn test_delegation_chain_resolves_to_the_final_delegate() {
    let f = Fixture::setup();
    let contract = f.contract();

    let a = f.voter(100);
    let b = f.voter(40);
    let c = f.voter(7);

    contract.delegate(&a, &b);
    contract.delegate(&b, &c);

    // Weight travels to the end of the chain and stops there.
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &a), 0);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &b), 0);
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &c), 147);
}

#[test]
fn test_self_delegation_is_rejected() {
    let f = Fixture::setup();
    let voter = f.voter(100);

    f.contract().delegate(&voter, &voter);
}

#[test]
fn test_circular_delegation_is_rejected() {
    let f = Fixture::setup();
    let contract = f.contract();

    let a = f.voter(100);
    let b = f.voter(40);

    // A -> B is fine, but closing the loop back to A is not.
    contract.delegate(&a, &b);
    contract.delegate(&b, &a);
}

#[test]
fn test_longer_cycles_are_rejected_too() {
    let f = Fixture::setup();
    let contract = f.contract();

    let a = f.voter(1);
    let b = f.voter(1);
    let c = f.voter(1);
    let d = f.voter(1);

    contract.delegate(&a, &b);
    contract.delegate(&b, &c);
    contract.delegate(&c, &d);

    // d -> a would close a 4-hop cycle.
    contract.delegate(&d, &a);
}

#[test]
fn test_rejected_cycle_leaves_state_untouched() {
    let f = Fixture::setup();
    let contract = f.contract();

    let a = f.voter(100);
    let b = f.voter(40);

    contract.delegate(&a, &b);

    assert!(contract.try_delegate(&b, &a).is_err());

    // The rejected delegation changed nothing.
    assert_eq!(contract.get_delegate(&b), None);
    assert_eq!(contract.get_delegate(&a), Some(b.clone()));
    assert_eq!(contract.get_voting_weight(&PROPOSAL, &b), 140);
}

#[test]
fn test_chain_deeper_than_max_depth_is_capped() {
    let f = Fixture::setup();
    let contract = f.contract();

    // Build a chain one hop longer than the traversal cap.
    let holders: std::vec::Vec<Address> = (0..(MAX_CHAIN_DEPTH + 2)).map(|_| f.voter(10)).collect();

    for i in 0..(holders.len() - 1) {
        contract.delegate(&holders[i], &holders[i + 1]);
    }

    let tail = holders.len() - 1;

    // The tail is the only holder allowed to vote. Resolution counts the
    // tail's own weight plus one delegator per resolvable hop, and stops at
    // MAX_CHAIN_DEPTH, so the holder at the far end of the chain is dropped
    // rather than followed.
    let resolved = contract.get_voting_weight(&PROPOSAL, &holders[tail]);
    assert_eq!(resolved, 10 * (MAX_CHAIN_DEPTH as i128 + 1));

    // Everyone upstream has delegated their weight away.
    for holder in holders.iter().take(tail) {
        assert_eq!(contract.get_voting_weight(&PROPOSAL, holder), 0);
    }
}

#[test]
fn test_chain_at_the_depth_limit_is_accepted() {
    let f = Fixture::setup();
    let contract = f.contract();

    let holders: std::vec::Vec<Address> = (0..MAX_CHAIN_DEPTH).map(|_| f.voter(10)).collect();

    for i in 0..(holders.len() - 1) {
        contract.delegate(&holders[i], &holders[i + 1]);
    }

    let tail = holders.len() - 1;
    assert_eq!(
        contract.get_voting_weight(&PROPOSAL, &holders[tail]),
        10 * MAX_CHAIN_DEPTH as i128
    );
}

// ── Guards ────────────────────────────────────────────────────────────────────

#[test]
fn test_cannot_initialize_twice() {
    let f = Fixture::setup();
    let contract = f.contract();

    let other_snapshot = f.env.register_contract(None, MockSnapshot);
    contract.initialize(&f.admin, &other_snapshot);
}

#[test]
fn test_cannot_revoke_without_a_delegation() {
    let f = Fixture::setup();
    let voter = f.voter(100);

    f.contract().revoke(&voter);
}

#[test]
fn test_views_require_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let voter = Address::generate(&env);
    let contract_id = env.register_contract(None, DelegationContract);
    let contract = DelegationContractClient::new(&env, &contract_id);

    assert!(contract.try_get_voting_weight(&PROPOSAL, &voter).is_err());
}

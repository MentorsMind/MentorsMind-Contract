extern crate std;

use std::panic::{catch_unwind, AssertUnwindSafe};

use mentorminds_escrow::{EscrowContract, EscrowContractClient};
use mentorminds_governance::{GovernanceContract, GovernanceContractClient, ProposalAction};
use mentorminds_oracle::{OracleContract, OracleContractClient};
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short,
    testutils::{Address as _, Ledger},
    token::StellarAssetClient,
    Address, Bytes, BytesN, Env, IntoVal, Symbol, Vec,
};

#[contracttype]
#[derive(Clone)]
enum MockTokenDataKey {
    Balance(Address),
    Holders,
    TotalSupply,
}

#[contract]
struct MockMntToken;

#[contractimpl]
impl MockMntToken {
    pub fn mint(env: Env, to: Address, amount: i128) {
        let bal: i128 = env
            .storage()
            .persistent()
            .get(&MockTokenDataKey::Balance(to.clone()))
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&MockTokenDataKey::Balance(to.clone()), &(bal + amount));

        let mut holders: Vec<Address> = env
            .storage()
            .persistent()
            .get(&MockTokenDataKey::Holders)
            .unwrap_or(Vec::new(&env));
        if !holders.contains(to.clone()) {
            holders.push_back(to);
            env.storage()
                .persistent()
                .set(&MockTokenDataKey::Holders, &holders);
        }

        let total: i128 = env
            .storage()
            .persistent()
            .get(&MockTokenDataKey::TotalSupply)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&MockTokenDataKey::TotalSupply, &(total + amount));
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        from.require_auth();
        if amount <= 0 {
            panic!("invalid amount");
        }

        let from_bal: i128 = Self::balance(env.clone(), from.clone());
        if from_bal < amount {
            panic!("insufficient balance");
        }

        let to_bal: i128 = Self::balance(env.clone(), to.clone());
        env.storage()
            .persistent()
            .set(&MockTokenDataKey::Balance(from), &(from_bal - amount));
        env.storage()
            .persistent()
            .set(&MockTokenDataKey::Balance(to.clone()), &(to_bal + amount));

        let mut holders: Vec<Address> = env
            .storage()
            .persistent()
            .get(&MockTokenDataKey::Holders)
            .unwrap_or(Vec::new(&env));
        if !holders.contains(to.clone()) {
            holders.push_back(to);
            env.storage()
                .persistent()
                .set(&MockTokenDataKey::Holders, &holders);
        }
    }

    pub fn balance(env: Env, addr: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&MockTokenDataKey::Balance(addr))
            .unwrap_or(0)
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&MockTokenDataKey::TotalSupply)
            .unwrap_or(0)
    }

    pub fn get_holders(env: Env) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&MockTokenDataKey::Holders)
            .unwrap_or(Vec::new(&env))
    }
}

#[contracttype]
#[derive(Clone)]
enum MockSnapshotDataKey {
    Token,
    TotalSupply(u32),
    BalanceAt(u32, Address),
}

#[contract]
struct MockSnapshot;

#[contractimpl]
impl MockSnapshot {
    pub fn initialize(env: Env, token: Address) {
        env.storage()
            .persistent()
            .set(&MockSnapshotDataKey::Token, &token);
    }

    pub fn record_snapshot(env: Env, snapshot_id: u32) {
        let token: Address = env
            .storage()
            .persistent()
            .get(&MockSnapshotDataKey::Token)
            .expect("token missing");

        let total_supply: i128 = env.invoke_contract(
            &token,
            &Symbol::new(&env, "total_supply"),
            ().into_val(&env),
        );
        env.storage().persistent().set(
            &MockSnapshotDataKey::TotalSupply(snapshot_id),
            &total_supply,
        );

        let holders: Vec<Address> =
            env.invoke_contract(&token, &Symbol::new(&env, "get_holders"), ().into_val(&env));

        for holder in holders.iter() {
            let bal: i128 = env.invoke_contract(
                &token,
                &Symbol::new(&env, "balance"),
                (holder.clone(),).into_val(&env),
            );
            env.storage()
                .persistent()
                .set(&MockSnapshotDataKey::BalanceAt(snapshot_id, holder), &bal);
        }
    }

    pub fn get_total_supply_at(env: Env, snapshot_id: u32) -> i128 {
        env.storage()
            .persistent()
            .get(&MockSnapshotDataKey::TotalSupply(snapshot_id))
            .unwrap_or(0)
    }

    pub fn get_voting_power(env: Env, snapshot_id: u32, voter: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&MockSnapshotDataKey::BalanceAt(snapshot_id, voter))
            .unwrap_or(0)
    }
}

fn assert_panics<F: FnOnce()>(f: F) {
    let result = catch_unwind(AssertUnwindSafe(f));
    assert!(result.is_err(), "expected call to panic");
}

fn create_token<'a>(env: &'a Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
    let address = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    (address.clone(), StellarAssetClient::new(env, &address))
}

fn should_liquidate(price: i128) -> bool {
    price < 80
}

fn cpmm_spot_price_after_swap(x_reserve: f64, y_reserve: f64, dx: f64) -> f64 {
    let k = x_reserve * y_reserve;
    let new_x = x_reserve + dx;
    let new_y = k / new_x;
    new_y / new_x
}

#[test]
fn simulate_flash_loan_governance_attack_blocked_by_snapshot_sc32() {
    let env = Env::default();
    env.mock_all_auths();

    let token_id = env.register_contract(None, MockMntToken);
    let snapshot_id = env.register_contract(None, MockSnapshot);
    let governance_id = env.register_contract(None, GovernanceContract);

    let token = MockMntTokenClient::new(&env, &token_id);
    let snapshot = MockSnapshotClient::new(&env, &snapshot_id);
    let governance = GovernanceContractClient::new(&env, &governance_id);

    let admin = Address::generate(&env);
    let proposer = Address::generate(&env);
    let lender = Address::generate(&env);
    let attacker = Address::generate(&env);

    snapshot.initialize(&token_id);
    governance.initialize(
        &admin,
        &token_id,
        &snapshot_id,
        &Some(120u64),
        &Some(1_000u32),
    );

    token.mint(&proposer, &1_000);
    token.mint(&lender, &1_000_000);

    let proposal_id = governance.create_proposal(
        &proposer,
        &Bytes::from_slice(&env, b"Flash-loan attempt"),
        &BytesN::from_array(&env, &[7u8; 32]),
        &ProposalAction::UpdateFee(300),
    );

    // Same transaction simulation: borrow -> vote -> repay.
    token.transfer(&lender, &attacker, &500_000);
    assert_panics(|| governance.vote(&attacker, &proposal_id, &true));
    token.transfer(&attacker, &lender, &500_000);

    assert_eq!(governance.get_vote_weight(&proposal_id, &attacker), 0);
    assert!(!governance.get_vote(&proposal_id, &attacker));
}

#[test]
fn simulate_oracle_manipulation_requires_three_compromised_feeders_sc42() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 1_000);

    let oracle_id = env.register_contract(None, OracleContract);
    let oracle = OracleContractClient::new(&env, &oracle_id);

    let admin = Address::generate(&env);
    let f1 = Address::generate(&env);
    let f2 = Address::generate(&env);
    let f3 = Address::generate(&env);
    let f4 = Address::generate(&env);
    let f5 = Address::generate(&env);

    oracle.initialize(&admin);
    oracle.add_feeder(&f1);
    oracle.add_feeder(&f2);
    oracle.add_feeder(&f3);
    oracle.add_feeder(&f4);
    oracle.add_feeder(&f5);

    let asset = symbol_short!("XLM");

    // Round 1: two manipulated feeds are not enough to move median into liquidation range.
    oracle.submit_price(&f1, &asset, &100, &1_000);
    oracle.submit_price(&f2, &asset, &101, &1_000);
    oracle.submit_price(&f3, &asset, &102, &1_000);
    oracle.submit_price(&f4, &asset, &2, &1_000);
    oracle.submit_price(&f5, &asset, &1, &1_000);

    let (price_round_1, _) = oracle.get_price(&asset);
    assert_eq!(price_round_1, 100);
    assert!(!should_liquidate(price_round_1));

    // Round 2: three manipulated feeds can push the median and trigger liquidation.
    oracle.submit_price(&f1, &asset, &100, &1_100);
    oracle.submit_price(&f2, &asset, &101, &1_100);
    oracle.submit_price(&f3, &asset, &3, &1_100);
    oracle.submit_price(&f4, &asset, &2, &1_100);
    oracle.submit_price(&f5, &asset, &1, &1_100);

    let (price_round_2, _) = oracle.get_price(&asset);
    assert_eq!(price_round_2, 3);
    assert!(should_liquidate(price_round_2));
}

#[test]
fn simulate_liquidity_drain_enforces_sc36_fee_floor() {
    let env = Env::default();
    env.mock_all_auths();

    // Pure market simulation: a large swap drains depth and collapses spot price.
    let initial_price = cpmm_spot_price_after_swap(1_000_000.0, 1_000_000.0, 0.0);
    let post_attack_price = cpmm_spot_price_after_swap(1_000_000.0, 1_000_000.0, 900_000.0);
    assert!(post_attack_price < initial_price * 0.5);

    // On-chain control check: fee floor prevents zero-fee configuration.
    let escrow_id = env.register_contract(None, EscrowContract);
    let escrow = EscrowContractClient::new(&env, &escrow_id);

    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let (token_addr, _sac) = create_token(&env, &admin);
    let mut approved = Vec::new(&env);
    approved.push_back(token_addr);

    escrow.initialize(&admin, &treasury, &500u32, &approved, &0u64, &None);
    let original_fee = escrow.get_fee_bps();
    assert_panics(|| escrow.update_fee(&0u32));
    assert_eq!(
        escrow.get_fee_bps(),
        original_fee,
        "SC-36: fee floor must reject zero fee"
    );
}

#[test]
fn simulate_sybil_self_review_attempts_blocked_by_session_verification_requirement() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 10_000);

    let admin = Address::generate(&env);
    let mentor = Address::generate(&env);
    let learner = Address::generate(&env);
    let treasury = Address::generate(&env);

    let (token, sac) = create_token(&env, &admin);
    sac.mint(&learner, &100_000);

    let escrow_id = env.register_contract(None, EscrowContract);
    let escrow = EscrowContractClient::new(&env, &escrow_id);

    let mut approved = Vec::new(&env);
    approved.push_back(token.clone());
    escrow.initialize(&admin, &treasury, &500u32, &approved, &0u64, &None);

    let now = env.ledger().timestamp();
    let eid = escrow.create_escrow(
        &mentor,
        &learner,
        &10_000,
        &symbol_short!("SYBIL1"),
        &token,
        &now,
        &1u32,
    );
    escrow.release_funds(&learner, &eid);

    // Session verification gate: only released sessions are reviewable, and only by the learner.
    assert_panics(|| escrow.submit_review(&mentor, &eid, &symbol_short!("BIASED")));

    // Sybil swarm cannot bypass caller-role check.
    for _ in 0..12 {
        let sybil = Address::generate(&env);
        assert_panics(|| escrow.submit_review(&sybil, &eid, &symbol_short!("SPAM")));
    }

    let ok = catch_unwind(AssertUnwindSafe(|| {
        escrow.submit_review(&learner, &eid, &symbol_short!("VALID"));
    }));
    assert!(ok.is_ok(), "learner review should succeed");
}

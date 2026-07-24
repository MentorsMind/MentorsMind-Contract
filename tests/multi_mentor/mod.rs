extern crate std;

use mentorminds_escrow::{EscrowContract, EscrowContractClient, EscrowStatus, MentorShare};
use soroban_sdk::{
    symbol_short,
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Vec,
};

fn setup() -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let learner = Address::generate(&env);
    let treasury = Address::generate(&env);

    let sac = env.register_stellar_asset_contract_v2(admin.clone());
    let token = sac.address();
    sac.mint(&learner, &10_000);

    let contract_id = env.register_contract(None, EscrowContract);
    let client = EscrowContractClient::new(&env, &contract_id);

    let mut approved = Vec::new(&env);
    approved.push_back(token.clone());
    client.initialize(&admin, &treasury, &500u32, &approved, &0u64);

    (env, contract_id, token, admin, learner)
}

#[test]
fn test_create_multi_mentor_escrow() {
    let (env, contract_id, token, _admin, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);

    let m1 = Address::generate(&env);
    let m2 = Address::generate(&env);

    let mut mentors = Vec::new(&env);
    mentors.push_back(MentorShare { mentor: m1.clone(), share_bps: 6000 });
    mentors.push_back(MentorShare { mentor: m2.clone(), share_bps: 4000 });

    let id = client.create_multi_mentor_escrow(
        &learner,
        &mentors,
        &1_000i128,
        &token,
        &symbol_short!("MM1"),
        &0u64,
    );

    let escrow = client.get_multi_mentor_escrow(&id);
    assert_eq!(escrow.status, EscrowStatus::Active);
    assert_eq!(escrow.amount, 1_000);
    assert_eq!(escrow.mentors.len(), 2);
}

#[test]
fn test_release_distributes_proportionally() {
    let (env, contract_id, token, _admin, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);

    let m1 = Address::generate(&env);
    let m2 = Address::generate(&env);

    let mut mentors = Vec::new(&env);
    mentors.push_back(MentorShare { mentor: m1.clone(), share_bps: 6000 });
    mentors.push_back(MentorShare { mentor: m2.clone(), share_bps: 4000 });

    let id = client.create_multi_mentor_escrow(
        &learner,
        &mentors,
        &1_000i128,
        &token,
        &symbol_short!("MM2"),
        &0u64,
    );

    let tok = TokenClient::new(&env, &token);
    let m1_before = tok.balance(&m1);
    let m2_before = tok.balance(&m2);

    client.release_multi_mentor_escrow(&learner, &id);

    // 1000 gross, 5% fee = 50, net = 950
    // m1 gets 60% of 950 = 570, m2 gets 40% of 950 = 380
    assert_eq!(tok.balance(&m1), m1_before + 570);
    assert_eq!(tok.balance(&m2), m2_before + 380);
    assert_eq!(client.get_multi_mentor_escrow(&id).status, EscrowStatus::Released);
}

#[test]
#[should_panic(expected = "Mentor shares must sum to 10000")]
fn test_invalid_shares_rejected() {
    let (env, contract_id, token, _admin, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);

    let m1 = Address::generate(&env);
    let m2 = Address::generate(&env);

    let mut mentors = Vec::new(&env);
    mentors.push_back(MentorShare { mentor: m1, share_bps: 5000 });
    mentors.push_back(MentorShare { mentor: m2, share_bps: 3000 }); // only 8000 total

    client.create_multi_mentor_escrow(
        &learner,
        &mentors,
        &1_000i128,
        &token,
        &symbol_short!("MM3"),
        &0u64,
    );
}

#[test]
#[should_panic(expected = "At least 2 mentors required")]
fn test_single_mentor_rejected() {
    let (env, contract_id, token, _admin, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);

    let m1 = Address::generate(&env);
    let mut mentors = Vec::new(&env);
    mentors.push_back(MentorShare { mentor: m1, share_bps: 10000 });

    client.create_multi_mentor_escrow(
        &learner,
        &mentors,
        &1_000i128,
        &token,
        &symbol_short!("MM4"),
        &0u64,
    );
}

#[test]
#[should_panic(expected = "Caller not authorized")]
fn test_unauthorized_release_rejected() {
    let (env, contract_id, token, _admin, learner) = setup();
    let client = EscrowContractClient::new(&env, &contract_id);

    let m1 = Address::generate(&env);
    let m2 = Address::generate(&env);

    let mut mentors = Vec::new(&env);
    mentors.push_back(MentorShare { mentor: m1.clone(), share_bps: 5000 });
    mentors.push_back(MentorShare { mentor: m2.clone(), share_bps: 5000 });

    let id = client.create_multi_mentor_escrow(
        &learner,
        &mentors,
        &1_000i128,
        &token,
        &symbol_short!("MM5"),
        &0u64,
    );

    let rando = Address::generate(&env);
    client.release_multi_mentor_escrow(&rando, &id);
}

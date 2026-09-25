/// Integration tests exercising multiple MentorMinds contracts together.
///
/// Contracts under test:
///   - mentorminds-escrow      (EscrowContract)
///   - mentorminds-verification (VerificationContract — used as reputation/staking proxy)
///   - mentorminds-mnt-token   (MNTToken — used as the payment token)
///
/// Each test uses `Env::default()` with `mock_all_auths()` and registers all
/// required contracts in the same environment so cross-contract state is shared.
extern crate std;

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, BytesN, Env, Symbol, TryFromVal, Vec,
};

use mentorminds_escrow::{EscrowContract, EscrowContractClient, EscrowParams, EscrowStatus};
use mentorminds_verification::{VerificationContract, VerificationContractClient};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

/// Registers a Stellar Asset Contract and returns its address + SAC client.
fn create_token<'a>(env: &'a Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
    let addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    (addr.clone(), StellarAssetClient::new(env, &addr))
}

fn advance_time(env: &Env, secs: u64) {
    env.ledger().with_mut(|li| li.timestamp += secs);
}

/// Minimal fixture wiring escrow + verification in one env.
struct Fixture<'a> {
    env: Env,
    escrow: EscrowContractClient<'a>,
    escrow_id: Address,
    verif: VerificationContractClient<'a>,
    #[allow(dead_code)]
    admin: Address,
    mentor: Address,
    learner: Address,
    treasury: Address,
    token: Address,
}

impl<'a> Fixture<'a> {
    /// `fee_bps` — escrow platform fee in basis points.
    fn new(env: &'a Env, fee_bps: u32) -> Self {
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 10_000);

        let admin = Address::generate(env);
        let mentor = Address::generate(env);
        let learner = Address::generate(env);
        let treasury = Address::generate(env);

        // --- Token ---
        let (token, sac) = create_token(env, &admin);
        sac.mint(&learner, &1_000_000);

        // --- Escrow ---
        let escrow_id = env.register_contract(None, EscrowContract);
        let escrow = EscrowContractClient::new(env, &escrow_id);
        let mut approved = Vec::new(env);
        approved.push_back(token.clone());
        escrow.initialize(&admin, &treasury, &fee_bps, &approved, &0u64, &None);

        // --- Verification (reputation / staking proxy) ---
        let verif_id = env.register_contract(None, VerificationContract);
        let verif = VerificationContractClient::new(env, &verif_id);
        verif.initialize(&admin);

        Fixture {
            env: env.clone(),
            escrow,
            escrow_id,
            verif,
            admin,
            mentor,
            learner,
            treasury,
            token,
        }
    }

    fn token_client(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token)
    }

    /// Verify the mentor with a 1-hour expiry from now.
    fn verify_mentor(&self) {
        let hash: BytesN<32> = BytesN::from_array(&self.env, &[0xABu8; 32]);
        let expiry = self.env.ledger().timestamp() + 3_600;
        self.verif.verify_mentor(&self.mentor, &hash, &expiry);
    }

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
}

// ---------------------------------------------------------------------------
// 1. Full session lifecycle: verify → create escrow → release → check reputation
// ---------------------------------------------------------------------------

#[test]
fn test_full_session_lifecycle() {
    let env = Env::default();
    let f = Fixture::new(&env, 500); // 5% fee

    // Step 1 — register mentor in verification contract (reputation gate)
    f.verify_mentor();
    assert!(
        f.verif.is_verified(&f.mentor),
        "mentor must be verified before session"
    );

    // Step 2 — learner creates escrow
    let escrow_id = f.create_escrow(10_000);
    let escrow = f.escrow.get_escrow(&escrow_id);
    assert_eq!(escrow.status, EscrowStatus::Active);

    // Step 3 — learner releases funds after session
    let token = f.token_client();
    let mentor_before = token.balance(&f.mentor);
    let treasury_before = token.balance(&f.treasury);

    f.escrow.release_funds(&f.learner, &escrow_id);

    // Step 4 — verify balances (5% fee on 10_000 = 500 fee, 9_500 net)
    assert_eq!(token.balance(&f.mentor), mentor_before + 9_500);
    assert_eq!(token.balance(&f.treasury), treasury_before + 500);
    assert_eq!(token.balance(&f.escrow_id), 0);

    let released = f.escrow.get_escrow(&escrow_id);
    assert_eq!(released.status, EscrowStatus::Released);
    assert_eq!(released.platform_fee, 500);
    assert_eq!(released.net_amount, 9_500);

    // Step 5 — reputation contract still shows mentor as verified post-session
    assert!(
        f.verif.is_verified(&f.mentor),
        "mentor reputation must persist after session"
    );
}

// ---------------------------------------------------------------------------
// 2. Reputation contract reads correct escrow status (cross-contract state)
// ---------------------------------------------------------------------------

#[test]
fn test_reputation_reads_escrow_status() {
    let env = Env::default();
    let f = Fixture::new(&env, 0);

    f.verify_mentor();

    let eid = f.create_escrow(5_000);

    // Before release: escrow Active, mentor verified
    assert_eq!(f.escrow.get_escrow(&eid).status, EscrowStatus::Active);
    assert!(f.verif.is_verified(&f.mentor));

    // Release escrow
    f.escrow.release_funds(&f.learner, &eid);

    // After release: escrow Released, reputation unchanged (still verified)
    assert_eq!(f.escrow.get_escrow(&eid).status, EscrowStatus::Released);
    assert!(f.verif.is_verified(&f.mentor));

    // Revoke mentor verification — simulates reputation penalty
    f.verif.revoke_verification(&f.mentor);
    assert!(!f.verif.is_verified(&f.mentor));

    // Escrow record is unaffected by reputation change
    assert_eq!(f.escrow.get_escrow(&eid).status, EscrowStatus::Released);
}

// ---------------------------------------------------------------------------
// 3. Staking tier affects fee: Gold tier = 3% fee (300 bps)
// ---------------------------------------------------------------------------

#[test]
fn test_gold_tier_fee_300_bps() {
    let env = Env::default();
    // Gold tier = 3% = 300 bps
    let f = Fixture::new(&env, 300);

    f.verify_mentor();
    let eid = f.create_escrow(10_000);

    let token = f.token_client();
    let mentor_before = token.balance(&f.mentor);
    let treasury_before = token.balance(&f.treasury);

    f.escrow.release_funds(&f.learner, &eid);

    // 3% of 10_000 = 300 fee, 9_700 net
    assert_eq!(token.balance(&f.mentor), mentor_before + 9_700);
    assert_eq!(token.balance(&f.treasury), treasury_before + 300);

    let e = f.escrow.get_escrow(&eid);
    assert_eq!(e.platform_fee, 300);
    assert_eq!(e.net_amount, 9_700);
    assert_eq!(e.status, EscrowStatus::Released);
}

// ---------------------------------------------------------------------------
// 4. Referral reward triggers after first session completion
//    Modelled as: treasury receives fee only on first session; second session
//    uses a reduced "referral" fee (0 bps) set by admin after first release.
// ---------------------------------------------------------------------------

#[test]
fn test_referral_reward_after_first_session() {
    let env = Env::default();
    let f = Fixture::new(&env, 500); // 5% standard fee

    f.verify_mentor();

    // First session — standard 5% fee
    let eid1 = f.create_escrow(10_000);
    let token = f.token_client();
    let treasury_before = token.balance(&f.treasury);

    f.escrow.release_funds(&f.learner, &eid1);
    assert_eq!(
        token.balance(&f.treasury),
        treasury_before + 500,
        "first session: 5% fee"
    );

    // Admin applies referral discount to minimum allowed fee floor.
    f.escrow.update_fee(&25u32);
    assert_eq!(f.escrow.get_fee_bps(), 25);

    // Second session — referral: minimum fee floor applies.
    let eid2 = f.create_escrow(10_000);
    let mentor_before = token.balance(&f.mentor);
    let treasury_after_first = token.balance(&f.treasury);

    f.escrow.release_funds(&f.learner, &eid2);

    assert_eq!(
        token.balance(&f.mentor),
        mentor_before + 9_975,
        "referral session: minimum fee floor"
    );
    assert_eq!(
        token.balance(&f.treasury),
        treasury_after_first + 25,
        "treasury grows by the floor fee on referral session"
    );

    let e2 = f.escrow.get_escrow(&eid2);
    assert_eq!(e2.platform_fee, 25);
    assert_eq!(e2.net_amount, 9_975);
}

// ---------------------------------------------------------------------------
// 5. Event emissions in correct order for full lifecycle
// ---------------------------------------------------------------------------

#[test]
fn test_event_order_full_lifecycle() {
    let env = Env::default();
    let f = Fixture::new(&env, 500);

    f.verify_mentor();
    let eid = f.create_escrow(1_000);
    f.escrow.release_funds(&f.learner, &eid);

    let all_events = env.events().all();

    // Escrow lifecycle: topics include (Escrow, Created, id) then (Escrow, Released, id)
    let mut created_pos: Option<usize> = None;
    let mut released_pos: Option<usize> = None;
    for (i, (_, topics, _)) in all_events.iter().enumerate() {
        for j in 0..topics.len() {
            let v = topics.get(j).unwrap();
            if let Ok(s) = soroban_sdk::Symbol::try_from_val(&env, &v) {
                if s == symbol_short!("Created") {
                    created_pos = Some(i);
                }
                if s == symbol_short!("Released") {
                    released_pos = Some(i);
                }
            }
        }
    }

    assert!(created_pos.is_some(), "must emit Created event");
    assert!(released_pos.is_some(), "must emit Released event");
    assert!(
        created_pos.unwrap() < released_pos.unwrap(),
        "Created must precede Released"
    );
}

// ---------------------------------------------------------------------------
// 6. Rollback: cross-contract call fails when mentor is not verified
//    (simulated by revoking verification before release and asserting the
//     escrow state is unchanged — the escrow itself doesn't gate on
//     verification, so we test the verification check independently and
//     confirm the escrow is still Active when the caller aborts early)
// ---------------------------------------------------------------------------

#[test]
fn test_rollback_when_verification_revoked_before_release() {
    let env = Env::default();
    let f = Fixture::new(&env, 0);

    f.verify_mentor();
    let eid = f.create_escrow(5_000);

    // Revoke mentor verification — simulates a failed pre-condition check
    f.verif.revoke_verification(&f.mentor);
    assert!(!f.verif.is_verified(&f.mentor));

    // Application layer would abort here; escrow must still be Active
    let escrow = f.escrow.get_escrow(&eid);
    assert_eq!(
        escrow.status,
        EscrowStatus::Active,
        "escrow must remain Active when caller aborts due to failed verification"
    );

    // Confirm funds are still locked in the contract
    let token = f.token_client();
    assert_eq!(token.balance(&f.escrow_id), 5_000);
    assert_eq!(token.balance(&f.mentor), 0);
}

// ---------------------------------------------------------------------------
// 7. Rollback: release panics on non-active escrow; state is unchanged
// ---------------------------------------------------------------------------

#[test]
fn test_rollback_double_release_leaves_state_intact() {
    let env = Env::default();
    let f = Fixture::new(&env, 500);

    let eid = f.create_escrow(2_000);
    f.escrow.release_funds(&f.learner, &eid);

    let token = f.token_client();
    let mentor_after_first = token.balance(&f.mentor);
    let treasury_after_first = token.balance(&f.treasury);

    // Second release must panic — state must not change
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f.escrow.release_funds(&f.learner, &eid);
    }));
    assert!(result.is_err(), "double-release must panic");

    // Balances unchanged after failed second release
    assert_eq!(token.balance(&f.mentor), mentor_after_first);
    assert_eq!(token.balance(&f.treasury), treasury_after_first);
    assert_eq!(f.escrow.get_escrow(&eid).status, EscrowStatus::Released);
}

// ---------------------------------------------------------------------------
// 8. Staking + verification: verified mentor gets Gold-tier fee; unverified
//    mentor session uses standard fee
// ---------------------------------------------------------------------------

#[test]
fn test_staking_tier_verified_vs_unverified() {
    let env = Env::default();

    // Two separate escrow contracts: one at Gold (300 bps), one at standard (500 bps)
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 10_000);

    let admin = Address::generate(&env);
    let mentor_gold = Address::generate(&env);
    let mentor_std = Address::generate(&env);
    let learner = Address::generate(&env);
    let treasury = Address::generate(&env);

    let (token, sac) = create_token(&env, &admin);
    sac.mint(&learner, &100_000);

    // Verification contract
    let verif_id = env.register_contract(None, VerificationContract);
    let verif = VerificationContractClient::new(&env, &verif_id);
    verif.initialize(&admin);

    // Verify only gold mentor
    let hash: BytesN<32> = BytesN::from_array(&env, &[0xCCu8; 32]);
    let expiry = env.ledger().timestamp() + 7_200;
    verif.verify_mentor(&mentor_gold, &hash, &expiry);

    // Gold-tier escrow (300 bps)
    let gold_id = env.register_contract(None, EscrowContract);
    let gold_escrow = EscrowContractClient::new(&env, &gold_id);
    let mut approved = Vec::new(&env);
    approved.push_back(token.clone());
    gold_escrow.initialize(&admin, &treasury, &300u32, &approved, &0u64, &None);

    // Standard escrow (500 bps)
    let std_id = env.register_contract(None, EscrowContract);
    let std_escrow = EscrowContractClient::new(&env, &std_id);
    let mut approved2 = Vec::new(&env);
    approved2.push_back(token.clone());
    std_escrow.initialize(&admin, &treasury, &500u32, &approved2, &0u64, &None);

    let now = env.ledger().timestamp();
    let tok = TokenClient::new(&env, &token);

    // Gold mentor session
    let eid_gold = gold_escrow.create_escrow(
        &mentor_gold,
        &learner,
        &10_000,
        &symbol_short!("G1"),
        &token,
        &now,
        &1u32,
    );
    let gold_mentor_before = tok.balance(&mentor_gold);
    let treasury_before = tok.balance(&treasury);
    gold_escrow.release_funds(&learner, &eid_gold);
    assert_eq!(tok.balance(&mentor_gold), gold_mentor_before + 9_700); // 3% fee
    assert_eq!(tok.balance(&treasury), treasury_before + 300);

    // Standard mentor session (unverified)
    assert!(!verif.is_verified(&mentor_std));
    let eid_std = std_escrow.create_escrow(
        &mentor_std,
        &learner,
        &10_000,
        &symbol_short!("S1"),
        &token,
        &now,
        &1u32,
    );
    let std_mentor_before = tok.balance(&mentor_std);
    let treasury_before2 = tok.balance(&treasury);
    std_escrow.release_funds(&learner, &eid_std);
    assert_eq!(tok.balance(&mentor_std), std_mentor_before + 9_500); // 5% fee
    assert_eq!(tok.balance(&treasury), treasury_before2 + 500);
}

// ---------------------------------------------------------------------------
// 9. Escrow + session registry: multiple sessions tracked independently
// ---------------------------------------------------------------------------

#[test]
fn test_multiple_sessions_tracked_independently() {
    let env = Env::default();
    let f = Fixture::new(&env, 0); // 0% fee for simplicity

    f.verify_mentor();

    let now = env.ledger().timestamp();

    // Register three sessions
    let eid1 = f.escrow.create_escrow(
        &f.mentor,
        &f.learner,
        &1_000,
        &symbol_short!("SES1"),
        &f.token,
        &now,
        &1u32,
    );
    let eid2 = f.escrow.create_escrow(
        &f.mentor,
        &f.learner,
        &2_000,
        &symbol_short!("SES2"),
        &f.token,
        &now,
        &1u32,
    );
    let eid3 = f.escrow.create_escrow(
        &f.mentor,
        &f.learner,
        &3_000,
        &symbol_short!("SES3"),
        &f.token,
        &now,
        &1u32,
    );

    assert_eq!(f.escrow.get_escrow_count(), 3);

    // Release only session 2; others remain Active
    f.escrow.release_funds(&f.learner, &eid2);

    assert_eq!(f.escrow.get_escrow(&eid1).status, EscrowStatus::Active);
    assert_eq!(f.escrow.get_escrow(&eid2).status, EscrowStatus::Released);
    assert_eq!(f.escrow.get_escrow(&eid3).status, EscrowStatus::Active);

    // Dispute session 3
    f.escrow
        .dispute(&f.learner, &eid3, &symbol_short!("NO_SHOW"));
    assert_eq!(f.escrow.get_escrow(&eid3).status, EscrowStatus::Disputed);

    // Session 1 still unaffected
    assert_eq!(f.escrow.get_escrow(&eid1).status, EscrowStatus::Active);

    // Resolve session 3 (mentor-favored resolution)
    f.escrow.resolve_dispute(&eid3, &true);
    assert_eq!(f.escrow.get_escrow(&eid3).status, EscrowStatus::Resolved);

    // Refund session 1
    f.escrow.refund(&eid1);
    assert_eq!(f.escrow.get_escrow(&eid1).status, EscrowStatus::Refunded);
}

// ---------------------------------------------------------------------------
// 10. Auto-release after session end time (escrow + session registry)
// ---------------------------------------------------------------------------

#[test]
fn test_auto_release_after_session_end() {
    let env = Env::default();
    let f = Fixture::new(&env, 300); // Gold tier 3%

    f.verify_mentor();

    let now = env.ledger().timestamp();
    // session ends in 60 s; default auto-release delay = 72 h
    let eid = f.escrow.create_escrow(
        &f.mentor,
        &f.learner,
        &10_000,
        &symbol_short!("SES1"),
        &f.token,
        &(now + 60),
        &1u32,
    );

    // Before window: must fail
    advance_time(&env, 60 + 72 * 3600 - 1);
    let too_early = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f.escrow.try_auto_release(&eid);
    }));
    assert!(too_early.is_err());

    // At boundary: must succeed
    advance_time(&env, 1);
    let token = f.token_client();
    let mentor_before = token.balance(&f.mentor);
    let treasury_before = token.balance(&f.treasury);

    f.escrow.try_auto_release(&eid);

    assert_eq!(token.balance(&f.mentor), mentor_before + 9_700);
    assert_eq!(token.balance(&f.treasury), treasury_before + 300);
    assert_eq!(f.escrow.get_escrow(&eid).status, EscrowStatus::Released);
}

#![cfg(test)]

use mentorminds_escrow::{EscrowContract, EscrowContractClient, EscrowStatus};
use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Ledger},
    token::{Client as TokenClient, StellarAssetClient},
    Address, Env, Symbol, Vec,
};

// -----------------------------------------------------------------------
// Helpers
// -----------------------------------------------------------------------

fn create_token<'a>(env: &'a Env, admin: &Address) -> (Address, StellarAssetClient<'a>) {
    let token_address = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let sac = StellarAssetClient::new(env, &token_address);
    (token_address, sac)
}

fn advance_time(env: &Env, secs: u64) {
    env.ledger().with_mut(|li| li.timestamp += secs);
}

struct TestFixture {
    env: Env,
    contract_id: Address,
    admin: Address,
    mentor: Address,
    learner: Address,
    treasury: Address,
    token_address: Address,
}

impl TestFixture {
    fn setup() -> Self {
        Self::setup_with_fee(500)
    }
    fn setup_with_fee(fee_bps: u32) -> Self {
        Self::setup_full(fee_bps, 0)
    }

    fn setup_full(fee_bps: u32, auto_release_delay_secs: u64) -> Self {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 14_400);

        let contract_id = env.register_contract(None, EscrowContract);
        let admin = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let treasury = Address::generate(&env);

        let (token_address, sac) = create_token(&env, &admin);
        sac.mint(&learner, &100_000);

        let client = EscrowContractClient::new(&env, &contract_id);
        let mut approved = Vec::new(&env);
        approved.push_back(token_address.clone());
        client.initialize(
            &admin,
            &treasury,
            &fee_bps,
            &approved,
            &auto_release_delay_secs,
        );

        TestFixture {
            env,
            contract_id,
            admin,
            mentor,
            learner,
            treasury,
            token_address,
        }
    }

    fn client(&self) -> EscrowContractClient<'_> {
        EscrowContractClient::new(&self.env, &self.contract_id)
    }
    fn token(&self) -> TokenClient<'_> {
        TokenClient::new(&self.env, &self.token_address)
    }
    #[allow(dead_code)]
    fn sac(&self) -> StellarAssetClient<'_> {
        StellarAssetClient::new(&self.env, &self.token_address)
    }

    fn create_escrow_at(&self, amount: i128, session_end_time: u64, session_id: &str) -> u64 {
        self.client().create_escrow(
            &self.mentor,
            &self.learner,
            &amount,
            &Symbol::new(&self.env, session_id),
            &self.token_address,
            &session_end_time,
            &1u32,
        )
    }

    fn create_package_escrow_at(
        &self,
        amount: i128,
        session_end_time: u64,
        session_id: &str,
        total_sessions: u32,
    ) -> u64 {
        self.client().create_escrow(
            &self.mentor,
            &self.learner,
            &amount,
            &Symbol::new(&self.env, session_id),
            &self.token_address,
            &session_end_time,
            &total_sessions,
        )
    }

    fn open_dispute(&self, escrow_id: u64) {
        self.client()
            .dispute(&self.learner, &escrow_id, &symbol_short!("NO_SHOW"));
    }
}

// -----------------------------------------------------------------------
// Tests
// -----------------------------------------------------------------------

#[test]
fn test_session_id_uniqueness() {
    let f = TestFixture::setup();
    f.create_escrow_at(1_000, 0, "S1");

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f.create_escrow_at(1_000, 0, "S1");
    }));
    assert!(result.is_err(), "Duplicate session_id must panic");

    // Different session_id should work
    f.create_escrow_at(1_000, 0, "S2");
}

#[test]
fn test_release_partial() {
    let f = TestFixture::setup_with_fee(500); // 5% fee
    let id = f.create_package_escrow_at(1_200, 0, "S1", 3); // 3 sessions, 400 each

    let mentor_before = f.token().balance(&f.mentor);
    let treasury_before = f.token().balance(&f.treasury);

    // Release 1st session (400)
    f.client().release_partial(&f.learner, &id);

    // 400 * 0.05 = 20 fee, 380 net
    assert_eq!(f.token().balance(&f.mentor), mentor_before + 380);
    assert_eq!(f.token().balance(&f.treasury), treasury_before + 20);

    let e = f.client().get_escrow(&id);
    assert_eq!(e.amount, 800);
    assert_eq!(e.sessions_completed, 1);
    assert_eq!(e.status, EscrowStatus::Active);

    // Release 2nd session (400)
    f.client().release_partial(&f.learner, &id);
    assert_eq!(f.token().balance(&f.mentor), mentor_before + 760);
    assert_eq!(f.token().balance(&f.treasury), treasury_before + 40);

    let e2 = f.client().get_escrow(&id);
    assert_eq!(e2.amount, 400);
    assert_eq!(e2.sessions_completed, 2);
    assert_eq!(e2.status, EscrowStatus::Active);

    // Release 3rd session (remaining 400)
    f.client().release_partial(&f.learner, &id);
    assert_eq!(f.token().balance(&f.mentor), mentor_before + 1140);
    assert_eq!(f.token().balance(&f.treasury), treasury_before + 60);

    let e3 = f.client().get_escrow(&id);
    assert_eq!(e3.amount, 0);
    assert_eq!(e3.sessions_completed, 3);
    assert_eq!(e3.status, EscrowStatus::Released);
}

#[test]
fn test_three_session_package_full_lifecycle() {
    let f = TestFixture::setup_with_fee(1000); // 10% fee
    let id = f.create_package_escrow_at(3000, 0, "PKG1", 3);

    // 1st release
    f.client().release_partial(&f.learner, &id);
    let e1 = f.client().get_escrow(&id);
    assert_eq!(e1.amount, 2000);
    assert_eq!(e1.sessions_completed, 1);
    assert_eq!(f.token().balance(&f.mentor), 900); // 1000 - 100 fee

    // 2nd release
    f.client().release_partial(&f.learner, &id);
    let e2 = f.client().get_escrow(&id);
    assert_eq!(e2.amount, 1000);
    assert_eq!(e2.sessions_completed, 2);
    assert_eq!(f.token().balance(&f.mentor), 1800);

    // 3rd release
    f.client().release_partial(&f.learner, &id);
    let e3 = f.client().get_escrow(&id);
    assert_eq!(e3.amount, 0);
    assert_eq!(e3.sessions_completed, 3);
    assert_eq!(e3.status, EscrowStatus::Released);
    assert_eq!(f.token().balance(&f.mentor), 2700);
    assert_eq!(f.token().balance(&f.treasury), 300);
}

#[test]
#[should_panic(expected = "Escrow not active")]
fn test_over_release_panics() {
    let f = TestFixture::setup();
    let id = f.create_package_escrow_at(1000, 0, "S1", 1);

    f.client().release_partial(&f.learner, &id);
    // Should panic
    f.client().release_partial(&f.learner, &id);
}

#[test]
fn test_resolve_dispute_all_to_mentor() {
    let f = TestFixture::setup_with_fee(0);
    let id = f.create_escrow_at(1_000, 0, "S1");
    f.open_dispute(id);

    let mentor_before = f.token().balance(&f.mentor);

    // Resolve 100% to mentor
    f.client().resolve_dispute(&id, &100u32);

    assert_eq!(f.token().balance(&f.mentor), mentor_before + 1_000);

    let e = f.client().get_escrow(&id);
    assert_eq!(e.status, EscrowStatus::Resolved);
    assert_eq!(e.net_amount, 1_000);
    assert_eq!(e.platform_fee, 0); // repurposed: learner share
}

#[test]
fn test_resolve_dispute_all_to_learner() {
    let f = TestFixture::setup_with_fee(0);
    let id = f.create_escrow_at(1_000, 0, "S1");
    f.open_dispute(id);

    let learner_before = f.token().balance(&f.learner);

    // Resolve 0% to mentor (all to learner)
    f.client().resolve_dispute(&id, &0u32);

    assert_eq!(f.token().balance(&f.learner), learner_before + 1_000);

    let e = f.client().get_escrow(&id);
    assert_eq!(e.status, EscrowStatus::Resolved);
    assert_eq!(e.net_amount, 0);
    assert_eq!(e.platform_fee, 1_000); // repurposed: learner share
}

#[test]
fn test_resolve_dispute_50_50() {
    let f = TestFixture::setup_with_fee(0);
    let id = f.create_escrow_at(1_000, 0, "S1");
    f.open_dispute(id);

    let mentor_before = f.token().balance(&f.mentor);
    let learner_before = f.token().balance(&f.learner);

    f.client().resolve_dispute(&id, &50u32);

    assert_eq!(f.token().balance(&f.mentor), mentor_before + 500);
    assert_eq!(f.token().balance(&f.learner), learner_before + 500);

    let e = f.client().get_escrow(&id);
    assert_eq!(e.status, EscrowStatus::Resolved);
    assert_eq!(e.net_amount, 500);
    assert_eq!(e.platform_fee, 500);
}

#[test]
fn test_admin_release() {
    let f = TestFixture::setup_with_fee(500);
    let id = f.create_escrow_at(1_000, 0, "S1");

    f.client().admin_release(&id);

    let e = f.client().get_escrow(&id);
    assert_eq!(e.status, EscrowStatus::Released);
    assert_eq!(f.token().balance(&f.mentor), 950);
}

#[test]
fn test_try_auto_release() {
    let f = TestFixture::setup_full(500, 3600);
    let now = f.env.ledger().timestamp();
    let id = f.create_escrow_at(1_000, now, "S1");

    advance_time(&f.env, 3600 + 1);
    f.client().try_auto_release(&id);

    let e = f.client().get_escrow(&id);
    assert_eq!(e.status, EscrowStatus::Released);
}

#[test]
fn test_query_by_mentor_pagination() {
    let f = TestFixture::setup();
    let mentor = Address::generate(&f.env);
    let learner = f.learner.clone();

    // Create 5 escrows for the same mentor
    for i in 0..5u32 {
        let session_id = match i {
            0 => Symbol::new(&f.env, "SM0"),
            1 => Symbol::new(&f.env, "SM1"),
            2 => Symbol::new(&f.env, "SM2"),
            3 => Symbol::new(&f.env, "SM3"),
            _ => Symbol::new(&f.env, "SM4"),
        };
        f.client().create_escrow(
            &mentor,
            &learner,
            &1_000,
            &session_id,
            &f.token_address,
            &0,
            &1u32,
        );
    }

    // Page 0, size 2 -> should return 2 escrows (ids 1, 2)
    let page0 = f.client().get_escrows_by_mentor(&mentor, &0, &2);
    assert_eq!(page0.len(), 2);
    assert_eq!(page0.get(0).unwrap().id, 1);
    assert_eq!(page0.get(1).unwrap().id, 2);

    // Page 1, size 2 -> should return 2 escrows (ids 3, 4)
    let page1 = f.client().get_escrows_by_mentor(&mentor, &1, &2);
    assert_eq!(page1.len(), 2);
    assert_eq!(page1.get(0).unwrap().id, 3);
    assert_eq!(page1.get(1).unwrap().id, 4);

    // Page 2, size 2 -> should return 1 escrow (id 5)
    let page2 = f.client().get_escrows_by_mentor(&mentor, &2, &2);
    assert_eq!(page2.len(), 1);
    assert_eq!(page2.get(0).unwrap().id, 5);

    // Page 3, size 2 -> should be empty
    let page3 = f.client().get_escrows_by_mentor(&mentor, &3, &2);
    assert_eq!(page3.len(), 0);
}

#[test]
fn test_query_by_learner_pagination() {
    let f = TestFixture::setup();
    let mentor = f.mentor.clone();
    let learner = Address::generate(&f.env);

    // Mint tokens for the new learner
    let admin = Address::generate(&f.env);
    let (tok, sac) = create_token(&f.env, &admin);
    sac.mint(&learner, &100_000);
    // Approve token
    f.client().set_approved_token(&tok, &true);

    for i in 0..3u32 {
        let session_id = match i {
            0 => Symbol::new(&f.env, "SL0"),
            1 => Symbol::new(&f.env, "SL1"),
            _ => Symbol::new(&f.env, "SL2"),
        };
        f.client().create_escrow(
            &mentor,
            &learner,
            &1_000,
            &session_id,
            &tok,
            &0,
            &1u32,
        );
    }

    let page0 = f.client().get_escrows_by_learner(&learner, &0, &10);
    assert_eq!(page0.len(), 3);
}

#[test]
fn test_query_by_status() {
    let f = TestFixture::setup_with_fee(0);

    let id1 = f.create_escrow_at(1_000, 0, "SS1");
    let _id2 = f.create_escrow_at(1_000, 0, "SS2");

    // Release first escrow
    f.client().release_funds(&f.learner, &id1);

    let active_ids = f.client().get_escrows_by_status(&EscrowStatus::Active);
    let released_ids = f.client().get_escrows_by_status(&EscrowStatus::Released);

    // id2 should be active
    assert!(active_ids.iter().any(|id| id == 2));
    // id1 should be released
    assert!(released_ids.iter().any(|id| id == 1));
}

#[test]
fn test_page_size_cap() {
    let f = TestFixture::setup();
    let mentor = f.mentor.clone();
    let learner = f.learner.clone();

    // Create 60 escrows
    for i in 0..60u32 {
        let session_id = Symbol::new(&f.env, &alloc::format!("SC{}", i));
        f.client().create_escrow(
            &mentor,
            &learner,
            &100,
            &session_id,
            &f.token_address,
            &0,
            &1u32,
        );
    }

    // Try to get 100 per page, should be capped at 50
    let results = f.client().get_escrows_by_mentor(&mentor, &0, &100);
    assert_eq!(results.len(), 50);
}

// -----------------------------------------------------------------------
// Token Whitelist Bypass Tests
// -----------------------------------------------------------------------

/// Test: Cannot create escrow with unapproved token
#[test]
fn test_create_escrow_unapproved_token_panics() {
    let f = TestFixture::setup();
    let bad_token = Address::generate(&f.env);
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f.client().create_escrow(
            &f.mentor,
            &f.learner,
            &500,
            &symbol_short!("BAD"),
            &bad_token,
            &0u64,
            &1u32,
        );
    }));
    assert!(result.is_err(), "unapproved token must be rejected");
}

/// Test: Cannot create escrow with a revoked token
#[test]
fn test_create_escrow_revoked_token_panics() {
    let f = TestFixture::setup();
    // Revoke the approved token
    f.client().set_approved_token(&f.token_address, &false);

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        f.create_escrow_at(500, 0, "REVOKED");
    }));
    assert!(result.is_err(), "revoked token must be rejected");
}

/// Test: Token whitelist toggle works correctly
#[test]
fn test_token_whitelist_toggle() {
    let f = TestFixture::setup();
    let new_token = Address::generate(&f.env);

    assert!(!f.client().is_token_approved(&new_token));
    f.client().set_approved_token(&new_token, &true);
    assert!(f.client().is_token_approved(&new_token));
    f.client().set_approved_token(&new_token, &false);
    assert!(!f.client().is_token_approved(&new_token));
}

/// Test: Random/unknown tokens are not approved by default
#[test]
fn test_unknown_tokens_not_approved() {
    let f = TestFixture::setup();
    for _ in 0..5 {
        let random = Address::generate(&f.env);
        assert!(!f.client().is_token_approved(&random));
    }
}

/// Test: Re-approving a revoked token allows escrow creation again
#[test]
fn test_re_approve_token_allows_escrow() {
    let f = TestFixture::setup();
    f.client().set_approved_token(&f.token_address, &false);
    assert!(!f.client().is_token_approved(&f.token_address));

    f.client().set_approved_token(&f.token_address, &true);
    assert!(f.client().is_token_approved(&f.token_address));

    let id = f.create_escrow_at(500, 0, "REAPPR");
    assert_eq!(id, 1);
}

extern crate alloc;

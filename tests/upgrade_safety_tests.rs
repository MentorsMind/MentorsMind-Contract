//! #419 — Contract Upgrade Safety
//!
//! Tests for:
//! - Initialization guard (double-init prevented)
//! - Version monotonicity (new_version must be > current)
//! - Timelock: upgrade cannot execute before delay elapses
//! - Only one pending upgrade at a time
//! - Cancel clears the pending slot
//! - Upgrade authorization (admin-only)

#![cfg(test)]

use mentorminds_upgrade_registry::{Error, UpgradeRegistryContract, UpgradeRegistryContractClient};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    vec, Address, BytesN, Env, IntoVal, Symbol, Vec,
};

fn zero_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

fn setup() -> (Env, Address, UpgradeRegistryContractClient<'static>) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(UpgradeRegistryContract, ());
    let client = UpgradeRegistryContractClient::new(&env, &id);
    client.initialize(&admin);
    (env, admin, client)
}

fn approvals(env: &Env, signers: &[Address]) -> Vec<Address> {
    let mut out = Vec::new(env);
    for signer in signers {
        out.push_back(signer.clone());
    }
    out
}

fn advance_time(env: &Env, secs: u64) {
    let current = env.ledger().timestamp();
    env.ledger().set(LedgerInfo {
        timestamp: current + secs,
        protocol_version: 22,
        sequence_number: env.ledger().sequence() + 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 100,
        min_persistent_entry_ttl: 100,
        max_entry_ttl: 9_999_999,
    });
}

// ─── Initialization guard ─────────────────────────────────────────────────────

#[test]
fn double_initialize_returns_error() {
    let (env, admin, client) = setup();
    let result = client.try_initialize(&admin);
    assert!(result.is_err(), "second initialize must fail");
}

// ─── Version monotonicity ─────────────────────────────────────────────────────

#[test]
fn schedule_upgrade_requires_version_monotonic() {
    let (env, admin, client) = setup();
    let name = Symbol::new(&env, "escrow");

    // First upgrade: v0 → v1 OK
    client.schedule_upgrade(
        &zero_hash(&env),
        &name,
        &1,
        &zero_hash(&env),
        &approvals(&env, &[admin.clone()]),
    );
    client.cancel_pending_upgrade();

    // Register v1 so the latest version is set
    client.register_upgrade(&name, &0, &1, &zero_hash(&env));

    // Now trying to schedule v1 again must fail (not monotonic)
    let result = client.try_schedule_upgrade(
        &zero_hash(&env),
        &name,
        &1,
        &zero_hash(&env),
        &approvals(&env, &[admin.clone()]),
    );
    assert!(result.is_err(), "same version must be rejected");

    // v0 < v1 must also be rejected
    let result = client.try_schedule_upgrade(
        &zero_hash(&env),
        &name,
        &0,
        &zero_hash(&env),
        &approvals(&env, &[admin.clone()]),
    );
    assert!(result.is_err(), "lower version must be rejected");

    // v2 > v1 must succeed
    client.schedule_upgrade(
        &zero_hash(&env),
        &name,
        &2,
        &zero_hash(&env),
        &approvals(&env, &[admin]),
    );
}

// ─── Timelock ─────────────────────────────────────────────────────────────────

#[test]
fn execute_upgrade_before_timelock_returns_error() {
    let (env, admin, client) = setup();
    let name = Symbol::new(&env, "lending");

    client.schedule_upgrade(
        &zero_hash(&env),
        &name,
        &1,
        &zero_hash(&env),
        &approvals(&env, &[admin.clone()]),
    );

    // Do NOT advance time — timelock has not elapsed
    let result = client.try_execute_pending_upgrade(&approvals(&env, &[admin]));
    assert!(result.is_err(), "execute before timelock must fail");
}

#[test]
fn execute_upgrade_after_timelock_succeeds() {
    let (env, admin, client) = setup();
    let name = Symbol::new(&env, "staking");

    client.schedule_upgrade(
        &zero_hash(&env),
        &name,
        &1,
        &zero_hash(&env),
        &approvals(&env, &[admin.clone()]),
    );

    // Advance past the default 48-hour delay
    advance_time(&env, 48 * 3_600 + 1);

    // execute_pending_upgrade calls env.deployer().update_current_contract_wasm
    // which is allowed in the test environment with a zero hash.
    client.execute_pending_upgrade(&approvals(&env, &[admin]));

    // After execution the pending slot must be cleared.
    assert!(client.get_pending_upgrade().is_none());
}

// ─── Single pending upgrade ───────────────────────────────────────────────────

#[test]
fn cannot_schedule_two_upgrades_simultaneously() {
    let (env, admin, client) = setup();
    let name = Symbol::new(&env, "payment");

    client.schedule_upgrade(
        &zero_hash(&env),
        &name,
        &1,
        &zero_hash(&env),
        &approvals(&env, &[admin.clone()]),
    );

    // Second schedule must fail while first is pending
    let result = client.try_schedule_upgrade(
        &zero_hash(&env),
        &name,
        &2,
        &zero_hash(&env),
        &approvals(&env, &[admin]),
    );
    assert!(result.is_err(), "second pending upgrade must be rejected");
}

// ─── Cancel ───────────────────────────────────────────────────────────────────

#[test]
fn cancel_clears_pending_upgrade() {
    let (env, admin, client) = setup();
    let name = Symbol::new(&env, "referral");

    client.schedule_upgrade(
        &zero_hash(&env),
        &name,
        &1,
        &zero_hash(&env),
        &approvals(&env, &[admin]),
    );
    assert!(client.get_pending_upgrade().is_some());

    client.cancel_pending_upgrade();
    assert!(client.get_pending_upgrade().is_none());
}

#[test]
fn cancel_with_no_pending_upgrade_returns_error() {
    let (_env, _admin, client) = setup();
    let result = client.try_cancel_pending_upgrade();
    assert!(result.is_err());
}

// ─── Execute with no pending upgrade ─────────────────────────────────────────

#[test]
fn execute_with_no_pending_upgrade_returns_error() {
    let (env, admin, client) = setup();
    let result = client.try_execute_pending_upgrade(&approvals(&env, &[admin]));
    assert!(result.is_err());
}

// ─── Upgrade delay configuration ─────────────────────────────────────────────

#[test]
fn upgrade_delay_defaults_to_48_hours() {
    let (_env, _admin, client) = setup();
    assert_eq!(client.get_upgrade_delay(), 48 * 3_600);
}

#[test]
fn set_upgrade_delay_accepted_in_valid_range() {
    let (_env, _admin, client) = setup();
    // 24 hours
    client.set_upgrade_delay(&(24 * 3_600));
    assert_eq!(client.get_upgrade_delay(), 24 * 3_600);
}

// ─── Upgrade history preserved after execute ─────────────────────────────────

#[test]
fn upgrade_history_recorded_after_execute() {
    let (env, admin, client) = setup();
    let name = Symbol::new(&env, "bounty");

    client.schedule_upgrade(
        &zero_hash(&env),
        &name,
        &1,
        &zero_hash(&env),
        &approvals(&env, &[admin.clone()]),
    );
    advance_time(&env, 48 * 3_600 + 1);
    client.execute_pending_upgrade(&approvals(&env, &[admin]));

    let history = client.get_upgrade_history(&name);
    assert_eq!(history.len(), 1);
    assert_eq!(history.get(0).unwrap().new_version, 1);
}

#[test]
fn schedule_upgrade_fails_below_threshold() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);
    let signer_set = vec![&env, s1.clone(), s2.clone(), s3.clone()];

    client.set_upgrade_signers(&signer_set, &2, &approvals(&env, &[admin]));

    let result = client.try_schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "escrow"),
        &1,
        &zero_hash(&env),
        &approvals(&env, &[s1]),
    );

    assert_eq!(result, Err(Ok(Error::BelowThreshold)));
}

#[test]
fn schedule_upgrade_rejects_duplicate_approvers() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);

    client.set_upgrade_signers(
        &vec![&env, s1.clone(), s2.clone(), s3.clone()],
        &2,
        &approvals(&env, &[admin]),
    );

    let result = client.try_schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "escrow"),
        &1,
        &zero_hash(&env),
        &approvals(&env, &[s1.clone(), s1]),
    );

    assert_eq!(result, Err(Ok(Error::DuplicateSigner)));
}

#[test]
fn schedule_upgrade_rejects_unregistered_approver() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);
    let outsider = Address::generate(&env);

    client.set_upgrade_signers(
        &vec![&env, s1.clone(), s2, s3],
        &2,
        &approvals(&env, &[admin]),
    );

    let result = client.try_schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "escrow"),
        &1,
        &zero_hash(&env),
        &approvals(&env, &[s1, outsider]),
    );

    assert_eq!(result, Err(Ok(Error::NotSigner)));
}

#[test]
fn single_compromised_key_cannot_schedule_upgrade() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);

    client.set_upgrade_signers(
        &vec![&env, s1.clone(), s2.clone(), s3.clone()],
        &2,
        &approvals(&env, &[admin]),
    );

    let result = client.try_schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "escrow"),
        &1,
        &zero_hash(&env),
        &approvals(&env, &[s1]),
    );

    assert_eq!(result, Err(Ok(Error::BelowThreshold)));
}

#[test]
fn direct_upgrade_contract_cannot_bypass_threshold() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);

    client.set_upgrade_signers(
        &vec![&env, s1.clone(), s2.clone(), s3.clone()],
        &2,
        &approvals(&env, &[admin]),
    );

    let result = client.try_upgrade_contract(
        &zero_hash(&env),
        &Symbol::new(&env, "upgrade_registry"),
        &1,
        &zero_hash(&env),
        &approvals(&env, &[s1.clone()]),
    );
    assert_eq!(result, Err(Ok(Error::BelowThreshold)));

    client.upgrade_contract(
        &zero_hash(&env),
        &Symbol::new(&env, "upgrade_registry"),
        &1,
        &zero_hash(&env),
        &approvals(&env, &[s1, s2]),
    );
}

#[test]
fn signer_rotation_supports_two_of_three() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);

    client.set_upgrade_signers(
        &vec![&env, s1.clone(), s2.clone(), s3.clone()],
        &2,
        &approvals(&env, &[admin]),
    );

    let config = client.get_upgrade_config();
    assert_eq!(config.threshold, 2);
    assert_eq!(config.signers.len(), 3);

    client.schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "escrow"),
        &1,
        &zero_hash(&env),
        &approvals(&env, &[s1, s2]),
    );
}

#[test]
fn signer_rotation_supports_three_of_five() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);
    let s4 = Address::generate(&env);
    let s5 = Address::generate(&env);

    client.set_upgrade_signers(
        &vec![
            &env,
            s1.clone(),
            s2.clone(),
            s3.clone(),
            s4.clone(),
            s5.clone(),
        ],
        &3,
        &approvals(&env, &[admin]),
    );

    let config = client.get_upgrade_config();
    assert_eq!(config.threshold, 3);
    assert_eq!(config.signers.len(), 5);

    client.schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "escrow"),
        &1,
        &zero_hash(&env),
        &approvals(&env, &[s1, s2, s3]),
    );
}

#[test]
fn execute_pending_upgrade_rechecks_threshold_independently() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);
    let name = Symbol::new(&env, "escrow");

    client.set_upgrade_signers(
        &vec![&env, s1.clone(), s2.clone(), s3.clone()],
        &2,
        &approvals(&env, &[admin]),
    );
    client.schedule_upgrade(
        &zero_hash(&env),
        &name,
        &1,
        &zero_hash(&env),
        &approvals(&env, &[s1.clone(), s2.clone()]),
    );
    advance_time(&env, 48 * 3_600 + 1);

    let result = client.try_execute_pending_upgrade(&approvals(&env, &[s1.clone()]));
    assert_eq!(result, Err(Ok(Error::BelowThreshold)));

    client.execute_pending_upgrade(&approvals(&env, &[s1, s3]));
    assert!(client.get_pending_upgrade().is_none());
}

#[test]
fn signer_rotation_requires_current_threshold() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);
    let s4 = Address::generate(&env);
    let s5 = Address::generate(&env);

    client.set_upgrade_signers(
        &vec![&env, s1.clone(), s2.clone(), s3.clone()],
        &2,
        &approvals(&env, &[admin]),
    );

    let result = client.try_set_upgrade_signers(
        &vec![
            &env,
            s1.clone(),
            s2.clone(),
            s3.clone(),
            s4.clone(),
            s5.clone(),
        ],
        &3,
        &approvals(&env, &[s1.clone()]),
    );
    assert_eq!(result, Err(Ok(Error::BelowThreshold)));

    client.set_upgrade_signers(
        &vec![&env, s1.clone(), s2.clone(), s3, s4, s5],
        &3,
        &approvals(&env, &[s1, s2]),
    );
    assert_eq!(client.get_upgrade_config().threshold, 3);
}

#[test]
fn admin_rotation_requires_current_threshold() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);
    let new_admin = Address::generate(&env);

    client.set_upgrade_signers(
        &vec![&env, s1.clone(), s2.clone(), s3],
        &2,
        &approvals(&env, &[admin]),
    );

    let result = client.try_set_admin(&new_admin, &approvals(&env, &[s1.clone()]));
    assert_eq!(result, Err(Ok(Error::BelowThreshold)));

    client.set_admin(&new_admin, &approvals(&env, &[s1, s2]));
    assert_eq!(client.get_admin(), new_admin);
}

#[test]
fn schedule_event_lists_approved_signers() {
    let (env, admin, client) = setup();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);
    let name = Symbol::new(&env, "escrow");
    let hash = zero_hash(&env);
    let approved = approvals(&env, &[s1.clone(), s2.clone()]);

    client.set_upgrade_signers(&vec![&env, s1, s2, s3], &2, &approvals(&env, &[admin]));
    client.schedule_upgrade(&hash, &name, &1, &hash, &approved);

    let events = env.events().all();
    let last = events.last().unwrap();
    let payload: (u32, u64, BytesN<32>, Vec<Address>) =
        <(u32, u64, BytesN<32>, Vec<Address>)>::try_from_val(&env, &last.2).unwrap();
    assert_eq!(payload.0, 1);
    assert_eq!(payload.2, hash);
    assert_eq!(payload.3, approved);
}

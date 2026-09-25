//! Security Regression Suite — MentorsMind Protocol
//!
//! Continuously validates that identified attack vectors remain closed.
//! Every test corresponds to a specific threat category; the category tag
//! appears in the test name so CI can filter by threat class.
//!
//! # Threat categories
//!
//! | Tag              | Description                                          |
//! |------------------|------------------------------------------------------|
//! | `priv_esc`       | Privilege escalation — non-admin acquires admin power|
//! | `replay`         | Replay attacks — reusing a spent nonce/sig           |
//! | `unauth_upgrade` | Unauthorized upgrades — bypass M-of-N or timelock    |
//! | `multisig_bypass`| Multisig bypass — execute without enough approvals   |
//! | `timelock_manip` | Timelock manipulation — execute before delay elapses |
//! | `reinit`         | Re-initialization — overwrite initialized state      |
//! | `param_abuse`    | Parameter abuse — set params without governance role |
//!
//! # Running
//!
//! ```bash
//! # All security tests
//! cargo test -p mentorminds-integration-tests --test security_regression
//!
//! # One threat category
//! cargo test -p mentorminds-integration-tests --test security_regression priv_esc
//! ```

#![cfg(test)]
extern crate std;

use std::panic::{catch_unwind, AssertUnwindSafe};

use mentorminds_multisig::{MultiSigContract, MultiSigContractClient, TransactionStatus};
use mentorminds_rbac::{RbacContract, RbacContractClient};
use mentorminds_timelock::{
    Error as TimelockError, TimelockController, TimelockControllerClient, MIN_DELAY,
    TIMESTAMP_TOLERANCE_SECS,
};
use mentorminds_upgrade_registry::{
    Error as UpgradeError, UpgradeRegistryContract, UpgradeRegistryContractClient,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger, LedgerInfo},
    token::StellarAssetClient,
    Address, BytesN, Env, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Shared helpers
// ---------------------------------------------------------------------------

fn advance_time(env: &Env, secs: u64) {
    let ts = env.ledger().timestamp();
    env.ledger().set(LedgerInfo {
        timestamp: ts + secs,
        protocol_version: 22,
        sequence_number: env.ledger().sequence() + 1,
        network_id: Default::default(),
        base_reserve: 10,
        min_temp_entry_ttl: 100,
        min_persistent_entry_ttl: 100,
        max_entry_ttl: 9_999_999,
    });
}

fn zero_hash(env: &Env) -> BytesN<32> {
    BytesN::from_array(env, &[0u8; 32])
}

fn addr_vec(env: &Env, addrs: &[Address]) -> Vec<Address> {
    let mut v = Vec::new(env);
    for a in addrs {
        v.push_back(a.clone());
    }
    v
}

fn assert_panics<F: FnOnce()>(f: F) {
    assert!(
        catch_unwind(AssertUnwindSafe(f)).is_err(),
        "expected call to panic but it succeeded"
    );
}

// ---------------------------------------------------------------------------
// Fixture builders
// ---------------------------------------------------------------------------

fn setup_rbac(env: &Env) -> (Address, RbacContractClient<'static>) {
    let admin = Address::generate(env);
    let id = env.register(RbacContract, ());
    let client = RbacContractClient::new(env, &id);
    client.initialize(&admin);
    (admin, client)
}

fn setup_upgrade_registry(env: &Env) -> (Address, UpgradeRegistryContractClient<'static>) {
    let admin = Address::generate(env);
    let id = env.register(UpgradeRegistryContract, ());
    let client = UpgradeRegistryContractClient::new(env, &id);
    client.initialize(&admin);
    (admin, client)
}

fn setup_timelock(env: &Env) -> (Address, TimelockControllerClient<'static>) {
    let admin = Address::generate(env);
    env.ledger().with_mut(|li| li.timestamp = 1_000);
    let id = env.register(TimelockController, ());
    let client = TimelockControllerClient::new(env, &id);
    client.initialize(&admin);
    (admin, client)
}

/// Returns (admin, [s1,s2,s3], threshold=2, client).
fn setup_multisig(env: &Env) -> (Address, [Address; 3], MultiSigContractClient<'static>) {
    let admin = Address::generate(env);
    let s1 = Address::generate(env);
    let s2 = Address::generate(env);
    let s3 = Address::generate(env);
    let signers = addr_vec(env, &[s1.clone(), s2.clone(), s3.clone()]);
    let id = env.register(MultiSigContract, ());
    let client = MultiSigContractClient::new(env, &id);
    client.initialize(&admin, &signers, &2);
    (admin, [s1, s2, s3], client)
}

/// Mint a Stellar asset token and return (address, admin_client).
fn create_sat(env: &Env, admin: &Address) -> (Address, StellarAssetClient<'static>) {
    let addr = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    (addr.clone(), StellarAssetClient::new(env, &addr))
}

// ============================================================================
// 1. PRIVILEGE ESCALATION (priv_esc)
// ============================================================================

/// Non-admin cannot grant roles — granting requires super-admin auth.
#[test]
fn priv_esc_non_admin_cannot_grant_role() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, rbac) = setup_rbac(&env);
    let attacker = Address::generate(&env);
    let victim = Address::generate(&env);
    let role = Symbol::new(&env, "ESCROW_ADMIN");

    // Attacker tries to grant themselves ESCROW_ADMIN.
    let result = rbac.try_grant_role(&attacker, &role, &attacker);
    assert!(result.is_err(), "non-admin must not grant roles");

    // Legitimate admin grant works.
    rbac.grant_role(&admin, &role, &victim);
    assert!(rbac.has_role(&role, &victim));

    // Attacker still has no role.
    assert!(!rbac.has_role(&role, &attacker));
}

/// Non-admin cannot revoke an existing role.
#[test]
fn priv_esc_non_admin_cannot_revoke_role() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, rbac) = setup_rbac(&env);
    let holder = Address::generate(&env);
    let attacker = Address::generate(&env);
    let role = Symbol::new(&env, "KYC_OPERATOR");

    rbac.grant_role(&admin, &role, &holder);

    let result = rbac.try_revoke_role(&attacker, &role, &holder);
    assert!(result.is_err(), "non-admin must not revoke roles");
    assert!(rbac.has_role(&role, &holder), "role must still be held");
}

/// Revoking a role that was never granted returns an error, not silent success.
#[test]
fn priv_esc_revoke_nonexistent_role_returns_error() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, rbac) = setup_rbac(&env);
    let target = Address::generate(&env);
    let role = Symbol::new(&env, "ORACLE_FEEDER");

    let result = rbac.try_revoke_role(&admin, &role, &target);
    assert!(result.is_err(), "revoking unheld role must fail");
}

/// A holder of one role cannot use it to acquire a different role.
#[test]
fn priv_esc_role_holder_cannot_escalate_to_super_admin() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, rbac) = setup_rbac(&env);
    let operator = Address::generate(&env);
    let super_admin_role = Symbol::new(&env, "SUPER_ADMIN");
    let kyc_role = Symbol::new(&env, "KYC_OPERATOR");

    // Give operator only KYC role.
    rbac.grant_role(&admin, &kyc_role, &operator);

    // Operator attempts to self-grant SUPER_ADMIN.
    let result = rbac.try_grant_role(&operator, &super_admin_role, &operator);
    assert!(result.is_err(), "KYC operator must not self-elevate to SUPER_ADMIN");
}

/// Upgrade registry: non-signer cannot schedule an upgrade.
#[test]
fn priv_esc_outsider_cannot_schedule_upgrade() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, registry) = setup_upgrade_registry(&env);
    let outsider = Address::generate(&env);

    let result = registry.try_schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "escrow"),
        &1,
        &zero_hash(&env),
        &addr_vec(&env, &[outsider]),
    );
    assert_eq!(result, Err(Ok(UpgradeError::NotSigner)));
}

/// Multisig: a non-signer cannot approve a transaction.
#[test]
fn priv_esc_non_signer_cannot_approve_multisig_tx() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, [s1, _, _], ms) = setup_multisig(&env);
    let outsider = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (token, sac) = create_sat(&env, &admin);
    sac.mint(&ms.address, &1_000);

    let tx_id = ms.propose_transaction(&s1, &recipient, &token, &100, &0u64);

    assert_panics(|| ms.approve_transaction(&outsider, &tx_id));

    // Real signer can approve.
    ms.approve_transaction(&s1, &tx_id);
    let tx = ms.get_transaction(&tx_id);
    assert_eq!(tx.approvals, 1);
}

// ============================================================================
// 2. REPLAY ATTACKS (replay)
// ============================================================================

/// Timelock: scheduling the same operation salt twice produces a distinct op_id
/// and the second schedule is a new operation, not a replay.
#[test]
fn replay_timelock_same_salt_different_nonce_distinct_op_ids() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, tl) = setup_timelock(&env);

    let target = Address::generate(&env);
    let noop = Symbol::new(&env, "noop");
    let args = Vec::new(&env);
    let salt = zero_hash(&env);

    let id1 = tl.schedule(&admin, &target, &noop, &args, &MIN_DELAY, &salt);

    // Advance past the operation and execute to consume it.
    advance_time(&env, MIN_DELAY + TIMESTAMP_TOLERANCE_SECS + 1);
    tl.execute(&id1);

    // Scheduling again with the same salt gets a fresh op_id (nonce incremented).
    let id2 = tl.schedule(&admin, &target, &noop, &args, &MIN_DELAY, &salt);
    assert_ne!(id1, id2, "nonce must produce distinct op_id after reuse of salt");
}

/// Timelock: an executed operation cannot be executed again.
#[test]
fn replay_timelock_executed_op_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, tl) = setup_timelock(&env);

    let target = Address::generate(&env);
    let noop = Symbol::new(&env, "noop");
    let args = Vec::new(&env);

    let op_id = tl.schedule(&admin, &target, &noop, &args, &MIN_DELAY, &zero_hash(&env));
    advance_time(&env, MIN_DELAY + TIMESTAMP_TOLERANCE_SECS + 1);
    tl.execute(&op_id);

    let result = tl.try_execute(&op_id);
    assert_eq!(result, Err(Ok(TimelockError::AlreadyDone)));
}

/// Multisig: a signer cannot double-approve the same transaction.
#[test]
fn replay_multisig_double_approval_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (_admin, [s1, _, _], ms) = setup_multisig(&env);
    let recipient = Address::generate(&env);
    let (token, sac) = create_sat(&env, &Address::generate(&env));
    sac.mint(&ms.address, &500);

    let tx_id = ms.propose_transaction(&s1, &recipient, &token, &100, &0u64);
    ms.approve_transaction(&s1, &tx_id);

    assert_panics(|| ms.approve_transaction(&s1, &tx_id));

    let tx = ms.get_transaction(&tx_id);
    assert_eq!(tx.approvals, 1, "approval count must not increase on replay");
}

/// Upgrade registry: an executed upgrade cannot be re-executed.
/// After execute_pending_upgrade the slot is empty; re-executing panics.
#[test]
fn replay_upgrade_executed_upgrade_cannot_repeat() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, registry) = setup_upgrade_registry(&env);

    registry.schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "bounty"),
        &1,
        &zero_hash(&env),
        &addr_vec(&env, &[admin.clone()]),
    );
    advance_time(&env, 48 * 3_600 + 1);
    registry.execute_pending_upgrade(&addr_vec(&env, &[admin.clone()]));

    // No pending upgrade — second execute must fail.
    let result = registry.try_execute_pending_upgrade(&addr_vec(&env, &[admin]));
    assert_eq!(result, Err(Ok(UpgradeError::NoPendingUpgrade)));
}

/// Upgrade registry: version registry prevents rollback to a previously used version.
#[test]
fn replay_upgrade_version_rollback_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, registry) = setup_upgrade_registry(&env);
    let name = Symbol::new(&env, "staking");

    registry.register_upgrade(&name, &0, &3, &zero_hash(&env));
    assert_eq!(registry.get_latest_version(&name), 3);

    // v3 again
    let r3 = registry.try_schedule_upgrade(
        &zero_hash(&env), &name, &3, &zero_hash(&env),
        &addr_vec(&env, &[admin.clone()]),
    );
    assert_eq!(r3, Err(Ok(UpgradeError::VersionNotMonotonic)));

    // v2 (rollback)
    let r2 = registry.try_schedule_upgrade(
        &zero_hash(&env), &name, &2, &zero_hash(&env),
        &addr_vec(&env, &[admin.clone()]),
    );
    assert_eq!(r2, Err(Ok(UpgradeError::VersionNotMonotonic)));

    // v4 (correct)
    registry.schedule_upgrade(
        &zero_hash(&env), &name, &4, &zero_hash(&env),
        &addr_vec(&env, &[admin]),
    );
}

// ============================================================================
// 3. UNAUTHORIZED UPGRADES (unauth_upgrade)
// ============================================================================

/// A single key below the configured M-of-N threshold cannot schedule an upgrade.
#[test]
fn unauth_upgrade_single_key_below_threshold_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, registry) = setup_upgrade_registry(&env);
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);

    registry.set_upgrade_signers(
        &addr_vec(&env, &[s1.clone(), s2.clone(), s3.clone()]),
        &2,
        &addr_vec(&env, &[admin]),
    );

    let result = registry.try_schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "governance"),
        &1,
        &zero_hash(&env),
        &addr_vec(&env, &[s1]),
    );
    assert_eq!(result, Err(Ok(UpgradeError::BelowThreshold)));
}

/// An outsider address that was never registered as signer is explicitly rejected
/// rather than silently falling through.
#[test]
fn unauth_upgrade_unregistered_signer_explicitly_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, registry) = setup_upgrade_registry(&env);
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let outsider = Address::generate(&env);

    registry.set_upgrade_signers(
        &addr_vec(&env, &[s1.clone(), s2.clone()]),
        &2,
        &addr_vec(&env, &[admin]),
    );

    let result = registry.try_schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "timelock"),
        &1,
        &zero_hash(&env),
        &addr_vec(&env, &[s1, outsider]),
    );
    assert_eq!(result, Err(Ok(UpgradeError::NotSigner)));
}

/// Duplicate signer in the approver list is not double-counted.
#[test]
fn unauth_upgrade_duplicate_signer_in_approvers_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, registry) = setup_upgrade_registry(&env);
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);

    registry.set_upgrade_signers(
        &addr_vec(&env, &[s1.clone(), s2.clone(), s3.clone()]),
        &2,
        &addr_vec(&env, &[admin]),
    );

    // Passing s1 twice should not satisfy threshold-2.
    let result = registry.try_schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "rbac"),
        &1,
        &zero_hash(&env),
        &addr_vec(&env, &[s1.clone(), s1]),
    );
    assert_eq!(result, Err(Ok(UpgradeError::DuplicateSigner)));
}

/// Execution step independently re-validates approvers; a new compromised key
/// cannot execute an upgrade that was correctly scheduled.
#[test]
fn unauth_upgrade_execute_step_rechecks_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, registry) = setup_upgrade_registry(&env);
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let s3 = Address::generate(&env);

    registry.set_upgrade_signers(
        &addr_vec(&env, &[s1.clone(), s2.clone(), s3.clone()]),
        &2,
        &addr_vec(&env, &[admin]),
    );
    registry.schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "treasury"),
        &1,
        &zero_hash(&env),
        &addr_vec(&env, &[s1.clone(), s2.clone()]),
    );
    advance_time(&env, 48 * 3_600 + 1);

    // Only one signer approves the execution — must fail.
    let result = registry.try_execute_pending_upgrade(&addr_vec(&env, &[s1.clone()]));
    assert_eq!(result, Err(Ok(UpgradeError::BelowThreshold)));

    // Two signers — succeeds.
    registry.execute_pending_upgrade(&addr_vec(&env, &[s1, s3]));
    assert!(registry.get_pending_upgrade().is_none());
}

/// Only one upgrade can be in flight; a second schedule call is rejected
/// until the first is either executed or cancelled.
#[test]
fn unauth_upgrade_concurrent_upgrade_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, registry) = setup_upgrade_registry(&env);

    registry.schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "snapshot"),
        &1,
        &zero_hash(&env),
        &addr_vec(&env, &[admin.clone()]),
    );

    let result = registry.try_schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "snapshot"),
        &2,
        &zero_hash(&env),
        &addr_vec(&env, &[admin]),
    );
    assert_eq!(result, Err(Ok(UpgradeError::UpgradePending)));
}

/// Admin rotation requires the full current threshold — a single compromised key
/// cannot redirect the admin address.
#[test]
fn unauth_upgrade_admin_rotation_requires_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, registry) = setup_upgrade_registry(&env);
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let new_admin = Address::generate(&env);

    registry.set_upgrade_signers(
        &addr_vec(&env, &[s1.clone(), s2.clone()]),
        &2,
        &addr_vec(&env, &[admin]),
    );

    let result = registry.try_set_admin(&new_admin, &addr_vec(&env, &[s1.clone()]));
    assert_eq!(result, Err(Ok(UpgradeError::BelowThreshold)));

    registry.set_admin(&new_admin, &addr_vec(&env, &[s1, s2]));
    assert_eq!(registry.get_admin(), new_admin);
}

// ============================================================================
// 4. MULTISIG BYPASS (multisig_bypass)
// ============================================================================
/// Executing below threshold panics — threshold is enforced at execution time,
/// not just at approval time.
#[test]
fn multisig_bypass_execute_below_threshold_panics() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, [s1, _, _], ms) = setup_multisig(&env);
    let recipient = Address::generate(&env);
    let (token, sac) = create_sat(&env, &admin);
    sac.mint(&ms.address, &1_000);

    // Threshold is 2; only one approval given.
    let tx_id = ms.propose_transaction(&s1, &recipient, &token, &100, &0u64);
    ms.approve_transaction(&s1, &tx_id);

    assert_panics(|| ms.execute_transaction(&s1, &tx_id));

    // Approval count is still 1; funds not moved.
    let tx = ms.get_transaction(&tx_id);
    assert_eq!(tx.approvals, 1);
    assert_eq!(tx.status, TransactionStatus::Pending);
}

/// Zero approvals — can never execute regardless of timelock.
#[test]
fn multisig_bypass_zero_approvals_cannot_execute() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, [s1, _, _], ms) = setup_multisig(&env);
    let recipient = Address::generate(&env);
    let (token, sac) = create_sat(&env, &admin);
    sac.mint(&ms.address, &500);

    let tx_id = ms.propose_transaction(&s1, &recipient, &token, &100, &0u64);
    advance_time(&env, 10 * 86_400); // well past any timelock

    assert_panics(|| ms.execute_transaction(&s1, &tx_id));
}

/// A cancelled transaction cannot be approved or executed after cancellation.
#[test]
fn multisig_bypass_cancelled_tx_locked_out() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, [s1, s2, _], ms) = setup_multisig(&env);
    let recipient = Address::generate(&env);
    let (token, sac) = create_sat(&env, &admin);
    sac.mint(&ms.address, &500);

    let tx_id = ms.propose_transaction(&s1, &recipient, &token, &100, &0u64);
    ms.approve_transaction(&s1, &tx_id);
    ms.cancel_transaction(&admin, &tx_id);

    // Cannot approve after cancel.
    assert_panics(|| ms.approve_transaction(&s2, &tx_id));
    // Cannot execute after cancel.
    assert_panics(|| ms.execute_transaction(&s1, &tx_id));

    let tx = ms.get_transaction(&tx_id);
    assert_eq!(tx.status, TransactionStatus::Cancelled);
}

/// An executed transaction cannot receive further approvals or be re-executed.
#[test]
fn multisig_bypass_executed_tx_locked_out() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, [s1, s2, s3], ms) = setup_multisig(&env);
    let recipient = Address::generate(&env);
    let (token, sac) = create_sat(&env, &admin);
    sac.mint(&ms.address, &1_000);

    let tx_id = ms.propose_transaction(&s1, &recipient, &token, &100, &0u64);
    ms.approve_transaction(&s1, &tx_id);
    ms.approve_transaction(&s2, &tx_id);
    ms.execute_transaction(&s1, &tx_id);

    // Both approval and execution must be rejected on an already-executed tx.
    assert_panics(|| ms.approve_transaction(&s3, &tx_id));
    assert_panics(|| ms.execute_transaction(&s1, &tx_id));

    let tx = ms.get_transaction(&tx_id);
    assert_eq!(tx.status, TransactionStatus::Executed);
}

/// A stranger (non-admin, non-proposer) cannot cancel someone else's transaction.
#[test]
fn multisig_bypass_stranger_cannot_cancel_tx() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, [s1, _, _], ms) = setup_multisig(&env);
    let stranger = Address::generate(&env);
    let recipient = Address::generate(&env);
    let (token, sac) = create_sat(&env, &admin);
    sac.mint(&ms.address, &500);

    let tx_id = ms.propose_transaction(&s1, &recipient, &token, &100, &0u64);

    assert_panics(|| ms.cancel_transaction(&stranger, &tx_id));

    // Transaction is still pending.
    assert_eq!(ms.get_transaction(&tx_id).status, TransactionStatus::Pending);
}

/// Removing a signer below threshold level is rejected — prevents threshold
/// from becoming unsatisfiable.
#[test]
fn multisig_bypass_remove_signer_below_threshold_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, [s1, s2, s3], ms) = setup_multisig(&env);

    // threshold=2, signers=3 — removing one leaves 2=threshold, still valid.
    ms.remove_signer(&admin, &s3);

    // Now signers=2, threshold=2. Removing another would leave signers=1 < threshold=2.
    assert_panics(|| ms.remove_signer(&admin, &s2));

    // s1 is still a signer, s2 is still a signer.
    let tx_id = ms.propose_transaction(&s1, &Address::generate(&env), &Address::generate(&env), &1, &0u64);
    ms.approve_transaction(&s1, &tx_id);
    ms.approve_transaction(&s2, &tx_id);
    // Transaction reaches threshold with the remaining signers.
    assert_eq!(ms.get_transaction(&tx_id).approvals, 2);
}

/// threshold=0 is rejected — a zero threshold would allow execution with no
/// approvals.
#[test]
fn multisig_bypass_zero_threshold_rejected_at_init() {
    let env = Env::default();
    env.mock_all_auths();
    let s1 = Address::generate(&env);
    let s2 = Address::generate(&env);
    let admin = Address::generate(&env);
    let id = env.register(MultiSigContract, ());
    let ms = MultiSigContractClient::new(&env, &id);

    assert_panics(|| {
        ms.initialize(&admin, &addr_vec(&env, &[s1, s2]), &0);
    });
}

/// threshold > signer count is rejected — would make execution permanently
/// impossible.
#[test]
fn multisig_bypass_threshold_exceeds_signers_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let s1 = Address::generate(&env);
    let id = env.register(MultiSigContract, ());
    let ms = MultiSigContractClient::new(&env, &id);

    assert_panics(|| {
        ms.initialize(&admin, &addr_vec(&env, &[s1]), &5);
    });
}

// ============================================================================
// 5. TIMELOCK MANIPULATION (timelock_manip)
// ============================================================================

/// Executing before the delay elapses is always rejected.
#[test]
fn timelock_manip_execute_before_delay_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, tl) = setup_timelock(&env);
    let target = Address::generate(&env);

    let op_id = tl.schedule(
        &admin, &target, &Symbol::new(&env, "noop"),
        &Vec::new(&env), &MIN_DELAY, &zero_hash(&env),
    );

    // Advance to one second before ready.
    advance_time(&env, MIN_DELAY - 1);

    let result = tl.try_execute(&op_id);
    assert_eq!(result, Err(Ok(TimelockError::NotReady)));
}

/// Executing exactly at the tolerance boundary (ready_at - tolerance) is
/// rejected; the contract must be at or past ready_at.
#[test]
fn timelock_manip_execute_at_tolerance_boundary_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, tl) = setup_timelock(&env);
    let target = Address::generate(&env);

    let op_id = tl.schedule(
        &admin, &target, &Symbol::new(&env, "noop"),
        &Vec::new(&env), &MIN_DELAY, &zero_hash(&env),
    );

    // Advance to exactly MIN_DELAY — still within tolerance window boundary.
    // ready_at = 1_000 + MIN_DELAY; tolerance = 60s.
    // execute requires timestamp >= ready_at - tolerance, but must also not
    // be before ready_at for a strict contract. Test the boundary.
    advance_time(&env, MIN_DELAY.saturating_sub(TIMESTAMP_TOLERANCE_SECS + 1));

    let result = tl.try_execute(&op_id);
    assert_eq!(result, Err(Ok(TimelockError::NotReady)));
}

/// An operation scheduled with delay below MIN_DELAY is rejected at schedule
/// time — cannot be sneaked in with a shorter window.
#[test]
fn timelock_manip_below_min_delay_rejected_at_schedule() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, tl) = setup_timelock(&env);
    let target = Address::generate(&env);

    let result = tl.try_schedule(
        &admin, &target, &Symbol::new(&env, "noop"),
        &Vec::new(&env), &(MIN_DELAY - 1), &zero_hash(&env),
    );
    assert_eq!(result, Err(Ok(TimelockError::InvalidDelay)));
}

/// Cancelling an operation removes it; execution after cancel is rejected.
#[test]
fn timelock_manip_cancelled_op_cannot_execute() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, tl) = setup_timelock(&env);
    let target = Address::generate(&env);

    let op_id = tl.schedule(
        &admin, &target, &Symbol::new(&env, "noop"),
        &Vec::new(&env), &MIN_DELAY, &zero_hash(&env),
    );
    tl.cancel(&admin, &op_id);

    advance_time(&env, MIN_DELAY + TIMESTAMP_TOLERANCE_SECS + 1);

    let result = tl.try_execute(&op_id);
    assert_eq!(result, Err(Ok(TimelockError::OperationNotFound)));
}

/// Non-admin cannot cancel a pending operation.
#[test]
fn timelock_manip_non_admin_cannot_cancel() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, tl) = setup_timelock(&env);
    let attacker = Address::generate(&env);
    let target = Address::generate(&env);

    let op_id = tl.schedule(
        &admin, &target, &Symbol::new(&env, "noop"),
        &Vec::new(&env), &MIN_DELAY, &zero_hash(&env),
    );

    let result = tl.try_cancel(&attacker, &op_id);
    assert_eq!(result, Err(Ok(TimelockError::NotAdmin)));

    // Op still present — can be executed after delay.
    advance_time(&env, MIN_DELAY + TIMESTAMP_TOLERANCE_SECS + 1);
    tl.execute(&op_id); // must not panic
}

/// An expired operation (past OPERATION_EXPIRY_SECS after ready_at) cannot
/// execute — prevents stale operations from being triggered arbitrarily late.
#[test]
fn timelock_manip_expired_operation_rejected() {
    use mentorminds_timelock::OPERATION_EXPIRY_SECS;

    let env = Env::default();
    env.mock_all_auths();
    let (admin, tl) = setup_timelock(&env);
    let target = Address::generate(&env);

    let op_id = tl.schedule(
        &admin, &target, &Symbol::new(&env, "noop"),
        &Vec::new(&env), &MIN_DELAY, &zero_hash(&env),
    );

    // Advance past ready_at AND past the expiry window.
    advance_time(&env, MIN_DELAY + OPERATION_EXPIRY_SECS + 1);

    let result = tl.try_execute(&op_id);
    assert_eq!(result, Err(Ok(TimelockError::NotReady)),
        "operation past expiry must be rejected");
}

/// The upgrade registry enforces its own 48-hour timelock independently of
/// the standalone timelock contract.
#[test]
fn timelock_manip_upgrade_registry_timelock_enforced() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, registry) = setup_upgrade_registry(&env);

    registry.schedule_upgrade(
        &zero_hash(&env),
        &Symbol::new(&env, "rbac"),
        &1,
        &zero_hash(&env),
        &addr_vec(&env, &[admin.clone()]),
    );

    // Advance by only 47 hours — one hour short.
    advance_time(&env, 47 * 3_600);

    let result = registry.try_execute_pending_upgrade(&addr_vec(&env, &[admin]));
    assert_eq!(result, Err(Ok(UpgradeError::TimelockNotElapsed)));
}

/// Multisig time-lock: executing before execute_after timestamp panics.
#[test]
fn timelock_manip_multisig_execute_before_timelock_panics() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.timestamp = 0);
    let (admin, [s1, s2, _], ms) = setup_multisig(&env);
    let recipient = Address::generate(&env);
    let (token, sac) = create_sat(&env, &admin);
    sac.mint(&ms.address, &1_000);

    // execute_after = 7 days from now
    let execute_after: u64 = 7 * 86_400;
    let tx_id = ms.propose_transaction(&s1, &recipient, &token, &100, &execute_after);
    ms.approve_transaction(&s1, &tx_id);
    ms.approve_transaction(&s2, &tx_id);

    // Try to execute with 0 seconds elapsed.
    assert_panics(|| ms.execute_transaction(&s1, &tx_id));

    // Advance past the time-lock.
    advance_time(&env, execute_after + 1);
    ms.execute_transaction(&s1, &tx_id); // must succeed
    assert_eq!(ms.get_transaction(&tx_id).status, TransactionStatus::Executed);
}

// ============================================================================
// 6. RE-INITIALIZATION (reinit)
// ============================================================================

/// All contracts must reject a second initialize call.
#[test]
fn reinit_rbac_double_init_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, rbac) = setup_rbac(&env);

    let result = rbac.try_initialize(&admin);
    assert!(result.is_err(), "RBAC: second initialize must fail");
}

#[test]
fn reinit_upgrade_registry_double_init_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, registry) = setup_upgrade_registry(&env);

    let result = registry.try_initialize(&admin);
    assert!(result.is_err(), "UpgradeRegistry: second initialize must fail");
}

#[test]
fn reinit_timelock_double_init_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let (admin, tl) = setup_timelock(&env);

    let result = tl.try_initialize(&admin);
    assert_eq!(result, Err(Ok(TimelockError::AlreadyInitialized)));
}

#[test]
fn reinit_multisig_double_init_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let s1 = Address::generate(&env);
    let id = env.register(MultiSigContract, ());
    let ms = MultiSigContractClient::new(&env, &id);
    ms.initialize(&admin, &addr_vec(&env, &[s1.clone()]), &1);

    assert_panics(|| ms.initialize(&admin, &addr_vec(&env, &[s1]), &1));
}

/// Re-initialization must not overwrite existing admin state — attacker
/// cannot take over by calling initialize with their own address.
#[test]
fn reinit_upgrade_registry_admin_unchanged_after_failed_reinit() {
    let env = Env::default();
    env.mock_all_auths();
    let (original_admin, registry) = setup_upgrade_registry(&env);
    let attacker = Address::generate(&env);

    let _ = registry.try_initialize(&attacker);

    // Admin must still be the original.
    assert_eq!(registry.get_admin(), original_admin);
}

// ============================================================================
// 7. PARAMETER ABUSE (param_abuse)
// ============================================================================

/// set_param without GOVERNANCE_ADMIN role must panic.
/// Uses the performance bond contract which carries the protocol param registry.
#[test]
fn param_abuse_set_param_without_role_panics() {
    use mentorminds_performance_bond::{PerformanceBondContract, PerformanceBondContractClient};
    use mentorminds_rbac::{RbacContract, RbacContractClient};

    let env = Env::default();
    env.mock_all_auths();

    // Deploy RBAC — attacker has NO roles.
    let rbac_admin = Address::generate(&env);
    let rbac_id = env.register(RbacContract, ());
    RbacContractClient::new(&env, &rbac_id).initialize(&rbac_admin);

    let bond_admin = Address::generate(&env);
    let insurance = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let (mnt_token, _sac) = create_sat(&env, &token_admin);

    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&bond_admin, &mnt_token, &insurance, &rbac_id);

    let attacker = Address::generate(&env);
    let key = soroban_sdk::Symbol::new(&env, "MIN_BOND");

    // Attacker has no GOVERNANCE_ADMIN role — must panic.
    assert_panics(|| bond.set_param(&attacker, &key, &1i128));
}

/// get_all_params returns the complete canonical parameter set including
/// governance-unset keys at their compile-time defaults.
#[test]
fn param_abuse_get_all_params_returns_complete_set() {
    use mentorminds_performance_bond::{PerformanceBondContract, PerformanceBondContractClient};
    use mentorminds_rbac::{RbacContract, RbacContractClient};

    let env = Env::default();
    env.mock_all_auths();

    let rbac_admin = Address::generate(&env);
    let rbac_id = env.register(RbacContract, ());
    RbacContractClient::new(&env, &rbac_id).initialize(&rbac_admin);

    let bond_admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let (mnt_token, _sac) = create_sat(&env, &token_admin);

    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&bond_admin, &mnt_token, &Address::generate(&env), &rbac_id);

    let params = bond.get_all_params();

    // Must contain at least the canonical keys (MIN_BOND, COOLDOWN, etc.)
    assert!(params.len() >= 2, "get_all_params must return all canonical keys");

    // MIN_BOND default = 100_000_000
    let min_bond_key = soroban_sdk::Symbol::new(&env, "MIN_BOND");
    let min_bond_val = params
        .iter()
        .find(|(k, _)| *k == min_bond_key)
        .map(|(_, v)| v);
    assert_eq!(min_bond_val, Some(100_000_000i128), "MIN_BOND must default to 100 MNT");

    // COOLDOWN default = 30
    let cooldown_key = soroban_sdk::Symbol::new(&env, "COOLDOWN");
    let cooldown_val = params
        .iter()
        .find(|(k, _)| *k == cooldown_key)
        .map(|(_, v)| v);
    assert_eq!(cooldown_val, Some(30i128), "COOLDOWN must default to 30 days");
}

/// Governance admin CAN update a parameter; the new value is then enforced.
#[test]
fn param_abuse_governance_admin_can_update_param() {
    use mentorminds_performance_bond::{PerformanceBondContract, PerformanceBondContractClient};
    use mentorminds_rbac::{RbacContract, RbacContractClient};

    let env = Env::default();
    env.mock_all_auths();

    // Set up RBAC and grant GOVERNANCE_ADMIN to an executor address.
    let rbac_admin = Address::generate(&env);
    let rbac_id = env.register(RbacContract, ());
    let rbac = RbacContractClient::new(&env, &rbac_id);
    rbac.initialize(&rbac_admin);

    let gov_executor = Address::generate(&env);
    let gov_role = soroban_sdk::Symbol::new(&env, "GOVERNANCE_ADMIN");
    rbac.grant_role(&rbac_admin, &gov_role, &gov_executor);

    // Deploy performance bond backed by the RBAC contract.
    let bond_admin = Address::generate(&env);
    let token_admin = Address::generate(&env);
    let (mnt_token, _sac) = create_sat(&env, &token_admin);
    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(&bond_admin, &mnt_token, &Address::generate(&env), &rbac_id);

    // Governor updates MIN_BOND from 100 MNT to 200 MNT.
    let min_bond_key = soroban_sdk::Symbol::new(&env, "MIN_BOND");
    let new_value: i128 = 200_000_000;
    bond.set_param(&gov_executor, &min_bond_key, &new_value);

    // get_param reflects the new governance value.
    let live = bond.get_param(&min_bond_key, &100_000_000i128);
    assert_eq!(live, new_value, "MIN_BOND must reflect governance update");
}

/// Negative parameter values are rejected regardless of caller.
#[test]
fn param_abuse_negative_value_always_rejected() {
    use mentorminds_performance_bond::{PerformanceBondContract, PerformanceBondContractClient};
    use mentorminds_rbac::{RbacContract, RbacContractClient};

    let env = Env::default();
    env.mock_all_auths();

    let rbac_admin = Address::generate(&env);
    let rbac_id = env.register(RbacContract, ());
    let rbac = RbacContractClient::new(&env, &rbac_id);
    rbac.initialize(&rbac_admin);

    let gov_executor = Address::generate(&env);
    let gov_role = soroban_sdk::Symbol::new(&env, "GOVERNANCE_ADMIN");
    rbac.grant_role(&rbac_admin, &gov_role, &gov_executor);

    let token_admin = Address::generate(&env);
    let (mnt_token, _sac) = create_sat(&env, &token_admin);
    let bond_id = env.register(PerformanceBondContract, ());
    let bond = PerformanceBondContractClient::new(&env, &bond_id);
    bond.initialize(
        &Address::generate(&env), &mnt_token, &Address::generate(&env), &rbac_id,
    );

    let key = soroban_sdk::Symbol::new(&env, "MIN_BOND");
    assert_panics(|| bond.set_param(&gov_executor, &key, &(-1i128)));
}

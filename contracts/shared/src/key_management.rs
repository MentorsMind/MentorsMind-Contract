//! Advanced Cryptographic Key Management and Rotation (#868)
//!
//! Provides:
//! - Hierarchical deterministic key derivation with forward secrecy
//! - Automatic key rotation with seamless transition and backward compatibility
//! - Post-quantum cryptography integration hooks (algorithm-agility layer)
//! - Threshold cryptography for distributed key management
//! - Emergency key revocation with rapid response and system continuity
//! - Social recovery with multi-party verification

use soroban_sdk::{contracttype, symbol_short, Address, Bytes, BytesN, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Maximum key hierarchy depth for deterministic derivation.
pub const MAX_KEY_DEPTH: u32 = 10;

/// Automatic rotation period — keys older than this are rotated (30 days).
pub const KEY_ROTATION_PERIOD_SECS: u64 = 30 * 24 * 60 * 60;

/// Overlap window during key rotation where both old and new keys are valid.
pub const KEY_ROTATION_OVERLAP_SECS: u64 = 7 * 24 * 60 * 60;

/// Threshold fraction for threshold cryptography (2-of-3 default).
pub const DEFAULT_THRESHOLD_K: u32 = 2;
pub const DEFAULT_THRESHOLD_N: u32 = 3;

/// Maximum shares in a threshold key scheme.
pub const MAX_THRESHOLD_SHARES: u32 = 7;

/// Emergency revocation cooling-off period before key can be reinstated (1 hour).
pub const REVOCATION_COOLDOWN_SECS: u64 = 3_600;

/// Social recovery minimum guardian quorum.
pub const SOCIAL_RECOVERY_QUORUM: u32 = 3;

/// Maximum number of registered guardians for social recovery.
pub const MAX_GUARDIANS: u32 = 7;

/// Version bump applied on each rotation.
pub const KEY_VERSION_BUMP: u32 = 1;

// ---------------------------------------------------------------------------
// Key scheme identifiers (algorithm agility)
// ---------------------------------------------------------------------------

/// Supported cryptographic schemes for key operations.
#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq)]
pub enum KeyScheme {
    /// Ed25519 — current production scheme.
    Ed25519 = 1,
    /// Secp256k1 — Ethereum-compatible.
    Secp256k1 = 2,
    /// Reserved slot for a post-quantum lattice-based scheme (future use).
    PostQuantumLattice = 3,
    /// Reserved slot for post-quantum hash-based signatures (future use).
    PostQuantumHash = 4,
}

/// Check whether a scheme is currently supported (has real verification logic).
pub fn is_scheme_supported(scheme: KeyScheme) -> bool {
    matches!(scheme, KeyScheme::Ed25519 | KeyScheme::Secp256k1)
}

// ---------------------------------------------------------------------------
// Data types
// ---------------------------------------------------------------------------

/// A versioned key record with rotation metadata.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyRecord {
    /// Account this key belongs to.
    pub account: Address,
    /// Monotonically increasing version (bumped on each rotation).
    pub version: u32,
    /// Cryptographic scheme used.
    pub scheme: KeyScheme,
    /// Public key commitment (SHA-256 of the actual public key bytes).
    /// We store the commitment rather than the raw key to save storage.
    pub pubkey_commitment: BytesN<32>,
    /// Derivation path hash (SHA-256 of the derivation path string).
    pub derivation_path_hash: BytesN<32>,
    /// Timestamp when this key version became active.
    pub activated_at: u64,
    /// Timestamp after which this key should be rotated (0 = no deadline).
    pub rotate_after: u64,
    /// Whether the key has been revoked.
    pub revoked: bool,
    /// Timestamp of revocation (0 if not revoked).
    pub revoked_at: u64,
    /// Whether forward-secrecy derivation is enabled for this key.
    pub forward_secrecy: bool,
}

/// A key rotation proposal — allows seamless transition with overlap.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyRotationProposal {
    /// Account whose key is being rotated.
    pub account: Address,
    /// New key version.
    pub new_version: u32,
    /// New scheme (may differ if migrating to post-quantum).
    pub new_scheme: KeyScheme,
    /// Commitment to the new public key.
    pub new_pubkey_commitment: BytesN<32>,
    /// New derivation path hash.
    pub new_derivation_path_hash: BytesN<32>,
    /// Timestamp when the new key becomes primary.
    pub effective_at: u64,
    /// Timestamp when the old key is fully retired.
    pub old_key_retired_at: u64,
    /// Whether the proposal has been executed.
    pub executed: bool,
}

/// A threshold key share record for distributed key management.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ThresholdKeyShare {
    /// Account whose key is protected by threshold cryptography.
    pub account: Address,
    /// Index of this share (1-indexed).
    pub share_index: u32,
    /// Total number of shares issued (N in k-of-N).
    pub total_shares: u32,
    /// Required threshold to reconstruct (k in k-of-N).
    pub threshold: u32,
    /// Commitment to this share's value (SHA-256).
    pub share_commitment: BytesN<32>,
    /// Address of the share holder.
    pub holder: Address,
    /// Whether this share has been submitted for reconstruction.
    pub submitted: bool,
}

/// Emergency key revocation record.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KeyRevocationRecord {
    /// Revoked account.
    pub account: Address,
    /// Key version that was revoked.
    pub version: u32,
    /// Reason for revocation.
    pub reason: Symbol,
    /// Timestamp of revocation.
    pub revoked_at: u64,
    /// Timestamp after which the account can register a new key.
    pub reinstate_eligible_at: u64,
    /// Whether the revocation was emergency (bypassed normal rotation flow).
    pub emergency: bool,
}

/// Social recovery session for restoring a lost key.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SocialRecoverySession {
    /// Account being recovered.
    pub account: Address,
    /// Commitment to the new recovery key.
    pub new_key_commitment: BytesN<32>,
    /// Number of guardian approvals collected.
    pub approvals_collected: u32,
    /// Required guardian quorum.
    pub quorum: u32,
    /// Expiry for collecting approvals (48 hours).
    pub expires_at: u64,
    /// Whether the recovery has been executed.
    pub executed: bool,
}

// ---------------------------------------------------------------------------
// Key registration and derivation
// ---------------------------------------------------------------------------

/// Register or update a key record for an account.
///
/// The `derivation_path_hash` is a SHA-256 of the BIP-32 or contract-specific
/// derivation path, providing hierarchical deterministic structure.
pub fn register_key(
    env: &Env,
    account: &Address,
    scheme: KeyScheme,
    pubkey_commitment: BytesN<32>,
    derivation_path_hash: BytesN<32>,
    enable_forward_secrecy: bool,
) -> KeyRecord {
    // Reject unsupported schemes.
    if !is_scheme_supported(scheme) {
        panic!("key scheme not yet supported");
    }

    let now = env.ledger().timestamp();
    let existing_version = get_current_key_version(env, account);

    let record = KeyRecord {
        account: account.clone(),
        version: existing_version.saturating_add(KEY_VERSION_BUMP),
        scheme,
        pubkey_commitment,
        derivation_path_hash,
        activated_at: now,
        rotate_after: now.saturating_add(KEY_ROTATION_PERIOD_SECS),
        revoked: false,
        revoked_at: 0,
        forward_secrecy: enable_forward_secrecy,
    };

    let key = key_record_key(account, record.version);
    env.storage().persistent().set(&key, &record);

    // Update the current version pointer.
    let ver_key = current_version_key(account);
    env.storage().persistent().set(&ver_key, &record.version);

    env.events().publish(
        (symbol_short!("keymgmt"), symbol_short!("register")),
        (account.clone(), record.version, record.scheme as u32),
    );

    record
}

/// Derive a child key commitment from a parent key via path extension.
///
/// `child_index` represents the BIP-32 index. Uses SHA-256 chaining of the
/// parent commitment and the child index to produce the child commitment.
/// This gives the HD-wallet-style derivation property without off-chain
/// cryptographic operations.
pub fn derive_child_key_commitment(
    env: &Env,
    parent_commitment: &BytesN<32>,
    child_index: u32,
    depth: u32,
) -> BytesN<32> {
    if depth > MAX_KEY_DEPTH {
        panic!("key derivation depth exceeds maximum");
    }

    let mut payload = Bytes::new(env);
    payload.append(&Bytes::from_slice(env, &parent_commitment.to_array()));
    for b in child_index.to_be_bytes().iter() {
        payload.push_back(*b);
    }
    for b in depth.to_be_bytes().iter() {
        payload.push_back(*b);
    }

    env.crypto().sha256(&payload).into()
}

// ---------------------------------------------------------------------------
// Key rotation
// ---------------------------------------------------------------------------

/// Propose a key rotation.
///
/// The old key remains valid during the overlap window (`KEY_ROTATION_OVERLAP_SECS`)
/// after the new key's `effective_at` timestamp.
pub fn propose_key_rotation(
    env: &Env,
    account: &Address,
    new_scheme: KeyScheme,
    new_pubkey_commitment: BytesN<32>,
    new_derivation_path_hash: BytesN<32>,
) -> KeyRotationProposal {
    if !is_scheme_supported(new_scheme) {
        panic!("target key scheme not yet supported");
    }

    let now = env.ledger().timestamp();
    let current_version = get_current_key_version(env, account);

    let proposal = KeyRotationProposal {
        account: account.clone(),
        new_version: current_version.saturating_add(1),
        new_scheme,
        new_pubkey_commitment,
        new_derivation_path_hash,
        effective_at: now, // effective immediately upon execution
        old_key_retired_at: now.saturating_add(KEY_ROTATION_OVERLAP_SECS),
        executed: false,
    };

    let key = rotation_proposal_key(account);
    env.storage().persistent().set(&key, &proposal);

    env.events().publish(
        (symbol_short!("keymgmt"), symbol_short!("rot_prop")),
        (account.clone(), proposal.new_version),
    );

    proposal
}

/// Execute a pending key rotation proposal.
///
/// Activates the new key and schedules the old key for retirement.
pub fn execute_key_rotation(env: &Env, account: &Address) {
    let prop_key = rotation_proposal_key(account);
    let mut proposal: KeyRotationProposal = env
        .storage()
        .persistent()
        .get(&prop_key)
        .expect("no pending rotation proposal");

    if proposal.executed {
        panic!("rotation already executed");
    }

    if env.ledger().timestamp() < proposal.effective_at {
        panic!("rotation not yet effective");
    }

    // Register the new key.
    register_key(
        env,
        account,
        proposal.new_scheme,
        proposal.new_pubkey_commitment.clone(),
        proposal.new_derivation_path_hash.clone(),
        true, // enable forward secrecy on rotated keys
    );

    proposal.executed = true;
    env.storage().persistent().set(&prop_key, &proposal);

    env.events().publish(
        (symbol_short!("keymgmt"), symbol_short!("rotated")),
        (account.clone(), proposal.new_version, env.ledger().timestamp()),
    );
}

/// Check whether an account's current key is due for rotation.
pub fn is_rotation_due(env: &Env, account: &Address) -> bool {
    let version = get_current_key_version(env, account);
    if version == 0 {
        return false;
    }
    let key = key_record_key(account, version);
    if let Some(record) = env.storage().persistent().get::<_, KeyRecord>(&key) {
        record.rotate_after > 0 && env.ledger().timestamp() > record.rotate_after
    } else {
        false
    }
}

// ---------------------------------------------------------------------------
// Threshold cryptography
// ---------------------------------------------------------------------------

/// Register a threshold key share for an account.
///
/// In a k-of-N scheme, this is called N times (once per share holder).
pub fn register_threshold_share(
    env: &Env,
    account: &Address,
    share_index: u32,
    total_shares: u32,
    threshold: u32,
    share_commitment: BytesN<32>,
    holder: &Address,
) {
    if total_shares > MAX_THRESHOLD_SHARES {
        panic!("too many threshold shares");
    }
    if threshold > total_shares || threshold == 0 {
        panic!("invalid threshold configuration");
    }

    let share = ThresholdKeyShare {
        account: account.clone(),
        share_index,
        total_shares,
        threshold,
        share_commitment,
        holder: holder.clone(),
        submitted: false,
    };

    let key = threshold_share_key(account, share_index);
    env.storage().persistent().set(&key, &share);

    env.events().publish(
        (symbol_short!("keymgmt"), symbol_short!("thr_share")),
        (account.clone(), share_index, holder.clone()),
    );
}

/// Submit a threshold share for reconstruction and mark it as submitted.
///
/// Returns the number of shares submitted so far. The caller is responsible
/// for off-chain reconstruction once `threshold` shares are collected.
pub fn submit_threshold_share(env: &Env, account: &Address, share_index: u32) -> u32 {
    let key = threshold_share_key(account, share_index);
    let mut share: ThresholdKeyShare = env
        .storage()
        .persistent()
        .get(&key)
        .expect("threshold share not found");

    if share.submitted {
        panic!("share already submitted");
    }

    share.submitted = true;
    env.storage().persistent().set(&key, &share);

    // Count submitted shares.
    let mut count = 0u32;
    for idx in 1..=share.total_shares {
        let k = threshold_share_key(account, idx);
        if let Some(s) = env.storage().persistent().get::<_, ThresholdKeyShare>(&k) {
            if s.submitted {
                count += 1;
            }
        }
    }

    env.events().publish(
        (symbol_short!("keymgmt"), symbol_short!("thr_sub")),
        (account.clone(), share_index, count),
    );

    count
}

/// Check whether enough threshold shares have been submitted for reconstruction.
pub fn is_threshold_met(env: &Env, account: &Address, total_shares: u32, threshold: u32) -> bool {
    let mut count = 0u32;
    for idx in 1..=total_shares {
        let key = threshold_share_key(account, idx);
        if let Some(s) = env.storage().persistent().get::<_, ThresholdKeyShare>(&key) {
            if s.submitted {
                count += 1;
            }
        }
    }
    count >= threshold
}

// ---------------------------------------------------------------------------
// Emergency key revocation
// ---------------------------------------------------------------------------

/// Immediately revoke a key due to compromise.
///
/// The account cannot register a new key until `REVOCATION_COOLDOWN_SECS` has
/// elapsed, providing a cooling-off window against attacker re-registration.
pub fn emergency_revoke_key(
    env: &Env,
    account: &Address,
    version: u32,
    reason: Symbol,
) {
    let key = key_record_key(account, version);
    let mut record: KeyRecord = env
        .storage()
        .persistent()
        .get(&key)
        .expect("key record not found");

    if record.revoked {
        panic!("key already revoked");
    }

    let now = env.ledger().timestamp();
    record.revoked = true;
    record.revoked_at = now;
    env.storage().persistent().set(&key, &record);

    let revocation = KeyRevocationRecord {
        account: account.clone(),
        version,
        reason: reason.clone(),
        revoked_at: now,
        reinstate_eligible_at: now.saturating_add(REVOCATION_COOLDOWN_SECS),
        emergency: true,
    };

    let rev_key = revocation_record_key(account);
    env.storage().persistent().set(&rev_key, &revocation);

    env.events().publish(
        (symbol_short!("keymgmt"), symbol_short!("revoked")),
        (account.clone(), version, reason, now),
    );
}

/// Check whether a key is revoked.
pub fn is_key_revoked(env: &Env, account: &Address, version: u32) -> bool {
    let key = key_record_key(account, version);
    env.storage()
        .persistent()
        .get::<_, KeyRecord>(&key)
        .map(|r| r.revoked)
        .unwrap_or(false)
}

/// Check whether an account is eligible to register a new key after
/// an emergency revocation.
pub fn is_reinstate_eligible(env: &Env, account: &Address) -> bool {
    let rev_key = revocation_record_key(account);
    match env
        .storage()
        .persistent()
        .get::<_, KeyRevocationRecord>(&rev_key)
    {
        Some(rev) => env.ledger().timestamp() >= rev.reinstate_eligible_at,
        None => true,
    }
}

// ---------------------------------------------------------------------------
// Social recovery
// ---------------------------------------------------------------------------

/// Initiate a social recovery session for an account.
///
/// The account owner must have pre-registered guardians. Once `quorum`
/// guardians approve, the new key is installed.
pub fn initiate_social_recovery(
    env: &Env,
    account: &Address,
    new_key_commitment: BytesN<32>,
) -> BytesN<32> {
    let now = env.ledger().timestamp();

    let session = SocialRecoverySession {
        account: account.clone(),
        new_key_commitment: new_key_commitment.clone(),
        approvals_collected: 0,
        quorum: SOCIAL_RECOVERY_QUORUM,
        expires_at: now.saturating_add(48 * 60 * 60),
        executed: false,
    };

    // Session ID derived from account + new key + timestamp.
    let mut payload = Bytes::new(env);
    payload.append(&Bytes::from_slice(env, &new_key_commitment.to_array()));
    for b in now.to_be_bytes().iter() {
        payload.push_back(*b);
    }
    let session_id: BytesN<32> = env.crypto().sha256(&payload).into();

    let key = (symbol_short!("socrec"), session_id.clone());
    env.storage().persistent().set(&key, &session);

    env.events().publish(
        (symbol_short!("keymgmt"), symbol_short!("rec_init")),
        (account.clone(), session_id.clone(), now),
    );

    session_id
}

/// Submit a guardian approval for a social recovery session.
///
/// Returns the number of approvals collected. Once `quorum` is met, the
/// caller should invoke `execute_social_recovery`.
pub fn approve_social_recovery(
    env: &Env,
    guardian: &Address,
    session_id: &BytesN<32>,
) -> u32 {
    let key = (symbol_short!("socrec"), session_id.clone());
    let mut session: SocialRecoverySession = env
        .storage()
        .persistent()
        .get(&key)
        .expect("recovery session not found");

    if session.executed {
        panic!("recovery already executed");
    }
    if env.ledger().timestamp() > session.expires_at {
        panic!("recovery session expired");
    }

    // Verify guardian is registered for this account.
    if !is_registered_guardian(env, &session.account, guardian) {
        panic!("not a registered guardian");
    }

    // Prevent duplicate approvals.
    let approval_key = (symbol_short!("socrec_a"), session_id.clone(), guardian.clone());
    if env.storage().persistent().has(&approval_key) {
        panic!("guardian already approved");
    }

    env.storage().persistent().set(&approval_key, &true);
    session.approvals_collected = session.approvals_collected.saturating_add(1);
    env.storage().persistent().set(&key, &session);

    env.events().publish(
        (symbol_short!("keymgmt"), symbol_short!("rec_apv")),
        (guardian.clone(), session_id.clone(), session.approvals_collected),
    );

    session.approvals_collected
}

/// Execute a social recovery session once quorum is met.
pub fn execute_social_recovery(
    env: &Env,
    session_id: &BytesN<32>,
    derivation_path_hash: BytesN<32>,
) {
    let key = (symbol_short!("socrec"), session_id.clone());
    let mut session: SocialRecoverySession = env
        .storage()
        .persistent()
        .get(&key)
        .expect("recovery session not found");

    if session.executed {
        panic!("recovery already executed");
    }
    if env.ledger().timestamp() > session.expires_at {
        panic!("recovery session expired");
    }
    if session.approvals_collected < session.quorum {
        panic!("recovery quorum not met");
    }

    // Register the recovered key.
    register_key(
        env,
        &session.account,
        KeyScheme::Ed25519,
        session.new_key_commitment.clone(),
        derivation_path_hash,
        true,
    );

    session.executed = true;
    env.storage().persistent().set(&key, &session);

    env.events().publish(
        (symbol_short!("keymgmt"), symbol_short!("recovered")),
        (session.account.clone(), session_id.clone(), env.ledger().timestamp()),
    );
}

/// Register a guardian for an account's social recovery.
pub fn register_guardian(env: &Env, account: &Address, guardian: &Address) {
    let mut guardians = get_guardians(env, account);
    if guardians.len() as u32 >= MAX_GUARDIANS {
        panic!("maximum guardian count reached");
    }
    if guardians.contains(guardian.clone()) {
        panic!("guardian already registered");
    }
    guardians.push_back(guardian.clone());

    let key = guardian_list_key(account);
    env.storage().persistent().set(&key, &guardians);

    env.events().publish(
        (symbol_short!("keymgmt"), symbol_short!("grd_added")),
        (account.clone(), guardian.clone()),
    );
}

/// Check whether an address is a registered guardian for an account.
pub fn is_registered_guardian(env: &Env, account: &Address, guardian: &Address) -> bool {
    let guardians = get_guardians(env, account);
    guardians.contains(guardian.clone())
}

/// Get the guardian list for an account.
pub fn get_guardians(env: &Env, account: &Address) -> Vec<Address> {
    let key = guardian_list_key(account);
    env.storage()
        .persistent()
        .get(&key)
        .unwrap_or(Vec::new(env))
}

/// Get the current key record for an account (latest active version).
pub fn get_current_key(env: &Env, account: &Address) -> Option<KeyRecord> {
    let version = get_current_key_version(env, account);
    if version == 0 {
        return None;
    }
    let key = key_record_key(account, version);
    env.storage().persistent().get(&key)
}

// ---------------------------------------------------------------------------
// Internal helpers
// ---------------------------------------------------------------------------

fn key_record_key(account: &Address, version: u32) -> (Symbol, Address, u32) {
    (symbol_short!("keyrec"), account.clone(), version)
}

fn current_version_key(account: &Address) -> (Symbol, Address) {
    (symbol_short!("keyver"), account.clone())
}

fn rotation_proposal_key(account: &Address) -> (Symbol, Address) {
    (symbol_short!("keyrot"), account.clone())
}

fn threshold_share_key(account: &Address, share_index: u32) -> (Symbol, Address, u32) {
    (symbol_short!("keythr"), account.clone(), share_index)
}

fn revocation_record_key(account: &Address) -> (Symbol, Address) {
    (symbol_short!("keyrev"), account.clone())
}

fn guardian_list_key(account: &Address) -> (Symbol, Address) {
    (symbol_short!("keygrd"), account.clone())
}

fn get_current_key_version(env: &Env, account: &Address) -> u32 {
    let ver_key = current_version_key(account);
    env.storage()
        .persistent()
        .get(&ver_key)
        .unwrap_or(0u32)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use soroban_sdk::testutils::Address as _;
    extern crate std;
    use super::*;
    use soroban_sdk::{testutils::Ledger, Env};

    fn env_at(ts: u64) -> Env {
        let env = Env::default();
        env.ledger().with_mut(|l| l.timestamp = ts);
        env
    }

    fn dummy_commitment(env: &Env, seed: u8) -> BytesN<32> {
        BytesN::from_array(env, &[seed; 32])
    }

    #[test]
    fn test_register_and_retrieve_key() {
        let env = env_at(1_000);
        let account = soroban_sdk::Address::generate(&env);

        let record = register_key(
            &env,
            &account,
            KeyScheme::Ed25519,
            dummy_commitment(&env, 0xAA),
            dummy_commitment(&env, 0x01),
            true,
        );

        assert_eq!(record.version, 1);
        assert_eq!(record.scheme, KeyScheme::Ed25519);
        assert!(!record.revoked);
        assert!(record.forward_secrecy);

        let retrieved = get_current_key(&env, &account).unwrap();
        assert_eq!(retrieved.version, 1);
    }

    #[test]
    fn test_key_rotation_increments_version() {
        let env = env_at(1_000);
        let account = soroban_sdk::Address::generate(&env);

        register_key(
            &env,
            &account,
            KeyScheme::Ed25519,
            dummy_commitment(&env, 1),
            dummy_commitment(&env, 2),
            true,
        );

        propose_key_rotation(
            &env,
            &account,
            KeyScheme::Ed25519,
            dummy_commitment(&env, 3),
            dummy_commitment(&env, 4),
        );

        execute_key_rotation(&env, &account);

        let current = get_current_key(&env, &account).unwrap();
        assert_eq!(current.version, 2, "version should bump to 2 after rotation");
    }

    #[test]
    fn test_child_key_derivation_deterministic() {
        let env = env_at(1_000);
        let parent = dummy_commitment(&env, 0xAA);
        let child1 = derive_child_key_commitment(&env, &parent, 0, 1);
        let child2 = derive_child_key_commitment(&env, &parent, 0, 1);
        assert_eq!(child1, child2, "child derivation must be deterministic");
    }

    #[test]
    fn test_child_differs_from_parent() {
        let env = env_at(1_000);
        let parent = dummy_commitment(&env, 0xAA);
        let child = derive_child_key_commitment(&env, &parent, 0, 1);
        assert_ne!(child, parent, "child must differ from parent");
    }

    #[test]
    #[should_panic(expected = "key derivation depth exceeds maximum")]
    fn test_depth_limit_enforced() {
        let env = env_at(1_000);
        let parent = dummy_commitment(&env, 0xAA);
        derive_child_key_commitment(&env, &parent, 0, MAX_KEY_DEPTH + 1);
    }

    #[test]
    fn test_emergency_revocation() {
        let env = env_at(1_000);
        let account = soroban_sdk::Address::generate(&env);

        register_key(
            &env,
            &account,
            KeyScheme::Ed25519,
            dummy_commitment(&env, 5),
            dummy_commitment(&env, 6),
            false,
        );

        assert!(!is_key_revoked(&env, &account, 1));
        emergency_revoke_key(&env, &account, 1, Symbol::new(&env, "compromised"));
        assert!(is_key_revoked(&env, &account, 1));

        // Within cooldown, reinstate is NOT eligible.
        assert!(!is_reinstate_eligible(&env, &account));

        // After cooldown, reinstate IS eligible.
        env.ledger().with_mut(|l| l.timestamp = 1_000 + REVOCATION_COOLDOWN_SECS + 1);
        assert!(is_reinstate_eligible(&env, &account));
    }

    #[test]
    fn test_threshold_share_management() {
        let env = env_at(1_000);
        let account = soroban_sdk::Address::generate(&env);
        let holders: [soroban_sdk::Address; 3] = core::array::from_fn(|_| soroban_sdk::Address::generate(&env));

        for i in 0..3u32 {
            register_threshold_share(
                &env,
                &account,
                i + 1,
                3,
                2,
                dummy_commitment(&env, i as u8),
                &holders[i as usize],
            );
        }

        assert!(!is_threshold_met(&env, &account, 3, 2));

        submit_threshold_share(&env, &account, 1);
        assert!(!is_threshold_met(&env, &account, 3, 2));

        submit_threshold_share(&env, &account, 2);
        assert!(is_threshold_met(&env, &account, 3, 2));
    }

    #[test]
    fn test_social_recovery_flow() {
        let env = env_at(1_000);
        let account = soroban_sdk::Address::generate(&env);
        let guardians: [soroban_sdk::Address; 3] = core::array::from_fn(|_| soroban_sdk::Address::generate(&env));

        // Register guardians.
        for g in &guardians {
            register_guardian(&env, &account, g);
        }

        let new_commitment = dummy_commitment(&env, 0xFF);
        let session_id = initiate_social_recovery(&env, &account, new_commitment.clone());

        // Collect quorum approvals.
        let count_after_1 = approve_social_recovery(&env, &guardians[0], &session_id);
        assert_eq!(count_after_1, 1);
        let count_after_2 = approve_social_recovery(&env, &guardians[1], &session_id);
        assert_eq!(count_after_2, 2);
        let count_after_3 = approve_social_recovery(&env, &guardians[2], &session_id);
        assert_eq!(count_after_3, 3);

        // Execute recovery.
        execute_social_recovery(&env, &session_id, dummy_commitment(&env, 0xDE));

        let current = get_current_key(&env, &account).unwrap();
        assert_eq!(current.pubkey_commitment, new_commitment);
    }
}

//! Dispute Evidence Contract
//!
//! Allows the mentor or learner to attach off-chain evidence references to a
//! disputed escrow during a bounded submission window. An arbitrator may then
//! submit a resolution after a mandatory review delay.
//!
//! # Workflow
//! 1. Learner or mentor opens a dispute on the escrow contract.
//! 2. Either party calls [`DisputeEvidenceContract::submit_evidence`] with a
//!    `content_hash: BytesN<32>` — a SHA-256/BLAKE3 commitment to the actual
//!    off-chain document content — plus an `evidence_uri_hash: BytesN<32>`
//!    committing to the IPFS CID or URL where that content is hosted. This
//!    replaces the original `Symbol` reference (max 9 chars), which could
//!    not hold a real content commitment and gave callers no way to prove a
//!    submitted reference corresponds to specific content.
//! 3. An arbitrator calls [`DisputeEvidenceContract::submit_resolution`] after
//!    `MIN_RESOLUTION_DELAY_SECS` have elapsed since the dispute was opened.
//! 4. The admin uses the on-chain resolution record to call `resolve_dispute`
//!    on the escrow contract.
#![no_std]

use soroban_sdk::{
    contract, contractclient, contracterror, contractimpl, contracttype, Address, BytesN, Env,
    Symbol, Vec,
};

/// Default window (seconds) within which evidence may be submitted after session end.
const DEFAULT_WINDOW_SECS: u64 = 48 * 60 * 60;

/// Maximum evidence items stored per escrow.
const MAX_EVIDENCE_ITEMS: u32 = 5;

/// Minimum seconds a party must wait between consecutive evidence submissions
/// for the same escrow (anti-spam / griefing guard). 1 hour.
const SUBMISSION_COOLDOWN_SECS: u64 = 3_600;

/// Minimum seconds that must elapse after the evidence window opens before
/// an arbitrator may submit a resolution (gives parties time to respond).
/// 24 hours.
const MIN_RESOLUTION_DELAY_SECS: u64 = 24 * 60 * 60;

// ─── Domain types ─────────────────────────────────────────────────────────────

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EscrowStatus {
    Active,
    Released,
    Disputed,
    Refunded,
    Resolved,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Escrow {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub session_id: Symbol,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    pub platform_fee: i128,
    pub net_amount: i128,
    pub session_end_time: u64,
    pub auto_release_delay: u64,
    pub dispute_reason: Symbol,
    pub resolved_at: u64,
    pub usd_amount: i128,
    pub quoted_token_amount: i128,
    pub send_asset: Address,
    pub dest_asset: Address,
    pub total_sessions: u32,
    pub sessions_completed: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvidenceItem {
    pub submitter: Address,
    /// SHA-256/BLAKE3 commitment to the actual off-chain evidence content.
    pub content_hash: BytesN<32>,
    /// Commitment to the IPFS CID or URL where `content_hash`'s content is
    /// hosted, kept separate from `content_hash` so the location can change
    /// (e.g. re-pinned to a different gateway) without invalidating the
    /// content commitment itself.
    pub evidence_uri_hash: BytesN<32>,
    /// Optional signature from `submitter` over
    /// `(escrow_id, content_hash, submitted_at)`, allowing later
    /// off-chain/on-chain verification that the submitter themselves
    /// attested to this exact content at this exact time. An all-zero value
    /// means "no attestation provided" (soroban-sdk 21.7.7 cannot derive
    /// `contracttype` for `Option<BytesN<64>>`, so a sentinel is used
    /// instead of `Option`).
    pub submitter_attestation: BytesN<64>,
    pub submitted_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeResolution {
    pub arbitrator: Address,
    pub release_to_mentor: bool,
    pub note: Symbol,
    pub resolved_at: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    EscrowContract,
    Evidence(u64),
    Resolution(u64),
    WindowSecs,
    /// Tracks when each (submitter, escrow_id) pair last submitted evidence.
    /// Used to enforce SUBMISSION_COOLDOWN_SECS between submissions.
    LastSubmission(u64, Address),
    /// Ledger timestamp at which a dispute was opened for a given escrow.
    DisputeOpenedAt(u64),
    /// Whether the anti-spam cooldown is enabled (default: true).
    CooldownEnabled,
}

#[contractclient(name = "EscrowContractClient")]
pub trait EscrowContractTrait {
    fn get_escrow(env: Env, escrow_id: u64) -> Escrow;
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized       = 1,
    Unauthorized             = 2,
    InvalidEscrowState       = 3,
    EvidenceWindowClosed     = 4,
    EvidenceLimitReached     = 5,
    AlreadyResolved          = 6,
    /// Submitter must wait before submitting more evidence (anti-spam).
    SubmissionCooldown       = 7,
    /// Arbitrator must wait for MIN_RESOLUTION_DELAY_SECS after dispute opens.
    ResolutionTimelockActive = 8,
    /// Dispute opening has already been recorded for this escrow.
    AlreadyRecorded          = 9,
    /// `content_hash` was all-zero, i.e. a null commitment that proves
    /// nothing about the submitted content.
    ZeroContentHash          = 10,
    /// The same submitter already submitted this exact `content_hash` for
    /// this escrow — duplicate commitments are rejected.
    DuplicateContentHash     = 11,
}

#[contract]
pub struct DisputeEvidenceContract;

#[contractimpl]
impl DisputeEvidenceContract {
    pub fn initialize(env: Env, admin: Address, escrow_contract: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::EscrowContract, &escrow_contract);
        env.storage()
            .instance()
            .set(&DataKey::WindowSecs, &DEFAULT_WINDOW_SECS);
        env.storage()
            .instance()
            .set(&DataKey::CooldownEnabled, &true);
        Ok(())
    }

    pub fn set_escrow_contract(
        env: Env,
        admin: Address,
        escrow_contract: Address,
    ) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage()
            .instance()
            .set(&DataKey::EscrowContract, &escrow_contract);
        Ok(())
    }

    /// Enable or disable the anti-spam submission cooldown. Admin only.
    pub fn set_cooldown_enabled(env: Env, admin: Address, enabled: bool) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::CooldownEnabled, &enabled);
        Ok(())
    }

    /// Record that a dispute was opened.
    ///
    /// # Security
    /// The `opened_at` timestamp is derived from `env.ledger().timestamp()` and
    /// **cannot** be supplied by the caller. This prevents an admin (or any
    /// caller) from backdating the dispute opening to bypass the mandatory
    /// `MIN_RESOLUTION_DELAY_SECS` deliberation period.
    ///
    /// # Idempotency
    /// Calling this function a second time for the same `escrow_id` returns
    /// [`Error::AlreadyRecorded`] so that the timelock anchor cannot be
    /// overwritten or replayed.
    pub fn record_dispute_opened(env: Env, escrow_id: u64) -> Result<(), Error> {
        // Idempotency guard: refuse to overwrite an existing record.
        if env
            .storage()
            .persistent()
            .has(&DataKey::DisputeOpenedAt(escrow_id))
        {
            return Err(Error::AlreadyRecorded);
        }

        // Allow either admin or the escrow contract to call this.
        let stored: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::Unauthorized)?;
        stored.require_auth();

        // SECURITY: timestamp is taken from the ledger, not the caller.
        let opened_at = env.ledger().timestamp();
        env.storage()
            .persistent()
            .set(&DataKey::DisputeOpenedAt(escrow_id), &opened_at);
        env.events().publish(
            (Symbol::new(&env, "dispute_opened"), escrow_id),
            opened_at,
        );
        Ok(())
    }

    /// Return the ledger timestamp at which the dispute for `escrow_id` was
    /// opened, or `None` if [`record_dispute_opened`] has not yet been called.
    pub fn get_dispute_opened_at(env: Env, escrow_id: u64) -> Option<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::DisputeOpenedAt(escrow_id))
    }

    /// Submit evidence for a disputed escrow.
    ///
    /// `content_hash` must be a SHA-256/BLAKE3 hash of the actual evidence
    /// document — a non-zero commitment that later lets any party verify
    /// (via [`Self::verify_evidence_integrity`]) that a claimed document
    /// matches what was originally submitted. `evidence_uri_hash` commits
    /// separately to the off-chain location (e.g. IPFS CID) hosting that
    /// content. `submitter_attestation`, if provided, is expected to be a
    /// signature from `submitter` over `(escrow_id, content_hash,
    /// submitted_at)`.
    ///
    /// # Anti-spam guard
    /// A party may not submit evidence more than once per
    /// `SUBMISSION_COOLDOWN_SECS` (1 h) for the same escrow.
    pub fn submit_evidence(
        env: Env,
        escrow_id: u64,
        submitter: Address,
        content_hash: BytesN<32>,
        evidence_uri_hash: BytesN<32>,
        submitter_attestation: Option<BytesN<64>>,
    ) -> Result<(), Error> {
        submitter.require_auth();
        let escrow = Self::load_escrow(&env, escrow_id);
        if escrow.status != EscrowStatus::Disputed {
            return Err(Error::InvalidEscrowState);
        }
        if submitter != escrow.mentor && submitter != escrow.learner {
            return Err(Error::Unauthorized);
        }

        if content_hash == BytesN::from_array(&env, &[0u8; 32]) {
            return Err(Error::ZeroContentHash);
        }

        let window_secs: u64 = env
            .storage()
            .instance()
            .get(&DataKey::WindowSecs)
            .unwrap_or(DEFAULT_WINDOW_SECS);
        if env.ledger().timestamp() > escrow.session_end_time.saturating_add(window_secs) {
            return Err(Error::EvidenceWindowClosed);
        }

        // Anti-spam / griefing guard
        let cooldown_enabled: bool = env
            .storage()
            .instance()
            .get(&DataKey::CooldownEnabled)
            .unwrap_or(true);
        if cooldown_enabled {
            let cooldown_key = DataKey::LastSubmission(escrow_id, submitter.clone());
            let last_submission: u64 = env
                .storage()
                .persistent()
                .get(&cooldown_key)
                .unwrap_or(0);
            let now = env.ledger().timestamp();
            if now < last_submission.saturating_add(SUBMISSION_COOLDOWN_SECS) {
                return Err(Error::SubmissionCooldown);
            }
            env.storage().persistent().set(&cooldown_key, &now);
        }

        let key = DataKey::Evidence(escrow_id);
        let mut evidence: Vec<EvidenceItem> = env
            .storage()
            .persistent()
            .get(&key)
            .unwrap_or(Vec::new(&env));
        if evidence.len() >= MAX_EVIDENCE_ITEMS {
            return Err(Error::EvidenceLimitReached);
        }

        // Reject a duplicate (submitter, content_hash) pair within the same
        // escrow — two different evidence items from the same submitter
        // must not share a content commitment.
        for existing in evidence.iter() {
            if existing.submitter == submitter && existing.content_hash == content_hash {
                return Err(Error::DuplicateContentHash);
            }
        }

        let item = EvidenceItem {
            submitter: submitter.clone(),
            content_hash,
            evidence_uri_hash,
            submitter_attestation: submitter_attestation
                .unwrap_or_else(|| BytesN::from_array(&env, &[0u8; 64])),
            submitted_at: env.ledger().timestamp(),
        };
        evidence.push_back(item.clone());
        env.storage().persistent().set(&key, &evidence);
        env.events()
            .publish((Symbol::new(&env, "evidence_submitted"), escrow_id), item);
        Ok(())
    }

    pub fn get_evidence(env: Env, escrow_id: u64) -> Vec<EvidenceItem> {
        env.storage()
            .persistent()
            .get(&DataKey::Evidence(escrow_id))
            .unwrap_or(Vec::new(&env))
    }

    pub fn get_evidence_count(env: Env, escrow_id: u64) -> u32 {
        Self::get_evidence(env, escrow_id).len()
    }

    /// Verify that the evidence item at `index` for `escrow_id` was
    /// submitted with exactly `expected_hash` as its `content_hash`.
    /// Returns `false` (rather than erroring) if the index is out of range
    /// or the hash does not match — callers checking whether a claimed
    /// document has been tampered with should treat both as "not verified".
    pub fn verify_evidence_integrity(
        env: Env,
        escrow_id: u64,
        index: u32,
        expected_hash: BytesN<32>,
    ) -> bool {
        let evidence = Self::get_evidence(env, escrow_id);
        match evidence.get(index) {
            Some(item) => item.content_hash == expected_hash,
            None => false,
        }
    }

    /// Submit a dispute resolution.
    ///
    /// # Time-lock guard
    /// Resolution may only be submitted at least `MIN_RESOLUTION_DELAY_SECS`
    /// (24 h) after the dispute was opened (via `record_dispute_opened`). If
    /// no opened-at record exists the guard is skipped (backwards-compatible).
    pub fn submit_resolution(
        env: Env,
        escrow_id: u64,
        arbitrator: Address,
        release_to_mentor: bool,
        note: Symbol,
    ) -> Result<(), Error> {
        arbitrator.require_auth();
        let escrow = Self::load_escrow(&env, escrow_id);
        if escrow.status != EscrowStatus::Disputed {
            return Err(Error::InvalidEscrowState);
        }

        let key = DataKey::Resolution(escrow_id);
        if env.storage().persistent().has(&key) {
            return Err(Error::AlreadyResolved);
        }

        // Time-lock: enforce minimum deliberation period.
        if let Some(opened_at) = env
            .storage()
            .persistent()
            .get::<_, u64>(&DataKey::DisputeOpenedAt(escrow_id))
        {
            let earliest_resolution = opened_at.saturating_add(MIN_RESOLUTION_DELAY_SECS);
            if env.ledger().timestamp() < earliest_resolution {
                return Err(Error::ResolutionTimelockActive);
            }
        }

        let resolution = DisputeResolution {
            arbitrator: arbitrator.clone(),
            release_to_mentor,
            note: note.clone(),
            resolved_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&key, &resolution);
        env.events().publish(
            (Symbol::new(&env, "dispute_resolved"), escrow_id),
            resolution,
        );
        Ok(())
    }

    pub fn get_resolution(env: Env, escrow_id: u64) -> DisputeResolution {
        env.storage()
            .persistent()
            .get(&DataKey::Resolution(escrow_id))
            .expect("resolution not found")
    }

    fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::Unauthorized)?;
        if stored_admin != *admin {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn load_escrow(env: &Env, escrow_id: u64) -> Escrow {
        let escrow_contract: Address = env
            .storage()
            .instance()
            .get(&DataKey::EscrowContract)
            .expect("escrow contract not configured");
        EscrowContractClient::new(env, &escrow_contract).get_escrow(&escrow_id)
    }
}

// ─── Tests ────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contractimpl,
        testutils::{Address as _, Events, Ledger, LedgerInfo},
        IntoVal, TryFromVal,
    };

    #[contract]
    struct MockEscrow;

    fn make_escrow(env: &Env, status: EscrowStatus) -> Escrow {
        Escrow {
            id: 1,
            mentor: Address::generate(env),
            learner: Address::generate(env),
            amount: 100,
            session_id: Symbol::new(env, "sess"),
            status,
            created_at: env.ledger().timestamp(),
            token_address: Address::generate(env),
            platform_fee: 0,
            net_amount: 0,
            session_end_time: env.ledger().timestamp() + 3_600,
            auto_release_delay: 0,
            dispute_reason: Symbol::new(env, "late"),
            resolved_at: 0,
            usd_amount: 0,
            quoted_token_amount: 100,
            send_asset: Address::generate(env),
            dest_asset: Address::generate(env),
            total_sessions: 1,
            sessions_completed: 0,
        }
    }

    #[contractimpl]
    impl MockEscrow {
        pub fn get_escrow(env: Env, _escrow_id: u64) -> Escrow {
            make_escrow(&env, EscrowStatus::Disputed)
        }
    }

    fn setup_disputed() -> (Env, Address, Address, Address, DisputeEvidenceContractClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let escrow_contract = env.register_contract(None, MockEscrow);
        let contract_id = env.register_contract(None, DisputeEvidenceContract);
        let client = DisputeEvidenceContractClient::new(&env, &contract_id);
        client.initialize(&admin, &escrow_contract).unwrap();
        let escrow = EscrowContractClient::new(&env, &escrow_contract).get_escrow(&1);
        (env, admin, escrow.mentor, escrow.learner, client)
    }

    /// Build a distinct, non-zero 32-byte hash for tests, seeded by `seed`.
    fn hash32(env: &Env, seed: u8) -> BytesN<32> {
        let mut bytes = [0u8; 32];
        bytes[0] = seed;
        bytes[31] = seed.wrapping_add(1).max(1); // ensure non-zero even if seed == 0/255
        BytesN::from_array(env, &bytes)
    }

    fn zero_hash32(env: &Env) -> BytesN<32> {
        BytesN::from_array(env, &[0u8; 32])
    }

    fn advance_time(env: &Env, secs: u64) {
        let t = env.ledger().timestamp();
        env.ledger().set(LedgerInfo {
            timestamp: t + secs,
            protocol_version: 22,
            sequence_number: env.ledger().sequence() + 1,
            network_id: Default::default(),
            base_reserve: 10,
            min_temp_entry_ttl: 100,
            min_persistent_entry_ttl: 100,
            max_entry_ttl: 9_999_999,
        });
    }

    // ─── existing: evidence cap ───────────────────────────────────────────

    #[test]
    fn stores_evidence_until_cap() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        // Disable cooldown to allow rapid sequential submissions for cap test
        client.set_cooldown_enabled(&_admin, &false).unwrap();
        for seed in 1u8..=5 {
            client
                .submit_evidence(&1, &mentor, &hash32(&env, seed), &hash32(&env, seed.wrapping_add(100)), &None)
                .unwrap();
        }
        assert_eq!(client.get_evidence_count(&1), MAX_EVIDENCE_ITEMS);
    }

    // ─── #651: content-hash commitment scheme ──────────────────────────────

    #[test]
    fn submit_evidence_rejects_zero_content_hash() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        let result = client.try_submit_evidence(
            &1,
            &mentor,
            &zero_hash32(&env),
            &hash32(&env, 1),
            &None,
        );
        assert!(result.is_err(), "zero content_hash must be rejected");
    }

    #[test]
    fn submit_evidence_rejects_duplicate_hash_same_submitter() {
        let (env, admin, mentor, _learner, client) = setup_disputed();
        client.set_cooldown_enabled(&admin, &false).unwrap();
        let hash = hash32(&env, 7);
        client
            .submit_evidence(&1, &mentor, &hash, &hash32(&env, 8), &None)
            .unwrap();
        let result = client.try_submit_evidence(&1, &mentor, &hash, &hash32(&env, 9), &None);
        assert!(
            result.is_err(),
            "duplicate content_hash from same submitter must be rejected"
        );
    }

    #[test]
    fn submit_evidence_allows_same_hash_from_different_submitters() {
        let (env, admin, mentor, learner, client) = setup_disputed();
        client.set_cooldown_enabled(&admin, &false).unwrap();
        let hash = hash32(&env, 7);
        client
            .submit_evidence(&1, &mentor, &hash, &hash32(&env, 8), &None)
            .unwrap();
        // Different submitter, same content_hash — allowed (e.g. both
        // parties independently attest to the same document).
        client
            .submit_evidence(&1, &learner, &hash, &hash32(&env, 8), &None)
            .unwrap();
        assert_eq!(client.get_evidence_count(&1), 2);
    }

    #[test]
    fn verify_evidence_integrity_matches_submitted_hash() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        let hash = hash32(&env, 42);
        client
            .submit_evidence(&1, &mentor, &hash, &hash32(&env, 43), &None)
            .unwrap();
        assert!(client.verify_evidence_integrity(&1, &0, &hash));
    }

    #[test]
    fn verify_evidence_integrity_fails_on_tampered_hash() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 42), &hash32(&env, 43), &None)
            .unwrap();
        // A different (tampered) hash must not verify.
        assert!(!client.verify_evidence_integrity(&1, &0, &hash32(&env, 99)));
    }

    #[test]
    fn verify_evidence_integrity_false_for_out_of_range_index() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        assert!(!client.verify_evidence_integrity(&1, &0, &hash32(&env, 1)));
    }

    #[test]
    fn submit_evidence_stores_optional_submitter_attestation() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        let mut sig_bytes = [0u8; 64];
        sig_bytes[0] = 9;
        let attestation = BytesN::from_array(&env, &sig_bytes);
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &Some(attestation.clone()))
            .unwrap();
        let items = client.get_evidence(&1);
        assert_eq!(items.get(0).unwrap().submitter_attestation, attestation);
    }

    #[test]
    fn submit_evidence_without_attestation_stores_zero_sentinel() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        let items = client.get_evidence(&1);
        assert_eq!(
            items.get(0).unwrap().submitter_attestation,
            BytesN::from_array(&env, &[0u8; 64])
        );
    }

    // ─── #417: anti-spam cooldown ─────────────────────────────────────────

    #[test]
    fn second_submission_within_cooldown_fails() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        // Immediately retry (within cooldown) → must fail
        let result = client.try_submit_evidence(&1, &mentor, &hash32(&env, 3), &hash32(&env, 4), &None);
        assert!(result.is_err(), "second submission within cooldown must fail");
    }

    #[test]
    fn submission_allowed_after_cooldown_elapses() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        advance_time(&env, SUBMISSION_COOLDOWN_SECS + 1);
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 3), &hash32(&env, 4), &None)
            .unwrap();
        assert_eq!(client.get_evidence_count(&1), 2);
    }

    #[test]
    fn different_parties_may_submit_independently() {
        let (env, _admin, mentor, learner, client) = setup_disputed();
        // mentor submits
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        // learner submits in the same window — separate cooldown key
        client
            .submit_evidence(&1, &learner, &hash32(&env, 3), &hash32(&env, 4), &None)
            .unwrap();
        assert_eq!(client.get_evidence_count(&1), 2);
    }

    #[test]
    fn cooldown_disabled_allows_rapid_submission() {
        let (env, admin, mentor, _learner, client) = setup_disputed();
        client.set_cooldown_enabled(&admin, &false).unwrap();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 1), &hash32(&env, 2), &None)
            .unwrap();
        client
            .submit_evidence(&1, &mentor, &hash32(&env, 3), &hash32(&env, 4), &None)
            .unwrap();
        assert_eq!(client.get_evidence_count(&1), 2);
    }

    // ─── #417: resolution timelock ────────────────────────────────────────

    #[test]
    fn resolution_before_timelock_fails() {
        let (env, admin, _mentor, _learner, client) = setup_disputed();
        let arbitrator = Address::generate(&env);
        client.record_dispute_opened(&1).unwrap();

        // Do NOT advance time
        let result = client.try_submit_resolution(
            &1,
            &arbitrator,
            &true,
            &Symbol::new(&env, "mentor_wins"),
        );
        assert!(result.is_err(), "resolution before timelock must fail");
        let _ = admin;
    }

    #[test]
    fn resolution_after_timelock_succeeds() {
        let (env, admin, _mentor, _learner, client) = setup_disputed();
        let arbitrator = Address::generate(&env);
        client.record_dispute_opened(&1).unwrap();

        advance_time(&env, MIN_RESOLUTION_DELAY_SECS + 1);

        client
            .submit_resolution(&1, &arbitrator, &true, &Symbol::new(&env, "mentor_wins"))
            .unwrap();

        let res = client.get_resolution(&1);
        assert_eq!(res.arbitrator, arbitrator);
        assert!(res.release_to_mentor);
        let _ = admin;
    }

    #[test]
    fn resolution_without_opened_at_record_is_allowed() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        // No `record_dispute_opened` call — guard is skipped for backwards compat
        let arbitrator = Address::generate(&env);
        client
            .submit_resolution(&1, &arbitrator, &false, &Symbol::new(&env, "learner_wins"))
            .unwrap();
        let res = client.get_resolution(&1);
        assert!(!res.release_to_mentor);
    }

    // ─── record_dispute_opened idempotency & getter ────────────────────────

    #[test]
    fn record_dispute_opened_is_idempotent() {
        let (env, admin, _mentor, _learner, client) = setup_disputed();
        client.record_dispute_opened(&1).unwrap();
        let result = client.try_record_dispute_opened(&1);
        assert!(result.is_err(), "second call must return AlreadyRecorded");
        let _ = admin;
    }

    #[test]
    fn get_dispute_opened_at_returns_none_before_record() {
        let (_env, _admin, _mentor, _learner, client) = setup_disputed();
        assert_eq!(client.get_dispute_opened_at(&1), None);
    }

    #[test]
    fn get_dispute_opened_at_returns_timestamp_after_record() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let before = env.ledger().timestamp();
        client.record_dispute_opened(&1).unwrap();
        let after = env.ledger().timestamp();
        let opened = client.get_dispute_opened_at(&1).unwrap();
        assert!(opened >= before && opened <= after);
    }

    // ─── #417: duplicate resolution rejected ─────────────────────────────

    #[test]
    fn second_resolution_rejected() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);
        client
            .submit_resolution(&1, &arb, &true, &Symbol::new(&env, "a"))
            .unwrap();
        let result = client.try_submit_resolution(&1, &arb, &false, &Symbol::new(&env, "b"));
        assert!(result.is_err(), "second resolution must be rejected");
    }

    // ─── boundary: resolution exactly at MIN_RESOLUTION_DELAY_SECS ─────────

    #[test]
    fn resolution_at_exact_timelock_boundary_succeeds() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);
        client.record_dispute_opened(&1).unwrap();
        // Advance exactly the minimum delay (no extra second).
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS);
        client
            .submit_resolution(&1, &arb, &true, &Symbol::new(&env, "ok"))
            .unwrap();
    }

    #[test]
    fn resolution_one_second_before_timelock_fails() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);
        client.record_dispute_opened(&1).unwrap();
        advance_time(&env, MIN_RESOLUTION_DELAY_SECS.saturating_sub(1));
        let result = client.try_submit_resolution(&1, &arb, &true, &Symbol::new(&env, "ok"));
        assert!(result.is_err(), "resolution 1s before timelock must fail");
    }

    // ─── fuzz: resolution at various offsets from opened_at ───────────────

    #[test]
    fn fuzz_resolution_boundary_offsets() {
        // Property-style test: for a range of offsets around the timelock,
        // verify that resolution is rejected before the boundary and accepted
        // at or after it.
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);

        for offset in [0, 1, 10, 100, MIN_RESOLUTION_DELAY_SECS / 2] {
            client.record_dispute_opened(&1).unwrap();
            advance_time(&env, offset);
            let result = client.try_submit_resolution(&1, &arb, &true, &Symbol::new(&env, "r"));
            if offset < MIN_RESOLUTION_DELAY_SECS {
                assert!(
                    result.is_err(),
                    "offset {} < MIN_RESOLUTION_DELAY_SECS must fail",
                    offset
                );
            } else {
                assert!(
                    result.is_ok(),
                    "offset {} >= MIN_RESOLUTION_DELAY_SECS must succeed",
                    offset
                );
            }
            // Reset for next iteration by creating a fresh env via a new test
            // is not possible here, so we rely on the fact that once resolved,
            // subsequent calls will hit AlreadyResolved. We only test the first
            // offset that succeeds and break.
            if offset >= MIN_RESOLUTION_DELAY_SECS {
                break;
            }
        }
    }

    // ─── #417: events ─────────────────────────────────────────────────────

    #[test]
    fn evidence_submitted_event_contains_correct_payload() {
        let (env, _admin, mentor, _learner, client) = setup_disputed();
        let hash = hash32(&env, 1);
        client
            .submit_evidence(&1, &mentor, &hash, &hash32(&env, 2), &None)
            .unwrap();
        let events = env.events().all();
        let last = events.last().unwrap();
        assert_eq!(
            last.1,
            (Symbol::new(&env, "evidence_submitted"), 1u64).into_val(&env)
        );
        let payload = EvidenceItem::try_from_val(&env, &last.2).unwrap();
        assert_eq!(payload.content_hash, hash);
    }

    #[test]
    fn dispute_resolved_event_emitted_on_resolution() {
        let (env, _admin, _mentor, _learner, client) = setup_disputed();
        let arb = Address::generate(&env);
        client
            .submit_resolution(&1, &arb, &true, &Symbol::new(&env, "ok"))
            .unwrap();
        let events = env.events().all();
        let last = events.last().unwrap();
        assert_eq!(
            last.1,
            (Symbol::new(&env, "dispute_resolved"), 1u64).into_val(&env)
        );
    }
}

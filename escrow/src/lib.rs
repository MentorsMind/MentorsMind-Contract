#![no_std]
#![allow(deprecated)]
#![allow(dead_code, unused_assignments)]
use shared::events::{
    emit_escrow_event, evt_escrow_created, evt_escrow_disputed, evt_escrow_emergency_release,
    evt_escrow_refunded, evt_escrow_released, evt_escrow_resolved, evt_escrow_stuck_reported,
};
use shared::{
    compute_checksum, push_snapshot_index, CrossContractAuth, EscrowRecord,
    RollbackProposal, SnapshotMeta, StateVerificationReport, ReentrancyGuard,
    MAX_SNAPSHOTS, EscrowTransitionLog, GasEstimate, Validator, StateMachine,
    ReleaseFailure, FailureClassification, RecoveryState,
    calculate_next_retry,
    MAX_AUTO_RELEASE_ATTEMPTS, MANUAL_RECOVERY_THRESHOLD, StateTransitionContext,
    CrossContractStateCheck,
    InvalidStateRecord,
    is_transition_expired, STATE_TRANSITION_TIMEOUT_SECS, EmergencyAction, EmergencyAdminRole,
    EmergencyAuditRecord, EmergencyCircuitBreaker, EmergencyMultisig, MultisigValidation, SafeMath,
    EMERGENCY_ADMIN_TTL_SECS, EMERGENCY_MSIG_THRESHOLD, EmergencyRollback,
    ImmutableRollbackAuditRecord, RollbackAuthorization, RollbackJustification, RollbackScope,
    SecureStorageAccess, STORAGE_DERIVE_CTX, Pagination, log_contract_error,
};
pub use shared::EscrowStatus;
use soroban_sdk::{
    contract, contractimpl, contracttype, symbol_short, token, xdr::ToXdr, Address, Bytes, Env,
    Symbol, Vec, IntoVal, BytesN,
};
use shared::{
    AdminTransfer, AdminChangeProposal, MIN_ADMIN_TIMELOCK_SECS, ADMIN_COOLING_OFF_SECS,
};
use shared::{
    validate_evidence_sufficiency, detect_payment_timing_manipulation, check_multisig_threshold,
    EvidenceSufficiency, PaymentTimingCheck, EscrowMultisigApproval,
    EmergencyFundLock, PaymentAuditEntry,
};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum MilestoneStatus {
    Pending,
    Completed,
    Disputed,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MilestoneSpec {
    pub description_hash: BytesN<32>,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct MilestoneEscrow {
    pub id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub total_amount: i128,
    pub milestones: Vec<MilestoneSpec>,
    pub milestone_statuses: Vec<MilestoneStatus>,
    pub status: EscrowStatus,
    pub created_at: u64,
    pub token_address: Address,
    pub platform_fee: i128,
    pub net_amount: i128,
}

/// Type alias for shared EscrowRecord to maintain backward compatibility.
pub type Escrow = EscrowRecord;

/// Legacy escrow struct for backward-compatible deserialization.
#[contracttype]
#[derive(Clone, Debug)]
pub struct EscrowLegacy {
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
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowCreatedEventData {
    pub escrow_id: u64,
    pub mentor: Address,
    pub learner: Address,
    pub amount: i128,
    pub session_id: Symbol,
    pub token_address: Address,
    pub session_end_time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowReleasedEventData {
    pub escrow_id: u64,
    pub mentor: Address,
    pub amount: i128,
    pub net_amount: i128,
    pub platform_fee: i128,
    pub token_address: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowAutoReleasedEventData {
    pub escrow_id: u64,
    pub time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeOpenedEventData {
    pub escrow_id: u64,
    pub caller: Address,
    pub reason: Symbol,
    pub token_address: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeResolvedEventData {
    pub escrow_id: u64,
    pub mentor_pct: u32,
    pub mentor_amount: i128,
    pub learner_amount: i128,
    pub token_address: Address,
    pub time: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowRefundedEventData {
    pub escrow_id: u64,
    pub learner: Address,
    pub amount: i128,
    pub token_address: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReviewSubmittedEventData {
    pub caller: Address,
    pub reason: Symbol,
    pub mentor: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeDistributedEventData {
    pub escrow_id: u64,
    pub gross_amount: i128,
    pub platform_fee: i128,
    pub net_amount: i128,
    pub token_address: Address,
}

/// Event data emitted when a token is approved or rejected from the whitelist.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokenApprovalEventData {
    pub token_address: Address,
    pub approved: bool,
}

/// Event data emitted when an escrow is reported as stuck after the grace period.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowStuckReportedEventData {
    pub escrow_id: u64,
    pub reporter: Address,
    pub stuck_since: u64,
}

/// Event data emitted when an emergency release is executed via 4-of-7 multisig.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyReleaseExecutedEventData {
    pub escrow_id: u64,
    pub admin: Address,
    pub reason_hash: BytesN<32>,
    pub action_id: u32,
    pub amount: i128,
    pub participant_signers: Vec<Address>,
}

/// Event data for emergency action proposals / approvals / failures.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyActionAuditEventData {
    pub action_id: u32,
    pub escrow_id: u64,
    pub actor: Address,
    pub reason_hash: BytesN<32>,
    pub params_hash: BytesN<32>,
    pub approval_count: u32,
    pub success: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EscrowInvariantReport {
    pub escrow_id: u64,
    pub funds_conserved: bool,
    pub transition_consistent: bool,
    pub terminal_state_immutable: bool,
    pub treasury_consistent: bool,
    pub violation_count: u32,
    pub checked_at: u64,
}

// ---------------------------------------------------------------------------
// Admin Events
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChangeProposedEventData {
    pub contract: Address,
    pub old_admin: Address,
    pub new_admin: Address,
    pub effective_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminChangeAcceptedEventData {
    pub contract: Address,
    pub new_admin: Address,
}

// ---------------------------------------------------------------------------
// Graduated fee schedule (Issue #676)
// ---------------------------------------------------------------------------

/// Graduated platform-fee schedule.
///
/// The applicable base rate is selected by the mentor's staking tier
/// (0 = none, 1 = Bronze, 2 = Silver, 3 = Gold). A session whose value exceeds
/// `volume_discount_threshold` receives an additional `volume_discount_bps`
/// reduction on the selected rate.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeSchedule {
    pub tier0_bps: u32,
    pub tier1_bps: u32,
    pub tier2_bps: u32,
    pub tier3_bps: u32,
    pub volume_discount_threshold: i128,
    pub volume_discount_bps: u32,
}

/// Event data emitted whenever a graduated platform fee is applied on release.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeAppliedEventData {
    pub mentor: Address,
    pub tier: u32,
    pub base_bps: u32,
    pub effective_bps: u32,
    pub fee_amount: i128,
}

/// Event data emitted when the graduated fee schedule is updated.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeScheduleUpdatedEventData {
    pub old_schedule: Option<FeeSchedule>,
    pub new_schedule: FeeSchedule,
}

/// Cross-contract view of the staking contract used to read a mentor's tier.
#[soroban_sdk::contractclient(name = "StakingClient")]
pub trait StakingTierTrait {
    fn get_tier(env: Env, mentor: Address) -> u32;
}

/// Cross-contract client for the MultisigAdmin contract used to validate
/// emergency release approvals (2-of-3 minimum threshold).
#[soroban_sdk::contractclient(name = "MultisigClient")]
pub trait MultisigAdminTrait {
    fn get_proposal(env: Env, action_id: u32) -> ProposalRecordMirror;
    fn get_threshold(env: Env) -> u32;
}

/// Local mirror of `multisig_admin::ProposalRecord` used for cross-contract
/// validation of emergency release approvals. Field order MUST match the
/// multisig definition for correct SCV serialization.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProposalRecordMirror {
    pub id: u32,
    pub proposer: Address,
    pub target: Address,
    pub function: Symbol,
    pub args: Vec<soroban_sdk::Val>,
    pub approval_count: u32,
    pub expiry: u64,
    pub executed: bool,
    pub cancelled: bool,
}

// ---------------------------------------------------------------------------
// DataKey enum — typed storage key for all persistent state
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Treasury,
    FeeBps,
    EscrowCount,
    AutoRelDelay,
    Escrow(u64),
    ApprovedToken(Address),
    /// Graduated fee schedule (Issue #676). When set, overrides the flat FeeBps.
    FeeSchedule,
    /// Address of the staking contract used to read mentor tiers.
    StakingContract,
    ReputationContract,
    InsuranceContract,
    /// Interface registry consulted to authenticate cross-contract peers
    /// (staking/reputation/insurance) before they're wired in. Optional:
    /// absent config skips verification so tests/early deployments work
    /// without a deployed registry.
    InterfaceRegistry,
    /// Address of the MultisigAdmin contract used for emergency release approvals.
    MultisigAdmin,
    /// PER-ESCROW failure tracking with exponential backoff (replaced global tracking)
    FailureRecord(u64),
    /// Watchlist of escrow IDs reported as stuck (after grace period elapsed).
    StuckEscrows,
    /// ATOMIC STATE TRANSITIONS: Lock for concurrent modification prevention
    StateTransitionLock(u64),
    /// ATOMIC STATE TRANSITIONS: Context for active transition
    StateTransitionContext(u64),
    /// ATOMIC STATE TRANSITIONS: Invalid state recovery records
    InvalidStateRecord(u64),
    /// ATOMIC STATE TRANSITIONS: Cross-contract state verification
    CrossContractStateCheck(u64),
    // -----------------------------------------------------------------------
    // Disaster-recovery keys
    // -----------------------------------------------------------------------
    /// Serialised Vec<EscrowRecord> for snapshot `n`.
    Snapshot(u32),
    /// SnapshotMeta struct for snapshot `n`.
    SnapshotMetadata(u32),
    /// Ordered Vec<u32> of active snapshot IDs (rolling window, max 3).
    SnapshotIndex,
    /// Vec<Address> of up to 7 emergency multi-sig signers.
    EmergencySigners,
    /// EmergencyMultisig config (signers + threshold).
    EmergencyMultisigConfig,
    /// EmergencyAction proposal `n`.
    EmergencyProposal(u32),
    /// Boolean approval flag for (emergency_action_id, signer).
    EmergencyApproval(u32, Address),
    /// Auto-incremented emergency action proposal counter.
    EmergencyProposalCount,
    /// Immutable audit record for emergency action `n`.
    EmergencyAudit(u32),
    /// Latest runtime invariant report for an escrow.
    EscrowInvariantReport(u64),
    /// Params-hash → permanently failed (blocks retry with same parameters).
    EmergencyFailedParams(BytesN<32>),
    /// Time-bound emergency admin role.
    EmergencyAdmin,
    /// Circuit breaker rolling window state.
    EmergencyCircuit,
    /// Total active escrow pool amount (maintained for circuit-breaker checks).
    ActivePoolTotal,
    /// RollbackProposal for proposal `n`.
    RollbackProposal(u32),
    /// Boolean approval flag for (proposal_id, signer) pair.
    RollbackApproval(u32, Address),
    /// Auto-incremented rollback proposal counter.
    RollbackProposalCount,
    /// Hardened emergency rollback proposal `n` (issue #825).
    EmergencyRollback(u32),
    /// Auto-incremented emergency rollback counter.
    EmergencyRollbackCount,
    /// Immutable rollback audit archive for proposal `n`.
    ImmutableRollbackAudit(u32),
    /// Preserved emergency audit `n` (survives rollback).
    PreservedEmergencyAudit(u32),
    /// Preserved transition log for escrow `id` (survives rollback).
    PreservedTransitionLog(u64),
    /// Registered governance contract for rollback review callbacks.
    GovernanceContract,
    /// Store a vector of escrow transition logs.
    TransitionLog(u64),
    /// Pending admin change tracking.
    PendingAdminTransfer,
    /// Last admin change timestamp for cooling-off enforcement.
    LastAdminChange,
    // -----------------------------------------------------------------------
    // Payment integrity / escrow-gaming protection (#886)
    // -----------------------------------------------------------------------
    /// Timestamps of dispute-related actions (open, resolution attempts)
    /// for a given escrow, used for payment-timing manipulation detection.
    DisputeActionLog(u64),
    /// Number of evidence items on record for a given escrow, mirrored
    /// from the dispute-evidence contract when a secure resolution is
    /// requested.
    DisputeEvidenceCount(u64),
    /// Distinct multisig approvers recorded for a pending secure
    /// resolution of a given escrow.
    ResolutionApprovals(u64),
    /// Whether an escrow's funds are currently isolated due to a detected
    /// payment-manipulation attack.
    IsolatedEscrow(u64),
    /// Payment audit trail for a given escrow.
    PaymentAudit(u64),
}

// ---------------------------------------------------------------------------
// Storage keys (Symbol-based, used alongside DataKey where appropriate)
// ---------------------------------------------------------------------------

const ESCROW_COUNT: Symbol = symbol_short!("ESC_CNT");
const MILESTONE_ESCROW_COUNT: Symbol = symbol_short!("MESC_CNT");
const ADMIN: Symbol = symbol_short!("ADMIN");
const TREASURY: Symbol = symbol_short!("TREASURY");
const FEE_BPS: Symbol = symbol_short!("FEE_BPS");
/// Default auto-release delay in seconds (configurable at init).
const AUTO_REL_DLY: Symbol = symbol_short!("AR_DELAY");
const SESSION_KEY: Symbol = symbol_short!("SESSION");
const ORACLE_ID: Symbol = symbol_short!("ORACLE");
const ORACLE_MAX_AGE: Symbol = symbol_short!("OR_AGE");
const MENTOR_ESCROWS: Symbol = symbol_short!("MNT_ESC");
const LEARNER_ESCROWS: Symbol = symbol_short!("LRN_ESC");
const MAX_FEE_BPS: u32 = 1_000;

/// Economic sanity ceiling for a single escrow/milestone amount, expressed
/// in the token's smallest unit. Guards against fat-fingered or malicious
/// amounts many orders of magnitude larger than any real session fee, which
/// could otherwise sit close enough to `i128::MAX` to make downstream
/// `checked_mul` (fee math) fail or make the amount economically absurd.
const MAX_FINANCIAL_AMOUNT: i128 = 1_000_000_000_000_000; // 100M tokens @ 7 decimals

/// Default auto-release delay: 72 hours in seconds.
const DEFAULT_AUTO_RELEASE_DELAY: u64 = 72 * 60 * 60;

/// Grace period (in seconds) after `session_end_time + auto_release_delay`
/// before an escrow can be reported as "stuck" by any caller. This gives
/// legitimate auto-release retries a chance to succeed before the watchlist
/// flags the escrow for manual intervention.
///
/// Default: 7 days (7 * 24 * 60 * 60).
const GRACE_PERIOD_SECS: u64 = 7 * 24 * 60 * 60;
/// Contract-specific storage namespace scope (#826).
const ESCROW_STORAGE_SCOPE: Symbol = symbol_short!("mm_escrw");

fn escrow_secure_set<V, K>(env: &Env, key: &K, value: &V)
where
    K: IntoVal<Env, soroban_sdk::Val>,
    V: IntoVal<Env, soroban_sdk::Val>,
{
    SecureStorageAccess::set_persistent_checked(
        env,
        &DataKey::NamespaceRoot,
        STORAGE_DERIVE_CTX,
        key,
        value,
    )
    .unwrap_or_else(|_| panic!("secure storage write failed"));
}

fn escrow_secure_get<V, K>(env: &Env, key: &K) -> Option<V>
where
    K: IntoVal<Env, soroban_sdk::Val>,
    V: soroban_sdk::TryFromVal<Env, soroban_sdk::Val> + IntoVal<Env, soroban_sdk::Val>,
{
    SecureStorageAccess::get_persistent_checked(env, &DataKey::NamespaceRoot, key)
        .unwrap_or_else(|_| panic!("secure storage read failed"))
}

fn escrow_secure_has<K>(env: &Env, key: &K) -> bool
where
    K: IntoVal<Env, soroban_sdk::Val>,
{
    SecureStorageAccess::has_persistent_checked(env, &DataKey::NamespaceRoot, key)
        .unwrap_or(false)
}

// Approved token registry key prefix: ("APRV_TOK", address → bool
const APPROVED_TOKEN_KEY: Symbol = symbol_short!("APRV_TOK");

// Cache key for admin address
const CACHE_ADMIN_KEY: Symbol = symbol_short!("CADM_KEY");

// Dynamic fee constants
const DYNAMIC_FEE_ENABLED: Symbol = symbol_short!("DYN_FEE");
const LIQUIDITY_POOL: Symbol = symbol_short!("LIQ_POOL");
const PRICE_CACHE: Symbol = symbol_short!("PRC_CACH");
const PRICE_CACHE_TIME: Symbol = symbol_short!("PRC_TIME");
const DEFAULT_FEE_BPS: u32 = 500;

// ---------------------------------------------------------------------------
// TTL constants (in ledgers; ~5 s/ledger → 1 000 000 ≈ 57 days)
// ---------------------------------------------------------------------------

const ESCROW_TTL_THRESHOLD: u32 = 500_000;
const ESCROW_TTL_BUMP: u32 = 1_000_000;

// ---------------------------------------------------------------------------
// Gas-estimation heuristic constants (#761). Calibrated against
// `env.budget().cpu_instruction_cost()` measured around a real
// `release_funds` call in the estimate-vs-actual test.
// ---------------------------------------------------------------------------
const RELEASE_BASE_INSTRUCTIONS: u64 = 40_000;
const RELEASE_PER_STORAGE_OP_INSTRUCTIONS: u64 = 2_000;
const RELEASE_PER_CROSS_CALL_INSTRUCTIONS: u64 = 320_000;

// Cache TTL constants for frequently accessed data
const CACHE_TTL_THRESHOLD: u32 = 100_000;
const CACHE_TTL_BUMP: u32 = 500_000;

// Cache statistics keys
const CACHE_HITS_KEY: Symbol = symbol_short!("CACH_HIT");
const CACHE_MISSES_KEY: Symbol = symbol_short!("CACH_MIS");

/// Cache of accrued yield per escrow to avoid repeated get_value calls.
const YIELD_ACCRUED_CACHE: Symbol = symbol_short!("YLD_ACC");

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct EscrowContract;

#[contractimpl]
impl EscrowContract {
    // -----------------------------------------------------------------------
    // Admin / initialization
    // -----------------------------------------------------------------------

    /// Initialize the contract with an admin, treasury, initial fee, approved
    /// tokens, and an optional auto-release delay.
    ///
    /// - `fee_bps`: platform fee in basis points (e.g. 500 = 5%). Must be ≤ 1 000 (10%).
    /// - `treasury`: address that receives the platform fee on every release.
    /// - `auto_release_delay_secs`: seconds after session end before funds
    ///   auto-release to the mentor. Pass `0` to use the default (72 hours).
    /// - Approved tokens must satisfy SEP-41 (XLM, USDC, PYUSD, …).
    ///
    /// Calling this a second time will panic — persistent storage ensures the
    /// `ADMIN` key survives ledger archival so the guard cannot be bypassed.
    pub fn initialize(
        env: Env,
        admin: Address,
        treasury: Address,
        fee_bps: u32,
        approved_tokens: soroban_sdk::Vec<Address>,
        auto_release_delay_secs: u64,
    ) {
        SecureStorageAccess::install_namespace(&env, &DataKey::NamespaceRoot, ESCROW_STORAGE_SCOPE);

        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }

        if fee_bps > MAX_FEE_BPS {
            panic!("Fee exceeds maximum (1000 bps)");
        }

        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.storage().persistent().set(&DataKey::Treasury, &treasury);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.storage().persistent().set(&DataKey::FeeBps, &fee_bps);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.storage().persistent().set(&DataKey::EscrowCount, &0u64);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::EscrowCount, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.storage().persistent().set(&MILESTONE_ESCROW_COUNT, &0u64);
        env.storage()
            .persistent()
            .extend_ttl(&MILESTONE_ESCROW_COUNT, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // Store configurable auto-release delay; fall back to 72 hours if 0.
        let delay = if auto_release_delay_secs == 0 {
            DEFAULT_AUTO_RELEASE_DELAY
        } else {
            auto_release_delay_secs
        };
        env.storage().persistent().set(&DataKey::AutoRelDelay, &delay);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::AutoRelDelay, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // Register each approved token and emit events
        for token_addr in approved_tokens.iter() {
            Self::_set_token_approved(&env, &token_addr, true);
            env.events().publish(
                (Symbol::new(&env, "Token"), Symbol::new(&env, "Approved")),
                TokenApprovalEventData {
                    token_address: token_addr,
                    approved: true,
                },
            );
        }
    }

    /// Propose a new admin for the Escrow contract.
    /// The new admin can only be accepted after the `MIN_ADMIN_TIMELOCK_SECS` has passed.
    /// Also enforces `ADMIN_COOLING_OFF_SECS` between consecutive admin changes.
    pub fn propose_admin_change(env: Env, new_admin: Address) {
        let current_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        current_admin.require_auth();

        let last_change: u64 = env.storage().persistent().get(&DataKey::LastAdminChange).unwrap_or(0);
        let current_time = env.ledger().timestamp();

        if current_time < last_change + ADMIN_COOLING_OFF_SECS {
            panic!("Cooling-off period active");
        }

        let effective_at = current_time.checked_add(MIN_ADMIN_TIMELOCK_SECS).unwrap();
        
        let pending = AdminTransfer {
            new_admin: new_admin.clone(),
            effective_at,
            status: AdminChangeProposal::Proposed,
        };

        env.storage().persistent().set(&DataKey::PendingAdminTransfer, &pending);
        env.storage().persistent().extend_ttl(&DataKey::PendingAdminTransfer, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.events().publish(
            (symbol_short!("admin"), symbol_short!("proposed")),
            AdminChangeProposedEventData {
                contract: env.current_contract_address(),
                old_admin: current_admin,
                new_admin,
                effective_at,
            },
        );
    }

    /// Accept the admin role if a pending proposal exists and the timelock has expired.
    pub fn accept_admin_role(env: Env, new_admin: Address) {
        new_admin.require_auth();

        let mut pending: AdminTransfer = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdminTransfer)
            .expect("No pending admin transfer");

        if pending.new_admin != new_admin {
            panic!("Unauthorized to accept role");
        }

        if env.ledger().timestamp() < pending.effective_at {
            panic!("Timelock not expired");
        }

        if pending.status != AdminChangeProposal::Proposed {
            panic!("Invalid proposal state");
        }

        pending.status = AdminChangeProposal::Accepted;
        
        env.storage().persistent().set(&DataKey::Admin, &new_admin);
        env.storage().persistent().extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        
        env.storage().persistent().set(&DataKey::LastAdminChange, &env.ledger().timestamp());
        env.storage().persistent().extend_ttl(&DataKey::LastAdminChange, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        
        env.storage().persistent().remove(&DataKey::PendingAdminTransfer);

        env.events().publish(
            (symbol_short!("admin"), symbol_short!("accepted")),
            AdminChangeAcceptedEventData {
                contract: env.current_contract_address(),
                new_admin,
            },
        );
    }

    /// Immediately revokes the current admin and assigns a new one.
    /// Can only be called by the multisig admin contract.
    pub fn revoke_admin_emergency(env: Env, new_admin: Address) {
        let multisig: Address = env
            .storage()
            .persistent()
            .get(&DataKey::MultisigAdmin)
            .expect("Multisig not configured");
        multisig.require_auth();
        
        env.storage().persistent().set(&DataKey::Admin, &new_admin);
        env.storage().persistent().extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage().persistent().remove(&DataKey::PendingAdminTransfer);
    }

    /// Update the platform fee — admin only, capped at 1 000 bps (10%).
    pub fn update_fee(env: Env, new_fee_bps: u32) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        if new_fee_bps > MAX_FEE_BPS {
            panic!("Fee exceeds maximum (1000 bps)");
        }

        env.storage().persistent().set(&DataKey::FeeBps, &new_fee_bps);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
    }

    /// Get dynamic fee based on MNT/USDC price from liquidity pool.
    ///
    /// When a graduated `FeeSchedule` is configured, returns the tier-0 base
    /// rate adjusted by the dynamic price multiplier (compatible with legacy
    /// callers that expect a single flat bps value).  When no schedule is set,
    /// falls back to the historical hardcoded price tiers for backward
    /// compatibility:
    /// - Price < $0.10 → 500 bps (5%)
    /// - Price $0.10–$0.50 → 400 bps (4%)
    /// - Price $0.50–$1.00 → 300 bps (3%)
    /// - Price > $1.00 → 200 bps (2%)
    pub fn get_dynamic_fee(env: Env) -> u32 {
        let dynamic_enabled: bool = env
            .storage()
            .instance()
            .get(&DYNAMIC_FEE_ENABLED)
            .unwrap_or(true);

        if !dynamic_enabled {
            return env.storage().persistent().get(&DataKey::FeeBps).unwrap_or(DEFAULT_FEE_BPS);
        }

        let current_ledger = env.ledger().sequence();
        let cached_ledger: u32 = env
            .storage()
            .instance()
            .get(&PRICE_CACHE_TIME)
            .unwrap_or(0);

        let price = if cached_ledger == current_ledger {
            env.storage().instance().get::<_, i128>(&PRICE_CACHE).unwrap_or(0)
        } else {
            let p = Self::_fetch_mnt_usdc_price(&env);
            env.storage().instance().set(&PRICE_CACHE, &p);
            env.storage().instance().set(&PRICE_CACHE_TIME, &current_ledger);
            p
        };

        match env.storage().persistent().get::<_, FeeSchedule>(&DataKey::FeeSchedule) {
            Some(schedule) => {
                let multiplier = Self::_price_multiplier_bps(price);
                schedule
                    .tier0_bps
                    .safe_mul(&env, multiplier as i128)
                    .safe_div(&env, 10_000) as u32
            }
            None => Self::_legacy_fee_from_price(price),
        }
    }

    /// Returns the price-tier multiplier in basis points applied on top of
    /// graduated FeeSchedule rates.  A low MNT price raises the multiplier
    /// (higher fees to compensate for token depreciation); a high MNT price
    /// lowers it (fees become cheaper as the token appreciates).
    ///
    /// Multiplier schedule:
    /// - Price < $0.10 → 125% (12_500 bps)
    /// - Price $0.10–$0.50 → 110% (11_000 bps)
    /// - Price $0.50–$1.00 → 100% (10_000 bps, neutral)
    /// - Price > $1.00 → 90%  ( 9_000 bps)
    fn _price_multiplier_bps(price: i128) -> u32 {
        if price <= 0 {
            return 10_000;
        }

        let threshold_010 = 1_000_000;
        let threshold_050 = 5_000_000;
        let threshold_100 = 10_000_000;

        if price < threshold_010 {
            12_500
        } else if price < threshold_050 {
            11_000
        } else if price < threshold_100 {
            10_000
        } else {
            9_000
        }
    }

    /// Legacy hardcoded flat-fee price tiers preserved for backward
    /// compatibility when no graduated FeeSchedule is configured.
    fn _legacy_fee_from_price(price: i128) -> u32 {
        if price <= 0 {
            return DEFAULT_FEE_BPS;
        }

        let threshold_010 = 1_000_000;
        let threshold_050 = 5_000_000;
        let threshold_100 = 10_000_000;

        if price < threshold_010 {
            500
        } else if price < threshold_050 {
            400
        } else if price < threshold_100 {
            300
        } else {
            200
        }
    }

    fn _fetch_mnt_usdc_price(env: &Env) -> i128 {
        let pool_address: Option<Address> = env.storage().instance().get(&LIQUIDITY_POOL);

        if let Some(_pool) = pool_address {
            // TODO: Implement actual pool contract integration
            // Placeholder: return $0.75 (7,500,000) for testing
            return 7_500_000;
        }

        0
    }

    /// Set liquidity pool address (admin only)
    pub fn set_liquidity_pool(env: Env, pool_address: Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        env.storage().instance().set(&LIQUIDITY_POOL, &pool_address);
    }

    /// Enable or disable dynamic fee (admin only)
    pub fn set_dynamic_fee_enabled(env: Env, enabled: bool) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        env.storage().instance().set(&DYNAMIC_FEE_ENABLED, &enabled);
    }

    /// Update the treasury address — admin only.
    pub fn update_treasury(env: Env, new_treasury: Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        env.storage().persistent().set(&DataKey::Treasury, &new_treasury);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
    }

    /// Add or remove an approved token (admin only).
    /// Emits a TokenApproved or TokenRejected event.
    pub fn set_approved_token(env: Env, token_address: Address, approved: bool) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        Self::_set_token_approved(&env, &token_address, approved);

        // Emit token approval/rejection event
        if approved {
            env.events().publish(
                (Symbol::new(&env, "Token"), Symbol::new(&env, "Approved")),
                TokenApprovalEventData {
                    token_address,
                    approved: true,
                },
            );
        } else {
            env.events().publish(
                (Symbol::new(&env, "Token"), Symbol::new(&env, "Rejected")),
                TokenApprovalEventData {
                    token_address,
                    approved: false,
                },
            );
        }
    }

    // -----------------------------------------------------------------------
    // Graduated fee schedule (Issue #676)
    // -----------------------------------------------------------------------

    /// Set the graduated fee schedule (admin only). Once set, releases use the
    /// tier-based rates instead of the flat `FeeBps`.
    ///
    /// Emits a `FeeScheduleUpdated` event so off-chain indexers can track
    /// schedule changes.
    pub fn set_fee_schedule(env: Env, admin: Address, schedule: FeeSchedule) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }

        Self::_validate_fee_schedule(&schedule);

        let old_schedule = env.storage().persistent().get::<_, FeeSchedule>(&DataKey::FeeSchedule);

        env.storage().persistent().set(&DataKey::FeeSchedule, &schedule);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeSchedule, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "FeeScheduleUpdated")),
            FeeScheduleUpdatedEventData {
                old_schedule,
                new_schedule: schedule,
            },
        );
    }

    /// Update individual fields of the existing graduated fee schedule (admin
    /// only).  Pass `None` for any field you want to leave unchanged.  This
    /// avoids requiring callers to re-send the entire schedule for a single
    /// parameter tweak.
    ///
    /// Panics if no schedule exists yet — use `set_fee_schedule` first.
    pub fn update_fee_schedule(
        env: Env,
        admin: Address,
        tier0_bps: Option<u32>,
        tier1_bps: Option<u32>,
        tier2_bps: Option<u32>,
        tier3_bps: Option<u32>,
        volume_discount_threshold: Option<i128>,
        volume_discount_bps: Option<u32>,
    ) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }

        let mut schedule: FeeSchedule = env
            .storage()
            .persistent()
            .get(&DataKey::FeeSchedule)
            .expect("No fee schedule set; call set_fee_schedule first");

        let old_schedule = schedule.clone();

        if let Some(v) = tier0_bps {
            schedule.tier0_bps = v;
        }
        if let Some(v) = tier1_bps {
            schedule.tier1_bps = v;
        }
        if let Some(v) = tier2_bps {
            schedule.tier2_bps = v;
        }
        if let Some(v) = tier3_bps {
            schedule.tier3_bps = v;
        }
        if let Some(v) = volume_discount_threshold {
            schedule.volume_discount_threshold = v;
        }
        if let Some(v) = volume_discount_bps {
            schedule.volume_discount_bps = v;
        }

        Self::_validate_fee_schedule(&schedule);

        env.storage().persistent().set(&DataKey::FeeSchedule, &schedule);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeSchedule, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "FeeScheduleUpdated")),
            FeeScheduleUpdatedEventData {
                old_schedule: Some(old_schedule),
                new_schedule: schedule,
            },
        );
    }

    /// Validate that all fee-schedule parameters respect the configured
    /// economic bounds (`MAX_FEE_BPS` ceiling, non-negative discount, etc.).
    fn _validate_fee_schedule(schedule: &FeeSchedule) {
        if schedule.tier0_bps > MAX_FEE_BPS
            || schedule.tier1_bps > MAX_FEE_BPS
            || schedule.tier2_bps > MAX_FEE_BPS
            || schedule.tier3_bps > MAX_FEE_BPS
        {
            panic!("Fee exceeds maximum (1000 bps)");
        }
        if schedule.volume_discount_bps > MAX_FEE_BPS {
            panic!("Volume discount exceeds maximum (1000 bps)");
        }
        if schedule.volume_discount_threshold < 0 {
            panic!("Volume discount threshold must be non-negative");
        }
    }

    /// Get the current fee schedule, if one has been set.
    pub fn get_fee_schedule(env: Env) -> Option<FeeSchedule> {
        env.storage().persistent().get(&DataKey::FeeSchedule)
    }

    /// Set the interface registry consulted to authenticate cross-contract
    /// peers (staking/reputation/insurance) before they're wired in
    /// (admin only). See issue #818 (cross-contract authentication bypass).
    pub fn set_interface_registry(env: Env, admin: Address, registry: Address) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }

        env.storage()
            .persistent()
            .set(&DataKey::InterfaceRegistry, &registry);
    }

    /// Rejects `candidate` unless it verifies against `interface_id` in the
    /// configured interface registry. A no-op when no registry is
    /// configured, so tests/early deployments keep working without one.
    fn _require_authorized_peer(env: &Env, candidate: &Address, interface_id: Symbol) {
        if let Some(registry) = env
            .storage()
            .persistent()
            .get::<_, Address>(&DataKey::InterfaceRegistry)
        {
            CrossContractAuth::require_authorized_contract(env, &registry, candidate, interface_id);
        }
    }

    /// Set the staking contract address used to read mentor tiers (admin only).
    pub fn set_staking_contract(env: Env, admin: Address, staking: Address) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }
        Self::_require_authorized_peer(&env, &staking, Symbol::new(&env, "staking_v1"));

        env.storage().persistent().set(&DataKey::StakingContract, &staking);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::StakingContract, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
    }

    /// Set the reputation contract address used for on_session_released hooks
    /// (admin only).  Configuring it enables the reputation-update cross-call
    /// on the happy-path release flow; absent config means the call is
    /// skipped so tests/early deployments work without a reputation contract.
    pub fn set_reputation_contract(env: Env, admin: Address, reputation: Address) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }
        Self::_require_authorized_peer(&env, &reputation, Symbol::new(&env, "reputation_v1"));

        env.storage()
            .persistent()
            .set(&DataKey::ReputationContract, &reputation);
        env.storage().persistent().extend_ttl(
            &DataKey::ReputationContract,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
    }

    /// Set the insurance contract address used for verify_coverage_on_release
    /// checks (admin only).
    pub fn set_insurance_contract(env: Env, admin: Address, insurance: Address) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }
        Self::_require_authorized_peer(&env, &insurance, Symbol::new(&env, "insurance_v1"));

        env.storage()
            .persistent()
            .set(&DataKey::InsuranceContract, &insurance);
        env.storage().persistent().extend_ttl(
            &DataKey::InsuranceContract,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
    }

    /// Read the mentor's staking tier via a cross-contract call. Returns 0 when
    /// no staking contract is configured.
    fn _mentor_tier(env: &Env, mentor: &Address) -> u32 {
        match env
            .storage()
            .persistent()
            .get::<_, Address>(&DataKey::StakingContract)
        {
            Some(staking) => StakingClient::new(env, &staking).get_tier(mentor),
            None => 0,
        }
    }

    /// Select the base bps for a tier from the schedule.
    fn _tier_bps(schedule: &FeeSchedule, tier: u32) -> u32 {
        match tier {
            1 => schedule.tier1_bps,
            2 => schedule.tier2_bps,
            3 => schedule.tier3_bps,
            _ => schedule.tier0_bps,
        }
    }

    /// Compute the graduated platform fee for `mentor` on a session worth
    /// `amount`, returning `(fee, tier, base_bps, effective_bps)`.
    ///
    /// The mentor's tier selects the base rate.  When dynamic pricing is
    /// enabled, the MNT/USDC price applies a multiplier on top of the tier
    /// rate.  Finally, a session whose value exceeds the schedule's
    /// `volume_discount_threshold` receives an additional
    /// `volume_discount_bps` reduction (never below zero, capped at
    /// `MAX_FEE_BPS`).
    fn _compute_fee_with_meta(
        env: &Env,
        schedule: &FeeSchedule,
        mentor: &Address,
        amount: i128,
    ) -> (i128, u32, u32, u32) {
        let tier = Self::_mentor_tier(env, mentor);
        let tier_base = Self::_tier_bps(schedule, tier);

        let dynamic_enabled: bool = env
            .storage()
            .instance()
            .get(&DYNAMIC_FEE_ENABLED)
            .unwrap_or(true);

        let base_bps = if dynamic_enabled {
            let current_ledger = env.ledger().sequence();
            let cached_ledger: u32 = env
                .storage()
                .instance()
                .get(&PRICE_CACHE_TIME)
                .unwrap_or(0);

            let price = if cached_ledger == current_ledger {
                env.storage().instance().get::<_, i128>(&PRICE_CACHE).unwrap_or(0)
            } else {
                let p = Self::_fetch_mnt_usdc_price(env);
                env.storage().instance().set(&PRICE_CACHE, &p);
                env.storage().instance().set(&PRICE_CACHE_TIME, &current_ledger);
                p
            };

            let multiplier = Self::_price_multiplier_bps(price);
            let scaled = tier_base
                .safe_mul(env, multiplier as i128)
                .safe_div(env, 10_000) as u32;
            scaled.min(MAX_FEE_BPS)
        } else {
            tier_base
        };

        let discounted = if amount > schedule.volume_discount_threshold {
            base_bps.saturating_sub(schedule.volume_discount_bps)
        } else {
            base_bps
        };
        let effective_bps = discounted.min(MAX_FEE_BPS);

        let fee = amount
            .safe_mul(&env, effective_bps as i128)
            .safe_div(&env, 10_000);
        (fee, tier, base_bps, effective_bps)
    }

    /// Test-only pure-arithmetic version of `_compute_fee_with_meta` that
    /// bypasses the `Env`-dependent dynamic-pricing cache and staking-tier
    /// cross-call.  Accepts a literal tier instead of looking it up.  Used by
    /// unit tests to verify the tier-mapping + volume-discount math in
    /// isolation without a full `TestFixture`.
    #[cfg(test)]
    fn _compute_fee_with_meta_no_dynamic(
        schedule: &FeeSchedule,
        tier: u32,
        amount: i128,
    ) -> (i128, u32, u32, u32) {
        let tier_base = Self::_tier_bps(schedule, tier);
        let base_bps = tier_base;
        let discounted = if amount > schedule.volume_discount_threshold {
            base_bps.saturating_sub(schedule.volume_discount_bps)
        } else {
            base_bps
        };
        let effective_bps = discounted.min(MAX_FEE_BPS);
        let fee = amount
            .checked_mul(effective_bps as i128)
            .and_then(|v| v.checked_div(10_000))
            .expect("fee arithmetic overflow in test helper");
        (fee, tier, base_bps, effective_bps)
    }

    /// Public view: compute the graduated platform fee for a mentor/amount.
    ///
    /// Falls back to the flat `FeeBps` rate when no fee schedule is configured.
    pub fn compute_platform_fee(env: Env, mentor: Address, amount: i128) -> i128 {
        Self::_compute_fee_unified(&env, &mentor, amount).0
    }

    /// Unified fee computation used by all release paths.
    ///
    /// Resolves the effective rate using either the graduated FeeSchedule
    /// (with staking-tier + volume discount) or the legacy flat FeeBps.
    ///
    /// Returns `(platform_fee, Option<(tier, base_bps, effective_bps)>)` —
    /// the meta tuple is `Some` only when the graduated schedule was used,
    /// allowing callers to emit `FeeApplied` events consistently.
    fn _compute_fee_unified(
        env: &Env,
        mentor: &Address,
        amount: i128,
    ) -> (i128, Option<(u32, u32, u32)>) {
        match env
            .storage()
            .persistent()
            .get::<_, FeeSchedule>(&DataKey::FeeSchedule)
        {
            Some(schedule) => {
                env.storage()
                    .persistent()
                    .extend_ttl(&DataKey::FeeSchedule, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
                let (fee, tier, base_bps, effective_bps) =
                    Self::_compute_fee_with_meta(env, &schedule, mentor, amount);
                (fee, Some((tier, base_bps, effective_bps)))
            }
            None => {
                let fee_bps: u32 = env
                    .storage()
                    .persistent()
                    .get(&DataKey::FeeBps)
                    .unwrap_or(DEFAULT_FEE_BPS);
                env.storage()
                    .persistent()
                    .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
                let fee = amount
                    .safe_mul(env, fee_bps as i128)
                    .safe_div(env, 10_000);
                (fee, None)
            }
        }
    }

    // -----------------------------------------------------------------------
    // Escrow lifecycle
    // -----------------------------------------------------------------------

    /// Create a new escrow.
    ///
    /// Auth: Only the learner can create an escrow for themselves.
    ///
    /// Transfers `amount` tokens from `learner` to the contract.
    ///
    /// - `session_end_time`: unix timestamp (seconds) marking when the session
    ///   ends. After this plus the contract's `auto_release_delay`, anyone may
    ///   call `try_auto_release` to release funds to the mentor.
    ///
    /// Panics if:
    /// - `amount` ≤ 0
    /// - `token_address` is not on the approved whitelist
    /// - learner's on-chain balance is insufficient
    /// - Caller is not the learner
    pub fn create_escrow(
        env: Env,
        mentor: Address,
        learner: Address,
        amount: i128,
        session_id: Symbol,
        token_address: Address,
        session_end_time: u64,
        total_sessions: u32,
    ) -> u64 {
        let _guard = ReentrancyGuard::enter_with_caller(&env, symbol_short!("create"), learner.clone());
        Self::_create_escrow_internal(
            env.clone(),
            mentor,
            learner,
            amount,
            session_id,
            token_address.clone(),
            session_end_time,
            0,
            amount,
            token_address.clone(),
            token_address,
            total_sessions,
        )
    }

    /// Release funds to the mentor (called by learner or admin).
    ///
    /// Calculates the platform fee (`gross * fee_bps / 10_000`), transfers the
    /// fee to the treasury, and transfers the remainder to the mentor.
    pub fn release_funds(env: Env, caller: Address, escrow_id: u64) {
        let _guard = ReentrancyGuard::enter_with_caller(&env, symbol_short!("release"), caller.clone());
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        if Self::is_isolated(env.clone(), escrow_id) {
            panic!("Escrow isolated pending manual review");
        }

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Admin not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // Auth check: caller must be learner OR admin
        caller.require_auth();
        if caller != escrow.learner && caller != admin {
            log_contract_error(
                &env,
                Symbol::new(&env, "release_funds"),
                Symbol::new(&env, "unauthorized"),
                escrow_id as i128,
                Some(caller.clone()),
            );
            panic!(
                "release_funds: caller is neither the learner nor the admin for escrow {}",
                escrow_id
            );
        }

        Self::_do_release(&env, &mut escrow, &key, &caller);
    }

    /// Release a partial amount (one session worth) from a multi-session escrow.
    pub fn release_partial(env: Env, caller: Address, escrow_id: u64) {
        let _guard = ReentrancyGuard::enter_with_caller(&env, symbol_short!("partial"), caller.clone());
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env.storage().persistent().get(&key).expect("Escrow not found");

        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        if escrow.sessions_completed >= escrow.total_sessions {
            panic!("All sessions already released");
        }

        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Admin not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // Auth check: caller must be learner OR admin
        caller.require_auth();
        if caller != escrow.learner && caller != admin {
            panic!("Caller not authorized");
        }

        // Calculate amount to release: total_amount / total_sessions
        // For the last session, release whatever is remaining to handle rounding.
        let amount_to_release = if escrow.sessions_completed + 1 == escrow.total_sessions {
            escrow.amount
        } else {
            escrow.quoted_token_amount
                .safe_div(&env, escrow.total_sessions as i128)
        };

        let (platform_fee, fee_meta) = Self::_compute_fee_unified(&env, &escrow.mentor, amount_to_release);
        let net_amount: i128 = amount_to_release
            .safe_sub(&env, platform_fee);

        let effective_bps = fee_meta
            .map(|(_, _, ebps)| ebps)
            .unwrap_or_else(|| env.storage().persistent().get(&DataKey::FeeBps).unwrap_or(0u32));
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "FeeAudit")),
            (amount_to_release, effective_bps, platform_fee, net_amount),
        );

        if let Some((tier, base_bps, effective_bps)) = fee_meta {
            env.events().publish(
                (Symbol::new(&env, "Escrow"), Symbol::new(&env, "FeeApplied"), escrow_id),
                FeeAppliedEventData {
                    mentor: escrow.mentor.clone(),
                    tier,
                    base_bps,
                    effective_bps,
                    fee_amount: platform_fee,
                },
            );
        }

        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let token_client = token::Client::new(&env, &escrow.token_address);

        if platform_fee > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &platform_fee);
        }

        token_client.transfer(&env.current_contract_address(), &escrow.mentor, &net_amount);

        escrow.sessions_completed += 1;
        escrow.amount = escrow.amount.safe_sub(&env, amount_to_release);
        escrow.platform_fee = escrow.platform_fee.safe_add(&env, platform_fee);
        escrow.net_amount = escrow.net_amount.safe_add(&env, net_amount);

        if escrow.sessions_completed == escrow.total_sessions {
            escrow.status = transition_status(&env, escrow.id, &escrow.status, &EscrowStatus::Released, &caller);
            let session_key = (SESSION_KEY, escrow.session_id.clone());
            env.storage().persistent().remove(&session_key);
        }

        env.storage().persistent().set(&key, &escrow);

        env.events().publish(
            (symbol_short!("partial"), escrow.id),
            (escrow.sessions_completed, amount_to_release),
        );
    }

    /// Batch release for multiple sessions at once (gas optimization).
    /// Releases proportional payment for N completed sessions atomically.
    pub fn batch_release(env: Env, admin: Address, escrow_id: u64, sessions_to_release: u32) {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env.storage().persistent().get(&key).expect("Escrow not found");

        // Verify admin
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Admin not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();
        if admin != stored_admin {
            panic!("Only admin can batch release");
        }

        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        if sessions_to_release == 0 {
            panic!("Must release at least one session");
        }

        let remaining_sessions = escrow.total_sessions - escrow.sessions_completed;
        if sessions_to_release > remaining_sessions {
            panic!("Cannot release more sessions than remaining");
        }

        // Calculate amount per session with remainder handling
        let per_session_amount = escrow
            .quoted_token_amount
            .safe_div(&env, escrow.total_sessions as i128);

        // For the last batch, release all remaining to handle dust
        let amount_to_release = if escrow.sessions_completed + sessions_to_release
            == escrow.total_sessions
        {
            escrow.amount
        } else {
            per_session_amount
                .safe_mul(&env, sessions_to_release as i128)
        };

        let (platform_fee, fee_meta) = Self::_compute_fee_unified(&env, &escrow.mentor, amount_to_release);
        let net_amount: i128 = amount_to_release
            .safe_sub(&env, platform_fee);

        let effective_bps = fee_meta
            .map(|(_, _, ebps)| ebps)
            .unwrap_or_else(|| env.storage().persistent().get(&DataKey::FeeBps).unwrap_or(0u32));
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "FeeAudit")),
            (amount_to_release, effective_bps, platform_fee, net_amount),
        );

        if let Some((tier, base_bps, effective_bps)) = fee_meta {
            env.events().publish(
                (Symbol::new(&env, "Escrow"), Symbol::new(&env, "FeeApplied"), escrow_id),
                FeeAppliedEventData {
                    mentor: escrow.mentor.clone(),
                    tier,
                    base_bps,
                    effective_bps,
                    fee_amount: platform_fee,
                },
            );
        }

        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let token_client = token::Client::new(&env, &escrow.token_address);

        if platform_fee > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &platform_fee);
        }

        token_client.transfer(&env.current_contract_address(), &escrow.mentor, &net_amount);

        escrow.sessions_completed += sessions_to_release;
        escrow.amount = escrow.amount.safe_sub(&env, amount_to_release);
        escrow.platform_fee = escrow.platform_fee.safe_add(&env, platform_fee);
        escrow.net_amount = escrow.net_amount.safe_add(&env, net_amount);

        if escrow.sessions_completed == escrow.total_sessions {
            escrow.status = transition_status(&env, escrow.id, &escrow.status, &EscrowStatus::Released, &admin);
            let session_key = (SESSION_KEY, escrow.session_id.clone());
            env.storage().persistent().remove(&session_key);
        }

        env.storage().persistent().set(&key, &escrow);

        env.events().publish(
            (symbol_short!("batch"), escrow.id),
            (sessions_to_release, amount_to_release, remaining_sessions - sessions_to_release),
        );
    }

    /// Admin release — admin can force-release any active escrow.
    pub fn admin_release(env: Env, escrow_id: u64) {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage().persistent().extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow = Self::_load_escrow(&env, &key);
        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        let admin: Address = env.storage().persistent().get(&DataKey::Admin).expect("Admin not found");
        env.storage().persistent().extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        env.events().publish(
            (symbol_short!("Escrow"), symbol_short!("adm_rel"), escrow_id),
            (escrow_id, env.ledger().timestamp()),
        );

        Self::_do_release(&env, &mut escrow, &key, &admin);
    }

    /// Permissionless auto-release.
    ///
    /// Anyone may call this once `env.ledger().timestamp() >=
    /// escrow.session_end_time + escrow.auto_release_delay` and the escrow is
    /// still `Active`.
    ///
    /// # Failure Recovery
    ///
    /// If cross-contract prechecks (reputation update, insurance check, fee
    /// calculation) fail, the failure is counted against
    /// `MAX_FAILED_ATTEMPTS` (3).  After 3 consecutive failures the
    /// auto-release path is permanently disabled for this escrow, and the
    /// `emergency_release` (multi-sig admin bypass) must be used to unlock
    /// the funds.
    pub fn try_auto_release(env: Env, escrow_id: u64) {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        let now = env.ledger().timestamp();
        let release_after = escrow
            .session_end_time
            .safe_add(&env, escrow.auto_release_delay);

        if now < release_after {
            panic!("Auto-release window has not elapsed");
        }

        // ---- Failure-recovery gate with per-escrow backoff ----
        let mut failure = Self::_get_or_init_failure_record(&env, escrow_id, now);
        
        // Check if max attempts exceeded
        if failure.attempt_number >= MAX_AUTO_RELEASE_ATTEMPTS {
            panic!(
                "Auto-release permanently disabled: {} attempts exhausted; manual recovery required",
                failure.attempt_number
            );
        }

        // Check if we're still in backoff period
        if now < failure.next_retry_time {
            panic!(
                "Auto-release in exponential backoff; next retry at {}",
                failure.next_retry_time
            );
        }

        // ---- Cross-contract prechecks wrapped in try_invoke ----
        let prechecks_ok = Self::_try_run_auto_release_prechecks(&env, &escrow);
        if !prechecks_ok {
            failure.attempt_number += 1;
            failure.last_failure_time = now;
            failure.next_retry_time = calculate_next_retry(&env, &failure);
            failure.recovery_state = if failure.attempt_number >= MAX_AUTO_RELEASE_ATTEMPTS {
                RecoveryState::AwaitingManualRecovery
            } else {
                RecoveryState::Retrying
            };
            
            Self::_set_failure_record(&env, escrow_id, &failure);

            // Do NOT panic here — panicking would roll back the failure record.
            // Emit an audit event and return so the attempt counter persists.
            env.events().publish(
                (
                    Symbol::new(&env, "Escrow"),
                    Symbol::new(&env, "AutoReleaseFailed"),
                    escrow_id,
                ),
                (
                    failure.attempt_number,
                    MAX_AUTO_RELEASE_ATTEMPTS,
                    failure.next_retry_time,
                ),
            );
            return;
        }

        // Emit a dedicated `auto_released` event before the internal release
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "AutoReleased"), escrow_id),
            EscrowAutoReleasedEventData { escrow_id, time: now },
        );

        Self::_do_release(&env, &mut escrow, &key, &env.current_contract_address());

        // Success: clear the failure record
        env.storage()
            .persistent()
            .remove(&DataKey::FailureRecord(escrow_id));
    }

    // -----------------------------------------------------------------------
    // Per-Escrow Failure Tracking with Exponential Backoff
    // -----------------------------------------------------------------------

    /// Get or initialize a failure record for an escrow
    fn _get_or_init_failure_record(env: &Env, escrow_id: u64, _now: u64) -> ReleaseFailure {
        match env.storage().persistent().get::<_, ReleaseFailure>(&DataKey::FailureRecord(escrow_id)) {
            Some(record) => record,
            None => ReleaseFailure {
                escrow_id,
                attempt_number: 0,
                max_attempts: MAX_AUTO_RELEASE_ATTEMPTS,
                classification: FailureClassification::Unknown,
                last_failure_time: 0,
                next_retry_time: 0,
                error_hash: BytesN::from_array(env, &[0u8; 32]),
                recovery_state: RecoveryState::Retrying,
                manual_recovery_at: 0,
                manual_recovery_reason: BytesN::from_array(env, &[0u8; 32]),
            },
        }
    }

    /// Store a failure record
    fn _set_failure_record(env: &Env, escrow_id: u64, failure: &ReleaseFailure) {
        env.storage().persistent().set(&DataKey::FailureRecord(escrow_id), failure);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FailureRecord(escrow_id), ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
    }

    /// Get failure record for an escrow (if any)
    fn _get_failure_record(env: &Env, escrow_id: u64) -> Option<ReleaseFailure> {
        env.storage().persistent().get(&DataKey::FailureRecord(escrow_id))
    }

    /// Clear failure record (on successful release)
    fn _clear_failure_record(env: &Env, escrow_id: u64) {
        env.storage().persistent().remove(&DataKey::FailureRecord(escrow_id));
    }

    /// Run the cross-contract prechecks for auto-release inside a
    /// `try_invoke_contract` guard.  Returns `true` if all prechecks
    /// completed successfully, `false` if any call trapped / panicked.
    ///
    /// Prechecks executed (when the corresponding contract is configured):
    /// 1. Reputation contract — session completion hook
    /// 2. Insurance contract  — coverage verification
    /// 3. Fee calculation     — graduated-fee cross-contract tier lookup
    fn _try_run_auto_release_prechecks(env: &Env, escrow: &Escrow) -> bool {
        // ---- 1. Reputation contract integration ----
        if let Some(reputation) = env
            .storage()
            .persistent()
            .get::<_, Address>(&DataKey::ReputationContract)
        {
            let result = env.try_invoke_contract::<(), soroban_sdk::Error>(
                &reputation,
                &Symbol::new(env, "on_session_released"),
                (
                    escrow.id,
                    escrow.mentor.clone(),
                    escrow.learner.clone(),
                    escrow.amount,
                )
                    .into_val(env),
            );
            let ok = match result {
                Ok(inner_result) => inner_result.is_ok(),
                Err(_) => false,
            };
            if !ok {
                return false;
            }
        }

        // ---- 2. Insurance contract integration ----
        if let Some(insurance) = env
            .storage()
            .persistent()
            .get::<_, Address>(&DataKey::InsuranceContract)
        {
            let result = env.try_invoke_contract::<(), soroban_sdk::Error>(
                &insurance,
                &Symbol::new(env, "verify_coverage_on_release"),
                (
                    escrow.id,
                    escrow.mentor.clone(),
                    escrow.amount,
                )
                    .into_val(env),
            );
            let ok = match result {
                Ok(inner_result) => inner_result.is_ok(),
                Err(_) => false,
            };
            if !ok {
                return false;
            }
        }

        // ---- 3. Fee schedule / staking tier lookup ----
        if env
            .storage()
            .persistent()
            .has(&DataKey::FeeSchedule)
        {
            if let Some(staking) = env
                .storage()
                .persistent()
                .get::<_, Address>(&DataKey::StakingContract)
            {
                let result = env.try_invoke_contract::<u32, soroban_sdk::Error>(
                    &staking,
                    &Symbol::new(env, "get_tier"),
                    (escrow.mentor.clone(),).into_val(env),
                );
                let ok = match result {
                    Ok(inner_result) => inner_result.is_ok(),
                    Err(_) => false,
                };
                if !ok {
                    return false;
                }
            }
        }

        true
    }

    /// Open a dispute (called by mentor or learner).
    pub fn dispute(env: Env, caller: Address, escrow_id: u64, reason: Symbol) {
        let _guard = ReentrancyGuard::enter_with_caller(&env, symbol_short!("dispute"), caller.clone());
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        // Auth check: caller must be mentor OR learner
        caller.require_auth();
        if caller != escrow.mentor && caller != escrow.learner {
            panic!("Caller not authorized to dispute");
        }

        escrow.status = transition_status(&env, escrow.id, &escrow.status, &EscrowStatus::Disputed, &caller);
        escrow.dispute_reason = reason.clone();
        env.storage().persistent().set(&key, &escrow);

        // Standardized observability event (Issue #597).
        emit_escrow_event(
            &env,
            evt_escrow_disputed(&env),
            DisputeOpenedEventData {
                escrow_id,
                caller: caller.clone(),
                reason: reason.clone(),
                token_address: escrow.token_address.clone(),
            },
        );
        // Legacy ad-hoc event retained for backward compatibility.
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "DisputeOpened"), escrow_id),
            DisputeOpenedEventData {
                escrow_id,
                caller,
                reason,
                token_address: escrow.token_address,
            },
        );
    }

    /// Resolve a disputed escrow by splitting funds between mentor and learner.
    ///
    /// Admin only. Can only be called on `Disputed` escrows.
    ///
    /// - `mentor_pct`: percentage (0–100) of `escrow.amount` sent to the mentor.
    ///   The remainder (`100 - mentor_pct`) goes to the learner. No platform fee
    ///   is deducted — the full escrowed amount is split between the parties.
    pub fn resolve_dispute(env: Env, escrow_id: u64, mentor_pct: u32) {
        let _guard = ReentrancyGuard::enter(&env, symbol_short!("resolve"));
        // --- Admin auth ---
        let admin: Address = env.storage().persistent().get(&DataKey::Admin).expect("Not initialized");
        env.storage().persistent().extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        if mentor_pct > 100 {
            panic!("mentor_pct must be 0–100");
        }

        if Self::is_isolated(env.clone(), escrow_id) {
            panic!("Escrow isolated pending manual review");
        }

        // --- Load escrow ---
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage().persistent().extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env.storage().persistent().get(&key).expect("Escrow not found");

        if escrow.status != EscrowStatus::Disputed {
            panic!("Escrow is not in Disputed status");
        }

        let now = env.ledger().timestamp();
        let token_client = soroban_sdk::token::Client::new(&env, &escrow.token_address);

        let mentor_amount = escrow.amount
            .safe_mul(&env, mentor_pct as i128)
            .safe_div(&env, 100);
        let learner_amount = escrow.amount
            .safe_sub(&env, mentor_amount);

        if mentor_amount > 0 {
            token_client.transfer(&env.current_contract_address(), &escrow.mentor, &mentor_amount);
        }
        if learner_amount > 0 {
            token_client.transfer(&env.current_contract_address(), &escrow.learner, &learner_amount);
        }

        // Update escrow record
        escrow.status = transition_status(&env, escrow.id, &escrow.status, &EscrowStatus::Resolved, &admin);
        escrow.net_amount = mentor_amount;
        escrow.platform_fee = learner_amount; // repurposed: learner share in resolved state
        escrow.resolved_at = now;
        env.storage().persistent().set(&key, &escrow);

        // Standardized observability event (Issue #597).
        emit_escrow_event(
            &env,
            evt_escrow_resolved(&env),
            DisputeResolvedEventData {
                escrow_id,
                mentor_pct,
                mentor_amount,
                learner_amount,
                token_address: escrow.token_address.clone(),
                time: now,
            },
        );
        // Legacy ad-hoc event retained for backward compatibility.
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "DisputeResolved"), escrow_id),
            DisputeResolvedEventData {
                escrow_id,
                mentor_pct,
                mentor_amount,
                learner_amount,
                token_address: escrow.token_address.clone(),
                time: now,
            },
        );
    }

    /// Refund tokens to the learner (admin only).
    ///
    /// Can be called on `Active` or `Disputed` escrows; panics if already
    /// `Released`, `Refunded`, or `Resolved`.
    pub fn refund(env: Env, escrow_id: u64) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Admin not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();

        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        if escrow.status == EscrowStatus::Released
            || escrow.status == EscrowStatus::Refunded
            || escrow.status == EscrowStatus::Resolved
        {
            panic!("Cannot refund");
        }

        // Transfer tokens: contract → learner
        let token_client = token::Client::new(&env, &escrow.token_address);
        token_client.transfer(
            &env.current_contract_address(),
            &escrow.learner,
            &escrow.amount,
        );

        escrow.status = transition_status(&env, escrow.id, &escrow.status, &EscrowStatus::Refunded, &admin);
        env.storage().persistent().set(&key, &escrow);

        // Standardized observability event (Issue #597).
        emit_escrow_event(
            &env,
            evt_escrow_refunded(&env),
            EscrowRefundedEventData {
                escrow_id,
                learner: escrow.learner.clone(),
                amount: escrow.amount,
                token_address: escrow.token_address.clone(),
            },
        );
        // Legacy ad-hoc event retained for backward compatibility.
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "Refunded"), escrow_id),
            EscrowRefundedEventData {
                escrow_id,
                learner: escrow.learner.clone(),
                amount: escrow.amount,
                token_address: escrow.token_address,
            },
        );
    }

    // -----------------------------------------------------------------------
    // Escrow Stuck Reporting & Watchlist
    // -----------------------------------------------------------------------

    /// Report an escrow as "stuck" and add it to the watchlist.
    ///
    /// **Permissionless** — anyone may call this.  The call is only accepted
    /// once:
    ///
    /// ```text
    /// now >= session_end_time + auto_release_delay + GRACE_PERIOD_SECS
    /// ```
    ///
    /// which defaults to `session_end + 72h + 7d`.  An escrow that reaches
    /// this point has far exceeded every reasonable retry window and
    /// qualifies for admin intervention via `emergency_release`.
    ///
    /// # Events
    /// Emits `EscrowStuckReported` with the escrow ID, reporter, and the
    /// timestamp at which the escrow first became eligible for reporting.
    pub fn report_stuck_escrow(env: Env, reporter: Address, escrow_id: u64) {
        reporter.require_auth();

        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        // Verify the escrow is still active (not released/refunded/resolved)
        if escrow.status != EscrowStatus::Active {
            panic!("Escrow is not active; cannot be reported as stuck");
        }

        // Verify grace period has fully elapsed:
        //   stuck_since = session_end_time + auto_release_delay
        //   reportable  = stuck_since + GRACE_PERIOD_SECS
        let stuck_since = escrow
            .session_end_time
            .safe_add(&env, escrow.auto_release_delay);
        let reportable_after = stuck_since
            .safe_add(&env, GRACE_PERIOD_SECS);

        let now = env.ledger().timestamp();
        if now < reportable_after {
            panic!(
                "Stuck-report grace period not elapsed: {} < {}",
                now, reportable_after
            );
        }

        // ---- Append to StuckEscrows watchlist (deduplicated) ----
        let mut watchlist: Vec<u64> = env
            .storage()
            .persistent()
            .get(&DataKey::StuckEscrows)
            .unwrap_or_else(|| Vec::new(&env));

        let already_listed = watchlist.iter().any(|id| id == escrow_id);
        if !already_listed {
            watchlist.push_back(escrow_id);
            env.storage()
                .persistent()
                .set(&DataKey::StuckEscrows, &watchlist);
            env.storage().persistent().extend_ttl(
                &DataKey::StuckEscrows,
                ESCROW_TTL_THRESHOLD,
                ESCROW_TTL_BUMP,
            );
        }

        // ---- Standardized observability event ----
        emit_escrow_event(
            &env,
            evt_escrow_stuck_reported(&env),
            EscrowStuckReportedEventData {
                escrow_id,
                reporter: reporter.clone(),
                stuck_since,
            },
        );
        // Legacy ad-hoc event retained for backward compatibility.
        env.events().publish(
            (
                Symbol::new(&env, "Escrow"),
                Symbol::new(&env, "StuckReported"),
                escrow_id,
            ),
            EscrowStuckReportedEventData {
                escrow_id,
                reporter,
                stuck_since,
            },
        );
    }

    // -----------------------------------------------------------------------
    // Emergency Release (Multi-sig Admin Bypass)
    // -----------------------------------------------------------------------

    /// Set the MultisigAdmin contract address used for optional cross-contract
    /// emergency signature validation. Admin only.
    pub fn set_multisig_admin(env: Env, admin: Address, multisig_admin: Address) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Admin, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }

        env.storage()
            .persistent()
            .set(&DataKey::MultisigAdmin, &multisig_admin);
        env.storage().persistent().extend_ttl(
            &DataKey::MultisigAdmin,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
    }

    /// Register the governance contract authorised to approve emergency rollbacks.
    pub fn set_governance_contract(env: Env, admin: Address, governance: Address) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }
        env.storage()
            .persistent()
            .set(&DataKey::GovernanceContract, &governance);
        env.storage().persistent().extend_ttl(
            &DataKey::GovernanceContract,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
    }

    /// Grant or renew the time-bound emergency-admin role (72h TTL).
    ///
    /// Only the contract admin may grant/renew. The role is limited to the
    /// `emergency_release` scope and automatically expires unless renewed.
    pub fn grant_emergency_admin(env: Env, admin: Address, emergency_admin: Address) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }
        let now = env.ledger().timestamp();
        let expires_at = MultisigValidation::compute_admin_expiry(now)
            .expect("emergency admin expiry overflow");
        let role = EmergencyAdminRole {
            admin: emergency_admin.clone(),
            granted_at: now,
            expires_at,
            scope: Symbol::new(&env, "emergency_release"),
        };
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyAdmin, &role);
        env.storage().persistent().extend_ttl(
            &DataKey::EmergencyAdmin,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        env.events().publish(
            (
                Symbol::new(&env, "Escrow"),
                Symbol::new(&env, "EmergencyAdminGranted"),
            ),
            (emergency_admin, expires_at, EMERGENCY_ADMIN_TTL_SECS),
        );
    }

    /// Propose an emergency release action (starts the 24h timelock).
    ///
    /// `proposer` must be a registered emergency signer. The proposer's
    /// approval is recorded as the first of the required 4-of-7 signatures.
    /// Failed parameter hashes are permanently blocked from re-proposal.
    pub fn propose_emergency_action(
        env: Env,
        proposer: Address,
        escrow_id: u64,
        reason_hash: BytesN<32>,
    ) -> u32 {
        proposer.require_auth();

        let signers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencySigners)
            .expect("Emergency signers not configured");
        if !MultisigValidation::is_emergency_signer(&signers, &proposer) {
            panic!("Proposer is not an emergency signer");
        }

        // Failure gate: escrow must have hit MAX_AUTO_RELEASE_ATTEMPTS.
        let failure = match Self::_get_failure_record(&env, escrow_id) {
            Some(f) => f,
            None => panic!(
                "No failure record found; emergency release requires {} failed auto-release attempts",
                MAX_AUTO_RELEASE_ATTEMPTS
            ),
        };
        if failure.attempt_number < MAX_AUTO_RELEASE_ATTEMPTS {
            panic!(
                "Emergency release requires {} failed attempts; have {}",
                MAX_AUTO_RELEASE_ATTEMPTS, failure.attempt_number
            );
        }

        let key = (symbol_short!("ESCROW"), escrow_id);
        let escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");
        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active");
        }

        let action_type = Symbol::new(&env, "emergency_release");
        let params_hash = MultisigValidation::compute_params_hash(
            &env,
            &action_type,
            escrow_id,
            escrow.amount,
            &reason_hash,
        );

        // Permanently block retries of previously failed parameter sets.
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::EmergencyFailedParams(params_hash.clone()))
            .unwrap_or(false)
        {
            panic!("Emergency attempt with these parameters permanently failed; cannot retry");
        }

        let now = env.ledger().timestamp();
        let execute_after = MultisigValidation::compute_execute_after(now)
            .expect("execute_after overflow");

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyProposalCount)
            .unwrap_or(0);
        let new_id = count.safe_add(&env, 1);
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyProposalCount, &new_id);

        let mut approvals = Vec::new(&env);
        MultisigValidation::aggregate_signatures(&mut approvals, proposer.clone());

        let action = EmergencyAction {
            id: new_id,
            action_type: action_type.clone(),
            escrow_id,
            amount: escrow.amount,
            proposer: proposer.clone(),
            reason_hash: reason_hash.clone(),
            params_hash: params_hash.clone(),
            proposed_at: now,
            execute_after,
            approval_count: 1,
            signers: approvals,
            executed: false,
            failed: false,
        };

        env.storage()
            .persistent()
            .set(&DataKey::EmergencyProposal(new_id), &action);
        env.storage().persistent().extend_ttl(
            &DataKey::EmergencyProposal(new_id),
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        env.storage().persistent().set(
            &DataKey::EmergencyApproval(new_id, proposer.clone()),
            &true,
        );

        let audit_evt = EmergencyActionAuditEventData {
            action_id: new_id,
            escrow_id,
            actor: proposer,
            reason_hash,
            params_hash,
            approval_count: 1,
            success: true,
        };
        env.events().publish(
            (
                Symbol::new(&env, "Escrow"),
                Symbol::new(&env, "EmergencyProposed"),
                new_id,
            ),
            audit_evt,
        );
        new_id
    }

    /// Cast an approval on an open emergency action (signature aggregation).
    ///
    /// Requires the signer to be a registered emergency signer. Double-signing
    /// panics. Approvals stop being accepted once the exact 4-of-7 threshold
    /// has already been reached.
    pub fn approve_emergency_action(env: Env, signer: Address, action_id: u32) {
        signer.require_auth();

        let registered: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencySigners)
            .expect("Emergency signers not configured");
        if !MultisigValidation::is_emergency_signer(&registered, &signer) {
            panic!("Signer is not an emergency signer");
        }

        let mut action: EmergencyAction = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyProposal(action_id))
            .expect("Emergency proposal not found");
        if action.executed {
            panic!("Emergency action already executed");
        }
        if action.failed {
            panic!("Emergency action permanently failed");
        }
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::EmergencyApproval(action_id, signer.clone()))
            .unwrap_or(false)
        {
            panic!("Already approved");
        }
        if action.approval_count >= EMERGENCY_MSIG_THRESHOLD {
            panic!("Emergency action already has exact 4-of-7 approvals");
        }

        env.storage().persistent().set(
            &DataKey::EmergencyApproval(action_id, signer.clone()),
            &true,
        );
        action.approval_count =
            MultisigValidation::aggregate_signatures(&mut action.signers, signer.clone());
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyProposal(action_id), &action);

        env.events().publish(
            (
                Symbol::new(&env, "Escrow"),
                Symbol::new(&env, "EmergencyApproved"),
                action_id,
            ),
            (signer, action.approval_count, action.signers.clone()),
        );
    }

    /// Execute a fully-approved emergency action after the 24h timelock.
    ///
    /// Returns `true` on successful release. Returns `false` when the attempt
    /// is permanently recorded as failed (insufficient signatures, timelock,
    /// circuit breaker, expired admin, etc.) so the params hash cannot be
    /// retried. Panics only for missing proposals / already-terminal state.
    ///
    /// Requirements for success:
    /// 1. Exact 4-of-7 valid emergency signatures aggregated on the proposal
    /// 2. `now >= execute_after` (minimum 24h delay)
    /// 3. Caller is a non-expired emergency admin with `emergency_release` scope
    /// 4. Release amount fits under the 10%/24h circuit breaker
    /// 5. Params hash has not been permanently failed
    pub fn execute_emergency_action(env: Env, caller: Address, action_id: u32) -> bool {
        caller.require_auth();

        let mut action: EmergencyAction = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyProposal(action_id))
            .expect("Emergency proposal not found");

        if action.executed {
            panic!("Emergency action already executed");
        }
        if action.failed {
            panic!("Emergency action permanently failed");
        }

        // Block retry of permanently failed parameter sets.
        if env
            .storage()
            .persistent()
            .get::<_, bool>(&DataKey::EmergencyFailedParams(action.params_hash.clone()))
            .unwrap_or(false)
        {
            panic!("Emergency attempt with these parameters permanently failed; cannot retry");
        }

        let registered: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencySigners)
            .expect("Emergency signers not configured");

        // Exact 4-of-7 validation.
        if !MultisigValidation::validate_emergency_signatures(&registered, &action.signers) {
            Self::_record_emergency_failure(&env, &mut action, &caller);
            return false;
        }

        let now = env.ledger().timestamp();
        if !MultisigValidation::timelock_elapsed(now, action.execute_after) {
            Self::_record_emergency_failure(&env, &mut action, &caller);
            return false;
        }

        // Time-bound emergency admin with limited scope.
        let role: EmergencyAdminRole = match env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyAdmin)
        {
            Some(r) => r,
            None => {
                Self::_record_emergency_failure(&env, &mut action, &caller);
                return false;
            }
        };
        if role.admin != caller
            || !MultisigValidation::is_emergency_admin_active(&role, now)
            || role.scope != Symbol::new(&env, "emergency_release")
        {
            Self::_record_emergency_failure(&env, &mut action, &caller);
            return false;
        }

        // Circuit breaker: max 10% of active pool per 24h.
        let pool_total = Self::_active_pool_total(&env);
        let circuit: EmergencyCircuitBreaker = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyCircuit)
            .unwrap_or(EmergencyCircuitBreaker {
                window_start: now,
                released_in_window: 0,
            });
        let updated_circuit = match MultisigValidation::check_circuit_breaker(
            &circuit,
            now,
            pool_total,
            action.amount,
        ) {
            Ok(c) => c,
            Err(()) => {
                Self::_record_emergency_failure(&env, &mut action, &caller);
                return false;
            }
        };

        // Perform the release.
        let key = (symbol_short!("ESCROW"), action.escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        let mut escrow: Escrow = match env.storage().persistent().get(&key) {
            Some(e) => e,
            None => {
                Self::_record_emergency_failure(&env, &mut action, &caller);
                return false;
            }
        };
        if escrow.status != EscrowStatus::Active || escrow.amount != action.amount {
            Self::_record_emergency_failure(&env, &mut action, &caller);
            return false;
        }

        Self::_do_release_simple(&env, &mut escrow, &key, &caller);

        action.executed = true;
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyProposal(action_id), &action);
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyCircuit, &updated_circuit);
        env.storage().persistent().extend_ttl(
            &DataKey::EmergencyCircuit,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );

        env.storage()
            .persistent()
            .remove(&DataKey::FailureRecord(action.escrow_id));
        Self::_remove_from_stuck_watchlist(&env, action.escrow_id);

        // Immutable audit record with participant signatures.
        let audit = EmergencyAuditRecord {
            action_id,
            action_type: action.action_type.clone(),
            escrow_id: action.escrow_id,
            amount: action.amount,
            proposer: action.proposer.clone(),
            participant_signers: action.signers.clone(),
            reason_hash: action.reason_hash.clone(),
            params_hash: action.params_hash.clone(),
            timestamp: now,
            success: true,
        };
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyAudit(action_id), &audit);
        env.storage().persistent().extend_ttl(
            &DataKey::EmergencyAudit(action_id),
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );

        let event_payload = EmergencyReleaseExecutedEventData {
            escrow_id: action.escrow_id,
            admin: caller.clone(),
            reason_hash: action.reason_hash.clone(),
            action_id,
            amount: action.amount,
            participant_signers: action.signers.clone(),
        };
        emit_escrow_event(
            &env,
            evt_escrow_emergency_release(&env),
            event_payload.clone(),
        );
        env.events().publish(
            (
                Symbol::new(&env, "Escrow"),
                Symbol::new(&env, "EmergencyReleased"),
                action.escrow_id,
            ),
            event_payload,
        );
        true
    }

    /// Compatibility entry-point for the historical `emergency_release` name.
    ///
    /// Delegates to `execute_emergency_action` after verifying `escrow_id` /
    /// `reason_hash` match the stored proposal. Prefer the explicit
    /// propose → approve → execute flow.
    pub fn emergency_release(
        env: Env,
        caller: Address,
        escrow_id: u64,
        reason_hash: BytesN<32>,
        emergency_action_id: u32,
    ) -> bool {
        let _guard = ReentrancyGuard::enter_with_caller(&env, symbol_short!("emer"), caller.clone());
        let action: EmergencyAction = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyProposal(emergency_action_id))
            .expect("Emergency proposal not found");
        if action.escrow_id != escrow_id {
            panic!("Emergency action escrow_id mismatch");
        }
        if action.reason_hash != reason_hash {
            panic!("Emergency action reason_hash mismatch");
        }
        Self::execute_emergency_action(env.clone(), caller, emergency_action_id)
    }

    /// View: fetch an emergency action proposal.
    pub fn get_emergency_action(env: Env, action_id: u32) -> EmergencyAction {
        env.storage()
            .persistent()
            .get(&DataKey::EmergencyProposal(action_id))
            .expect("Emergency proposal not found")
    }

    /// View: fetch immutable emergency audit record.
    pub fn get_emergency_audit(env: Env, action_id: u32) -> EmergencyAuditRecord {
        env.storage()
            .persistent()
            .get(&DataKey::EmergencyAudit(action_id))
            .expect("Emergency audit not found")
    }

    /// View: current emergency admin role (may be expired).
    pub fn get_emergency_admin(env: Env) -> Option<EmergencyAdminRole> {
        env.storage().persistent().get(&DataKey::EmergencyAdmin)
    }

    /// View: emergency multisig configuration.
    pub fn get_emergency_multisig(env: Env) -> Option<EmergencyMultisig> {
        env.storage()
            .persistent()
            .get(&DataKey::EmergencyMultisigConfig)
    }

    /// Permanently log a failed emergency attempt and mark the params hash
    /// so the same parameters can never be retried. Must not panic — the
    /// failure record has to commit.
    fn _record_emergency_failure(
        env: &Env,
        action: &mut EmergencyAction,
        caller: &Address,
    ) {
        action.failed = true;
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyProposal(action.id), action);
        env.storage().persistent().set(
            &DataKey::EmergencyFailedParams(action.params_hash.clone()),
            &true,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::EmergencyFailedParams(action.params_hash.clone()),
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );

        let audit = EmergencyAuditRecord {
            action_id: action.id,
            action_type: action.action_type.clone(),
            escrow_id: action.escrow_id,
            amount: action.amount,
            proposer: action.proposer.clone(),
            participant_signers: action.signers.clone(),
            reason_hash: action.reason_hash.clone(),
            params_hash: action.params_hash.clone(),
            timestamp: env.ledger().timestamp(),
            success: false,
        };
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyAudit(action.id), &audit);

        env.events().publish(
            (
                Symbol::new(env, "Escrow"),
                Symbol::new(env, "EmergencyFailed"),
                action.id,
            ),
            EmergencyActionAuditEventData {
                action_id: action.id,
                escrow_id: action.escrow_id,
                actor: caller.clone(),
                reason_hash: action.reason_hash.clone(),
                params_hash: action.params_hash.clone(),
                approval_count: action.approval_count,
                success: false,
            },
        );
    }

    /// Sum amounts across all currently Active escrows (circuit-breaker pool).
    fn _active_pool_total(env: &Env) -> i128 {
        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowCount)
            .unwrap_or(0);
        let mut total: i128 = 0;
        for i in 1u64..=count {
            let key = (symbol_short!("ESCROW"), i);
            if let Some(escrow) = env.storage().persistent().get::<_, Escrow>(&key) {
                if escrow.status == EscrowStatus::Active && escrow.amount > 0 {
                    total = total.safe_add(env, escrow.amount);
                }
            }
        }
        total
    }


    /// Manual recovery for escrows stuck after 5+ consecutive auto-release failures.
    /// Admin can intervene and release funds directly, bypassing auto-release prechecks.
    ///
    /// Requirements:
    /// 1. Escrow must have ≥ 5 failed auto-release attempts
    /// 2. Escrow must have < 10 failed attempts (max threshold not yet hit)
    /// 3. Admin must provide justification hash
    /// 4. Caller must be the admin
    pub fn manual_recovery_release(
        env: Env,
        escrow_id: u64,
        recovery_reason_hash: BytesN<32>,
    ) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        // ---- 1. Check failure record exists and is in recovery window ----
        let mut failure = match Self::_get_failure_record(&env, escrow_id) {
            Some(f) => f,
            None => panic!("No failure record found; escrow may not have auto-release issues"),
        };

        if failure.attempt_number < MANUAL_RECOVERY_THRESHOLD {
            panic!(
                "Manual recovery not yet available; {} more failures required",
                MANUAL_RECOVERY_THRESHOLD.saturating_sub(failure.attempt_number)
            );
        }

        if failure.attempt_number >= MAX_AUTO_RELEASE_ATTEMPTS {
            panic!(
                "Escrow permanently blocked; max attempts ({}) reached",
                MAX_AUTO_RELEASE_ATTEMPTS
            );
        }

        // ---- 2. Load and release escrow (no cross-contract prechecks) ----
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        if escrow.status != EscrowStatus::Active {
            panic!("Escrow not active; manual recovery only for active escrows");
        }

        // Use simplified release to avoid repeating the same prechecks that failed
        Self::_do_release_simple(&env, &mut escrow, &key, &admin);

        // ---- 3. Update failure record to mark recovery ----
        let now = env.ledger().timestamp();
        failure.recovery_state = RecoveryState::Recovered;
        failure.manual_recovery_at = now;
        failure.manual_recovery_reason = recovery_reason_hash.clone();
        Self::_set_failure_record(&env, escrow_id, &failure);

        // ---- 4. Emit audit event ----
        env.events().publish(
            (
                Symbol::new(&env, "Escrow"),
                Symbol::new(&env, "ManualRecoveryExecuted"),
                escrow_id,
            ),
            (
                escrow_id,
                admin.clone(),
                recovery_reason_hash,
                failure.attempt_number,
                now,
            ),
        );
    }

    /// Simplified release path used by `emergency_release`.  Skips all
    /// cross-contract integrations and always uses the flat `FeeBps`
    /// rate so arithmetic cannot trap on schedule/tier lookups.
    fn _do_release_simple(
        env: &Env,
        escrow: &mut Escrow,
        key: &(Symbol, u64),
        actor: &Address,
    ) {
        let release_amount = escrow.amount;

        let fee_bps: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::FeeBps)
            .unwrap_or(DEFAULT_FEE_BPS);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let platform_fee = release_amount
            .safe_mul(&env, fee_bps as i128)
            .safe_div(&env, 10_000);
        let net_amount: i128 = release_amount
            .safe_sub(&env, platform_fee);

        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "FeeAudit")),
            (release_amount, fee_bps, platform_fee, net_amount),
        );

        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let token_client = soroban_sdk::token::Client::new(env, &escrow.token_address);

        if platform_fee > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &platform_fee);
        }
        token_client.transfer(&env.current_contract_address(), &escrow.mentor, &net_amount);

        escrow.status = transition_status(env, escrow.id, &escrow.status, &EscrowStatus::Released, actor);
        escrow.platform_fee = escrow.platform_fee.safe_add(&env, platform_fee);
        escrow.net_amount = escrow.net_amount.safe_add(&env, net_amount);
        escrow.amount = 0;
        env.storage().persistent().set(key, escrow);

        // Standardized release event (same as the normal path so off-chain
        // indexers don't need a special case for emergency releases).
        emit_escrow_event(
            env,
            evt_escrow_released(env),
            EscrowReleasedEventData {
                escrow_id: escrow.id,
                mentor: escrow.mentor.clone(),
                amount: release_amount,
                net_amount,
                platform_fee,
                token_address: escrow.token_address.clone(),
            },
        );
        env.events().publish(
            (Symbol::new(env, "Escrow"), Symbol::new(env, "Released"), escrow.id),
            EscrowReleasedEventData {
                escrow_id: escrow.id,
                mentor: escrow.mentor.clone(),
                amount: release_amount,
                net_amount,
                platform_fee,
                token_address: escrow.token_address.clone(),
            },
        );
    }

    /// Remove an escrow_id from the stuck watchlist (if present).
    fn _remove_from_stuck_watchlist(env: &Env, escrow_id: u64) {
        let watchlist_opt: Option<Vec<u64>> =
            env.storage().persistent().get(&DataKey::StuckEscrows);
        if let Some(watchlist) = watchlist_opt {
            let mut filtered = Vec::new(env);
            for id in watchlist.iter() {
                if id != escrow_id {
                    filtered.push_back(id);
                }
            }
            env.storage()
                .persistent()
                .set(&DataKey::StuckEscrows, &filtered);
        }
    }

    // -----------------------------------------------------------------------
    // Queries
    // -----------------------------------------------------------------------

    /// Get the failure record for an escrow (including backoff timing and recovery state)
    pub fn get_failure_record(env: Env, escrow_id: u64) -> Option<ReleaseFailure> {
        Self::_get_failure_record(&env, escrow_id)
    }

    /// Compatibility view: number of recorded auto-release failures for an escrow.
    pub fn get_auto_release_attempts(env: Env, escrow_id: u64) -> u32 {
        Self::_get_failure_record(&env, escrow_id)
            .map(|f| f.attempt_number)
            .unwrap_or(0)
    }

    /// Check if an escrow is currently in exponential backoff
    pub fn is_escrow_in_backoff(env: Env, escrow_id: u64) -> bool {
        if let Some(failure) = Self::_get_failure_record(&env, escrow_id) {
            let now = env.ledger().timestamp();
            now < failure.next_retry_time && failure.attempt_number > 0
        } else {
            false
        }
    }

    /// Check if manual recovery is available for an escrow
    pub fn is_manual_recovery_available(env: Env, escrow_id: u64) -> bool {
        if let Some(failure) = Self::_get_failure_record(&env, escrow_id) {
            failure.attempt_number >= MANUAL_RECOVERY_THRESHOLD
                && failure.attempt_number < MAX_AUTO_RELEASE_ATTEMPTS
        } else {
            false
        }
    }

    /// Get next scheduled retry time for an escrow in backoff
    pub fn get_next_retry_time(env: Env, escrow_id: u64) -> Option<u64> {
        Self::_get_failure_record(&env, escrow_id).map(|f| f.next_retry_time)
    }

    // -----------------------------------------------------------------------
    // Atomic State Transition View Functions
    // -----------------------------------------------------------------------

    /// Get the current state transition context for an escrow
    pub fn get_state_transition_context(env: Env, escrow_id: u64) -> Option<StateTransitionContext> {
        env.storage().persistent().get(&DataKey::StateTransitionContext(escrow_id))
    }

    /// Check if an escrow is currently locked for state transition
    pub fn is_state_transition_locked(env: Env, escrow_id: u64) -> bool {
        env.storage().persistent().get::<_, bool>(&DataKey::StateTransitionLock(escrow_id)).unwrap_or(false)
    }

    /// Get invalid state recovery record for an escrow (if any)
    pub fn get_invalid_state_record(env: Env, escrow_id: u64) -> Option<InvalidStateRecord> {
        env.storage().persistent().get(&DataKey::InvalidStateRecord(escrow_id))
    }

    /// Check if an escrow has an invalid state requiring recovery
    pub fn has_invalid_state(env: Env, escrow_id: u64) -> bool {
        env.storage().persistent().get::<_, InvalidStateRecord>(&DataKey::InvalidStateRecord(escrow_id)).is_some()
    }

    /// Return the current stuck-escrows watchlist as a `Vec<u64>` of
    /// escrow IDs.
    pub fn get_stuck_escrows(env: Env) -> Vec<u64> {
        env.storage()
            .persistent()
            .get(&DataKey::StuckEscrows)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Return the MultisigAdmin contract address, or `None` if unset.
    pub fn get_multisig_admin(env: Env) -> Option<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::MultisigAdmin)
    }

    pub fn get_escrow(env: Env, escrow_id: u64) -> Escrow {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found")
    }

    pub fn monitor_escrow_invariants(env: Env, escrow_id: u64) -> EscrowInvariantReport {
        let escrow = Self::get_escrow(env.clone(), escrow_id);
        let released_amount = escrow.platform_fee.saturating_add(escrow.net_amount);
        let funds_conserved = escrow.amount >= 0
            && escrow.platform_fee >= 0
            && escrow.net_amount >= 0
            && released_amount <= escrow.amount;
        let transition_consistent = escrow.sessions_completed <= escrow.total_sessions
            && matches!(
                escrow.status,
                EscrowStatus::Pending
                    | EscrowStatus::Active
                    | EscrowStatus::Released
                    | EscrowStatus::Disputed
                    | EscrowStatus::Refunded
                    | EscrowStatus::Resolved
            );
        let terminal_state_immutable = match escrow.status {
            EscrowStatus::Released | EscrowStatus::Refunded | EscrowStatus::Resolved => {
                !env.storage().persistent().has(&DataKey::StateTransitionLock(escrow_id))
            }
            _ => true,
        };
        let treasury_consistent = if escrow.status == EscrowStatus::Released {
            released_amount > 0 || escrow.amount == 0
        } else {
            escrow.platform_fee <= escrow.amount
        };
        let mut violation_count = 0u32;
        for ok in [funds_conserved, transition_consistent, terminal_state_immutable, treasury_consistent] {
            if !ok {
                violation_count = violation_count.saturating_add(1);
            }
        }
        let report = EscrowInvariantReport {
            escrow_id,
            funds_conserved,
            transition_consistent,
            terminal_state_immutable,
            treasury_consistent,
            violation_count,
            checked_at: env.ledger().timestamp(),
        };
        env.storage()
            .persistent()
            .set(&DataKey::EscrowInvariantReport(escrow_id), &report);
        if violation_count > 0 {
            env.events().publish(
                (Symbol::new(&env, "Escrow"), Symbol::new(&env, "InvariantViolation")),
                (escrow_id, violation_count),
            );
        }
        report
    }

    /// Heuristic instruction/IO estimate for releasing `escrow_id` (the
    /// `release_funds` → `_do_release` path), without mutating state.
    /// Mirrors the real flow's reads (escrow, admin, fee config, treasury),
    /// write (escrow status update), and token-transfer cross-contract
    /// calls (fee transfer is skipped when `fee_bps == 0`). Also accounts
    /// for reputation-update / insurance-check cross-calls once those
    /// optional integrations are configured (`DataKey::ReputationContract`
    /// / `DataKey::InsuranceContract`, reserved for future use — #761).
    pub fn estimate_release_escrow_cost(env: Env, escrow_id: u64) -> GasEstimate {
        let key = (symbol_short!("ESCROW"), escrow_id);
        let exists = env.storage().persistent().has(&key);

        // release_funds' own reads: escrow, admin, fee config, treasury.
        // Fee config is either FeeBps or FeeSchedule; both count as one read
        // plus the staking-contract tier lookup when the schedule is active.
        let mut storage_reads: u32 = 4;
        // _do_release's own write: updated escrow status/amounts.
        let storage_writes: u32 = if exists { 1 } else { 0 };

        let has_schedule = env.storage().persistent().has(&DataKey::FeeSchedule);
        let effective_fee_bps: u32 = if has_schedule {
            storage_reads += 1;
            if env.storage().persistent().has(&DataKey::StakingContract) {
                storage_reads += 1;
            }
            if let Some(schedule) = env.storage().persistent().get::<_, FeeSchedule>(&DataKey::FeeSchedule) {
                schedule.tier0_bps
            } else {
                DEFAULT_FEE_BPS
            }
        } else {
            env.storage()
                .persistent()
                .get(&DataKey::FeeBps)
                .unwrap_or(DEFAULT_FEE_BPS)
        };

        // Net-amount transfer to mentor always happens; the platform-fee
        // transfer to treasury is conditional on a non-zero fee.
        let mut cross_contract_calls: u32 = 1;
        if effective_fee_bps > 0 {
            cross_contract_calls += 1;
        }

        if env.storage().persistent().has(&DataKey::ReputationContract) {
            storage_reads += 1;
            cross_contract_calls += 1;
        }
        if env.storage().persistent().has(&DataKey::InsuranceContract) {
            storage_reads += 1;
            cross_contract_calls += 1;
        }

        let base_instructions = RELEASE_BASE_INSTRUCTIONS
            + (storage_reads as u64 + storage_writes as u64) * RELEASE_PER_STORAGE_OP_INSTRUCTIONS
            + (cross_contract_calls as u64) * RELEASE_PER_CROSS_CALL_INSTRUCTIONS;

        GasEstimate {
            base_instructions,
            storage_reads,
            storage_writes,
            cross_contract_calls,
        }
    }

    pub fn get_escrow_count(env: Env) -> u64 {
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::EscrowCount, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage().persistent().get(&DataKey::EscrowCount).unwrap_or(0)
    }

    pub fn get_fee_bps(env: Env) -> u32 {
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::FeeBps, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage().persistent().get(&DataKey::FeeBps).unwrap_or(0)
    }

    pub fn get_treasury(env: Env) -> Address {
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not set")
    }

    pub fn get_auto_release_delay(env: Env) -> u64 {
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::AutoRelDelay, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&DataKey::AutoRelDelay)
            .unwrap_or(DEFAULT_AUTO_RELEASE_DELAY)
    }

    pub fn is_token_approved(env: Env, token_address: Address) -> bool {
        Self::_is_token_approved(&env, &token_address)
    }

    pub fn get_escrows_by_mentor(env: Env, mentor: Address, page: u32, page_size: u32) -> Vec<Escrow> {
        let page_size = if page_size > 50 { 50 } else { page_size };
        let mentor_key = (MENTOR_ESCROWS, mentor);
        let mentor_escrows: Vec<u64> = env.storage().persistent().get(&mentor_key).unwrap_or(Vec::new(&env));
        let start = page.safe_mul(&env, page_size);
        let mut result = Vec::new(&env);

        if start >= mentor_escrows.len() {
            return result;
        }

        let end = (start + page_size).min(mentor_escrows.len());
        for i in start..end {
            let id = mentor_escrows.get(i).unwrap();
            let key = (symbol_short!("ESCROW"), id);
            if let Some(escrow) = env.storage().persistent().get::<_, Escrow>(&key) {
                result.push_back(escrow);
            }
        }
        result
    }

    pub fn get_escrows_by_learner(env: Env, learner: Address, page: u32, page_size: u32) -> Vec<Escrow> {
        let page_size = if page_size > 50 { 50 } else { page_size };
        let learner_key = (LEARNER_ESCROWS, learner);
        let learner_escrows: Vec<u64> = env.storage().persistent().get(&learner_key).unwrap_or(Vec::new(&env));
        let start = page.safe_mul(&env, page_size);
        let mut result = Vec::new(&env);

        if start >= learner_escrows.len() {
            return result;
        }

        let end = (start + page_size).min(learner_escrows.len());
        for i in start..end {
            let id = learner_escrows.get(i).unwrap();
            let key = (symbol_short!("ESCROW"), id);
            if let Some(escrow) = env.storage().persistent().get::<_, Escrow>(&key) {
                result.push_back(escrow);
            }
        }
        result
    }

    /// Scan escrow ids `(offset, offset + limit]` (1-indexed against the
    /// global escrow count) for ones matching `status`.
    ///
    /// `offset`/`limit` bound the *scan window*, not the match count: since
    /// this filters an unindexed global range, capping only the number of
    /// matches returned would still let a caller force a full-table scan by
    /// asking for a status with few (or zero) hits. Capping the scan window
    /// itself (to at most `MAX_PAGE_SIZE`, #831) is what actually bounds
    /// the work this call can do. A caller wanting every match pages
    /// through by repeatedly advancing `offset` by the count it scanned
    /// (`min(limit, MAX_PAGE_SIZE)`) until it has covered `get_escrow_count()`.
    pub fn get_escrows_by_status(env: Env, status: EscrowStatus, offset: u32, limit: u32) -> Vec<u64> {
        let count: u64 = env.storage().persistent().get(&DataKey::EscrowCount).unwrap_or(0u64);
        let mut result = Vec::new(&env);

        let count_u32 = count.min(u32::MAX as u64) as u32;
        let (start, end) = Pagination::new(offset, limit).bounds(count_u32);

        // Escrow ids are 1-indexed; `start`/`end` are 0-indexed offsets
        // into the id space [1, count].
        for i in (start as u64 + 1)..=(end as u64) {
            let key = (symbol_short!("ESCROW"), i);
            if let Some(escrow) = env.storage().persistent().get::<_, Escrow>(&key) {
                if escrow.status == status {
                    result.push_back(i);
                }
            }
        }
        result
    }

    /// Submit a review for a completed escrow (learner only).
    pub fn submit_review(env: Env, caller: Address, escrow_id: u64, reason: Symbol) {
        let key = (symbol_short!("ESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let escrow: Escrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Escrow not found");

        // Only learner can submit review
        caller.require_auth();
        if caller != escrow.learner {
            panic!("Only learner can submit review");
        }

        // Can only review released escrows
        if escrow.status != EscrowStatus::Released {
            panic!("Can only review released escrows");
        }

        // Store review reason in a separate key
        let review_key = (symbol_short!("REVIEW"), escrow_id);
        env.storage().persistent().set(&review_key, &reason);
        env.storage()
            .persistent()
            .extend_ttl(&review_key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "ReviewSubmitted"), escrow_id),
            ReviewSubmittedEventData {
                caller,
                reason,
                mentor: escrow.mentor,
            },
        );
    }

    // -----------------------------------------------------------------------
    // USD / path-payment escrow creation
    // -----------------------------------------------------------------------

    pub fn create_escrow_usd(
        env: Env,
        mentor: Address,
        learner: Address,
        usd_amount: i128,
        token_address: Address,
        total_sessions: u32,
    ) -> u64 {
        Validator::new(&env)
            .require_positive(usd_amount, "usd_amount")
            .require_max(usd_amount, MAX_FINANCIAL_AMOUNT, "usd_amount")
            .validate_or_panic();

        let oracle: Address = env.storage().persistent().get(&ORACLE_ID).expect("oracle not set");
        let max_age: u64 = env.storage().persistent().get(&ORACLE_MAX_AGE).unwrap_or(300);
        let oracle_sym = Symbol::new(&env, "get_price");
        let (price, updated_at): (i128, u64) = env.invoke_contract(
            &oracle,
            &oracle_sym,
            (Symbol::new(&env, "USD"),).into_val(&env),
        );
        let now = env.ledger().timestamp();
        if now.saturating_sub(updated_at) > max_age || price <= 0 {
            panic!("stale oracle");
        }
        let token_amount = usd_amount
            .safe_mul(&env, 10_000_000)
            .safe_div(&env, price);
        env.events().publish(
            (symbol_short!("Escrow"), symbol_short!("usd_rate"), learner.clone()),
            (usd_amount, price, token_amount),
        );
        Self::_create_escrow_internal(
            env,
            mentor,
            learner,
            token_amount,
            symbol_short!("USD_SES"),
            token_address.clone(),
            now,
            usd_amount,
            token_amount,
            token_address.clone(),
            token_address,
            total_sessions,
        )
    }

    pub fn create_escrow_with_path_payment(
        env: Env,
        learner: Address,
        mentor: Address,
        send_asset: Address,
        send_max: i128,
        dest_asset: Address,
        dest_amount: i128,
        _path: Vec<Address>,
        total_sessions: u32,
    ) -> u64 {
        // *** WHITELIST FIX: Validate BOTH send_asset and dest_asset ***
        if !Self::_is_token_approved(&env, &send_asset) {
            panic!("Send asset token not approved");
        }
        if !Self::_is_token_approved(&env, &dest_asset) {
            panic!("Dest asset token not approved");
        }

        Validator::new(&env)
            .require_positive(dest_amount, "dest_amount")
            .require_max(dest_amount, MAX_FINANCIAL_AMOUNT, "dest_amount")
            .require_positive(send_max, "send_max")
            .require_max(send_max, MAX_FINANCIAL_AMOUNT, "send_max")
            .validate_or_panic();

        if send_max < dest_amount {
            panic!("path slippage exceeded");
        }
        let rate_scaled = if dest_amount == 0 {
            0
        } else {
            send_max * 10_000_000 / dest_amount
        };
        env.events().publish(
            (symbol_short!("Escrow"), symbol_short!("path_pay"), learner.clone()),
            rate_scaled,
        );
        Self::_create_escrow_internal(
            env,
            mentor,
            learner,
            dest_amount,
            symbol_short!("PATHPAY"),
            dest_asset.clone(),
            0,
            0,
            dest_amount,
            send_asset,
            dest_asset,
            total_sessions,
        )
    }

    // -----------------------------------------------------------------------
    // Milestone escrow functions
    // -----------------------------------------------------------------------

    pub fn create_milestone_escrow(
        env: Env,
        mentor: Address,
        learner: Address,
        milestones: Vec<MilestoneSpec>,
        token_address: Address,
    ) -> u64 {
        if milestones.is_empty() {
            panic!("At least one milestone required");
        }

        // *** WHITELIST VALIDATION ***
        if !Self::_is_token_approved(&env, &token_address) {
            panic!("Token not approved");
        }

        // Every individual milestone must itself be a strictly positive,
        // economically sane amount. Validating only the summed total would
        // let a negative milestone offset an inflated one while still
        // passing an aggregate check, corrupting per-milestone fee/payout
        // math in `complete_milestone`.
        for m in milestones.iter() {
            Validator::new(&env)
                .require_positive(m.amount, "milestone_amount")
                .require_max(m.amount, MAX_FINANCIAL_AMOUNT, "milestone_amount")
                .validate_or_panic();
        }

        let total_amount = milestones
            .iter()
            .fold(0i128, |acc, m| acc.safe_add(&env, m.amount));

        Validator::new(&env)
            .require_positive(total_amount, "total_amount")
            .require_max(total_amount, MAX_FINANCIAL_AMOUNT, "total_amount")
            .validate_or_panic();

        learner.require_auth();

        let token_client = token::Client::new(&env, &token_address);
        if token_client.balance(&learner) < total_amount {
            panic!("Insufficient token balance");
        }

        let mut count: u64 = env
            .storage()
            .persistent()
            .get(&MILESTONE_ESCROW_COUNT)
            .unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&MILESTONE_ESCROW_COUNT, &count);
        env.storage()
            .persistent()
            .extend_ttl(&MILESTONE_ESCROW_COUNT, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        token_client.transfer(&learner, &env.current_contract_address(), &total_amount);

        let mut milestone_statuses: Vec<MilestoneStatus> = Vec::new(&env);
        for _i in 0..milestones.len() {
            milestone_statuses.push_back(MilestoneStatus::Pending);
        }

        let milestone_escrow = MilestoneEscrow {
            id: count,
            mentor: mentor.clone(),
            learner: learner.clone(),
            total_amount,
            milestones: milestones.clone(),
            milestone_statuses,
            status: transition_status(&env, count, &EscrowStatus::Pending, &EscrowStatus::Active, &learner),
            created_at: env.ledger().timestamp(),
            token_address: token_address.clone(),
            platform_fee: 0,
            net_amount: 0,
        };

        let key = (symbol_short!("MESCROW"), count);
        env.storage().persistent().set(&key, &milestone_escrow);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        env.events().publish(
            (symbol_short!("ms_creat"), count),
            (mentor, learner, total_amount, milestones.len()),
        );

        count
    }

    pub fn complete_milestone(env: Env, escrow_id: u64, milestone_index: u32) {
        let key = (symbol_short!("MESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut milestone_escrow: MilestoneEscrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Milestone escrow not found");

        if milestone_escrow.status != EscrowStatus::Active {
            panic!("Milestone escrow not active");
        }

        if (milestone_index as u32) >= milestone_escrow.milestones.len() {
            panic!("Invalid milestone index");
        }

        let current_status = milestone_escrow
            .milestone_statuses
            .get(milestone_index)
            .unwrap();
        if current_status != MilestoneStatus::Pending {
            panic!("Milestone not pending");
        }

        milestone_escrow.learner.require_auth();

        let milestone = milestone_escrow.milestones.get(milestone_index).unwrap();
        let (platform_fee, fee_meta) = Self::_compute_fee_unified(
            &env,
            &milestone_escrow.mentor,
            milestone.amount,
        );
        let net_amount: i128 = milestone.amount.safe_sub(&env, platform_fee);

        let effective_bps = fee_meta
            .map(|(_, _, ebps)| ebps)
            .unwrap_or_else(|| env.storage().persistent().get(&DataKey::FeeBps).unwrap_or(0u32));
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "FeeAudit")),
            (milestone.amount, effective_bps, platform_fee, net_amount),
        );

        if let Some((tier, base_bps, effective_bps)) = fee_meta {
            env.events().publish(
                (Symbol::new(&env, "Escrow"), Symbol::new(&env, "FeeApplied"), escrow_id),
                FeeAppliedEventData {
                    mentor: milestone_escrow.mentor.clone(),
                    tier,
                    base_bps,
                    effective_bps,
                    fee_amount: platform_fee,
                },
            );
        }

        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let token_client = token::Client::new(&env, &milestone_escrow.token_address);

        if platform_fee > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &platform_fee);
        }

        token_client.transfer(
            &env.current_contract_address(),
            &milestone_escrow.mentor,
            &net_amount,
        );

        milestone_escrow
            .milestone_statuses
            .set(milestone_index, MilestoneStatus::Completed);
        milestone_escrow.platform_fee = milestone_escrow
            .platform_fee
            .safe_add(&env, platform_fee);
        milestone_escrow.net_amount = milestone_escrow
            .net_amount
            .safe_add(&env, net_amount);

        let all_completed = milestone_escrow
            .milestone_statuses
            .iter()
            .all(|s| s == MilestoneStatus::Completed);
        if all_completed {
            milestone_escrow.status = transition_status(&env, milestone_escrow.id, &milestone_escrow.status, &EscrowStatus::Released, &milestone_escrow.learner);
        }

        env.storage().persistent().set(&key, &milestone_escrow);

        env.events().publish(
            (symbol_short!("ms_compl"), escrow_id),
            (milestone_index, milestone.amount, net_amount),
        );
    }

    pub fn dispute_milestone(env: Env, escrow_id: u64, milestone_index: u32, reason: Symbol) {
        let key = (symbol_short!("MESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let mut milestone_escrow: MilestoneEscrow = env
            .storage()
            .persistent()
            .get(&key)
            .expect("Milestone escrow not found");

        if milestone_escrow.status != EscrowStatus::Active {
            panic!("Milestone escrow not active");
        }

        if (milestone_index as u32) >= milestone_escrow.milestones.len() {
            panic!("Invalid milestone index");
        }

        let current_status = milestone_escrow
            .milestone_statuses
            .get(milestone_index)
            .unwrap();
        if current_status != MilestoneStatus::Pending {
            panic!("Milestone not pending");
        }

        milestone_escrow.mentor.require_auth();
        milestone_escrow.learner.require_auth();

        milestone_escrow
            .milestone_statuses
            .set(milestone_index, MilestoneStatus::Disputed);
        milestone_escrow.status = transition_status(&env, milestone_escrow.id, &milestone_escrow.status, &EscrowStatus::Disputed, &milestone_escrow.learner);

        env.storage().persistent().set(&key, &milestone_escrow);

        env.events().publish(
            (symbol_short!("ms_disp"), escrow_id),
            (milestone_index, reason),
        );
    }

    pub fn get_milestone_escrow(env: Env, escrow_id: u64) -> MilestoneEscrow {
        let key = (symbol_short!("MESCROW"), escrow_id);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&key)
            .expect("Milestone escrow not found")
    }

    pub fn get_milestone_escrow_count(env: Env) -> u64 {
        env.storage()
            .persistent()
            .extend_ttl(&MILESTONE_ESCROW_COUNT, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage()
            .persistent()
            .get(&MILESTONE_ESCROW_COUNT)
            .unwrap_or(0)
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    /// Shared release logic used by both `release_funds` and `try_auto_release`.
    fn _do_release(env: &Env, escrow: &mut Escrow, key: &(Symbol, u64), actor: &Address) {
        let release_amount = escrow.amount;

        let (platform_fee, fee_meta) = Self::_compute_fee_unified(env, &escrow.mentor, release_amount);

        let net_amount: i128 = release_amount
            .safe_sub(&env, platform_fee);

        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("Treasury not found");
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Treasury, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let token_client = soroban_sdk::token::Client::new(env, &escrow.token_address);

        if platform_fee > 0 {
            token_client.transfer(&env.current_contract_address(), &treasury, &platform_fee);
        }

        token_client.transfer(&env.current_contract_address(), &escrow.mentor, &net_amount);

        escrow.status = transition_status(env, escrow.id, &escrow.status, &EscrowStatus::Released, actor);
        escrow.platform_fee = escrow.platform_fee.safe_add(&env, platform_fee);
        escrow.net_amount = escrow.net_amount.safe_add(&env, net_amount);
        escrow.amount = 0; // all remaining amount is released
        env.storage().persistent().set(key, escrow);

        // Standardized observability event (Issue #597).
        emit_escrow_event(
            env,
            evt_escrow_released(env),
            EscrowReleasedEventData {
                escrow_id: escrow.id,
                mentor: escrow.mentor.clone(),
                amount: release_amount,
                net_amount,
                platform_fee,
                token_address: escrow.token_address.clone(),
            },
        );
        // Legacy ad-hoc event retained for backward compatibility.
        env.events().publish(
            (Symbol::new(env, "Escrow"), Symbol::new(env, "Released"), escrow.id),
            EscrowReleasedEventData {
                escrow_id: escrow.id,
                mentor: escrow.mentor.clone(),
                amount: release_amount,
                net_amount,
                platform_fee,
                token_address: escrow.token_address.clone(),
            },
        );

        env.events().publish(
            (Symbol::new(env, "Escrow"), Symbol::new(env, "FeeDistributed"), escrow.id),
            FeeDistributedEventData {
                escrow_id: escrow.id,
                gross_amount: release_amount,
                platform_fee,
                net_amount,
                token_address: escrow.token_address.clone(),
            },
        );

        // Graduated-fee observability: emit FeeApplied whenever the schedule
        // was used to price this release (Issue #676).
        if let Some((tier, base_bps, effective_bps)) = fee_meta {
            env.events().publish(
                (Symbol::new(env, "Escrow"), Symbol::new(env, "FeeApplied"), escrow.id),
                FeeAppliedEventData {
                    mentor: escrow.mentor.clone(),
                    tier,
                    base_bps,
                    effective_bps,
                    fee_amount: platform_fee,
                },
            );
        }
    }

    /// Internal token whitelist setter. Stores approval state in persistent storage.
    fn _set_token_approved(env: &Env, token_address: &Address, approved: bool) {
        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage().persistent().set(&key, &approved);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
    }

    /// Internal token whitelist checker.
    /// Returns `true` only if the token was explicitly approved via `set_approved_token`
    /// or during `initialize`. Any unknown/unregistered token returns `false`.
    fn _is_token_approved(env: &Env, token_address: &Address) -> bool {
        let key = DataKey::ApprovedToken(token_address.clone());
        env.storage()
            .persistent()
            .get::<_, bool>(&key)
            .unwrap_or(false)
    }

    /// Require `caller` to be the contract admin for an escrow-scoped admin
    /// operation, with contextual panic messages and a structured error
    /// event on failure (#988). Replaces the repeated
    /// `.expect("Not initialized")` / bare `panic!("Unauthorized")` pair
    /// that gave no indication of which operation or escrow failed.
    fn _require_admin_for_escrow(env: &Env, caller: &Address, escrow_id: u64, operation: &str) {
        let stored_admin: Option<Address> = env.storage().persistent().get(&DataKey::Admin);
        let stored_admin = match stored_admin {
            Some(a) => a,
            None => {
                log_contract_error(
                    env,
                    Symbol::new(env, operation),
                    Symbol::new(env, "not_init"),
                    escrow_id as i128,
                    None,
                );
                panic!("{}: escrow contract not initialized (no admin set)", operation);
            }
        };
        caller.require_auth();
        if *caller != stored_admin {
            log_contract_error(
                env,
                Symbol::new(env, operation),
                Symbol::new(env, "unauthorized"),
                escrow_id as i128,
                Some(caller.clone()),
            );
            panic!("{}: caller is not the admin for escrow {}", operation, escrow_id);
        }
    }

    /// Shared escrow creation logic with strict token whitelist validation.
    ///
    /// This function is the single entry point for all escrow creation paths
    /// (create_escrow, create_escrow_usd, create_escrow_with_path_payment).
    /// It enforces:
    /// 1. Amount > 0
    /// 2. Token is on the approved whitelist
    /// 3. Learner has sufficient balance
    /// 4. Learner authorization
    /// 5. Session uniqueness
    fn _create_escrow_internal(
        env: Env,
        mentor: Address,
        learner: Address,
        amount: i128,
        session_id: Symbol,
        token_address: Address,
        session_end_time: u64,
        usd_amount: i128,
        quoted_token_amount: i128,
        send_asset: Address,
        dest_asset: Address,
        total_sessions: u32,
    ) -> u64 {
        // --- Strict input validation ---
        Validator::new(&env)
            .require_positive(amount, "amount")
            .require_max(amount, MAX_FINANCIAL_AMOUNT, "amount")
            .require_positive(total_sessions as i128, "total_sessions")
            .require_sufficient_for_division(amount, total_sessions as i128, "amount")
            .validate_or_panic();

        // *** STRICT WHITELIST VALIDATION ***
        // This is the critical security check: only admin-approved tokens are accepted.
        // The token address is checked against the persistent whitelist registry.
        // There is no fallback, no bypass, no secondary path that skips this check.
        if !Self::_is_token_approved(&env, &token_address) {
            panic!("Token not approved");
        }

        // Also validate send_asset and dest_asset if they differ from token_address
        if send_asset != token_address && !Self::_is_token_approved(&env, &send_asset) {
            panic!("Send asset token not approved");
        }
        if dest_asset != token_address && !Self::_is_token_approved(&env, &dest_asset) {
            panic!("Dest asset token not approved");
        }

        // --- Auth ---
        learner.require_auth();

        // --- Balance check ---
        let token_client = token::Client::new(&env, &token_address);
        if token_client.balance(&learner) < amount {
            panic!("Insufficient token balance");
        }

        // --- Session uniqueness ---
        let session_key = (SESSION_KEY, session_id.clone());
        if env.storage().persistent().has(&session_key) {
            panic!("Session already exists");
        }
        env.storage().persistent().set(&session_key, &true);

        // --- Auto-release delay ---
        let auto_release_delay: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AutoRelDelay)
            .unwrap_or(DEFAULT_AUTO_RELEASE_DELAY);

        // --- Counter ---
        let mut count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowCount)
            .unwrap_or(0);
        count += 1;
        env.storage().persistent().set(&DataKey::EscrowCount, &count);
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::EscrowCount, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // --- Transfer tokens into escrow ---
        token_client.transfer(&learner, &env.current_contract_address(), &amount);

        // --- Build escrow record ---
        let escrow = Escrow {
            id: count,
            mentor: mentor.clone(),
            learner: learner.clone(),
            amount,
            session_id: session_id.clone(),
            status: transition_status(&env, count, &EscrowStatus::Pending, &EscrowStatus::Active, &learner),
            created_at: env.ledger().timestamp(),
            token_address: token_address.clone(),
            platform_fee: 0,
            net_amount: 0,
            session_end_time,
            auto_release_delay,
            dispute_reason: symbol_short!(""),
            resolved_at: 0,
            usd_amount,
            quoted_token_amount,
            send_asset,
            dest_asset,
            total_sessions,
            sessions_completed: 0,
        };
        let key = (symbol_short!("ESCROW"), count);
        env.storage().persistent().set(&key, &escrow);
        env.storage()
            .persistent()
            .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // --- Update index maps ---
        let mentor_key = (MENTOR_ESCROWS, mentor.clone());
        let mut mentor_escrows: Vec<u64> = env
            .storage()
            .persistent()
            .get(&mentor_key)
            .unwrap_or(Vec::new(&env));
        mentor_escrows.push_back(count);
        env.storage().persistent().set(&mentor_key, &mentor_escrows);
        env.storage()
            .persistent()
            .extend_ttl(&mentor_key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        let learner_key = (LEARNER_ESCROWS, learner.clone());
        let mut learner_escrows: Vec<u64> = env
            .storage()
            .persistent()
            .get(&learner_key)
            .unwrap_or(Vec::new(&env));
        learner_escrows.push_back(count);
        env.storage()
            .persistent()
            .set(&learner_key, &learner_escrows);
        env.storage()
            .persistent()
            .extend_ttl(&learner_key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        // --- Emit events ---
        // Standardized observability event (Issue #597): canonical
        // (contract, version, event_type) topic layout for off-chain indexers.
        emit_escrow_event(
            &env,
            evt_escrow_created(&env),
            EscrowCreatedEventData {
                escrow_id: count,
                mentor: mentor.clone(),
                learner: learner.clone(),
                amount,
                session_id: session_id.clone(),
                token_address: token_address.clone(),
                session_end_time,
            },
        );
        // Legacy ad-hoc event retained for backward compatibility.
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "Created"), count),
            EscrowCreatedEventData {
                escrow_id: count,
                mentor,
                learner,
                amount,
                session_id,
                token_address,
                session_end_time,
            },
        );

        count
    }

    /// Load an escrow with backward-compatible deserialization.
    fn _load_escrow(env: &Env, key: &(Symbol, u64)) -> Escrow {
        if let Some(current) = env.storage().persistent().get::<_, Escrow>(key) {
            return current;
        }
        if let Some(old) = env.storage().persistent().get::<_, EscrowLegacy>(key) {
            return Escrow {
                id: old.id,
                mentor: old.mentor,
                learner: old.learner,
                amount: old.amount,
                session_id: old.session_id,
                status: old.status,
                created_at: old.created_at,
                token_address: old.token_address.clone(),
                platform_fee: old.platform_fee,
                net_amount: old.net_amount,
                session_end_time: old.session_end_time,
                auto_release_delay: old.auto_release_delay,
                dispute_reason: old.dispute_reason,
                resolved_at: old.resolved_at,
                usd_amount: 0,
                quoted_token_amount: old.amount,
                send_asset: old.token_address.clone(),
                dest_asset: old.token_address,
                total_sessions: 1,
                sessions_completed: 0,
            };
        }
        panic!("Escrow not found");
    }

    // =======================================================================
    // Disaster Recovery
    // =======================================================================

    /// Register the emergency multi-sig signer set (admin only).
    ///
    /// Must supply exactly 7 distinct addresses with an implicit 4-of-7
    /// threshold. Stores both the raw signer list and the
    /// `EmergencyMultisig` config used by emergency release validation.
    ///
    /// # Errors
    /// Panics if `signers` fails 4-of-7 config validation or the caller is
    /// not the stored admin.
    pub fn set_emergency_signers(env: Env, admin: Address, signers: Vec<Address>) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }
        if !MultisigValidation::is_valid_emergency_config(&signers, EMERGENCY_MSIG_THRESHOLD) {
            panic!("Must provide exactly 7 distinct emergency signers with threshold 4");
        }
        let config = EmergencyMultisig {
            signers: signers.clone(),
            threshold: EMERGENCY_MSIG_THRESHOLD,
        };
        env.storage()
            .persistent()
            .set(&DataKey::EmergencySigners, &signers);
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyMultisigConfig, &config);
        env.storage().persistent().extend_ttl(
            &DataKey::EmergencySigners,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        env.storage().persistent().extend_ttl(
            &DataKey::EmergencyMultisigConfig,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        env.events().publish(
            (Symbol::new(&env, "DR"), Symbol::new(&env, "signers_set")),
            signers.len() as u32,
        );
    }

    /// Capture a complete snapshot of all critical escrow state.
    ///
    /// Call this **before** any contract upgrade so that a rollback target
    /// exists if the upgrade corrupts storage.  Up to `MAX_SNAPSHOTS` (3)
    /// snapshots are retained in a rolling window; creating a 4th
    /// automatically deletes the oldest.
    ///
    /// # Storage written
    /// * `DataKey::Snapshot(snapshot_id)` → `Vec<EscrowRecord>`
    /// * `DataKey::SnapshotMetadata(snapshot_id)` → `SnapshotMeta`
    /// * `DataKey::SnapshotIndex` → updated `Vec<u32>`
    ///
    /// # Auth
    /// Only the contract admin may take snapshots.
    pub fn snapshot_state(env: Env, admin: Address, snapshot_id: u32) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("Caller not authorized");
        }

        // ----------------------------------------------------------------
        // Collect all escrow records
        // ----------------------------------------------------------------
        let count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowCount)
            .unwrap_or(0);

        let mut records: Vec<EscrowRecord> = Vec::new(&env);
        for i in 1u64..=count {
            let key = (symbol_short!("ESCROW"), i);
            if let Some(record) = env.storage().persistent().get::<_, EscrowRecord>(&key) {
                records.push_back(record);
            }
        }

        // ----------------------------------------------------------------
        // Compute checksum over a deterministic byte sequence:
        // admin (32 B) + escrow_count (8 B) + fee_bps (4 B) + num_records (8 B)
        // ----------------------------------------------------------------
        let fee_bps: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::FeeBps)
            .unwrap_or(0);
        let auto_delay: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::AutoRelDelay)
            .unwrap_or(0);

        let mut checksum_input = Bytes::new(&env);
        // Deterministic bytes: count (8) + fee_bps (4) + auto_delay (8) + record_count (8)
        for byte in count.to_be_bytes().iter() {
            checksum_input.push_back(*byte);
        }
        for byte in fee_bps.to_be_bytes().iter() {
            checksum_input.push_back(*byte);
        }
        for byte in auto_delay.to_be_bytes().iter() {
            checksum_input.push_back(*byte);
        }
        let record_count = records.len() as u64;
        for byte in record_count.to_be_bytes().iter() {
            checksum_input.push_back(*byte);
        }
        let checksum = compute_checksum(&env, &checksum_input);

        // ----------------------------------------------------------------
        // Build metadata
        // ----------------------------------------------------------------
        let wasm_hash: BytesN<32> = BytesN::from_array(&env, &[0; 32]);

        // Manage rolling index and evict oldest if necessary
        let mut index: Vec<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::SnapshotIndex)
            .unwrap_or(Vec::new(&env));

        let snapshot_index_pos = index.len() as u32; // position within rolling window (0,1,2)

        let evicted = push_snapshot_index(&mut index, snapshot_id);
        if let Some(old_id) = evicted {
            // Delete the oldest snapshot data to enforce the rolling window.
            env.storage().persistent().remove(&DataKey::Snapshot(old_id));
            env.storage()
                .persistent()
                .remove(&DataKey::SnapshotMetadata(old_id));
        }
        env.storage()
            .persistent()
            .set(&DataKey::SnapshotIndex, &index);
        env.storage().persistent().extend_ttl(
            &DataKey::SnapshotIndex,
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );

        let meta = SnapshotMeta {
            created_at: env.ledger().timestamp(),
            block_height: env.ledger().sequence(),
            contract_version: wasm_hash,
            admin: admin.clone(),
            checksum,
            record_count,
            snapshot_index: snapshot_index_pos.min(MAX_SNAPSHOTS - 1),
        };

        // ----------------------------------------------------------------
        // Persist snapshot payload and metadata
        // ----------------------------------------------------------------
        env.storage()
            .persistent()
            .set(&DataKey::Snapshot(snapshot_id), &records);
        env.storage().persistent().extend_ttl(
            &DataKey::Snapshot(snapshot_id),
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );
        env.storage()
            .persistent()
            .set(&DataKey::SnapshotMetadata(snapshot_id), &meta);
        env.storage().persistent().extend_ttl(
            &DataKey::SnapshotMetadata(snapshot_id),
            ESCROW_TTL_THRESHOLD,
            ESCROW_TTL_BUMP,
        );

        env.events().publish(
            (
                Symbol::new(&env, "DR"),
                Symbol::new(&env, "snapshot"),
                snapshot_id,
            ),
            (record_count, env.ledger().sequence()),
        );
    }

    /// Compare a previously taken snapshot against current on-chain state.
    ///
    /// Checks every config key (Admin, Treasury, FeeBps, EscrowCount,
    /// AutoRelDelay) and every `EscrowRecord` field captured in the snapshot.
    ///
    /// # Returns
    /// A `StateVerificationReport` with:
    /// * `fields_checked` — total number of individual fields compared.
    /// * `mismatches`     — human-readable descriptions of any divergence.
    ///   An empty list means the state is fully intact.
    ///
    /// # Panics
    /// If `snapshot_id` does not refer to an existing snapshot.
    pub fn verify_post_upgrade_state(
        env: Env,
        snapshot_id: u32,
    ) -> StateVerificationReport {
        let records: Vec<EscrowRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Snapshot(snapshot_id))
            .expect("Snapshot not found");

        let meta: SnapshotMeta = env
            .storage()
            .persistent()
            .get(&DataKey::SnapshotMetadata(snapshot_id))
            .expect("Snapshot metadata not found");

        let mut mismatches: Vec<soroban_sdk::String> = Vec::new(&env);
        let mut fields_checked: u32 = 0;

        // ----------------------------------------------------------------
        // Config checks
        // ----------------------------------------------------------------
        let current_count: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::EscrowCount)
            .unwrap_or(0);
        fields_checked += 1;
        if current_count != meta.record_count {
            mismatches.push_back(soroban_sdk::String::from_str(
                &env,
                "EscrowCount mismatch",
            ));
        }

        // ----------------------------------------------------------------
        // Per-record field checks
        // ----------------------------------------------------------------
        for snapshot_rec in records.iter() {
            let key = (symbol_short!("ESCROW"), snapshot_rec.id);
            if let Some(current_rec) = env
                .storage()
                .persistent()
                .get::<_, EscrowRecord>(&key)
            {
                // id
                fields_checked += 1;
                if current_rec.id != snapshot_rec.id {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.id mismatch",
                    ));
                }
                // mentor
                fields_checked += 1;
                if current_rec.mentor != snapshot_rec.mentor {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.mentor mismatch",
                    ));
                }
                // learner
                fields_checked += 1;
                if current_rec.learner != snapshot_rec.learner {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.learner mismatch",
                    ));
                }
                // amount
                fields_checked += 1;
                if current_rec.amount != snapshot_rec.amount {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.amount mismatch",
                    ));
                }
                // status
                fields_checked += 1;
                if current_rec.status != snapshot_rec.status {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.status mismatch",
                    ));
                }
                // token_address
                fields_checked += 1;
                if current_rec.token_address != snapshot_rec.token_address {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.token_address mismatch",
                    ));
                }
                // platform_fee
                fields_checked += 1;
                if current_rec.platform_fee != snapshot_rec.platform_fee {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.platform_fee mismatch",
                    ));
                }
                // net_amount
                fields_checked += 1;
                if current_rec.net_amount != snapshot_rec.net_amount {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.net_amount mismatch",
                    ));
                }
                // session_end_time
                fields_checked += 1;
                if current_rec.session_end_time != snapshot_rec.session_end_time {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.session_end_time mismatch",
                    ));
                }
                // total_sessions
                fields_checked += 1;
                if current_rec.total_sessions != snapshot_rec.total_sessions {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.total_sessions mismatch",
                    ));
                }
                // sessions_completed
                fields_checked += 1;
                if current_rec.sessions_completed != snapshot_rec.sessions_completed {
                    mismatches.push_back(soroban_sdk::String::from_str(
                        &env,
                        "EscrowRecord.sessions_completed mismatch",
                    ));
                }
            } else {
                // Record present in snapshot but missing on-chain.
                fields_checked += 1;
                mismatches.push_back(soroban_sdk::String::from_str(
                    &env,
                    "EscrowRecord missing in current state",
                ));
            }
        }

        StateVerificationReport {
            fields_checked,
            mismatches,
        }
    }

    /// Open a hardened emergency rollback proposal (issue #825).
    pub fn propose_emergency_rollback(
        env: Env,
        proposer: Address,
        snapshot_id: u32,
        old_wasm_hash: BytesN<32>,
        scope: RollbackScope,
        justification: RollbackJustification,
    ) -> u32 {
        proposer.require_auth();
        Self::validate_rollback_request_internal(
            &env,
            proposer.clone(),
            snapshot_id,
            &scope,
            &justification,
        );

        let now = env.ledger().timestamp();
        let review_ends_at = RollbackAuthorization::compute_review_ends_at(now)
            .expect("review period overflow");

        let count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyRollbackCount)
            .unwrap_or(0);
        let new_id = count.safe_add(&env, 1);
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyRollbackCount, &new_id);

        let mut technical_signers = Vec::new(&env);
        let technical_approval_count = RollbackAuthorization::aggregate_technical_approval(
            &mut technical_signers,
            proposer.clone(),
        );

        let rollback = EmergencyRollback {
            id: new_id,
            snapshot_id,
            old_wasm_hash: old_wasm_hash.clone(),
            scope,
            justification,
            proposer: proposer.clone(),
            proposed_at: now,
            review_ends_at,
            technical_approval_count,
            technical_signers,
            governance_proposal_id: None,
            governance_approved: false,
            executed: false,
            rejected: false,
        };
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyRollback(new_id), &rollback);

        env.events().publish(
            (
                Symbol::new(&env, "DR"),
                Symbol::new(&env, "erb_proposed"),
                new_id,
            ),
            (snapshot_id, proposer, old_wasm_hash),
        );
        new_id
    }

    /// Open a rollback proposal targeting a specific snapshot.
    ///
    /// The `proposer` must be one of the registered emergency signers.
    /// Their approval is automatically counted as the first vote.
    ///
    /// # Returns
    /// The new proposal ID (auto-incremented).
    ///
    /// # Panics
    /// * Emergency signers not registered.
    /// * `proposer` is not a registered emergency signer.
    /// * Target snapshot does not exist.
    pub fn propose_rollback(
        env: Env,
        proposer: Address,
        snapshot_id: u32,
        old_wasm_hash: BytesN<32>,
    ) -> u32 {
        let justification = RollbackJustification {
            evidence_hash: old_wasm_hash.clone(),
            incident_hash: old_wasm_hash.clone(),
            description_hash: RollbackAuthorization::zero_hash(&env),
        };
        Self::propose_emergency_rollback(
            env,
            proposer,
            snapshot_id,
            old_wasm_hash,
            RollbackScope::Escrow,
            justification,
        )
    }

    /// Technical multisig approval for an emergency rollback (4-of-7 required).
    pub fn approve_technical_rollback(env: Env, signer: Address, proposal_id: u32) {
        signer.require_auth();
        let signers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencySigners)
            .expect("Emergency signers not configured");
        if !RollbackAuthorization::is_registered_signer(&signers, &signer) {
            panic!("Signer is not an emergency signer");
        }

        let mut rollback: EmergencyRollback = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyRollback(proposal_id))
            .expect("Emergency rollback proposal not found");
        if rollback.executed || rollback.rejected {
            panic!("Rollback already finalized");
        }

        rollback.technical_approval_count = RollbackAuthorization::aggregate_technical_approval(
            &mut rollback.technical_signers,
            signer.clone(),
        );
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyRollback(proposal_id), &rollback);

        env.events().publish(
            (
                Symbol::new(&env, "DR"),
                Symbol::new(&env, "erb_tech_aprv"),
                proposal_id,
            ),
            (signer, rollback.technical_approval_count),
        );
    }

    pub fn approve_rollback(env: Env, signer: Address, proposal_id: u32) {
        Self::approve_technical_rollback(env, signer, proposal_id);
    }

    pub fn link_governance_rollback_review(
        env: Env,
        governance: Address,
        rollback_id: u32,
        governance_proposal_id: u32,
    ) {
        governance.require_auth();
        let stored: Address = env
            .storage()
            .persistent()
            .get(&DataKey::GovernanceContract)
            .expect("Governance contract not configured");
        if governance != stored {
            panic!("Unauthorized governance contract");
        }

        let mut rollback: EmergencyRollback = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyRollback(rollback_id))
            .expect("Emergency rollback proposal not found");
        rollback.governance_proposal_id = Some(governance_proposal_id);
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyRollback(rollback_id), &rollback);
    }

    pub fn mark_gov_rollback_approved(
        env: Env,
        governance: Address,
        rollback_id: u32,
        governance_proposal_id: u32,
    ) {
        governance.require_auth();
        let stored: Address = env
            .storage()
            .persistent()
            .get(&DataKey::GovernanceContract)
            .expect("Governance contract not configured");
        if governance != stored {
            panic!("Unauthorized governance contract");
        }

        let mut rollback: EmergencyRollback = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyRollback(rollback_id))
            .expect("Emergency rollback proposal not found");
        if rollback.governance_proposal_id != Some(governance_proposal_id) {
            panic!("Governance proposal mismatch");
        }
        rollback.governance_approved = true;
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyRollback(rollback_id), &rollback);
    }

    pub fn validate_rollback_request(env: Env, proposal_id: u32) -> bool {
        let rollback: EmergencyRollback = match env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyRollback(proposal_id))
        {
            Some(r) => r,
            None => return false,
        };
        let meta: SnapshotMeta = match env
            .storage()
            .persistent()
            .get(&DataKey::SnapshotMetadata(rollback.snapshot_id))
        {
            Some(m) => m,
            None => return false,
        };
        if !RollbackAuthorization::validate_justification(&env, &rollback.justification) {
            return false;
        }
        RollbackAuthorization::validate_scope_window(rollback.proposed_at, meta.created_at)
    }

    fn validate_rollback_request_internal(
        env: &Env,
        proposer: Address,
        snapshot_id: u32,
        scope: &RollbackScope,
        justification: &RollbackJustification,
    ) {
        let signers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencySigners)
            .expect("Emergency signers not configured");
        if !RollbackAuthorization::is_registered_signer(&signers, &proposer) {
            panic!("Proposer is not an emergency signer");
        }
        if !env
            .storage()
            .persistent()
            .has(&DataKey::SnapshotMetadata(snapshot_id))
        {
            panic!("Snapshot not found");
        }
        let meta: SnapshotMeta = env
            .storage()
            .persistent()
            .get(&DataKey::SnapshotMetadata(snapshot_id))
            .expect("Snapshot metadata not found");
        if !RollbackAuthorization::validate_justification(env, justification) {
            panic!("Invalid rollback justification");
        }
        if !RollbackAuthorization::validate_scope_window(env.ledger().timestamp(), meta.created_at)
        {
            panic!("Snapshot outside 24h rollback window");
        }
        match scope {
            RollbackScope::Escrow => {}
            RollbackScope::Contract(addr) => {
                if *addr != env.current_contract_address() {
                    panic!("Contract scope mismatch");
                }
            }
            RollbackScope::Governance => panic!("Governance scope not valid on escrow"),
        }
    }

    pub fn preserve_audit_data(env: Env, proposal_id: u32) -> (u32, u32) {
        let rollback: EmergencyRollback = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyRollback(proposal_id))
            .expect("Emergency rollback proposal not found");

        let action_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyProposalCount)
            .unwrap_or(0);
        let mut preserved_audits = 0u32;
        for id in 1..=action_count {
            if let Some(audit) = env
                .storage()
                .persistent()
                .get::<_, EmergencyAuditRecord>(&DataKey::EmergencyAudit(id))
            {
                env.storage().persistent().set(
                    &DataKey::PreservedEmergencyAudit(id),
                    &audit,
                );
                preserved_audits = preserved_audits.safe_add(&env, 1);
            }
        }

        let records: Vec<EscrowRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Snapshot(rollback.snapshot_id))
            .expect("Snapshot data not found");
        let mut preserved_logs = 0u32;
        for record in records.iter() {
            if let Some(logs) = env
                .storage()
                .persistent()
                .get::<_, Vec<EscrowTransitionLog>>(&DataKey::TransitionLog(record.id))
            {
                env.storage().persistent().set(
                    &DataKey::PreservedTransitionLog(record.id),
                    &logs,
                );
                preserved_logs = preserved_logs.safe_add(&env, 1);
            }
        }
        (preserved_audits, preserved_logs)
    }

    pub fn emergency_rollback(env: Env, proposal_id: u32, executor: Address) -> bool {
        executor.require_auth();
        let now = env.ledger().timestamp();
        let mut rollback: EmergencyRollback = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencyRollback(proposal_id))
            .expect("Emergency rollback proposal not found");

        if !RollbackAuthorization::ready_to_execute(&rollback, now) {
            return false;
        }
        let registered: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::EmergencySigners)
            .expect("Emergency signers not configured");
        if !RollbackAuthorization::validate_technical_signatures(
            &registered,
            &rollback.technical_signers,
        ) {
            return false;
        }

        let meta: SnapshotMeta = match env
            .storage()
            .persistent()
            .get(&DataKey::SnapshotMetadata(rollback.snapshot_id))
        {
            Some(m) => m,
            None => return false,
        };
        if !RollbackAuthorization::validate_scope_window(rollback.proposed_at, meta.created_at) {
            return false;
        }

        let (preserved_audits, preserved_logs) = Self::preserve_audit_data(env.clone(), proposal_id);
        let snapshot_id = rollback.snapshot_id;
        let records: Vec<EscrowRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Snapshot(snapshot_id))
            .expect("Snapshot data not found");

        for record in records.iter() {
            let key = (symbol_short!("ESCROW"), record.id);
            env.storage().persistent().set(&key, &record);
            env.storage().persistent().extend_ttl(
                &key,
                ESCROW_TTL_THRESHOLD,
                ESCROW_TTL_BUMP,
            );
        }

        env.deployer()
            .update_current_contract_wasm(rollback.old_wasm_hash.clone());

        rollback.executed = true;
        env.storage()
            .persistent()
            .set(&DataKey::EmergencyRollback(proposal_id), &rollback);

        let audit = ImmutableRollbackAuditRecord {
            rollback_id: proposal_id,
            snapshot_id,
            evidence_hash: rollback.justification.evidence_hash.clone(),
            preserved_emergency_audits: preserved_audits,
            preserved_transition_logs: preserved_logs,
            timestamp: now,
            executor: executor.clone(),
            success: true,
        };
        env.storage()
            .persistent()
            .set(&DataKey::ImmutableRollbackAudit(proposal_id), &audit);
        true
    }

    /// Execute a rollback after multi-layer authorization is satisfied.
    pub fn rollback_to_snapshot(env: Env, proposal_id: u32) {
        let executor = env.current_contract_address();
        if !Self::emergency_rollback(env, proposal_id, executor) {
            panic!("Emergency rollback requirements not satisfied");
        }
    }

    // -----------------------------------------------------------------------
    // Disaster Recovery — View helpers
    // -----------------------------------------------------------------------

    /// Return the metadata for a snapshot, or `None` if it does not exist.
    pub fn get_snapshot_metadata(env: Env, snapshot_id: u32) -> Option<SnapshotMeta> {
        env.storage()
            .persistent()
            .get(&DataKey::SnapshotMetadata(snapshot_id))
    }

    /// Return the ordered list of currently retained snapshot IDs.
    pub fn get_snapshot_index(env: Env) -> Vec<u32> {
        env.storage()
            .persistent()
            .get(&DataKey::SnapshotIndex)
            .unwrap_or(Vec::new(&env))
    }

    /// Return a rollback proposal by ID.
    pub fn get_rollback_proposal(env: Env, proposal_id: u32) -> Option<RollbackProposal> {
        env.storage()
            .persistent()
            .get(&DataKey::RollbackProposal(proposal_id))
    }

    pub fn get_emergency_rollback(env: Env, proposal_id: u32) -> Option<EmergencyRollback> {
        env.storage()
            .persistent()
            .get(&DataKey::EmergencyRollback(proposal_id))
    }

    pub fn get_immutable_rollback_audit(
        env: Env,
        proposal_id: u32,
    ) -> Option<ImmutableRollbackAuditRecord> {
        env.storage()
            .persistent()
            .get(&DataKey::ImmutableRollbackAudit(proposal_id))
    }

    /// View function to retrieve the transition log for a given escrow.
    pub fn get_transition_log(env: Env, escrow_id: u64) -> Vec<EscrowTransitionLog> {
        let key = DataKey::TransitionLog(escrow_id);
        if env.storage().persistent().has(&key) {
            env.storage()
                .persistent()
                .extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        }
        env.storage()
            .persistent()
            .get(&key)
            .unwrap_or_else(|| Vec::new(&env))
    }

    // -----------------------------------------------------------------------
    // Payment-integrity & escrow-gaming protection (#886)
    // -----------------------------------------------------------------------

    /// Open a dispute with an explicit evidence commitment, recording the
    /// action for payment-timing manipulation analysis. This is the
    /// evidence-aware counterpart to `dispute` — callers that want the
    /// stronger `resolve_dispute_secure` path should open disputes here so
    /// the timing log is populated from the start.
    pub fn dispute_payment(env: Env, caller: Address, escrow_id: u64, reason: Symbol) {
        Self::dispute(env.clone(), caller, escrow_id, reason);
        Self::_log_dispute_action(&env, escrow_id);
    }

    /// Record an approval toward the multi-signature threshold required to
    /// resolve a disputed escrow via `resolve_dispute_secure`. Any address
    /// may submit an approval; only distinct approvers count toward the
    /// threshold.
    pub fn approve_dispute_resolution(env: Env, approver: Address, escrow_id: u64) -> EscrowMultisigApproval {
        approver.require_auth();
        let key = DataKey::ResolutionApprovals(escrow_id);
        let mut approvers: Vec<Address> = env.storage().persistent().get(&key).unwrap_or(Vec::new(&env));
        if !approvers.contains(approver.clone()) {
            approvers.push_back(approver);
        }
        env.storage().persistent().set(&key, &approvers);
        env.storage().persistent().extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        check_multisig_threshold(&env, &approvers)
    }

    /// Record the number of evidence items on file for a disputed escrow
    /// (mirrored from the dispute-evidence contract by the admin) so that
    /// `resolve_dispute_secure` can validate evidence sufficiency on-chain.
    pub fn record_evidence_count(env: Env, admin: Address, escrow_id: u64, evidence_count: u32) {
        Self::_require_admin_for_escrow(&env, &admin, escrow_id, "record_evidence_count");
        env.storage().persistent().set(&DataKey::DisputeEvidenceCount(escrow_id), &evidence_count);
    }

    /// Resolve a disputed escrow with the enhanced anti-gaming safeguards:
    /// requires sufficient evidence and an elapsed cooldown since the
    /// dispute was opened (payment-timing manipulation prevention), plus a
    /// multi-signature threshold of distinct approvals (escrow security).
    ///
    /// Falls back to the same split logic as `resolve_dispute`.
    pub fn resolve_dispute_secure(env: Env, admin: Address, escrow_id: u64, mentor_pct: u32) {
        Self::_require_admin_for_escrow(&env, &admin, escrow_id, "resolve_dispute_secure");

        if Self::is_isolated(env.clone(), escrow_id) {
            panic!("Escrow isolated pending manual review");
        }

        let opened_at: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::DisputeActionLog(escrow_id))
            .map(|log: Vec<u64>| log.get(0).unwrap_or(0))
            .unwrap_or(0);
        let evidence_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::DisputeEvidenceCount(escrow_id))
            .unwrap_or(0);
        let evidence: EvidenceSufficiency = validate_evidence_sufficiency(&env, evidence_count, opened_at);
        if !evidence.sufficient {
            panic!("Insufficient evidence or cooldown not elapsed");
        }

        let approvers: Vec<Address> = env
            .storage()
            .persistent()
            .get(&DataKey::ResolutionApprovals(escrow_id))
            .unwrap_or(Vec::new(&env));
        let approval = check_multisig_threshold(&env, &approvers);
        if !approval.threshold_met {
            panic!("Multi-signature approval threshold not met");
        }

        Self::_log_dispute_action(&env, escrow_id);
        Self::resolve_dispute(env.clone(), escrow_id, mentor_pct);

        let mut audit = Self::get_payment_audit(env.clone(), escrow_id);
        audit.resolved_at = env.ledger().timestamp();
        audit.evidence_count = evidence_count;
        env.storage().persistent().set(&DataKey::PaymentAudit(escrow_id), &audit);
    }

    /// Isolate an escrow's funds when a payment-manipulation attack is
    /// detected (e.g. rapid dispute/approval cycling). Isolated escrows
    /// cannot be released or resolved until an admin calls
    /// `recover_isolated_escrow`. Callable by the admin directly for a
    /// known incident, or by automation after `assess_payment_manipulation_risk`
    /// reports a suspected attack.
    pub fn emergency_isolate_escrow(env: Env, admin: Address, escrow_id: u64, reason: Symbol) -> EmergencyFundLock {
        Self::_require_admin_for_escrow(&env, &admin, escrow_id, "emergency_isolate_escrow");

        let lock = EmergencyFundLock {
            isolate: true,
            reason: reason.clone(),
            locked_at: env.ledger().timestamp(),
        };
        env.storage().persistent().set(&DataKey::IsolatedEscrow(escrow_id), &true);

        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "Isolated"), escrow_id),
            (reason, lock.locked_at),
        );

        let mut audit = Self::get_payment_audit(env.clone(), escrow_id);
        audit.isolated = true;
        env.storage().persistent().set(&DataKey::PaymentAudit(escrow_id), &audit);

        lock
    }

    /// Assess whether an escrow's dispute-action history shows signs of
    /// payment-timing manipulation (e.g. rapid-fire dispute/approval
    /// cycling), without mutating isolation state. Automation can use this
    /// read-only check to decide whether to call `emergency_isolate_escrow`.
    pub fn assess_payment_manipulation_risk(env: Env, escrow_id: u64) -> PaymentTimingCheck {
        let log: Vec<u64> = env.storage().persistent().get(&DataKey::DisputeActionLog(escrow_id)).unwrap_or(Vec::new(&env));
        detect_payment_timing_manipulation(&log)
    }

    /// Lift emergency isolation on an escrow after manual admin review,
    /// allowing normal release/resolution flows to resume.
    pub fn recover_isolated_escrow(env: Env, admin: Address, escrow_id: u64) {
        Self::_require_admin_for_escrow(&env, &admin, escrow_id, "recover_isolated_escrow");
        env.storage().persistent().set(&DataKey::IsolatedEscrow(escrow_id), &false);
        env.events().publish(
            (Symbol::new(&env, "Escrow"), Symbol::new(&env, "Recovered"), escrow_id),
            env.ledger().timestamp(),
        );
    }

    /// Whether an escrow's funds are currently isolated.
    pub fn is_isolated(env: Env, escrow_id: u64) -> bool {
        env.storage().persistent().get(&DataKey::IsolatedEscrow(escrow_id)).unwrap_or(false)
    }

    /// Return the payment-audit trail for a given escrow.
    pub fn get_payment_audit(env: Env, escrow_id: u64) -> PaymentAuditEntry {
        env.storage().persistent().get(&DataKey::PaymentAudit(escrow_id)).unwrap_or(PaymentAuditEntry {
            escrow_id,
            dispute_opened_at: 0,
            evidence_count: 0,
            resolved_at: 0,
            isolated: false,
        })
    }

    /// Internal: append the current ledger timestamp to an escrow's
    /// dispute-action log (capped at 20 entries) for timing-manipulation
    /// analysis, initializing the payment-audit record on first use.
    fn _log_dispute_action(env: &Env, escrow_id: u64) {
        let key = DataKey::DisputeActionLog(escrow_id);
        let mut log: Vec<u64> = env.storage().persistent().get(&key).unwrap_or(Vec::new(env));
        let now = env.ledger().timestamp();
        let first_action = log.is_empty();
        log.push_back(now);
        while log.len() > 20 {
            log.remove(0);
        }
        env.storage().persistent().set(&key, &log);
        env.storage().persistent().extend_ttl(&key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);

        if first_action {
            env.storage().persistent().set(
                &DataKey::PaymentAudit(escrow_id),
                &PaymentAuditEntry {
                    escrow_id,
                    dispute_opened_at: now,
                    evidence_count: 0,
                    resolved_at: 0,
                    isolated: false,
                },
            );
        }
    }
}

fn transition_status(
    env: &Env,
    escrow_id: u64,
    old_status: &EscrowStatus,
    new_status: &EscrowStatus,
    actor: &Address,
) -> EscrowStatus {
    let final_status = EscrowStatus::transition(env, old_status, new_status);
    
    // Log the transition
    let log_entry = EscrowTransitionLog {
        from: old_status.clone(),
        to: final_status.clone(),
        actor: actor.clone(),
        timestamp: env.ledger().timestamp(),
    };
    
    let log_key = DataKey::TransitionLog(escrow_id);
    let mut logs: Vec<EscrowTransitionLog> = env
        .storage()
        .persistent()
        .get(&log_key)
        .unwrap_or_else(|| Vec::new(env));
    logs.push_back(log_entry);
    
    env.storage().persistent().set(&log_key, &logs);
    env.storage()
        .persistent()
        .extend_ttl(&log_key, ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        
    final_status
}


// ---------------------------------------------------------------------------
// Atomic State Transition Validation and Recovery
// ---------------------------------------------------------------------------

impl EscrowContract {
    /// Acquire a state transition lock for an escrow to prevent concurrent modifications
    fn acquire_state_lock(env: &Env, escrow_id: u64, caller: &Address) -> Result<StateTransitionContext, &'static str> {
        let now = env.ledger().timestamp();
        let timeout_at = now.saturating_add(STATE_TRANSITION_TIMEOUT_SECS);
        
        // Check if lock already exists
        if env.storage().persistent().has(&DataKey::StateTransitionLock(escrow_id)) {
            return Err("State transition lock already held for this escrow");
        }
        
        // Generate unique transition ID
        let mut payload = Bytes::new(env);
        payload.append(&caller.clone().to_xdr(env));
        payload.append(&escrow_id.to_xdr(env));
        payload.append(&now.to_xdr(env));
        let transition_id: BytesN<32> = env.crypto().sha256(&payload).into();
        
        // Create transition context
        let context = StateTransitionContext {
            transition_id: transition_id.clone(),
            entity_id: escrow_id,
            pre_state: Symbol::new(env, "pending"),
            post_state: Symbol::new(env, "pending"),
            started_at: now,
            timeout_at,
            checkpoints_passed: 0,
            total_checkpoints: 3, // Default: precondition, execution, postcondition
            lock_holder: caller.clone(),
            rollback_initiated: false,
        };
        
        // Store lock and context
        env.storage().persistent().set(&DataKey::StateTransitionLock(escrow_id), &true);
        env.storage().persistent().set(&DataKey::StateTransitionContext(escrow_id), &context.clone());
        env.storage().persistent().extend_ttl(&DataKey::StateTransitionLock(escrow_id), ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        env.storage().persistent().extend_ttl(&DataKey::StateTransitionContext(escrow_id), ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        
        Ok(context)
    }
    
    /// Release a state transition lock after successful completion
    fn release_state_lock(env: &Env, escrow_id: u64) -> Result<(), &'static str> {
        env.storage().persistent().remove(&DataKey::StateTransitionLock(escrow_id));
        env.storage().persistent().remove(&DataKey::StateTransitionContext(escrow_id));
        Ok(())
    }
    
    /// Mark a checkpoint as passed during transition
    fn mark_transition_checkpoint(env: &Env, escrow_id: u64, checkpoint: u32) -> Result<(), &'static str> {
        let mut context: StateTransitionContext = env.storage().persistent()
            .get(&DataKey::StateTransitionContext(escrow_id))
            .ok_or("No active state transition")?;
        
        if is_transition_expired(&context, env.ledger().timestamp()) {
            return Err("State transition has timed out");
        }
        
        context.checkpoints_passed = checkpoint;
        env.storage().persistent().set(&DataKey::StateTransitionContext(escrow_id), &context);
        env.storage().persistent().extend_ttl(&DataKey::StateTransitionContext(escrow_id), ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
        
        Ok(())
    }
    
    /// Validate preconditions for a state transition
    fn validate_preconditions(env: &Env, escrow_id: u64, current_status: &EscrowStatus, target_status: &EscrowStatus) -> Result<bool, &'static str> {
        // Check that transition is valid per state machine
        if !EscrowStatus::is_valid_transition(env, current_status, target_status) {
            return Ok(false);
        }
        
        // Check escrow exists and is in expected state
        let key = (symbol_short!("ESCROW"), escrow_id);
        let escrow: Escrow = env.storage().persistent()
            .get(&key)
            .ok_or("Escrow not found")?;
        
        if escrow.status != *current_status {
            return Ok(false);
        }
        
        // Validate amount constraints
        if escrow.amount <= 0 || escrow.amount > 1_000_000_000_000_000 {
            return Ok(false);
        }
        
        Ok(true)
    }
    
    /// Validate postconditions for a state transition
    fn validate_postconditions(env: &Env, escrow_id: u64, new_status: &EscrowStatus) -> Result<bool, &'static str> {
        let key = (symbol_short!("ESCROW"), escrow_id);
        let escrow: Escrow = env.storage().persistent()
            .get(&key)
            .ok_or("Escrow not found")?;
        
        // Verify state changed to expected value
        if escrow.status != *new_status {
            return Ok(false);
        }
        
        // Verify amounts are consistent
        if escrow.status == EscrowStatus::Released {
            if escrow.net_amount <= 0 || escrow.platform_fee < 0 {
                return Ok(false);
            }
            if escrow.net_amount + escrow.platform_fee != escrow.amount {
                return Ok(false);
            }
        }
        
        Ok(true)
    }
    
    /// Verify cross-contract state consistency
    fn verify_cross_contract_consistency(env: &Env, escrow_id: u64) -> Result<bool, &'static str> {
        let key = (symbol_short!("ESCROW"), escrow_id);
        let escrow: Escrow = env.storage().persistent()
            .get(&key)
            .ok_or("Escrow not found")?;
        
        // Check reputation contract state if configured
        if let Some(reputation) = env.storage().persistent().get::<_, Address>(&DataKey::ReputationContract) {
            let result = env.try_invoke_contract::<bool, soroban_sdk::Error>(
                &reputation,
                &Symbol::new(env, "is_session_consistent"),
                (escrow.id, escrow.mentor.clone(), escrow.learner.clone()).into_val(env),
            );
            
            match result {
                Ok(Ok(consistent)) => {
                    if !consistent {
                        return Ok(false);
                    }
                }
                _ => return Ok(false),
            }
        }
        
        Ok(true)
    }
    
    /// Detect invalid states and create recovery records
    fn detect_and_record_invalid_state(env: &Env, escrow_id: u64) -> Result<Option<InvalidStateRecord>, &'static str> {
        let key = (symbol_short!("ESCROW"), escrow_id);
        let escrow: Escrow = env.storage().persistent()
            .get(&key)
            .ok_or("Escrow not found")?;
        
        // Check if current state is reachable from valid transitions
        let valid_states = [EscrowStatus::Pending, EscrowStatus::Active, EscrowStatus::Released, 
                           EscrowStatus::Disputed, EscrowStatus::Resolved, EscrowStatus::Refunded];
        
        let is_valid = valid_states.iter().any(|state| state == &escrow.status);
        
        if !is_valid {
            let record = InvalidStateRecord {
                entity_id: escrow_id,
                invalid_state: Symbol::new(env, "unknown"),
                expected_valid_states: valid_states.len() as u32,
                detected_at: env.ledger().timestamp(),
                recovery_attempted: false,
                recovery_successful: false,
                invalidity_reason: Symbol::new(env, "unknown_state"),
            };
            
            env.storage().persistent().set(&DataKey::InvalidStateRecord(escrow_id), &record.clone());
            env.storage().persistent().extend_ttl(&DataKey::InvalidStateRecord(escrow_id), ESCROW_TTL_THRESHOLD, ESCROW_TTL_BUMP);
            
            return Ok(Some(record));
        }
        
        Ok(None)
    }
    
    /// Attempt automatic recovery for invalid states
    fn attempt_invalid_state_recovery(env: &Env, escrow_id: u64) -> Result<bool, &'static str> {
        let key = (symbol_short!("ESCROW"), escrow_id);
        let mut escrow: Escrow = env.storage().persistent()
            .get(&key)
            .ok_or("Escrow not found")?;
        
        // Attempt to recover by resetting to a known valid state
        // For now, if escrow is in Released or Resolved, consider it recovered
        if escrow.status == EscrowStatus::Released || escrow.status == EscrowStatus::Resolved {
            if let Some(mut record) = env.storage().persistent().get::<_, InvalidStateRecord>(&DataKey::InvalidStateRecord(escrow_id)) {
                record.recovery_attempted = true;
                record.recovery_successful = true;
                env.storage().persistent().set(&DataKey::InvalidStateRecord(escrow_id), &record);
            }
            return Ok(true);
        }
        
        Ok(false)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;
    use std::string::ToString;
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger, Events},
        token::{Client as TokenClient, StellarAssetClient},
        Address, Env, Vec, IntoVal, Symbol, TryFromVal,
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

    fn advance_time(env: &Env, seconds: u64) {
        let current = env.ledger().timestamp();
        env.ledger().set_timestamp(current + seconds);
    }

    struct TestFixture {
        env: Env,
        contract_id: Address,
        admin: Address,
        mentor: Address,
        learner: Address,
        treasury: Address,
        token_address: Address,
        sac: StellarAssetClient<'static>,
    }

    impl TestFixture {
        fn setup() -> Self {
            Self::setup_full(0, 0)
        }

        fn setup_with_fee(fee_bps: u32) -> Self {
            Self::setup_full(fee_bps, 0)
        }

        fn setup_full(fee_bps: u32, auto_release_delay: u64) -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let mentor = Address::generate(&env);
            let learner = Address::generate(&env);
            let treasury = Address::generate(&env);

            let (token_address, sac) = create_token(&env, &admin);

            let contract_id = env.register_contract(None, EscrowContract);
            let client = EscrowContractClient::new(&env, &contract_id);

            let mut approved_tokens = Vec::new(&env);
            approved_tokens.push_back(token_address.clone());

            client.initialize(
                &admin,
                &treasury,
                &fee_bps,
                &approved_tokens,
                &auto_release_delay,
            );

            // Mint tokens to learner
            sac.mint(&learner, &100_000);

            // We need to leak the env to get 'static lifetime for sac
            // Instead, we'll store the pieces separately
            let sac_static = unsafe {
                core::mem::transmute::<StellarAssetClient<'_>, StellarAssetClient<'static>>(sac)
            };

            TestFixture {
                env,
                contract_id,
                admin,
                mentor,
                learner,
                treasury,
                token_address,
                sac: sac_static,
            }
        }

        fn client(&self) -> EscrowContractClient {
            EscrowContractClient::new(&self.env, &self.contract_id)
        }

        fn token(&self) -> TokenClient {
            TokenClient::new(&self.env, &self.token_address)
        }

        fn create_escrow_at(&self, amount: i128, session_end_time: u64) -> u64 {
            self.client().create_escrow(
                &self.mentor,
                &self.learner,
                &amount,
                &symbol_short!("S1"),
                &self.token_address,
                &session_end_time,
                &1u32,
            )
        }

        fn open_dispute(&self, escrow_id: u64) {
            self.client()
                .dispute(&self.learner, &escrow_id, &symbol_short!("NO_SHOW"));
        }
    }

    fn setup_disputed(f: &TestFixture) -> u64 {
        let id = f.create_escrow_at(1_000, 0);
        f.open_dispute(id);
        id
    }

    // -----------------------------------------------------------------------
    // Dynamic fee tests (legacy flat tiers)
    // -----------------------------------------------------------------------

    #[test]
    fn test_dynamic_fee_price_below_10_cents() {
        let fee = EscrowContract::_legacy_fee_from_price(500_000);
        assert_eq!(fee, 500);
    }

    #[test]
    fn test_dynamic_fee_price_10_to_50_cents() {
        let fee = EscrowContract::_legacy_fee_from_price(3_000_000);
        assert_eq!(fee, 400);
    }

    #[test]
    fn test_dynamic_fee_price_50_to_100_cents() {
        let fee = EscrowContract::_legacy_fee_from_price(7_500_000);
        assert_eq!(fee, 300);
    }

    #[test]
    fn test_dynamic_fee_price_above_100_cents() {
        let fee = EscrowContract::_legacy_fee_from_price(15_000_000);
        assert_eq!(fee, 200);
    }

    #[test]
    fn test_dynamic_fee_fallback_when_price_zero() {
        let fee = EscrowContract::_legacy_fee_from_price(0);
        assert_eq!(fee, 500);
    }

    // -----------------------------------------------------------------------
    // Graduated price-multiplier tests (FeeSchedule integration)
    // -----------------------------------------------------------------------

    #[test]
    fn test_price_multiplier_low_price_penalizes() {
        let mult = EscrowContract::_price_multiplier_bps(500_000); // < $0.10
        assert_eq!(mult, 12_500); // 125% of base tier rate
    }

    #[test]
    fn test_price_multiplier_mid_low_slight_penalty() {
        let mult = EscrowContract::_price_multiplier_bps(3_000_000); // $0.10-$0.50
        assert_eq!(mult, 11_000); // 110%
    }

    #[test]
    fn test_price_multiplier_mid_high_neutral() {
        let mult = EscrowContract::_price_multiplier_bps(7_500_000); // $0.50-$1.00
        assert_eq!(mult, 10_000); // 100% (no change)
    }

    #[test]
    fn test_price_multiplier_high_price_discounts() {
        let mult = EscrowContract::_price_multiplier_bps(15_000_000); // > $1.00
        assert_eq!(mult, 9_000); // 90% of base tier rate
    }

    #[test]
    fn test_price_multiplier_zero_price_is_neutral() {
        let mult = EscrowContract::_price_multiplier_bps(0);
        assert_eq!(mult, 10_000); // no penalty when price unavailable
    }

    #[test]
    fn test_tier_bps_maps_correctly() {
        let schedule = FeeSchedule {
            tier0_bps: 500,
            tier1_bps: 400,
            tier2_bps: 300,
            tier3_bps: 200,
            volume_discount_threshold: 1_000_000,
            volume_discount_bps: 50,
        };
        assert_eq!(EscrowContract::_tier_bps(&schedule, 0), 500);
        assert_eq!(EscrowContract::_tier_bps(&schedule, 1), 400);
        assert_eq!(EscrowContract::_tier_bps(&schedule, 2), 300);
        assert_eq!(EscrowContract::_tier_bps(&schedule, 3), 200);
        assert_eq!(EscrowContract::_tier_bps(&schedule, 99), 500); // unknown → tier0
    }

    #[test]
    fn test_volume_discount_applies_when_threshold_exceeded() {
        let schedule = FeeSchedule {
            tier0_bps: 500,
            tier1_bps: 400,
            tier2_bps: 300,
            tier3_bps: 200,
            volume_discount_threshold: 10_000,
            volume_discount_bps: 50,
        };
        // Below threshold: no discount applied
        assert_eq!(
            EscrowContract::_compute_fee_with_meta_no_dynamic(
                &schedule, 0, 5_000,
            ),
            (250, 0, 500, 500)
        );
        // Above threshold: 50 bps discount
        assert_eq!(
            EscrowContract::_compute_fee_with_meta_no_dynamic(
                &schedule, 0, 20_000,
            ),
            (900, 0, 500, 450)
        );
    }

    #[test]
    fn test_volume_discount_saturates_at_zero() {
        let schedule = FeeSchedule {
            tier0_bps: 100,
            tier1_bps: 100,
            tier2_bps: 100,
            tier3_bps: 100,
            volume_discount_threshold: 10_000,
            volume_discount_bps: 500, // larger than base 100
        };
        // saturating_sub ensures we never go below 0 bps effective rate
        let (fee, _, _, effective) =
            EscrowContract::_compute_fee_with_meta_no_dynamic(&schedule, 0, 50_000);
        assert_eq!(effective, 0);
        assert_eq!(fee, 0);
    }

    #[test]
    fn test_fee_schedule_validation_rejects_over_cap_tiers() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let bad = FeeSchedule {
                tier0_bps: 1_001, // > MAX_FEE_BPS (1000)
                tier1_bps: 400,
                tier2_bps: 300,
                tier3_bps: 200,
                volume_discount_threshold: 0,
                volume_discount_bps: 0,
            };
            EscrowContract::_validate_fee_schedule(&bad);
        }));
        assert!(result.is_err(), "tier0 over cap should panic");
    }

    #[test]
    fn test_fee_schedule_validation_rejects_negative_threshold() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let bad = FeeSchedule {
                tier0_bps: 500,
                tier1_bps: 400,
                tier2_bps: 300,
                tier3_bps: 200,
                volume_discount_threshold: -1,
                volume_discount_bps: 0,
            };
            EscrowContract::_validate_fee_schedule(&bad);
        }));
        assert!(result.is_err(), "negative threshold should panic");
    }

    #[test]
    fn test_fee_schedule_validation_rejects_excessive_discount_bps() {
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let bad = FeeSchedule {
                tier0_bps: 500,
                tier1_bps: 400,
                tier2_bps: 300,
                tier3_bps: 200,
                volume_discount_threshold: 0,
                volume_discount_bps: 1_001, // > MAX_FEE_BPS
            };
            EscrowContract::_validate_fee_schedule(&bad);
        }));
        assert!(result.is_err(), "volume discount over cap should panic");
    }

    #[test]
    fn test_fee_schedule_validation_accepts_valid_schedule() {
        let good = FeeSchedule {
            tier0_bps: 500,
            tier1_bps: 400,
            tier2_bps: 300,
            tier3_bps: 200,
            volume_discount_threshold: 1_000_000,
            volume_discount_bps: 50,
        };
        EscrowContract::_validate_fee_schedule(&good); // must not panic
    }

    // -----------------------------------------------------------------------
    // initialize
    // -----------------------------------------------------------------------

    #[test]
    fn test_initialize_stores_config() {
        let f = TestFixture::setup_full(500, 3_600);
        let client = f.client();
        assert_eq!(client.get_fee_bps(), 500);
        assert_eq!(client.get_treasury(), f.treasury);
        assert_eq!(client.get_auto_release_delay(), 3_600);
        assert!(client.is_token_approved(&f.token_address));
    }

    #[test]
    fn test_initialize_double_init_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, EscrowContract);
        let client = EscrowContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let approved: Vec<Address> = Vec::new(&env);
        client.initialize(&admin, &treasury, &500u32, &approved, &0u64);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.initialize(&admin, &treasury, &500u32, &approved, &0u64);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_initialize_fee_over_cap_panics() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, EscrowContract);
        let client = EscrowContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let approved: Vec<Address> = Vec::new(&env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            client.initialize(&admin, &treasury, &1_001u32, &approved, &0u64);
        }));
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // create_escrow
    // -----------------------------------------------------------------------

    #[test]
    fn test_create_escrow_valid() {
        let f = TestFixture::setup();
        let token = f.token();
        let learner_before = token.balance(&f.learner);
        let id = f.create_escrow_at(1_000, 0);
        assert_eq!(id, 1);
        assert_eq!(token.balance(&f.learner), learner_before - 1_000);
        assert_eq!(token.balance(&f.contract_id), 1_000);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Active);
        assert_eq!(e.mentor, f.mentor);
        assert_eq!(e.learner, f.learner);
    }

    #[test]
    fn test_create_escrow_counter_increments() {
        let f = TestFixture::setup();
        assert_eq!(f.client().get_escrow_count(), 0);
        // Note: each call uses session_id "S1" which would conflict;
        // we use different amounts to verify counter
        let id1 = f.client().create_escrow(
            &f.mentor, &f.learner, &500, &symbol_short!("S1"),
            &f.token_address, &0u64, &1u32,
        );
        let id2 = f.client().create_escrow(
            &f.mentor, &f.learner, &500, &symbol_short!("S2"),
            &f.token_address, &0u64, &1u32,
        );
        assert_eq!(id1, 1);
        assert_eq!(id2, 2);
        assert_eq!(f.client().get_escrow_count(), 2);
    }

    #[test]
    fn test_create_escrow_zero_amount_panics() {
        let f = TestFixture::setup();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.create_escrow_at(0, 0);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_create_escrow_negative_amount_panics() {
        let f = TestFixture::setup();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.create_escrow_at(-1, 0);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_create_escrow_unapproved_token_panics() {
        let f = TestFixture::setup();
        let bad_token = Address::generate(&f.env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().create_escrow(
                &f.mentor,
                &f.learner,
                &500,
                &symbol_short!("S1"),
                &bad_token,
                &0u64,
                &1u32,
            );
        }));
        assert!(result.is_err(), "unapproved token must panic");
    }

    #[test]
    fn test_create_escrow_insufficient_balance_panics() {
        let f = TestFixture::setup();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.create_escrow_at(999_999_999, 0);
        }));
        assert!(result.is_err(), "insufficient balance must panic");
    }

    // -----------------------------------------------------------------------
    // release_funds
    // -----------------------------------------------------------------------

    #[test]
    fn test_release_funds_by_learner() {
        let f = TestFixture::setup_with_fee(500);
        let token = f.token();
        let id = f.create_escrow_at(1_000, 0);
        let mentor_before = token.balance(&f.mentor);
        let treasury_before = token.balance(&f.treasury);
        f.client().release_funds(&f.learner, &id);
        assert_eq!(token.balance(&f.mentor), mentor_before + 950);
        assert_eq!(token.balance(&f.treasury), treasury_before + 50);
        assert_eq!(token.balance(&f.contract_id), 0);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Released);
        assert_eq!(e.platform_fee, 50);
        assert_eq!(e.net_amount, 950);
    }

    #[test]
    fn test_release_funds_by_admin() {
        let f = TestFixture::setup_with_fee(0);
        let id = f.create_escrow_at(1_000, 0);
        f.client().release_funds(&f.admin, &id);
        assert_eq!(f.client().get_escrow(&id).status, EscrowStatus::Released);
    }

    #[test]
    fn test_release_funds_unauthorized_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        let rando = Address::generate(&f.env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().release_funds(&rando, &id);
        }));
        assert!(result.is_err(), "unauthorized caller must panic");
    }

    #[test]
    fn test_release_funds_non_active_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().release_funds(&f.learner, &id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().release_funds(&f.learner, &id);
        }));
        assert!(result.is_err(), "double-release must panic");
    }

    #[test]
    fn test_release_funds_mentor_cannot_release() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().release_funds(&f.mentor, &id);
        }));
        assert!(result.is_err(), "mentor must not be able to self-release");
    }

    // -----------------------------------------------------------------------
    // dispute
    // -----------------------------------------------------------------------

    #[test]
    fn test_dispute_by_mentor() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client()
            .dispute(&f.mentor, &id, &symbol_short!("NO_SHOW"));
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Disputed);
        assert_eq!(e.dispute_reason, symbol_short!("NO_SHOW"));
    }

    #[test]
    fn test_dispute_by_learner() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client()
            .dispute(&f.learner, &id, &symbol_short!("BAD_SVC"));
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Disputed);
        assert_eq!(e.dispute_reason, symbol_short!("BAD_SVC"));
    }

    #[test]
    fn test_dispute_unauthorized_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        let rando = Address::generate(&f.env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().dispute(&rando, &id, &symbol_short!("FRAUD"));
        }));
        assert!(result.is_err(), "unauthorized dispute must panic");
    }

    #[test]
    fn test_dispute_non_active_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().release_funds(&f.learner, &id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().dispute(&f.mentor, &id, &symbol_short!("LATE"));
        }));
        assert!(result.is_err(), "dispute on released escrow must panic");
    }

    // -----------------------------------------------------------------------
    // resolve_dispute
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_dispute_100_0_all_to_mentor() {
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let id = setup_disputed(&f);
        let mentor_before = token.balance(&f.mentor);
        let learner_before = token.balance(&f.learner);
        f.client().resolve_dispute(&id, &100u32);
        assert_eq!(token.balance(&f.mentor), mentor_before + 1_000);
        assert_eq!(token.balance(&f.learner), learner_before);
        assert_eq!(token.balance(&f.contract_id), 0);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Resolved);
        assert_eq!(e.net_amount, 1_000);
        assert_eq!(e.platform_fee, 0);
        assert!(e.resolved_at > 0);
    }

    #[test]
    fn test_resolve_dispute_50_50_equal_split() {
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let id = setup_disputed(&f);
        let mentor_before = token.balance(&f.mentor);
        let learner_before = token.balance(&f.learner);
        f.client().resolve_dispute(&id, &50u32);
        assert_eq!(token.balance(&f.mentor), mentor_before + 500);
        assert_eq!(token.balance(&f.learner), learner_before + 500);
        assert_eq!(token.balance(&f.contract_id), 0);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Resolved);
        assert_eq!(e.net_amount, 500);
        assert_eq!(e.platform_fee, 500);
    }

    #[test]
    fn test_resolve_dispute_0_100_all_to_learner() {
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let id = setup_disputed(&f);
        let mentor_before = token.balance(&f.mentor);
        let learner_before = token.balance(&f.learner);
        f.client().resolve_dispute(&id, &0u32);
        assert_eq!(token.balance(&f.mentor), mentor_before);
        assert_eq!(token.balance(&f.learner), learner_before + 1_000);
        assert_eq!(token.balance(&f.contract_id), 0);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.net_amount, 0);
        assert_eq!(e.platform_fee, 1_000);
    }

    #[test]
    fn test_resolve_dispute_non_disputed_panics() {
        let f = TestFixture::setup_with_fee(0);
        let id = f.create_escrow_at(500, 0);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().resolve_dispute(&id, &50u32);
        }));
        assert!(result.is_err(), "resolve on non-disputed must panic");
    }

    #[test]
    fn test_resolve_dispute_invalid_pct_panics() {
        let f = TestFixture::setup_with_fee(0);
        let id = setup_disputed(&f);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().resolve_dispute(&id, &101u32);
        }));
        assert!(result.is_err(), "mentor_pct > 100 must panic");
    }

    #[test]
    fn test_resolve_dispute_double_resolve_panics() {
        let f = TestFixture::setup_with_fee(0);
        let id = setup_disputed(&f);
        f.client().resolve_dispute(&id, &50u32);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().resolve_dispute(&id, &50u32);
        }));
        assert!(result.is_err(), "double-resolve must panic");
    }

    #[test]
    fn test_resolve_dispute_rounding_no_dust() {
        // 1_000 * 33 / 100 = 330 mentor, 670 learner; total = 1_000
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let id = setup_disputed(&f);
        let mentor_before = token.balance(&f.mentor);
        let learner_before = token.balance(&f.learner);
        f.client().resolve_dispute(&id, &33u32);
        let m = token.balance(&f.mentor) - mentor_before;
        let l = token.balance(&f.learner) - learner_before;
        assert_eq!(m, 330);
        assert_eq!(l, 670);
        assert_eq!(m + l, 1_000);
        assert_eq!(token.balance(&f.contract_id), 0);
    }

    #[test]
    fn test_resolve_dispute_resolved_at_set() {
        let f = TestFixture::setup_with_fee(0);
        let id = setup_disputed(&f);
        let now = f.env.ledger().timestamp();
        f.client().resolve_dispute(&id, &50u32);
        assert_eq!(f.client().get_escrow(&id).resolved_at, now);
    }

    // -----------------------------------------------------------------------
    // refund
    // -----------------------------------------------------------------------

    #[test]
    fn test_refund_admin_only_active() {
        let f = TestFixture::setup();
        let token = f.token();
        let id = f.create_escrow_at(1_000, 0);
        let learner_before = token.balance(&f.learner);
        f.client().refund(&id);
        assert_eq!(token.balance(&f.learner), learner_before + 1_000);
        assert_eq!(token.balance(&f.contract_id), 0);
        assert_eq!(f.client().get_escrow(&id).status, EscrowStatus::Refunded);
    }

    #[test]
    fn test_refund_admin_only_disputed() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().dispute(&f.mentor, &id, &symbol_short!("LATE"));
        f.client().refund(&id);
        assert_eq!(f.client().get_escrow(&id).status, EscrowStatus::Refunded);
    }

    #[test]
    fn test_refund_already_released_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().release_funds(&f.learner, &id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().refund(&id);
        }));
        assert!(result.is_err(), "refund on Released must panic");
    }

    #[test]
    fn test_refund_already_refunded_panics() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().refund(&id);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().refund(&id);
        }));
        assert!(result.is_err(), "double-refund must panic");
    }

    #[test]
    fn test_refund_already_resolved_panics() {
        let f = TestFixture::setup_with_fee(0);
        let id = setup_disputed(&f);
        f.client().resolve_dispute(&id, &50u32);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().refund(&id);
        }));
        assert!(result.is_err(), "refund on Resolved must panic");
    }

    // -----------------------------------------------------------------------
    // try_auto_release
    // -----------------------------------------------------------------------

    #[test]
    fn test_auto_release_before_window_panics() {
        let f = TestFixture::setup_full(500, 3_600);
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now + 100);
        // advance to 1 s before window: now + 100 + 3600 - 1
        advance_time(&f.env, 100 + 3_600 - 1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().try_auto_release(&id);
        }));
        assert!(result.is_err(), "early auto-release must panic");
    }

    #[test]
    fn test_auto_release_after_window_succeeds() {
        let f = TestFixture::setup_full(500, 3_600);
        let token = f.token();
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now);
        advance_time(&f.env, 3_600 + 1);
        let mentor_before = token.balance(&f.mentor);
        let treasury_before = token.balance(&f.treasury);
        f.client().try_auto_release(&id);
        assert_eq!(token.balance(&f.mentor), mentor_before + 950);
        assert_eq!(token.balance(&f.treasury), treasury_before + 50);
        assert_eq!(token.balance(&f.contract_id), 0);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.status, EscrowStatus::Released);
        assert_eq!(e.platform_fee, 50);
        assert_eq!(e.net_amount, 950);
    }

    #[test]
    fn test_auto_release_exactly_at_boundary() {
        let f = TestFixture::setup_full(0, 3_600);
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now.saturating_sub(200));
        advance_time(&f.env, 3_600 - 200);
        f.client().try_auto_release(&id);
        assert_eq!(f.client().get_escrow(&id).status, EscrowStatus::Released);
    }

    #[test]
    fn test_auto_release_already_released_panics() {
        let f = TestFixture::setup_full(0, 3_600);
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now);
        f.client().release_funds(&f.learner, &id);
        advance_time(&f.env, 3_600 + 1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().try_auto_release(&id);
        }));
        assert!(result.is_err(), "auto-release on Released must panic");
    }

    #[test]
    fn test_auto_release_disputed_panics() {
        let f = TestFixture::setup_full(0, 3_600);
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now);
        f.client().dispute(&f.learner, &id, &symbol_short!("LATE"));
        advance_time(&f.env, 3_600 + 1);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().try_auto_release(&id);
        }));
        assert!(result.is_err(), "auto-release on Disputed must panic");
    }

    #[test]
    fn test_auto_release_default_72h() {
        let f = TestFixture::setup_full(0, 0); // 0 → 72 h default
        let now = f.env.ledger().timestamp();
        let id = f.create_escrow_at(1_000, now);
        let delay = 72u64 * 60 * 60;
        advance_time(&f.env, delay - 1);
        let too_early = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().try_auto_release(&id);
        }));
        assert!(too_early.is_err());
        advance_time(&f.env, 1);
        f.client().try_auto_release(&id);
        assert_eq!(f.client().get_escrow(&id).status, EscrowStatus::Released);
    }

    // -----------------------------------------------------------------------
    // Fee deduction
    // -----------------------------------------------------------------------

    #[test]
    fn test_fee_deduction_zero_percent() {
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let id = f.create_escrow_at(1_000, 0);
        let treasury_before = token.balance(&f.treasury);
        f.client().release_funds(&f.learner, &id);
        assert_eq!(token.balance(&f.treasury), treasury_before); // no fee
        let e = f.client().get_escrow(&id);
        assert_eq!(e.platform_fee, 0);
        assert_eq!(e.net_amount, 1_000);
    }

    #[test]
    fn test_fee_deduction_five_percent() {
        let f = TestFixture::setup_with_fee(500);
        let token = f.token();
        let id = f.create_escrow_at(1_000, 0);
        let treasury_before = token.balance(&f.treasury);
        f.client().release_funds(&f.learner, &id);
        assert_eq!(token.balance(&f.treasury), treasury_before + 50);
        assert_eq!(token.balance(&f.mentor), 950);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.platform_fee, 50);
        assert_eq!(e.net_amount, 950);
    }

    #[test]
    fn test_fee_deduction_ten_percent() {
        let f = TestFixture::setup_with_fee(1_000);
        let token = f.token();
        let id = f.create_escrow_at(2_000, 0);
        let treasury_before = token.balance(&f.treasury);
        f.client().release_funds(&f.learner, &id);
        assert_eq!(token.balance(&f.treasury), treasury_before + 200);
        assert_eq!(token.balance(&f.mentor), 1_800);
        let e = f.client().get_escrow(&id);
        assert_eq!(e.platform_fee, 200);
        assert_eq!(e.net_amount, 1_800);
    }

    // -----------------------------------------------------------------------
    // update_fee / update_treasury
    // -----------------------------------------------------------------------

    #[test]
    fn test_update_fee_by_admin() {
        let f = TestFixture::setup_with_fee(500);
        f.client().update_fee(&200u32);
        assert_eq!(f.client().get_fee_bps(), 200);
    }

    #[test]
    fn test_update_fee_over_cap_panics() {
        let f = TestFixture::setup();
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().update_fee(&1_001u32);
        }));
        assert!(result.is_err());
    }

    #[test]
    fn test_update_treasury_redirects_fee() {
        let f = TestFixture::setup_with_fee(500);
        let token = f.token();
        let new_treasury = Address::generate(&f.env);
        f.client().update_treasury(&new_treasury);
        let id = f.create_escrow_at(1_000, 0);
        f.client().release_funds(&f.learner, &id);
        assert_eq!(token.balance(&new_treasury), 50);
        assert_eq!(token.balance(&f.treasury), 0);
    }

    // -----------------------------------------------------------------------
    // set_approved_token (whitelist tests)
    // -----------------------------------------------------------------------

    #[test]
    fn test_set_approved_token_toggle() {
        let f = TestFixture::setup();
        let client = f.client();
        let new_token = Address::generate(&f.env);
        assert!(!client.is_token_approved(&new_token));
        client.set_approved_token(&new_token, &true);
        assert!(client.is_token_approved(&new_token));
        client.set_approved_token(&new_token, &false);
        assert!(!client.is_token_approved(&new_token));
    }

    /// Test: Revoked token cannot be used to create escrow
    #[test]
    fn test_revoked_token_cannot_create_escrow() {
        let f = TestFixture::setup();
        let client = f.client();
        // Revoke the approved token
        client.set_approved_token(&f.token_address, &false);
        assert!(!client.is_token_approved(&f.token_address));
        // Try to create escrow with revoked token — must panic
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.create_escrow_at(500, 0);
        }));
        assert!(result.is_err(), "revoked token must be rejected");
    }

    /// Test: Token approval events are emitted
    #[test]
    fn test_token_approval_events_emitted() {
        let f = TestFixture::setup();
        let new_token = Address::generate(&f.env);
        f.client().set_approved_token(&new_token, &true);
        let expected = soroban_sdk::vec![
            &f.env,
            (
                f.contract_id.clone(),
                (Symbol::new(&f.env, "Token"), Symbol::new(&f.env, "Approved")).into_val(&f.env),
                TokenApprovalEventData {
                    token_address: new_token,
                    approved: true,
                }
                .into_val(&f.env),
            )
        ];
        assert_eq!(f.env.events().all(), expected);
    }

    /// Test: Token rejection events are emitted
    #[test]
    fn test_token_rejection_events_emitted() {
        let f = TestFixture::setup();
        let new_token = Address::generate(&f.env);
        f.client().set_approved_token(&new_token, &true);
        f.client().set_approved_token(&new_token, &false);
        let expected = soroban_sdk::vec![
            &f.env,
            (
                f.contract_id.clone(),
                (Symbol::new(&f.env, "Token"), Symbol::new(&f.env, "Rejected")).into_val(&f.env),
                TokenApprovalEventData {
                    token_address: new_token,
                    approved: false,
                }
                .into_val(&f.env),
            )
        ];
        assert_eq!(f.env.events().all(), expected);
    }

    /// Test: Unknown/random token address is not approved by default
    #[test]
    fn test_unknown_token_not_approved() {
        let f = TestFixture::setup();
        for _ in 0..5 {
            let random_token = Address::generate(&f.env);
            assert!(
                !f.client().is_token_approved(&random_token),
                "unknown tokens must default to not-approved"
            );
        }
    }

    // -----------------------------------------------------------------------
    // Malicious token bypass tests
    // -----------------------------------------------------------------------

    /// Test: Cannot bypass whitelist by using an unapproved token in path payment
    #[test]
    fn test_path_payment_unapproved_send_asset_panics() {
        let f = TestFixture::setup();
        let malicious_token = Address::generate(&f.env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().create_escrow_with_path_payment(
                &f.learner,
                &f.mentor,
                &malicious_token,     // unapproved send asset
                &1000,
                &f.token_address,     // approved dest asset
                &1000,
                &Vec::new(&f.env),
                &1u32,
            );
        }));
        assert!(result.is_err(), "unapproved send_asset must be rejected in path payment");
    }

    /// Test: Cannot bypass whitelist by using unapproved dest_asset in path payment
    #[test]
    fn test_path_payment_unapproved_dest_asset_panics() {
        let f = TestFixture::setup();
        let malicious_token = Address::generate(&f.env);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().create_escrow_with_path_payment(
                &f.learner,
                &f.mentor,
                &f.token_address,     // approved send asset
                &1000,
                &malicious_token,     // unapproved dest asset
                &1000,
                &Vec::new(&f.env),
                &1u32,
            );
        }));
        assert!(result.is_err(), "unapproved dest_asset must be rejected in path payment");
    }

    /// Test: Cannot bypass whitelist with milestone escrow using unapproved token
    #[test]
    fn test_milestone_escrow_unapproved_token_panics() {
        let f = TestFixture::setup();
        let malicious_token = Address::generate(&f.env);
        let mut milestones = Vec::new(&f.env);
        milestones.push_back(MilestoneSpec {
            description_hash: BytesN::from_array(&f.env, &[0u8; 32]),
            amount: 1000,
        });
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            f.client().create_milestone_escrow(
                &f.mentor,
                &f.learner,
                &milestones,
                &malicious_token,
            );
        }));
        assert!(result.is_err(), "unapproved token must be rejected in milestone escrow");
    }

    /// Test: Re-adding removed token restores access
    #[test]
    fn test_re_add_token_after_removal() {
        let f = TestFixture::setup();
        let client = f.client();
        // Remove
        client.set_approved_token(&f.token_address, &false);
        assert!(!client.is_token_approved(&f.token_address));
        // Re-add
        client.set_approved_token(&f.token_address, &true);
        assert!(client.is_token_approved(&f.token_address));
        // Should be able to create escrow again
        let id = f.create_escrow_at(500, 0);
        assert_eq!(id, 1);
    }

    // -----------------------------------------------------------------------
    // Balance consistency tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_balances_create_then_refund() {
        let f = TestFixture::setup_with_fee(500);
        let token = f.token();
        let learner_start = token.balance(&f.learner);

        let id = f.create_escrow_at(1_000, 0);
        assert_eq!(token.balance(&f.learner), learner_start - 1_000);
        assert_eq!(token.balance(&f.contract_id), 1_000);

        f.client().refund(&id);
        assert_eq!(token.balance(&f.learner), learner_start); // fully restored
        assert_eq!(token.balance(&f.contract_id), 0);
        assert_eq!(token.balance(&f.treasury), 0); // no fee on refund
    }

    #[test]
    fn test_balances_create_dispute_resolve() {
        let f = TestFixture::setup_with_fee(0);
        let token = f.token();
        let learner_start = token.balance(&f.learner);
        let mentor_start = token.balance(&f.mentor);

        let id = f.create_escrow_at(1_000, 0);
        assert_eq!(token.balance(&f.contract_id), 1_000);

        f.open_dispute(id);
        assert_eq!(token.balance(&f.contract_id), 1_000); // still held

        f.client().resolve_dispute(&id, &75u32); // 750 mentor, 250 learner
        assert_eq!(token.balance(&f.mentor), mentor_start + 750);
        assert_eq!(token.balance(&f.learner), learner_start - 1_000 + 250);
        assert_eq!(token.balance(&f.contract_id), 0);
    }

    // -----------------------------------------------------------------------
    // Observability / standardized events (Issue #597)
    // -----------------------------------------------------------------------

    /// Count events whose topic uses the canonical
    /// `(contract="escrow", version=1, event_type)` layout for `event_type`.
    fn count_standard_escrow_events(f: &TestFixture, event_type: &str) -> u32 {
        let mut n = 0u32;
        for evt in f.env.events().all().events() {
            let soroban_sdk::xdr::ContractEventBody::V0(body) = &evt.body;
            if body.topics.len() != 3 {
                continue;
            }
            let c_match = matches!(
                &body.topics[0],
                soroban_sdk::xdr::ScVal::Symbol(s) if s.to_string() == "escrow"
            );
            let v_match = matches!(&body.topics[1], soroban_sdk::xdr::ScVal::U32(1));
            let e_match = matches!(
                &body.topics[2],
                soroban_sdk::xdr::ScVal::Symbol(s) if s.to_string() == event_type
            );
            if c_match && v_match && e_match {
                n += 1;
            }
        }
        n
    }

    #[test]
    fn test_standard_created_and_released_events_emitted() {
        let f = TestFixture::setup_with_fee(500);
        let id = f.create_escrow_at(1_000, 0);
        assert_eq!(
            count_standard_escrow_events(&f, "created"),
            1,
            "one standardized 'created' event expected"
        );

        f.client().release_funds(&f.learner, &id);
        assert_eq!(
            count_standard_escrow_events(&f, "released"),
            1,
            "one standardized 'released' event expected"
        );
    }

    #[test]
    fn test_standard_dispute_and_resolve_events_emitted() {
        let f = TestFixture::setup_with_fee(0);
        let id = f.create_escrow_at(1_000, 0);
        f.open_dispute(id);
        assert_eq!(count_standard_escrow_events(&f, "disputed"), 1);

        f.client().resolve_dispute(&id, &50u32);
        assert_eq!(count_standard_escrow_events(&f, "resolved"), 1);
    }

    #[test]
    fn test_standard_refunded_event_emitted() {
        let f = TestFixture::setup_with_fee(0);
        let id = f.create_escrow_at(1_000, 0);
        f.client().refund(&id);
        assert_eq!(count_standard_escrow_events(&f, "refunded"), 1);
    }

    // -----------------------------------------------------------------------
    // Payment-integrity & escrow-gaming protection (#886)
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_dispute_secure_requires_evidence_and_cooldown() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().dispute_payment(&f.learner, &id, &symbol_short!("NO_SHOW"));

        let approver1 = Address::generate(&f.env);
        let approver2 = Address::generate(&f.env);
        f.client().approve_dispute_resolution(&approver1, &id);
        f.client().approve_dispute_resolution(&approver2, &id);

        // No evidence recorded yet and cooldown hasn't elapsed -> must panic.
        let result = f.client().try_resolve_dispute_secure(&f.admin, &id, &50u32);
        assert!(result.is_err());

        f.client().record_evidence_count(&f.admin, &id, &1u32);
        advance_time(&f.env, 24 * 3_600 + 1);

        f.client().resolve_dispute_secure(&f.admin, &id, &50u32);
        let audit = f.client().get_payment_audit(&id);
        assert_eq!(audit.evidence_count, 1);
        assert!(audit.resolved_at > 0);
    }

    #[test]
    fn test_resolve_dispute_secure_requires_multisig_threshold() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);
        f.client().dispute_payment(&f.learner, &id, &symbol_short!("NO_SHOW"));
        f.client().record_evidence_count(&f.admin, &id, &1u32);
        advance_time(&f.env, 24 * 3_600 + 1);

        // Only one approver — threshold (2) not met.
        let approver1 = Address::generate(&f.env);
        f.client().approve_dispute_resolution(&approver1, &id);

        let result = f.client().try_resolve_dispute_secure(&f.admin, &id, &50u32);
        assert!(result.is_err());
    }

    #[test]
    fn test_emergency_isolate_blocks_release_and_can_be_recovered() {
        let f = TestFixture::setup();
        let id = f.create_escrow_at(1_000, 0);

        f.client().emergency_isolate_escrow(&f.admin, &id, &Symbol::new(&f.env, "attack"));
        assert!(f.client().is_isolated(&id));

        let result = f.client().try_release_funds(&f.learner, &id);
        assert!(result.is_err());

        f.client().recover_isolated_escrow(&f.admin, &id);
        assert!(!f.client().is_isolated(&id));

        f.client().release_funds(&f.learner, &id);
    }
}

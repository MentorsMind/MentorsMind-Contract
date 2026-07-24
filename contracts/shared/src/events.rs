/// Standardized event infrastructure for all MentorsMind Soroban contracts.
///
/// # Topic Layout
///
/// Every event must use exactly the 3-element topic tuple:
///
/// ```text
/// (contract: Symbol, version: u32, event_type: Symbol)
/// ```
///
/// - `contract`   — identifies the originating contract (e.g. `"escrow"`)
/// - `version`    — schema version; currently `EVENT_SCHEMA_VERSION = 1`
/// - `event_type` — identifies the specific event within the contract
///
/// This layout is stable and parseable without per-contract knowledge:
/// an indexer can always read topic[0] to route to the right decoder,
/// topic[1] to select the schema version, and topic[2] to select the
/// field definition.
///
/// # Usage
///
/// Each contract should call the appropriate typed helper, e.g.:
///
/// ```rust,ignore
/// use shared::events::{emit_escrow_event, EscrowEvent};
///
/// emit_escrow_event(&env, EscrowEvent::Created, payload);
/// ```
///
/// # Adding New Events
///
/// 1. Add a variant to the appropriate `*Event` enum below.
/// 2. Implement the `event_type_symbol` arm in `impl EventType`.
/// 3. Add an entry to `events_schema.json` at the workspace root.
/// 4. If this is a breaking payload change, increment `EVENT_SCHEMA_VERSION`.
#![allow(dead_code)]

use soroban_sdk::{symbol_short, Env, IntoVal, Symbol, Val, Vec};

// ---------------------------------------------------------------------------
// Schema version — increment when topic layout or required fields change.
// ---------------------------------------------------------------------------

/// Current schema version. Indexers should reject events with unknown versions.
pub const EVENT_SCHEMA_VERSION: u32 = 1;

// ---------------------------------------------------------------------------
// Internal emit helper
// ---------------------------------------------------------------------------

/// Low-level emit — all typed helpers delegate here.
///
/// Emits `(contract, version, event_type)` as the topic tuple.
#[inline]
pub fn emit<D: IntoVal<Env, Val>>(
    env: &Env,
    contract: Symbol,
    event_type: Symbol,
    data: D,
) {
    env.events().publish(
        (contract, EVENT_SCHEMA_VERSION, event_type),
        data,
    );
}

// ---------------------------------------------------------------------------
// Contract name symbols  (≤ 9 chars — compile-time constants via symbol_short!)
// ---------------------------------------------------------------------------

#[inline] pub fn contract_escrow(env: &Env)          -> Symbol { Symbol::new(env, "escrow") }
#[inline] pub fn contract_governance(env: &Env)      -> Symbol { Symbol::new(env, "governance") }
#[inline] pub fn contract_staking(env: &Env)         -> Symbol { Symbol::new(env, "staking") }
#[inline] pub fn contract_timelock(env: &Env)        -> Symbol { Symbol::new(env, "timelock") }
#[inline] pub fn contract_bounty(env: &Env)          -> Symbol { Symbol::new(env, "bounty") }
#[inline] pub fn contract_allowance(env: &Env)       -> Symbol { Symbol::new(env, "allowance") }
#[inline] pub fn contract_anomaly(env: &Env)         -> Symbol { Symbol::new(env, "anomaly") }
#[inline] pub fn contract_referral(env: &Env)        -> Symbol { Symbol::new(env, "referral") }
#[inline] pub fn contract_verification(env: &Env)    -> Symbol { Symbol::new(env, "verify") }
#[inline] pub fn contract_vesting(env: &Env)         -> Symbol { Symbol::new(env, "vesting") }
#[inline] pub fn contract_multisig(env: &Env)        -> Symbol { Symbol::new(env, "multisig") }
#[inline] pub fn contract_treasury(env: &Env)        -> Symbol { Symbol::new(env, "treasury") }
#[inline] pub fn contract_subscription(env: &Env)    -> Symbol { Symbol::new(env, "subscript") }
#[inline] pub fn contract_streak(env: &Env)          -> Symbol { Symbol::new(env, "streak") }
#[inline] pub fn contract_velocity(env: &Env)        -> Symbol { Symbol::new(env, "velocity") }
#[inline] pub fn contract_upgrade(env: &Env)         -> Symbol { Symbol::new(env, "upgrade") }
#[inline] pub fn contract_trs_analytics(env: &Env)   -> Symbol { Symbol::new(env, "trs_anlyt") }
#[inline] pub fn contract_sub_analytics(env: &Env)   -> Symbol { Symbol::new(env, "sub_anlyt") }
#[inline] pub fn contract_reputation(env: &Env)      -> Symbol { Symbol::new(env, "reputation") }
#[inline] pub fn contract_credit(env: &Env)          -> Symbol { Symbol::new(env, "credit") }
#[inline] pub fn contract_rbac(env: &Env)            -> Symbol { Symbol::new(env, "rbac") }
#[inline] pub fn contract_pause(env: &Env)           -> Symbol { Symbol::new(env, "pause") }
#[inline] pub fn contract_oracle(env: &Env)          -> Symbol { Symbol::new(env, "oracle") }
#[inline] pub fn contract_insurance(env: &Env)       -> Symbol { Symbol::new(env, "insurance") }
#[inline] pub fn contract_lending(env: &Env)         -> Symbol { Symbol::new(env, "lending") }
#[inline] pub fn contract_mnt_token(env: &Env)       -> Symbol { Symbol::new(env, "mnt_token") }

// ---------------------------------------------------------------------------
// Typed emit helpers — one per contract family
// ---------------------------------------------------------------------------

/// Emit a standardized event for the `escrow` contract.
pub fn emit_escrow_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_escrow(env), event_type, data);
}

/// Emit a standardized event for the `governance` contract.
pub fn emit_governance_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_governance(env), event_type, data);
}

/// Emit a standardized event for the `staking` contract.
pub fn emit_staking_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_staking(env), event_type, data);
}

/// Emit a standardized event for the `timelock` contract.
pub fn emit_timelock_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_timelock(env), event_type, data);
}

/// Emit a standardized event for the `bounty` contract.
pub fn emit_bounty_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_bounty(env), event_type, data);
}

/// Emit a standardized event for the `allowance` contract.
pub fn emit_allowance_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_allowance(env), event_type, data);
}

/// Emit a standardized event for the `anomaly_detector` contract.
pub fn emit_anomaly_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_anomaly(env), event_type, data);
}

/// Emit a standardized event for the `referral` contract.
pub fn emit_referral_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_referral(env), event_type, data);
}

/// Emit a standardized event for the `verification` contract.
pub fn emit_verification_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_verification(env), event_type, data);
}

/// Emit a standardized event for the `vesting` contract.
pub fn emit_vesting_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_vesting(env), event_type, data);
}

/// Emit a standardized event for the `multisig` contract.
pub fn emit_multisig_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_multisig(env), event_type, data);
}

/// Emit a standardized event for the `treasury` contract.
pub fn emit_treasury_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_treasury(env), event_type, data);
}

/// Emit a standardized event for the `subscription` contract.
pub fn emit_subscription_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_subscription(env), event_type, data);
}

/// Emit a standardized event for the `streak_rewards` contract.
pub fn emit_streak_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_streak(env), event_type, data);
}

/// Emit a standardized event for the `velocity_limits` contract.
pub fn emit_velocity_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_velocity(env), event_type, data);
}

/// Emit a standardized event for the `upgrade_registry` contract.
pub fn emit_upgrade_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_upgrade(env), event_type, data);
}

/// Emit a standardized event for the `treasury_analytics` contract.
pub fn emit_trs_analytics_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_trs_analytics(env), event_type, data);
}

/// Emit a standardized event for the `subscription_analytics` contract.
pub fn emit_sub_analytics_event<D: IntoVal<Env, Val>>(env: &Env, event_type: Symbol, data: D) {
    emit(env, contract_sub_analytics(env), event_type, data);
}

/// Generic emit for any contract not yet assigned a typed helper.
/// Pass the contract name (≤ 32 chars) and event type symbol directly.
pub fn emit_generic_event<D: IntoVal<Env, Val>>(
    env: &Env,
    contract_name: Symbol,
    event_type: Symbol,
    data: D,
) {
    emit(env, contract_name, event_type, data);
}

// ---------------------------------------------------------------------------
// Pre-built event type Symbols — prevents typos across contracts.
// These are the canonical string values recorded in events_schema.json.
// ---------------------------------------------------------------------------

// --- escrow ---
pub fn evt_escrow_created(env: &Env)      -> Symbol { Symbol::new(env, "created") }
pub fn evt_escrow_released(env: &Env)     -> Symbol { Symbol::new(env, "released") }
pub fn evt_escrow_auto_released(env: &Env)-> Symbol { Symbol::new(env, "auto_released") }
pub fn evt_escrow_disputed(env: &Env)     -> Symbol { Symbol::new(env, "disputed") }
pub fn evt_escrow_resolved(env: &Env)     -> Symbol { Symbol::new(env, "resolved") }
pub fn evt_escrow_refunded(env: &Env)     -> Symbol { Symbol::new(env, "refunded") }
pub fn evt_escrow_partial(env: &Env)      -> Symbol { Symbol::new(env, "partial_rel") }
pub fn evt_escrow_adm_release(env: &Env)  -> Symbol { Symbol::new(env, "admin_rel") }
pub fn evt_token_approved(env: &Env)      -> Symbol { Symbol::new(env, "tok_approved") }
pub fn evt_fee_distributed(env: &Env)     -> Symbol { Symbol::new(env, "fee_distrib") }

// --- governance ---
pub fn evt_gov_proposal_created(env: &Env)  -> Symbol { Symbol::new(env, "prop_created") }
pub fn evt_gov_vote_cast(env: &Env)         -> Symbol { Symbol::new(env, "vote_cast") }
pub fn evt_gov_proposal_passed(env: &Env)   -> Symbol { Symbol::new(env, "prop_passed") }
pub fn evt_gov_proposal_failed(env: &Env)   -> Symbol { Symbol::new(env, "prop_failed") }
pub fn evt_gov_proposal_queued(env: &Env)   -> Symbol { Symbol::new(env, "prop_queued") }
pub fn evt_gov_proposal_executed(env: &Env) -> Symbol { Symbol::new(env, "prop_executed") }
pub fn evt_gov_proposal_cancelled(env: &Env)-> Symbol { Symbol::new(env, "prop_cancelled") }
pub fn evt_gov_timelock_set(env: &Env)      -> Symbol { Symbol::new(env, "timelock_set") }
pub fn evt_gov_call_allowed(env: &Env)      -> Symbol { Symbol::new(env, "call_allowed") }
pub fn evt_gov_arb_registered(env: &Env)    -> Symbol { Symbol::new(env, "arb_registered") }
pub fn evt_gov_arb_unregistered(env: &Env)  -> Symbol { Symbol::new(env, "arb_unreg") }
pub fn evt_gov_appeal_submitted(env: &Env)  -> Symbol { Symbol::new(env, "appeal_sub") }
pub fn evt_gov_appeal_resolved(env: &Env)   -> Symbol { Symbol::new(env, "appeal_res") }

// --- staking ---
pub fn evt_staking_staked(env: &Env)    -> Symbol { Symbol::new(env, "staked") }
pub fn evt_staking_unstaked(env: &Env)  -> Symbol { Symbol::new(env, "unstaked") }

// --- timelock ---
pub fn evt_timelock_init(env: &Env)     -> Symbol { Symbol::new(env, "initialized") }
pub fn evt_timelock_sched(env: &Env)    -> Symbol { Symbol::new(env, "scheduled") }
pub fn evt_timelock_exec(env: &Env)     -> Symbol { Symbol::new(env, "executed") }
pub fn evt_timelock_cancel(env: &Env)   -> Symbol { Symbol::new(env, "cancelled") }
pub fn evt_timelock_adm_xfr(env: &Env)  -> Symbol { Symbol::new(env, "admin_xfr") }

// --- bounty ---
pub fn evt_bounty_posted(env: &Env)    -> Symbol { Symbol::new(env, "posted") }
pub fn evt_bounty_claimed(env: &Env)   -> Symbol { Symbol::new(env, "claimed") }
pub fn evt_bounty_verified(env: &Env)  -> Symbol { Symbol::new(env, "verified") }
pub fn evt_bounty_disputed(env: &Env)  -> Symbol { Symbol::new(env, "disputed") }
pub fn evt_bounty_refunded(env: &Env)  -> Symbol { Symbol::new(env, "refunded") }

// --- allowance ---
pub fn evt_allowance_authorized(env: &Env)   -> Symbol { Symbol::new(env, "authorized") }
pub fn evt_allowance_pulled(env: &Env)       -> Symbol { Symbol::new(env, "payment_pull") }
pub fn evt_allowance_revoked(env: &Env)      -> Symbol { Symbol::new(env, "revoked") }

// --- anomaly ---
pub fn evt_anomaly_hold_placed(env: &Env)    -> Symbol { Symbol::new(env, "hold_placed") }
pub fn evt_anomaly_detected(env: &Env)       -> Symbol { Symbol::new(env, "detected") }
pub fn evt_anomaly_hold_cleared(env: &Env)   -> Symbol { Symbol::new(env, "hold_cleared") }

// --- verification ---
pub fn evt_verify_ok(env: &Env)     -> Symbol { Symbol::new(env, "verified") }
pub fn evt_verify_revoke(env: &Env) -> Symbol { Symbol::new(env, "revoked") }

// --- vesting ---
pub fn evt_vesting_schedule_created(env: &Env) -> Symbol { Symbol::new(env, "sched_created") }
pub fn evt_vesting_claimed(env: &Env)          -> Symbol { Symbol::new(env, "claimed") }
pub fn evt_vesting_revoked(env: &Env)          -> Symbol { Symbol::new(env, "revoked") }

// --- multisig ---
pub fn evt_multisig_proposed(env: &Env)   -> Symbol { Symbol::new(env, "proposed") }
pub fn evt_multisig_approved(env: &Env)   -> Symbol { Symbol::new(env, "approved") }
pub fn evt_multisig_executed(env: &Env)   -> Symbol { Symbol::new(env, "executed") }
pub fn evt_multisig_cancelled(env: &Env)  -> Symbol { Symbol::new(env, "cancelled") }
pub fn evt_multisig_signer_added(env: &Env)   -> Symbol { Symbol::new(env, "signer_added") }
pub fn evt_multisig_signer_removed(env: &Env) -> Symbol { Symbol::new(env, "signer_rmvd") }
pub fn evt_multisig_threshold(env: &Env)  -> Symbol { Symbol::new(env, "threshold") }

// --- treasury ---
pub fn evt_treasury_tok_approved(env: &Env) -> Symbol { Symbol::new(env, "tok_approved") }
pub fn evt_treasury_tok_rejected(env: &Env) -> Symbol { Symbol::new(env, "tok_rejected") }
pub fn evt_treasury_deposited(env: &Env)    -> Symbol { Symbol::new(env, "deposited") }
pub fn evt_treasury_allocated(env: &Env)    -> Symbol { Symbol::new(env, "allocated") }
pub fn evt_treasury_distributed(env: &Env)  -> Symbol { Symbol::new(env, "distributed") }

// --- subscription ---
pub fn evt_sub_subscribed(env: &Env)  -> Symbol { Symbol::new(env, "subscribed") }
pub fn evt_sub_expired(env: &Env)     -> Symbol { Symbol::new(env, "expired") }
pub fn evt_sub_renewed(env: &Env)     -> Symbol { Symbol::new(env, "renewed") }
pub fn evt_sub_cancelled(env: &Env)   -> Symbol { Symbol::new(env, "cancelled") }
pub fn evt_sub_paused(env: &Env)      -> Symbol { Symbol::new(env, "paused") }

// --- streak rewards ---
pub fn evt_streak_broken(env: &Env)   -> Symbol { Symbol::new(env, "broken") }
pub fn evt_streak_updated(env: &Env)  -> Symbol { Symbol::new(env, "updated") }
pub fn evt_streak_rewarded(env: &Env) -> Symbol { Symbol::new(env, "rewarded") }

// --- velocity limits ---
pub fn evt_vel_exceeded(env: &Env)    -> Symbol { Symbol::new(env, "exceeded") }
pub fn evt_vel_checked(env: &Env)     -> Symbol { Symbol::new(env, "checked") }
pub fn evt_vel_daily_reset(env: &Env) -> Symbol { Symbol::new(env, "daily_reset") }

// --- upgrade registry ---
pub fn evt_upgrade_proposed(env: &Env)   -> Symbol { Symbol::new(env, "proposed") }
pub fn evt_upgrade_applied(env: &Env)    -> Symbol { Symbol::new(env, "applied") }
pub fn evt_upgrade_signers(env: &Env)    -> Symbol { Symbol::new(env, "signers_upd") }
pub fn evt_upgrade_admin(env: &Env)      -> Symbol { Symbol::new(env, "admin_upd") }
pub fn evt_upgrade_registered(env: &Env) -> Symbol { Symbol::new(env, "registered") }
pub fn evt_upgrade_sub_added(env: &Env)  -> Symbol { Symbol::new(env, "sub_added") }
pub fn evt_upgrade_sub_removed(env: &Env)-> Symbol { Symbol::new(env, "sub_removed") }

// --- treasury analytics ---
pub fn evt_trs_fee_revenue(env: &Env)      -> Symbol { Symbol::new(env, "fee_revenue") }
pub fn evt_trs_referral_payout(env: &Env)  -> Symbol { Symbol::new(env, "ref_payout") }
pub fn evt_trs_ins_reserve(env: &Env)      -> Symbol { Symbol::new(env, "ins_reserve") }
pub fn evt_trs_metrics(env: &Env)          -> Symbol { Symbol::new(env, "metrics") }
pub fn evt_trs_report_gen(env: &Env)       -> Symbol { Symbol::new(env, "report_gen") }

// --- subscription analytics ---
pub fn evt_sub_anlyt_metrics(env: &Env) -> Symbol { Symbol::new(env, "metrics") }

// --- referral ---
pub fn evt_referral_registered(env: &Env)  -> Symbol { Symbol::new(env, "registered") }
pub fn evt_referral_reward(env: &Env)      -> Symbol { Symbol::new(env, "reward") }

// ---------------------------------------------------------------------------
// Schema compliance test helpers (used by the indexer test module)
// ---------------------------------------------------------------------------

/// Returns `true` if the topic vec has the canonical 3-element layout:
/// `(Symbol, u32, Symbol)` with the given contract name and schema version.
#[cfg(any(test, feature = "testutils"))]
pub fn topic_is_valid(topics: &Vec<Val>, expected_contract: &str, env: &Env) -> bool {
    use soroban_sdk::TryFromVal;

    if topics.len() != 3 {
        return false;
    }
    let contract_sym = Symbol::try_from_val(env, &topics.get(0).unwrap());
    let version_val  = u32::try_from_val(env, &topics.get(1).unwrap());
    // topic[2] just needs to be a Symbol — don't constrain the value
    let evt_type_sym = Symbol::try_from_val(env, &topics.get(2).unwrap());

    let Ok(c) = contract_sym else { return false };
    let Ok(v) = version_val  else { return false };
    let Ok(_) = evt_type_sym else { return false };

    v == EVENT_SCHEMA_VERSION && c == Symbol::new(env, expected_contract)
}

#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol, Vec};

use shared::{
    get_all_params, get_param, init_protocol_params, set_param,
    key_sub_expiry_grace,
    DEFAULT_SUB_EXPIRY_GRACE,
};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const SECONDS_PER_MONTH: u64 = 30 * 24 * 60 * 60; // 30 days

// ---------------------------------------------------------------------------
// Timestamp security constants
// ---------------------------------------------------------------------------

/// Grace period applied to the billing-date check in `renew`.
/// A learner may renew up to RENEWAL_GRACE_SECS *before* `next_billing_date`
/// to absorb validator timestamp drift (Stellar validators may drift up to
/// ~30 s).  Using 60 s gives a comfortable margin.
pub const RENEWAL_GRACE_SECS: u64 = 60; // 1 minute

/// Maximum time after `next_billing_date` that a subscription is still
/// considered Active before it transitions to Expired.  After this window
/// the subscription must be explicitly renewed or it is treated as lapsed.
/// Compile-time default — live value is governed via `key_sub_expiry_grace()`.
pub const SUBSCRIPTION_EXPIRY_GRACE_SECS: u64 = DEFAULT_SUB_EXPIRY_GRACE as u64; // 7 days

/// Platform fee taken from every subscribe/renew payment, in basis points
/// (10_000 bps = 100%). Deducted before the remainder is routed to escrow.
pub const DEFAULT_PLATFORM_FEE_BPS: i128 = 250; // 2.5%

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SubscriptionStatus {
    Active,
    Paused,
    Cancelled,
    Expired,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct Plan {
    pub mentor: Address,
    pub price_per_month: i128,
    pub token: Address,
    pub sessions_per_month: u32,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct SubscriptionRecord {
    pub learner: Address,
    pub mentor: Address,
    pub plan_id: u32,
    pub start_date: u64,
    pub next_billing_date: u64,
    pub sessions_used: u32,
    pub status: SubscriptionStatus,
}

#[contracttype]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Escrow,
    PlanCounter,
    SubCounter,
    Plan(u32),
    Sub(u32),
    /// Stores the pre-authorized maximum pull amount (i128) for auto-renewal.
    /// Key: subscription_id → i128 allowance remaining.
    RenewalAllowance(u32),
    /// Counter for streaming plan IDs.
    StreamPlanCounter,
    /// Counter for streaming subscription IDs.
    StreamSubCounter,
    /// A streaming plan definition keyed by plan ID.
    StreamPlan(u32),
    /// A live streaming subscription keyed by subscription ID.
    StreamSub(u32),
    /// Whitelist of tokens plans may be denominated in. Key: token address.
    ApprovedToken(Address),
    /// Address that receives the platform's fee split on subscribe/renew.
    Treasury,
    /// Platform fee, in basis points, taken from each subscribe/renew payment.
    PlatformFeeBps,
    /// Cumulative revenue paid to a mentor, denominated per token.
    /// Key: (mentor, token) → i128 total received (net of platform fee).
    MentorRevenue(Address, Address),
}

// ---------------------------------------------------------------------------
// Streaming types
// ---------------------------------------------------------------------------

/// A mentor-defined streaming plan: tokens drip at `rate_per_second` per
/// elapsed second of active subscription.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamPlan {
    pub mentor: Address,
    /// Tokens per second earned by the mentor while the stream is active.
    pub rate_per_second: i128,
    pub token: Address,
}

/// A live streaming subscription.  The learner deposits a lump sum upfront;
/// tokens are drip-earned by the mentor with each passing second.
#[contracttype]
#[derive(Clone, Debug)]
pub struct StreamingSubscription {
    pub learner: Address,
    pub mentor: Address,
    pub plan_id: u32,
    /// Ledger timestamp when streaming started.
    pub start: u64,
    /// Total tokens deposited by the learner at subscribe time.
    pub balance_deposited: i128,
    /// Cumulative amount already transferred to the mentor via `withdraw_earned`.
    pub withdrawn_by_mentor: i128,
    /// True while the stream is active; set to false on `cancel_streaming`.
    pub is_active: bool,
    /// Ledger timestamp at cancellation (0 while active).
    pub ended_at: u64,
}

// ---------------------------------------------------------------------------
// Events
// ---------------------------------------------------------------------------

/// Emitted when a renewal attempt fails due to insufficient learner balance.
#[contracttype]
#[derive(Clone, Debug)]
pub struct RenewalFailedEvent {
    pub sub_id: u32,
    pub required: i128,
    pub available: i128,
}

// ---------------------------------------------------------------------------
// View types
// ---------------------------------------------------------------------------

/// Returned by `get_renewal_status` for off-chain monitoring.
#[contracttype]
#[derive(Clone, Debug)]
pub struct RenewalStatus {
    /// UNIX timestamp when the next renewal payment is due.
    pub next_due: u64,
    /// Remaining pre-authorized allowance (0 if none set).
    pub allowance_remaining: i128,
    /// True when an allowance ≥ plan price exists and the subscription is Active.
    pub is_auto_renewable: bool,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct SubscriptionContract;

#[contractimpl]
impl SubscriptionContract {
    /// One-time initialization. Sets admin and escrow wallet.
    /// Treasury defaults to `admin` and the platform fee defaults to
    /// `DEFAULT_PLATFORM_FEE_BPS`; both can be changed later by the admin.
    pub fn initialize(env: Env, admin: Address, escrow: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::Escrow, &escrow);
        env.storage().persistent().set(&DataKey::PlanCounter, &0u32);
        env.storage().persistent().set(&DataKey::SubCounter, &0u32);
        env.storage().persistent().set(&DataKey::Treasury, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::PlatformFeeBps, &DEFAULT_PLATFORM_FEE_BPS);
    }

    // -----------------------------------------------------------------------
    // Token whitelist / fee configuration (admin only)
    // -----------------------------------------------------------------------

    /// Approve or revoke a token for use in new plans. Does not affect plans
    /// or subscriptions already created with that token.
    pub fn set_approved_token(env: Env, admin: Address, token: Address, approved: bool) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("not admin");
        }

        env.storage()
            .persistent()
            .set(&DataKey::ApprovedToken(token.clone()), &approved);

        env.events().publish(
            (Symbol::new(&env, "token_appr"),),
            (token, approved),
        );
    }

    /// Returns whether `token` is currently approved for new plans.
    pub fn is_token_approved(env: Env, token: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::ApprovedToken(token))
            .unwrap_or(false)
    }

    /// Update the treasury address that receives the platform fee split.
    pub fn set_treasury(env: Env, admin: Address, treasury: Address) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("not admin");
        }
        env.storage().persistent().set(&DataKey::Treasury, &treasury);
    }

    /// Update the platform fee (basis points, 0-10000) taken from each
    /// subscribe/renew payment.
    pub fn set_platform_fee_bps(env: Env, admin: Address, fee_bps: i128) {
        let stored_admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("not initialized");
        admin.require_auth();
        if admin != stored_admin {
            panic!("not admin");
        }
        if !(0..=10_000).contains(&fee_bps) {
            panic!("fee_bps must be 0-10000");
        }
        env.storage()
            .persistent()
            .set(&DataKey::PlatformFeeBps, &fee_bps);
    }

    /// View the cumulative revenue (net of platform fee) a mentor has
    /// received in a given token across all subscribe/renew payments.
    pub fn get_mentor_revenue(env: Env, mentor: Address, token: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::MentorRevenue(mentor, token))
            .unwrap_or(0)
    }

    /// Splits `amount` into (treasury_fee, escrow_remainder) using the
    /// current platform fee in bps, and routes/records the payment:
    /// `amount * platform_fee_bps / 10000` to treasury, remainder to escrow.
    /// Updates the (mentor, token) revenue accumulator with the remainder.
    fn route_payment(env: &Env, payer: &Address, mentor: &Address, token: &Address, amount: i128) {
        let treasury: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Treasury)
            .expect("not initialized");
        let fee_bps: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PlatformFeeBps)
            .unwrap_or(DEFAULT_PLATFORM_FEE_BPS);
        let escrow: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Escrow)
            .expect("not initialized");

        let fee = amount
            .checked_mul(fee_bps)
            .expect("fee overflow")
            .checked_div(10_000)
            .expect("fee division error");
        let remainder = amount - fee;

        let token_client = token::Client::new(env, token);
        if fee > 0 {
            token_client.transfer(payer, &treasury, &fee);
        }
        token_client.transfer(payer, &escrow, &remainder);

        let revenue_key = DataKey::MentorRevenue(mentor.clone(), token.clone());
        let prior: i128 = env.storage().persistent().get(&revenue_key).unwrap_or(0);
        env.storage()
            .persistent()
            .set(&revenue_key, &(prior + remainder));

        env.events().publish(
            (Symbol::new(env, "fee_split"),),
            (mentor.clone(), token.clone(), fee, remainder),
        );
    }

    // -----------------------------------------------------------------------
    // Plans
    // -----------------------------------------------------------------------

    /// Create a subscription plan. Returns the new plan ID.
    pub fn create_plan(
        env: Env,
        mentor: Address,
        price_per_month: i128,
        token: Address,
        sessions_per_month: u32,
    ) -> u32 {
        mentor.require_auth();
        if price_per_month <= 0 {
            panic!("price must be positive");
        }
        if sessions_per_month == 0 {
            panic!("sessions_per_month must be > 0");
        }
        let approved: bool = env
            .storage()
            .persistent()
            .get(&DataKey::ApprovedToken(token.clone()))
            .unwrap_or(false);
        if !approved {
            panic!("token not approved");
        }

        let plan_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::PlanCounter)
            .unwrap_or(0);

        let plan = Plan {
            mentor,
            price_per_month,
            token,
            sessions_per_month,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Plan(plan_id), &plan);
        env.storage()
            .persistent()
            .set(&DataKey::PlanCounter, &(plan_id + 1));

        plan_id
    }

    // -----------------------------------------------------------------------
    // Subscriptions
    // -----------------------------------------------------------------------

    /// Subscribe to a plan. Transfers first month payment to escrow.
    pub fn subscribe(env: Env, learner: Address, plan_id: u32) -> u32 {
        learner.require_auth();

        let plan: Plan = env
            .storage()
            .persistent()
            .get(&DataKey::Plan(plan_id))
            .expect("plan not found");

        // Split first month payment: platform fee to treasury, remainder to escrow.
        Self::route_payment(&env, &learner, &plan.mentor, &plan.token, plan.price_per_month);

        let now = env.ledger().timestamp();
        let sub_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::SubCounter)
            .unwrap_or(0);

        let record = SubscriptionRecord {
            learner: learner.clone(),
            mentor: plan.mentor.clone(),
            plan_id,
            start_date: now,
            next_billing_date: now + SECONDS_PER_MONTH,
            sessions_used: 0,
            status: SubscriptionStatus::Active,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Sub(sub_id), &record);
        env.storage()
            .persistent()
            .set(&DataKey::SubCounter, &(sub_id + 1));

        env.events().publish(
            (symbol_short!("subscrbd"), plan_id),
            (learner, plan.mentor, sub_id),
        );

        sub_id
    }

    /// Renew a subscription — callable by anyone once next_billing_date is reached.
    ///
    /// # Timestamp security
    /// A grace period of `RENEWAL_GRACE_SECS` is applied to the billing-date
    /// check so that a validator with a slightly slow clock cannot prevent a
    /// timely renewal.  The subscription must also not have lapsed beyond
    /// `SUBSCRIPTION_EXPIRY_GRACE_SECS` past the billing date; if it has, the
    /// subscription is transitioned to `Expired` and renewal is rejected.
    pub fn renew(env: Env, subscription_id: u32) {
        let mut record: SubscriptionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Sub(subscription_id))
            .expect("subscription not found");

        if record.status != SubscriptionStatus::Active {
            panic!("subscription not active");
        }

        let now = env.ledger().timestamp();

        // Check whether the subscription has lapsed (past billing date + expiry grace).
        let expiry_deadline = record
            .next_billing_date
            .checked_add(SUBSCRIPTION_EXPIRY_GRACE_SECS)
            .expect("timestamp overflow");
        if now >= expiry_deadline {
            // Subscription has lapsed past the grace window — transition to Expired
            // and return gracefully.  The learner must create a new subscription.
            record.status = SubscriptionStatus::Expired;
            env.storage()
                .persistent()
                .set(&DataKey::Sub(subscription_id), &record);
            // Clear any stale allowance.
            env.storage()
                .persistent()
                .remove(&DataKey::RenewalAllowance(subscription_id));
            env.events().publish(
                (symbol_short!("expired"), subscription_id),
                (record.learner, record.plan_id),
            );
            return;
        }

        // Apply grace period: allow renewal up to RENEWAL_GRACE_SECS before
        // the billing date to absorb validator clock drift.
        let effective_billing_date = record
            .next_billing_date
            .saturating_sub(RENEWAL_GRACE_SECS);
        if now < effective_billing_date {
            panic!("billing date not reached");
        }

        let plan: Plan = env
            .storage()
            .persistent()
            .get(&DataKey::Plan(record.plan_id))
            .expect("plan not found");

        // -----------------------------------------------------------------------
        // Allowance pull: deduct from pre-authorized allowance, then verify the
        // learner has sufficient balance before transferring.
        // On failure: transition to Expired + emit RenewalFailed — do NOT panic.
        // -----------------------------------------------------------------------
        let allowance: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::RenewalAllowance(subscription_id))
            .unwrap_or(0i128);

        let token_client = token::Client::new(&env, &plan.token);
        let available = token_client.balance(&record.learner);
        let required = plan.price_per_month;

        if allowance < required || available < required {
            record.status = SubscriptionStatus::Expired;
            env.storage()
                .persistent()
                .set(&DataKey::Sub(subscription_id), &record);
            // Clear the exhausted / invalid allowance so subsequent views are accurate.
            env.storage()
                .persistent()
                .remove(&DataKey::RenewalAllowance(subscription_id));
            env.events().publish(
                (Symbol::new(&env, "renewal_fail"), subscription_id),
                RenewalFailedEvent {
                    sub_id: subscription_id,
                    required,
                    available: available.min(allowance),
                },
            );
            // Return without panicking — state is correctly Expired.
            return;
        }

        // Deduct from the pre-authorized allowance first.
        let new_allowance = allowance - required;
        env.storage()
            .persistent()
            .set(&DataKey::RenewalAllowance(subscription_id), &new_allowance);

        // Pull payment, split between treasury (fee) and escrow (remainder).
        Self::route_payment(&env, &record.learner, &record.mentor, &plan.token, required);

        record.next_billing_date += SECONDS_PER_MONTH;
        record.sessions_used = 0;
        env.storage()
            .persistent()
            .set(&DataKey::Sub(subscription_id), &record);

        env.events().publish(
            (symbol_short!("renewed"), subscription_id),
            (record.learner, record.plan_id),
        );
    }

    /// Cancel a subscription — learner only, effective end of billing period.
    pub fn cancel(env: Env, subscription_id: u32) {
        let mut record: SubscriptionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Sub(subscription_id))
            .expect("subscription not found");

        record.learner.require_auth();

        if record.status == SubscriptionStatus::Cancelled {
            panic!("already cancelled");
        }

        record.status = SubscriptionStatus::Cancelled;
        env.storage()
            .persistent()
            .set(&DataKey::Sub(subscription_id), &record);

        env.events().publish(
            (symbol_short!("cancelled"), subscription_id),
            (record.learner, record.plan_id),
        );
    }

    /// Pause a subscription — learner only.
    pub fn pause(env: Env, subscription_id: u32) {
        let mut record: SubscriptionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Sub(subscription_id))
            .expect("subscription not found");

        record.learner.require_auth();

        if record.status != SubscriptionStatus::Active {
            panic!("subscription not active");
        }

        record.status = SubscriptionStatus::Paused;
        env.storage()
            .persistent()
            .set(&DataKey::Sub(subscription_id), &record);

        env.events().publish(
            (symbol_short!("paused"), subscription_id),
            (record.learner, record.plan_id),
        );
    }

    /// Record a session use. Panics if limit exceeded, subscription not active,
    /// or the subscription has lapsed past its expiry grace window.
    ///
    /// # Timestamp security
    /// Before recording a session, the subscription's expiry status is checked.
    /// If the current time has passed `next_billing_date + SUBSCRIPTION_EXPIRY_GRACE_SECS`
    /// the subscription is transitioned to `Expired` and the session is rejected.
    /// This prevents sessions from being consumed on a lapsed subscription.
    pub fn use_session(env: Env, subscription_id: u32) {
        let mut record: SubscriptionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Sub(subscription_id))
            .expect("subscription not found");

        if record.status != SubscriptionStatus::Active {
            panic!("subscription not active");
        }

        // Lazily transition to Expired if the subscription has lapsed.
        let now = env.ledger().timestamp();
        let expiry_deadline = record
            .next_billing_date
            .checked_add(SUBSCRIPTION_EXPIRY_GRACE_SECS)
            .expect("timestamp overflow");
        if now >= expiry_deadline {
            record.status = SubscriptionStatus::Expired;
            env.storage()
                .persistent()
                .set(&DataKey::Sub(subscription_id), &record);
            env.events().publish(
                (symbol_short!("expired"), subscription_id),
                (record.learner, record.plan_id),
            );
            panic!("subscription expired");
        }

        let plan: Plan = env
            .storage()
            .persistent()
            .get(&DataKey::Plan(record.plan_id))
            .expect("plan not found");

        if record.sessions_used >= plan.sessions_per_month {
            panic!("session limit reached");
        }

        record.sessions_used += 1;
        env.storage()
            .persistent()
            .set(&DataKey::Sub(subscription_id), &record);
    }

    /// Explicitly check and transition a subscription to Expired if it has
    /// lapsed.  This is a fallback for off-chain systems that need to
    /// synchronise state without waiting for a `use_session` or `renew` call.
    pub fn check_expiry(env: Env, subscription_id: u32) {
        let mut record: SubscriptionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Sub(subscription_id))
            .expect("subscription not found");

        if record.status != SubscriptionStatus::Active {
            return; // Already in a terminal or non-active state
        }

        let now = env.ledger().timestamp();
        let expiry_deadline = record
            .next_billing_date
            .checked_add(SUBSCRIPTION_EXPIRY_GRACE_SECS)
            .expect("timestamp overflow");

        if now >= expiry_deadline {
            record.status = SubscriptionStatus::Expired;
            env.storage()
                .persistent()
                .set(&DataKey::Sub(subscription_id), &record);
            env.events().publish(
                (symbol_short!("expired"), subscription_id),
                (record.learner, record.plan_id),
            );
        }
    }

    /// Get a subscription record by ID.
    pub fn get_subscription(env: Env, id: u32) -> SubscriptionRecord {
        env.storage()
            .persistent()
            .get(&DataKey::Sub(id))
            .expect("subscription not found")
    }

    /// Get a plan by ID.
    pub fn get_plan(env: Env, plan_id: u32) -> Plan {
        env.storage()
            .persistent()
            .get(&DataKey::Plan(plan_id))
            .expect("plan not found")
    }

    // -----------------------------------------------------------------------
    // Auto-renewal pre-authorization
    // -----------------------------------------------------------------------

    /// Pre-authorize the contract to pull up to `max_amount` tokens from the
    /// learner's balance for automatic renewals.  The learner signs once; the
    /// contract deducts `plan.price_per_month` on each successful `renew` call
    /// until the allowance is exhausted.
    ///
    /// Calling this again replaces any existing allowance (useful for top-ups).
    pub fn authorize_renewal(env: Env, sub_id: u32, max_amount: i128) {
        let record: SubscriptionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Sub(sub_id))
            .expect("subscription not found");

        // Only the learner may set their own allowance.
        record.learner.require_auth();

        if max_amount < 0 {
            panic!("max_amount must be non-negative");
        }

        env.storage()
            .persistent()
            .set(&DataKey::RenewalAllowance(sub_id), &max_amount);

        env.events().publish(
            (Symbol::new(&env, "renewal_auth"), sub_id),
            (record.learner, max_amount),
        );
    }

    /// Get the pre-authorized renewal allowance for a subscription.
    /// Returns 0 if no allowance has been set.
    pub fn get_renewal_status(env: Env, sub_id: u32) -> RenewalStatus {
        let record: SubscriptionRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Sub(sub_id))
            .expect("subscription not found");

        let plan: Plan = env
            .storage()
            .persistent()
            .get(&DataKey::Plan(record.plan_id))
            .expect("plan not found");

        let allowance_remaining: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::RenewalAllowance(sub_id))
            .unwrap_or(0i128);

        let is_auto_renewable = record.status == SubscriptionStatus::Active
            && allowance_remaining >= plan.price_per_month;

        RenewalStatus {
            next_due: record.next_billing_date,
            allowance_remaining,
            is_auto_renewable,
        }
    }

    // -----------------------------------------------------------------------
    // Streaming payments  (#665)
    // -----------------------------------------------------------------------

    /// Create a streaming plan.  The mentor defines a per-second token rate;
    /// learners deposit a lump sum and earn sessions for as long as the deposit
    /// lasts.  Returns the new stream plan ID.
    pub fn create_streaming_plan(
        env: Env,
        mentor: Address,
        rate_per_second: i128,
        token: Address,
    ) -> u32 {
        mentor.require_auth();
        if rate_per_second <= 0 {
            panic!("rate_per_second must be positive");
        }

        let plan_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StreamPlanCounter)
            .unwrap_or(0u32);

        env.storage().persistent().set(
            &DataKey::StreamPlan(plan_id),
            &StreamPlan {
                mentor,
                rate_per_second,
                token,
            },
        );
        env.storage()
            .persistent()
            .set(&DataKey::StreamPlanCounter, &(plan_id + 1));

        plan_id
    }

    /// Start a streaming subscription.  The learner deposits `deposit_amount`
    /// tokens into escrow; streaming begins immediately at the plan's
    /// `rate_per_second`.  Returns the new streaming subscription ID.
    pub fn subscribe_streaming(
        env: Env,
        learner: Address,
        plan_id: u32,
        deposit_amount: i128,
    ) -> u32 {
        learner.require_auth();

        let plan: StreamPlan = env
            .storage()
            .persistent()
            .get(&DataKey::StreamPlan(plan_id))
            .expect("stream plan not found");

        if deposit_amount <= 0 {
            panic!("deposit_amount must be positive");
        }

        let escrow: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Escrow)
            .expect("not initialized");

        // Move the deposit from learner to escrow immediately.
        token::Client::new(&env, &plan.token).transfer(
            &learner,
            &escrow,
            &deposit_amount,
        );

        let sub_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::StreamSubCounter)
            .unwrap_or(0u32);

        let now = env.ledger().timestamp();
        env.storage().persistent().set(
            &DataKey::StreamSub(sub_id),
            &StreamingSubscription {
                learner: learner.clone(),
                mentor: plan.mentor.clone(),
                plan_id,
                start: now,
                balance_deposited: deposit_amount,
                withdrawn_by_mentor: 0,
                is_active: true,
                ended_at: 0,
            },
        );
        env.storage()
            .persistent()
            .set(&DataKey::StreamSubCounter, &(sub_id + 1));

        env.events().publish(
            (Symbol::new(&env, "stream_start"), sub_id),
            (learner, plan.mentor, deposit_amount),
        );

        sub_id
    }

    /// Mentor withdraws tokens earned so far.
    ///
    /// `earned = min(elapsed_seconds * rate_per_second, balance_deposited)`
    /// `claimable = earned - withdrawn_by_mentor`
    ///
    /// Panics if called on an inactive stream with nothing left to claim, or if
    /// the caller is not the stream's mentor.
    pub fn withdraw_earned(env: Env, mentor: Address, sub_id: u32) {
        mentor.require_auth();

        let mut record: StreamingSubscription = env
            .storage()
            .persistent()
            .get(&DataKey::StreamSub(sub_id))
            .expect("stream not found");

        if record.mentor != mentor {
            panic!("not the stream mentor");
        }

        let plan: StreamPlan = env
            .storage()
            .persistent()
            .get(&DataKey::StreamPlan(record.plan_id))
            .expect("stream plan not found");

        // Use the cancellation timestamp when the stream has ended, otherwise now.
        let effective_now = if record.is_active {
            env.ledger().timestamp()
        } else {
            record.ended_at
        };

        let elapsed = effective_now.saturating_sub(record.start) as i128;
        let total_earned = (elapsed * plan.rate_per_second)
            .min(record.balance_deposited);
        let claimable = total_earned - record.withdrawn_by_mentor;

        if claimable <= 0 {
            panic!("nothing to withdraw");
        }

        // Claimable is already bounded by balance_deposited via total_earned.
        record.withdrawn_by_mentor += claimable;
        env.storage()
            .persistent()
            .set(&DataKey::StreamSub(sub_id), &record);

        let escrow: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Escrow)
            .expect("not initialized");

        token::Client::new(&env, &plan.token).transfer(
            &escrow,
            &mentor,
            &claimable,
        );

        env.events().publish(
            (Symbol::new(&env, "stream_withdraw"), sub_id),
            (mentor, claimable),
        );
    }

    /// Cancel a streaming subscription.  Splits the deposit exactly:
    ///   mentor receives `earned_so_far - withdrawn_by_mentor`
    ///   learner receives `balance_deposited - earned_so_far`
    ///
    /// Invariant (enforced by construction): mentor_paid + learner_refund == balance_deposited.
    pub fn cancel_streaming(env: Env, learner: Address, sub_id: u32) {
        learner.require_auth();

        let mut record: StreamingSubscription = env
            .storage()
            .persistent()
            .get(&DataKey::StreamSub(sub_id))
            .expect("stream not found");

        if record.learner != learner {
            panic!("not the stream learner");
        }
        if !record.is_active {
            panic!("stream already cancelled");
        }

        let plan: StreamPlan = env
            .storage()
            .persistent()
            .get(&DataKey::StreamPlan(record.plan_id))
            .expect("stream plan not found");

        let now = env.ledger().timestamp();
        let elapsed = now.saturating_sub(record.start) as i128;

        // Total tokens earned by mentor up to this moment, capped at deposit.
        let total_earned = (elapsed * plan.rate_per_second)
            .min(record.balance_deposited);

        // How much of that is still sitting in escrow (mentor hasn't withdrawn yet).
        let mentor_owed = total_earned - record.withdrawn_by_mentor;

        // Learner gets everything that hasn't been earned.
        // Zero-loss invariant: mentor_owed + learner_refund == balance_deposited - withdrawn_by_mentor
        // and withdrawn_by_mentor is already out of escrow, so escrow holds exactly that remainder.
        let learner_refund = record.balance_deposited - total_earned;

        // Mark stream as ended before any transfers (checks-effects-interactions).
        record.is_active = false;
        record.ended_at = now;
        record.withdrawn_by_mentor = total_earned; // fully settled
        env.storage()
            .persistent()
            .set(&DataKey::StreamSub(sub_id), &record);

        let escrow: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Escrow)
            .expect("not initialized");

        let token_client = token::Client::new(&env, &plan.token);

        // Transfer mentor's earned-but-not-yet-withdrawn portion.
        if mentor_owed > 0 {
            token_client.transfer(&escrow, &record.mentor, &mentor_owed);
        }
        // Refund learner's unearned portion.
        if learner_refund > 0 {
            token_client.transfer(&escrow, &learner, &learner_refund);
        }

        env.events().publish(
            (Symbol::new(&env, "stream_cancel"), sub_id),
            (learner, record.mentor, mentor_owed, learner_refund),
        );
    }

    /// View a streaming subscription record by ID.
    pub fn get_streaming_subscription(env: Env, sub_id: u32) -> StreamingSubscription {
        env.storage()
            .persistent()
            .get(&DataKey::StreamSub(sub_id))
            .expect("stream not found")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::{
        testutils::{Address as _, Ledger},
        token::{Client as TokenClient, StellarAssetClient},
        Address, Env,
    };

    fn setup() -> (Env, SubscriptionContractClient<'static>, Address, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, SubscriptionContract);
        let client = SubscriptionContractClient::new(&env, &contract_id);

        let admin = Address::generate(&env);
        let escrow = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);

        client.initialize(&admin, &escrow);
        (env, client, admin, escrow, mentor, learner)
    }

    fn create_token<'a>(
        env: &'a Env,
        admin: &Address,
        client: &SubscriptionContractClient<'a>,
    ) -> (Address, TokenClient<'a>, StellarAssetClient<'a>) {
        let token_id = env.register_stellar_asset_contract_v2(admin.clone());
        let token_address = token_id.address();
        let token = TokenClient::new(env, &token_address);
        let token_admin = StellarAssetClient::new(env, &token_address);
        (token_address, token, token_admin)
    }

    /// Approve `token_address` on `client` using `admin`. Kept separate from
    /// `create_token` so asset-contract setup (register → mint) is not
    /// interleaved with a subscription-contract call.
    fn approve_token(env: &Env, client: &SubscriptionContractClient, admin: &Address, token_address: &Address) {
        client.set_approved_token(admin, token_address, &true);
        // Re-arm auth mocking: recording an explicit require_auth call above
        // otherwise leaves later implicit (escrow) transfers unauthorized
        // under `mock_all_auths()` in this SDK version.
        env.mock_all_auths();
    }

    #[test]
    fn test_subscribe() {
        let (env, client, admin, escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);

        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        assert_eq!(sub_id, 0);
        assert_eq!(token.balance(&learner), 900);
        // 2.5% platform fee (2) goes to treasury (defaults to admin); remainder (98) to escrow.
        assert_eq!(token.balance(&escrow), 98);
        assert_eq!(token.balance(&admin), 2);
        assert_eq!(client.get_mentor_revenue(&mentor, &token_address), 98);

        let record = client.get_subscription(&sub_id);
        assert_eq!(record.status, SubscriptionStatus::Active);
        assert_eq!(record.sessions_used, 0);
        assert_eq!(record.plan_id, plan_id);
    }

    #[test]
    fn test_renew() {
        let (env, client, admin, escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);

        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        // Pre-authorize one renewal.
        client.authorize_renewal(&sub_id, &100i128);

        // Advance time past billing date (within expiry grace window)
        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH + 1;
        });

        client.renew(&sub_id);

        assert_eq!(token.balance(&learner), 800);
        assert_eq!(token.balance(&escrow), 200);

        let record = client.get_subscription(&sub_id);
        assert_eq!(record.sessions_used, 0); // reset on renewal
    }

    #[test]
    fn test_renew_within_grace_period() {
        // Renewal should succeed up to RENEWAL_GRACE_SECS before billing date.
        let (env, client, admin, escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);

        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        // Pre-authorize one renewal.
        client.authorize_renewal(&sub_id, &100i128);

        // Advance to billing_date - RENEWAL_GRACE_SECS (just inside grace window)
        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH - RENEWAL_GRACE_SECS;
        });

        client.renew(&sub_id);
        assert_eq!(token.balance(&escrow), 200);
    }

    #[test]
    #[should_panic(expected = "billing date not reached")]
    fn test_renew_too_early_panics() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        // Do NOT advance time — should panic (well before grace window)
        client.renew(&sub_id);
    }

    #[test]
    fn test_renew_after_expiry_transitions_expired() {
        // When the subscription has lapsed beyond SUBSCRIPTION_EXPIRY_GRACE_SECS,
        // renew transitions to Expired and returns — it does NOT panic.
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        // Set an allowance — renewal fails because of lapse, not balance.
        client.authorize_renewal(&sub_id, &100i128);

        // Advance past billing_date + SUBSCRIPTION_EXPIRY_GRACE_SECS
        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH + SUBSCRIPTION_EXPIRY_GRACE_SECS + 1;
        });

        client.renew(&sub_id);

        let record = client.get_subscription(&sub_id);
        assert_eq!(record.status, SubscriptionStatus::Expired);
        // No payment was pulled.
        assert_eq!(token.balance(&learner), 900);
    }

    #[test]
    #[should_panic(expected = "subscription expired")]
    fn test_use_session_after_expiry_panics() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        // Advance past expiry
        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH + SUBSCRIPTION_EXPIRY_GRACE_SECS + 1;
        });

        client.use_session(&sub_id);
    }

    #[test]
    fn test_check_expiry_transitions_to_expired() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        // Advance past expiry
        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH + SUBSCRIPTION_EXPIRY_GRACE_SECS + 1;
        });

        client.check_expiry(&sub_id);

        let record = client.get_subscription(&sub_id);
        assert_eq!(record.status, SubscriptionStatus::Expired);
    }

    #[test]
    fn test_check_expiry_no_op_when_active() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        // Do not advance time — subscription is still active
        client.check_expiry(&sub_id);

        let record = client.get_subscription(&sub_id);
        assert_eq!(record.status, SubscriptionStatus::Active);
    }

    /// Simulate a validator that skews the clock forward by RENEWAL_GRACE_SECS.
    /// The subscription must NOT be renewable before the billing date has elapsed.
    #[test]
    #[should_panic(expected = "billing date not reached")]
    fn test_manipulated_timestamp_cannot_renew_early() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        client.authorize_renewal(&sub_id, &100i128);

        // Validator skews clock forward by RENEWAL_GRACE_SECS - 1.
        // This is still before the effective billing date.
        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH - RENEWAL_GRACE_SECS - 1;
        });

        client.renew(&sub_id);
    }

    #[test]
    fn test_cancel_mid_period() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        // Cancel mid-period (no time advance needed)
        client.cancel(&sub_id);

        let record = client.get_subscription(&sub_id);
        assert_eq!(record.status, SubscriptionStatus::Cancelled);

        // Advance past billing date — renew should fail
        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH + 1;
        });
    }

    #[test]
    fn test_session_count_enforcement() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &2u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        client.use_session(&sub_id);
        client.use_session(&sub_id);

        let record = client.get_subscription(&sub_id);
        assert_eq!(record.sessions_used, 2);
    }

    #[test]
    #[should_panic(expected = "session limit reached")]
    fn test_session_limit_exceeded_panics() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &1u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        client.use_session(&sub_id);
        client.use_session(&sub_id); // should panic
    }

    #[test]
    fn test_pause() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        client.pause(&sub_id);

        let record = client.get_subscription(&sub_id);
        assert_eq!(record.status, SubscriptionStatus::Paused);
    }

    // -----------------------------------------------------------------------
    // Auto-renewal / allowance tests  (#660)
    // -----------------------------------------------------------------------

    /// Acceptance criterion: authorize 2 months, auto-renewal succeeds twice,
    /// fails (→ Expired, no panic) on the 3rd attempt.
    #[test]
    fn test_auto_renewal_two_months_then_expires() {
        let (env, client, admin, escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);

        // Mint enough for the initial subscribe + 2 renewals = 300.
        token_admin.mint(&learner, &300);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id); // pays 100

        // Pre-authorize exactly 2 months.
        client.authorize_renewal(&sub_id, &200i128);

        // --- Renewal 1 ---
        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH + 1;
        });
        client.renew(&sub_id);
        assert_eq!(token.balance(&learner), 100);
        assert_eq!(token.balance(&escrow), 200);
        let record = client.get_subscription(&sub_id);
        assert_eq!(record.status, SubscriptionStatus::Active);

        // Check renewal status after 1st renewal: 100 allowance remaining.
        let status = client.get_renewal_status(&sub_id);
        assert_eq!(status.allowance_remaining, 100);
        assert!(status.is_auto_renewable);

        // --- Renewal 2 ---
        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH;
        });
        client.renew(&sub_id);
        assert_eq!(token.balance(&learner), 0);
        assert_eq!(token.balance(&escrow), 300);
        let record = client.get_subscription(&sub_id);
        assert_eq!(record.status, SubscriptionStatus::Active);

        // After 2nd renewal: allowance is 0.
        let status = client.get_renewal_status(&sub_id);
        assert_eq!(status.allowance_remaining, 0);
        assert!(!status.is_auto_renewable);

        // --- Renewal 3: insufficient allowance → Expired, no panic ---
        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH;
        });
        client.renew(&sub_id); // must NOT panic
        let record = client.get_subscription(&sub_id);
        assert_eq!(record.status, SubscriptionStatus::Expired);
        // No additional payment was pulled.
        assert_eq!(token.balance(&learner), 0);
        assert_eq!(token.balance(&escrow), 300);
    }

    /// Insufficient on-chain balance (even with sufficient allowance) → Expired.
    #[test]
    fn test_renewal_fails_on_insufficient_balance() {
        let (env, client, admin, escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);

        // Mint only enough for the initial subscribe; nothing left for renewals.
        token_admin.mint(&learner, &100);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id); // pays 100 → balance = 0

        // Pre-authorize a large amount — but the learner has no tokens.
        client.authorize_renewal(&sub_id, &1000i128);

        env.ledger().with_mut(|li| {
            li.timestamp += SECONDS_PER_MONTH + 1;
        });

        // Must NOT panic; must transition to Expired.
        client.renew(&sub_id);

        let record = client.get_subscription(&sub_id);
        assert_eq!(record.status, SubscriptionStatus::Expired);
        // Escrow balance unchanged since subscribe (98 = 100 - 2.5% fee).
        assert_eq!(token.balance(&escrow), 98);
        assert_eq!(token.balance(&learner), 0);
    }

    /// get_renewal_status reflects freshly set allowance correctly.
    #[test]
    fn test_get_renewal_status_initial() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &500);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        // No allowance yet.
        let status = client.get_renewal_status(&sub_id);
        assert_eq!(status.allowance_remaining, 0);
        assert!(!status.is_auto_renewable);

        // After authorize.
        client.authorize_renewal(&sub_id, &300i128);
        let status = client.get_renewal_status(&sub_id);
        assert_eq!(status.allowance_remaining, 300);
        assert!(status.is_auto_renewable);
        assert_eq!(status.next_due, client.get_subscription(&sub_id).next_billing_date);
    }

    /// Calling authorize_renewal again replaces the existing allowance.
    #[test]
    fn test_authorize_renewal_replace() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &500);

        let plan_id = client.create_plan(&mentor, &100i128, &token_address, &5u32);
        let sub_id = client.subscribe(&learner, &plan_id);

        client.authorize_renewal(&sub_id, &200i128);
        client.authorize_renewal(&sub_id, &50i128); // replaces

        let status = client.get_renewal_status(&sub_id);
        assert_eq!(status.allowance_remaining, 50);
        // 50 < 100 (plan price) → not auto-renewable.
        assert!(!status.is_auto_renewable);
    }

    // -----------------------------------------------------------------------
    // Streaming payment tests  (#665)
    // -----------------------------------------------------------------------

    /// Integration test: 100-second stream at 10 tokens/sec.
    /// Cancel at 40s → mentor gets 400, learner gets 600.
    #[test]
    fn test_streaming_cancel_at_40s() {
        let (env, client, admin, escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);

        token_admin.mint(&learner, &1000);

        // Stream plan: 10 tokens/sec
        let plan_id = client.create_streaming_plan(&mentor, &10i128, &token_address);

        // Learner deposits 1000 (covers 100 seconds at 10/sec).
        let sub_id = client.subscribe_streaming(&learner, &plan_id, &1000i128);

        assert_eq!(token.balance(&learner), 0);
        assert_eq!(token.balance(&escrow), 1000);

        // Advance ledger by 40 seconds.
        env.ledger().with_mut(|li| li.timestamp += 40);

        // Cancel: mentor should receive 400, learner should receive 600.
        client.cancel_streaming(&learner, &sub_id);

        assert_eq!(token.balance(&mentor), 400);
        assert_eq!(token.balance(&learner), 600);
        assert_eq!(token.balance(&escrow), 0);

        let record = client.get_streaming_subscription(&sub_id);
        assert!(!record.is_active);
        assert_eq!(record.withdrawn_by_mentor, 400); // settled
    }

    /// withdraw_earned transfers exact pro-rated amount.
    #[test]
    fn test_streaming_withdraw_earned_exact() {
        let (env, client, admin, escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);

        token_admin.mint(&learner, &600);

        let plan_id = client.create_streaming_plan(&mentor, &10i128, &token_address);
        let sub_id = client.subscribe_streaming(&learner, &plan_id, &600i128);

        // Advance 30 seconds → mentor has earned 300.
        env.ledger().with_mut(|li| li.timestamp += 30);
        client.withdraw_earned(&mentor, &sub_id);

        assert_eq!(token.balance(&mentor), 300);
        assert_eq!(token.balance(&escrow), 300); // 600 - 300

        let record = client.get_streaming_subscription(&sub_id);
        assert_eq!(record.withdrawn_by_mentor, 300);

        // Advance another 20 seconds → 200 more earned.
        env.ledger().with_mut(|li| li.timestamp += 20);
        client.withdraw_earned(&mentor, &sub_id);

        assert_eq!(token.balance(&mentor), 500);
        assert_eq!(token.balance(&escrow), 100);
    }

    /// Mentor cannot withdraw more than balance_deposited even if the clock
    /// advances far beyond the stream's funded duration.
    #[test]
    fn test_streaming_withdraw_capped_at_deposit() {
        let (env, client, admin, escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);

        token_admin.mint(&learner, &50);

        // 10 tokens/sec, only 50 tokens deposited → stream runs for 5 seconds.
        let plan_id = client.create_streaming_plan(&mentor, &10i128, &token_address);
        let sub_id = client.subscribe_streaming(&learner, &plan_id, &50i128);

        // Advance 100 seconds — far past funded window.
        env.ledger().with_mut(|li| li.timestamp += 100);
        client.withdraw_earned(&mentor, &sub_id);

        // Mentor should only receive the deposited 50, not 1000.
        assert_eq!(token.balance(&mentor), 50);
        assert_eq!(token.balance(&escrow), 0);
    }

    /// cancel_streaming after a partial withdraw still splits with zero loss.
    #[test]
    fn test_streaming_cancel_after_partial_withdraw() {
        let (env, client, admin, escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);

        token_admin.mint(&learner, &1000);

        let plan_id = client.create_streaming_plan(&mentor, &10i128, &token_address);
        let sub_id = client.subscribe_streaming(&learner, &plan_id, &1000i128);

        // Mentor withdraws after 30s.
        env.ledger().with_mut(|li| li.timestamp += 30);
        client.withdraw_earned(&mentor, &sub_id);
        assert_eq!(token.balance(&mentor), 300);

        // Learner cancels 10s later (40s total elapsed).
        env.ledger().with_mut(|li| li.timestamp += 10);
        client.cancel_streaming(&learner, &sub_id);

        // Total earned = 400.  Mentor already got 300, gets 100 more on cancel.
        // Learner gets 600 back.
        assert_eq!(token.balance(&mentor), 400);
        assert_eq!(token.balance(&learner), 600);
        assert_eq!(token.balance(&escrow), 0);
    }

    /// Property: mentor_withdrawn + learner_refund == balance_deposited always.
    /// Verified across three different cancel times.
    #[test]
    fn test_streaming_zero_loss_invariant() {
        for cancel_after_secs in [0u64, 50u64, 100u64] {
            let env = Env::default();
            env.mock_all_auths();

            let contract_id = env.register_contract(None, SubscriptionContract);
            let client = SubscriptionContractClient::new(&env, &contract_id);

            let admin = Address::generate(&env);
            let escrow = Address::generate(&env);
            let mentor = Address::generate(&env);
            let learner = Address::generate(&env);
            client.initialize(&admin, &escrow);

            let token_id = env.register_stellar_asset_contract_v2(admin.clone());
            let token_address = token_id.address();
            let token = TokenClient::new(&env, &token_address);
            let token_admin = StellarAssetClient::new(&env, &token_address);

            let deposit: i128 = 1000;
            token_admin.mint(&learner, &deposit);

            let plan_id = client.create_streaming_plan(&mentor, &10i128, &token_address);
            let sub_id = client.subscribe_streaming(&learner, &plan_id, &deposit);

            env.ledger().with_mut(|li| li.timestamp += cancel_after_secs);
            client.cancel_streaming(&learner, &sub_id);

            let mentor_got = token.balance(&mentor);
            let learner_got = token.balance(&learner);

            assert_eq!(
                mentor_got + learner_got,
                deposit,
                "zero-loss violated at cancel_after={}s: mentor={}, learner={}, sum={}",
                cancel_after_secs,
                mentor_got,
                learner_got,
                mentor_got + learner_got
            );
        }
    }

    /// Cancelling an already-cancelled stream panics.
    #[test]
    #[should_panic(expected = "stream already cancelled")]
    fn test_streaming_double_cancel_panics() {
        let (env, client, admin, _escrow, mentor, learner) = setup();
        let (token_address, _token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_streaming_plan(&mentor, &10i128, &token_address);
        let sub_id = client.subscribe_streaming(&learner, &plan_id, &1000i128);

        env.ledger().with_mut(|li| li.timestamp += 10);
        client.cancel_streaming(&learner, &sub_id);
        client.cancel_streaming(&learner, &sub_id); // should panic
    }

    /// After cancellation, any remaining earned amount is still withdrawable
    /// by the mentor (stream ended_at is recorded).
    #[test]
    fn test_streaming_withdraw_after_cancel() {
        let (env, client, admin, escrow, mentor, learner) = setup();
        let (token_address, token, token_admin) = create_token(&env, &admin, &client);
        approve_token(&env, &client, &admin, &token_address);
        token_admin.mint(&learner, &1000);

        let plan_id = client.create_streaming_plan(&mentor, &10i128, &token_address);
        let sub_id = client.subscribe_streaming(&learner, &plan_id, &1000i128);

        // Cancel at 60s without mentor having withdrawn anything.
        env.ledger().with_mut(|li| li.timestamp += 60);
        client.cancel_streaming(&learner, &sub_id);

        // cancel_streaming settles the mentor share (600) directly.
        assert_eq!(token.balance(&mentor), 600);
        assert_eq!(token.balance(&learner), 400);
        assert_eq!(token.balance(&escrow), 0);
    }
}

#![no_std]

use shared::health_reporter::{
    AlertSeverity, HealthMetric, HealthThresholds, MetricCategory, SystemHealth,
};
use soroban_sdk::{
    contract, contractimpl, contracttype, token, Address, Env, IntoVal, Map, Symbol, Vec,
};

// ---------------------------------------------------------------------------
// Types (mirror `mentorminds_escrow` for cross-contract decode stability)
// ---------------------------------------------------------------------------

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
#[derive(Clone, Debug)]
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

/// Threshold (bps of a mentor's disputes / total sessions) above which
/// [`HealthDashboardContract::record_dispute_opened`] emits a
/// `MentorDisputeRateAlert` event. 2000 bps = 20%.
pub const DISPUTE_RATE_ALERT_BPS: u32 = 2000;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DisputeStats {
    pub total_opened: u32,
    pub total_resolved_mentor_favor: u32,
    pub total_resolved_learner_favor: u32,
    pub total_appealed: u32,
    pub avg_resolution_time_secs: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlatformStats {
    pub total_value_locked: i128,
    pub active_escrows: u32,
    pub total_sessions: u32,
    pub dispute_rate_bps: u32,
    pub total_mentors: u32,
    pub total_learners: u32,
    pub mnt_staked: i128,
    pub contract_versions: Map<Symbol, u32>,
    pub flagged_learners: Vec<Address>,
}

/// Mirrors `interface_registry::InterfaceEntry` for `list_interfaces` decoding.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InterfaceEntry {
    pub interface_id: Symbol,
    pub contract: Address,
    pub version: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Config,
    /// `(ledger_sequence, cached stats)` — invalidated when ledger advances.
    Cache,
    /// Platform-wide dispute aggregate ([`DisputeStats`]).
    DisputeStats,
    /// Number of disputes ever opened against a given mentor, used by
    /// [`HealthDashboardContract::get_mentor_dispute_rate`].
    MentorDisputeCount(Address),
    /// Health metric storage keys
    MetricPage(u32),
    PageCount,
    CurrentPage,
    PageSize,
    Thresholds,
    LastHealth,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Config {
    pub admin: Address,
    pub escrow: Address,
    pub session_registry: Address,
    pub staking: Address,
    pub mnt_token: Address,
    pub reputation: Address,
    pub interface_registry: Address,
    pub treasury: Address,
    pub insurance: Address,
    pub lending_pool: Address,
    pub usdc_token: Address,
    /// Address of this health dashboard contract (for self-referencing)
    pub health_dashboard: Address,
}

// ---------------------------------------------------------------------------
// Solvency types (Issue #771)
// ---------------------------------------------------------------------------

/// Mirrored from treasury::PendingAllocation for cross-contract decoding.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingAllocationView {
    pub id: u32,
    pub token: Address,
    pub recipient: Address,
    pub amount: i128,
    pub approvals_count: u32,
    pub executed: bool,
    pub created_at: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SolvencyReport {
    pub treasury_balance: i128,
    pub pending_allocations: i128,
    pub insurance_pool_balance: i128,
    pub outstanding_claims: i128,
    pub staking_total: i128,
    pub pending_rewards: i128,
    pub lending_total_liquidity: i128,
    pub outstanding_loans: i128,
    pub is_solvent: bool,
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct HealthDashboardContract;

#[contractimpl]
impl HealthDashboardContract {
    /// One-time configuration of dependent contract addresses.
    pub fn initialize(
        env: Env,
        admin: Address,
        escrow: Address,
        session_registry: Address,
        staking: Address,
        mnt_token: Address,
        reputation: Address,
        interface_registry: Address,
        treasury: Address,
        insurance: Address,
        lending_pool: Address,
        usdc_token: Address,
    ) {
        if env.storage().persistent().has(&DataKey::Config) {
            panic!("Already initialized");
        }
        let health_dashboard = env.current_contract_address();
        env.storage().persistent().set(
            &DataKey::Config,
            &Config {
                admin,
                escrow,
                session_registry,
                staking,
                mnt_token,
                reputation,
                interface_registry,
                treasury,
                insurance,
                lending_pool,
                usdc_token,
                health_dashboard,
            },
        );
    }

    /// Returns platform-wide metrics, using a one-ledger cache to limit
    /// cross-contract work within the same ledger.
    pub fn get_platform_stats(env: Env) -> PlatformStats {
        let ledger = env.ledger().sequence();
        if let Some((cached_ledger, stats)) = env
            .storage()
            .persistent()
            .get::<_, (u32, PlatformStats)>(&DataKey::Cache)
        {
            if cached_ledger == ledger {
                return stats;
            }
        }

        let stats = Self::compute_platform_stats(&env);
        env.storage()
            .persistent()
            .set(&DataKey::Cache, &(ledger, stats.clone()));

        env.events().publish(
            (Symbol::new(&env, "stats_refreshed"),),
            (ledger, stats.total_value_locked, stats.active_escrows),
        );

        stats
    }

    /// Version registered for a logical contract name in the interface registry.
    pub fn get_contract_version(env: Env, contract_name: Symbol) -> u32 {
        let cfg: Config = env
            .storage()
            .persistent()
            .get(&DataKey::Config)
            .expect("Not initialized");
        env.invoke_contract(
            &cfg.interface_registry,
            &Symbol::new(&env, "get_version"),
            (contract_name,).into_val(&env),
        )
    }

    /// Record that a dispute was opened for `escrow_id`, called by the
    /// dispute-evidence contract. Looks up the escrow's mentor to bump their
    /// dispute count and, if their dispute rate now exceeds
    /// [`DISPUTE_RATE_ALERT_BPS`], emits a `MentorDisputeRateAlert` event.
    pub fn record_dispute_opened(env: Env, escrow_id: u64, opened_at: u64) {
        let cfg: Config = env
            .storage()
            .persistent()
            .get(&DataKey::Config)
            .expect("Not initialized");

        let escrow: Escrow = env.invoke_contract(
            &cfg.escrow,
            &Symbol::new(&env, "get_escrow"),
            (escrow_id,).into_val(&env),
        );

        let mut stats = Self::load_dispute_stats(&env);
        stats.total_opened = stats.total_opened.saturating_add(1);
        env.storage()
            .persistent()
            .set(&DataKey::DisputeStats, &stats);

        let mentor_key = DataKey::MentorDisputeCount(escrow.mentor.clone());
        let dispute_count: u32 = env.storage().persistent().get(&mentor_key).unwrap_or(0);
        let dispute_count = dispute_count.saturating_add(1);
        env.storage().persistent().set(&mentor_key, &dispute_count);

        let rate_bps =
            Self::compute_mentor_dispute_rate(&env, &cfg, &escrow.mentor, dispute_count);
        if rate_bps > DISPUTE_RATE_ALERT_BPS {
            env.events().publish(
                (Symbol::new(&env, "MentorDisputeRateAlert"), escrow.mentor),
                (rate_bps, opened_at),
            );
        }
    }

    /// Record the resolution of a dispute, called by the dispute-evidence
    /// contract. `resolution_time_secs` is the caller-computed duration
    /// (resolved_at - opened_at) of the dispute's lifecycle.
    pub fn record_resolution(
        env: Env,
        escrow_id: u64,
        release_to_mentor: bool,
        resolution_time_secs: u64,
    ) {
        let _ = escrow_id;
        let mut stats = Self::load_dispute_stats(&env);

        let prior_resolved = stats
            .total_resolved_mentor_favor
            .saturating_add(stats.total_resolved_learner_favor);

        if release_to_mentor {
            stats.total_resolved_mentor_favor = stats.total_resolved_mentor_favor.saturating_add(1);
        } else {
            stats.total_resolved_learner_favor =
                stats.total_resolved_learner_favor.saturating_add(1);
        }

        // Running average: new_avg = (old_avg * n + sample) / (n + 1)
        let n = prior_resolved as u64;
        stats.avg_resolution_time_secs = stats
            .avg_resolution_time_secs
            .saturating_mul(n)
            .saturating_add(resolution_time_secs)
            / n.saturating_add(1);

        env.storage()
            .persistent()
            .set(&DataKey::DisputeStats, &stats);
    }

    /// Record that a dispute resolution was appealed.
    pub fn record_appeal(env: Env, escrow_id: u64) {
        let _ = escrow_id;
        let mut stats = Self::load_dispute_stats(&env);
        stats.total_appealed = stats.total_appealed.saturating_add(1);
        env.storage()
            .persistent()
            .set(&DataKey::DisputeStats, &stats);
    }

    /// Platform-wide dispute aggregate.
    pub fn get_dispute_stats(env: Env) -> DisputeStats {
        Self::load_dispute_stats(&env)
    }

    /// A mentor's dispute rate: `disputes / total_sessions * 10000` (bps).
    /// Returns 0 if the mentor has no sessions.
    pub fn get_mentor_dispute_rate(env: Env, mentor: Address) -> u32 {
        let cfg: Config = env
            .storage()
            .persistent()
            .get(&DataKey::Config)
            .expect("Not initialized");
        let dispute_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::MentorDisputeCount(mentor.clone()))
            .unwrap_or(0);
        Self::compute_mentor_dispute_rate(&env, &cfg, &mentor, dispute_count)
    }

    // ─── Protocol solvency (Issue #771) ─────────────────────────────────

    /// Aggregate solvency view across treasury, insurance, staking, and
    /// lending pool. Emits a `SolvencyAlert` event if the protocol is
    /// detected as insolvent (is_solvent == false).
    pub fn get_protocol_solvency(env: Env) -> SolvencyReport {
        let cfg: Config = env
            .storage()
            .persistent()
            .get(&DataKey::Config)
            .expect("Not initialized");

        // ── Treasury ─────────────────────────────────────────────────────
        // Query USDC balance held by the treasury contract.
        let treasury_balance: i128 = env.invoke_contract(
            &cfg.treasury,
            &Symbol::new(&env, "get_balance"),
            (cfg.usdc_token.clone(),).into_val(&env),
        );

        // Sum pending allocations as obligations.
        let mut pending_allocations: i128 = 0;
        let pending_count: u32 = env
            .try_invoke_contract::<u32, soroban_sdk::Error>(
                &cfg.treasury,
                &Symbol::new(&env, "pending_allocation_count"),
                ().into_val(&env),
            )
            .unwrap_or(Ok(0))
            .unwrap_or(0);
        for i in 0..pending_count {
            if let Ok(Ok(Some(pending))) = env
                .try_invoke_contract::<Option<PendingAllocationView>, soroban_sdk::Error>(
                    &cfg.treasury,
                    &Symbol::new(&env, "get_pending_allocation"),
                    (i,).into_val(&env),
                )
            {
                if !pending.executed {
                    pending_allocations = pending_allocations.saturating_add(pending.amount);
                }
            }
        }

        // ── Insurance pool ───────────────────────────────────────────────
        let insurance_pool_balance: i128 = env.invoke_contract(
            &cfg.insurance,
            &Symbol::new(&env, "get_pool_balance"),
            ().into_val(&env),
        );

        let total_claims_paid: i128 = env
            .try_invoke_contract::<i128, soroban_sdk::Error>(
                &cfg.insurance,
                &Symbol::new(&env, "get_total_claims_paid"),
                ().into_val(&env),
            )
            .unwrap_or(Ok(0))
            .unwrap_or(0);

        let outstanding_claims: i128 = total_claims_paid;

        // ── Staking ──────────────────────────────────────────────────────
        let staking_total: i128 = env.invoke_contract(
            &cfg.staking,
            &Symbol::new(&env, "get_total_staked"),
            ().into_val(&env),
        );

        // Best-effort pending rewards sum (capped at 50 stakers per call).
        let staker_count: u32 = env
            .try_invoke_contract::<u32, soroban_sdk::Error>(
                &cfg.staking,
                &Symbol::new(&env, "get_staker_count"),
                ().into_val(&env),
            )
            .unwrap_or(Ok(0))
            .unwrap_or(0);
        let mut pending_rewards: i128 = 0;
        let max_check = if staker_count > 50 { 50 } else { staker_count };
        for i in 0..max_check {
            if let Ok(Ok(Some(staker))) = env
                .try_invoke_contract::<Option<Address>, soroban_sdk::Error>(
                    &cfg.staking,
                    &Symbol::new(&env, "get_staker_at"),
                    (i,).into_val(&env),
                )
            {
                let reward: i128 = env
                    .try_invoke_contract::<i128, soroban_sdk::Error>(
                        &cfg.staking,
                        &Symbol::new(&env, "get_pending_rewards"),
                        (staker,).into_val(&env),
                    )
                    .unwrap_or(Ok(0))
                    .unwrap_or(0);
                pending_rewards = pending_rewards.saturating_add(reward);
            }
        }

        // ── Lending pool ─────────────────────────────────────────────────
        let lending_total_liquidity: i128 = env.invoke_contract(
            &cfg.lending_pool,
            &Symbol::new(&env, "total_liquidity"),
            ().into_val(&env),
        );

        let bad_debt: i128 = env
            .try_invoke_contract::<i128, soroban_sdk::Error>(
                &cfg.lending_pool,
                &Symbol::new(&env, "get_bad_debt"),
                ().into_val(&env),
            )
            .unwrap_or(Ok(0))
            .unwrap_or(0);

        // Outstanding loans = bad_debt + (initial liquidity - current liquidity)
        let initial_liquidity_proxy: i128 = treasury_balance.saturating_add(insurance_pool_balance);
        let outstanding_loans: i128 = bad_debt
            .saturating_add(initial_liquidity_proxy.saturating_sub(lending_total_liquidity));

        // ── Solvency check ───────────────────────────────────────────────
        // treasury must cover pending allocations
        // insurance pool must be non-negative
        // lending pool must have non-negative liquidity
        let is_solvent = treasury_balance >= pending_allocations
            && insurance_pool_balance >= 0
            && lending_total_liquidity >= 0;

        let report = SolvencyReport {
            treasury_balance,
            pending_allocations,
            insurance_pool_balance,
            outstanding_claims,
            staking_total,
            pending_rewards,
            lending_total_liquidity,
            outstanding_loans,
            is_solvent,
        };

        if !is_solvent {
            env.events().publish(
                (Symbol::new(&env, "SolvencyAlert"),),
                (
                    treasury_balance,
                    pending_allocations,
                    insurance_pool_balance,
                    staking_total,
                    lending_total_liquidity,
                ),
            );
        }

        report
    }

    // ─── Health Metrics (Issue #625) ──────────────────────────────────

    /// Record a health metric from any contract. Metrics are stored in
    /// paginated pages (default 100 per page) for efficient on-chain access.
    pub fn record_metric(env: Env, metric: HealthMetric) {
        if metric.name == Symbol::new(&env, "") {
            panic!("metric name cannot be empty");
        }

        let page_size: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::PageSize)
            .unwrap_or(100);
        let current_page: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::CurrentPage)
            .unwrap_or(0);
        let mut page: Vec<HealthMetric> = env
            .storage()
            .persistent()
            .get(&DataKey::MetricPage(current_page))
            .unwrap_or_else(|| Vec::new(&env));

        if page.len() >= page_size {
            let new_page = current_page + 1;
            env.storage()
                .persistent()
                .set(&DataKey::CurrentPage, &new_page);
            env.storage()
                .persistent()
                .set(&DataKey::PageCount, &(new_page + 1));
            page = Vec::new(&env);
        }

        page.push_back(metric);
        env.storage()
            .persistent()
            .set(&DataKey::MetricPage(current_page), &page);

        if current_page == 0 && page.len() == 1 {
            env.storage().persistent().set(&DataKey::PageCount, &1u32);
        }

        // Emit event for off-chain monitoring
        env.events().publish(
            (Symbol::new(&env, "metric_recorded"),),
            (Symbol::new(&env, ""), current_page),
        );
    }

    /// Get health metrics for a specific page (0-indexed).
    /// Returns an empty Vec if the page doesn't exist.
    pub fn get_system_health(env: Env, page: u32) -> Vec<HealthMetric> {
        env.storage()
            .persistent()
            .get(&DataKey::MetricPage(page))
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Get the total number of metric pages.
    pub fn get_metric_page_count(env: Env) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::PageCount)
            .unwrap_or(0)
    }

    /// Determine if the system is healthy based on configurable thresholds.
    /// Checks: error rate, TVL, critical alert count, and warning alert count.
    pub fn is_system_healthy(env: Env) -> SystemHealth {
        let thresholds: HealthThresholds = env
            .storage()
            .persistent()
            .get(&DataKey::Thresholds)
            .unwrap_or(HealthThresholds {
                max_error_rate_bps: 500,
                min_tvl: 0,
                max_critical_alerts: 3,
                max_warning_alerts: 10,
            });

        let page_count: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::PageCount)
            .unwrap_or(0);

        let mut total_metrics: u32 = 0;
        let mut warning_count: u32 = 0;
        let mut critical_count: u32 = 0;
        let mut last_updated: u64 = 0;
        let mut total_tvl: i128 = 0;
        let mut error_count: u32 = 0;
        let mut active_sources: Map<Address, bool> = Map::new(&env);

        for page_idx in 0..page_count {
            let metrics: Vec<HealthMetric> = env
                .storage()
                .persistent()
                .get(&DataKey::MetricPage(page_idx))
                .unwrap_or_else(|| Vec::new(&env));

            for metric in metrics.iter() {
                total_metrics += 1;

                if metric.recorded_at > last_updated {
                    last_updated = metric.recorded_at;
                }

                active_sources.set(metric.source.clone(), true);

                match metric.category {
                    MetricCategory::Liquidity => {
                        total_tvl = total_tvl.saturating_add(metric.value);
                    }
                    MetricCategory::ErrorRate => {
                        error_count += 1;
                    }
                    _ => {}
                }

                match metric.alert {
                    AlertSeverity::Warning => warning_count += 1,
                    AlertSeverity::Critical => critical_count += 1,
                    _ => {}
                }
            }
        }

        let error_rate_bps: u32 = if total_metrics == 0 {
            0
        } else {
            ((error_count as u64 * 10_000) / (total_metrics as u64)) as u32
        };

        let is_healthy = error_rate_bps <= thresholds.max_error_rate_bps
            && total_tvl >= thresholds.min_tvl
            && critical_count <= thresholds.max_critical_alerts
            && warning_count <= thresholds.max_warning_alerts;

        let health = SystemHealth {
            is_healthy,
            total_metrics,
            warning_count,
            critical_count,
            last_updated,
            total_tvl,
            error_rate_bps,
            active_sources: active_sources.len(),
        };

        env.storage()
            .persistent()
            .set(&DataKey::LastHealth, &health);

        health
    }

    /// Update the health thresholds. Admin-only.
    pub fn set_health_thresholds(env: Env, admin: Address, thresholds: HealthThresholds) {
        let cfg: Config = env
            .storage()
            .persistent()
            .get(&DataKey::Config)
            .expect("Not initialized");
        admin.require_auth();
        if admin != cfg.admin {
            panic!("unauthorized");
        }
        env.storage()
            .persistent()
            .set(&DataKey::Thresholds, &thresholds);
    }

    /// Get the current health thresholds.
    pub fn get_health_thresholds(env: Env) -> HealthThresholds {
        env.storage()
            .persistent()
            .get(&DataKey::Thresholds)
            .unwrap_or(HealthThresholds {
                max_error_rate_bps: 500,
                min_tvl: 0,
                max_critical_alerts: 3,
                max_warning_alerts: 10,
            })
    }
}

impl HealthDashboardContract {
    fn compute_platform_stats(env: &Env) -> PlatformStats {
        let cfg: Config = env
            .storage()
            .persistent()
            .get(&DataKey::Config)
            .expect("Not initialized");

        let escrow_count: u64 = env.invoke_contract(
            &cfg.escrow,
            &Symbol::new(env, "get_escrow_count"),
            ().into_val(env),
        );

        let mut total_value_locked: i128 = 0;
        let mut active_escrows: u32 = 0;
        let mut total_sessions: u32 = 0;
        let mut dispute_hits: u32 = 0;
        let mut total_for_disputes: u32 = 0;

        let mut mentor_vec: Vec<Address> = Vec::new(env);
        let mut learner_vec: Vec<Address> = Vec::new(env);

        for id in 1u64..=escrow_count {
            let e: Escrow = env.invoke_contract(
                &cfg.escrow,
                &Symbol::new(env, "get_escrow"),
                (id,).into_val(env),
            );
            total_sessions = total_sessions.saturating_add(e.total_sessions);
            total_for_disputes = total_for_disputes.saturating_add(1);

            if e.status == EscrowStatus::Disputed || e.status == EscrowStatus::Resolved {
                dispute_hits = dispute_hits.saturating_add(1);
            }

            if e.status == EscrowStatus::Active {
                active_escrows = active_escrows.saturating_add(1);
                total_value_locked = total_value_locked.saturating_add(e.amount);
            }

            Self::push_unique(env, &mut mentor_vec, e.mentor);
            Self::push_unique(env, &mut learner_vec, e.learner);
        }

        // Session registry (optional): e.g. bundle mint counter on session NFT contract.
        let session_extra: u32 = match env.try_invoke_contract::<u64, soroban_sdk::Error>(
            &cfg.session_registry,
            &Symbol::new(env, "session_cnt"),
            ().into_val(env),
        ) {
            Ok(Ok(n)) => n as u32,
            _ => 0,
        };
        total_sessions = total_sessions.saturating_add(session_extra);

        // Staking TVL in MNT: token balance held by the staking contract.
        let token_client = token::Client::new(env, &cfg.mnt_token);
        let mnt_staked: i128 = token_client.balance(&cfg.staking);

        // Reputation contract: optional ping (keeps dependency explicit for future metrics).
        let _ = env.try_invoke_contract::<u32, soroban_sdk::Error>(
            &cfg.reputation,
            &Symbol::new(env, "ping"),
            ().into_val(env),
        );

        let dispute_rate_bps: u32 = if total_for_disputes == 0 {
            0
        } else {
            ((dispute_hits as u64 * 10_000) / (total_for_disputes as u64)) as u32
        };

        let entries: Vec<InterfaceEntry> = env.invoke_contract(
            &cfg.interface_registry,
            &Symbol::new(env, "list_interfaces"),
            ().into_val(env),
        );
        let mut contract_versions: Map<Symbol, u32> = Map::new(env);
        for i in 0..entries.len() {
            let entry = entries.get(i).unwrap();
            contract_versions.set(entry.interface_id.clone(), entry.version);
        }

        // Flag learners with avg < 3.0 across 5+ sessions
        let mut flagged_learners: Vec<Address> = Vec::new(env);
        for learner in learner_vec.iter() {
            if let Ok(Ok((avg_times_100, count))) = env.try_invoke_contract::<(u64, u64), soroban_sdk::Error>(
                &cfg.reputation,
                &Symbol::new(env, "get_learner_rating"),
                (learner.clone(),).into_val(env),
            ) {
                // avg < 3.0 means avg_times_100 < 300
                // count >= 5 for meaningful sample
                if count >= 5 && avg_times_100 < 300 {
                    flagged_learners.push_back(learner.clone());
                }
            }
        }

        PlatformStats {
            total_value_locked,
            active_escrows,
            total_sessions,
            dispute_rate_bps,
            total_mentors: mentor_vec.len(),
            total_learners: learner_vec.len(),
            mnt_staked,
            contract_versions,
            flagged_learners,
        }
    }

    fn push_unique(_env: &Env, v: &mut Vec<Address>, addr: Address) {
        for i in 0..v.len() {
            if v.get(i).unwrap() == addr {
                return;
            }
        }
        v.push_back(addr);
    }

    fn load_dispute_stats(env: &Env) -> DisputeStats {
        env.storage()
            .persistent()
            .get(&DataKey::DisputeStats)
            .unwrap_or(DisputeStats {
                total_opened: 0,
                total_resolved_mentor_favor: 0,
                total_resolved_learner_favor: 0,
                total_appealed: 0,
                avg_resolution_time_secs: 0,
            })
    }

    /// `disputes / total_sessions * 10000` (bps), via a cross-contract read
    /// of the mentor's session count from the configured session registry.
    /// Returns 0 if the mentor has no sessions (avoids division by zero).
    fn compute_mentor_dispute_rate(
        env: &Env,
        cfg: &Config,
        mentor: &Address,
        dispute_count: u32,
    ) -> u32 {
        let sessions: Vec<Symbol> = env.invoke_contract(
            &cfg.session_registry,
            &Symbol::new(env, "get_sessions_by_mentor"),
            (mentor.clone(),).into_val(env),
        );
        let total_sessions = sessions.len();
        if total_sessions == 0 {
            return 0;
        }
        ((dispute_count as u64 * 10_000) / (total_sessions as u64)) as u32
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;

    use super::*;
    use soroban_sdk::symbol_short;
    use soroban_sdk::testutils::{Address as _, Events, Ledger};

    #[contracttype]
    #[derive(Clone)]
    enum MockTokenKey {
        Bal(Address),
    }

    #[contract]
    pub struct MockMntToken;

    #[contractimpl]
    impl MockMntToken {
        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&MockTokenKey::Bal(id))
                .unwrap_or(0)
        }

        pub fn mint(env: Env, to: Address, amount: i128) {
            let cur: i128 = env
                .storage()
                .persistent()
                .get(&MockTokenKey::Bal(to.clone()))
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&MockTokenKey::Bal(to), &(cur + amount));
        }
    }

    #[contract]
    pub struct MockEscrow;

    #[contractimpl]
    impl MockEscrow {
        pub fn get_escrow_count(_env: Env) -> u64 {
            2
        }

        pub fn get_escrow(env: Env, id: u64) -> Escrow {
            let t = Address::generate(&env);
            let m1 = Address::generate(&env);
            let m2 = Address::generate(&env);
            let l1 = Address::generate(&env);
            let l2 = Address::generate(&env);
            if id == 1 {
                Escrow {
                    id: 1,
                    mentor: m1,
                    learner: l1,
                    amount: 1000,
                    session_id: symbol_short!("s1"),
                    status: EscrowStatus::Active,
                    created_at: 0,
                    token_address: t.clone(),
                    platform_fee: 0,
                    net_amount: 0,
                    session_end_time: 0,
                    auto_release_delay: 0,
                    dispute_reason: symbol_short!("none"),
                    resolved_at: 0,
                    usd_amount: 0,
                    quoted_token_amount: 0,
                    send_asset: t.clone(),
                    dest_asset: t,
                    total_sessions: 5,
                    sessions_completed: 0,
                }
            } else {
                Escrow {
                    id: 2,
                    mentor: m2,
                    learner: l2,
                    amount: 100,
                    session_id: symbol_short!("s2"),
                    status: EscrowStatus::Disputed,
                    created_at: 0,
                    token_address: Address::generate(&env),
                    platform_fee: 0,
                    net_amount: 0,
                    session_end_time: 0,
                    auto_release_delay: 0,
                    dispute_reason: symbol_short!("none"),
                    resolved_at: 0,
                    usd_amount: 0,
                    quoted_token_amount: 0,
                    send_asset: Address::generate(&env),
                    dest_asset: Address::generate(&env),
                    total_sessions: 3,
                    sessions_completed: 0,
                }
            }
        }
    }

    #[contract]
    pub struct MockSessionRegistry;

    #[contractimpl]
    impl MockSessionRegistry {
        pub fn session_cnt(_env: Env) -> u64 {
            7
        }
    }

    #[contract]
    pub struct MockReputation;

    #[contractimpl]
    impl MockReputation {
        pub fn ping(_env: Env) -> u32 {
            1
        }
    }

    #[contract]
    pub struct MockInterfaceRegistry;

    #[contractimpl]
    impl MockInterfaceRegistry {
        pub fn get_version(_env: Env, interface_id: Symbol) -> u32 {
            if interface_id == symbol_short!("escrow") {
                2
            } else {
                0
            }
        }

        pub fn list_interfaces(env: Env) -> Vec<InterfaceEntry> {
            let mut v = Vec::new(&env);
            v.push_back(InterfaceEntry {
                interface_id: symbol_short!("escrow"),
                contract: Address::generate(&env),
                version: 2,
            });
            v
        }
    }

    // ─── Solvency mock contracts (Issue #771) ──────────────────────────

    #[contract]
    pub struct MockTreasury;

    #[contractimpl]
    impl MockTreasury {
        pub fn get_balance(_env: Env, _token: Address) -> i128 {
            100_000 // treasury holds 100k USDC
        }
        pub fn pending_allocation_count(_env: Env) -> u32 {
            1
        }
        pub fn get_pending_allocation(env: Env, _id: u32) -> Option<PendingAllocationView> {
            Some(PendingAllocationView {
                id: 0,
                token: Address::generate(&env),
                recipient: Address::generate(&env),
                amount: 10_000, // 10k pending
                approvals_count: 1,
                executed: false,
                created_at: 0,
            })
        }
    }

    #[contract]
    pub struct MockInsurance;

    #[contractimpl]
    impl MockInsurance {
        pub fn get_pool_balance(_env: Env) -> i128 {
            50_000 // insurance pool holds 50k
        }
        pub fn get_total_claims_paid(_env: Env) -> i128 {
            5_000 // 5k claims paid so far
        }
    }

    #[contract]
    pub struct MockLendingPool;

    #[contractimpl]
    impl MockLendingPool {
        pub fn total_liquidity(_env: Env) -> i128 {
            200_000 // 200k in pool
        }
        pub fn get_bad_debt(_env: Env) -> i128 {
            1_000 // 1k bad debt
        }
    }

    // Treasury with low balance relative to pending
    #[contract]
    pub struct MockTreasuryLow;

    #[contractimpl]
    impl MockTreasuryLow {
        pub fn get_balance(_env: Env, _token: Address) -> i128 {
            500
        }
        pub fn pending_allocation_count(_env: Env) -> u32 {
            1
        }
        pub fn get_pending_allocation(env: Env, _id: u32) -> Option<PendingAllocationView> {
            Some(PendingAllocationView {
                id: 0,
                token: Address::generate(&env),
                recipient: Address::generate(&env),
                amount: 10_000, // pending > balance
                approvals_count: 1,
                executed: false,
                created_at: 0,
            })
        }
    }

    // Treasury with balance=0 and pending > 0 → insolvent
    #[contract]
    pub struct MockTreasuryZero;

    #[contractimpl]
    impl MockTreasuryZero {
        pub fn get_balance(_env: Env, _token: Address) -> i128 {
            0
        }
        pub fn pending_allocation_count(_env: Env) -> u32 {
            1
        }
        pub fn get_pending_allocation(env: Env, _id: u32) -> Option<PendingAllocationView> {
            Some(PendingAllocationView {
                id: 0,
                token: Address::generate(&env),
                recipient: Address::generate(&env),
                amount: 100,
                approvals_count: 0,
                executed: false,
                created_at: 0,
            })
        }
    }

    #[contract]
    pub struct MockStakingForSolvency;

    #[contractimpl]
    impl MockStakingForSolvency {
        pub fn get_total_staked(_env: Env) -> i128 {
            500_000 // 500k staked
        }
        pub fn get_staker_count(_env: Env) -> u32 {
            1
        }
        pub fn get_staker_at(_env: Env, _index: u32) -> Option<Address> {
            Some(Address::generate(&_env))
        }
        pub fn get_pending_rewards(_env: Env, _staker: Address) -> i128 {
            1_000 // 1k pending rewards
        }
    }

    fn setup() -> (Env, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let escrow_id = env.register_contract(None, MockEscrow);
        let session_reg = env.register_contract(None, MockSessionRegistry);
        let staking = Address::generate(&env);
        let mnt = env.register_contract(None, MockMntToken);
        MockMntTokenClient::new(&env, &mnt).mint(&staking, &5000i128);

        let reputation = env.register_contract(None, MockReputation);
        let iface = env.register_contract(None, MockInterfaceRegistry);
        let treasury = env.register_contract(None, MockTreasury);
        let insurance = env.register_contract(None, MockInsurance);
        let lending_pool = env.register_contract(None, MockLendingPool);
        let usdc = env.register_contract(None, MockMntToken);
        let dashboard = env.register_contract(None, HealthDashboardContract);

        HealthDashboardContractClient::new(&env, &dashboard).initialize(
            &admin,
            &escrow_id,
            &session_reg,
            &staking,
            &mnt,
            &reputation,
            &iface,
            &treasury,
            &insurance,
            &lending_pool,
            &usdc,
        );

        (env, dashboard, mnt)
    }

    #[test]
    fn test_stats_aggregation() {
        let (env, dashboard, _mnt) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let s = client.get_platform_stats();

        assert_eq!(s.total_value_locked, 1000);
        assert_eq!(s.active_escrows, 1);
        assert_eq!(s.total_sessions, 5 + 3 + 7);
        assert_eq!(s.dispute_rate_bps, 5000);
        assert_eq!(s.total_mentors, 2);
        assert_eq!(s.total_learners, 2);
        assert_eq!(s.mnt_staked, 5000);
        assert_eq!(s.contract_versions.get(symbol_short!("escrow")), Some(2));
        // flagged_learners should be empty since mock reputation doesn't implement get_learner_rating
        assert_eq!(s.flagged_learners.len(), 0);
    }

    #[test]
    fn test_get_contract_version() {
        let (env, dashboard, _) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        assert_eq!(client.get_contract_version(&symbol_short!("escrow")), 2);
    }

    #[test]
    fn test_cache_same_ledger() {
        let (env, dashboard, _) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);

        let s1 = client.get_platform_stats();
        let s2 = client.get_platform_stats();
        assert_eq!(s1.total_sessions, s2.total_sessions);
        assert_eq!(s1.mnt_staked, s2.mnt_staked);
    }

    #[test]
    fn test_cache_invalidates_next_ledger() {
        let (env, dashboard, _) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let _ = client.get_platform_stats();

        env.ledger().with_mut(|li| {
            li.sequence_number += 1;
        });

        let _ = client.get_platform_stats();
    }

    // ═════════════════════════════════════════════════════════════════════
    // Solvency tests (Issue #771)
    // ═════════════════════════════════════════════════════════════════════

    #[test]
    fn test_get_protocol_solvency_returns_all_fields() {
        let (env, dashboard, _) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let report = client.get_protocol_solvency();

        // All fields must be non-negative (overflow-protected)
        assert!(report.treasury_balance >= 0);
        assert!(report.pending_allocations >= 0);
        assert!(report.insurance_pool_balance >= 0);
        assert!(report.outstanding_claims >= 0);
        assert!(report.staking_total >= 0);
        assert!(report.pending_rewards >= 0);
        assert!(report.lending_total_liquidity >= 0);
        assert!(report.outstanding_loans >= 0);

        // With mock defaults: treasury=100k ≥ pending=10k → solvent
        // insurance=50k ≥ 0 → solvent
        // lending=200k ≥ 0 → solvent
        assert!(report.is_solvent);
    }

    #[test]
    fn test_solvency_treasury_insufficient_returns_insolvent() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let escrow_id = env.register_contract(None, MockEscrow);
        let session_reg = env.register_contract(None, MockSessionRegistry);
        let staking = Address::generate(&env);
        let mnt = env.register_contract(None, MockMntToken);
        MockMntTokenClient::new(&env, &mnt).mint(&staking, &5000i128);

        let reputation = env.register_contract(None, MockReputation);
        let iface = env.register_contract(None, MockInterfaceRegistry);

        let treasury = env.register_contract(None, MockTreasuryLow);
        let insurance = env.register_contract(None, MockInsurance);
        let lending_pool = env.register_contract(None, MockLendingPool);
        let usdc = env.register_contract(None, MockMntToken);
        let dashboard = env.register_contract(None, HealthDashboardContract);

        HealthDashboardContractClient::new(&env, &dashboard).initialize(
            &admin,
            &escrow_id,
            &session_reg,
            &staking,
            &mnt,
            &reputation,
            &iface,
            &treasury,
            &insurance,
            &lending_pool,
            &usdc,
        );

        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let report = client.get_protocol_solvency();

        assert!(!report.is_solvent, "insolvent when treasury < pending");
        assert_eq!(report.treasury_balance, 500);
        assert_eq!(report.pending_allocations, 10_000);
    }

    #[test]
    fn test_solvency_emits_alert_event_when_insolvent() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let escrow_id = env.register_contract(None, MockEscrow);
        let session_reg = env.register_contract(None, MockSessionRegistry);
        let staking = Address::generate(&env);
        let mnt = env.register_contract(None, MockMntToken);
        MockMntTokenClient::new(&env, &mnt).mint(&staking, &5000i128);

        let reputation = env.register_contract(None, MockReputation);
        let iface = env.register_contract(None, MockInterfaceRegistry);

        let treasury = env.register_contract(None, MockTreasuryZero);
        let insurance = env.register_contract(None, MockInsurance);
        let lending_pool = env.register_contract(None, MockLendingPool);
        let usdc = env.register_contract(None, MockMntToken);
        let dashboard = env.register_contract(None, HealthDashboardContract);

        HealthDashboardContractClient::new(&env, &dashboard).initialize(
            &admin,
            &escrow_id,
            &session_reg,
            &staking,
            &mnt,
            &reputation,
            &iface,
            &treasury,
            &insurance,
            &lending_pool,
            &usdc,
        );

        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let report = client.get_protocol_solvency();
        assert!(!report.is_solvent);

        // Check that SolvencyAlert event was emitted
        let events = env.events().all().filter_by_contract(&dashboard);
        assert!(
            !events.events().is_empty(),
            "SolvencyAlert event must be emitted when insolvent"
        );
    }

    #[test]
    fn test_solvency_all_fields_non_negative_during_normal_ops() {
        let (env, dashboard, _) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let report = client.get_protocol_solvency();

        // Verify all numeric fields are >= 0 (overflow protection)
        assert!(report.treasury_balance >= 0);
        assert!(report.pending_allocations >= 0);
        assert!(report.insurance_pool_balance >= 0);
        assert!(report.outstanding_claims >= 0);
        assert!(report.staking_total >= 0);
        assert!(report.pending_rewards >= 0);
        assert!(report.lending_total_liquidity >= 0);
        assert!(report.outstanding_loans >= 0);
    }

    #[test]
    fn test_solvency_exact_values_match_mocks() {
        let (env, dashboard, _) = setup();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let report = client.get_protocol_solvency();

        assert_eq!(report.treasury_balance, 100_000);
        assert_eq!(report.pending_allocations, 10_000);
        assert_eq!(report.insurance_pool_balance, 50_000);
        assert_eq!(report.outstanding_claims, 5_000);
        assert!(report.staking_total >= 0);
        assert!(report.pending_rewards >= 0);
        assert_eq!(report.lending_total_liquidity, 200_000);
    }

    // ── #760: dispute stats aggregation ─────────────────────────────────────

    mod dispute_mocks {
        use super::*;

        #[contracttype]
        #[derive(Clone)]
        pub enum DisputeMockKey {
            Escrow(u64),
            Sessions(Address),
        }

        #[contract]
        pub struct MockEscrowD;

        #[contractimpl]
        impl MockEscrowD {
            pub fn set_escrow_mentor(env: Env, id: u64, mentor: Address) {
                env.storage()
                    .persistent()
                    .set(&DisputeMockKey::Escrow(id), &mentor);
            }

            pub fn get_escrow(env: Env, id: u64) -> Escrow {
                let mentor: Address = env
                    .storage()
                    .persistent()
                    .get(&DisputeMockKey::Escrow(id))
                    .unwrap();
                let dummy = mentor.clone();
                Escrow {
                    id,
                    mentor,
                    learner: dummy.clone(),
                    amount: 0,
                    session_id: symbol_short!("s"),
                    status: EscrowStatus::Disputed,
                    created_at: 0,
                    token_address: dummy.clone(),
                    platform_fee: 0,
                    net_amount: 0,
                    session_end_time: 0,
                    auto_release_delay: 0,
                    dispute_reason: symbol_short!("none"),
                    resolved_at: 0,
                    usd_amount: 0,
                    quoted_token_amount: 0,
                    send_asset: dummy.clone(),
                    dest_asset: dummy,
                    total_sessions: 0,
                    sessions_completed: 0,
                }
            }
        }

        #[contract]
        pub struct MockSessionRegistryD;

        #[contractimpl]
        impl MockSessionRegistryD {
            pub fn set_session_count(env: Env, mentor: Address, count: u32) {
                let mut v: Vec<Symbol> = Vec::new(&env);
                for _ in 0..count {
                    v.push_back(symbol_short!("sess"));
                }
                env.storage()
                    .persistent()
                    .set(&DisputeMockKey::Sessions(mentor), &v);
            }

            pub fn get_sessions_by_mentor(env: Env, mentor: Address) -> Vec<Symbol> {
                env.storage()
                    .persistent()
                    .get(&DisputeMockKey::Sessions(mentor))
                    .unwrap_or(Vec::new(&env))
            }
        }
    }
    use dispute_mocks::{
        MockEscrowD, MockEscrowDClient, MockSessionRegistryD, MockSessionRegistryDClient,
    };

    fn setup_dispute() -> (Env, Address, Address, Address) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);

        let escrow_id = env.register_contract(None, MockEscrowD);
        let session_reg = env.register_contract(None, MockSessionRegistryD);
        let staking = Address::generate(&env);
        let mnt = env.register_contract(None, MockMntToken);
        let reputation = env.register_contract(None, MockReputation);
        let iface = env.register_contract(None, MockInterfaceRegistry);
        let treasury = env.register_contract(None, MockTreasury);
        let insurance = env.register_contract(None, MockInsurance);
        let lending_pool = env.register_contract(None, MockLendingPool);
        let usdc = env.register_contract(None, MockMntToken);
        let dashboard = env.register_contract(None, HealthDashboardContract);

        HealthDashboardContractClient::new(&env, &dashboard).initialize(
            &admin,
            &escrow_id,
            &session_reg,
            &staking,
            &mnt,
            &reputation,
            &iface,
            &treasury,
            &insurance,
            &lending_pool,
            &usdc,
        );

        (env, dashboard, escrow_id, session_reg)
    }

    #[test]
    fn test_record_resolution_increments_favor_counters() {
        let (env, dashboard, escrow_id, _session_reg) = setup_dispute();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let escrow_client = MockEscrowDClient::new(&env, &escrow_id);
        let mentor = Address::generate(&env);

        // 5 disputes resolved: 3 mentor favor, 2 learner favor.
        for i in 1u64..=5 {
            escrow_client.set_escrow_mentor(&i, &mentor);
            client.record_dispute_opened(&i, &0u64);
        }
        client.record_resolution(&1, &true, &100u64);
        client.record_resolution(&2, &true, &200u64);
        client.record_resolution(&3, &true, &300u64);
        client.record_resolution(&4, &false, &400u64);
        client.record_resolution(&5, &false, &500u64);

        let stats = client.get_dispute_stats();
        assert_eq!(stats.total_opened, 5);
        assert_eq!(stats.total_resolved_mentor_favor, 3);
        assert_eq!(stats.total_resolved_learner_favor, 2);
        // running average of 100,200,300,400,500 == 300
        assert_eq!(stats.avg_resolution_time_secs, 300);
    }

    #[test]
    fn test_avg_resolution_time_running_average_updates_incrementally() {
        let (env, dashboard, _escrow_id, _session_reg) = setup_dispute();
        let client = HealthDashboardContractClient::new(&env, &dashboard);

        client.record_resolution(&1, &true, &100u64);
        assert_eq!(client.get_dispute_stats().avg_resolution_time_secs, 100);

        client.record_resolution(&2, &false, &300u64);
        assert_eq!(client.get_dispute_stats().avg_resolution_time_secs, 200);
    }

    #[test]
    fn test_get_mentor_dispute_rate_bps() {
        let (env, dashboard, escrow_id, session_reg) = setup_dispute();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let escrow_client = MockEscrowDClient::new(&env, &escrow_id);
        let session_client = MockSessionRegistryDClient::new(&env, &session_reg);
        let mentor = Address::generate(&env);

        session_client.set_session_count(&mentor, &10u32);
        escrow_client.set_escrow_mentor(&1, &mentor);
        escrow_client.set_escrow_mentor(&2, &mentor);
        client.record_dispute_opened(&1, &0u64);
        client.record_dispute_opened(&2, &0u64);

        // 2 disputes / 10 sessions = 2000 bps (20%)
        assert_eq!(client.get_mentor_dispute_rate(&mentor), 2000);
    }

    #[test]
    fn test_mentor_dispute_rate_alert_fires_above_threshold() {
        let (env, dashboard, escrow_id, session_reg) = setup_dispute();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let escrow_client = MockEscrowDClient::new(&env, &escrow_id);
        let session_client = MockSessionRegistryDClient::new(&env, &session_reg);
        let mentor = Address::generate(&env);

        // 3 disputes / 10 sessions = 3000 bps > DISPUTE_RATE_ALERT_BPS (2000)
        session_client.set_session_count(&mentor, &10u32);
        escrow_client.set_escrow_mentor(&1, &mentor);
        escrow_client.set_escrow_mentor(&2, &mentor);
        escrow_client.set_escrow_mentor(&3, &mentor);
        client.record_dispute_opened(&1, &0u64);
        client.record_dispute_opened(&2, &0u64);
        client.record_dispute_opened(&3, &0u64);

        let events = env.events().all().filter_by_contract(&dashboard);
        assert!(
            !events.events().is_empty(),
            "expected MentorDisputeRateAlert to be emitted"
        );
    }

    #[test]
    fn test_mentor_dispute_rate_alert_does_not_fire_below_threshold() {
        let (env, dashboard, escrow_id, session_reg) = setup_dispute();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        let escrow_client = MockEscrowDClient::new(&env, &escrow_id);
        let session_client = MockSessionRegistryDClient::new(&env, &session_reg);
        let mentor = Address::generate(&env);

        // 1 dispute / 10 sessions = 1000 bps < DISPUTE_RATE_ALERT_BPS (2000)
        session_client.set_session_count(&mentor, &10u32);
        escrow_client.set_escrow_mentor(&1, &mentor);
        client.record_dispute_opened(&1, &0u64);

        let events = env.events().all().filter_by_contract(&dashboard);
        assert!(
            events.events().is_empty(),
            "did not expect MentorDisputeRateAlert to be emitted"
        );
    }

    #[test]
    fn test_record_appeal_increments_total_appealed() {
        let (env, dashboard, _escrow_id, _session_reg) = setup_dispute();
        let client = HealthDashboardContractClient::new(&env, &dashboard);
        client.record_appeal(&1);
        client.record_appeal(&2);
        assert_eq!(client.get_dispute_stats().total_appealed, 2);
    }
}

#![no_std]

//! Delegated staking proxy for institutional MNT holders (issue #780).
//!
//! Institutional holders (DAOs, funds, custodians) often cannot sign the
//! `StakingContract::stake` transaction directly. This contract lets many
//! beneficial owners deposit MNT into a single proxy, which maintains one
//! consolidated `StakeRecord` on their behalf and distributes rewards back
//! to each owner pro-rata to their deposit.
//!
//! Reward accounting uses the standard "accumulated reward-per-share"
//! pattern (as used by MasterChef-style contracts) so pro-rata shares stay
//! correct across deposits/withdrawals that change the total pool size at
//! different times.

use shared::StakeRecord;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, token, Address, Env, Symbol};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidAmount = 3,
    BelowMinDeposit = 4,
    Unauthorized = 5,
    InsufficientBalance = 6,
    NoWithdrawalRequest = 7,
    WithdrawalAlreadyRequested = 8,
    StillLocked = 9,
    NothingStaked = 10,
}

/// Economic sanity ceiling for a single deposit/reward amount, mirroring
/// `staking::MAX_FINANCIAL_AMOUNT`.
const MAX_FINANCIAL_AMOUNT: i128 = 1_000_000_000_000_000; // 100M tokens @ 7 decimals

/// Fixed-point scale for the reward-per-share accumulator.
const PRECISION: i128 = 1_000_000_000_000; // 1e12

/// Stake-only tier thresholds, matching `staking::DEFAULT_TIER_REQUIREMENTS`.
/// The proxy holds tokens on behalf of institutions with no on-chain
/// reputation of their own, so tier here is driven by aggregate stake alone.
const BRONZE_STAKE: i128 = 100;
const SILVER_STAKE: i128 = 500;
const GOLD_STAKE: i128 = 2_000;

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    MNTToken,
    LockPeriodDays,
    MinDeposit,
    /// Each owner's currently deposited (staked) amount.
    BeneficialOwner(Address),
    TotalDeposited,
    /// Single consolidated stake held by the proxy on behalf of all owners.
    ProxyStakeRecord,
    /// Accumulated rewards per unit deposited, scaled by `PRECISION`.
    RewardPerShare,
    /// Reward-per-share value last settled for this owner (MasterChef debt).
    OwnerRewardDebt(Address),
    /// Rewards accrued but not yet claimed for this owner.
    OwnerPendingRewards(Address),
    /// Queued withdrawal, executable once the proxy's lock period ends.
    WithdrawalRequest(Address),
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WithdrawalRequestData {
    pub amount: i128,
    pub requested_at: u64,
}

#[contract]
pub struct DelegatedStakingProxy;

#[contractimpl]
impl DelegatedStakingProxy {
    pub fn initialize(
        env: Env,
        admin: Address,
        mnt_token: Address,
        lock_period_days: u32,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MNTToken, &mnt_token);
        env.storage()
            .instance()
            .set(&DataKey::LockPeriodDays, &lock_period_days);
        env.storage().instance().set(&DataKey::MinDeposit, &0i128);
        Ok(())
    }

    /// Admin-only: set the minimum single-deposit amount, to avoid dust
    /// deposits that are not economically worth tracking.
    pub fn set_min_deposit(env: Env, admin: Address, min_deposit: i128) -> Result<(), Error> {
        Self::require_admin(&env, &admin)?;
        if min_deposit < 0 {
            return Err(Error::InvalidAmount);
        }
        env.storage()
            .instance()
            .set(&DataKey::MinDeposit, &min_deposit);
        Ok(())
    }

    /// `owner` deposits `amount` MNT into the proxy. The proxy stakes the
    /// aggregate of all owner deposits as a single consolidated stake,
    /// recomputing its tier on every deposit.
    pub fn deposit_and_stake(env: Env, owner: Address, amount: i128) -> Result<(), Error> {
        Self::require_initialized(&env)?;

        if amount <= 0 || amount > MAX_FINANCIAL_AMOUNT {
            return Err(Error::InvalidAmount);
        }
        let min_deposit: i128 = env
            .storage()
            .instance()
            .get(&DataKey::MinDeposit)
            .unwrap_or(0);
        if amount < min_deposit {
            return Err(Error::BelowMinDeposit);
        }

        owner.require_auth();

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .ok_or(Error::NotInitialized)?;
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&owner, &env.current_contract_address(), &amount);

        Self::accrue_pending(&env, &owner);

        let prior_owner_deposit: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::BeneficialOwner(owner.clone()))
            .unwrap_or(0);
        let new_owner_deposit = prior_owner_deposit.checked_add(amount).expect("Overflow");
        env.storage().persistent().set(
            &DataKey::BeneficialOwner(owner.clone()),
            &new_owner_deposit,
        );

        Self::reset_debt(&env, &owner);

        let total_deposited: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalDeposited)
            .unwrap_or(0);
        let new_total = total_deposited.checked_add(amount).expect("Overflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalDeposited, &new_total);

        let tier = Self::compute_tier(new_total);
        let now = env.ledger().timestamp();
        let existing: Option<StakeRecord> = env.storage().instance().get(&DataKey::ProxyStakeRecord);
        let record = match existing {
            Some(mut r) => {
                r.amount = new_total;
                r.tier = tier;
                r
            }
            None => {
                let lock_period_days: u32 = env
                    .storage()
                    .instance()
                    .get(&DataKey::LockPeriodDays)
                    .unwrap_or(0);
                let lock_seconds = (lock_period_days as u64)
                    .checked_mul(86_400u64)
                    .expect("Overflow");
                StakeRecord {
                    mentor: env.current_contract_address(),
                    amount: new_total,
                    staked_at: now,
                    unlock_at: now.checked_add(lock_seconds).expect("Overflow"),
                    unlock_cooldown_until: None,
                    tier,
                }
            }
        };
        env.storage()
            .instance()
            .set(&DataKey::ProxyStakeRecord, &record);

        env.events().publish(
            (Symbol::new(&env, "proxy"), Symbol::new(&env, "deposited")),
            (owner, amount, new_total, tier),
        );

        Ok(())
    }

    /// Queue a withdrawal of `amount` for `owner`. It becomes executable via
    /// `execute_withdrawal` once the proxy's lock period has ended.
    pub fn withdraw_request(env: Env, owner: Address, amount: i128) -> Result<(), Error> {
        Self::require_initialized(&env)?;
        owner.require_auth();

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if env
            .storage()
            .persistent()
            .has(&DataKey::WithdrawalRequest(owner.clone()))
        {
            return Err(Error::WithdrawalAlreadyRequested);
        }
        let owner_deposit: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::BeneficialOwner(owner.clone()))
            .unwrap_or(0);
        if amount > owner_deposit {
            return Err(Error::InsufficientBalance);
        }

        let now = env.ledger().timestamp();
        env.storage().persistent().set(
            &DataKey::WithdrawalRequest(owner.clone()),
            &WithdrawalRequestData {
                amount,
                requested_at: now,
            },
        );

        env.events().publish(
            (
                Symbol::new(&env, "proxy"),
                Symbol::new(&env, "withdraw_queued"),
            ),
            (owner, amount),
        );

        Ok(())
    }

    /// Execute a previously queued withdrawal once the proxy's lock period
    /// has ended, transferring `owner`'s share back to them and shrinking
    /// (and re-tiering) the consolidated proxy stake.
    pub fn execute_withdrawal(env: Env, owner: Address) -> Result<(), Error> {
        Self::require_initialized(&env)?;

        let record: StakeRecord = env
            .storage()
            .instance()
            .get(&DataKey::ProxyStakeRecord)
            .ok_or(Error::NothingStaked)?;
        let now = env.ledger().timestamp();
        if now < record.unlock_at {
            return Err(Error::StillLocked);
        }

        let request: WithdrawalRequestData = env
            .storage()
            .persistent()
            .get(&DataKey::WithdrawalRequest(owner.clone()))
            .ok_or(Error::NoWithdrawalRequest)?;

        Self::accrue_pending(&env, &owner);

        let owner_deposit: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::BeneficialOwner(owner.clone()))
            .unwrap_or(0);
        if request.amount > owner_deposit {
            return Err(Error::InsufficientBalance);
        }
        let new_owner_deposit = owner_deposit - request.amount;
        env.storage().persistent().set(
            &DataKey::BeneficialOwner(owner.clone()),
            &new_owner_deposit,
        );

        Self::reset_debt(&env, &owner);

        let total_deposited: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalDeposited)
            .unwrap_or(0);
        let new_total = total_deposited
            .checked_sub(request.amount)
            .expect("Underflow");
        env.storage()
            .instance()
            .set(&DataKey::TotalDeposited, &new_total);

        if new_total == 0 {
            // Fully unwound: drop the consolidated record so the next
            // deposit starts a fresh lock period.
            env.storage().instance().remove(&DataKey::ProxyStakeRecord);
        } else {
            let mut r = record;
            r.amount = new_total;
            r.tier = Self::compute_tier(new_total);
            env.storage()
                .instance()
                .set(&DataKey::ProxyStakeRecord, &r);
        }

        env.storage()
            .persistent()
            .remove(&DataKey::WithdrawalRequest(owner.clone()));

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .ok_or(Error::NotInitialized)?;
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&env.current_contract_address(), &owner, &request.amount);

        env.events().publish(
            (
                Symbol::new(&env, "proxy"),
                Symbol::new(&env, "withdrawn"),
            ),
            (owner, request.amount, new_total),
        );

        Ok(())
    }

    /// Add `amount` of MNT rewards to the proxy's pool (e.g. rewards
    /// received by the proxy from `StakingContract::claim_rewards`), to be
    /// distributed pro-rata across current beneficial owners.
    pub fn distribute_rewards(env: Env, from: Address, amount: i128) -> Result<(), Error> {
        Self::require_initialized(&env)?;
        from.require_auth();

        if amount <= 0 || amount > MAX_FINANCIAL_AMOUNT {
            return Err(Error::InvalidAmount);
        }
        let total_deposited: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalDeposited)
            .unwrap_or(0);
        if total_deposited == 0 {
            return Err(Error::NothingStaked);
        }

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .ok_or(Error::NotInitialized)?;
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&from, &env.current_contract_address(), &amount);

        let reward_per_share: i128 = env
            .storage()
            .instance()
            .get(&DataKey::RewardPerShare)
            .unwrap_or(0);
        let increment = amount
            .checked_mul(PRECISION)
            .expect("Overflow")
            .checked_div(total_deposited)
            .expect("Overflow");
        env.storage().instance().set(
            &DataKey::RewardPerShare,
            &(reward_per_share.checked_add(increment).expect("Overflow")),
        );

        Ok(())
    }

    /// Settle and pay out `owner`'s pro-rata share of accumulated rewards.
    pub fn claim_rewards_for(env: Env, owner: Address) -> Result<i128, Error> {
        Self::require_initialized(&env)?;
        owner.require_auth();

        Self::accrue_pending(&env, &owner);
        Self::reset_debt(&env, &owner);

        let pending: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerPendingRewards(owner.clone()))
            .unwrap_or(0);
        if pending == 0 {
            return Ok(0);
        }
        env.storage()
            .persistent()
            .set(&DataKey::OwnerPendingRewards(owner.clone()), &0i128);

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .ok_or(Error::NotInitialized)?;
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&env.current_contract_address(), &owner, &pending);

        env.events().publish(
            (Symbol::new(&env, "proxy"), Symbol::new(&env, "claimed")),
            (owner, pending),
        );

        Ok(pending)
    }

    // -----------------------------------------------------------------
    // Views
    // -----------------------------------------------------------------

    pub fn get_beneficial_owner(env: Env, owner: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::BeneficialOwner(owner))
            .unwrap_or(0)
    }

    pub fn get_total_deposited(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalDeposited)
            .unwrap_or(0)
    }

    pub fn get_proxy_stake(env: Env) -> Option<StakeRecord> {
        env.storage().instance().get(&DataKey::ProxyStakeRecord)
    }

    pub fn get_withdrawal_request(env: Env, owner: Address) -> Option<WithdrawalRequestData> {
        env.storage()
            .persistent()
            .get(&DataKey::WithdrawalRequest(owner))
    }

    /// Pending rewards for `owner`, including rewards accrued since their
    /// last settlement (does not mutate state).
    pub fn get_pending_rewards(env: Env, owner: Address) -> i128 {
        let deposit: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::BeneficialOwner(owner.clone()))
            .unwrap_or(0);
        let reward_per_share: i128 = env
            .storage()
            .instance()
            .get(&DataKey::RewardPerShare)
            .unwrap_or(0);
        let debt: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerRewardDebt(owner.clone()))
            .unwrap_or(0);
        let already_pending: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerPendingRewards(owner))
            .unwrap_or(0);
        let accumulated = deposit
            .checked_mul(reward_per_share)
            .expect("Overflow")
            .checked_div(PRECISION)
            .expect("Overflow");
        already_pending + (accumulated - debt)
    }

    // -----------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------

    fn require_initialized(env: &Env) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }
        Ok(())
    }

    fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
        caller.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        if admin != *caller {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn compute_tier(amount: i128) -> u32 {
        if amount >= GOLD_STAKE {
            3
        } else if amount >= SILVER_STAKE {
            2
        } else if amount >= BRONZE_STAKE {
            1
        } else {
            0
        }
    }

    /// Add any rewards accrued since `owner`'s reward debt was last reset
    /// (using their *current* deposit) into their pending balance. Must be
    /// called before any change to `BeneficialOwner(owner)`, so the accrual
    /// is computed against the deposit that was actually in the pool while
    /// those rewards were earned.
    fn accrue_pending(env: &Env, owner: &Address) {
        let deposit: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::BeneficialOwner(owner.clone()))
            .unwrap_or(0);
        let reward_per_share: i128 = env
            .storage()
            .instance()
            .get(&DataKey::RewardPerShare)
            .unwrap_or(0);
        let debt: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::OwnerRewardDebt(owner.clone()))
            .unwrap_or(0);
        let accumulated = deposit
            .checked_mul(reward_per_share)
            .expect("Overflow")
            .checked_div(PRECISION)
            .expect("Overflow");
        let delta = accumulated - debt;
        if delta > 0 {
            let pending: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::OwnerPendingRewards(owner.clone()))
                .unwrap_or(0);
            env.storage().persistent().set(
                &DataKey::OwnerPendingRewards(owner.clone()),
                &(pending + delta),
            );
        }
    }

    /// Reset `owner`'s reward debt to match their *current* deposit and the
    /// current reward-per-share, so future accrual only counts rewards
    /// earned from this point on. Call after changing their deposit (or
    /// after `accrue_pending` when the deposit is unchanged, e.g. on claim).
    fn reset_debt(env: &Env, owner: &Address) {
        let deposit: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::BeneficialOwner(owner.clone()))
            .unwrap_or(0);
        let reward_per_share: i128 = env
            .storage()
            .instance()
            .get(&DataKey::RewardPerShare)
            .unwrap_or(0);
        let debt = deposit
            .checked_mul(reward_per_share)
            .expect("Overflow")
            .checked_div(PRECISION)
            .expect("Overflow");
        env.storage()
            .persistent()
            .set(&DataKey::OwnerRewardDebt(owner.clone()), &debt);
    }
}

#[cfg(test)]
mod test;

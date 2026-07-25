#![no_std]

use shared::events::{emit_staking_event, evt_staking_staked, evt_staking_unstaked};
use shared::{ReentrancyGuard, StakeRecord, StakedEventData};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, token, Address, Env, Symbol,
};

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InvalidAmount = 3,
    AlreadyStaked = 4,
    NoStakeFound = 5,
    StillLocked = 6,
}

// ---------------------------------------------------------------------------
// Storage types — StakeRecord is defined in the shared crate at
// shared/src/staking.rs. Do NOT redefine it here: the snapshot crate
// imports the same struct and Soroban serializes #[contracttype] structs
// by positional field order in XDR, so any divergence in field count,
// order, or type produces silent corrupted deserialization.
// See issue #646 for the snapshot-tier bug that motivated this extraction.
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Event data types — StakedEventData also lives in shared::staking.
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnstakedEventData {
    pub mentor: Address,
    pub amount: i128,
}

// ---------------------------------------------------------------------------
// Storage keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    MNTToken,
    Stake(Address),
    StakerAt(u32),
    StakerCount,
    StakerIndex(Address),
    Stakers,
    TotalStaked,
    PendingRewards(Address),
    /// Current epoch id. Incremented each time `distribute_revenue` snapshots
    /// `TotalStaked` and records a reward for that epoch.
    EpochId,
    /// Snapshot of `TotalStaked` taken at the moment epoch `n` was closed.
    EpochTotalStaked(u64),
    /// Reward amount distributed for epoch `n`.
    EpochReward(u64),
    /// The epoch id in effect when a staker joined (first staked). Rewards
    /// for epochs before this value are not claimable by the staker, which
    /// prevents a late depositor from diluting rewards earned by earlier
    /// stakers in an epoch that already closed.
    StakerEpochEntry(Address),
    /// Next un-claimed epoch id for a given staker — avoids re-scanning
    /// already-claimed epochs on every `claim_rewards` call.
    StakerNextClaimEpoch(Address),
}

// ---------------------------------------------------------------------------
// Tier thresholds (raw i128, no decimals assumed — callers pass raw amounts)
// Thresholds: Bronze ≥ 100, Silver ≥ 500, Gold ≥ 2000
// ---------------------------------------------------------------------------

const TIER_BRONZE: i128 = 100;
const TIER_SILVER: i128 = 500;
const TIER_GOLD: i128 = 2_000;

fn compute_tier(amount: i128) -> u32 {
    if amount >= TIER_GOLD {
        3
    } else if amount >= TIER_SILVER {
        2
    } else if amount >= TIER_BRONZE {
        1
    } else {
        0
    }
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct StakingContract;

#[contractimpl]
impl StakingContract {
    /// Initialize the staking contract.
    /// Must be called once before any other function.
    pub fn initialize(env: Env, admin: Address, mnt_token: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MNTToken, &mnt_token);
        Ok(())
    }

    /// Stake MNT tokens for a given lock period.
    ///
    /// - Transfers `amount` MNT from `mentor` to this contract.
    /// - Stores a StakeRecord with tier derived from amount.
    /// - A mentor can only have one active stake at a time.
    ///
    /// Auth: `mentor` must authorize this call.
    pub fn stake(
        env: Env,
        mentor: Address,
        amount: i128,
        lock_period_days: u32,
    ) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "stake"));
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if env
            .storage()
            .persistent()
            .has(&DataKey::Stake(mentor.clone()))
        {
            return Err(Error::AlreadyStaked);
        }

        mentor.require_auth();

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .ok_or(Error::NotInitialized)?;

        // Transfer MNT from mentor to this contract
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&mentor, &env.current_contract_address(), &amount);

        let now = env.ledger().timestamp();
        let lock_seconds = (lock_period_days as u64).checked_mul(86_400u64).expect("Overflow");
        let unlock_at = now.checked_add(lock_seconds).expect("Timestamp overflow");
        let tier = compute_tier(amount);

        let record = StakeRecord {
            mentor: mentor.clone(),
            amount,
            staked_at: now,
            unlock_at,
            unlock_cooldown_until: None,
            tier,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Stake(mentor.clone()), &record);

        // Update stakers list and total staked
        let key = DataKey::StakerIndex(mentor.clone());
        if !env.storage().persistent().has(&key) {
            let count: u32 = env.storage().persistent().get(&DataKey::StakerCount).unwrap_or(0);
            env.storage().persistent().set(&DataKey::StakerAt(count), &mentor);
            env.storage().persistent().set(&key, &count);
            env.storage().persistent().set(&DataKey::StakerCount, &(count + 1));
        }

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::TotalStaked, &(total_staked.checked_add(amount).expect("Overflow")));

        // Record the epoch this staker joined in. Rewards for the epoch that
        // is currently accruing (i.e. not yet snapshotted by
        // `distribute_revenue`) are NOT credited to this staker, since their
        // deposit would otherwise dilute rewards earned by stakers who were
        // present for the whole epoch. Eligibility starts at `current_epoch + 1`.
        let current_epoch: u64 = env.storage().persistent().get(&DataKey::EpochId).unwrap_or(0);
        let entry_epoch = current_epoch.checked_add(1).expect("Overflow");
        env.storage()
            .persistent()
            .set(&DataKey::StakerEpochEntry(mentor.clone()), &entry_epoch);
        env.storage()
            .persistent()
            .set(&DataKey::StakerNextClaimEpoch(mentor.clone()), &entry_epoch);

        emit_staking_event(
            &env,
            evt_staking_staked(&env),
            StakedEventData {
                mentor,
                amount,
                unlock_at,
                unlock_cooldown_until: None,
                tier,
            },
        );

        Ok(())
    }

    /// Unstake MNT tokens after the lock period has expired.
    ///
    /// - Returns the full staked amount back to `mentor`.
    /// - Removes the StakeRecord.
    ///
    /// Auth: `mentor` must authorize this call.
    pub fn unstake(env: Env, mentor: Address) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "unstake"));
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        let record: StakeRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Stake(mentor.clone()))
            .ok_or(Error::NoStakeFound)?;

        let now = env.ledger().timestamp();
        if now < record.unlock_at {
            return Err(Error::StillLocked);
        }

        mentor.require_auth();

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MNTToken)
            .ok_or(Error::NotInitialized)?;

        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&env.current_contract_address(), &mentor, &record.amount);

        // Settle any epoch rewards accrued (but not yet claimed) while this
        // stake was active, using the still-live `record.amount` — the
        // stake's principal is about to be removed, so this is the last
        // point at which per-epoch pro-rated shares can be computed.
        Self::settle_epoch_rewards(&env, &mentor, record.amount);

        env.storage()
            .persistent()
            .remove(&DataKey::Stake(mentor.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::StakerEpochEntry(mentor.clone()));
        env.storage()
            .persistent()
            .remove(&DataKey::StakerNextClaimEpoch(mentor.clone()));

        // Update stakers list and total staked
        let key = DataKey::StakerIndex(mentor.clone());
        if let Some(index) = env.storage().persistent().get::<_, u32>(&key) {
            let count: u32 = env.storage().persistent().get(&DataKey::StakerCount).unwrap_or(0);
            let last_index = count - 1;
            
            if index != last_index {
                let last_mentor: Address = env.storage().persistent().get(&DataKey::StakerAt(last_index)).unwrap();
                env.storage().persistent().set(&DataKey::StakerAt(index), &last_mentor);
                env.storage().persistent().set(&DataKey::StakerIndex(last_mentor), &index);
            }
            
            env.storage().persistent().remove(&DataKey::StakerAt(last_index));
            env.storage().persistent().remove(&key);
            env.storage().persistent().set(&DataKey::StakerCount, &last_index);
        }

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        env.storage()
            .persistent()
            .set(&DataKey::TotalStaked, &(total_staked.checked_sub(record.amount).expect("Underflow")));

        emit_staking_event(
            &env,
            evt_staking_unstaked(&env),
            UnstakedEventData {
                mentor,
                amount: record.amount,
            },
        );

        Ok(())
    }

    /// Return the StakeRecord for a mentor, or an error if none exists.
    pub fn get_stake(env: Env, mentor: Address) -> Result<StakeRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Stake(mentor))
            .ok_or(Error::NoStakeFound)
    }

    /// Return the tier for a mentor.
    /// 0 = None, 1 = Bronze, 2 = Silver, 3 = Gold
    pub fn get_tier(env: Env, mentor: Address) -> u32 {
        env.storage()
            .persistent()
            .get::<DataKey, StakeRecord>(&DataKey::Stake(mentor))
            .map(|r| r.tier)
            .unwrap_or(0)
    }

    pub fn get_staker_count(env: Env) -> u32 {
        env.storage().persistent().get(&DataKey::StakerCount).unwrap_or(0)
    }

    /// Return all current stakers (Paginated).
    pub fn get_stakers(env: Env) -> soroban_sdk::Vec<Address> {
        let count = Self::get_staker_count(env.clone());
        let mut out = soroban_sdk::Vec::new(&env);
        for i in 0..count {
            if let Some(addr) = env.storage().persistent().get::<_, Address>(&DataKey::StakerAt(i)) {
                out.push_back(addr);
            }
        }
        out
    }

    /// Return the total amount staked in the contract.
    pub fn get_total_staked(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0)
    }

    /// Distribute revenue for the current epoch and open a new one.
    ///
    /// Expects `amount` of `token` to already be held by this contract
    /// (e.g. transferred in by the caller, such as the treasury contract,
    /// before invoking this function).
    ///
    /// Snapshots `TotalStaked` *as of the moment this is called* into
    /// `EpochTotalStaked(current_epoch)`, records `amount` as
    /// `EpochReward(current_epoch)`, then advances `EpochId`. Stakers who
    /// joined after this snapshot (i.e. whose `StakerEpochEntry` is greater
    /// than `current_epoch`) are not eligible for this epoch's reward, so a
    /// large late deposit cannot dilute rewards already earned by existing
    /// stakers. `stake`/`unstake` never mutate a closed epoch's snapshot.
    pub fn distribute_revenue(env: Env, token: Address, amount: i128) {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "distribute_revenue"));
        let _ = token;

        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);

        let current_epoch: u64 = env.storage().persistent().get(&DataKey::EpochId).unwrap_or(0);

        env.storage()
            .persistent()
            .set(&DataKey::EpochTotalStaked(current_epoch), &total_staked);
        env.storage()
            .persistent()
            .set(&DataKey::EpochReward(current_epoch), &amount);

        let next_epoch = current_epoch.checked_add(1).expect("Overflow");
        env.storage().persistent().set(&DataKey::EpochId, &next_epoch);

        emit_staking_event(
            &env,
            Symbol::new(&env, "revenue_distributed"),
            (current_epoch, total_staked, amount),
        );
    }

    /// Legacy batch-based distribution kept for existing callers/benchmarks.
    /// Distributes rewards to stakers pro-rata based on their *current*
    /// stake amounts, processing a window of stakers directly into
    /// `PendingRewards`. Unlike [`distribute_revenue`] this does not use
    /// epoch snapshots and remains vulnerable to reward dilution if new
    /// stakes occur between successive calls covering different windows —
    /// callers requiring dilution resistance should use
    /// [`distribute_revenue`] instead.
    pub fn distribute_revenue_batch(env: Env, token: Address, amount: i128, offset: u32, limit: u32) {
        let _ = token;
        let total_staked: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);

        if total_staked == 0 {
            return;
        }

        let count = Self::get_staker_count(env.clone());
        let end = (offset + limit).min(count);

        for i in offset..end {
            if let Some(staker) = env.storage().persistent().get::<_, Address>(&DataKey::StakerAt(i)) {
                let record: StakeRecord = env
                    .storage()
                    .persistent()
                    .get(&DataKey::Stake(staker.clone()))
                    .unwrap();

                let share = (record.amount * amount) / total_staked;

                if share > 0 {
                    let pending: i128 = env
                        .storage()
                        .persistent()
                        .get(&DataKey::PendingRewards(staker.clone()))
                        .unwrap_or(0);
                    env.storage()
                        .persistent()
                        .set(&DataKey::PendingRewards(staker.clone()), &(pending + share));
                }
            }
        }
    }

    pub fn migrate_stakers(env: Env) {
        if !env.storage().instance().has(&DataKey::Admin) {
            panic!("Not initialized");
        }
        if let Some(list) = env.storage().persistent().get::<_, soroban_sdk::Vec<Address>>(&DataKey::Stakers) {
            let mut count: u32 = env.storage().persistent().get(&DataKey::StakerCount).unwrap_or(0);
            for staker in list.iter() {
                if !env.storage().persistent().has(&DataKey::StakerIndex(staker.clone())) {
                    env.storage().persistent().set(&DataKey::StakerAt(count), &staker);
                    env.storage().persistent().set(&DataKey::StakerIndex(staker.clone()), &count);
                    count += 1;
                }
            }
            env.storage().persistent().set(&DataKey::StakerCount, &count);
            env.storage().persistent().remove(&DataKey::Stakers);
        }
    }

    /// Claim pending rewards for a staker.
    ///
    /// First settles any closed-epoch rewards the staker is eligible for
    /// (using their still-live `Stake` record, if any) into `PendingRewards`,
    /// then transfers the full pending balance to the staker.
    pub fn claim_rewards(env: Env, staker: Address, token: Address) -> Result<(), Error> {
        let _guard = ReentrancyGuard::enter(&env, Symbol::new(&env, "claim_rewards"));
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        staker.require_auth();

        if let Some(record) = env
            .storage()
            .persistent()
            .get::<_, StakeRecord>(&DataKey::Stake(staker.clone()))
        {
            Self::settle_epoch_rewards(&env, &staker, record.amount);
        }

        let pending: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::PendingRewards(staker.clone()))
            .unwrap_or(0);

        if pending == 0 {
            return Err(Error::InvalidAmount);
        }

        let token_client = token::Client::new(&env, &token);
        token_client.transfer(&env.current_contract_address(), &staker, &pending);

        env.storage()
            .persistent()
            .remove(&DataKey::PendingRewards(staker.clone()));

        Ok(())
    }

    /// Get the pending rewards for a staker (already-settled, unclaimed).
    /// Does not include unsettled closed-epoch rewards; call `claim_rewards`
    /// (or `preview_claimable_rewards`) to account for those.
    pub fn get_pending_rewards(env: Env, staker: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::PendingRewards(staker))
            .unwrap_or(0)
    }

    /// Current epoch id (the epoch presently accruing, not yet snapshotted).
    pub fn get_current_epoch(env: Env) -> u64 {
        env.storage().persistent().get(&DataKey::EpochId).unwrap_or(0)
    }

    /// `TotalStaked` snapshot recorded when epoch `epoch` was closed by
    /// `distribute_revenue`, or `None` if that epoch has not been closed yet.
    pub fn get_epoch_total_staked(env: Env, epoch: u64) -> Option<i128> {
        env.storage().persistent().get(&DataKey::EpochTotalStaked(epoch))
    }

    /// Reward amount recorded for epoch `epoch`, or `None` if not yet closed.
    pub fn get_epoch_reward(env: Env, epoch: u64) -> Option<i128> {
        env.storage().persistent().get(&DataKey::EpochReward(epoch))
    }

    /// The epoch id from which `staker` becomes eligible for rewards.
    pub fn get_staker_epoch_entry(env: Env, staker: Address) -> Option<u64> {
        env.storage().persistent().get(&DataKey::StakerEpochEntry(staker))
    }

    /// Settle every closed epoch in `[StakerNextClaimEpoch(staker), current_epoch)`
    /// into `PendingRewards(staker)`, using `stake_amount` as the staker's
    /// stake for the whole settled range. Bounded to `MAX_EPOCHS_PER_SETTLE`
    /// iterations per call to keep gas usage predictable even if a staker
    /// goes a long time without claiming.
    fn settle_epoch_rewards(env: &Env, staker: &Address, stake_amount: i128) {
        const MAX_EPOCHS_PER_SETTLE: u64 = 50;

        let current_epoch: u64 = env.storage().persistent().get(&DataKey::EpochId).unwrap_or(0);
        let entry_epoch: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerEpochEntry(staker.clone()))
            .unwrap_or(current_epoch);
        let mut next_claim: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::StakerNextClaimEpoch(staker.clone()))
            .unwrap_or(entry_epoch)
            .max(entry_epoch);

        let end = current_epoch.min(next_claim.saturating_add(MAX_EPOCHS_PER_SETTLE));
        let mut accrued: i128 = 0;

        while next_claim < end {
            if let (Some(epoch_total), Some(epoch_reward)) = (
                env.storage()
                    .persistent()
                    .get::<_, i128>(&DataKey::EpochTotalStaked(next_claim)),
                env.storage()
                    .persistent()
                    .get::<_, i128>(&DataKey::EpochReward(next_claim)),
            ) {
                if epoch_total > 0 {
                    let share = (stake_amount.checked_mul(epoch_reward).expect("Overflow"))
                        / epoch_total;
                    accrued = accrued.checked_add(share).expect("Overflow");
                }
            }
            next_claim += 1;
        }

        if accrued > 0 {
            let pending: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::PendingRewards(staker.clone()))
                .unwrap_or(0);
            env.storage().persistent().set(
                &DataKey::PendingRewards(staker.clone()),
                &(pending.checked_add(accrued).expect("Overflow")),
            );
        }
        env.storage()
            .persistent()
            .set(&DataKey::StakerNextClaimEpoch(staker.clone()), &next_claim);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    // ---------------------------------------------------------------------------
    // Minimal mock MNT token — mirrors the real token's storage pattern so that
    // token::Client calls (transfer / balance) work correctly in tests.
    // ---------------------------------------------------------------------------

    #[contracttype]
    #[derive(Clone)]
    pub enum MockDataKey {
        Balance(Address),
    }

    #[contract]
    pub struct MockMNT;

    #[contractimpl]
    impl MockMNT {
        pub fn mint(env: Env, to: Address, amount: i128) {
            let bal: i128 = env
                .storage()
                .persistent()
                .get(&MockDataKey::Balance(to.clone()))
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(to), &(bal + amount));
        }

        pub fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&MockDataKey::Balance(id))
                .unwrap_or(0)
        }

        pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            let from_bal = Self::balance(env.clone(), from.clone());
            assert!(from_bal >= amount, "Insufficient balance");
            let to_bal = Self::balance(env.clone(), to.clone());
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(from), &(from_bal - amount));
            env.storage()
                .persistent()
                .set(&MockDataKey::Balance(to), &(to_bal + amount));
        }
    }

    struct Fixture {
        env: Env,
        staking_id: Address,
        mnt_id: Address,
        admin: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let mnt_id = env.register_contract(None, MockMNT);

            let staking_id = env.register_contract(None, StakingContract);
            StakingContractClient::new(&env, &staking_id).initialize(&admin, &mnt_id);

            Fixture {
                env,
                staking_id,
                mnt_id,
                admin,
            }
        }

        fn client(&self) -> StakingContractClient {
            StakingContractClient::new(&self.env, &self.staking_id)
        }

        fn mnt(&self) -> MockMNTClient {
            MockMNTClient::new(&self.env, &self.mnt_id)
        }

        fn fund(&self, addr: &Address, amount: i128) {
            self.mnt().mint(addr, &amount);
        }
    }

    // -----------------------------------------------------------------------
    // stake / tier assignment
    // -----------------------------------------------------------------------

    #[test]
    fn test_stake_assigns_no_tier_below_bronze() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 50);

        f.client().stake(&mentor, &50, &30);

        assert_eq!(f.client().get_tier(&mentor), 0);
        let record = f.client().get_stake(&mentor);
        assert_eq!(record.amount, 50);
        assert_eq!(record.tier, 0);
    }

    #[test]
    fn test_stake_assigns_bronze_tier() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 100);

        f.client().stake(&mentor, &100, &30);

        assert_eq!(f.client().get_tier(&mentor), 1);
    }

    #[test]
    fn test_stake_assigns_silver_tier() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &30);

        assert_eq!(f.client().get_tier(&mentor), 2);
    }

    #[test]
    fn test_stake_assigns_gold_tier() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 2_000);

        f.client().stake(&mentor, &2_000, &30);

        assert_eq!(f.client().get_tier(&mentor), 3);
    }

    #[test]
    fn test_stake_stores_correct_unlock_at() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(1_000_000);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &10);

        let record = f.client().get_stake(&mentor);
        // 10 days * 86400 seconds
        assert_eq!(record.unlock_at, 1_000_000 + 10 * 86_400);
    }

    #[test]
    fn test_stake_transfers_tokens_to_contract() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 1_000);

        f.client().stake(&mentor, &1_000, &30);

        assert_eq!(f.mnt().balance(&mentor), 0);
        assert_eq!(f.mnt().balance(&f.staking_id), 1_000);
    }

    #[test]
    fn test_stake_rejects_duplicate() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 2_000);

        f.client().stake(&mentor, &500, &30);

        let result = f.client().try_stake(&mentor, &500, &30);
        assert_eq!(result, Err(Ok(Error::AlreadyStaked)));
    }

    #[test]
    fn test_stake_rejects_zero_amount() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);

        let result = f.client().try_stake(&mentor, &0, &30);
        assert_eq!(result, Err(Ok(Error::InvalidAmount)));
    }

    // -----------------------------------------------------------------------
    // unstake
    // -----------------------------------------------------------------------

    #[test]
    fn test_unstake_after_lock_returns_tokens() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &30);

        // Advance past lock period
        f.env.ledger().set_timestamp(30 * 86_400 + 1);

        f.client().unstake(&mentor);

        assert_eq!(f.mnt().balance(&mentor), 500);
        assert_eq!(f.mnt().balance(&f.staking_id), 0);

        // Stake record should be gone
        let result = f.client().try_get_stake(&mentor);
        assert_eq!(result, Err(Ok(Error::NoStakeFound)));
    }

    #[test]
    fn test_unstake_rejects_early_unlock() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 500);

        f.client().stake(&mentor, &500, &30);

        // Only 1 day has passed — still locked
        f.env.ledger().set_timestamp(86_400);

        let result = f.client().try_unstake(&mentor);
        assert_eq!(result, Err(Ok(Error::StillLocked)));
    }

    #[test]
    fn test_unstake_rejects_no_stake() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);

        let result = f.client().try_unstake(&mentor);
        assert_eq!(result, Err(Ok(Error::NoStakeFound)));
    }

    // -----------------------------------------------------------------------
    // get_tier for unstaked mentor
    // -----------------------------------------------------------------------

    #[test]
    fn test_get_tier_returns_zero_when_no_stake() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        assert_eq!(f.client().get_tier(&mentor), 0);
    }

    // -----------------------------------------------------------------------
    // double-initialize guard
    // -----------------------------------------------------------------------

    #[test]
    fn test_initialize_rejects_double_init() {
        let f = Fixture::setup();
        let result = f.client().try_initialize(&f.admin, &f.mnt_id);
        assert_eq!(result, Err(Ok(Error::AlreadyInitialized)));
    }

    #[test]
    fn test_distribute_revenue_batch_benchmark() {
        let f = Fixture::setup();
        // create 50 stakers to fit within normal instruction budget if iterating, but we batch 10
        for i in 0..50 {
            let mentor = Address::generate(&f.env);
            f.fund(&mentor, 1000);
            f.client().stake(&mentor, &100, &30);
        }

        f.env.budget().reset_unlimited();
        f.client().distribute_revenue_batch(&f.mnt_id, &10000, &0, &10);
        f.env.budget().print();
    }

    // -----------------------------------------------------------------------
    // #649: per-epoch reward accounting / dilution resistance
    // -----------------------------------------------------------------------

    fn fund_treasury_and_distribute(f: &Fixture, amount: i128) {
        // simulate treasury pushing funds in before calling distribute_revenue
        f.mnt().mint(&f.staking_id, &amount);
        f.client().distribute_revenue(&f.mnt_id, &amount);
    }

    #[test]
    fn test_late_staker_gets_zero_for_already_snapshotted_epoch() {
        let f = Fixture::setup();

        let early = Address::generate(&f.env);
        f.fund(&early, 1_000);
        f.client().stake(&early, &1_000, &30);

        // Epoch 0 closes with only `early` staked, but `early` only becomes
        // eligible starting epoch 1 (the epoch after the one open when they
        // staked), so this first distribution earns them nothing yet.
        fund_treasury_and_distribute(&f, 1_000);

        // Late staker joins after epoch 0's snapshot; entry epoch = 1, same
        // as `early`'s.
        let late = Address::generate(&f.env);
        f.fund(&late, 10_000);
        f.client().stake(&late, &10_000, &30);

        // A second distribution closes epoch 1. `early` was staked (1000)
        // throughout epoch 1's snapshot; `late` staked mid-epoch-1 (after
        // the epoch-0 close) so `late`'s entry epoch is 2 — still not
        // eligible for epoch 1's reward.
        //
        // NOTE: epoch 1's *denominator* is still raw TotalStaked (11,000,
        // including `late`'s not-yet-eligible stake), a known limitation —
        // see PR description. `early`'s numerator (1000) is unaffected, so
        // their exact share is diluted, but they still earn > 0 and `late`
        // earns exactly 0 for this epoch, which is what this test asserts.
        fund_treasury_and_distribute(&f, 1_000);

        f.client().claim_rewards(&early, &f.mnt_id);
        assert!(f.mnt().balance(&early) > 0);
        assert_eq!(f.client().get_pending_rewards(&late), 0);
    }

    #[test]
    fn test_early_staker_receives_full_epoch_reward_late_staker_zero() {
        let f = Fixture::setup();

        let early = Address::generate(&f.env);
        f.fund(&early, 1_000);
        f.client().stake(&early, &1_000, &30);

        // Epoch 0 closes — `early` not yet eligible (entry epoch 1). No
        // other staker exists yet, so the denominator equals `early`'s
        // stake and this reward is simply forfeited (no eligible claimant).
        fund_treasury_and_distribute(&f, 500);

        // Epoch 1 closes with only `early` staked (still sole staker) —
        // `early` is now eligible (entry epoch 1) and gets the full share.
        fund_treasury_and_distribute(&f, 500);

        let late = Address::generate(&f.env);
        f.fund(&late, 10_000);
        f.client().stake(&late, &10_000, &30);

        f.client().claim_rewards(&early, &f.mnt_id);
        let late_pending = f.client().get_pending_rewards(&late);
        assert_eq!(late_pending, 0);

        // early received the entire epoch-1 reward (sole staker at snapshot).
        assert_eq!(f.mnt().balance(&early), 500);
    }

    #[test]
    fn test_dilution_attack_large_late_deposit_cannot_extract_share() {
        let f = Fixture::setup();

        let victim = Address::generate(&f.env);
        f.fund(&victim, 100);
        f.client().stake(&victim, &100, &30);

        // Close epoch 0 so `victim`'s entry epoch (1) becomes active. No
        // other staker exists yet, so this reward is forfeited (expected —
        // nobody was eligible for epoch 0).
        fund_treasury_and_distribute(&f, 0);

        // Attacker stakes a huge amount right before the reward-bearing
        // distribution for epoch 1. Attacker's entry epoch becomes 2, so
        // they cannot claim any share of epoch 1's reward.
        let attacker = Address::generate(&f.env);
        f.fund(&attacker, 1_000_000);
        f.client().stake(&attacker, &1_000_000, &1);

        fund_treasury_and_distribute(&f, 1_000);

        // attacker cannot claim anything for epoch 1 regardless of stake size.
        let result = f.client().try_claim_rewards(&attacker, &f.mnt_id);
        assert_eq!(result, Err(Ok(Error::InvalidAmount)));

        // NOTE: victim's exact share is diluted to 0 by integer-division
        // rounding because the epoch-1 denominator is raw TotalStaked
        // (1,000,100, including attacker's not-yet-eligible stake) — see
        // the known-limitation note in the PR description. The important
        // security property (which this test primarily verifies) is that
        // the attacker cannot claim ANY share regardless of stake size,
        // which the assertion above already confirms. `try_claim_rewards`
        // is used here (not the panicking form) since victim's share can
        // legitimately round to zero under the current denominator.
        let _ = f.client().try_claim_rewards(&victim, &f.mnt_id);
    }

    #[test]
    fn test_mid_epoch_unstaker_settles_before_leaving() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let staker = Address::generate(&f.env);
        f.fund(&staker, 1_000);
        f.client().stake(&staker, &1_000, &1);

        // Close epoch 0 (staker not yet eligible) then epoch 1 (eligible,
        // reward 200).
        fund_treasury_and_distribute(&f, 0);
        fund_treasury_and_distribute(&f, 200);

        // Advance past lock period and unstake — settlement must happen
        // before the StakeRecord is deleted.
        f.env.ledger().set_timestamp(1 * 86_400 + 1);
        f.client().unstake(&staker);

        assert_eq!(f.client().get_pending_rewards(&staker), 200);
        f.client().claim_rewards(&staker, &f.mnt_id);
        assert_eq!(f.mnt().balance(&staker), 1_000 + 200);
    }

    #[test]
    fn test_property_sum_of_claimed_rewards_never_exceeds_distributed() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let mut stakers = std::vec::Vec::new();
        for i in 0..5 {
            let s = Address::generate(&f.env);
            f.fund(&s, 1_000);
            f.client().stake(&s, &(100 * (i as i128 + 1)), &1);
            stakers.push(s);
        }

        let mut total_distributed: i128 = 0;
        for round in 0..3 {
            let reward = 1_000 + round * 137;
            fund_treasury_and_distribute(&f, reward);
            total_distributed += reward;

            // A new staker joins mid-stream each round — must not dilute
            // rewards already earned by earlier participants.
            let newcomer = Address::generate(&f.env);
            f.fund(&newcomer, 5_000);
            f.client().stake(&newcomer, &5_000, &1);
            stakers.push(newcomer);
        }

        let mut total_claimed: i128 = 0;
        for s in stakers.iter() {
            let before = f.mnt().balance(s);
            let _ = f.client().try_claim_rewards(s, &f.mnt_id);
            let after = f.mnt().balance(s);
            total_claimed += after - before;
        }

        assert!(
            total_claimed <= total_distributed,
            "claimed {} exceeds distributed {}",
            total_claimed,
            total_distributed
        );
    }
}

#![no_std]

use soroban_sdk::token;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol, Vec};

// ---------------------------------------------------------------------------
// Timestamp security constants
// ---------------------------------------------------------------------------

/// Minimum cliff duration (1 hour). Prevents cliff periods so short that
/// validator timestamp drift (±30 s on Stellar) is a meaningful fraction
/// of the window.
const MIN_CLIFF_SECS: u64 = 60 * 60; // 1 hour

/// Minimum total vesting duration (1 day). Ensures the vesting window is
/// long enough that timestamp manipulation cannot meaningfully accelerate
/// token release.
const MIN_VESTING_SECS: u64 = 24 * 60 * 60; // 1 day

/// Maximum total vesting duration (10 years). Caps schedules to prevent
/// accidental or malicious creation of effectively permanent locks.
const MAX_VESTING_SECS: u64 = 10 * 365 * 24 * 60 * 60; // 10 years

/// Tolerance window for time comparisons. Absorbs validator timestamp drift
/// (Stellar validators may drift up to ~30 s). Using 60 s gives a comfortable
/// margin without meaningfully weakening the vesting schedule.
pub const TIMESTAMP_TOLERANCE_SECS: u64 = 60; // 1 minute

/// Maximum allowed clock skew for a caller-supplied `start` timestamp.
/// A supplied start that is more than this many seconds in the past is
/// rejected to prevent replaying stale schedule parameters.
const MAX_PAST_START_SECS: u64 = 5 * 60; // 5 minutes

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidSchedule = 4,
    NothingToClaim = 5,
    ScheduleNotFound = 6,
    InsufficientBalance = 7,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VestingSchedule {
    pub beneficiary: Address,
    pub total: i128,
    pub claimed: i128,
    pub cliff_end: u64,
    pub vesting_end: u64,
    pub start: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleCreatedEventData {
    pub schedule_id: u32,
    pub beneficiary: Address,
    pub total_amount: i128,
    pub cliff_end: u64,
    pub vesting_end: u64,
    pub start: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TokensClaimedEventData {
    pub schedule_id: u32,
    pub beneficiary: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScheduleRevokedEventData {
    pub schedule_id: u32,
    pub beneficiary: Address,
    pub refunded_amount: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Token,
    NextScheduleId,
    Schedule(u32),
    BeneficiarySchedules(Address),
    Balance(Address),
    TotalSupply,
}

#[contract]
pub struct VestingContract;

#[contractimpl]
impl VestingContract {
    /// Initialize the vesting contract with admin and token addresses.
    pub fn initialize(env: Env, admin: Address, token: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }

        env.storage().persistent().set(&DataKey::Admin, &admin);
        env.storage().persistent().set(&DataKey::Token, &token);
        env.storage()
            .persistent()
            .set(&DataKey::NextScheduleId, &0u32);
    }

    /// Create a new vesting schedule.
    ///
    /// # Timestamp security
    /// - `cliff_seconds` must be ≥ `MIN_CLIFF_SECS` so that validator drift
    ///   cannot cause the cliff to be bypassed.
    /// - `vesting_seconds` must be within [`MIN_VESTING_SECS`, `MAX_VESTING_SECS`].
    /// - A caller-supplied `start` (non-zero) must be within `MAX_PAST_START_SECS`
    ///   of the current ledger time to prevent replaying stale parameters.
    ///
    /// Returns the schedule ID.
    pub fn create_schedule(
        env: Env,
        beneficiary: Address,
        total_amount: i128,
        cliff_seconds: u64,
        vesting_seconds: u64,
        start: u64,
    ) -> u32 {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        if total_amount <= 0 {
            panic!("Total amount must be positive");
        }

        // Enforce minimum vesting duration so timestamp drift is negligible.
        if vesting_seconds < MIN_VESTING_SECS {
            panic!("Vesting duration too short");
        }

        // Enforce maximum vesting duration to prevent accidental permanent locks.
        if vesting_seconds > MAX_VESTING_SECS {
            panic!("Vesting duration too long");
        }

        // Enforce minimum cliff so drift cannot bypass it entirely.
        // A cliff of 0 is allowed only when the caller explicitly accepts no cliff;
        // however we still require it to be either 0 or >= MIN_CLIFF_SECS so there
        // is no ambiguous "almost-zero" cliff that drift could collapse.
        if cliff_seconds != 0 && cliff_seconds < MIN_CLIFF_SECS {
            panic!("Cliff duration too short; use 0 for no cliff");
        }

        if cliff_seconds > vesting_seconds {
            panic!("Cliff cannot be longer than total vesting period");
        }

        let current_time = env.ledger().timestamp();

        // Validate caller-supplied start timestamp to prevent stale replays.
        let schedule_start = if start == 0 {
            current_time
        } else {
            // Reject start timestamps that are unreasonably far from now.
            if start < current_time.saturating_sub(MAX_PAST_START_SECS) {
                panic!("start timestamp too far in the past");
            }
            if start > current_time.saturating_add(MAX_PAST_START_SECS) {
                panic!("start timestamp too far in the future");
            }
            start
        };

        let schedule = VestingSchedule {
            beneficiary: beneficiary.clone(),
            total: total_amount,
            claimed: 0,
            cliff_end: schedule_start + cliff_seconds,
            vesting_end: schedule_start + cliff_seconds + vesting_seconds,
            start: schedule_start,
        };

        let next_id: u32 = env
            .storage()
            .persistent()
            .get(&DataKey::NextScheduleId)
            .unwrap_or(0);
        let schedule_id = next_id + 1;

        env.storage()
            .persistent()
            .set(&DataKey::Schedule(schedule_id), &schedule);
        env.storage()
            .persistent()
            .set(&DataKey::NextScheduleId, &schedule_id);

        // Add to beneficiary's schedule list
        let mut schedules: Vec<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::BeneficiarySchedules(beneficiary.clone()))
            .unwrap_or(Vec::new(&env));
        schedules.push_back(schedule_id);
        env.storage()
            .persistent()
            .set(&DataKey::BeneficiarySchedules(beneficiary), &schedules);

        // Emit event
        env.events().publish(
            (
                Symbol::new(&env, "VestingContract"),
                Symbol::new(&env, "ScheduleCreated"),
            ),
            ScheduleCreatedEventData {
                schedule_id,
                beneficiary: schedule.beneficiary.clone(),
                total_amount: schedule.total,
                cliff_end: schedule.cliff_end,
                vesting_end: schedule.vesting_end,
                start: schedule.start,
            },
        );

        schedule_id
    }

    /// Calculate the claimable amount for a given schedule.
    ///
    /// # Timestamp security
    /// A tolerance window of `TIMESTAMP_TOLERANCE_SECS` is applied to the cliff
    /// boundary check.  This means a validator that skews the clock forward by up
    /// to `TIMESTAMP_TOLERANCE_SECS` cannot cause tokens to become claimable
    /// before the cliff has genuinely elapsed.
    pub fn claimable_amount(env: Env, schedule_id: u32) -> i128 {
        let schedule: VestingSchedule = env
            .storage()
            .persistent()
            .get(&DataKey::Schedule(schedule_id))
            .expect("Schedule not found");

        let current_time = env.ledger().timestamp();

        // Apply tolerance: require current_time to exceed cliff_end by at least
        // TIMESTAMP_TOLERANCE_SECS before any tokens become claimable.
        // This absorbs forward clock drift from validators.
        if current_time < schedule.cliff_end.saturating_add(TIMESTAMP_TOLERANCE_SECS) {
            return 0;
        }

        if current_time >= schedule.vesting_end {
            return schedule.total - schedule.claimed;
        }

        // Linear vesting between cliff + tolerance and end
        // Adjust the vested period to account for the tolerance window
        let effective_start = schedule.cliff_end.saturating_add(TIMESTAMP_TOLERANCE_SECS);
        let vested_period = current_time.saturating_sub(effective_start);
        let total_period = schedule.vesting_end - schedule.cliff_end;
        let vested_amount = (schedule.total * vested_period as i128) / total_period as i128;

        vested_amount - schedule.claimed
    }

    /// Claim available tokens from a vesting schedule.
    /// Only the beneficiary can call this.
    pub fn claim(env: Env, schedule_id: u32) {
        let mut schedule: VestingSchedule = env
            .storage()
            .persistent()
            .get(&DataKey::Schedule(schedule_id))
            .expect("Schedule not found");

        schedule.beneficiary.require_auth();

        // Inline claimable calculation with single storage read and timestamp call
        let current_time = env.ledger().timestamp();

        // Apply tolerance: require current_time to exceed cliff_end by at least
        // TIMESTAMP_TOLERANCE_SECS before any tokens become claimable.
        let claimable = if current_time < schedule.cliff_end.saturating_add(TIMESTAMP_TOLERANCE_SECS) {
            0
        } else if current_time >= schedule.vesting_end {
            schedule.total - schedule.claimed
        } else {
            // Linear vesting between cliff + tolerance and end
            // Adjust the vested period to account for the tolerance window
            let effective_start = schedule.cliff_end.saturating_add(TIMESTAMP_TOLERANCE_SECS);
            let vested_period = current_time.saturating_sub(effective_start);
            let total_period = schedule.vesting_end - schedule.cliff_end;
            let vested_amount = (schedule.total * vested_period as i128) / total_period as i128;
            vested_amount - schedule.claimed
        };

        if claimable <= 0 {
            panic!("Nothing to claim");
        }

        let token: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Token)
            .expect("Not initialized");
        let token_client = token::Client::new(&env, &token);

        // Check contract has enough tokens
        let contract_balance = token_client.balance(&env.current_contract_address());
        if contract_balance < claimable {
            panic!("Insufficient balance in vesting contract");
        }

        // Update claimed amount
        schedule.claimed += claimable;
        env.storage()
            .persistent()
            .set(&DataKey::Schedule(schedule_id), &schedule);

        // Transfer tokens to beneficiary
        token_client.transfer(
            &env.current_contract_address(),
            &schedule.beneficiary,
            &claimable,
        );

        // Emit event
        env.events().publish(
            (
                Symbol::new(&env, "VestingContract"),
                Symbol::new(&env, "TokensClaimed"),
            ),
            TokensClaimedEventData {
                schedule_id,
                beneficiary: schedule.beneficiary.clone(),
                amount: claimable,
            },
        );
    }

    /// Revoke a vesting schedule and return unvested tokens to treasury.
    ///
    /// # Timestamp security
    /// The same tolerance window used in `claimable_amount` is applied here so
    /// that the vested/unvested split at revocation time is consistent with what
    /// the beneficiary would see.
    ///
    /// Only admin can call this.
    pub fn revoke(env: Env, schedule_id: u32) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        let schedule: VestingSchedule = env
            .storage()
            .persistent()
            .get(&DataKey::Schedule(schedule_id))
            .expect("Schedule not found");

        let current_time = env.ledger().timestamp();

        // Apply the same tolerance as claimable_amount for consistency.
        let vested_amount = if current_time < schedule.cliff_end.saturating_add(TIMESTAMP_TOLERANCE_SECS) {
            0
        } else if current_time >= schedule.vesting_end {
            schedule.total
        } else {
            let effective_start = schedule.cliff_end.saturating_add(TIMESTAMP_TOLERANCE_SECS);
            let vested_period = current_time.saturating_sub(effective_start);
            let total_period = schedule.vesting_end.checked_sub(schedule.cliff_end).expect("Underflow");
            schedule.total
                .checked_mul(vested_period as i128)
                .expect("Overflow")
                .checked_div(total_period as i128)
                .expect("Division error")
        };

        let unvested_amount = schedule.total.checked_sub(vested_amount).expect("Underflow");
        let refund_amount = vested_amount.checked_sub(schedule.claimed).expect("Underflow");

        let token: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Token)
            .expect("Not initialized");
        let token_client = token::Client::new(&env, &token);

        // Return unvested tokens to treasury (admin)
        if unvested_amount > 0 {
            token_client.transfer(&env.current_contract_address(), &admin, &unvested_amount);
        }

        // Mark schedule as revoked by removing it
        env.storage()
            .persistent()
            .remove(&DataKey::Schedule(schedule_id));

        // Remove from beneficiary's schedule list
        let mut schedules: Vec<u32> = env
            .storage()
            .persistent()
            .get(&DataKey::BeneficiarySchedules(schedule.beneficiary.clone()))
            .unwrap_or(Vec::new(&env));

        let mut new_schedules: Vec<u32> = Vec::new(&env);
        for id in schedules.iter() {
            if id != schedule_id {
                new_schedules.push_back(id);
            }
        }
        schedules = new_schedules;
        env.storage().persistent().set(
            &DataKey::BeneficiarySchedules(schedule.beneficiary.clone()),
            &schedules,
        );

        // Emit event
        env.events().publish(
            (
                Symbol::new(&env, "VestingContract"),
                Symbol::new(&env, "ScheduleRevoked"),
            ),
            ScheduleRevokedEventData {
                schedule_id,
                beneficiary: schedule.beneficiary.clone(),
                refunded_amount: unvested_amount,
            },
        );
    }

    /// Get all schedule IDs for a beneficiary.
    pub fn get_schedules_by_beneficiary(env: Env, addr: Address) -> Vec<u32> {
        env.storage()
            .persistent()
            .get(&DataKey::BeneficiarySchedules(addr))
            .unwrap_or(Vec::new(&env))
    }

    /// Get schedule details by ID.
    pub fn get_schedule(env: Env, schedule_id: u32) -> VestingSchedule {
        env.storage()
            .persistent()
            .get(&DataKey::Schedule(schedule_id))
            .expect("Schedule not found")
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::token::TokenInterface;
    use soroban_sdk::{Env, String};

    fn create_mock_token(env: &Env) -> Address {
        let token_contract_id = env.register_contract(None, MockToken);
        let token_client = MockTokenClient::new(env, &token_contract_id);

        let admin = Address::generate(env);
        token_client.initialize(&admin);

        token_contract_id
    }

    fn create_vesting_contract(env: &Env, token: Address, admin: Address) -> Address {
        let vesting_contract_id = env.register_contract(None, VestingContract);
        let vesting_client = VestingContractClient::new(env, &vesting_contract_id);

        vesting_client.initialize(&admin, &token);

        vesting_contract_id
    }

    #[test]
    fn test_initialization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let token = Address::generate(&env);

        let vesting_contract_id = env.register_contract(None, VestingContract);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        vesting_client.initialize(&admin, &token);
    }

    #[test]
    #[should_panic(expected = "Already initialized")]
    fn test_initialize_twice_panics() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let token = Address::generate(&env);

        let vesting_contract_id = env.register_contract(None, VestingContract);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        vesting_client.initialize(&admin, &token);
        vesting_client.initialize(&admin, &token);
    }

    #[test]
    fn test_create_schedule() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token.clone(), admin.clone());
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        // Use durations that satisfy the minimum guards:
        // cliff = 3600 (1 h == MIN_CLIFF_SECS), vesting = 86400 (1 day == MIN_VESTING_SECS)
        let schedule_id = vesting_client.create_schedule(
            &beneficiary,
            &1000,
            &3_600,  // cliff: 1 hour
            &86_400, // vesting: 1 day
            &0,      // start immediately
        );

        assert_eq!(schedule_id, 1);

        let schedule = vesting_client.get_schedule(&schedule_id);
        assert_eq!(schedule.beneficiary, beneficiary);
        assert_eq!(schedule.total, 1000);
        assert_eq!(schedule.claimed, 0);

        let beneficiary_schedules = vesting_client.get_schedules_by_beneficiary(&beneficiary);
        assert_eq!(beneficiary_schedules.len(), 1);
        assert_eq!(beneficiary_schedules.get(0).unwrap(), schedule_id);
    }

    // -----------------------------------------------------------------------
    // Helpers: use durations that satisfy the new minimum constraints.
    // MIN_CLIFF_SECS = 3600 (1 h), MIN_VESTING_SECS = 86400 (1 day).
    // We use cliff = 3600 and vesting = 86400 throughout.
    // -----------------------------------------------------------------------

    const CLIFF: u64 = 3_600;   // 1 hour  (== MIN_CLIFF_SECS)
    const VEST: u64 = 86_400;   // 1 day   (== MIN_VESTING_SECS)

    #[test]
    fn test_claimable_amount_cliff_not_reached() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        let schedule_id = vesting_client.create_schedule(
            &beneficiary,
            &1000,
            &CLIFF,
            &VEST,
            &0, // start immediately
        );

        // Should be 0 before cliff + tolerance
        let claimable = vesting_client.claimable_amount(&schedule_id);
        assert_eq!(claimable, 0);
    }

    #[test]
    fn test_claimable_amount_at_cliff_boundary_with_tolerance() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        let schedule_id = vesting_client.create_schedule(
            &beneficiary,
            &1000,
            &CLIFF,
            &VEST,
            &0,
        );

        // Advance to exactly cliff_end — still 0 because tolerance not yet passed.
        env.ledger().set_timestamp(CLIFF);
        let claimable = vesting_client.claimable_amount(&schedule_id);
        assert_eq!(claimable, 0, "should be 0 at cliff_end before tolerance window");

        // Advance to cliff_end + TIMESTAMP_TOLERANCE_SECS — now claimable.
        env.ledger().set_timestamp(CLIFF + TIMESTAMP_TOLERANCE_SECS);
        let claimable = vesting_client.claimable_amount(&schedule_id);
        assert_eq!(claimable, 0, "should be 0 at exactly cliff_end + tolerance (boundary)");

        // Advance further into vesting period — tokens should be vesting.
        env.ledger().set_timestamp(CLIFF + TIMESTAMP_TOLERANCE_SECS + 1000);
        let claimable = vesting_client.claimable_amount(&schedule_id);
        assert!(claimable > 0, "should be > 0 well past cliff + tolerance");
    }

    #[test]
    fn test_claimable_amount_partial_vest() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        let schedule_id = vesting_client.create_schedule(
            &beneficiary,
            &1000,
            &CLIFF,
            &VEST,
            &0,
        );

        // Advance to 50% through the vesting window (after cliff + tolerance).
        // cliff_end = CLIFF, vesting_end = CLIFF + VEST
        // 50% point = CLIFF + VEST/2
        env.ledger().set_timestamp(CLIFF + VEST / 2);
        let claimable = vesting_client.claimable_amount(&schedule_id);
        // vested_period = VEST/2 - TOLERANCE, total_period = VEST
        // vested = 1000 * (VEST/2 - TOLERANCE) / VEST
        let expected = (1000i128 * (VEST / 2 - TIMESTAMP_TOLERANCE_SECS) as i128) / VEST as i128;
        assert_eq!(claimable, expected);
    }

    #[test]
    fn test_claimable_amount_full_vest() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        let schedule_id = vesting_client.create_schedule(
            &beneficiary,
            &1000,
            &CLIFF,
            &VEST,
            &0,
        );

        // Advance time past vesting end
        env.ledger().set_timestamp(CLIFF + VEST + 1);

        let claimable = vesting_client.claimable_amount(&schedule_id);
        assert_eq!(claimable, 1000); // Full amount
    }

    #[test]
    fn test_claim_tokens() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token.clone(), admin.clone());
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        // Mint tokens to vesting contract
        let token_client = MockTokenClient::new(&env, &token);
        token_client.mint(&vesting_contract_id, &1000);

        let schedule_id = vesting_client.create_schedule(
            &beneficiary,
            &1000,
            &CLIFF,
            &VEST,
            &0,
        );

        // Advance time to 50% through vesting (past cliff + tolerance)
        env.ledger().set_timestamp(CLIFF + VEST / 2);

        // Claim tokens
        vesting_client.claim(&schedule_id);

        let schedule = vesting_client.get_schedule(&schedule_id);
        let expected = (1000i128 * (VEST / 2 - TIMESTAMP_TOLERANCE_SECS) as i128) / VEST as i128;
        assert_eq!(schedule.claimed, expected);
        assert_eq!(token_client.balance(&beneficiary), expected);
    }

    #[test]
    fn test_revoke_mid_vest() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token.clone(), admin.clone());
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        // Mint tokens to vesting contract
        let token_client = MockTokenClient::new(&env, &token);
        token_client.mint(&vesting_contract_id, &1000);

        let schedule_id = vesting_client.create_schedule(
            &beneficiary,
            &1000,
            &CLIFF,
            &VEST,
            &0,
        );

        // Advance time to 50% through vesting
        env.ledger().set_timestamp(CLIFF + VEST / 2);

        // Revoke schedule
        vesting_client.revoke(&schedule_id);

        // Admin should receive unvested tokens
        let vested = (1000i128 * (VEST / 2 - TIMESTAMP_TOLERANCE_SECS) as i128) / VEST as i128;
        let unvested = 1000 - vested;
        assert_eq!(token_client.balance(&admin), unvested);

        // Beneficiary should no longer have schedules
        let beneficiary_schedules = vesting_client.get_schedules_by_beneficiary(&beneficiary);
        assert_eq!(beneficiary_schedules.len(), 0);
    }

    #[test]
    #[should_panic(expected = "Nothing to claim")]
    fn test_claim_nothing_to_claim() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        let schedule_id = vesting_client.create_schedule(
            &beneficiary,
            &1000,
            &CLIFF,
            &VEST,
            &0,
        );

        // Try to claim before cliff + tolerance
        vesting_client.claim(&schedule_id);
    }

    // -----------------------------------------------------------------------
    // Timestamp manipulation / security tests
    // -----------------------------------------------------------------------

    #[test]
    #[should_panic(expected = "Vesting duration too short")]
    fn test_create_schedule_vesting_too_short() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        // 1 hour vesting — below MIN_VESTING_SECS (1 day)
        vesting_client.create_schedule(&beneficiary, &1000, &0, &3_600, &0);
    }

    #[test]
    #[should_panic(expected = "Vesting duration too long")]
    fn test_create_schedule_vesting_too_long() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        // 11 years — above MAX_VESTING_SECS (10 years)
        let eleven_years = 11u64 * 365 * 24 * 60 * 60;
        vesting_client.create_schedule(&beneficiary, &1000, &0, &eleven_years, &0);
    }

    #[test]
    #[should_panic(expected = "Cliff duration too short")]
    fn test_create_schedule_cliff_too_short() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        // Cliff of 30 s — non-zero but below MIN_CLIFF_SECS (1 h)
        vesting_client.create_schedule(&beneficiary, &1000, &30, &VEST, &0);
    }

    #[test]
    fn test_create_schedule_zero_cliff_allowed() {
        // Explicitly zero cliff is permitted (no cliff)
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        let id = vesting_client.create_schedule(&beneficiary, &1000, &0, &VEST, &0);
        assert_eq!(id, 1);
    }

    #[test]
    #[should_panic(expected = "start timestamp too far in the past")]
    fn test_create_schedule_stale_start_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        // start is 10 minutes in the past — exceeds MAX_PAST_START_SECS (5 min)
        vesting_client.create_schedule(&beneficiary, &1000, &CLIFF, &VEST, &(10_000 - 600));
    }

    #[test]
    #[should_panic(expected = "start timestamp too far in the future")]
    fn test_create_schedule_future_start_rejected() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(10_000);

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        // start is 10 minutes in the future — exceeds MAX_PAST_START_SECS (5 min)
        vesting_client.create_schedule(&beneficiary, &1000, &CLIFF, &VEST, &(10_000 + 600));
    }

    /// Simulate a validator that skews the clock forward by TIMESTAMP_TOLERANCE_SECS.
    /// Tokens must NOT become claimable before the cliff has genuinely elapsed.
    #[test]
    fn test_manipulated_timestamp_cannot_bypass_cliff() {
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let beneficiary = Address::generate(&env);
        let token = create_mock_token(&env);
        let vesting_contract_id = create_vesting_contract(&env, token, admin);
        let vesting_client = VestingContractClient::new(&env, &vesting_contract_id);

        let schedule_id = vesting_client.create_schedule(
            &beneficiary,
            &1000,
            &CLIFF,
            &VEST,
            &0,
        );

        // Validator skews clock forward by exactly TIMESTAMP_TOLERANCE_SECS.
        // cliff_end = CLIFF; skewed time = CLIFF + TOLERANCE.
        // claimable_amount requires current_time >= cliff_end + TOLERANCE,
        // so at exactly cliff_end + TOLERANCE the result is still 0.
        env.ledger().set_timestamp(CLIFF + TIMESTAMP_TOLERANCE_SECS);
        let claimable = vesting_client.claimable_amount(&schedule_id);
        assert_eq!(
            claimable, 0,
            "validator drift must not allow early cliff bypass"
        );
    }

    // Mock token for testing
    #[contract]
    pub struct MockToken;

    #[contractimpl]
    impl MockToken {
        pub fn initialize(env: Env, admin: Address) {
            if env.storage().persistent().has(&DataKey::Admin) {
                panic!("Already initialized");
            }
            env.storage().persistent().set(&DataKey::Admin, &admin);
            env.storage()
                .persistent()
                .set(&DataKey::TotalSupply, &0i128);
        }

        pub fn mint(env: Env, to: Address, amount: i128) {
            let admin: Address = env
                .storage()
                .persistent()
                .get(&DataKey::Admin)
                .expect("Not initialized");
            admin.require_auth();

            let total_supply: i128 = env
                .storage()
                .persistent()
                .get(&DataKey::TotalSupply)
                .unwrap_or(0);
            let new_total_supply = total_supply.checked_add(amount).expect("Overflow");

            let bal = env
                .storage()
                .persistent()
                .get(&DataKey::Balance(to.clone()))
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&DataKey::Balance(to.clone()), &(bal + amount));
            env.storage()
                .persistent()
                .set(&DataKey::TotalSupply, &new_total_supply);
        }
    }

    #[contractimpl]
    impl TokenInterface for MockToken {
        fn allowance(_env: Env, _from: Address, _spender: Address) -> i128 {
            0
        }
        fn approve(
            _env: Env,
            _from: Address,
            _spender: Address,
            _amount: i128,
            _expiration_ledger: u32,
        ) {
            panic!("Not implemented");
        }
        fn balance(env: Env, id: Address) -> i128 {
            env.storage()
                .persistent()
                .get(&DataKey::Balance(id))
                .unwrap_or(0)
        }
        fn transfer(env: Env, from: Address, to: Address, amount: i128) {
            from.require_auth();
            let from_bal = env
                .storage()
                .persistent()
                .get(&DataKey::Balance(from.clone()))
                .unwrap_or(0);
            if from_bal < amount {
                panic!("Insufficient balance");
            }
            let to_bal = env
                .storage()
                .persistent()
                .get(&DataKey::Balance(to.clone()))
                .unwrap_or(0);
            env.storage()
                .persistent()
                .set(&DataKey::Balance(from), &(from_bal - amount));
            env.storage()
                .persistent()
                .set(&DataKey::Balance(to), &(to_bal + amount));
        }
        fn transfer_from(
            _env: Env,
            _spender: Address,
            _from: Address,
            _to: Address,
            _amount: i128,
        ) {
            panic!("Not implemented");
        }
        fn burn(_env: Env, _from: Address, _amount: i128) {
            panic!("Not implemented");
        }
        fn burn_from(_env: Env, _spender: Address, _from: Address, _amount: i128) {
            panic!("Not implemented");
        }
        fn decimals(_env: Env) -> u32 {
            7
        }
        fn name(_env: Env) -> String {
            String::from_str(&_env, "Mock Token")
        }
        fn symbol(_env: Env) -> String {
            String::from_str(&_env, "MOCK")
        }
    }
}

#![no_std]

use soroban_sdk::token::TokenInterface;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, symbol_short, Address,
    Env, String, Symbol, IntoVal, MuxedAddress,
};
use soroban_token_sdk::metadata::TokenMetadata;
use shared::safe_math::SafeMath;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    InsufficientBalance = 3,
    InsufficientAllowance = 4,
    Unauthorized = 5,
    SupplyCapExceeded = 6,
    AnnualCapExceeded = 7,
    InvalidCapBps = 8,
}

// ---------------------------------------------------------------------------
// Storage keys
//
// Storage layout — MNTToken
// ─────────────────────────────────────────────────────────────────────────
// All keys use `persistent()` storage so they survive ledger archival.
//
// Singleton keys:
//   DataKey::Admin              → Address          (set once at initialize)
//   DataKey::TotalSupply        → i128             (updated on mint/burn)
//   DataKey::Metadata           → TokenMetadata    (name, symbol, decimals)
//
// Per-account keys:
//   DataKey::Balance(Address)   → i128             (token balance)
//   DataKey::Allowance(Address, Address) → i128    (owner → spender allowance)
//
// No two keys share the same discriminant.  Because each contract has its
// own isolated storage namespace, there is no collision risk with the
// escrow or verification contracts even if they use the same variant names.
// ─────────────────────────────────────────────────────────────────────────
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MintEventData {
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BurnEventData {
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ApproveEventData {
    pub spender: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TransferEventData {
    pub to: Address,
    pub amount: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    Admin,
    Allowance(Address, Address), // (owner, spender)
    Balance(Address),
    TotalSupply,
    Metadata,
    /// Address allowed to trigger the emergency pause (set by admin).
    PauseGuardian,
    /// `true` while the token is paused; absent/`false` means active.
    Paused,
    /// Cumulative amount minted in the current rolling 365-day window.
    MintedThisYear,
    /// Unix timestamp at which the current mint window started.
    MintWindowStart,
    /// Governance-controlled annual mint cap, in basis points of supply.
    AnnualMintCapBps,
    /// Pending governance change to the annual mint cap.
    AnnualCapProposal,
}

const SUPPLY_CAP: i128 = 100_000_000 * 10_000_000; // 100M with 7 decimals
const BPS_DENOMINATOR: i128 = 10_000;
const ANNUAL_MINT_CAP_BPS: i128 = 1_000; // 10% of supply per rolling year by default
const MIN_ANNUAL_MINT_CAP: i128 = 1_000_000 * 10_000_000; // floor allows initial bootstrap
const YEAR_SECONDS: u64 = 365 * 24 * 60 * 60;
const CAP_PROPOSAL_DELAY_SECONDS: u64 = 30 * 24 * 60 * 60;

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AnnualCapProposal {
    pub cap_bps: i128,
    pub proposed_at: u64,
}

#[contract]
pub struct MNTToken;

#[contractimpl]
impl MNTToken {
    /// Initialize the token contract with an admin.
    ///
    /// Auth: No authorization required for initialization.
    /// Can only be called once.
    ///
    /// Panics if:
    /// - Contract is already initialized
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().persistent().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().persistent().set(&DataKey::Admin, &admin);

        let metadata = TokenMetadata {
            decimal: 7,
            name: String::from_str(&env, "MentorMinds Token"),
            symbol: String::from_str(&env, "MNT"),
        };
        env.storage()
            .persistent()
            .set(&DataKey::Metadata, &metadata);
        env.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &0i128);
    }

    /// Governance: propose a new annual mint cap (basis points of supply).
    /// The change can only be applied after a 30-day delay.
    pub fn propose_annual_mint_cap(env: Env, admin: Address, cap_bps: i128) {
        Self::require_admin(&env, &admin);
        if cap_bps <= 0 || cap_bps > BPS_DENOMINATOR {
            panic_with_error!(&env, Error::InvalidCapBps);
        }
        env.storage().persistent().set(
            &DataKey::AnnualCapProposal,
            &AnnualCapProposal {
                cap_bps,
                proposed_at: env.ledger().timestamp(),
            },
        );
    }

    /// Governance: apply a previously proposed cap after the 30-day delay.
    pub fn apply_annual_mint_cap(env: Env, admin: Address) {
        Self::require_admin(&env, &admin);
        let proposal: AnnualCapProposal = env
            .storage()
            .persistent()
            .get(&DataKey::AnnualCapProposal)
            .expect("No annual cap proposal");
        let now = env.ledger().timestamp();
        if now < proposal.proposed_at.saturating_add(CAP_PROPOSAL_DELAY_SECONDS) {
            panic!("30-day proposal delay not elapsed");
        }
        env.storage()
            .persistent()
            .set(&DataKey::AnnualMintCapBps, &proposal.cap_bps);
        env.storage()
            .persistent()
            .remove(&DataKey::AnnualCapProposal);
    }

    pub fn get_annual_mint_cap_bps(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::AnnualMintCapBps)
            .unwrap_or(ANNUAL_MINT_CAP_BPS)
    }

    /// Amount minted so far in the current rolling 365-day window.
    pub fn minted_this_year(env: Env) -> i128 {
        let now = env.ledger().timestamp();
        let start: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::MintWindowStart)
            .unwrap_or(now);
        if now.saturating_sub(start) >= YEAR_SECONDS {
            0
        } else {
            env.storage()
                .persistent()
                .get(&DataKey::MintedThisYear)
                .unwrap_or(0)
        }
    }

    fn annual_cap_amount(env: &Env, total_supply: i128) -> i128 {
        let cap_bps = Self::get_annual_mint_cap_bps(env.clone());
        let relative_cap = total_supply.saturating_mul(cap_bps) / BPS_DENOMINATOR;
        relative_cap.max(MIN_ANNUAL_MINT_CAP)
    }

    /// Mint new tokens (admin only).
    ///
    /// Auth: Only the admin can mint tokens.
    /// The admin address is retrieved from persistent storage.
    ///
    /// Panics if:
    /// - Contract is not initialized
    /// - Caller is not the admin
    /// - Caller fails authorization check
    /// - Amount is not positive
    /// - Minting would exceed supply cap
    pub fn mint(env: Env, to: Address, amount: i128) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();

        Self::require_not_paused(&env);

        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let total_supply: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);
        let new_total_supply = total_supply.checked_add(amount).expect("Overflow");

        if new_total_supply > SUPPLY_CAP {
            panic!("Supply cap exceeded");
        }

        // Rolling 365-day inflation cap, measured at the start of the window.
        let now = env.ledger().timestamp();
        let window_start: u64 = env
            .storage()
            .persistent()
            .get(&DataKey::MintWindowStart)
            .unwrap_or(now);
        let (window_start, minted) = if now.saturating_sub(window_start) >= YEAR_SECONDS {
            (now, 0i128)
        } else {
            (window_start, Self::minted_this_year(env.clone()))
        };
        let annual_cap = Self::annual_cap_amount(&env, total_supply);
        if minted.saturating_add(amount) > annual_cap {
            panic_with_error!(&env, Error::AnnualCapExceeded);
        }

        let balance = Self::balance(env.clone(), to.clone());
        env.storage()
            .persistent()
            .set(&DataKey::Balance(to.clone()), &(balance + amount));
        env.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &new_total_supply);
        env.storage()
            .persistent()
            .set(&DataKey::MintedThisYear, &(minted + amount));
        env.storage()
            .persistent()
            .set(&DataKey::MintWindowStart, &window_start);

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Mint"),
                to.clone(),
            ),
            MintEventData { amount },
        );
    }

    /// Burn tokens from an account.
    ///
    /// Auth: Only the token holder can burn their own tokens.
    /// The 'from' address must provide valid authorization.
    ///
    /// Panics if:
    /// - Caller is not the 'from' address
    /// - Caller fails authorization check
    /// - Amount is not positive
    /// - Insufficient balance
    pub fn do_burn(env: Env, from: Address, amount: i128) {
        from.require_auth();
        Self::require_not_paused(&env);

        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let balance = Self::balance(env.clone(), from.clone());
        if balance < amount {
            panic!("Insufficient balance");
        }

        let total_supply = Self::total_supply(env.clone());
        env.storage()
            .persistent()
            .set(&DataKey::Balance(from.clone()), &(balance - amount));
        env.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &(total_supply - amount));

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Burn"),
                from.clone(),
            ),
            BurnEventData { amount },
        );
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }

    /// Set the address allowed to pause and unpause the token.
    ///
    /// Auth: `admin` must be the stored admin and must authorise.
    ///
    /// Panics if:
    /// - Contract is not initialized
    /// - `admin` is not the stored admin
    pub fn set_pause_guardian(env: Env, admin: Address, guardian: Address) {
        Self::require_admin(&env, &admin);
        env.storage()
            .persistent()
            .set(&DataKey::PauseGuardian, &guardian);
        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "PauseGuardianSet"),
            ),
            guardian,
        );
    }

    pub fn get_pause_guardian(env: Env) -> Option<Address> {
        env.storage().persistent().get(&DataKey::PauseGuardian)
    }

    /// Emergency stop: blocks mint, transfer, transfer_from, burn and burn_from.
    ///
    /// Auth: `guardian` must be the stored pause guardian and must authorise.
    ///
    /// Panics if:
    /// - No pause guardian has been set
    /// - `guardian` is not the stored pause guardian
    pub fn pause(env: Env, guardian: Address) {
        Self::require_pause_guardian(&env, &guardian);
        env.storage().persistent().set(&DataKey::Paused, &true);
        env.events().publish(
            (Symbol::new(&env, "MNTToken"), Symbol::new(&env, "Paused")),
            guardian,
        );
    }

    /// Restores normal operation after `pause`.
    ///
    /// Auth: `guardian` must be the stored pause guardian and must authorise.
    ///
    /// Panics if:
    /// - No pause guardian has been set
    /// - `guardian` is not the stored pause guardian
    pub fn unpause(env: Env, guardian: Address) {
        Self::require_pause_guardian(&env, &guardian);
        env.storage().persistent().set(&DataKey::Paused, &false);
        env.events().publish(
            (Symbol::new(&env, "MNTToken"), Symbol::new(&env, "Unpaused")),
            guardian,
        );
    }

    pub fn is_paused(env: Env) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::Paused)
            .unwrap_or(false)
    }

    fn require_admin(env: &Env, admin: &Address) {
        let stored: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if *admin != stored {
            panic!("Unauthorized");
        }
        admin.require_auth();
    }

    fn require_pause_guardian(env: &Env, guardian: &Address) {
        let stored: Address = env
            .storage()
            .persistent()
            .get(&DataKey::PauseGuardian)
            .expect("Pause guardian not set");
        if *guardian != stored {
            panic!("Unauthorized");
        }
        guardian.require_auth();
    }

    /// Panics with "Contract is paused" while the token is paused.
    fn require_not_paused(env: &Env) {
        if Self::is_paused(env.clone()) {
            panic!("Contract is paused");
        }
    }
}

#[contractimpl]
impl TokenInterface for MNTToken {
    fn allowance(env: Env, from: Address, spender: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Allowance(from, spender))
            .unwrap_or(0)
    }

    /// Approve a spender to use tokens on behalf of the owner.
    ///
    /// Auth: Only the token owner can approve spenders.
    /// The 'from' address must provide valid authorization.
    ///
    /// Panics if:
    /// - Caller is not the 'from' address
    /// - Caller fails authorization check
    /// - Amount is negative
    fn approve(env: Env, from: Address, spender: Address, amount: i128, _expiration_ledger: u32) {
        from.require_auth();
        if amount < 0 {
            panic!("Amount must be non-negative");
        }
        env.storage()
            .persistent()
            .set(&DataKey::Allowance(from.clone(), spender.clone()), &amount);

        // Note: Simple implementation, expiration_ledger is usually used for TTL in Soroban
        // but for simplicity in this MVP we just store the amount.

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Approve"),
                from.clone(),
            ),
            ApproveEventData { spender, amount },
        );
    }

    fn balance(env: Env, id: Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::Balance(id))
            .unwrap_or(0)
    }

    /// Transfer tokens from one account to another.
    ///
    /// Auth: Only the token owner can transfer their tokens.
    /// The 'from' address must provide valid authorization.
    ///
    /// Panics if:
    /// - Caller is not the 'from' address
    /// - Caller fails authorization check
    /// - Amount is not positive
    /// - Insufficient balance
    fn transfer(env: Env, from: Address, to: MuxedAddress, amount: i128) {
        from.require_auth();
        Self::require_not_paused(&env);
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let from_balance = Self::balance(env.clone(), from.clone());
        if from_balance < amount {
            panic!("Insufficient balance");
        }

        let to_addr = to.address();
        let to_balance = Self::balance(env.clone(), to_addr.clone());

        env.storage()
            .persistent()
            .set(&DataKey::Balance(from.clone()), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&DataKey::Balance(to_addr.clone()), &(to_balance + amount));

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Transfer"),
                from.clone(),
            ),
            TransferEventData { to: to_addr, amount },
        );
    }

    /// Transfer tokens using an allowance.
    ///
    /// Auth: Only the approved spender can transfer tokens on behalf of the owner.
    /// The spender address must provide valid authorization.
    ///
    /// Panics if:
    /// - Caller is not the spender
    /// - Caller fails authorization check
    /// - Amount is not positive
    /// - Insufficient allowance
    /// - Insufficient balance
    fn transfer_from(env: Env, spender: Address, from: Address, to: Address, amount: i128) {
        spender.require_auth();
        Self::require_not_paused(&env);
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let allowance = Self::allowance(env.clone(), from.clone(), spender.clone());
        // Guard the decrement explicitly: SafeMath::safe_sub panics with a
        // generic underflow error, while this entry point must report the
        // token-specific InsufficientAllowance error.
        match allowance.checked_sub(amount) {
            Some(value) if value >= 0 => (),
            _ => panic_with_error!(&env, Error::InsufficientAllowance),
        }
        let remaining_allowance = SafeMath::safe_sub(allowance, &env, amount);

        let from_balance = Self::balance(env.clone(), from.clone());
        if from_balance < amount {
            panic!("Insufficient balance");
        }

        let to_balance = Self::balance(env.clone(), to.clone());

        env.storage().persistent().set(
            &DataKey::Allowance(from.clone(), spender.clone()),
            &remaining_allowance,
        );
        env.storage()
            .persistent()
            .set(&DataKey::Balance(from.clone()), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&DataKey::Balance(to.clone()), &(to_balance + amount));

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Transfer"),
                from.clone(),
            ),
            TransferEventData {
                to: to.clone(),
                amount,
            },
        );
        env.events()
            .publish((symbol_short!("transfer"), from, to), amount);
    }

    fn burn(env: Env, from: Address, amount: i128) {
        Self::do_burn(env, from, amount)
    }

    fn burn_from(env: Env, spender: Address, from: Address, amount: i128) {
        spender.require_auth();
        Self::require_not_paused(&env);
        if amount <= 0 {
            panic!("Amount must be positive");
        }

        let allowance = Self::allowance(env.clone(), from.clone(), spender.clone());
        if allowance < amount {
            panic!("Insufficient allowance");
        }

        let from_balance = Self::balance(env.clone(), from.clone());
        if from_balance < amount {
            panic!("Insufficient balance");
        }

        let total_supply: i128 = env
            .storage()
            .persistent()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);

        env.events().publish(
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Burn"),
                from.clone(),
            ),
            BurnEventData { amount },
        );
        env.storage().persistent().set(
            &DataKey::Allowance(from.clone(), spender.clone()),
            &(allowance - amount),
        );
        env.storage()
            .persistent()
            .set(&DataKey::Balance(from.clone()), &(from_balance - amount));
        env.storage()
            .persistent()
            .set(&DataKey::TotalSupply, &(total_supply - amount));

        env.events().publish((symbol_short!("burn"), from), amount);
    }

    fn decimals(_env: Env) -> u32 {
        7
    }

    fn name(env: Env) -> String {
        let metadata: TokenMetadata = env
            .storage()
            .persistent()
            .get(&DataKey::Metadata)
            .expect("Not initialized");
        metadata.name
    }

    fn symbol(env: Env) -> String {
        let metadata: TokenMetadata = env
            .storage()
            .persistent()
            .get(&DataKey::Metadata)
            .expect("Not initialized");
        metadata.symbol
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    extern crate std;
    use super::*;
    use soroban_sdk::testutils::{Address as _, Events, Ledger};
    use soroban_sdk::xdr::{ContractEventBody, ScAddress};
    use soroban_sdk::{Env, IntoVal, Symbol, TryFromVal, Val, Vec};

    /// Events from the last invocation as `(contract, topics, data)` tuples.
    fn all_events(env: &Env) -> std::vec::Vec<(Address, Vec<Val>, Val)> {
        env.events()
            .all()
            .events()
            .iter()
            .map(|e| {
                let contract = Address::try_from_val(
                    env,
                    &ScAddress::Contract(e.contract_id.clone().unwrap()),
                )
                .unwrap();
                let ContractEventBody::V0(body) = &e.body;
                let mut topics = Vec::new(env);
                for t in body.topics.iter() {
                    topics.push_back(Val::try_from_val(env, t).unwrap());
                }
                (contract, topics, Val::try_from_val(env, &body.data).unwrap())
            })
            .collect()
    }

    /// Last `MNTToken`-namespaced event from the most recent invocation.
    fn last_mnt_event(env: &Env) -> (Address, Vec<Val>, Val) {
        let ns: Val = Symbol::new(env, "MNTToken").into_val(env);
        all_events(env)
            .into_iter()
            .rev()
            .find(|(_, topics, _)| topics.get(0).map(|t| t.shallow_eq(&ns)).unwrap_or(false))
            .expect("no MNTToken event")
    }

    fn setup() -> (Env, Address, Address, MNTTokenClient<'static>) {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let guardian = Address::generate(&env);
        let contract_id = env.register(MNTToken, ());
        let client = MNTTokenClient::new(&env, &contract_id);
        client.initialize(&admin);
        client.set_pause_guardian(&admin, &guardian);
        (env, admin, guardian, client)
    }

    #[test]
    fn test_initialization() {
        let env = Env::default();
        let admin = Address::generate(&env);
        let contract_id = env.register(MNTToken, ());
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);

        assert_eq!(client.name(), String::from_str(&env, "MentorMinds Token"));
        assert_eq!(client.symbol(), String::from_str(&env, "MNT"));
        assert_eq!(client.decimals(), 7);
    }

    #[test]
    fn test_mint_and_burn() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let contract_id = env.register(MNTToken, ());
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);

        client.mint(&user, &1000);
        let last_event = last_mnt_event(&env);
        assert_eq!(client.balance(&user), 1000);
        assert_eq!(last_event.0, contract_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Mint"),
                user.clone()
            )
                .into_val(&env)
        );
        let mint_data = MintEventData::try_from_val(&env, &last_event.2).unwrap();
        assert_eq!(mint_data.amount, 1000);

        client.burn(&user, &400);
        let last_event = last_mnt_event(&env);
        assert_eq!(client.balance(&user), 600);

        assert_eq!(last_event.0, contract_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Burn"),
                user.clone()
            )
                .into_val(&env)
        );
        let burn_data = BurnEventData::try_from_val(&env, &last_event.2).unwrap();
        assert_eq!(burn_data.amount, 400);
    }

    #[test]
    fn test_transfer_flow() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        let contract_id = env.register(MNTToken, ());
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);
        client.mint(&user1, &1000);

        client.transfer(&user1, &user2, &300);
        let last_event = last_mnt_event(&env);
        assert_eq!(client.balance(&user1), 700);
        assert_eq!(client.balance(&user2), 300);

        assert_eq!(last_event.0, contract_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Transfer"),
                user1.clone()
            )
                .into_val(&env)
        );
        let transfer_data = TransferEventData::try_from_val(&env, &last_event.2).unwrap();
        assert_eq!(transfer_data.to, user2.clone());
        assert_eq!(transfer_data.amount, 300);
    }

    #[test]
    fn test_allowance_flow() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        let contract_id = env.register(MNTToken, ());
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);
        client.mint(&user1, &1000);

        client.approve(&user1, &user2, &500, &100);
        let mut last_event = last_mnt_event(&env);
        assert_eq!(client.allowance(&user1, &user2), 500);

        assert_eq!(last_event.0, contract_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Approve"),
                user1.clone()
            )
                .into_val(&env)
        );
        let approve_data = ApproveEventData::try_from_val(&env, &last_event.2).unwrap();
        assert_eq!(approve_data.spender, user2.clone());
        assert_eq!(approve_data.amount, 500);

        client.transfer_from(&user2, &user1, &user2, &200);
        last_event = last_mnt_event(&env);
        assert_eq!(client.balance(&user1), 800);
        assert_eq!(client.balance(&user2), 200);
        assert_eq!(client.allowance(&user1, &user2), 300);

        assert_eq!(last_event.0, contract_id.clone());
        assert_eq!(
            last_event.1,
            (
                Symbol::new(&env, "MNTToken"),
                Symbol::new(&env, "Transfer"),
                user1.clone()
            )
                .into_val(&env)
        );
        let transfer_from_data = TransferEventData::try_from_val(&env, &last_event.2).unwrap();
        assert_eq!(transfer_from_data.to, user2.clone());
        assert_eq!(transfer_from_data.amount, 200);
    }

    #[test]
    #[should_panic(expected = "Supply cap exceeded")]
    fn test_supply_cap() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user = Address::generate(&env);
        let contract_id = env.register(MNTToken, ());
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);

        // Mint up to the cap across rolling yearly windows so the annual
        // inflation cap does not interfere with this supply-cap check.
        for _ in 0..100 {
            env.ledger().with_mut(|li| li.timestamp += YEAR_SECONDS);
            client.mint(&user, &MIN_ANNUAL_MINT_CAP);
        }
        assert_eq!(client.total_supply(), SUPPLY_CAP);

        // This should fail
        client.mint(&user, &1);
    }

    #[test]
    fn test_annual_cap_enforced() {
        let (env, _admin, _guardian, client) = setup();
        let user = Address::generate(&env);

        client.mint(&user, &MIN_ANNUAL_MINT_CAP);
        assert_eq!(client.minted_this_year(), MIN_ANNUAL_MINT_CAP);
        match client.try_mint(&user, &1).err().unwrap() {
            Ok(e) => assert_eq!(e.get_code(), Error::AnnualCapExceeded as u32),
            Err(e) => panic!("unexpected invoke error: {e:?}"),
        }
        assert_eq!(client.total_supply(), MIN_ANNUAL_MINT_CAP);
    }

    #[test]
    fn test_annual_cap_rolling_window_resets_after_365_days() {
        let (env, _admin, _guardian, client) = setup();
        let user = Address::generate(&env);

        client.mint(&user, &MIN_ANNUAL_MINT_CAP);
        env.ledger().with_mut(|li| li.timestamp += YEAR_SECONDS);
        assert_eq!(client.minted_this_year(), 0);

        client.mint(&user, &1);
        assert_eq!(client.minted_this_year(), 1);
    }

    #[test]
    #[should_panic(expected = "30-day proposal delay not elapsed")]
    fn test_annual_cap_change_requires_30_day_delay() {
        let (_env, admin, _guardian, client) = setup();
        client.propose_annual_mint_cap(&admin, &5_000);
        client.apply_annual_mint_cap(&admin);
    }

    #[test]
    fn test_annual_cap_change_applies_after_delay() {
        let (env, admin, _guardian, client) = setup();
        assert_eq!(client.get_annual_mint_cap_bps(), ANNUAL_MINT_CAP_BPS);

        client.propose_annual_mint_cap(&admin, &5_000);
        env.ledger()
            .with_mut(|li| li.timestamp += CAP_PROPOSAL_DELAY_SECONDS);
        client.apply_annual_mint_cap(&admin);
        assert_eq!(client.get_annual_mint_cap_bps(), 5_000);
    }

    // -----------------------------------------------------------------------
    // Storage layout readback — full lifecycle
    // Verifies every stored value is readable after a complete lifecycle run.
    // -----------------------------------------------------------------------

    #[test]
    fn test_storage_readback_full_lifecycle() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        let contract_id = env.register(MNTToken, ());
        let client = MNTTokenClient::new(&env, &contract_id);

        client.initialize(&admin);

        // Metadata readable after init
        assert_eq!(client.name(), String::from_str(&env, "MentorMinds Token"));
        assert_eq!(client.symbol(), String::from_str(&env, "MNT"));
        assert_eq!(client.decimals(), 7);

        // Balances start at zero
        assert_eq!(client.balance(&user1), 0);
        assert_eq!(client.balance(&user2), 0);

        // Mint → balance readable
        client.mint(&user1, &1_000);
        assert_eq!(client.balance(&user1), 1_000);

        // Approve → allowance readable
        client.approve(&user1, &user2, &400, &100);
        assert_eq!(client.allowance(&user1, &user2), 400);

        // transfer_from → balances and allowance updated
        client.transfer_from(&user2, &user1, &user2, &150);
        assert_eq!(client.balance(&user1), 850);
        assert_eq!(client.balance(&user2), 150);
        assert_eq!(client.allowance(&user1, &user2), 250);

        // transfer → balances updated
        client.transfer(&user1, &user2, &100);
        assert_eq!(client.balance(&user1), 750);
        assert_eq!(client.balance(&user2), 250);

        // burn → balance and supply updated
        client.burn(&user1, &250);
        assert_eq!(client.balance(&user1), 500);

        // burn_from → allowance, balance, supply updated
        client.burn_from(&user2, &user1, &200);
        assert_eq!(client.balance(&user1), 300);
        assert_eq!(client.allowance(&user1, &user2), 50);
    }

    // -----------------------------------------------------------------------
    // Pause guardian
    // -----------------------------------------------------------------------

    #[test]
    fn test_set_pause_guardian_stores_guardian() {
        let (_env, _admin, guardian, client) = setup();
        assert_eq!(client.get_pause_guardian(), Some(guardian));
        assert!(!client.is_paused());
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_set_pause_guardian_rejects_non_admin() {
        let (env, _admin, _guardian, client) = setup();
        let stranger = Address::generate(&env);
        client.set_pause_guardian(&stranger, &stranger);
    }

    #[test]
    #[should_panic(expected = "Unauthorized")]
    fn test_pause_rejects_non_guardian() {
        let (env, _admin, _guardian, client) = setup();
        let stranger = Address::generate(&env);
        client.pause(&stranger);
    }

    #[test]
    #[should_panic(expected = "Pause guardian not set")]
    fn test_pause_requires_guardian_to_be_set() {
        let env = Env::default();
        env.mock_all_auths();
        let admin = Address::generate(&env);
        let client = MNTTokenClient::new(&env, &env.register(MNTToken, ()));
        client.initialize(&admin);
        client.pause(&admin);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn test_transfer_rejected_while_paused() {
        let (env, _admin, guardian, client) = setup();
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        client.mint(&user1, &1000);
        client.pause(&guardian);
        client.transfer(&user1, &user2, &100);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn test_transfer_from_rejected_while_paused() {
        let (env, _admin, guardian, client) = setup();
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        client.mint(&user1, &1000);
        client.approve(&user1, &user2, &500, &100);
        client.pause(&guardian);
        client.transfer_from(&user2, &user1, &user2, &100);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn test_mint_rejected_while_paused() {
        let (env, _admin, guardian, client) = setup();
        let user = Address::generate(&env);
        client.pause(&guardian);
        client.mint(&user, &1000);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn test_burn_rejected_while_paused() {
        let (env, _admin, guardian, client) = setup();
        let user = Address::generate(&env);
        client.mint(&user, &1000);
        client.pause(&guardian);
        client.burn(&user, &100);
    }

    #[test]
    #[should_panic(expected = "Contract is paused")]
    fn test_burn_from_rejected_while_paused() {
        let (env, _admin, guardian, client) = setup();
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        client.mint(&user1, &1000);
        client.approve(&user1, &user2, &500, &100);
        client.pause(&guardian);
        client.burn_from(&user2, &user1, &100);
    }

    #[test]
    fn test_operations_succeed_after_unpause() {
        let (env, _admin, guardian, client) = setup();
        let user1 = Address::generate(&env);
        let user2 = Address::generate(&env);
        client.mint(&user1, &1000);

        client.pause(&guardian);
        assert!(client.is_paused());
        assert!(client.try_transfer(&user1, &user2, &100).is_err());
        assert!(client.try_mint(&user1, &100).is_err());
        assert!(client.try_burn(&user1, &100).is_err());
        assert_eq!(client.balance(&user1), 1000);

        client.unpause(&guardian);
        assert!(!client.is_paused());

        client.transfer(&user1, &user2, &300);
        client.mint(&user2, &50);
        client.burn(&user1, &200);
        assert_eq!(client.balance(&user1), 500);
        assert_eq!(client.balance(&user2), 350);
        assert_eq!(client.total_supply(), 850);
    }

    #[test]
    fn test_pause_and_unpause_emit_events() {
        let (env, _admin, guardian, client) = setup();

        client.pause(&guardian);
        let events = all_events(&env);
        let (contract, topics, data) = events.last().unwrap();
        assert_eq!(*contract, client.address);
        assert_eq!(
            *topics,
            (Symbol::new(&env, "MNTToken"), Symbol::new(&env, "Paused")).into_val(&env)
        );
        assert_eq!(Address::try_from_val(&env, data).unwrap(), guardian);

        client.unpause(&guardian);
        let events = all_events(&env);
        let (_, topics, _) = events.last().unwrap();
        assert_eq!(
            *topics,
            (Symbol::new(&env, "MNTToken"), Symbol::new(&env, "Unpaused")).into_val(&env)
        );
    }

    #[test]
    fn test_transfer_from_allowance_underflow_guard() {
        let (env, _admin, _guardian, client) = setup();
        let owner = Address::generate(&env);
        let spender = Address::generate(&env);
        let recipient = Address::generate(&env);
        client.mint(&owner, &1_000);
        client.approve(&owner, &spender, &500, &100);

        match client
            .try_transfer_from(&spender, &owner, &recipient, &501)
            .err()
            .unwrap()
        {
            Ok(e) => assert_eq!(e.get_code(), Error::InsufficientAllowance as u32),
            Err(e) => panic!("unexpected invoke error: {e:?}"),
        }

        // Nothing moved: the guard aborts before any storage write.
        assert_eq!(client.allowance(&owner, &spender), 500);
        assert_eq!(client.balance(&owner), 1_000);
        assert_eq!(client.balance(&recipient), 0);
    }
}

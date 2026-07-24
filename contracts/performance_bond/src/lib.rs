#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, token, Address, Env, Symbol,
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
    NotAdmin = 3,
    InvalidAmount = 4,
    BelowMinimum = 5,
    AlreadyBonded = 6,
    NoBondFound = 7,
    StillInCooldown = 8,
    InsufficientBond = 9,
}

// ---------------------------------------------------------------------------
// Data Types
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BondRecord {
    pub mentor: Address,
    pub amount: i128,
    pub posted_at: u64,
    pub last_slash_at: u64,
    pub slash_count: u32,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MINIMUM_BOND: i128 = 100_000_000; // 100 MNT (with 7 decimals)
const COOLDOWN_DAYS: u64 = 30;
const COOLDOWN_SECONDS: u64 = COOLDOWN_DAYS * 86_400;

// Slash amounts (with 7 decimals)
#[allow(dead_code)]
const SLASH_NO_SHOW: i128 = 10_000_000; // 10 MNT
#[allow(dead_code)]
const SLASH_DISPUTE_LOST: i128 = 50_000_000; // 50 MNT

// ---------------------------------------------------------------------------
// Storage Keys
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    MntToken,
    InsurancePool,
    Bond(Address),
}

// ---------------------------------------------------------------------------
// Contract
// ---------------------------------------------------------------------------

#[contract]
pub struct PerformanceBondContract;

#[contractimpl]
impl PerformanceBondContract {
    /// Initialize the performance bond contract.
    pub fn initialize(
        env: Env,
        admin: Address,
        mnt_token: Address,
        insurance_pool: Address,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::MntToken, &mnt_token);
        env.storage()
            .instance()
            .set(&DataKey::InsurancePool, &insurance_pool);
        Ok(())
    }

    /// Post a performance bond.
    /// Minimum 100 MNT required to activate.
    ///
    /// Auth: mentor must authorize this call.
    pub fn post_bond(env: Env, mentor: Address, amount: i128) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if amount < MINIMUM_BOND {
            return Err(Error::BelowMinimum);
        }

        // Check if already bonded
        if env
            .storage()
            .persistent()
            .has(&DataKey::Bond(mentor.clone()))
        {
            return Err(Error::AlreadyBonded);
        }

        mentor.require_auth();

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MntToken)
            .ok_or(Error::NotInitialized)?;

        // Transfer MNT from mentor to this contract
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&mentor, &env.current_contract_address(), &amount);

        let record = BondRecord {
            mentor: mentor.clone(),
            amount,
            posted_at: env.ledger().timestamp(),
            last_slash_at: 0,
            slash_count: 0,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Bond(mentor.clone()), &record);

        env.events().publish(
            (
                symbol_short!("bond"),
                symbol_short!("posted"),
                mentor.clone(),
            ),
            amount,
        );

        Ok(())
    }

    /// Slash a mentor's bond.
    /// Admin only.
    ///
    /// reason: "no_show" = 10 MNT, "dispute_lost" = 50 MNT, "fraud" = full bond
    pub fn slash_bond(
        env: Env,
        mentor: Address,
        amount: i128,
        reason: Symbol,
    ) -> Result<(), Error> {
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;

        admin.require_auth();

        let mut record: BondRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bond(mentor.clone()))
            .ok_or(Error::NoBondFound)?;

        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        if amount > record.amount {
            return Err(Error::InsufficientBond);
        }

        let insurance_pool: Address = env
            .storage()
            .instance()
            .get(&DataKey::InsurancePool)
            .ok_or(Error::NotInitialized)?;

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MntToken)
            .ok_or(Error::NotInitialized)?;

        // Transfer slashed amount to insurance pool
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&env.current_contract_address(), &insurance_pool, &amount);

        // Update bond record
        record.amount -= amount;
        record.last_slash_at = env.ledger().timestamp();
        record.slash_count += 1;

        if record.amount == 0 {
            // Bond fully slashed - remove record
            env.storage()
                .persistent()
                .remove(&DataKey::Bond(mentor.clone()));
        } else {
            env.storage()
                .persistent()
                .set(&DataKey::Bond(mentor.clone()), &record);
        }

        env.events().publish(
            (
                symbol_short!("bond"),
                symbol_short!("slashed"),
                mentor.clone(),
            ),
            (amount, reason),
        );

        Ok(())
    }

    /// Release a mentor's bond after cooldown period with no disputes.
    /// Mentor can withdraw their bond after 30 days with no slashes.
    ///
    /// Auth: mentor must authorize this call.
    pub fn release_bond(env: Env, mentor: Address) -> Result<(), Error> {
        if !env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::NotInitialized);
        }

        let record: BondRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Bond(mentor.clone()))
            .ok_or(Error::NoBondFound)?;

        mentor.require_auth();

        let now = env.ledger().timestamp();

        // Check cooldown period since last slash
        if record.last_slash_at > 0 && now < record.last_slash_at + COOLDOWN_SECONDS {
            return Err(Error::StillInCooldown);
        }

        // Also check from posting time if no slashes
        if record.last_slash_at == 0 && now < record.posted_at + COOLDOWN_SECONDS {
            return Err(Error::StillInCooldown);
        }

        let mnt_token: Address = env
            .storage()
            .instance()
            .get(&DataKey::MntToken)
            .ok_or(Error::NotInitialized)?;

        // Transfer bond back to mentor
        let token_client = token::Client::new(&env, &mnt_token);
        token_client.transfer(&env.current_contract_address(), &mentor, &record.amount);

        // Remove bond record
        env.storage()
            .persistent()
            .remove(&DataKey::Bond(mentor.clone()));

        env.events().publish(
            (
                symbol_short!("bond"),
                symbol_short!("released"),
                mentor.clone(),
            ),
            record.amount,
        );

        Ok(())
    }

    /// Get bond record for a mentor.
    pub fn get_bond(env: Env, mentor: Address) -> Result<BondRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Bond(mentor))
            .ok_or(Error::NoBondFound)
    }

    /// Check if a mentor is bonded (has active bond).
    /// Used by escrow to verify mentor can accept bookings.
    pub fn is_bonded(env: Env, mentor: Address) -> bool {
        env.storage().persistent().has(&DataKey::Bond(mentor))
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod test {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Ledger};
    use soroban_sdk::Env;

    // Mock MNT Token
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
        bond_id: Address,
        mnt_id: Address,
        admin: Address,
        insurance_pool: Address,
    }

    impl Fixture {
        fn setup() -> Self {
            let env = Env::default();
            env.mock_all_auths();

            let admin = Address::generate(&env);
            let insurance_pool = Address::generate(&env);
            let mnt_id = env.register_contract(None, MockMNT);

            let bond_id = env.register_contract(None, PerformanceBondContract);
            PerformanceBondContractClient::new(&env, &bond_id).initialize(
                &admin,
                &mnt_id,
                &insurance_pool,
            );

            Fixture {
                env,
                bond_id,
                mnt_id,
                admin,
                insurance_pool,
            }
        }

        fn client(&self) -> PerformanceBondContractClient {
            PerformanceBondContractClient::new(&self.env, &self.bond_id)
        }

        fn mnt(&self) -> MockMNTClient {
            MockMNTClient::new(&self.env, &self.mnt_id)
        }

        fn fund(&self, addr: &Address, amount: i128) {
            self.mnt().mint(addr, &amount);
        }
    }

    #[test]
    fn test_post_bond() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 200_000_000);

        f.client().post_bond(&mentor, &100_000_000);

        assert!(f.client().is_bonded(&mentor));
        let bond = f.client().get_bond(&mentor);
        assert_eq!(bond.amount, 100_000_000);
        assert_eq!(f.mnt().balance(&f.bond_id), 100_000_000);
    }

    #[test]
    fn test_post_bond_below_minimum() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 50_000_000);

        let result = f.client().try_post_bond(&mentor, &50_000_000);
        assert_eq!(result, Err(Ok(Error::BelowMinimum)));
    }

    #[test]
    fn test_slash_bond() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 200_000_000);

        f.client().post_bond(&mentor, &200_000_000);

        // Slash 10 MNT for no-show
        f.client()
            .slash_bond(&mentor, &10_000_000, &symbol_short!("no_show"));

        let bond = f.client().get_bond(&mentor);
        assert_eq!(bond.amount, 190_000_000);
        assert_eq!(bond.slash_count, 1);
        assert_eq!(f.mnt().balance(&f.insurance_pool), 10_000_000);
    }

    #[test]
    fn test_full_slash() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 100_000_000);

        f.client().post_bond(&mentor, &100_000_000);

        // Full slash for fraud
        f.client()
            .slash_bond(&mentor, &100_000_000, &symbol_short!("fraud"));

        assert!(!f.client().is_bonded(&mentor));
        assert_eq!(f.mnt().balance(&f.insurance_pool), 100_000_000);
    }

    #[test]
    fn test_release_bond_after_cooldown() {
        let f = Fixture::setup();
        f.env.ledger().set_timestamp(0);

        let mentor = Address::generate(&f.env);
        f.fund(&mentor, 100_000_000);

        f.client().post_bond(&mentor, &100_000_000);

        // Try to release immediately - should fail
        let result = f.client().try_release_bond(&mentor);
        assert_eq!(result, Err(Ok(Error::StillInCooldown)));

        // Advance past cooldown
        f.env.ledger().set_timestamp(31 * 86_400);

        f.client().release_bond(&mentor);
        assert!(!f.client().is_bonded(&mentor));
        assert_eq!(f.mnt().balance(&mentor), 100_000_000);
    }

    #[test]
    fn test_is_bonded_booking_gate() {
        let f = Fixture::setup();
        let mentor = Address::generate(&f.env);

        // Not bonded
        assert!(!f.client().is_bonded(&mentor));

        // Post bond
        f.fund(&mentor, 100_000_000);
        f.client().post_bond(&mentor, &100_000_000);

        // Now bonded
        assert!(f.client().is_bonded(&mentor));
    }
}

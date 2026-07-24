#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, symbol_short, Address, BytesN, Env};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    Verification(Address),
    Tier(Address),
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct VerificationRecord {
    pub credential_hash: BytesN<32>,
    pub verified_at: u64,
    pub expiry: u64,
    pub is_active: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MentorVerifiedEventData {
    pub credential_hash: BytesN<32>,
    pub verified_at: u64,
    pub expiry: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VerificationRevokedEventData {
    pub revoked: bool,
}

#[contract]
pub struct VerificationContract;

#[contractimpl]
impl VerificationContract {
    /// Initialize the verification contract with an admin.
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
    }

    /// Verify a mentor with credentials (admin only).
    ///
    /// Auth: Only the admin can verify mentors.
    /// The admin address is retrieved from persistent storage.
    ///
    /// Panics if:
    /// - Contract is not initialized
    /// - Caller is not the admin
    /// - Caller fails authorization check
    pub fn verify_mentor(env: Env, mentor: Address, credential_hash: BytesN<32>, expiry: u64) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        let now = env.ledger().timestamp();
        let rec = VerificationRecord {
            credential_hash,
            verified_at: now,
            expiry,
            is_active: true,
        };
        let key = DataKey::Verification(mentor.clone());
        env.storage().persistent().set(&key, &rec);
        let tkey = DataKey::Tier(mentor.clone());
        if !env.storage().persistent().has(&tkey) {
            // New mentors start from the base tier until a separate promotion
            // path raises their score.
            env.storage().persistent().set(&tkey, &0i32);
        }
        env.events().publish(
            (
                symbol_short!("Verify"),
                symbol_short!("VrfyOk"),
                mentor.clone(),
            ),
            MentorVerifiedEventData {
                credential_hash: rec.credential_hash.clone(),
                verified_at: rec.verified_at,
                expiry: rec.expiry,
            },
        );
    }

    /// Revoke a mentor's verification (admin only).
    ///
    /// Auth: Only the admin can revoke verifications.
    /// The admin address is retrieved from persistent storage.
    ///
    /// Panics if:
    /// - Contract is not initialized
    /// - Caller is not the admin
    /// - Caller fails authorization check
    /// - Mentor is not verified
    pub fn revoke_verification(env: Env, mentor: Address) {
        let admin: Address = env
            .storage()
            .persistent()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        admin.require_auth();
        let key = DataKey::Verification(mentor.clone());
        let mut rec: VerificationRecord =
            env.storage().persistent().get(&key).expect("Not verified");
        rec.is_active = false;
        env.storage().persistent().set(&key, &rec);
        env.events().publish(
            (
                symbol_short!("Verify"),
                symbol_short!("Revoke"),
                mentor.clone(),
            ),
            VerificationRevokedEventData { revoked: true },
        );
    }

    pub fn is_verified(env: Env, mentor: Address) -> bool {
        let key = DataKey::Verification(mentor);
        let rec: Option<VerificationRecord> = env.storage().persistent().get(&key);
        match rec {
            None => false,
            // Verification is only valid while the record is active and the
            // recorded expiry has not been reached yet.
            Some(r) => r.is_active && env.ledger().timestamp() <= r.expiry,
        }
    }

    pub fn get_verification(env: Env, mentor: Address) -> VerificationRecord {
        let key = DataKey::Verification(mentor);
        env.storage().persistent().get(&key).expect("Not verified")
    }
}

#[cfg(test)]
mod test {}

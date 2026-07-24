#![no_std]
use soroban_sdk::{
    contract, contractclient, contractimpl, contracttype, symbol_short, Address, BytesN, Env,
    Symbol,
};

#[contracttype]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd)]
pub enum KycLevel {
    None = 0,
    Basic = 1,
    Enhanced = 2,
    Institutional = 3,
}

#[contracttype]
#[derive(Clone, Debug)]
pub struct KycRecord {
    pub level: KycLevel,
    pub expiry: u64,
    pub kyc_provider_hash: BytesN<32>,
}

#[contracttype]
pub enum DataKey {
    Admin,
    Rbac,
    Kyc(Address),
}

#[contractclient(name = "RbacContractClient")]
pub trait RbacContractTrait {
    fn has_role(env: Env, role: Symbol, account: Address) -> bool;
}

#[contract]
pub struct KycRegistry;

#[contractimpl]
impl KycRegistry {
    /// Initialize the contract with an admin.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("Already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    pub fn set_rbac_contract(env: Env, admin: Address, rbac: Address) {
        Self::require_admin(&env, &admin);
        env.storage().instance().set(&DataKey::Rbac, &rbac);
    }

    /// Set the KYC level for a user. Admin only.
    pub fn set_kyc_level(
        env: Env,
        operator: Address,
        user: Address,
        level: KycLevel,
        expiry: u64,
        provider_hash: BytesN<32>,
    ) {
        Self::require_operator(&env, &operator);

        let now = env.ledger().timestamp();
        if expiry <= now {
            panic!("KYC expiry must be in the future");
        }

        let record = KycRecord {
            level,
            expiry,
            kyc_provider_hash: provider_hash,
        };

        env.storage()
            .persistent()
            .set(&DataKey::Kyc(user.clone()), &record);

        env.events()
            .publish((symbol_short!("kyc_set"), user), record.level);
    }

    /// Get the KYC level for a user. Returns None if expired or not found.
    pub fn get_kyc_level(env: Env, user: Address) -> KycLevel {
        match env
            .storage()
            .persistent()
            .get::<_, KycRecord>(&DataKey::Kyc(user))
        {
            Some(record) => {
                if env.ledger().timestamp() > record.expiry {
                    KycLevel::None
                } else {
                    record.level
                }
            }
            None => KycLevel::None,
        }
    }

    /// Check if a user's KYC level is valid and meets the minimum required level.
    pub fn is_kyc_valid(env: Env, user: Address, min_level: KycLevel) -> bool {
        let current_level = Self::get_kyc_level(env, user);
        current_level >= min_level && current_level != KycLevel::None
    }

    /// Revoke KYC for a user immediately. Admin only.
    pub fn revoke_kyc(env: Env, operator: Address, user: Address) {
        Self::require_operator(&env, &operator);

        env.storage()
            .persistent()
            .remove(&DataKey::Kyc(user.clone()));

        env.events().publish((symbol_short!("kyc_rvk"), user), ());
    }

    /// Internal helper to require admin authorization.
    fn require_admin(env: &Env, admin: &Address) {
        admin.require_auth();
        let stored_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if stored_admin != *admin {
            panic!("Admin address mismatch");
        }
    }

    fn require_operator(env: &Env, operator: &Address) {
        operator.require_auth();
        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Not initialized");
        if admin == *operator {
            return;
        }

        let rbac: Address = env
            .storage()
            .instance()
            .get(&DataKey::Rbac)
            .expect("RBAC not configured");
        if !RbacContractClient::new(env, &rbac)
            .has_role(&Symbol::new(env, "KYC_OPERATOR"), operator)
        {
            panic!("KYC_OPERATOR role required");
        }
    }
}

#[cfg(test)]
mod test;

#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol, Vec};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized = 2,
    RoleNotGranted = 3,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    SuperAdmin,
    RoleMember(Symbol, Address),
    RoleMembers(Symbol),
}

#[contract]
pub struct RbacContract;

#[contractimpl]
impl RbacContract {
    pub fn initialize(env: Env, super_admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::SuperAdmin) {
            return Err(Error::AlreadyInitialized);
        }

        super_admin.require_auth();
        env.storage()
            .instance()
            .set(&DataKey::SuperAdmin, &super_admin);
        Self::grant_internal(&env, &Self::super_admin_role(env.clone()), &super_admin);
        Ok(())
    }

    pub fn grant_role(
        env: Env,
        caller: Address,
        role: Symbol,
        account: Address,
    ) -> Result<(), Error> {
        Self::require_super_admin(&env, &caller)?;
        Self::grant_internal(&env, &role, &account);
        Ok(())
    }

    pub fn revoke_role(
        env: Env,
        caller: Address,
        role: Symbol,
        account: Address,
    ) -> Result<(), Error> {
        Self::require_super_admin(&env, &caller)?;
        let member_key = DataKey::RoleMember(role.clone(), account.clone());
        if !env.storage().persistent().has(&member_key) {
            return Err(Error::RoleNotGranted);
        }

        env.storage().persistent().remove(&member_key);
        let members_key = DataKey::RoleMembers(role.clone());
        let members: Vec<Address> = env
            .storage()
            .persistent()
            .get(&members_key)
            .unwrap_or(Vec::new(&env));
        let mut next = Vec::new(&env);
        for member in members.iter() {
            if member != account {
                next.push_back(member);
            }
        }
        env.storage().persistent().set(&members_key, &next);
        env.events()
            .publish((Symbol::new(&env, "role_revoked"), role), account);
        Ok(())
    }

    pub fn has_role(env: Env, role: Symbol, account: Address) -> bool {
        env.storage()
            .persistent()
            .get(&DataKey::RoleMember(role, account))
            .unwrap_or(false)
    }

    pub fn require_role(env: Env, role: Symbol, account: Address) -> Result<(), Error> {
        account.require_auth();
        if !Self::has_role(env, role, account) {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    pub fn get_role_members(env: Env, role: Symbol) -> Vec<Address> {
        env.storage()
            .persistent()
            .get(&DataKey::RoleMembers(role))
            .unwrap_or(Vec::new(&env))
    }

    pub fn super_admin_role(env: Env) -> Symbol {
        Symbol::new(&env, "SUPER_ADMIN")
    }

    pub fn escrow_admin_role(env: Env) -> Symbol {
        Symbol::new(&env, "ESCROW_ADMIN")
    }

    pub fn dispute_resolver_role(env: Env) -> Symbol {
        Symbol::new(&env, "DISPUTE_RESOLVER")
    }

    pub fn oracle_admin_role(env: Env) -> Symbol {
        Symbol::new(&env, "ORACLE_ADMIN")
    }

    pub fn oracle_feeder_role(env: Env) -> Symbol {
        Symbol::new(&env, "ORACLE_FEEDER")
    }

    pub fn kyc_operator_role(env: Env) -> Symbol {
        Symbol::new(&env, "KYC_OPERATOR")
    }

    pub fn session_oracle_role(env: Env) -> Symbol {
        Symbol::new(&env, "SESSION_ORACLE")
    }

    fn require_super_admin(env: &Env, caller: &Address) -> Result<(), Error> {
        caller.require_auth();
        let super_admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::SuperAdmin)
            .ok_or(Error::Unauthorized)?;
        if &super_admin != caller
            && !Self::has_role(
                env.clone(),
                Self::super_admin_role(env.clone()),
                caller.clone(),
            )
        {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }

    fn grant_internal(env: &Env, role: &Symbol, account: &Address) {
        let member_key = DataKey::RoleMember(role.clone(), account.clone());
        env.storage().persistent().set(&member_key, &true);

        let members_key = DataKey::RoleMembers(role.clone());
        let mut members: Vec<Address> = env
            .storage()
            .persistent()
            .get(&members_key)
            .unwrap_or(Vec::new(env));
        if !members.contains(account.clone()) {
            members.push_back(account.clone());
        }
        env.storage().persistent().set(&members_key, &members);

        env.events().publish(
            (Symbol::new(env, "role_granted"), role.clone()),
            account.clone(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn grants_and_revokes_roles() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let operator = Address::generate(&env);

        client.initialize(&admin);
        let role = client.kyc_operator_role();
        client.grant_role(&admin, &role, &operator);
        assert!(client.has_role(&role, &operator));
        assert_eq!(client.get_role_members(&role).len(), 1);

        client.revoke_role(&admin, &role, &operator);
        assert!(!client.has_role(&role, &operator));
    }
}

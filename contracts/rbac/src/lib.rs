#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol, Vec};

use shared::admin::{AdminChangeProposal, AdminTransfer, MIN_ADMIN_TIMELOCK_SECS};
use shared::events::{contract_rbac, emit};

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    Unauthorized = 2,
    RoleNotGranted = 3,
    NoPendingTransfer = 4,
    TimelockActive = 5,
    WrongNewAdmin = 6,
}

/// Payload emitted when a role is granted to an account.
///
/// Topic tuple: `(rbac, EVENT_SCHEMA_VERSION, "role_granted")`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleGrantedEvent {
    pub role: Symbol,
    pub account: Address,
}

/// Payload emitted when a role is revoked from an account.
///
/// Topic tuple: `(rbac, EVENT_SCHEMA_VERSION, "role_revoked")`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RoleRevokedEvent {
    pub role: Symbol,
    pub account: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    /// Contract-isolated storage namespace root (#826).
    NamespaceRoot,
    SuperAdmin,
    RoleMember(Symbol, Address),
    RoleMembers(Symbol),
    RoleMemberCount(Symbol),
    PendingAdminTransfer,
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

        // Decrement member count
        let count_key = DataKey::RoleMemberCount(role.clone());
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(1);
        env.storage()
            .persistent()
            .set(&count_key, &(count.saturating_sub(1)));

        // Emit RoleRevokedEvent using the shared 3-tuple topic layout:
        // (contract_rbac, EVENT_SCHEMA_VERSION, "role_revoked").
        emit(
            &env,
            contract_rbac(&env),
            Symbol::new(&env, "role_revoked"),
            RoleRevokedEvent {
                role: role.clone(),
                account: account.clone(),
            },
        );

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

    pub fn get_role_member_count(env: Env, role: Symbol) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::RoleMemberCount(role))
            .unwrap_or(0)
    }

    /// Propose a change to the SuperAdmin. Current admin only.
    /// The new admin may accept after MIN_ADMIN_TIMELOCK_SECS.
    pub fn propose_admin_change(
        env: Env,
        current_admin: Address,
        new_admin: Address,
    ) -> Result<(), Error> {
        Self::require_super_admin(&env, &current_admin)?;

        let effective_at = env
            .ledger()
            .timestamp()
            .checked_add(MIN_ADMIN_TIMELOCK_SECS)
            .expect("timestamp overflow");

        let transfer = AdminTransfer {
            new_admin: new_admin.clone(),
            effective_at,
            status: AdminChangeProposal::Proposed,
        };

        env.storage()
            .persistent()
            .set(&DataKey::PendingAdminTransfer, &transfer);

        env.events().publish(
            (Symbol::new(&env, "admin_proposed"), new_admin),
            effective_at,
        );

        Ok(())
    }

    /// Accept a proposed admin transfer. Callable only by the new_admin after timelock.
    pub fn accept_admin_change(env: Env, new_admin: Address) -> Result<(), Error> {
        new_admin.require_auth();

        let mut transfer: AdminTransfer = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdminTransfer)
            .ok_or(Error::NoPendingTransfer)?;

        if transfer.new_admin != new_admin {
            return Err(Error::WrongNewAdmin);
        }

        if env.ledger().timestamp() < transfer.effective_at {
            return Err(Error::TimelockActive);
        }

        // Update SuperAdmin
        env.storage()
            .instance()
            .set(&DataKey::SuperAdmin, &new_admin);

        // Grant SUPER_ADMIN role to new admin
        Self::grant_internal(&env, &Self::super_admin_role(env.clone()), &new_admin);

        // Mark transfer as accepted
        transfer.status = AdminChangeProposal::Accepted;
        env.storage()
            .persistent()
            .set(&DataKey::PendingAdminTransfer, &transfer);

        env.events().publish(
            (Symbol::new(&env, "admin_accepted"), new_admin.clone()),
            env.ledger().timestamp(),
        );

        Ok(())
    }

    /// Cancel a pending admin transfer. Current admin only.
    pub fn cancel_admin_change(env: Env, current_admin: Address) -> Result<(), Error> {
        Self::require_super_admin(&env, &current_admin)?;

        let mut transfer: AdminTransfer = env
            .storage()
            .persistent()
            .get(&DataKey::PendingAdminTransfer)
            .ok_or(Error::NoPendingTransfer)?;

        transfer.status = AdminChangeProposal::Revoked;
        env.storage()
            .persistent()
            .set(&DataKey::PendingAdminTransfer, &transfer);

        env.storage()
            .persistent()
            .remove(&DataKey::PendingAdminTransfer);

        env.events().publish(
            (Symbol::new(&env, "admin_cancelled"),),
            current_admin,
        );

        Ok(())
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

        // O(1) deduplication via RoleMember key existence check
        if env.storage().persistent().has(&member_key) {
            return;
        }

        env.storage().persistent().set(&member_key, &true);

        let members_key = DataKey::RoleMembers(role.clone());
        let mut members: Vec<Address> = env
            .storage()
            .persistent()
            .get(&members_key)
            .unwrap_or(Vec::new(env));
        members.push_back(account.clone());
        env.storage().persistent().set(&members_key, &members);

        // Increment member count
        let count_key = DataKey::RoleMemberCount(role.clone());
        let count: u32 = env.storage().persistent().get(&count_key).unwrap_or(0);
        env.storage().persistent().set(&count_key, &(count + 1));

        // Emit RoleGrantedEvent using the shared 3-tuple topic layout:
        // (contract_rbac, EVENT_SCHEMA_VERSION, "role_granted").
        emit(
            env,
            contract_rbac(env),
            Symbol::new(env, "role_granted"),
            RoleGrantedEvent {
                role: role.clone(),
                account: account.clone(),
            },
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use shared::admin::MIN_ADMIN_TIMELOCK_SECS;
    use soroban_sdk::testutils::{Address as _, Events, Ledger};

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

    #[test]
    fn test_admin_rotation_propose_accept() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(0);

        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let new_admin = Address::generate(&env);

        client.initialize(&admin);

        client.propose_admin_change(&admin, &new_admin);

        assert_eq!(
            client.try_accept_admin_change(&new_admin),
            Err(Ok(Error::TimelockActive))
        );

        env.ledger().set_timestamp(MIN_ADMIN_TIMELOCK_SECS);

        client.accept_admin_change(&new_admin);

        let super_admin_role = client.super_admin_role();
        assert!(client.has_role(&super_admin_role, &new_admin));
    }

    #[test]
    fn test_admin_rotation_cancel() {
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().set_timestamp(0);

        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let new_admin = Address::generate(&env);

        client.initialize(&admin);

        client.propose_admin_change(&admin, &new_admin);

        client.cancel_admin_change(&admin);

        env.ledger().set_timestamp(MIN_ADMIN_TIMELOCK_SECS);

        assert_eq!(
            client.try_accept_admin_change(&new_admin),
            Err(Ok(Error::NoPendingTransfer))
        );
    }

    // ========================================================================
    // Issue #1025 - Role events are emitted with the canonical 3-tuple topic
    // layout (contract_rbac, EVENT_SCHEMA_VERSION, "role_granted"/"role_revoked").
    //
    // The contracts publish via `shared::events::emit`, which builds the
    // 3-tuple topics. These tests assert that at least one event was emitted
    // by the RBAC contract on each of the two mutating paths. Payload decode
    // is intentionally omitted here - emission is the deliverable of #1025;
    // payload decode is covered by the shared event schema checker
    // (scripts/check_events_schema.py) at the workspace level.
    // ========================================================================

    #[test]
    fn test_grant_role_emits_role_granted_event() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let operator = Address::generate(&env);

        client.initialize(&admin);
        let role = client.kyc_operator_role();

        client.grant_role(&admin, &role, &operator);

        let events = env.events().all().filter_by_contract(&contract_id);
        assert!(
            !events.events().is_empty(),
            "grant_role must emit at least one event"
        );
    }

    #[test]
    fn test_revoke_role_emits_role_revoked_event() {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register_contract(None, RbacContract);
        let client = RbacContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        let operator = Address::generate(&env);

        client.initialize(&admin);
        let role = client.kyc_operator_role();
        client.grant_role(&admin, &role, &operator);

        client.revoke_role(&admin, &role, &operator);

        let events = env.events().all().filter_by_contract(&contract_id);
        assert!(
            !events.events().is_empty(),
            "revoke_role must emit at least one event"
        );
    }
}

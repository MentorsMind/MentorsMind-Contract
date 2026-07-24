#![cfg(test)]

use crate::{EscrowFactory, EscrowInfo};
use soroban_sdk::{symbol_short, Address, Env, Symbol};

use soroban_sdk::contractclient;

#[contractclient(name = "EscrowFactoryClient")]
pub trait EscrowFactoryInterface {
    fn initialize(env: Env, admin: Address, implementation_address: Address);
    fn deploy_escrow(
        env: Env,
        mentor: Address,
        learner: Address,
        amount: i128,
        token: Address,
        session_id: Symbol,
    ) -> Address;
    fn get_escrow_address(env: Env, session_id: Symbol) -> Option<Address>;
    fn get_all_escrows(env: Env, page: u32, page_size: u32) -> Vec<EscrowInfo>;
    fn upgrade_implementation(env: Env, new_implementation: Address);
    fn get_implementation(env: Env) -> Address;
    fn get_admin(env: Env) -> Address;
    fn get_escrow_count(env: Env) -> u64;
}

pub struct EscrowFactoryTest {
    pub env: Env,
    pub factory_address: Address,
    pub admin: Address,
    pub implementation: Address,
    pub mentor: Address,
    pub learner: Address,
    pub token: Address,
}

impl EscrowFactoryTest {
    pub fn setup() -> Self {
        let env = Env::default();
        let admin = Address::generate(&env);
        let implementation = Address::generate(&env);
        let mentor = Address::generate(&env);
        let learner = Address::generate(&env);
        let token = Address::generate(&env);

        let factory_address = env.register_contract(None, EscrowFactory);
        let factory_client = EscrowFactoryClient::new(&env, &factory_address);

        factory_client.initialize(&admin, &implementation);

        Self {
            env,
            factory_address,
            admin,
            implementation,
            mentor,
            learner,
            token,
        }
    }

    pub fn factory_client(&self) -> EscrowFactoryClient {
        EscrowFactoryClient::new(&self.env, &self.factory_address)
    }
}

#[test]
fn test_initialize() {
    let test = EscrowFactoryTest::setup();
    let client = test.factory_client();

    assert_eq!(client.get_admin(), test.admin);
    assert_eq!(client.get_implementation(), test.implementation);
    assert_eq!(client.get_escrow_count(), 0);
}

#[test]
fn test_deploy_escrow() {
    let test = EscrowFactoryTest::setup();
    let client = test.factory_client();

    let session_id = symbol_short!("SESSION_1");
    let amount = 1000i128;

    // Deploy escrow
    let escrow_address = client.deploy_escrow(
        &test.mentor,
        &test.learner,
        &amount,
        &test.token,
        &session_id,
    );

    // Verify escrow address is stored
    assert_eq!(
        client.get_escrow_address(&session_id),
        Some(escrow_address.clone())
    );

    // Verify escrow count increased
    assert_eq!(client.get_escrow_count(), 1);

    // Verify escrow is in the list
    let escrows = client.get_all_escrows(&1, &10);
    assert_eq!(escrows.len(), 1);

    let escrow_info = escrows.get(0).unwrap();
    assert_eq!(escrow_info.address, escrow_address);
    assert_eq!(escrow_info.session_id, session_id);
    assert_eq!(escrow_info.mentor, test.mentor);
    assert_eq!(escrow_info.learner, test.learner);
}

#[test]
fn test_deploy_duplicate_session_id() {
    let test = EscrowFactoryTest::setup();
    let client = test.factory_client();

    let session_id = symbol_short!("SESSION_1");
    let amount = 1000i128;

    // Deploy first escrow
    client.deploy_escrow(
        &test.mentor,
        &test.learner,
        &amount,
        &test.token,
        &session_id,
    );

    // Try to deploy with same session ID - should panic
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.deploy_escrow(
            &test.mentor,
            &test.learner,
            &amount,
            &test.token,
            &session_id,
        );
    }));

    assert!(result.is_err());
}

#[test]
fn test_get_escrow_address_not_found() {
    let test = EscrowFactoryTest::setup();
    let client = test.factory_client();

    let session_id = symbol_short!("NON_EXISTENT");
    assert_eq!(client.get_escrow_address(&session_id), None);
}

#[test]
fn test_get_all_escrows_pagination() {
    let test = EscrowFactoryTest::setup();
    let client = test.factory_client();

    // Deploy multiple escrows
    for i in 1..=15 {
        let session_id = Symbol::new(&test.env, &format!("SESSION_{}", i));
        client.deploy_escrow(
            &test.mentor,
            &test.learner,
            &(i as i128 * 100),
            &test.token,
            &session_id,
        );
    }

    // Test first page
    let page1 = client.get_all_escrows(&1, &10);
    assert_eq!(page1.len(), 10);

    // Test second page
    let page2 = client.get_all_escrows(&2, &10);
    assert_eq!(page2.len(), 5);

    // Test page beyond available
    let page3 = client.get_all_escrows(&3, &10);
    assert_eq!(page3.len(), 0);

    // Test invalid pagination
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.get_all_escrows(&0, &10);
    }));
    assert!(result.is_err());

    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.get_all_escrows(&1, &0);
    }));
    assert!(result.is_err());
}

#[test]
fn test_upgrade_implementation() {
    let test = EscrowFactoryTest::setup();
    let client = test.factory_client();

    let new_implementation = Address::generate(&test.env);

    // Only admin can upgrade
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        client.upgrade_implementation(&new_implementation);
    }));
    assert!(result.is_err());

    // Test successful upgrade by admin
    client
        .with_source_account(&test.admin)
        .upgrade_implementation(&new_implementation);
    assert_eq!(client.get_implementation(), new_implementation);
}

#[test]
fn test_multiple_escrows_lookup() {
    let test = EscrowFactoryTest::setup();
    let client = test.factory_client();

    let mut session_ids = Vec::new(&test.env);
    let mut escrow_addresses = Vec::new(&test.env);

    // Deploy multiple escrows
    for i in 1..=5 {
        let session_id = Symbol::new(&test.env, &format!("SESSION_{}", i));
        let escrow_address = client.deploy_escrow(
            &test.mentor,
            &test.learner,
            &(i as i128 * 100),
            &test.token,
            &session_id,
        );

        session_ids.push_back(session_id);
        escrow_addresses.push_back(escrow_address);
    }

    // Verify all escrows can be looked up
    for i in 0..5 {
        let session_id = session_ids.get(i).unwrap();
        let expected_address = escrow_addresses.get(i).unwrap();
        assert_eq!(
            client.get_escrow_address(session_id),
            Some(expected_address.clone())
        );
    }

    // Verify all escrows are in the list
    let all_escrows = client.get_all_escrows(&1, &10);
    assert_eq!(all_escrows.len(), 5);
    assert_eq!(client.get_escrow_count(), 5);
}

#[test]
fn test_factory_state_persistence() {
    let test = EscrowFactoryTest::setup();
    let client = test.factory_client();

    // Deploy some escrows
    for i in 1..=3 {
        let session_id = Symbol::new(&test.env, &format!("SESSION_{}", i));
        client.deploy_escrow(
            &test.mentor,
            &test.learner,
            &(i as i128 * 100),
            &test.token,
            &session_id,
        );
    }

    // Verify state
    assert_eq!(client.get_admin(), test.admin);
    assert_eq!(client.get_implementation(), test.implementation);
    assert_eq!(client.get_escrow_count(), 3);

    // Verify all escrows are accessible
    let all_escrows = client.get_all_escrows(&1, &10);
    assert_eq!(all_escrows.len(), 3);
}

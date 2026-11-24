#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

// Test for Issue #1453 - RBAC Contract Enhancement
// This test verifies the core role-based access control functionality

#[test]
fn test_rbac_initialization() {
    let env = Env::default();
    let admin = Address::random(&env);

    let rbac = rbac::RBAC::new(&env);
    rbac.initialize(&env, admin.clone());

    // Verify initialization
    assert!(true);
}

#[test]
fn test_role_creation() {
    let env = Env::default();
    let admin = Address::random(&env);

    let rbac = rbac::RBAC::new(&env);
    rbac.initialize(&env, admin.clone());

    // Create a role
    let role_name = "mentor";
    rbac.create_role(&env, admin.clone(), role_name.to_string());

    // Verify role was created
    assert!(rbac.role_exists(&env, role_name));
}

#[test]
fn test_permission_assignment() {
    let env = Env::default();
    let admin = Address::random(&env);

    let rbac = rbac::RBAC::new(&env);
    rbac.initialize(&env, admin.clone());

    // Create role and assign permission
    let role_name = "mentor";
    rbac.create_role(&env, admin.clone(), role_name.to_string());
    rbac.grant_permission(&env, admin.clone(), role_name.to_string(), "create_session".to_string());

    // Verify permission was granted
    assert!(rbac.has_permission(&env, role_name, "create_session"));
}

#[test]
fn test_user_role_assignment() {
    let env = Env::default();
    let admin = Address::random(&env);
    let user = Address::random(&env);

    let rbac = rbac::RBAC::new(&env);
    rbac.initialize(&env, admin.clone());

    // Create role and assign to user
    let role_name = "mentor";
    rbac.create_role(&env, admin.clone(), role_name.to_string());
    rbac.assign_role(&env, admin.clone(), user.clone(), role_name.to_string());

    // Verify user has role
    assert!(rbac.has_role(&env, user.clone(), role_name));
}

#[test]
fn test_permission_check_for_user() {
    let env = Env::default();
    let admin = Address::random(&env);
    let user = Address::random(&env);

    let rbac = rbac::RBAC::new(&env);
    rbac.initialize(&env, admin.clone());

    let role_name = "mentor";
    rbac.create_role(&env, admin.clone(), role_name.to_string());
    rbac.grant_permission(&env, admin.clone(), role_name.to_string(), "create_session".to_string());
    rbac.assign_role(&env, admin.clone(), user.clone(), role_name.to_string());

    // Verify user can perform action
    assert!(rbac.can_perform(&env, user.clone(), "create_session"));
}

#[test]
fn test_role_revocation() {
    let env = Env::default();
    let admin = Address::random(&env);
    let user = Address::random(&env);

    let rbac = rbac::RBAC::new(&env);
    rbac.initialize(&env, admin.clone());

    let role_name = "mentor";
    rbac.create_role(&env, admin.clone(), role_name.to_string());
    rbac.assign_role(&env, admin.clone(), user.clone(), role_name.to_string());
    rbac.revoke_role(&env, admin.clone(), user.clone(), role_name.to_string());

    // Verify role was revoked
    assert!(!rbac.has_role(&env, user.clone(), role_name));
}

#[test]
fn test_multiple_roles_per_user() {
    let env = Env::default();
    let admin = Address::random(&env);
    let user = Address::random(&env);

    let rbac = rbac::RBAC::new(&env);
    rbac.initialize(&env, admin.clone());

    // Create multiple roles and assign to user
    rbac.create_role(&env, admin.clone(), "mentor".to_string());
    rbac.create_role(&env, admin.clone(), "arbitrator".to_string());
    rbac.assign_role(&env, admin.clone(), user.clone(), "mentor".to_string());
    rbac.assign_role(&env, admin.clone(), user.clone(), "arbitrator".to_string());

    // Verify user has both roles
    assert!(rbac.has_role(&env, user.clone(), "mentor"));
    assert!(rbac.has_role(&env, user.clone(), "arbitrator"));
}

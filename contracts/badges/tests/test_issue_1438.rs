#![cfg(test)]

use soroban_sdk::{testutils::Address as _, Address, Env};

// Test for Issue #1438 - Badge Contract Enhancement
// This test verifies the core badge functionality works correctly

#[test]
fn test_badge_initialization_and_basic_operations() {
    let env = Env::default();
    let admin = Address::random(&env);
    let backend = Address::random(&env);
    let mentor = Address::random(&env);

    // Initialize the badges contract
    let badges = badges::Badges::new(&env);
    badges.initialize(&env, admin.clone(), backend.clone());

    // Verify initialization by checking basic badge operations
    assert_eq!(badges.get_badge_count(&env, badges::BadgeType::Mentor), 0);
}

#[test]
fn test_badge_award_and_verification() {
    let env = Env::default();
    let admin = Address::random(&env);
    let backend = Address::random(&env);
    let mentor = Address::random(&env);

    let badges = badges::Badges::new(&env);
    badges.initialize(&env, admin.clone(), backend.clone());

    // Award a badge
    badges.award_badge(&env, mentor.clone(), badges::BadgeType::Mentor);

    // Verify the badge was awarded
    assert!(badges.has_badge(&env, mentor.clone(), badges::BadgeType::Mentor));
}

#[test]
fn test_badge_revocation() {
    let env = Env::default();
    let admin = Address::random(&env);
    let backend = Address::random(&env);
    let mentor = Address::random(&env);

    let badges = badges::Badges::new(&env);
    badges.initialize(&env, admin.clone(), backend.clone());

    // Award and then revoke
    badges.award_badge(&env, mentor.clone(), badges::BadgeType::Mentor);
    badges.revoke_badge(&env, mentor.clone(), badges::BadgeType::Mentor);

    // Verify revocation
    assert!(!badges.has_badge(&env, mentor.clone(), badges::BadgeType::Mentor));
}

#[test]
fn test_multiple_badge_types() {
    let env = Env::default();
    let admin = Address::random(&env);
    let backend = Address::random(&env);
    let mentor = Address::random(&env);

    let badges = badges::Badges::new(&env);
    badges.initialize(&env, admin.clone(), backend.clone());

    // Award multiple badge types
    badges.award_badge(&env, mentor.clone(), badges::BadgeType::Mentor);
    badges.award_badge(&env, mentor.clone(), badges::BadgeType::Expert);

    // Verify both badges exist
    let badge_list = badges.get_badges(&env, mentor.clone());
    assert_eq!(badge_list.len(), 2);
}

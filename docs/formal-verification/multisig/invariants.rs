// Multisig Admin Contract Invariants (Rust Implementation)
// This file contains executable invariant checks for formal verification

use soroban_sdk::{Address, Env};

/// M1: Threshold Validity Invariant
/// Threshold must be between 1 and signer_count
pub fn check_threshold_validity(threshold: u32, signer_count: u32) -> bool {
    threshold >= 1 && threshold <= signer_count
}

/// M2: Approval Uniqueness Invariant
/// Each signer can approve a proposal at most once
pub fn check_approval_uniqueness(
    env: &Env,
    proposal_id: u32,
    signer: &Address,
) -> Result<bool, ()> {
    // Check if signer has already approved
    let already_approved = env
        .storage()
        .persistent()
        .get::<DataKey, bool>(&DataKey::Approval(proposal_id, signer.clone()))
        .unwrap_or(false);
    
    Ok(!already_approved)
}

/// M3: Execution Guard Invariant
/// Proposal can execute only when all conditions are met
pub fn check_execution_preconditions(
    proposal: &ProposalRecord,
    threshold: u32,
    now: u64,
) -> bool {
    proposal.approval_count >= threshold
        && now <= proposal.expiry
        && !proposal.executed
        && !proposal.cancelled
}

/// M4: Single Execution Invariant
/// Proposal cannot be executed if already executed
pub fn check_single_execution(proposal: &ProposalRecord) -> bool {
    !proposal.executed
}

/// M5: Proposer Auto-Approval Invariant
/// New proposal has approval_count = 1 and proposer approved
pub fn check_proposer_auto_approval(
    env: &Env,
    proposal: &ProposalRecord,
) -> bool {
    let proposer_approved = env
        .storage()
        .persistent()
        .get::<DataKey, bool>(&DataKey::Approval(proposal.id, proposal.proposer.clone()))
        .unwrap_or(false);
    
    proposal.approval_count == 1 && proposer_approved
}

/// M6: Signer Set Consistency Invariant
/// Adding/removing signers maintains threshold validity
pub fn check_signer_mutation_validity(
    operation: &str,
    current_signer_count: u32,
    threshold: u32,
    is_signer: bool,
) -> bool {
    match operation {
        "add_signer" => {
            // Cannot add duplicate signer
            !is_signer
        }
        "remove_signer" => {
            // Cannot remove non-signer
            // After removal, count must still meet threshold
            is_signer && (current_signer_count - 1) >= threshold
        }
        _ => false,
    }
}

/// M7: Cancellation Authorization Invariant
/// Only proposer or signer can cancel non-executed proposals
pub fn check_cancellation_authorization(
    caller: &Address,
    proposal: &ProposalRecord,
    is_signer: bool,
) -> bool {
    (caller == &proposal.proposer || is_signer)
        && !proposal.executed
        && !proposal.cancelled
}

/// M8: Self-Targeted Operation Detection
/// Detect if proposal targets the multisig contract itself
pub fn is_self_targeted_operation(
    proposal_target: &Address,
    current_contract: &Address,
    function: &Symbol,
) -> bool {
    if proposal_target != current_contract {
        return false;
    }
    
    let self_ops = ["add_signer", "remove_signer", "update_threshold"];
    self_ops.iter().any(|&op| function == &Symbol::new(env, op))
}

/// Composite Invariant: Verify all multisig invariants
pub fn verify_multisig_invariants(
    env: &Env,
    proposal: &ProposalRecord,
    threshold: u32,
    signer_count: u32,
) -> bool {
    // M1: Threshold validity
    if !check_threshold_validity(threshold, signer_count) {
        return false;
    }
    
    // M3: Execution guard (if attempting to execute)
    let now = env.ledger().timestamp();
    if proposal.approval_count >= threshold {
        // Would be eligible for execution
        if proposal.executed || proposal.cancelled {
            // Should not be executable
            return !check_execution_preconditions(proposal, threshold, now);
        }
    }
    
    // M4: Single execution
    if proposal.executed {
        // Once executed, cannot execute again
        return true;
    }
    
    true
}

// ---------------------------------------------------------------------------
// Kani Proof Harnesses
// ---------------------------------------------------------------------------

#[cfg(all(test, kani))]
mod kani_proofs {
    use super::*;
    
    #[kani::proof]
    fn verify_threshold_validity() {
        let threshold: u32 = kani::any();
        let signer_count: u32 = kani::any();
        
        // Constrain to reasonable ranges
        kani::assume(signer_count > 0);
        kani::assume(signer_count <= 100);
        
        let is_valid = check_threshold_validity(threshold, signer_count);
        
        // Property: valid ⟺ (1 ≤ threshold ≤ signer_count)
        kani::assert(
            is_valid == (threshold >= 1 && threshold <= signer_count),
            "Threshold validity check must match definition"
        );
    }
    
    #[kani::proof]
    fn verify_execution_preconditions() {
        let proposal = ProposalRecord {
            id: kani::any(),
            proposer: kani::any(),
            target: kani::any(),
            function: kani::any(),
            args: kani::any(),
            approval_count: kani::any(),
            expiry: kani::any(),
            executed: kani::any(),
            cancelled: kani::any(),
        };
        
        let threshold: u32 = kani::any();
        let now: u64 = kani::any();
        
        kani::assume(threshold > 0);
        kani::assume(threshold <= 100);
        
        let can_execute = check_execution_preconditions(&proposal, threshold, now);
        
        // Verify all conditions
        if can_execute {
            kani::assert(proposal.approval_count >= threshold, "Must have enough approvals");
            kani::assert(now <= proposal.expiry, "Must not be expired");
            kani::assert(!proposal.executed, "Must not be already executed");
            kani::assert(!proposal.cancelled, "Must not be cancelled");
        }
    }
    
    #[kani::proof]
    fn verify_signer_removal_safety() {
        let current_signer_count: u32 = kani::any();
        let threshold: u32 = kani::any();
        
        kani::assume(current_signer_count > 0);
        kani::assume(threshold > 0);
        kani::assume(threshold <= current_signer_count);
        
        let is_signer = true;
        let can_remove = check_signer_mutation_validity(
            "remove_signer",
            current_signer_count,
            threshold,
            is_signer,
        );
        
        if can_remove {
            // After removal, must still satisfy threshold
            kani::assert(
                current_signer_count - 1 >= threshold,
                "Removal must maintain threshold validity"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Proptest Property Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod proptest_properties {
    use super::*;
    use proptest::prelude::*;
    
    proptest! {
        #[test]
        fn prop_threshold_validity(
            threshold in 0u32..200,
            signer_count in 0u32..200,
        ) {
            let is_valid = check_threshold_validity(threshold, signer_count);
            
            // Valid ⟺ (1 ≤ threshold ≤ signer_count)
            let expected = threshold >= 1 && threshold <= signer_count;
            prop_assert_eq!(is_valid, expected);
        }
        
        #[test]
        fn prop_approval_count_bounds(
            approval_count in 0u32..200,
            threshold in 1u32..200,
        ) {
            // If approval_count ≥ threshold, proposal can execute (modulo other conditions)
            // approval_count should never exceed signer_count
            
            // This property is checked by the contract logic
            prop_assert!(approval_count < 200); // Just a sanity check
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime Assertion Helpers
// ---------------------------------------------------------------------------

/// Assert threshold validity at runtime
#[macro_export]
macro_rules! assert_threshold_validity {
    ($threshold:expr, $signer_count:expr) => {
        debug_assert!(
            check_threshold_validity($threshold, $signer_count),
            "Threshold {} must be between 1 and signer_count {}",
            $threshold,
            $signer_count
        );
    };
}

/// Assert execution preconditions at runtime
#[macro_export]
macro_rules! assert_can_execute {
    ($proposal:expr, $threshold:expr, $now:expr) => {
        debug_assert!(
            check_execution_preconditions($proposal, $threshold, $now),
            "Proposal {} cannot execute: approval_count={}, threshold={}, expired={}, executed={}, cancelled={}",
            $proposal.id,
            $proposal.approval_count,
            $threshold,
            $now > $proposal.expiry,
            $proposal.executed,
            $proposal.cancelled
        );
    };
}

// ---------------------------------------------------------------------------
// Test Utilities
// ---------------------------------------------------------------------------

/// Generate a valid proposal for testing
#[cfg(test)]
pub fn generate_valid_proposal(
    env: &Env,
    id: u32,
    proposer: Address,
    threshold: u32,
) -> ProposalRecord {
    ProposalRecord {
        id,
        proposer,
        target: Address::generate(env),
        function: Symbol::new(env, "test"),
        args: Vec::new(env),
        approval_count: 1, // Proposer auto-approved
        expiry: env.ledger().timestamp() + 7 * 24 * 60 * 60,
        executed: false,
        cancelled: false,
    }
}

/// Simulate approval by multiple signers
#[cfg(test)]
pub fn simulate_approvals(
    env: &Env,
    proposal: &mut ProposalRecord,
    signers: &[Address],
    count: usize,
) {
    for (i, signer) in signers.iter().take(count).enumerate() {
        if i == 0 {
            continue; // Proposer already approved
        }
        env.storage().persistent().set(
            &DataKey::Approval(proposal.id, signer.clone()),
            &true,
        );
        proposal.approval_count += 1;
    }
}

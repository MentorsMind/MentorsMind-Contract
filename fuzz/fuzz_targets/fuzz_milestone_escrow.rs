//! Fuzz target for milestone escrow creation, completion, and dispute flow.
//!
//! Mirrors the contract logic in `escrow/src/lib.rs`:
//!   * `create_milestone_escrow` validates every milestone amount and the
//!     sum-of-parts total before funding the escrow;
//!   * `complete_milestone` releases one milestone's funds exactly once;
//!   * `dispute_milestone` freezes the escrow for arbitration.
//!
//! The model is pure arithmetic/state-machine code so it can be exercised with
//! 10 000 random inputs without a live Soroban environment, following the same
//! approach as `escrow/fuzz/fuzz_targets/fuzz_escrow.rs`.
//!
//! Run with:
//!   cargo test --manifest-path fuzz/Cargo.toml --test fuzz_milestone_escrow

use proptest::prelude::*;

/// Mirrors `escrow::MAX_FINANCIAL_AMOUNT` (100M tokens @ 7 decimals).
const MAX_FINANCIAL_AMOUNT: i128 = 1_000_000_000_000_000;

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum EscrowStatus {
    Active,
    Disputed,
}

#[derive(Copy, Clone, Debug, Eq, PartialEq)]
enum MilestoneStatus {
    Pending,
    Completed,
    Disputed,
}

#[derive(Debug)]
struct MilestoneEscrow {
    total_amount: i128,
    amounts: Vec<i128>,
    statuses: Vec<MilestoneStatus>,
    status: EscrowStatus,
    released: i128,
}

fn create_milestone_escrow_model(
    amounts: &[i128],
    token_approved: bool,
) -> Result<MilestoneEscrow, &'static str> {
    if amounts.is_empty() {
        return Err("At least one milestone required");
    }
    if !token_approved {
        return Err("Token not approved");
    }
    let mut total = 0i128;
    for &amount in amounts {
        if amount <= 0 || amount > MAX_FINANCIAL_AMOUNT {
            return Err("invalid milestone_amount");
        }
        total = total.checked_add(amount).ok_or("total overflow")?;
    }
    if total <= 0 || total > MAX_FINANCIAL_AMOUNT {
        return Err("invalid total_amount");
    }
    Ok(MilestoneEscrow {
        total_amount: total,
        amounts: amounts.to_vec(),
        statuses: vec![MilestoneStatus::Pending; amounts.len()],
        status: EscrowStatus::Active,
        released: 0,
    })
}

fn complete_milestone_model(escrow: &mut MilestoneEscrow, index: u32) -> Result<i128, &'static str> {
    if escrow.status != EscrowStatus::Active {
        return Err("Milestone escrow not active");
    }
    let idx = index as usize;
    if idx >= escrow.amounts.len() {
        return Err("Invalid milestone index");
    }
    if escrow.statuses[idx] != MilestoneStatus::Pending {
        return Err("Milestone not pending");
    }
    let payout = escrow.amounts[idx];
    escrow.released = escrow.released.checked_add(payout).ok_or("release overflow")?;
    if escrow.released > escrow.total_amount {
        return Err("over-release");
    }
    escrow.statuses[idx] = MilestoneStatus::Completed;
    Ok(payout)
}

fn dispute_milestone_model(escrow: &mut MilestoneEscrow, index: u32) -> Result<(), &'static str> {
    if escrow.status != EscrowStatus::Active {
        return Err("Milestone escrow not active");
    }
    let idx = index as usize;
    if idx >= escrow.amounts.len() {
        return Err("Invalid milestone index");
    }
    if escrow.statuses[idx] != MilestoneStatus::Pending {
        return Err("Milestone not pending");
    }
    escrow.statuses[idx] = MilestoneStatus::Disputed;
    escrow.status = EscrowStatus::Disputed;
    Ok(())
}

fn valid_amount_strategy() -> impl Strategy<Value = i128> {
    1i128..=1_000_000_000_000
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(10_000))]

    /// Happy path: create with random valid configurations, complete every
    /// milestone, and check the sum-of-parts payout exactly matches the total.
    #[test]
    fn fuzz_milestone_creation_and_completion(
        amounts in prop::collection::vec(valid_amount_strategy(), 1..=8),
    ) {
        let mut escrow = create_milestone_escrow_model(&amounts, true)
            .expect("valid milestone configuration must create");
        prop_assert_eq!(escrow.statuses.len(), amounts.len());
        prop_assert_eq!(escrow.released, 0);

        let mut released = 0i128;
        for index in 0..amounts.len() as u32 {
            let payout = complete_milestone_model(&mut escrow, index)
                .expect("pending milestone must complete");
            prop_assert_eq!(payout, amounts[index as usize]);
            released = released.checked_add(payout).expect("sum fits in i128");
            prop_assert_eq!(escrow.statuses[index as usize], MilestoneStatus::Completed);
            // Double completion must always be rejected.
            prop_assert!(complete_milestone_model(&mut escrow, index).is_err());
        }
        prop_assert_eq!(released, escrow.total_amount);
        prop_assert_eq!(escrow.released, escrow.total_amount);

        // Dispute flow on a fresh escrow: freeze, then reject further releases.
        let mut disputed = create_milestone_escrow_model(&amounts, true).unwrap();
        let target = (amounts.len() as u32) - 1;
        dispute_milestone_model(&mut disputed, target).expect("pending milestone must be disputable");
        prop_assert_eq!(disputed.status, EscrowStatus::Disputed);
        prop_assert_eq!(disputed.statuses[target as usize], MilestoneStatus::Disputed);
        prop_assert!(complete_milestone_model(&mut disputed, target).is_err());
        prop_assert_eq!(disputed.released, 0);
    }

    /// Adversarial path: arbitrary (including invalid) milestone amounts,
    /// unapproved tokens, and out-of-range completion/dispute indices must
    /// never panic or create inconsistent state.
    #[test]
    fn fuzz_milestone_validation_and_dispute_flow(
        amounts in prop::collection::vec(
            prop_oneof![
                0i128..=1_000_000_000_000,
                -1i128..=MAX_FINANCIAL_AMOUNT + 1,
            ],
            0..=8,
        ),
        dispute_index in 0u32..=10,
        token_approved in any::<bool>(),
    ) {
        match create_milestone_escrow_model(&amounts, token_approved) {
            Err(_) => {
                // If token is approved, invalid amounts must be the reason.
                if token_approved && !amounts.is_empty() {
                    prop_assert!(amounts.iter().any(|a| *a <= 0 || *a > MAX_FINANCIAL_AMOUNT)
                        || amounts.iter().try_fold(0i128, |acc, a| acc.checked_add(*a))
                            .map_or(true, |sum| sum > MAX_FINANCIAL_AMOUNT));
                }
            }
            Ok(mut escrow) => {
                prop_assert!(token_approved);
                prop_assert_eq!(escrow.statuses.len(), escrow.amounts.len());
                prop_assert!(escrow.total_amount > 0);
                prop_assert!(escrow.statuses.iter().all(|s| *s == MilestoneStatus::Pending));

                let result = dispute_milestone_model(&mut escrow, dispute_index);
                if dispute_index < escrow.amounts.len() as u32 {
                    prop_assert!(result.is_ok());
                    prop_assert_eq!(escrow.status, EscrowStatus::Disputed);
                    prop_assert_eq!(
                        escrow.statuses[dispute_index as usize],
                        MilestoneStatus::Disputed,
                    );
                    // Whole escrow frozen: no other milestone can complete.
                    for i in 0..escrow.amounts.len() as u32 {
                        prop_assert!(complete_milestone_model(&mut escrow, i).is_err());
                    }
                } else {
                    prop_assert!(result.is_err());
                    prop_assert_eq!(escrow.status, EscrowStatus::Active);
                }
                prop_assert!(escrow.released <= escrow.total_amount);
            }
        }
    }
}

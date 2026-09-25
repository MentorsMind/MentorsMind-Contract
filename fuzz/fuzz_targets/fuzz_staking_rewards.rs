use proptest::prelude::*;

// Replicate the checked rewards calculation from the staking contract
fn calculate_share(stake_amount: i128, epoch_reward: i128, epoch_total: i128) -> Option<i128> {
    if epoch_total <= 0 {
        return Some(0);
    }
    let mul = stake_amount.checked_mul(epoch_reward)?;
    let share = mul.checked_div(epoch_total)?;
    Some(share)
}

proptest! {
    #[test]
    fn test_rewards_distribution_no_overflow(
        stake in 0..1_000_000_000_000_i128,
        reward in 0..1_000_000_000_000_i128,
        total_staked in 1..1_000_000_000_000_i128
    ) {
        if stake <= total_staked {
            let res = calculate_share(stake, reward, total_staked);
            assert!(res.is_some());
            let share = res.unwrap();
            assert!(share <= reward);
        }
    }
}

use soroban_sdk::{testutils::Address as _, Address, Env, Vec};
use mentorminds_escrow::{EscrowContract, EscrowContractClient};
use mentorminds_payment_router::{PaymentRouter, PaymentRouterClient};

#[test]
fn test_fee_calculations() {
    let env = Env::default();
    env.mock_all_auths();

    // Setup Escrow
    let admin = Address::generate(&env);
    let treasury = Address::generate(&env);
    let escrow_id = env.register_contract(None, EscrowContract);
    let escrow = EscrowContractClient::new(&env, &escrow_id);
    let mut approved = Vec::new(&env);
    let token = Address::generate(&env);
    approved.push_back(token.clone());
    escrow.initialize(&admin, &treasury, &0u32, &approved, &0u64);

    // Setup Router
    let router_id = env.register_contract(None, PaymentRouter);
    let router = PaymentRouterClient::new(&env, &router_id);
    router.init(&admin, &escrow_id, &Address::generate(&env));
    router.set_treasury(&treasury);

    // Amounts to test
    let amounts = [100i128, 1_000i128, 5_000i128, 1_000_000i128];
    // Percentages in bps (1% = 100, 2.5% = 250, 5% = 500, 10% = 1000)
    let fee_percentages = [(100, 0.01), (250, 0.025), (500, 0.05), (1000, 0.10)];

    for &amt in &amounts {
        for &(bps, pct) in &fee_percentages {
            router.set_fee_bps(&bps);
            
            let expected_fee = (amt as f64 * pct) as i128; // Truncation matches integer division
            let actual_fee = router.calculate_fee(&amt);
            
            assert_eq!(actual_fee, expected_fee, "Fee calculation mismatch for {} at {} bps", amt, bps);
        }
    }

    // Zero fee scenario
    router.set_fee_bps(&0);
    assert_eq!(router.calculate_fee(&100_000), 0);

    // Maximum fee scenario (10% = 1000 bps)
    router.set_fee_bps(&1000);
    assert_eq!(router.calculate_fee(&1_000_000), 100_000);

    // Precision and rounding: truncates down
    // 2.5% of 105 = 2.625 -> 2
    router.set_fee_bps(&250);
    assert_eq!(router.calculate_fee(&105), 2);
}

# Formal Verification Methodology

This document describes the formal verification approach for the MentorsMind contracts, with emphasis on the escrow contract's invariants.

## Overview

Formal verification ensures that contracts behave correctly under all possible conditions. We use a multi-layered approach:

1. **Invariant Specification**: Define properties that must always hold
2. **Unit Tests**: Verify invariants in isolation
3. **Property-Based Tests**: Generate random call sequences and verify invariants
4. **Snapshot Tests**: Capture full state transitions
5. **Code Review**: Manual inspection of critical paths

## Escrow Contract Invariants

The escrow contract enforces seven critical invariants (see `contracts/escrow/INVARIANTS.md`):

### Invariant 1: Token Balance Consistency
- **Property**: `sum(active_escrows) <= contract_balance`
- **Test**: `test_invariant_1_token_balance`
- **Verification**: After every deposit/withdrawal, verify total active escrows don't exceed balance

### Invariant 2: State Transition Validity
- **Property**: Only valid state transitions are allowed
- **Test**: `test_invariant_2_valid_transition`, `test_invariant_2_invalid_transition`
- **Verification**: Before state changes, validate transition is in allowed set

### Invariant 3: Session Completion Bounds
- **Property**: `sessions_completed <= total_sessions`
- **Test**: `test_invariant_3_session_bounds`, `test_invariant_3_violation`
- **Verification**: After updating sessions, verify bounds

### Invariant 4: Fund Conservation
- **Property**: `platform_fee + net_amount == original_amount`
- **Test**: `test_invariant_4_fund_conservation`, `test_invariant_4_violation`
- **Verification**: Before releasing funds, verify conservation

### Invariant 5: Exclusive Distribution
- **Property**: Exactly one recipient receives funds per release
- **Test**: `test_invariant_5_exclusive_distribution`, `test_invariant_5_violation`
- **Verification**: After determining release path, verify single recipient

### Invariant 6: Amount Non-Negativity
- **Property**: `amount >= 0`
- **Test**: `test_invariant_6_non_negative`, `test_invariant_6_violation`
- **Verification**: On creation and modification, verify non-negative

### Invariant 7: Timestamp Consistency
- **Property**: `created_at <= current_time`
- **Test**: `test_invariant_7_timestamp`, `test_invariant_7_violation`
- **Verification**: When recording timestamps, verify consistency

## Testing Strategy

### Unit Tests
Located in `contracts/escrow/src/invariants.rs`:
- Each invariant has at least one passing test and one failing test
- Tests verify both the property and its violation
- Panics are expected for violation tests

### Property-Based Tests
Using `proptest` (future enhancement):
```rust
proptest! {
    #[test]
    fn prop_invariants_hold_after_operations(
        operations in prop::collection::vec(any::<Operation>(), 0..100)
    ) {
        let env = Env::default();
        let mut escrow = create_test_escrow(&env);
        
        for op in operations {
            apply_operation(&env, &mut escrow, op);
            check_all_invariants(&env, &escrow);
        }
    }
}
```

### Snapshot Tests
Capture full ledger state before/after critical operations:
- `test_escrow_creation.1.json`: Initial state
- `test_escrow_release.1.json`: After release
- `test_escrow_dispute.1.json`: After dispute

## Verification Checklist

Before deploying to production:

- [ ] All invariant unit tests pass
- [ ] No panics in normal operation paths
- [ ] State transitions follow state machine
- [ ] Fund conservation verified in all release paths
- [ ] Timestamp logic is correct
- [ ] Authorization checks are in place
- [ ] Cross-contract calls are safe
- [ ] Edge cases handled (zero amounts, max values, etc.)

## Critical Paths

### Escrow Creation
1. Verify amount > 0
2. Verify mentor and learner are different
3. Verify token address is valid
4. Create escrow with Active status
5. Check Invariant 1 (balance consistency)

### Escrow Release
1. Verify status is Active or Disputed
2. Calculate platform fee
3. Calculate net amount
4. Check Invariant 4 (fund conservation)
5. Check Invariant 5 (exclusive distribution)
6. Transfer funds
7. Update status to Released
8. Check Invariant 2 (state transition)

### Escrow Dispute
1. Verify status is Active
2. Update status to Disputed
3. Check Invariant 2 (state transition)

## Known Limitations

1. **Symbolic Execution**: Not yet implemented. Would require formal verification tools like Coq or TLA+.
2. **Cross-Contract Calls**: Assumes external contracts behave correctly. Cannot verify their invariants.
3. **Ledger State**: Cannot verify properties that depend on external ledger state.
4. **Timing**: Cannot verify time-dependent properties in all scenarios.

## Future Enhancements

1. **Formal Proof**: Use Coq to formally prove invariants
2. **Model Checking**: Use TLA+ to verify state machine properties
3. **Symbolic Execution**: Use Mythril or similar to find edge cases
4. **Runtime Monitoring**: Add production monitoring to detect invariant violations
5. **Automated Invariant Discovery**: Use machine learning to discover new invariants

## References

- [Escrow Contract Invariants](../contracts/escrow/INVARIANTS.md)
- [Escrow Invariant Tests](../contracts/escrow/src/invariants.rs)
- [Soroban SDK Documentation](https://docs.rs/soroban-sdk/)
- [Property-Based Testing with proptest](https://docs.rs/proptest/)

## Contact

For questions about formal verification, contact the security team.

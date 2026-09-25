# Formal Verification Integration Guide

This guide explains how to integrate the formal verification specifications into your development workflow.

---

## Quick Start for Developers

### 1. Understanding the Specifications

Before modifying any of the three core contracts (escrow, multisig, timelock), review the relevant specification:

- **Escrow**: `docs/formal-verification/escrow/specifications.md`
- **Multisig**: `docs/formal-verification/multisig/specifications.md`
- **Timelock**: `docs/formal-verification/timelock/specifications.md`

Each specification lists the invariants that must be maintained during your changes.

### 2. Adding Runtime Checks (Optional Development Mode)

For debug builds, you can add runtime invariant checks:

```rust
// In contracts/escrow/src/lib.rs
#[cfg(debug_assertions)]
mod invariants {
    include!("../../../docs/formal-verification/escrow/invariants.rs");
}

// Then use in your code:
#[cfg(debug_assertions)]
{
    use invariants::*;
    debug_assert!(
        check_fee_accounting(gross, fee_bps, platform_fee, net),
        "Fee accounting invariant violated"
    );
}
```

### 3. Running Verification (Future)

Once Kani is set up:

```bash
# Verify escrow contract
cd contracts/escrow
cargo kani --harness verify_fund_conservation

# Verify all contracts
cd ../..
cargo kani --workspace
```

---

## Integration Checklist for Contract Changes

### Before Making Changes
- [ ] Read the contract's specification document
- [ ] Identify which invariants your change might affect
- [ ] Plan how to maintain those invariants

### During Development
- [ ] Add runtime assertions for new invariants (debug mode)
- [ ] Update state machine tests if state transitions change
- [ ] Add property tests for new arithmetic operations

### Before Committing
- [ ] Run all existing tests: `cargo test --workspace`
- [ ] Run state machine tests: `cargo test state_machine`
- [ ] Check that invariants are documented for new features
- [ ] Update specifications if contract behavior changes

### After Merging (Future)
- [ ] CI runs verification harnesses automatically
- [ ] Verification failures block deployment

---

## Updating Specifications

### When to Update

Update specifications when:
1. **Adding new operations**: Document new invariants
2. **Modifying state transitions**: Update state machine properties
3. **Changing access control**: Update authorization properties
4. **Adding storage fields**: Document immutability guarantees

### How to Update

1. **Document the change in the specification**:
   - Add new property to `specifications.md`
   - Use formal notation: `∀ x: condition(x) ⟹ result(x)`
   - Add verification strategy

2. **Add executable invariant to `invariants.rs`**:
   ```rust
   pub fn check_new_invariant(...) -> bool {
       // Implementation
   }
   ```

3. **Add Kani harness (future)**:
   ```rust
   #[cfg(all(test, kani))]
   #[kani::proof]
   fn verify_new_invariant() {
       // Proof implementation
   }
   ```

4. **Update INVARIANTS.md** if it's a cross-contract invariant

---

## Property-Based Testing Integration

The invariants can be used with proptest:

```rust
// In contracts/escrow/src/lib.rs
#[cfg(test)]
mod proptest_integration {
    use proptest::prelude::*;
    
    // Import invariants
    mod invariants {
        include!("../../../docs/formal-verification/escrow/invariants.rs");
    }
    use invariants::*;
    
    proptest! {
        #[test]
        fn prop_fee_always_valid(
            amount in 1i128..1_000_000_000,
            fee_bps in 0u32..=1000,
        ) {
            let platform_fee = (amount * fee_bps as i128) / 10_000;
            let net = amount - platform_fee;
            
            assert!(check_fee_accounting(amount, fee_bps, platform_fee, net));
        }
    }
}
```

---

## State Machine Test Integration

The specifications complement existing state machine tests:

```rust
// In tests/state_machine_tests.rs
use escrow::EscrowStatus;

#[test]
fn test_terminal_states() {
    let terminal_states = vec![
        EscrowStatus::Released,
        EscrowStatus::Refunded,
        EscrowStatus::Resolved,
    ];
    
    for state in terminal_states {
        // Per specification E2: terminal states are immutable
        assert_eq!(try_transition(state.clone(), any_operation()), state);
    }
}
```

---

## CI Integration

### GitHub Actions Example

```yaml
# .github/workflows/verification.yml
name: Formal Verification

on: [push, pull_request]

jobs:
  verify:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
          components: rust-src
      
      - name: Install Kani (future)
        run: |
          # cargo install --locked kani-verifier
          # cargo kani setup
          echo "Kani installation pending"
      
      - name: Run Tests
        run: cargo test --workspace
      
      - name: Run State Machine Tests
        run: cargo test state_machine
      
      # Future: Run Kani verification
      # - name: Verify Escrow
      #   run: cd contracts/escrow && cargo kani
```

---

## Documentation Standards

### Invariant Documentation Template

When adding a new invariant, use this template:

```markdown
### [ID]: [Property Name]
**Property**: [One-sentence description]

**Formal**:
```
∀ [variables]: [condition] ⟹ [result]
```

**Rationale**: [Why this property is important]

**Verification Strategy**:
- [How to test this property]
- [What tools to use]
- [Edge cases to consider]

**Implementation**:
```rust
pub fn check_[property_name](...) -> bool {
    // Implementation
}
```
```

---

## Common Patterns

### Pattern 1: Arithmetic Property
```rust
pub fn check_calculation(input: i128, output: i128) -> bool {
    // Use checked operations
    let expected = input.checked_mul(2).expect("overflow");
    output == expected
}
```

### Pattern 2: State Transition Property
```rust
pub fn check_valid_transition(old: State, new: State) -> bool {
    match (old, new) {
        (State::A, State::B) => true,
        (State::B, State::C) => true,
        _ => false,
    }
}
```

### Pattern 3: Authorization Property
```rust
pub fn check_authorized(
    operation: &str,
    caller: &Address,
    roles: &[&Address],
) -> bool {
    roles.contains(&caller)
}
```

---

## Troubleshooting

### Issue: Specification out of sync with code

**Symptom**: Code behavior doesn't match specification

**Solution**:
1. Update specification to match current behavior
2. Or fix code to match specification
3. Document why change was needed
4. Update verification harnesses

### Issue: Invariant violated in tests

**Symptom**: Invariant check fails during testing

**Solution**:
1. Determine if invariant is wrong or code is wrong
2. If invariant is wrong: update specification
3. If code is wrong: fix the bug
4. Add regression test

### Issue: New feature breaks old invariants

**Symptom**: Existing invariants no longer hold

**Solution**:
1. Evaluate if invariants need updating
2. If yes: update specifications with rationale
3. If no: redesign feature to maintain invariants
4. Document trade-offs

---

## Best Practices

### 1. Write Specifications First
When designing new features:
1. Write the specification (what should happen)
2. Define invariants (what must always hold)
3. Implement the feature
4. Add tests that verify the invariants

### 2. Use Type System
Let Rust's type system enforce invariants:
```rust
// Bad: use primitive types
pub fn transfer(amount: i128) { ... }

// Good: use newtype for domain constraints
pub struct PositiveAmount(i128);
impl PositiveAmount {
    pub fn new(value: i128) -> Result<Self, Error> {
        if value > 0 {
            Ok(PositiveAmount(value))
        } else {
            Err(Error::InvalidAmount)
        }
    }
}
```

### 3. Document Assumptions
Make assumptions explicit:
```rust
/// Transfer tokens to recipient.
///
/// # Invariants
/// - Balance conservation: sender.balance + recipient.balance unchanged
/// - Non-negative balances: all balances ≥ 0
///
/// # Assumptions
/// - Token contract follows SEP-41
/// - No reentrancy (Soroban platform guarantee)
pub fn transfer(recipient: Address, amount: i128) { ... }
```

### 4. Test at Multiple Levels
- Unit tests: Individual functions
- Integration tests: Cross-contract interactions
- Property tests: Random inputs over invariants
- State machine tests: All valid transitions
- Formal verification: Proof of invariants (future)

---

## Resources

- **Main Documentation**: `docs/formal-verification/README.md`
- **Invariants Catalog**: `docs/formal-verification/INVARIANTS.md`
- **Verification Workflow**: `docs/formal-verification/VERIFICATION_WORKFLOW.md`
- **Summary**: `docs/FORMAL_VERIFICATION_SUMMARY.md`

---

## Support

For questions about formal verification:
1. Review the specification documents
2. Check the verification workflow guide
3. Consult the invariants catalog
4. Open an issue with label `formal-verification`

---

**Last Updated**: 2026-07-24  
**Version**: 1.0.0  
**Status**: Active - Ready for Integration

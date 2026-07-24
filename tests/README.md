# Test Overview

The `tests/` package contains the integration and scenario tests for the contract workspace.

## Structure

- `integration_test.rs` covers cross-contract flows.
- `state_machine_tests.rs` validates allowed and disallowed state transitions.
- `upgrade_test.rs` checks upgrade-related behavior.
- Specialized files such as `flash_loan_tests.rs` and `oracle_manipulation_tests.rs` target specific risk areas.

## How to Run

```bash
npm test
```

or run the Rust workspace directly:

```bash
cargo test --workspace
```

## Writing Tests

- Keep fixtures small and explicit.
- Prefer helper functions for repeated setup.
- Cover both success and rejection cases.
- When a change affects storage, events, or access control, add a regression test in the same change.

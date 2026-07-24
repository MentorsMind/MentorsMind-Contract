# Test Strategy

This repository uses layered Rust tests to validate contract behavior before deployment.

## Test Structure

- `tests/` contains integration tests that exercise multiple contracts together.
- Each contract crate can also expose unit tests for contract-local state transitions and guard rails.
- Snapshot tests are used where the exact emitted event or serialized output matters.

## What the Test Suite Covers

- Authorization and ownership checks.
- Escrow lifecycle transitions.
- Upgrade and version tracking behavior.
- Oracle, risk, and dispute scenarios that can regress silently if left untested.
- Event emission correctness and ordering for state-changing operations (`tests/events/`).
- Event emission correctness and ordering checks (`tests/events/`).
- TTL bumping and persistence checks for long-lived storage keys (`tests/ttl/`).

## Testing Guidelines

- Test the full success path and the most important failure path for every new behavior.
- Prefer clear setup helpers over repeated inline fixture construction.
- Use ledger timestamps and generated addresses deliberately so the test intent is easy to read.
- When changing storage keys, event payloads, or validation rules, add a regression test in the same change.

## Coverage Goals

- Critical security and authorization paths should always have direct test coverage.
- State transitions should be verified for valid and invalid moves.
- New public contract entrypoints should ship with at least one positive and one negative test.

## Example

```rust
#[test]
fn release_requires_authorization() {
    let env = Env::default();
    env.mock_all_auths();
    // Arrange the escrow state, then assert the release call fails or succeeds as expected.
}
```

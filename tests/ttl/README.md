# TTL Management Tests

This folder tracks Time-To-Live (TTL) test coverage for persistent and instance storage keys.

## Automated TTL coverage

- `contracts/bounty/src/lib.rs`
  - verifies state survives ledger-sequence jumps after writes that call `extend_ttl`
  - verifies initialized admin/config keys remain available after sequence jumps
  - verifies deadline/dispute flows run under long-lived ledger settings

## Scenarios covered

1. TTL bumping on storage writes.
2. Data persistence across TTL renewal windows.
3. Expiration-window behavior via deadline/dispute tests.
4. Automatic renewal through state-changing contract calls.
5. Key-type coverage across config keys, counter keys, bounty records, and claim records.

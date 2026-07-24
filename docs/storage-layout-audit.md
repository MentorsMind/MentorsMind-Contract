# Storage Layout Audit Notes

This document captures the repository-level storage review performed for issue #406.

## Scope

- all Soroban contracts in `contracts/`
- `escrow/src/lib.rs`
- `contracts/upgrade_registry/src/lib.rs`
- existing upgrade verification in `tests/upgrade_test.rs`

## Soroban-Specific Model

MentorMinds stores state with typed `DataKey` enums and symbol or tuple namespaces. That means the main review target is serialized key compatibility across upgrades, not contiguous slot numbering as in EVM storage.

## Contract Families Reviewed

### Governance

- `Proposal(u32)`, `Vote(u32, Address)`, `VoteWeight(u32, Address)`, and `ApprovedAsset(Address)` are disjoint key shapes
- config items such as admin, token, quorum, and voting period stay separate from proposal history

### Staking

- admin and token config are stored in `instance` storage
- per-user stakes live under `Stake(Address)` in `persistent` storage
- aggregate state uses dedicated keys such as `Stakers` and `TotalStaked`

### Upgrade Registry

- `UpgradeHistory(Symbol)`, `LatestVersion(Symbol)`, and `Subscribers(Symbol)` keep each contract namespace isolated
- new upgrade metadata should be added under new variants instead of changing existing payloads

### Escrow

- escrow, session, milestone, and yield records are separated by dedicated symbols and tuple keys
- index structures for mentor and learner lookups are independent from the escrow payload itself

## Upgrade Integrity Guidance

- never repurpose an existing persisted key for a different payload
- add migration tests whenever persistent structs or key enums change
- prefer additive enum growth for `DataKey` instead of mutation
- keep this audit aligned with `tests/upgrade_test.rs` when future storage changes land

## Outcome

No EVM-style slot-gap patch is required for Soroban contracts in this repository. The relevant hardening action is preserving `DataKey` compatibility, documenting the key layout, and validating upgrade paths with storage-preservation tests.
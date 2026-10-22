# Snapshot Contract

A Soroban contract (`mentorminds-snapshot`) that records point-in-time staked MNT balances and delegation state, enabling stable, tamper-evident voting power and quorum calculations for governance proposals.

## Overview

- Records every staker's staked balance and the total staked supply at a given `snapshot_id`, freezing them for governance use.
- Snapshot delegation state at the same snapshot ID so delegated voters correctly show zero voting power.
- Computes and stores a checksum over the serialized staker data, allowing later integrity verification.
- Extends persistent-entry TTLs (~30 days at ~5s ledgers) for snapshot records, checksums, and total supply entries.
- Voting power is delegation-aware: a voter who delegated away at snapshot time returns 0; otherwise their recorded stake is returned.

## Entry Points

### Initialization

- `initialize(admin: Address, staking_contract: Address, delegation_contract: Address) -> Result<(), Error>` — Store the admin and the staking/delegation contract addresses; fails with `StateValidationFailed` if already initialized.

### Operations

- `record_snapshot(snapshot_id: u32) -> Result<(), Error>` — Record the snapshot: read total stake, staker list, and each `StakeRecord` from the staking contract; store per-staker balances and total supply; compute/store the data checksum; then trigger `snapshot_delegations` on the delegation contract. Fails with `CrossContractCallFailed` on any failed external call.

### Views

- `get_voting_power(snapshot_id: u32, voter: Address) -> Result<i128, Error>` — Voting power at the snapshot: 0 if the voter delegated away (per the delegation contract), otherwise their recorded staked balance.
- `get_snapshot_balance(snapshot_id: u32, staker: Address) -> i128` — Raw staked balance recorded for a staker at a snapshot (0 if absent).
- `get_total_supply_at(snapshot_id: u32) -> i128` — Total staked supply at the snapshot, for quorum calculation (0 if absent).
- `verify_snapshot_integrity(snapshot_id: u32) -> Result<bool, Error>` — Recompute the checksum from stored staker balances and compare with the stored checksum; returns false if the snapshot or any staker entry is missing.

The contract also exposes a `SnapshotContractTrait` declaring `get_voting_power(env: Env, snapshot_id: u32, voter: Address) -> i128` for use as a client interface via `#[contractclient(name = "SnapshotContractClient")]`.

## Storage

| DataKey variant | Description |
| --- | --- |
| `NamespaceRoot` | Contract-isolated storage namespace root |
| `Admin` | Admin address set at initialization |
| `StakingContract` | Address of the staking contract |
| `DelegationContract` | Address of the delegation contract |
| `Snapshot(u32, Address)` | Staked balance per `(snapshot_id, voter)` |
| `SnapshotTotalSupply(u32)` | Total staked supply at a snapshot ID |
| `SnapshotChecksum(u32)` | Checksum of serialized staker data for a snapshot ID |

## Events

This contract does not publish any events.

## Cross-Contract Dependencies

- Staking contract (stored in `StakingContract`): called via `try_invoke_contract` for `get_total_staked`, `get_stakers`, and `get_stake(staker) -> StakeRecord` (the `StakeRecord` type is imported from the `shared` crate so XDR layouts match the staking contract).
- Delegation contract (stored in `DelegationContract`): called via `try_invoke_contract` for `snapshot_delegations(snapshot_id)` during recording and `get_delegation_at_snapshot(snapshot_id, voter)` when computing voting power.
- `shared` crate: `compute_checksum` for snapshot integrity and `StakeRecord` for stake deserialization.

## Testing

```bash
cargo test -p mentorminds-snapshot
```

Tests cover multi-snapshot balance freezing, voting power across snapshots, Bronze/Silver/Gold tier correctness through the shared `StakeRecord` layout, full-structure serialize/deserialize round-trips, positional field-order documentation, and checksum-based integrity verification.

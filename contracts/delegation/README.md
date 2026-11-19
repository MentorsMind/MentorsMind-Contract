# Delegation Contract

On-chain governance delegation registry for the MentorMinds platform (`mentorminds-delegation`), letting token holders delegate voting weight with cycle protection, concentration caps, and historical snapshots.

## Overview

- Delegates governance weight to another address with re-delegation that moves weight rather than duplicating it.
- Rejects self-delegation and cycles; chain resolution is capped (`MAX_CHAIN_DEPTH = 8`).
- Eager subtree-weight caches allow O(1) weight propagation when a sub-chain re-delegates.
- Admin-configurable max delegation depth and concentration cap (basis points, default 10_000).
- Emergency suspension switch blocks new delegation writes while preserving reads.
- Snapshot support records historical delegations and delegated power per `snapshot_id`.

## Entry Points

### Initialization & Admin

- `initialize(admin: Address, mnt_token: Address)` — one-time setup; stores admin, MNT token, default depth/cap/suspension.
- `set_max_delegation_depth(admin: Address, depth: u32)` — update max delegation chain depth.
- `set_delegation_power_cap_bps(admin: Address, cap_bps: u32)` — update concentration cap in basis points.
- `set_delegation_suspended(admin: Address, suspended: bool)` — toggle emergency suspension.
- `set_snapshot_contract(admin: Address, snapshot_contract: Address)` — point at the governance snapshot registry.

### Delegation

- `delegate(delegator: Address, delegate: Address)` — delegate all governance weight; drops any prior delegation first.
- `undelegate(delegator: Address)` — cancel outgoing delegation and return weight.
- `revoke(delegator: Address)` — cancel outgoing delegation (alternate path).
- `validate_delegation_chain(delegator: Address, delegate: Address) -> Result<u32, DelegationError>` — returns chain depth, or `CircularDelegation`/`DepthExceeded`.

### Snapshots

- `snapshot_delegations(snapshot_id: u32)` — persist delegator→delegate and delegated power for a snapshot.
- `get_delegation_at_snapshot(snapshot_id: u32, delegator: Address) -> Option<Address>` — delegate at snapshot time.
- `get_delegated_power_at_snapshot(delegate: Address, snapshot_id: u32) -> i128` — total power delegated at snapshot time.

### Reads & Voting Weight

- `get_delegate(delegator: Address) -> Option<Address>` — current delegate, if any.
- `get_delegators(delegate: Address) -> Vec<Address>` — reverse index of delegators.
- `get_delegator_count(delegate: Address) -> u32` — number of delegators.
- `get_delegators_page(delegate: Address, offset: u32, limit: u32) -> Vec<Address>` — paginated delegators.
- `get_base_weight(proposal_id: u32, holder: Address) -> i128` — voting power from the snapshot contract.
- `get_voting_weight(proposal_id: u32, holder: Address) -> i128` — base weight plus resolved delegated weight.
- `get_delegated_power(delegate: Address) -> i128` — cached total delegated power.
- `get_effective_power(voter: Address) -> i128` — power the voter can currently cast with.
- `get_max_delegation_depth() -> u32` / `get_delegation_power_cap_bps() -> u32` / `is_delegation_suspended() -> bool` — config reads.
- `assess_delegate_concentration(delegate: Address) -> shared::DelegationConcentrationReport` — concentration vs cap.
- `invalidate_power_cache(delegator: Address, old_balance: i128, new_balance: i128)` — adjust caches after a balance change.

## Storage

| `DataKey` variant | Purpose |
|---|---|
| `NamespaceRoot` | Contract-isolated storage namespace root |
| `Admin` | Admin address (instance) |
| `MNTToken` | MNT token address (instance) |
| `SnapshotContract` | Snapshot registry address (instance) |
| `Delegate(Address)` | Outgoing delegation: delegator → delegate |
| `Delegators` | Global list of delegators |
| `MaxDelegationDepth` | Configurable max chain depth (u32) |
| `DelegatedPowerCache(Address)` | Ultimate delegate → sum of delegator balances |
| `SubtreeWeight(Address)` | Eager subtree weight for O(1) re-delegation |
| `DelegationAtSnapshot(u32, Address)` | Historical delegator → delegate per snapshot |
| `DelegationSnapshot(u32, Address)` | Snapshot-time delegated power per delegate |
| `MaxDelegatedPowerBps` | Concentration cap in bps |
| `DelegationSuspended` | Emergency suspension flag |
| `DelegatorCount(Address)` | Delegator count per delegate (pagination) |
| `DelegatorIndex(Address, u32)` | Sequential delegator index per delegate |

## Events

- `delegated` — a delegation was created (emitted both directly and via `shared::events::emit_delegation_event`).
- `revoked` — a delegation was revoked via `revoke`.
- `undelegated` — a delegation was removed via `undelegate`.
- `suspended` — suspension flag toggled.

## Cross-Contract Dependencies

- **Snapshot contract** — configured at init or via `set_snapshot_contract`; `get_base_weight` calls `get_voting_power(proposal_id, voter)` on it; `snapshot_delegations` invokes `get_snapshot_balance` on it.
- **MNT token** — stored at init; read through `soroban_sdk::token::Client::balance` to compute weights, subtree weight, and concentration.

## Testing

```bash
cargo test -p mentorminds-delegation
```

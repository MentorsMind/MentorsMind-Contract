# Composability Layer Guide

This guide documents the `interface_registry` contract that provides a standardized registry for platform interface discovery.

## Contracts
- `contracts/interface_registry/src/lib.rs`

## Interface IDs (standard)
- `escrow_v1`
- `reputation_v1`
- `token_v1`
- `oracle_v1`

## Public functions
- `initialize(env, admin: Address)` - set admin (one-time)
- `register_interface(env, contract: Address, interface_id: Symbol, version: u32)` - admin only
- `get_contract(env, interface_id: Symbol) -> Address` - get current implementation
- `get_version(env, interface_id: Symbol) -> u32` - get version
- `list_interfaces(env) -> Vec<InterfaceEntry>`

## Events
- `interface_registered` (id, address, version)
- `interface_updated` (id, address, version)

## Example
1. Deploy registry, set admin.
2. Admin registers `escrow_v1` pointing to escrow contract address.
3. Consumers call `get_contract(env, Symbol::new(&env, "escrow_v1"))`.
4. On upgrade, admin re-registers same ID with a new address and version.

## Behavior
- `register_interface` inserts new entry or updates existing.
- `list_interfaces` enumerates registry entries.
- `get_contract`/`get_version` panic if interface id is missing.

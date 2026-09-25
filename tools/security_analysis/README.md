# Cross-Contract Call Graph Analysis

This directory contains static, read-only analysis utilities that inventory the Soroban contract call graph without mutating the on-chain contract logic.

## What it analyzes

The scanner walks the contract source tree under `contracts/*/src/*.rs` and extracts `env.invoke_contract` / `env.try_invoke_contract` edges. The result is a directed graph where each node is a contract and each edge is a cross-contract method invocation.

## Included tools

- `call_graph.py` — builds the contract interaction graph and detects circular dependencies.
- `attack_path_discovery.py` — enumerates risky multi-hop trust chains in the graph.
- `visualize_call_graph.py` — emits Mermaid output suitable for static review and security documentation.

## Observed repository patterns

The current graph already contains several meaningful trust chains, including:

- `escrow_factory` -> `guardian` via `is_paused`
- `escrow_factory` -> `anomaly_detector` via `check_anomaly`
- `governance` -> `snapshot_contract` via `record_snapshot` and `get_total_supply_at`
- `governance` -> `delegation_contract` via `get_delegated_power_at_snapshot`
- `cross_contract_auth` -> `interface_registry` via `verify`

These are the exact kinds of cross-contract dependencies that the issue description is concerned with: authorization, state propagation, and path-based trust chaining.

## Security review focus

The generated graph is meant to support:

1. inter-contract dependency tracking,
2. circular dependency detection,
3. formal invariant review across state boundaries,
4. attack path discovery for chained trust assumptions,
5. documentation and visualization of contract security boundaries.

Because this is a static analysis-only tool, it intentionally does not modify any contract, tests, or runtime behavior.

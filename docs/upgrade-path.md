# Smart Contract Upgrade Path Documentation

**Version**: 1.0  
**Date**: March 2026  

---

## Overview

The MentorMinds smart contract ecosystem utilizes a **UUPS-style (Universal Upgradeable Proxy Standard) architecture** adapted for the Soroban runtime. Contracts can swap their underlying WASM bytecode while maintaining the same contract address and eternal storage state.

This document outlines the upgrade path and security model to assist auditors in verifying the upgrade mechanisms.

---

## 1. Upgrade Architecture

### 1.1 WASM Swapping Mechanism
In Soroban, contracts are upgraded by calling the built-in `env.deployer().update_current_contract_wasm(new_wasm_hash)` function. 
- **Preserved**: Contract ID (Address), all storage tiers (`Persistent`, `Instance`).
- **Replaced**: Contract logic (functions, behavior).

### 1.2 The Eternal Storage Pattern
To ensure that new WASM versions do not corrupt existing data, MentorMinds heavily relies on the **Eternal Storage Pattern**.
- Data keys are rigidly defined as explicit Rust Enums (e.g., `DataKey::Admin`, `DataKey::Escrow(id)`).
- Upgrades **must not** modify the order or type signature of existing `DataKey` enum variants. New storage requirements must introduce *new* enum variants.

---

## 2. Authorization & Security Controls

The upgrade process is the most privileged operation in the system. It is strictly guarded by the following path:

1. **Multi-Sig Admin Approval**: Upgrades cannot be triggered by a single hot wallet. They require consensus from the `contracts/multisig_admin` module.
2. **Timelock Delay**: Once proposed and signed, an upgrade is subject to the Timelock contract delay (e.g., 48 hours). This gives the community and users time to inspect the new WASM hash or exit the system if they disagree with the upgrade.
3. **Upgrade Registry Integration**: The upgrade is routed through `contracts/upgrade_registry/`. This central registry:
   - Emits definitive on-chain upgrade logs (`old_wasm`, `new_wasm`, `timestamp`).
   - Notifies subscribed listeners of the change.
4. **Execution**: The `upgrade` function validates the caller is the Multi-Sig/Timelock, verifies the WASM hash, and executes the swap.

---

## 3. Step-by-Step Upgrade Procedure

### Phase 1: Preparation
1. Compile the new contract: `cargo build --target wasm32-unknown-unknown --release`
2. Optimize the WASM payload.
3. Upload the new WASM to the Stellar ledger without executing it (generates a `WasmHash`).

### Phase 2: Proposal
1. Admin submits a transaction to the Multi-Sig proposing the `upgrade(new_wasm_hash)` function.
2. Quorum of signers approve the proposal.
3. Proposal is sent to the Timelock contract.

### Phase 3: Execution
1. The Timelock delay expires.
2. Any user can trigger the Timelock execution.
3. The target contract updates its WASM and registers the change.

---

## 4. Auditor Threat Model Constraints

Auditors should verify the following invariants during the review of upgradeable components:

- [ ] `upgrade()` functions **must** contain rigorous `require_auth()` checks verifying the Admin/Timelock.
- [ ] Upgrades **must not** inadvertently expose an `initialize()` vulnerability. `initialize()` functions must check `if env.storage().instance().has(&DataKey::Admin) { panic!() }`.
- [ ] No `temporary` storage keys should be relied upon to survive an upgrade process, as they may be evicted.

---

## 5. Rollback Plan

Because the contract state is preserved, "rolling back" requires deploying an upgrade payload containing the previous stable `WasmHash`. 
- A rollback is treated exactly like an upgrade. 
- It must follow the same Multi-Sig → Timelock authorization flow.
- *Note: Emergency fast-track rollbacks bypass the timelock ONLY IF the Multi-Sig signs a dedicated emergency priority flag.*
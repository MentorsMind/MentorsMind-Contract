# MentorMinds Contract Suite (Stellar Soroban)

This repository contains the on-chain smart contract suite for the MentorMinds platform, built on **Stellar Soroban** using **Rust**.

The contracts implement a secure escrow workflow for mentoring sessions and a set of supporting modules (admin/security tooling, dispute resolution, verification, payments, and analytics). Off-chain services (see `mentorminds-backend/`) are responsible for indexing, orchestration, and automation around the on-chain state.

---

## 1. What this project delivers

At a high level, the system provides:

- **Escrow-based payments** for mentoring sessions (fund custody, release/refund, and dispute handling)
- **Multi-sig + timelock administration** for sensitive operations (fee/treasury/admin changes)
- **Upgrade management** via an internal UUPS-style registry (recording upgrade history and performing WASM swaps)
- **Typed/upgrade-safe storage** using the shared “eternal storage” pattern
- **Verification / governance / dispute evidence plumbing** for the mentoring + arbitration lifecycle
- **Payment routing and revenue distribution** (platform fee, referral rewards, insurance accrual)

---

## 2. Repository layout

### 2.1 Root workspace

This is a Cargo workspace (`Cargo.toml`) containing multiple Soroban contract crates under `contracts/`, plus:

- `escrow/` – the primary escrow contract crate (often used as the core deployment artifact)
- `contracts/*` – the supporting contract crates (verification, timelock, treasury, dispute evidence, etc.)
- `tests/` – workspace tests and scenario suites (including state machine tests)
- `scripts/` – deployment, upgrade, local environment setup, and helper scripts
- `docs/` – architecture, deployment, security, runbooks, and operational guidance


### 2.2 Contracts (general pattern)

Each contract is a Rust crate with:

- `Cargo.toml`
- `src/` containing the Soroban contract implementation
- `test_snapshots/` and/or `tests/` directories (where applicable)

Contracts commonly expose:

- an `initialize` entrypoint (admin setup)
- core business methods (escrow lifecycle, releases, dispute flows, etc.)
- typed view methods for off-chain reads
- event emission for off-chain indexing

---

## 3. High-level contract architecture

The contract suite is designed around composability and explicit state transitions.

### 3.1 Admin/security modules

- **Multi-sig Admin** (`contracts/multisig_admin/`):
  - proposal → sign → execute flow for sensitive actions
  - supports configurable signer set and threshold
  - optional self-targeted operations (add/remove signer, update threshold)
  - includes proposal expiry

- **Timelock** (`contracts/timelock/`):
  - schedules operations with bounded delay (min/max)
  - anyone can execute after the delay elapses
  - timelock admin can cancel pending operations

- **Upgrade Registry** (`contracts/upgrade_registry/`):
  - implements an internal UUPS-style upgrade workflow adapted for Soroban
  - swap WASM at the same contract address while preserving storage keys
  - records upgrade history and supports subscription notifications

### 3.2 Shared infrastructure

- **Eternal Storage Pattern** (`contracts/shared/src/storage.rs`):
  - separates storage layout from business logic using typed key enums
  - defines storage tiers:
    - Instance storage: config read per call
    - Persistent storage: per-entity records
    - Temporary storage: locks/nonces/rate limiting
  - improves upgrade safety (keys are explicit; adding variants is safe)

- **State machine methodology**:
  - enforced via shared `StateMachine` trait (see `contracts/shared`)
  - `tests/state_machine_tests.rs` verifies valid transitions exhaustively

### 3.3 Business-domain contracts (examples)

The repository includes many domain contracts. Common roles:

- **Escrow core** (`escrow/` or `contracts/escrow/` depending on build target):
  - creates escrows, holds funds, releases/refunds based on workflow
  - manages dispute entry and resolution consumption

- **Dispute-related contracts** (`contracts/dispute_evidence/`, `contracts/governance/`, etc.):
  - evidence submission and dispute resolution records
  - governance to determine arbitrators / selection for deterministic IDs
  - escrow consumes arbitration outcomes to move from disputed → terminal states

- **Verification** (`contracts/verification/`):
  - mentor verification and reviews

- **Revenue distribution** (escrow + treasury/referral/insurance pattern):
  - escrow release deducts platform fee
  - treasury receives the platform share
  - referral rewards and insurance accrual are derived from the platform fee breakdown

---

## 4. Documentation map

Most of the “how it works” knowledge lives under `docs/` and the key root markdown files:

- `ARCHITECTURE.md` – security + upgrade architecture, system diagrams references, fee strategy
- `LOCAL_DEVELOPMENT.md` – local Docker-based environment setup and test accounts
- `docs/TYPES.md` – all contract types, enums, and structs with field constraints, usage examples, and type-relationship diagrams
- `docs/DEPLOYMENT_GUIDE.md` – deployment + initialization parameters and script usage
- `docs/STATE_MACHINE.md` and `docs/state-machines.md` – state transition rules and diagrams
- `docs/TROUBLESHOOTING.md` – operational issues and quick fixes
- `docs/SECURITY.md` / `SECURITY.md` / `docs/threat-model.md` – security model documentation
- `BENCHMARKS.md` – gas/resource tracking targets for critical functions
- `docs/TESTING.md` – testing strategy and environment notes
- `docs/UPGRADE_GUIDE.md` – upgrade concepts and recommended process

---

## 5. Local development

Local testing is designed around Dockerized Stellar infrastructure.

See `LOCAL_DEVELOPMENT.md` for:

- prerequisites (Docker Desktop, Rust, Node, Soroban CLI)
- available npm scripts (`local:start`, `local:stop`, `local:seed`, etc.)
- endpoints (Horizon/RPC/Friendbot/Soroban RPC)
- test accounts and generated deployment artifacts under `deployed/`

Typical workflow:

1. `npm run local:start`
2. `npm run local:seed` (optional sample data)
3. Build: `npm run build`
4. Test: `npm run test`
5. Invoke contract methods using `soroban contract invoke`

---

## 6. Build, test, and optimize

### 6.1 Build all contracts

```bash
cargo build --workspace --target wasm32-unknown-unknown --release
```

The repo also exposes this via npm:

```bash
npm run build
```

### 6.2 Test

```bash
cargo test --workspace
```

### 6.3 Optimize WASM

The npm script `optimize` runs `soroban contract optimize` for key artifacts.

---

## 7. Deployment (testnet/mainnet)

Deployment instructions and parameters are in `docs/DEPLOYMENT_GUIDE.md`.

Key points:

- `./scripts/deploy.sh` is the recommended single entrypoint for full deployment + initialization + verification
- deployed IDs are written to `deployed/<network>.json`
- initialization parameters include:
  - `fee_bps`
  - `auto_release_delay_secs`
  - `approved_tokens`
  - admin/treasury settings
- mainnet requires validation RPC access (via `VALIDATION_CLOUD_KEY`)

---

## 8. How admin operations are secured (pattern)

A typical security composition for sensitive changes is:

1. **Multi-sig** reaches threshold to approve an action
2. **Timelock** schedules the action so the community has time to react
3. after the delay, the action is executed

This design is documented in `ARCHITECTURE.md` under:

- Multi-sig Admin section
- Timelock section
- Security considerations

---

## 9. Upgrade strategy (high-level)

Soroban upgrades are handled by swapping WASM at the same contract address using Soroban deployer APIs.

The repo implements a **UUPS-style upgrade workflow**:

- admin-authorized upgrade entrypoint performs authorization
- `upgrade_registry` records history
- storage remains accessible through the eternal storage key scheme

See `docs/UPGRADE_GUIDE.md` and `ESCROW_FIXES_SUMMARY.md` / `IMPLEMENTATION_SUMMARY.md` (where present) for migration notes.

---

## 10. Performance and resource constraints

Gas/resource budgets are tracked in `BENCHMARKS.md`.

The project includes:

- benchmarks for core escrow functions
- a methodology for tracking CPU instruction and memory usage

A key requirement is to keep critical functions under a target CPU-instruction limit.

---

## 11. Contributing

See `CONTRIBUTING.md` for workflow guidelines.

---

## 13. License

MIT License (see `LICENSE` if present in the repository).

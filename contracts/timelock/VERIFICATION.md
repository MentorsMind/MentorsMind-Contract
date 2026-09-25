# TimelockController — Formal Verification Report (Issue #622)

This document describes the machine-checked invariant proofs for the
`TimelockController`, the security-critical contract that gates governance-driven
parameter changes and upgrades.

## Tooling

Proofs are written for [Kani](https://model-checking.github.io/kani/), a bit-precise
bounded model checker for Rust.

```sh
cargo install --locked kani-verifier
cargo kani setup
cd contracts/timelock
cargo kani
```

An optional, **non-blocking** CI job (`.github/workflows/kani.yml`) runs the same
proofs on pushes/PRs that touch the timelock. It uses `continue-on-error: true`
so it never blocks required checks.

## Verification boundary (important)

Kani verifies **native Rust**, not the Soroban WASM contract calling convention.
The `#[contractimpl]` entry points (`schedule`, `execute`, `cancel`) cannot be
symbolically executed directly, because they:

- depend on the Soroban host `Env` (ledger timestamp, persistent storage,
  cross-contract `invoke_contract`, `require_auth`), which has no Kani model, and
- are rewritten by the `#[contractimpl]` proc-macro into host-ABI shims.

To get **real, compiling, running** proofs rather than proofs that don't build,
the security-critical logic — timestamp arithmetic and state-transition rules —
is factored into pure functions in the `logic` module of `src/lib.rs`:

| Pure function                         | Used by contract entry point            |
| ------------------------------------- | --------------------------------------- |
| `logic::is_valid_delay(delay)`        | `schedule` delay-range check            |
| `logic::compute_ready_at(now, delay)` | `schedule` `ready_at` computation       |
| `logic::is_executable(now, ready_at)` | `execute` / `is_operation_ready` window |
| `logic::can_transition(done)`         | `execute` / `cancel` `!done` guard      |
| `logic::can_cancel(is_proposer, is_admin)` | `cancel` authorization             |

`is_operation_ready` is wired to call `logic::is_executable` directly, so the
verified predicate is the exact one used on-chain. The Kani harnesses in
`src/proofs.rs` (gated behind `#[cfg(kani)]`) prove properties over these
functions. This module boundary is the verification boundary: we prove the pure
invariant logic, and rely on code review for the thin `Env` glue that reads
`env.ledger().timestamp()` and storage and forwards them to these functions.

## Proven invariants

1. **`proof_schedule_sets_future_ready_at`** — for any valid delay
   (`MIN_DELAY..=MAX_DELAY`) and any non-overflowing `now`,
   `compute_ready_at(now, delay)` yields `ready_at > now` (in fact
   `ready_at >= now + MIN_DELAY`). *Invariant 1.*
2. **`proof_execute_window`** — `is_executable(now, ready_at)` is true **iff**
   `now >= ready_at + TOLERANCE && now < ready_at + EXPIRY`. Both directions are
   proven (no false positives and no false negatives). *Invariant 2.*
3. **`proof_done_is_terminal`** — `can_transition(done)` is false whenever
   `done`, so once an operation is `done` no further `execute`/`cancel`
   transition is permitted. *Invariant 3.*
4. **`proof_cancel_authorization`** — `can_cancel(is_proposer, is_admin)` holds
   iff at least one role is present, and a single role is always sufficient; the
   predicate never requires both simultaneously. *Invariant 4.*

Each harness is annotated with `#[kani::unwind(32)]` per the bounded-depth
requirement. The predicates are loop-free, so the bound is not a limiting factor;
the annotation documents the intended verification depth.

## Assumptions

- **Timestamp monotonicity / provenance.** `now` is supplied by the Soroban
  ledger environment (`env.ledger().timestamp()`). We assume the host provides a
  well-formed `u64` timestamp; the proofs quantify over all such values.
- **Overflow is a rejected path, not a success path.** The contract uses
  `checked_add(...).expect(...)` for `now + delay` and `ready_at + EXPIRY`.
  Overflow therefore panics (aborts the call) rather than producing a wrong
  result. The harnesses `kani::assume` the non-overflowing sub-domain, matching
  the reachable success paths; the overflow paths are safe by construction
  (guaranteed abort).

## Unverified paths

The following are **not** covered by Kani and are covered instead by the unit
tests in `src/lib.rs` and by code review:

- Storage read/write correctness (`DataKey::Op`, `DataKey::Admin`, `OpCount`).
- `require_auth` enforcement and the actual identity of `proposer` / `admin`
  (Kani models authorization as the boolean predicate `can_cancel`, not the host
  auth check).
- `op_id` derivation via SHA-256 over the XDR-encoded payload
  (`env.crypto().sha256`).
- The cross-contract dispatch performed by `execute`
  (`env.invoke_contract`).
- Event emission.

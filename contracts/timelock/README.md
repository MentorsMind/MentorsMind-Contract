# Timelock Contract

A Soroban timelock controller (`mentorminds-timelock`) that enforces a mandatory delay between proposing and executing privileged operations, with an independent guardian multisig able to emergency-cancel any pending operation even against a compromised admin.

## Overview

- Delayed execution: operations are scheduled with a delay between 48 hours (`MIN_DELAY`) and 30 days (`MAX_DELAY`).
- Operations expire 14 days (`OPERATION_EXPIRY_SECS`) after their `ready_at` time and cannot be executed afterwards.
- Operation IDs are SHA-256 hashes over the full payload (proposer, target, function, args, ready_at, nonce, salt), preventing prediction and collisions.
- A separately registered guardian multisig (minimum 4-of-7 configuration) can `emergency_cancel` any pending operation, rate-limited to 3 overrides per 30 days, with a required justification hash and an immutable audit trail.
- Guardian rotation is controlled by the current guardian itself (not the admin) once bootstrapped; the admin can only set the first guardian.
- Security-critical timestamp/state-transition logic is factored into a pure `logic` module verified with Kani formal-verification harnesses (`src/proofs.rs`, gated behind `#[cfg(kani)]`).

## Entry Points

### Initialization / Admin

- `initialize(admin: Address) -> Result<(), Error>` — Set the admin and zero the operation counters. Errors if already initialized.
- `transfer_admin(new_admin: Address)` — Transfer the admin role; requires current admin auth.
- `set_guardian_multisig(admin: Address, new_guardian: Address) -> Result<(), Error>` — Bootstrap (admin sets first guardian) or rotate (only current guardian can replace itself). Verifies the new guardian via cross-contract calls to `get_threshold`/`get_signer_count` (floor: 4-of-7).

### Operations

- `schedule(caller: Address, target: Address, function: Symbol, args: Vec<Val>, delay: u64, salt: BytesN<32>) -> Result<BytesN<32>, Error>` — Schedule a delayed call; requires caller auth, validates delay bounds, returns the derived `op_id`.
- `execute(operation_id: BytesN<32>)` — Invoke the target contract with the stored function/args once ready (after `ready_at` + 60s tolerance) and before expiry; marks the operation done.
- `cancel(operation_id: BytesN<32>)` — Cancel a pending operation; requires auth of the proposer or the admin (whichever is applicable).
- `emergency_cancel(guardian_multisig: Address, operation_id: BytesN<32>, reason_hash: BytesN<32>) -> Result<(), Error>` — Guardian-only cancel regardless of proposer/admin; requires non-zero justification hash, enforces the override rate limit, and records an audit entry.

### Views

- `get_guardian_multisig() -> Option<Address>` — Registered guardian multisig, if any.
- `is_operation_ready(operation_id: BytesN<32>) -> bool` — True if delay elapsed, tolerance satisfied, not expired, not done.
- `is_operation_expired(operation_id: BytesN<32>) -> bool` — True if the operation exists but passed its expiry window unexecuted.
- `is_operation_done(operation_id: BytesN<32>) -> bool` — True if the operation was executed or cancelled.
- `get_operation(operation_id: BytesN<32>) -> Operation` — Full operation struct.
- `get_operation_count() -> u32` — Total operations ever scheduled.
- `get_operations_page(offset: u32, limit: u32) -> Vec<OperationEntry>` — Bounded page of operations in scheduling order (limit clamped to `MAX_PAGE_SIZE`).
- `get_admin() -> Address` — Current admin.
- `get_guardian_audit(action_id: BytesN<32>) -> Option<GuardianActionAudit>` — Audit trail entry for a guardian action.
- `get_guardian_audit_count() -> u32` — Total guardian actions recorded.
- `get_guardian_rotation(rotation_id: u32) -> Option<GuardianRotationRecord>` — Guardian rotation history entry.
- `get_guardian_rotation_count() -> u32` — Total guardian rotations.
- `is_guardian_rotation_due() -> bool` — True if ~6 months (`GUARDIAN_ROLE_ROTATION_SECS`) elapsed since the last rotation.
- `get_guardian_action_veto_power(action_id: BytesN<32>) -> i128` — Accumulated veto power for a guardian action.

## Storage

| DataKey variant | Description |
| --- | --- |
| `NamespaceRoot` | Contract-isolated storage namespace root |
| `Admin` | Current admin address (instance storage) |
| `OpCount` | Global operation nonce counter (instance) |
| `OperationCount` | Total scheduled operations (instance) |
| `OperationIndex(u32)` | Index → operation ID mapping for pagination (persistent) |
| `Op(BytesN<32>)` | Operation struct by ID (persistent) |
| `Cancelled(BytesN<32>)` | Cancellation flag by operation ID (persistent) |
| `GuardianMultisig` | Registered emergency guardian multisig address (instance) |
| `GuardianOverrideTimestamps(Address)` | Per-guardian override timestamps for 30-day rate limiting (instance) |
| `GuardianAudit(BytesN<32>)` | Immutable guardian action audit trail (persistent) |
| `GuardianAuditCount` | Total guardian actions counter (instance) |
| `GuardianRotation(u32)` | Guardian rotation history by index (persistent) |
| `GuardianRotationCount` | Total guardian rotations (instance) |
| `LastGuardianRotation` | Last guardian rotation timestamp (instance) |
| `GuardianActionVetoPower(BytesN<32>)` | Veto power accumulator per guardian action (persistent) |

## Events

Timelock events are emitted via `emit_timelock_event` in the `shared` crate with these event-type symbols:

- `initialized` — Timelock initialized with an admin.
- `scheduled` — A delayed operation was scheduled (caller, target, function, delay).
- `executed` — An operation was executed (operation ID).
- `cancelled` — An operation was cancelled by proposer/admin (operation ID).
- `admin_xfr` — Admin role transferred (old admin, new admin).
- `em_cancel` — Guardian emergency cancel (operation ID, guardian, reason hash).
- `guard_set` — Guardian multisig registered or rotated (admin, new guardian).

## Cross-Contract Dependencies

- `multisig-admin` (`MultisigAdminContractClient`): `set_guardian_multisig` calls `get_threshold()` and `get_signer_count()` on the proposed guardian contract to enforce the 4-of-7 floor; the registered guardian multisig is expected to gate calls to `emergency_cancel` behind its own approval threshold (as exercised in tests via `propose_action` / `sign_action` / `execute_action`).
- Arbitrary target contracts: `execute` performs `env.invoke_contract` on the operation's stored target/function/args.
- `shared` crate: event helpers, `Pagination`, and `MAX_PAGE_SIZE` for operation listing.

## Testing

```bash
cargo test -p mentorminds-timelock
```

Tests cover initialization, delay/tolerance/expiry windows, salt-derived op-ID uniqueness, pagination of scheduled/executed/cancelled history, guardian bootstrap and rotation, weak-guardian rejection, and guardian emergency cancellation of admin-proposed operations. Kani proofs are run separately with `cargo kani` from this crate (see `VERIFICATION.md`).

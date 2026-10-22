# Onboarding Escrow Contract

Mentor onboarding state tracker for the MentorsMind platform, recording completion of the three onboarding steps (verified, bonded, first session completed) and flagging whether a mentor's first escrow uses the extended auto-release delay.

## Overview

- Tracks a fixed three-step onboarding flow: `Verified`, `Bonded`, and `FirstSessionCompleted`.
- `init_mentor` seeds a mentor's status and marks their first escrow as extended by default.
- `complete_onboarding_step` is called by the relevant subsystem contracts (verification, performance bond, escrow release) as each step finishes.
- Automatically flips `onboarding_complete` once all steps are done; later step calls are no-ops.
- Exposes constants for the 7-day extended first-escrow auto-release delay and the 30-day incomplete-onboarding refund deadline.
- Purely a state machine — holds no funds and makes no cross-contract calls.

## Entry Points

### Lifecycle

- `init_mentor(mentor: Address)` — initializes a mentor's onboarding status (idempotent; no-op if already initialized) and sets the first-escrow extended flag. Requires mentor auth.
- `complete_onboarding_step(mentor: Address, step: OnboardingStep)` — marks one step complete, persists updated status, and emits a `STEP` event (plus `DONE` when the final step lands). Panics if the mentor was never initialized; no-op once onboarding is complete.

### Queries

- `get_onboarding_status(mentor: Address) -> OnboardingStatus` — returns the stored per-mentor status flags. Panics if the mentor was never initialized.
- `is_first_escrow_extended(mentor: Address) -> bool` — whether the mentor's first escrow uses the extended delay (defaults to false if uninitialized).
- `is_onboarding_complete(mentor: Address) -> bool` — true once all three steps are complete.

## Storage

| DataKey Variant | Contents |
| --- | --- |
| `NamespaceRoot` | Contract-isolated storage namespace root |
| `MentorOnboardingStatus(Address)` | Per-mentor `OnboardingStatus` (verified / bonded / first-session / complete flags) |
| `FirstEscrowExtended(Address)` | Whether the mentor's first escrow uses the extended delay |

## Events

| Topic | Description |
| --- | --- |
| `("STEP", mentor)` | A step was completed; payload is `(step, remaining_steps)` |
| `("DONE", mentor)` | Onboarding fully complete (all steps done); empty payload |

## Cross-Contract Dependencies

- No on-chain contract invocations; `Cargo.toml` depends only on `soroban-sdk`.
- Source doc comments identify the callers that record step completion: the verification, performance-bond, and escrow-release subsystems.
- Escrow-related timing constants: `EXTENDED_AUTO_RELEASE_SECS` (7 days) for the first escrow's auto-release delay and `ONBOARDING_DEADLINE_SECS` (30 days) as the refund deadline for incomplete onboarding.

## Testing

```bash
cargo test -p onboarding_escrow
```

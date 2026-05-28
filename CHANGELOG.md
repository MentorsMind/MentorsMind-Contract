# Changelog

## [Unreleased]

### Added
- **Multi-sig admin** (`contracts/multisig_admin`): 2-of-3 and 3-of-5 approval thresholds, proposal/sign/execute/cancel lifecycle, `update_threshold` via proposal, full event logging and tests.
- **Timelock** (`contracts/timelock`): 24h minimum delay for fee changes, treasury updates, and admin transfers. Emergency cancel mechanism. Full test suite.
- **UUPS upgrade registry** (`contracts/upgrade_registry`): `upgrade_contract()` using Soroban's `update_current_contract_wasm`, upgrade history tracking, subscriber notifications.
- **Eternal storage** (`contracts/shared/src/storage.rs`): `EternalStorage` helper, canonical `InstanceKey` / `PersistentKey` / `TempKey` enums, schema version tracking.
- **ARCHITECTURE.md**: Documents multi-sig, timelock, UUPS, and eternal storage patterns.
- **docs/UPGRADE_GUIDE.md**: Step-by-step contract upgrade procedures.

## [0.1.0] - 2026-05-28

### Added
- Initial escrow contract with dispute resolution and auto-release.
- Multi-sig wallet skeleton.
- Payment router.
- State machine methodology via `contracts/shared`.

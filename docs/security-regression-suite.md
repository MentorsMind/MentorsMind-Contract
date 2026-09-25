# Security Regression Suite

Continuously validates that identified attack vectors remain closed across every
PR and on a nightly schedule. A failure means a previously-closed vulnerability
may have been re-opened — it blocks merge.

## Running locally

```bash
# Full suite
cargo test -p mentorminds-integration-tests --test security_regression

# One threat category
cargo test -p mentorminds-integration-tests --test security_regression priv_esc
cargo test -p mentorminds-integration-tests --test security_regression replay
cargo test -p mentorminds-integration-tests --test security_regression unauth_upgrade
cargo test -p mentorminds-integration-tests --test security_regression multisig_bypass
cargo test -p mentorminds-integration-tests --test security_regression timelock_manip
cargo test -p mentorminds-integration-tests --test security_regression reinit
cargo test -p mentorminds-integration-tests --test security_regression param_abuse
```

## Threat categories

| Tag | Tests | Contracts under test |
|-----|-------|----------------------|
| `priv_esc` | 6 | RBAC, UpgradeRegistry, Multisig |
| `replay` | 5 | Timelock, Multisig, UpgradeRegistry |
| `unauth_upgrade` | 7 | UpgradeRegistry |
| `multisig_bypass` | 7 | Multisig |
| `timelock_manip` | 7 | Timelock, UpgradeRegistry, Multisig |
| `reinit` | 5 | RBAC, UpgradeRegistry, Timelock, Multisig |
| `param_abuse` | 4 | PerformanceBond + shared params registry |

## Coverage map

### Privilege escalation (`priv_esc`)

| Test | Assumption validated |
|------|----------------------|
| `priv_esc_non_admin_cannot_grant_role` | Only super-admin can grant roles |
| `priv_esc_non_admin_cannot_revoke_role` | Only super-admin can revoke roles |
| `priv_esc_revoke_nonexistent_role_returns_error` | Revoking an unheld role is an explicit error, not silent |
| `priv_esc_role_holder_cannot_escalate_to_super_admin` | Lateral role use cannot escalate vertically |
| `priv_esc_outsider_cannot_schedule_upgrade` | Upgrade scheduling requires registered signer |
| `priv_esc_non_signer_cannot_approve_multisig_tx` | Multisig approval requires signer membership |

### Replay attacks (`replay`)

| Test | Assumption validated |
|------|----------------------|
| `replay_timelock_same_salt_different_nonce_distinct_op_ids` | Per-operation nonce prevents salt-based collision |
| `replay_timelock_executed_op_rejected` | `AlreadyDone` prevents double-execution |
| `replay_multisig_double_approval_rejected` | Approval idempotency is enforced per signer |
| `replay_upgrade_executed_upgrade_cannot_repeat` | `NoPendingUpgrade` prevents repeated execution |
| `replay_upgrade_version_rollback_rejected` | `VersionNotMonotonic` blocks downgrade and re-use |

Also covered by `sig_validation_tests.rs`:
- Nonce not consumed on deadline failure
- Cross-contract replay blocked by `contract_id` binding
- Cross-action replay blocked by `action` discriminant

### Unauthorized upgrades (`unauth_upgrade`)

| Test | Assumption validated |
|------|----------------------|
| `unauth_upgrade_single_key_below_threshold_rejected` | One compromised key cannot schedule |
| `unauth_upgrade_unregistered_signer_explicitly_rejected` | Unknown addresses get `NotSigner`, not `BelowThreshold` |
| `unauth_upgrade_duplicate_signer_in_approvers_rejected` | Vote stuffing via duplicates is detected |
| `unauth_upgrade_execute_step_rechecks_threshold` | Schedule and execute approvals are independent |
| `unauth_upgrade_concurrent_upgrade_rejected` | Only one upgrade can be in-flight |
| `unauth_upgrade_admin_rotation_requires_threshold` | Admin rotation requires M-of-N, not just admin |

### Multisig bypass (`multisig_bypass`)

| Test | Assumption validated |
|------|----------------------|
| `multisig_bypass_execute_below_threshold_panics` | Threshold enforced at execution, not just approval |
| `multisig_bypass_zero_approvals_cannot_execute` | Zero approvals always blocked |
| `multisig_bypass_cancelled_tx_locked_out` | Cancelled status is terminal — no further approvals |
| `multisig_bypass_executed_tx_locked_out` | Executed status is terminal — no re-execution |
| `multisig_bypass_stranger_cannot_cancel_tx` | Only admin or proposer can cancel |
| `multisig_bypass_remove_signer_below_threshold_rejected` | Signer count cannot fall below threshold |
| `multisig_bypass_zero_threshold_rejected_at_init` | Zero threshold rejected at construction |
| `multisig_bypass_threshold_exceeds_signers_rejected` | Unsatisfiable threshold rejected at construction |

### Timelock manipulation (`timelock_manip`)

| Test | Assumption validated |
|------|----------------------|
| `timelock_manip_execute_before_delay_rejected` | `NotReady` enforced for all premature executions |
| `timelock_manip_execute_at_tolerance_boundary_rejected` | Tolerance window does not open early execution |
| `timelock_manip_below_min_delay_rejected_at_schedule` | `InvalidDelay` catches short-circuit scheduling |
| `timelock_manip_cancelled_op_cannot_execute` | `OperationNotFound` after cancel |
| `timelock_manip_non_admin_cannot_cancel` | `NotAdmin` enforced on cancel |
| `timelock_manip_expired_operation_rejected` | Operations expire 14 days after ready_at |
| `timelock_manip_upgrade_registry_timelock_enforced` | Registry's own 48h delay is independent |
| `timelock_manip_multisig_execute_before_timelock_panics` | Multisig `execute_after` enforced |

### Re-initialization (`reinit`)

| Test | Assumption validated |
|------|----------------------|
| `reinit_rbac_double_init_rejected` | RBAC double-init returns error |
| `reinit_upgrade_registry_double_init_rejected` | Registry double-init returns error |
| `reinit_timelock_double_init_rejected` | Timelock returns `AlreadyInitialized` |
| `reinit_multisig_double_init_rejected` | Multisig panics on re-init |
| `reinit_upgrade_registry_admin_unchanged_after_failed_reinit` | Admin slot is immutable after init |

### Parameter abuse (`param_abuse`)

| Test | Assumption validated |
|------|----------------------|
| `param_abuse_set_param_without_role_panics` | Caller without `GOVERNANCE_ADMIN` cannot set params |
| `param_abuse_get_all_params_returns_complete_set` | All canonical keys present with correct defaults |
| `param_abuse_governance_admin_can_update_param` | Authorized governor CAN update params |
| `param_abuse_negative_value_always_rejected` | Negative values rejected even from authorized callers |

## CI integration

The `Security Regression Suite` workflow (`.github/workflows/security-regression.yml`) runs:

- On every PR touching security-sensitive contracts
- On every push to `main`
- Nightly at 02:00 UTC

Each threat category runs as a **separate named step** so the failing category
is immediately visible in the Actions UI without reading logs. On failure the
bot posts a PR comment identifying the regression.

Log artifacts are retained for 30 days at:
`security-regression-logs-<sha>/`

## Adding a new test

1. Identify the threat category tag (`priv_esc`, `replay`, etc.).
2. Name the test `<tag>_<short_description>`.
3. Add it to `tests/security_regression.rs` in the matching section.
4. If it covers a contract not yet listed in the section header, add the contract
   to the CI `paths:` filter in `security-regression.yml`.
5. Update this document's coverage table.

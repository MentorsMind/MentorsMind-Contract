# Runbooks

This document collects the main operational procedures for contract maintenance.

## Escrow Release Runbook

1. Confirm the escrow is in a releasable state.
2. Verify the caller has the expected authorization or that the auto-release window has passed.
3. Check that the configured token is still approved.
4. Release funds and confirm the expected event was emitted.

## Dispute Runbook

1. Confirm the dispute reason and timeline are recorded.
2. Check whether the dispute is governed by manual resolution or an automated rule.
3. Validate the final split before calling the resolution entrypoint.
4. Verify the contract emits the resolution event and updates the escrow status.

## Upgrade Runbook

1. Review the target version, changelog, and compatibility impact.
2. Build and test the new WASM artifact locally.
3. Register the upgrade and confirm the latest version is updated.
4. Verify downstream subscribers or indexers received the upgrade event.

## Incident Response

1. Contain the issue by pausing dependent automation if needed.
2. Capture the failing transaction, caller, and contract version.
3. Compare expected storage state with on-chain state.
4. Apply the smallest safe fix, then re-run the relevant tests before redeploying.

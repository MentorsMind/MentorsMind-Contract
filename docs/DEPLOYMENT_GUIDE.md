# Deployment Guide

This guide documents deployment, initialization, and environment configuration for MentorMinds Soroban contracts.

## Scope

Primary deployment targets:
- `testnet`: integration and staging validation
- `mainnet`: production

Primary deployed contracts:
- `escrow`
- `verification`
- `mnt_token`

## Prerequisites

- Rust toolchain with `wasm32-unknown-unknown`
- Stellar CLI (`stellar`)
- `jq`, `curl`
- A configured Stellar identity:
  - `stellar keys generate <identity>`
  - `stellar keys address <identity>`

## Deployment Scripts

Available scripts in `scripts/`:
- `deploy.sh`: full deployment + initialization + verification (recommended)
- `deploy-escrow.sh`: focused escrow deployment helper
- `deploy-multisig.sh`: multisig deployment helper
- `upgrade.sh`: upgrade path utilities

## Testnet Deployment

1. Verify tooling:
```bash
stellar version
cargo --version
jq --version
```

2. Deploy all contracts:
```bash
./scripts/deploy.sh --network testnet --identity default
```

3. Optional custom initialization:
```bash
./scripts/deploy.sh \
  --network testnet \
  --identity default \
  --fee-bps 300 \
  --auto-release-delay-secs 172800 \
  --approved-tokens '[]'
```

4. Confirm output in `deployed/testnet.json`.

## Mainnet Deployment

1. Prepare mainnet RPC access:
```bash
export VALIDATION_CLOUD_KEY=<your-key>
```

2. Dry-run on testnet with the exact parameters you will use in production.

3. Deploy to mainnet:
```bash
./scripts/deploy.sh \
  --network mainnet \
  --identity production \
  --validation-cloud-key "$VALIDATION_CLOUD_KEY"
```

4. Validate deployed IDs in `deployed/mainnet.json` and run post-deploy checks.

## Initialization Steps

`deploy.sh` initializes contracts automatically unless `--skip-init` is set.

Escrow initialization parameters:
- `admin`: deployer identity address
- `treasury`: treasury wallet (defaults to deployer)
- `fee_bps`: platform fee in basis points
- `approved_tokens`: JSON array string for allowlisted tokens
- `auto_release_delay_secs`: auto-release timeout in seconds

Verification + token initialization:
- `verification.initialize(admin)`
- `mnt_token.initialize(admin)`

## Configuration Options

`./scripts/deploy.sh --help` supports:

- `--network <testnet|mainnet>`
- `--identity <name>`
- `--rpc-url <url>`
- `--validation-cloud-key <key>`
- `--fee-bps <u32>`
- `--auto-release-delay-secs <u64>`
- `--treasury <address>`
- `--approved-tokens <json-array>`
- `--skip-build`
- `--skip-fund`
- `--skip-init`
- `--skip-verify`
- `--force-redeploy`

## Deployment Checklist

Before deploy:
- [ ] Branch merged and release tag prepared
- [ ] Unit/integration tests pass
- [ ] `cargo build --release` succeeds
- [ ] Signer/identity access verified
- [ ] Treasury/admin addresses verified
- [ ] Parameter set reviewed (`fee_bps`, delay, approved tokens)

During deploy:
- [ ] `deploy.sh` completed without errors
- [ ] Contract IDs captured in `deployed/<network>.json`
- [ ] Initialization steps succeeded
- [ ] Verification calls returned expected values

After deploy:
- [ ] Contract IDs documented for backend/frontend config
- [ ] Basic read/write smoke checks executed
- [ ] Monitoring + alerts enabled
- [ ] Rollback/upgrade plan documented for this release

## Troubleshooting Quick Hits

- Friendbot failures on testnet: re-run with `--skip-fund` if account is already funded.
- Existing config but needing fresh contract IDs: run with `--force-redeploy`.
- No mainnet RPC configured: pass `--rpc-url` or `--validation-cloud-key`.

For detailed error handling, see `docs/TROUBLESHOOTING.md`.

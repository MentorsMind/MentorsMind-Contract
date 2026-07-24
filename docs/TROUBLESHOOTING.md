# Troubleshooting Guide

Common developer and operator issues for MentorMinds contracts.

## Deployment Errors

### `cannot open '.git/FETCH_HEAD': Operation not permitted`
Cause:
- insufficient filesystem permission while updating git metadata

Fix:
- rerun pull with required permissions in your local environment

### `Unknown arg` in `scripts/deploy.sh`
Cause:
- unsupported or misspelled deploy flag

Fix:
- run `./scripts/deploy.sh --help`
- use only documented options

### `mainnet deploy requires --rpc-url or --validation-cloud-key`
Cause:
- no usable mainnet RPC endpoint configured

Fix:
- pass `--rpc-url` explicitly, or set `--validation-cloud-key` / `VALIDATION_CLOUD_KEY`

### `unable to resolve identity` from Stellar CLI
Cause:
- identity not configured in local Stellar CLI

Fix:
```bash
stellar keys generate <identity>
stellar keys address <identity>
```

### Build fails (`cargo build --target wasm32-unknown-unknown`)
Cause:
- missing target or toolchain mismatch

Fix:
```bash
rustup target add wasm32-unknown-unknown
cargo build --target wasm32-unknown-unknown --release
```

## Runtime / Contract Issues

### Contract reports `Already initialized`
Cause:
- initialization is idempotent guarded; contract already initialized

Fix:
- expected for repeat runs
- use `--skip-init` for validation-only or re-check workflows

### Verification call returns unexpected value
Cause:
- stale contract IDs in `deployed/<network>.json`
- wrong network/identity selected

Fix:
- verify `--network` and `--identity`
- redeploy with `--force-redeploy` when appropriate

### Escrow did not auto-release
Cause:
- auto release delay not yet elapsed
- incorrect delay configuration during initialization

Fix:
- inspect escrow timestamps and configured `auto_release_delay_secs`
- reinitialize only on fresh deployment with correct values

## Debugging Tips

- Validate deploy script syntax:
```bash
bash -n scripts/deploy.sh
```

- Inspect saved deployment metadata:
```bash
cat deployed/testnet.json
cat deployed/mainnet.json
```

- Check contract events:
```bash
stellar events --network testnet --id <contract_id>
```

- Invoke read methods directly:
```bash
stellar contract invoke --network testnet --source default --id <escrow_id> -- get_fee_bps
```

## Support Resources

- Project issues: https://github.com/MentorsMind/MentorsMind-Contract/issues
- Soroban docs: https://soroban.stellar.org/docs
- Stellar developer Discord: https://discord.gg/stellardev

If reporting a bug, include:
- network (`testnet`/`mainnet`)
- command used
- full error output
- commit SHA
- deployment config used (`fee_bps`, delay, treasury)

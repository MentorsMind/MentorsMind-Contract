# FAQ

## What is the recommended deployment path?
Use `scripts/deploy.sh` for both testnet and mainnet. It handles build, deployment, initialization, and verification.

## Where are contract IDs stored?
In `deployed/<network>.json` (`deployed/testnet.json` or `deployed/mainnet.json`).

## Can I rerun deploy safely?
Yes. Existing contract IDs are reused by default. Use `--force-redeploy` to force fresh deployments.

## How do I customize escrow initialization?
Use:
- `--fee-bps`
- `--auto-release-delay-secs`
- `--treasury`
- `--approved-tokens`

## How do I skip expensive or risky steps during checks?
Use:
- `--skip-build`
- `--skip-fund`
- `--skip-init`
- `--skip-verify`

## Why does mainnet deployment fail immediately?
Mainnet requires a valid RPC endpoint. Pass `--rpc-url` or `--validation-cloud-key`.

## What should be tested after deployment?
- contract IDs saved
- `get_fee_bps` read call succeeds
- verification contract read call succeeds
- key escrow lifecycle actions succeed in a smoke test

## Where are architecture and state machine docs?
- `ARCHITECTURE.md`
- `docs/STATE_MACHINE.md`
- `contracts/escrow/INVARIANTS.md`

## Where do I get help?
Open a GitHub issue with command, network, error logs, and commit SHA. See `docs/TROUBLESHOOTING.md` for the full checklist.

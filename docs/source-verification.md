# Source Verification Workflow

This document describes how to verify that a deployed Soroban contract on Stellar
was compiled from a specific source code commit — providing cryptographic
assurance that the on-chain WASM matches the audited source.

## Overview

When a contract is deployed to Stellar, only its WASM hash is visible on-chain.
There is no native record linking that hash back to the source code commit that
produced it. This means users, integrators, and auditors cannot easily verify
that the deployed contract matches a known version of the source code.

The MentorsMind source verification system bridges this gap:

1. **Registry on-chain**: The `upgrade_registry` contract stores a git commit
   hash alongside each contract upgrade, creating an on-chain record of which
   source version produced each deployed WASM.
2. **Deterministic builds**: The Docker reproducible build environment produces
   bit-identical WASM output across machines for the same commit.
3. **Verification script**: A CLI tool builds the contract from a given commit
   and compares the resulting WASM hash against the on-chain hash.

## Components

### 1. Upgrade Registry (`contracts/upgrade_registry/src/lib.rs`)

The upgrade registry contract now includes two new functions:

#### `register_source_commit`

Registers the git commit hash that produced the WASM for a specific contract
version. Admin only.

```rust
pub fn register_source_commit(
    env: Env,
    admin: Address,
    contract_name: Symbol,
    version: u32,
    commit_hash: BytesN<32>,
) -> Result<(), Error>
```

- `admin` — Must match the stored admin address.
- `contract_name` — e.g., `"escrow"`, `"kyc_registry"`.
- `version` — The contract version (should match `UpgradeRecord.new_version`).
- `commit_hash` — First 32 bytes of the git commit's SHA-256 (or zero-padded
  git SHA-1).

Emits a `(source, commit, contract_name)` event with the version and hash.

Fails with:
- `NotInitialized` if the registry hasn't been initialized.
- `NotAdmin` if the caller is not the stored admin.
- `SourceCommitAlreadyRegistered` if a commit hash is already registered for
  this contract name.

#### `get_source_commit`

Retrieves the registered commit hash for a contract.

```rust
pub fn get_source_commit(
    env: Env,
    contract_name: Symbol,
    version: u32,
) -> Option<BytesN<32>>
```

Returns `Some(hash)` if a commit hash has been registered, or `None` otherwise.

### 2. Verification Script (`source_verification/verify_wasm_source.sh`)

A shell script that performs the full verification workflow:

```
./source_verification/verify_wasm_source.sh \
    --contract escrow \
    --commit abc123def456... \
    --rpc-url https://soroban-rpc.example.com \
    [--network testnet] \
    [--contract-id CAF...]
```

#### What it does:

1. **Clones the source** at the specified commit into a temporary directory.
2. **Builds the contract** with deterministic flags (`--remap-path-prefix`,
   release mode, no incremental compilation).
3. **Optimizes the WASM** with `wasm-opt -Oz` for full determinism.
4. **Computes the WASM hash** via `sha256sum`.
5. **Queries the on-chain WASM hash** via the Stellar Soroban RPC
   (`getContractCode`).
6. **Compares** the two hashes and exits with:
   - `0` (success) if they match.
   - `1` (failure) if they don't match.

#### Prerequisites:

- `git`, `curl`, `jq`, `sha256sum` (or `shasum` on macOS)
- Rust toolchain with `wasm32-unknown-unknown` target
- `wasm-opt` from [binaryen](https://github.com/WebAssembly/binaryen)
  (recommended for deterministic output)
- `soroban-cli` (optional, for Soroban contract hash computation)

### 3. Docker Reproducible Build (`docker/reproducible_build/Dockerfile`)

A Docker image that ensures deterministic WASM output across different machines.

#### Building the Docker image

```bash
docker build \
    -t mentorminds/reproducible-builder \
    -f docker/reproducible_build/Dockerfile \
    .
```

#### Using the Docker image

```bash
# Build a specific contract
docker run --rm \
    -v /path/to/mentorminds-contract:/repo \
    -v /path/to/output:/output \
    mentorminds/reproducible-builder \
    /repo /output escrow

# Build all contracts
docker run --rm \
    -v /path/to/mentorminds-contract:/repo \
    -v /path/to/output:/output \
    mentorminds/reproducible-builder \
    /repo /output
```

#### Determinism guarantees

The Docker image provides:

1. **Pinned Rust version** (1.79.0) — the Rust compiler version affects code
   generation.
2. **Remapped paths** — `RUSTFLAGS="--remap-path-prefix=..."` strips local
   filesystem paths from the WASM debug sections.
3. **Disabled incremental compilation** — `CARGO_INCREMENTAL=0` ensures the
   same build always produces the same output.
4. **`wasm-opt -Oz`** — post-processing optimizes the WASM in a deterministic
   way, removing any remaining non-determinism from the linker.
5. **Pinned soroban-cli** — ensures the SDK versions match the CI build.

## Verification Workflow

### Step 1: Deploy a new version

When upgrading a contract via the `upgrade_registry`:

```bash
# After the upgrade is executed, register the source commit
soroban contract invoke \
    --id <upgrade_registry_id> \
    --source <admin_secret> \
    --network testnet \
    -- \
    register_source_commit \
    --admin <admin_address> \
    --contract_name escrow \
    --version 2 \
    --commit_hash <bytesn_32>
```

### Step 2: Verify a deployed contract

```bash
# Verify the escrow contract against a known commit
./source_verification/verify_wasm_source.sh \
    --contract escrow \
    --commit a1b2c3d4e5f6... \
    --rpc-url https://soroban-testnet.stellar.org \
    --network testnet \
    --contract-id CAF... \
    --verbose
```

### Step 3: Audit / CI integration

For automated verification in CI:

```bash
# Using the Docker build environment for full determinism
docker run --rm \
    -v $(pwd):/repo \
    -v $(pwd)/output:/output \
    mentorminds/reproducible-builder \
    /repo /output escrow

# Compare with deployed hash
LOCAL_HASH=$(sha256sum output/escrow.wasm | cut -d' ' -f1)
ON_CHAIN_HASH=$(curl -s -X POST <rpc_url> \
    -H "Content-Type: application/json" \
    -d '{ "jsonrpc":"2.0","id":1,"method":"getContractCode","params":{"contractId":"<id>"} }' \
    | jq -r '.result.hash')

if [ "$LOCAL_HASH" = "$ON_CHAIN_HASH" ]; then
    echo "✓ Verification passed"
else
    echo "✗ Verification failed"
    exit 1
fi
```

## Security Considerations

1. **Trust in the admin**: The `register_source_commit` function is admin-gated.
   Only the contract admin can register a commit hash. This means the security
   of the source verification system depends on the security of the admin key.

2. **Deterministic builds are critical**: If the build environment produces
   different WASM output from the same source, verification will fail even
   though the source is correct. The Docker image provides a deterministic
   build environment, but differences in Rust versions, soroban SDK versions,
   or system dependencies can still cause mismatches.

3. **First 32 bytes of SHA-256**: The `commit_hash` parameter stores the first
   32 bytes of the git commit's SHA-256 hash. Git uses SHA-1 for commit IDs,
   so in practice this means zero-padding the 20-byte SHA-1 to 32 bytes, or
   using SHA-256-based commit references if the repository has transitioned.

4. **Single commit per contract name**: The current implementation stores one
   commit hash per contract name. If multiple versions of the same contract are
   deployed, only the most recently registered commit is stored. A future
   enhancement could use a composite key `(contract_name, version)` for
   per-version tracking.

## Troubleshooting

| Symptom | Likely Cause | Solution |
|---------|-------------|----------|
| "SourceCommitAlreadyRegistered" | Commit already registered for this contract | Use a different contract name or modify the registry to support re-registration |
| "NotAdmin" | Caller is not the stored admin | Check the admin address with `get_admin()` |
| WASM hash mismatch | Non-deterministic build | Use the Docker build environment; ensure same Rust/soroban versions |
| "Failed to retrieve on-chain WASM hash" | Invalid contract ID or RPC URL | Verify the contract ID and RPC endpoint |
| WASM file not found | Contract package name differs from directory name | Use `--wasm-dir` or check the actual package name in `Cargo.toml` |

## Future Enhancements

- **Per-version commit tracking**: Store commit hashes keyed by
  `(contract_name, version)` instead of just `contract_name`.
- **Automated CI verification**: A GitHub Actions workflow that automatically
  verifies deployed contracts on each release.
- **Verification dashboard**: A web interface that displays the verification
  status of all deployed contracts.
- **Multi-signature commit registration**: Allow the M-of-N signer set to
  register source commits, not just the admin.


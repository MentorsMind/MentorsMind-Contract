# =============================================================================
# Dockerfile — Deterministic Soroban Contract Build Environment
#
# Produces bit-identical WASM binaries across different machines for the same
# source commit. This is the foundation of the source verification workflow.
#
# Build:
#   docker build -t mentorminds/reproducible-builder -f Dockerfile .
#
# Usage:
#   docker run --rm \
#       -v /path/to/repo:/repo \
#       -v /path/to/output:/output \
#       mentorminds/reproducible-builder \
#       /repo /output [contract_name]
#
# The output directory will contain the built .wasm file(s).
# =============================================================================

FROM rust:1.79.0-slim-bookworm AS builder

# Prevent interactive prompts during package installation
ENV DEBIAN_FRONTEND=noninteractive
ENV CARGO_INCREMENTAL=0
ENV RUST_BACKTRACE=1

# ─── System dependencies ────────────────────────────────────────────────────
RUN apt-get update && apt-get install -y --no-install-recommends \
    # Build essentials
    build-essential \
    pkg-config \
    libssl-dev \
    curl \
    ca-certificates \
    git \
    # WASM optimization tools (binaryen provides wasm-opt)
    binaryen \
    # For hash verification
    coreutils \
    # For RPC queries in the verification script
    jq \
    && rm -rf /var/lib/apt/lists/*

# ─── Rust WASM target & Soroban CLI ────────────────────────────────────────
RUN rustup target add wasm32-unknown-unknown

# Install a pinned version of soroban-cli for deterministic builds
RUN cargo install soroban-cli --version 25.3.0 --locked

# ─── Deterministic build configuration ──────────────────────────────────────
# These environment variables ensure that:
# 1. File paths are remapped to prevent local path leakage into the WASM
# 2. Debug symbols are stripped (release profile handles this)
# 3. Incremental compilation is disabled (ensures deterministic output)
# 4. The Rust version is pinned (via the base image tag)
ENV RUSTFLAGS="--remap-path-prefix=/repo=. --remap-path-prefix=$CARGO_HOME=~"
ENV CARGO_TARGET_DIR=/build/target

# ─── Entrypoint ─────────────────────────────────────────────────────────────
# Arguments: /path/to/repo /path/to/output [contract_name]
# If contract_name is omitted, builds all workspace contracts.
COPY <<"SCRIPT" /usr/local/bin/build-contract.sh
#!/usr/bin/env bash
set -euo pipefail

REPO_DIR="${1:-/repo}"
OUTPUT_DIR="${2:-/output}"
CONTRACT_NAME="${3:-}"

if [[ ! -d "$REPO_DIR" ]]; then
    echo "[error] Repository directory not found: $REPO_DIR"
    exit 1
fi

mkdir -p "$OUTPUT_DIR"

cd "$REPO_DIR"

echo "[info] Rust version: $(rustc --version)"
echo "[info] Soroban CLI version: $(soroban version 2>/dev/null || echo 'unknown')"
echo "[info] Repo root: $REPO_DIR"

if [[ -n "$CONTRACT_NAME" ]]; then
    echo "[info] Building contract: $CONTRACT_NAME"

    # Determine contract path (can be in contracts/ or repo root)
    if [[ -d "$REPO_DIR/contracts/$CONTRACT_NAME" ]]; then
        CONTRACT_PATH="$REPO_DIR/contracts/$CONTRACT_NAME"
    elif [[ -d "$REPO_DIR/$CONTRACT_NAME" ]]; then
        CONTRACT_PATH="$REPO_DIR/$CONTRACT_NAME"
    else
        echo "[error] Contract '$CONTRACT_NAME' not found"
        exit 1
    fi

    cargo build \
        --target wasm32-unknown-unknown \
        --release \
        --manifest-path "$CONTRACT_PATH/Cargo.toml" \
        2>&1

    # Copy WASM output
    WASM_FILE="$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/${CONTRACT_NAME//-/_}.wasm"
    if [[ -f "$WASM_FILE" ]]; then
        # Optimize with wasm-opt for deterministic output
        wasm-opt -Oz "$WASM_FILE" -o "$OUTPUT_DIR/${CONTRACT_NAME}.wasm" 2>&1
        echo "[info] WASM written to: $OUTPUT_DIR/${CONTRACT_NAME}.wasm"
        echo "[info] SHA256: $(sha256sum "$OUTPUT_DIR/${CONTRACT_NAME}.wasm" | cut -d' ' -f1)"
    else
        echo "[warn] Expected WASM not found at $WASM_FILE, searching..."
        find "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release" -name "*.wasm" -exec cp {} "$OUTPUT_DIR/" \;
        echo "[info] Copied WASM files to $OUTPUT_DIR"
    fi
else
    echo "[info] Building all workspace contracts"
    cargo build \
        --target wasm32-unknown-unknown \
        --release \
        --workspace \
        2>&1

    # Copy all WASM files
    find "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release" -name "*.wasm" -exec cp {} "$OUTPUT_DIR/" \;
    echo "[info] All WASM files copied to $OUTPUT_DIR"
fi

echo "[info] Build complete. Output checksums:"
for f in "$OUTPUT_DIR"/*.wasm; do
    sha256sum "$f"
done
SCRIPT

RUN chmod +x /usr/local/bin/build-contract.sh

ENTRYPOINT ["/usr/local/bin/build-contract.sh"]


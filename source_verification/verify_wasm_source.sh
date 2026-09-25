#!/usr/bin/env bash
# =============================================================================
# verify_wasm_source.sh — Soroban Source Verification Script
#
# Builds a Soroban contract from a specific git commit and compares the
# resulting WASM hash against the on-chain WASM hash via the Stellar RPC.
#
# Usage:
#   ./source_verification/verify_wasm_source.sh \
#       --contract escrow \
#       --commit abc123def456... \
#       --rpc-url https://soroban-rpc.example.com \
#       [--network testnet] \
#       [--contract-id CAF...] \
#       [--wasm-dir target/wasm32-unknown-unknown/release] \
#       [--verbose]
#
# Requirements:
#   - Git
#   - Rust toolchain (rustup, cargo, soroban-cli)
#   - wasm-opt (from binaryen) for deterministic WASM processing
#   - sha256sum (or shasum on macOS)
#   - curl, jq (for RPC queries)
# =============================================================================

set -euo pipefail

# ─── Color helpers ──────────────────────────────────────────────────────────
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
NC='\033[0m' # No Color

info()    { echo -e "${GREEN}[info]${NC} $*"; }
warn()    { echo -e "${YELLOW}[warn]${NC} $*"; }
error()   { echo -e "${RED}[error]${NC} $*"; }
verbose() { [[ -n "$VERBOSE" ]] && echo -e "[debug] $*"; }

# ─── Defaults ───────────────────────────────────────────────────────────────
REPO_DIR="$(cd "$(dirname "$0")/.." && pwd)"
WORK_DIR=$(mktemp -d /tmp/source-verify-XXXXXX)
CONTRACT=""
COMMIT_HASH=""
RPC_URL=""
NETWORK="testnet"
CONTRACT_ID=""
WASM_DIR="target/wasm32-unknown-unknown/release"
VERBOSE=""
CLEANUP="true"

# ─── Parse arguments ────────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
    case "$1" in
        --contract)       CONTRACT="$2";       shift 2 ;;
        --commit)         COMMIT_HASH="$2";    shift 2 ;;
        --rpc-url)        RPC_URL="$2";        shift 2 ;;
        --network)        NETWORK="$2";        shift 2 ;;
        --contract-id)    CONTRACT_ID="$2";    shift 2 ;;
        --wasm-dir)       WASM_DIR="$2";       shift 2 ;;
        --verbose)        VERBOSE="true";      shift ;;
        --no-cleanup)     CLEANUP="false";     shift ;;
        --help)
            echo "Usage: $0 --contract <name> --commit <hash> --rpc-url <url> [options]"
            echo ""
            echo "Options:"
            echo "  --contract <name>       Contract name (e.g., escrow, kyc_registry)"
            echo "  --commit <hash>         Git commit hash (full SHA or first 32 bytes)"
            echo "  --rpc-url <url>         Stellar Soroban RPC endpoint"
            echo "  --network <network>     Stellar network passphrase (default: testnet)"
            echo "  --contract-id <id>      Contract address (optional, overrides automatic lookup)"
            echo "  --wasm-dir <dir>        WASM output directory relative to repo root"
            echo "  --verbose               Enable verbose logging"
            echo "  --no-cleanup            Keep temporary files for debugging"
            echo "  --help                  Show this help message"
            exit 0
            ;;
        *) error "Unknown argument: $1"; exit 1 ;;
    esac
done

# ─── Validation ─────────────────────────────────────────────────────────────
if [[ -z "$CONTRACT" ]]; then
    error "Missing required argument: --contract"
    exit 1
fi

if [[ -z "$COMMIT_HASH" ]]; then
    error "Missing required argument: --commit"
    exit 1
fi

if [[ -z "$RPC_URL" ]]; then
    error "Missing required argument: --rpc-url"
    exit 1
fi

# ─── Cleanup trap ───────────────────────────────────────────────────────────
cleanup() {
    if [[ "$CLEANUP" == "true" ]]; then
        verbose "Cleaning up temporary directory: $WORK_DIR"
        rm -rf "$WORK_DIR"
    else
        info "Temporary files preserved at: $WORK_DIR"
    fi
}
trap cleanup EXIT

# ─── Step 1: Check out the source code at the given commit ──────────────────
info "Checking out commit ${COMMIT_HASH} into ${WORK_DIR}..."

if [[ ! -d "$REPO_DIR/.git" ]]; then
    error "Not a git repository: $REPO_DIR"
    exit 1
fi

# Use git clone with a shallow fetch at the specific commit for speed
git clone --no-checkout "$REPO_DIR" "$WORK_DIR/repo" 2>&1 | verbose || {
    error "Failed to clone repository"
    exit 1
}

cd "$WORK_DIR/repo"
git fetch origin "$COMMIT_HASH" 2>&1 | verbose || {
    error "Failed to fetch commit $COMMIT_HASH. Ensure the commit exists and is reachable."
    exit 1
}

git checkout "$COMMIT_HASH" 2>&1 | verbose || {
    error "Failed to checkout commit $COMMIT_HASH"
    exit 1
}

info "Checked out commit $(git rev-parse --short HEAD)"

# ─── Step 2: Build the contract WASM ────────────────────────────────────────
info "Building contract '${CONTRACT}' from source..."

# Determine the contract path
CONTRACT_PATH=""
if [[ -d "$WORK_DIR/repo/contracts/$CONTRACT" ]]; then
    CONTRACT_PATH="$WORK_DIR/repo/contracts/$CONTRACT"
elif [[ -d "$WORK_DIR/repo/$CONTRACT" ]]; then
    CONTRACT_PATH="$WORK_DIR/repo/$CONTRACT"
else
    error "Contract '${CONTRACT}' not found in contracts/ or repo root"
    exit 1
fi

verbose "Contract path: $CONTRACT_PATH"

cd "$WORK_DIR/repo"

# Build with deterministic flags:
# - RUSTFLAGS="--remap-path-prefix" to strip local paths
# - Use --release for optimized WASM
export RUSTFLAGS="--remap-path-prefix=$HOME=~ --remap-path-prefix=$WORK_DIR/repo=."
export CARGO_TARGET_DIR="$WORK_DIR/target"
export CARGO_INCREMENTAL=0

# Optional: pin the Rust version for determinism
verbose "Building with Rust $(rustc --version)"

cargo build \
    --target wasm32-unknown-unknown \
    --release \
    --manifest-path "$CONTRACT_PATH/Cargo.toml" \
    2>&1 | verbose

if [[ $? -ne 0 ]]; then
    error "WASM build failed for contract '${CONTRACT}' at commit ${COMMIT_HASH}"
    exit 1
fi

WASM_FILE="$CARGO_TARGET_DIR/wasm32-unknown-unknown/release/${CONTRACT//-/_}.wasm"
if [[ ! -f "$WASM_FILE" ]]; then
    # Try alternate naming: the package name in Cargo.toml may differ
    warn "Expected WASM at $WASM_FILE, searching..."
    WASM_FILE=$(find "$CARGO_TARGET_DIR/wasm32-unknown-unknown/release" -name "*.wasm" -print -quit 2>/dev/null || true)
    if [[ -z "$WASM_FILE" ]]; then
        error "No WASM file found in build output"
        exit 1
    fi
    info "Found WASM: $WASM_FILE"
fi

# Optional: Optimize WASM with wasm-opt for deterministic output
if command -v wasm-opt &>/dev/null; then
    verbose "Optimizing WASM with wasm-opt..."
    wasm-opt -Oz "$WASM_FILE" -o "$WASM_FILE.opt" 2>&1 | verbose
    mv "$WASM_FILE.opt" "$WASM_FILE"
else
    warn "wasm-opt not found. WASM may not be fully deterministic across machines."
fi

# Compute the local WASM hash
LOCAL_WASM_HASH=$(sha256sum "$WASM_FILE" | cut -d' ' -f1)
info "Local WASM SHA256: $LOCAL_WASM_HASH"

# Also compute Soroban-style hash (contract hash, if soroban CLI is available)
LOCAL_CONTRACT_HASH=""
if command -v soroban &>/dev/null; then
    verbose "Computing Soroban contract hash..."
    LOCAL_CONTRACT_HASH=$(soroban lab hash --wasm "$WASM_FILE" 2>/dev/null || true)
    if [[ -n "$LOCAL_CONTRACT_HASH" ]]; then
        info "Soroban contract hash: $LOCAL_CONTRACT_HASH"
    fi
else
    warn "soroban CLI not found. Will rely on on-chain WASM hash query."
fi

# ─── Step 3: Query on-chain WASM hash via Stellar RPC ───────────────────────
info "Querying on-chain WASM hash from ${RPC_URL}..."

# If a contract ID was provided, look up its WASM hash via the RPC
if [[ -n "$CONTRACT_ID" ]]; then
    verbose "Using provided contract ID: $CONTRACT_ID"
else
    # Try to derive the contract ID from the upgrade registry or deployment artifacts
    DEPLOYED_FILE="$REPO_DIR/deployed/${NETWORK}.json"
    if [[ -f "$DEPLOYED_FILE" ]]; then
        CONTRACT_ID=$(jq -r ".${CONTRACT}.contract_id // empty" "$DEPLOYED_FILE" 2>/dev/null || true)
        if [[ -n "$CONTRACT_ID" ]]; then
            info "Found contract ID in deployed/${NETWORK}.json: $CONTRACT_ID"
        fi
    fi
fi

if [[ -z "$CONTRACT_ID" ]]; then
    error "No contract ID provided or found. Use --contract-id to specify it."
    exit 1
fi

# Query the RPC for the contract's WASM hash
# Stellar Soroban RPC method: getContractCode
RPC_PAYLOAD=$(cat <<EOF
{
    "jsonrpc": "2.0",
    "id": 1,
    "method": "getContractCode",
    "params": {
        "contractId": "$CONTRACT_ID"
    }
}
EOF
)

RPC_RESPONSE=$(curl -s -X POST "$RPC_URL" \
    -H "Content-Type: application/json" \
    -d "$RPC_PAYLOAD" 2>&1)

verbose "RPC response: $RPC_RESPONSE"

ON_CHAIN_WASM_HASH=$(echo "$RPC_RESPONSE" | jq -r '.result.hash // empty' 2>/dev/null || true)

if [[ -z "$ON_CHAIN_WASM_HASH" ]]; then
    error "Failed to retrieve on-chain WASM hash from RPC"
    error "Response: $RPC_RESPONSE"
    exit 1
fi

info "On-chain WASM hash: $ON_CHAIN_WASM_HASH"

# ─── Step 4: Compare hashes ─────────────────────────────────────────────────
echo ""
echo "═══════════════════════════════════════════════════════════════"
echo "                   VERIFICATION RESULT"
echo "═══════════════════════════════════════════════════════════════"
echo " Contract:       $CONTRACT"
echo " Commit:         $COMMIT_HASH"
echo " Network:        $NETWORK"
echo " Contract ID:    $CONTRACT_ID"
echo ""
echo " Local WASM hash:    $LOCAL_WASM_HASH"
echo " On-chain WASM hash: $ON_CHAIN_WASM_HASH"
echo ""

if [[ "$LOCAL_WASM_HASH" == "$ON_CHAIN_WASM_HASH" ]]; then
    echo -e " ${GREEN}✓ MATCH${NC} — The deployed contract matches the source code."
    echo "═══════════════════════════════════════════════════════════════"
    exit 0
else
    echo -e " ${RED}✗ MISMATCH${NC} — The deployed contract does NOT match the source code."
    echo ""
    if command -v soroban &>/dev/null && [[ -n "$LOCAL_CONTRACT_HASH" ]]; then
        echo " Soroban contract hash: $LOCAL_CONTRACT_HASH"
        if [[ "$LOCAL_CONTRACT_HASH" == "$ON_CHAIN_WASM_HASH" ]]; then
            echo -e " ${GREEN}✓ But Soroban contract hash matches!${NC}"
            echo "═══════════════════════════════════════════════════════════════"
            exit 0
        fi
    fi
    echo "═══════════════════════════════════════════════════════════════"
    exit 1
fi


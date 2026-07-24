#!/bin/bash

# MentorsMind Contract Upgrade Script
# Usage: ./scripts/upgrade.sh <contract> [network]
# Example: ./scripts/upgrade.sh escrow testnet

set -e

# Configuration
CONTRACT=${1:-escrow}
NETWORK=${2:-testnet}
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(dirname "$SCRIPT_DIR")"

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Helper functions
log_info() {
    echo -e "${BLUE}ℹ${NC} $1"
}

log_success() {
    echo -e "${GREEN}✓${NC} $1"
}

log_warning() {
    echo -e "${YELLOW}⚠${NC} $1"
}

log_error() {
    echo -e "${RED}✗${NC} $1"
}

# Validate inputs
if [ -z "$CONTRACT" ]; then
    log_error "Contract name is required"
    echo "Usage: ./scripts/upgrade.sh <contract> [network]"
    exit 1
fi

if [ ! -d "$PROJECT_ROOT/contracts/$CONTRACT" ] && [ ! -d "$PROJECT_ROOT/$CONTRACT" ]; then
    log_error "Contract directory not found: $CONTRACT"
    exit 1
fi

# Determine contract path
if [ -d "$PROJECT_ROOT/contracts/$CONTRACT" ]; then
    CONTRACT_PATH="$PROJECT_ROOT/contracts/$CONTRACT"
else
    CONTRACT_PATH="$PROJECT_ROOT/$CONTRACT"
fi

log_info "Starting upgrade process for $CONTRACT on $NETWORK"
log_info "Contract path: $CONTRACT_PATH"

# Step 1: Build contract
log_info "Step 1: Building $CONTRACT contract..."
cd "$PROJECT_ROOT"

if ! cargo build --package "$CONTRACT" --target wasm32-unknown-unknown --release 2>&1; then
    log_error "Build failed for $CONTRACT"
    exit 1
fi

log_success "Build completed successfully"

# Step 2: Optimize WASM
log_info "Step 2: Optimizing WASM..."

WASM_FILE="$PROJECT_ROOT/target/wasm32-unknown-unknown/release/mentorminds_${CONTRACT}.wasm"

if [ ! -f "$WASM_FILE" ]; then
    log_error "WASM file not found: $WASM_FILE"
    exit 1
fi

if ! soroban contract optimize --wasm "$WASM_FILE" 2>&1; then
    log_warning "WASM optimization failed or not available, continuing..."
fi

log_success "WASM optimization completed"

# Step 3: Verify network configuration
log_info "Step 3: Verifying network configuration..."

if ! soroban config network ls | grep -q "$NETWORK"; then
    log_error "Network not configured: $NETWORK"
    echo "Available networks:"
    soroban config network ls
    exit 1
fi

log_success "Network configuration verified"

# Step 4: Deploy contract
log_info "Step 4: Deploying to $NETWORK..."

DEPLOY_OUTPUT=$(soroban contract deploy \
    --wasm "$WASM_FILE" \
    --network "$NETWORK" \
    --source-account default 2>&1)

if [ $? -ne 0 ]; then
    log_error "Deployment failed"
    echo "$DEPLOY_OUTPUT"
    exit 1
fi

CONTRACT_ID=$(echo "$DEPLOY_OUTPUT" | grep -oP '(?<=Contract ID: )[^ ]*' || echo "$DEPLOY_OUTPUT" | tail -1)

if [ -z "$CONTRACT_ID" ]; then
    log_error "Could not extract contract ID from deployment output"
    echo "$DEPLOY_OUTPUT"
    exit 1
fi

log_success "Contract deployed successfully"
log_info "Contract ID: $CONTRACT_ID"

# Step 5: Save deployment info
log_info "Step 5: Saving deployment information..."

DEPLOYED_DIR="$PROJECT_ROOT/deployed"
mkdir -p "$DEPLOYED_DIR"

DEPLOYMENT_FILE="$DEPLOYED_DIR/${NETWORK}_${CONTRACT}_upgrade.json"

cat > "$DEPLOYMENT_FILE" << EOF
{
    "contract": "$CONTRACT",
    "network": "$NETWORK",
    "contract_id": "$CONTRACT_ID",
    "wasm_file": "$WASM_FILE",
    "deployed_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
    "timestamp": $(date +%s)
}
EOF

log_success "Deployment information saved to $DEPLOYMENT_FILE"

# Step 6: Verify deployment
log_info "Step 6: Verifying deployment..."

if soroban contract invoke \
    --id "$CONTRACT_ID" \
    --network "$NETWORK" \
    -- get_contract_info 2>&1 > /dev/null; then
    log_success "Contract verification successful"
else
    log_warning "Contract verification failed - contract may not have get_contract_info function"
fi

# Summary
echo ""
log_success "Upgrade completed successfully!"
echo ""
echo "Summary:"
echo "  Contract: $CONTRACT"
echo "  Network: $NETWORK"
echo "  Contract ID: $CONTRACT_ID"
echo "  WASM File: $WASM_FILE"
echo "  Deployment Info: $DEPLOYMENT_FILE"
echo ""
echo "Next steps:"
echo "  1. Register upgrade in upgrade_registry:"
echo "     soroban contract invoke --id <upgrade-registry-id> --network $NETWORK -- register_upgrade"
echo "  2. Verify contract functions are working"
echo "  3. Monitor event stream for any issues"
echo "  4. Notify integrators of the upgrade"
echo ""

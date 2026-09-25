#!/bin/bash

# MentorsMind Contract Rollback Script
# Usage: ./scripts/rollback.sh <contract> <old-version> [network]
# Example: ./scripts/rollback.sh escrow v1 testnet

set -e

# Configuration
CONTRACT=${1:-escrow}
OLD_VERSION=${2:-v1}
NETWORK=${3:-testnet}
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
    echo "Usage: ./scripts/rollback.sh <contract> <old-version> [network]"
    exit 1
fi

if [ -z "$OLD_VERSION" ]; then
    log_error "Old version is required"
    echo "Usage: ./scripts/rollback.sh <contract> <old-version> [network]"
    exit 1
fi

log_warning "⚠️  ROLLBACK INITIATED - This will revert to a previous version"
log_info "Contract: $CONTRACT"
log_info "Target Version: $OLD_VERSION"
log_info "Network: $NETWORK"
echo ""

# Confirmation
read -p "Are you sure you want to rollback? (yes/no): " -r
echo ""
if [[ ! $REPLY =~ ^[Yy][Ee][Ss]$ ]]; then
    log_info "Rollback cancelled"
    exit 0
fi

# Step 1: Pause new contract
log_info "Step 1: Pausing new contract..."

# Note: This assumes the new contract has a pause function
# Adjust based on your contract implementation
log_warning "Manual step: Pause the new contract if it has a pause function"

# Step 2: Find old WASM file
log_info "Step 2: Locating old WASM file..."

OLD_WASM_FILE="$PROJECT_ROOT/deployed/${NETWORK}_${CONTRACT}_${OLD_VERSION}.wasm"

if [ ! -f "$OLD_WASM_FILE" ]; then
    log_warning "Old WASM file not found at: $OLD_WASM_FILE"
    log_info "Attempting to build from source..."
    
    # Try to build from git history
    if [ -d "$PROJECT_ROOT/.git" ]; then
        log_info "Checking git history for version $OLD_VERSION..."
        # This would require checking out the old version from git
        log_warning "Manual step: Check out version $OLD_VERSION from git and build"
    fi
    
    exit 1
fi

log_success "Old WASM file found"

# Step 3: Deploy old version
log_info "Step 3: Deploying old version..."

DEPLOY_OUTPUT=$(soroban contract deploy \
    --wasm "$OLD_WASM_FILE" \
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

log_success "Old version deployed successfully"
log_info "Contract ID: $CONTRACT_ID"

# Step 4: Verify rollback
log_info "Step 4: Verifying rollback..."

if soroban contract invoke \
    --id "$CONTRACT_ID" \
    --network "$NETWORK" \
    -- get_contract_info 2>&1 > /dev/null; then
    log_success "Rollback verification successful"
else
    log_warning "Rollback verification failed - contract may not have get_contract_info function"
fi

# Step 5: Save rollback info
log_info "Step 5: Saving rollback information..."

DEPLOYED_DIR="$PROJECT_ROOT/deployed"
mkdir -p "$DEPLOYED_DIR"

ROLLBACK_FILE="$DEPLOYED_DIR/${NETWORK}_${CONTRACT}_rollback.json"

cat > "$ROLLBACK_FILE" << EOF
{
    "contract": "$CONTRACT",
    "network": "$NETWORK",
    "rolled_back_to_version": "$OLD_VERSION",
    "contract_id": "$CONTRACT_ID",
    "wasm_file": "$OLD_WASM_FILE",
    "rolled_back_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
    "timestamp": $(date +%s)
}
EOF

log_success "Rollback information saved to $ROLLBACK_FILE"

# Summary
echo ""
log_success "Rollback completed successfully!"
echo ""
echo "Summary:"
echo "  Contract: $CONTRACT"
echo "  Rolled back to: $OLD_VERSION"
echo "  Network: $NETWORK"
echo "  Contract ID: $CONTRACT_ID"
echo "  Rollback Info: $ROLLBACK_FILE"
echo ""
echo "Next steps:"
echo "  1. Verify all contract functions are working"
echo "  2. Check event stream for any issues"
echo "  3. Notify integrators of the rollback"
echo "  4. Investigate root cause of the issue"
echo "  5. Schedule post-mortem meeting"
echo ""

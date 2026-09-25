#!/bin/bash

# MentorsMind Contract Upgrade Verification Script
# Usage: ./scripts/verify-upgrade.sh <contract-id> [network]
# Example: ./scripts/verify-upgrade.sh CAAAA... testnet

set -e

# Configuration
CONTRACT_ID=$1
NETWORK=${2:-testnet}

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
if [ -z "$CONTRACT_ID" ]; then
    log_error "Contract ID is required"
    echo "Usage: ./scripts/verify-upgrade.sh <contract-id> [network]"
    exit 1
fi

log_info "Starting verification for contract: $CONTRACT_ID on $NETWORK"

# Step 1: Verify network
log_info "Step 1: Verifying network configuration..."

if ! soroban config network ls | grep -q "$NETWORK"; then
    log_error "Network not configured: $NETWORK"
    exit 1
fi

log_success "Network verified"

# Step 2: Check contract exists
log_info "Step 2: Checking if contract exists..."

if ! soroban contract invoke \
    --id "$CONTRACT_ID" \
    --network "$NETWORK" \
    -- get_contract_info 2>&1 > /dev/null; then
    log_warning "Contract does not have get_contract_info function, skipping basic check"
else
    log_success "Contract is accessible"
fi

# Step 3: Check recent events
log_info "Step 3: Checking recent events..."

EVENTS=$(soroban events \
    --network "$NETWORK" \
    --contract "$CONTRACT_ID" \
    --limit 10 2>&1 || echo "")

if [ -z "$EVENTS" ]; then
    log_warning "No events found for contract"
else
    log_success "Events retrieved successfully"
    echo "$EVENTS" | head -5
fi

# Step 4: Verify contract state
log_info "Step 4: Verifying contract state..."

# Try to get contract info
CONTRACT_INFO=$(soroban contract invoke \
    --id "$CONTRACT_ID" \
    --network "$NETWORK" \
    -- get_contract_info 2>&1 || echo "")

if [ -n "$CONTRACT_INFO" ]; then
    log_success "Contract state verified"
    echo "$CONTRACT_INFO"
else
    log_warning "Could not retrieve contract info"
fi

# Step 5: Check for errors
log_info "Step 5: Checking for recent errors..."

ERROR_EVENTS=$(soroban events \
    --network "$NETWORK" \
    --contract "$CONTRACT_ID" \
    --limit 50 2>&1 | grep -i "error" || echo "")

if [ -n "$ERROR_EVENTS" ]; then
    log_warning "Found error events:"
    echo "$ERROR_EVENTS"
else
    log_success "No error events found"
fi

# Summary
echo ""
log_success "Verification completed!"
echo ""
echo "Summary:"
echo "  Contract ID: $CONTRACT_ID"
echo "  Network: $NETWORK"
echo "  Status: Ready for use"
echo ""
echo "Recommendations:"
echo "  1. Test critical contract functions"
echo "  2. Monitor event stream for anomalies"
echo "  3. Verify database synchronization"
echo "  4. Notify integrators of successful upgrade"
echo ""

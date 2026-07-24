#!/bin/bash

# MentorMinds Local Development Environment Setup Script
# This script sets up a complete local Soroban development environment

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
NETWORK_NAME="standalone"
HORIZON_URL="http://localhost:8000"
RPC_URL="http://localhost:8001"
SOROBAN_RPC_URL="http://localhost:8003"
FRIENDBOT_URL="http://localhost:8002"

# Account names and roles
declare -A ACCOUNTS=(
    ["admin"]="Admin account for platform management"
    ["mentor1"]="First mentor account"
    ["mentor2"]="Second mentor account" 
    ["learner1"]="First learner account"
    ["learner2"]="Second learner account"
)

# Function to print colored output
print_status() {
    echo -e "${BLUE}[INFO]${NC} $1"
}

print_success() {
    echo -e "${GREEN}[SUCCESS]${NC} $1"
}

print_warning() {
    echo -e "${YELLOW}[WARNING]${NC} $1"
}

print_error() {
    echo -e "${RED}[ERROR]${NC} $1"
}

# Function to check if Docker is running
check_docker() {
    if ! docker info > /dev/null 2>&1; then
        print_error "Docker is not running. Please start Docker first."
        exit 1
    fi
    print_success "Docker is running"
}

# Function to start Stellar container
start_stellar() {
    print_status "Starting Stellar quickstart container..."
    
    # Stop existing container if running
    if docker ps -q -f name=mentorminds-stellar | grep -q .; then
        print_warning "Stopping existing Stellar container..."
        docker stop mentorminds-stellar || true
        docker rm mentorminds-stellar || true
    fi
    
    # Start new container
    docker-compose up -d
    
    print_status "Waiting for Stellar services to be ready..."
    sleep 10
    
    # Wait for Horizon to be ready
    local max_attempts=30
    local attempt=1
    while [ $attempt -le $max_attempts ]; do
        if curl -s "$HORIZON_URL" > /dev/null 2>&1; then
            print_success "Stellar Horizon is ready"
            break
        fi
        if [ $attempt -eq $max_attempts ]; then
            print_error "Stellar Horizon failed to start after $max_attempts attempts"
            exit 1
        fi
        print_status "Waiting for Horizon... (attempt $attempt/$max_attempts)"
        sleep 2
        ((attempt++))
    done
    
    # Wait for Soroban RPC to be ready
    attempt=1
    while [ $attempt -le $max_attempts ]; do
        if curl -s "$SOROBAN_RPC_URL" > /dev/null 2>&1; then
            print_success "Soroban RPC is ready"
            break
        fi
        if [ $attempt -eq $max_attempts ]; then
            print_error "Soroban RPC failed to start after $max_attempts attempts"
            exit 1
        fi
        print_status "Waiting for Soroban RPC... (attempt $attempt/$max_attempts)"
        sleep 2
        ((attempt++))
    done
}

# Function to configure Soroban CLI
configure_soroban() {
    print_status "Configuring Soroban CLI for local network..."
    
    # Remove existing network configuration if it exists
    soroban config network remove $NETWORK_NAME 2>/dev/null || true
    
    # Add local network configuration
    soroban config network add $NETWORK_NAME \
        --rpc-url $SOROBAN_RPC_URL \
        --network-passphrase "Standalone Network ; February 2017"
    
    print_success "Soroban CLI configured for local network"
}

# Function to create and fund accounts
create_and_fund_accounts() {
    print_status "Creating and funding test accounts..."
    
    local accounts_file="deployed/accounts.json"
    mkdir -p deployed
    
    # Initialize accounts file
    echo "{" > $accounts_file
    echo "  \"network\": \"$NETWORK_NAME\"," >> $accounts_file
    echo "  \"horizon_url\": \"$HORIZON_URL\"," >> $accounts_file
    echo "  \"rpc_url\": \"$SOROBAN_RPC_URL\"," >> $accounts_file
    echo "  \"friendbot_url\": \"$FRIENDBOT_URL\"," >> $accounts_file
    echo "  \"accounts\": {" >> $accounts_file
    
    local first=true
    for account_name in "${!ACCOUNTS[@]}"; do
        if [ "$first" = true ]; then
            first=false
        else
            echo "," >> $accounts_file
        fi
        
        print_status "Creating account: $account_name"
        
        # Generate identity for account
        local identity_name="local_$account_name"
        soroban config identity remove $identity_name 2>/dev/null || true
        soroban config identity generate $identity_name
        
        # Get account address
        local address=$(soroban config identity address $identity_name)
        
        # Fund account using friendbot
        print_status "Funding account $account_name ($address)..."
        curl -X POST "$FRIENDBOT_URL?addr=$address" > /dev/null 2>&1
        
        # Wait a moment for funding to process
        sleep 1
        
        # Check account balance
        local balance=$(curl -s "$HORIZON_URL/accounts/$address" | jq -r '.balances[0].balance' 2>/dev/null || echo "0")
        print_success "Account $account_name funded with $balance XLM"
        
        # Add to accounts file
        echo "    \"$account_name\": {" >> $accounts_file
        echo "      \"address\": \"$address\"," >> $accounts_file
        echo "      \"identity\": \"$identity_name\"," >> $accounts_file
        echo "      \"role\": \"${ACCOUNTS[$account_name]}\"" >> $accounts_file
        echo -n "    }" >> $accounts_file
    done
    
    echo "" >> $accounts_file
    echo "  }" >> $accounts_file
    echo "}" >> $accounts_file
    
    print_success "All accounts created and funded"
}

# Function to build contracts
build_contracts() {
    print_status "Building all contracts..."
    
    # Build escrow contract
    print_status "Building escrow contract..."
    cd escrow
    cargo build --target wasm32-unknown-unknown --release
    soroban contract optimize --wasm target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm
    cd ..
    
    # Build verification contract
    print_status "Building verification contract..."
    cd contracts/verification
    cargo build --target wasm32-unknown-unknown --release
    soroban contract optimize --wasm target/wasm32-unknown-unknown/release/mentorminds_verification.wasm
    cd ../..
    
    # Build oracle contract
    print_status "Building oracle contract..."
    cd contracts/oracle
    cargo build --target wasm32-unknown-unknown --release
    soroban contract optimize --wasm target/wasm32-unknown-unknown/release/mentorminds_oracle.wasm
    cd ../..
    
    # Build timelock contract
    print_status "Building timelock contract..."
    cd contracts/timelock
    cargo build --target wasm32-unknown-unknown --release
    soroban contract optimize --wasm target/wasm32-unknown-unknown/release/mentorminds_timelock.wasm
    cd ../..
    
    print_success "All contracts built successfully"
}

# Function to deploy contracts
deploy_contracts() {
    print_status "Deploying contracts to local network..."
    
    local contracts_file="deployed/local.json"
    local admin_address=$(jq -r '.accounts.admin.address' deployed/accounts.json)
    
    # Initialize contracts file
    echo "{" > $contracts_file
    echo "  \"network\": \"$NETWORK_NAME\"," >> $contracts_file
    echo "  \"deployed_at\": \"$(date -u +%Y-%m-%dT%H:%M:%SZ)\"," >> $contracts_file
    echo "  \"contracts\": {" >> $contracts_file
    
    local first=true
    
    # Deploy escrow contract
    if [ "$first" = true ]; then
        first=false
    else
        echo "," >> $contracts_file
    fi
    
    print_status "Deploying escrow contract..."
    local escrow_id=$(soroban contract deploy \
        --wasm escrow/target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm \
        --source local_admin \
        --network $NETWORK_NAME)
    
    echo "    \"escrow\": {" >> $contracts_file
    echo "      \"contract_id\": \"$escrow_id\"," >> $contracts_file
    echo "      \"wasm_path\": \"escrow/target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm\"" >> $contracts_file
    echo -n "    }" >> $contracts_file
    
    # Initialize escrow contract
    print_status "Initializing escrow contract..."
    soroban contract invoke \
        --id $escrow_id \
        --source local_admin \
        --network $NETWORK_NAME \
        -- initialize \
        --admin $admin_address \
        --platform_fee 5
    
    # Deploy verification contract
    echo "," >> $contracts_file
    print_status "Deploying verification contract..."
    local verification_id=$(soroban contract deploy \
        --wasm contracts/verification/target/wasm32-unknown-unknown/release/mentorminds_verification.wasm \
        --source local_admin \
        --network $NETWORK_NAME)
    
    echo "    \"verification\": {" >> $contracts_file
    echo "      \"contract_id\": \"$verification_id\"," >> $contracts_file
    echo "      \"wasm_path\": \"contracts/verification/target/wasm32-unknown-unknown/release/mentorminds_verification.wasm\"" >> $contracts_file
    echo -n "    }" >> $contracts_file
    
    # Initialize verification contract
    print_status "Initializing verification contract..."
    soroban contract invoke \
        --id $verification_id \
        --source local_admin \
        --network $NETWORK_NAME \
        -- initialize \
        --admin $admin_address
    
    # Deploy oracle contract
    echo "," >> $contracts_file
    print_status "Deploying oracle contract..."
    local oracle_id=$(soroban contract deploy \
        --wasm contracts/oracle/target/wasm32-unknown-unknown/release/mentorminds_oracle.wasm \
        --source local_admin \
        --network $NETWORK_NAME)
    
    echo "    \"oracle\": {" >> $contracts_file
    echo "      \"contract_id\": \"$oracle_id\"," >> $contracts_file
    echo "      \"wasm_path\": \"contracts/oracle/target/wasm32-unknown-unknown/release/mentorminds_oracle.wasm\"" >> $contracts_file
    echo -n "    }" >> $contracts_file
    
    # Initialize oracle contract
    print_status "Initializing oracle contract..."
    soroban contract invoke \
        --id $oracle_id \
        --source local_admin \
        --network $NETWORK_NAME \
        -- initialize \
        --admin $admin_address
    
    # Deploy timelock contract
    echo "," >> $contracts_file
    print_status "Deploying timelock contract..."
    local timelock_id=$(soroban contract deploy \
        --wasm contracts/timelock/target/wasm32-unknown-unknown/release/mentorminds_timelock.wasm \
        --source local_admin \
        --network $NETWORK_NAME)
    
    echo "    \"timelock\": {" >> $contracts_file
    echo "      \"contract_id\": \"$timelock_id\"," >> $contracts_file
    echo "      \"wasm_path\": \"contracts/timelock/target/wasm32-unknown-unknown/release/mentorminds_timelock.wasm\"" >> $contracts_file
    echo -n "    }" >> $contracts_file
    
    # Initialize timelock contract
    print_status "Initializing timelock contract..."
    soroban contract invoke \
        --id $timelock_id \
        --source local_admin \
        --network $NETWORK_NAME \
        -- initialize \
        --admin $admin_address \
        --min_delay 3600
    
    echo "" >> $contracts_file
    echo "  }" >> $contracts_file
    echo "}" >> $contracts_file
    
    print_success "All contracts deployed successfully"
}

# Function to verify deployment
verify_deployment() {
    print_status "Verifying contract deployment..."
    
    local contracts_file="deployed/local.json"
    
    # Check each contract
    local contracts=("escrow" "verification" "oracle" "timelock")
    
    for contract in "${contracts[@]}"; do
        local contract_id=$(jq -r ".contracts.$contract.contract_id" $contracts_file)
        print_status "Verifying $contract contract ($contract_id)..."
        
        # Try to read contract info
        if soroban contract read \
            --id $contract_id \
            --network $NETWORK_NAME > /dev/null 2>&1; then
            print_success "$contract contract is deployed and accessible"
        else
            print_error "$contract contract verification failed"
            return 1
        fi
    done
    
    print_success "All contracts verified successfully"
}

# Main execution
main() {
    print_status "Setting up MentorMinds local development environment..."
    
    check_docker
    start_stellar
    configure_soroban
    create_and_fund_accounts
    build_contracts
    deploy_contracts
    verify_deployment
    
    print_success "Local development environment setup complete!"
    echo ""
    print_status "Available services:"
    echo "  - Horizon API: $HORIZON_URL"
    echo "  - Stellar RPC: $RPC_URL"
    echo "  - Soroban RPC: $SOROBAN_RPC_URL"
    echo "  - Friendbot: $FRIENDBOT_URL"
    echo ""
    print_status "Configuration files:"
    echo "  - Accounts: deployed/accounts.json"
    echo "  - Contracts: deployed/local.json"
    echo ""
    print_status "Next steps:"
    echo "  1. Run 'npm run local:seed' to create sample data"
    echo "  2. Start backend development with 'npm run dev'"
    echo "  3. Use 'npm run local:stop' to stop the environment"
}

# Run main function
main "$@"

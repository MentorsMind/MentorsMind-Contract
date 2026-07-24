#!/bin/bash

# Configuration
NETWORK="testnet"
SECRET_KEY="S..." # Replace with actual admin secret or use env variable
RPC_URL="https://soroban-testnet.stellar.org"
ADMIN_ADDRESS="G..." # Replace with admin public key

echo "Deploying MentorsMind Escrow Contract to $NETWORK..."

# Build the contract
cargo build --target wasm32-unknown-unknown --release -p mentorminds-escrow

# Deploy the contract
CONTRACT_ID=$(soroban contract deploy \
    --wasm target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm \
    --source $SECRET_KEY \
    --network $NETWORK)

echo "Contract deployed with ID: $CONTRACT_ID"

# Initialize the contract (example values)
# soroban contract invoke \
#     --id $CONTRACT_ID \
#     --source $SECRET_KEY \
#     --network $NETWORK \
#     -- \
#     initialize \
#     --admin $ADMIN_ADDRESS \
#     --treasury $ADMIN_ADDRESS \
#     --fee_bps 500 \
#     --approved_tokens "[\"CBG...\", \"CCG...\"]" \
#     --auto_release_delay_secs 259200

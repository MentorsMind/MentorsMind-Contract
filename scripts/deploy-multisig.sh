#!/bin/bash

# Configuration
NETWORK="testnet"
SOURCE_ACCOUNT="admin"
ADMIN_ADDRESS="G..." # Replace with actual admin address

echo "Building multisig contract..."
soroban contract build --package mentorminds-multisig

echo "Deploying multisig contract..."
CONTRACT_ID=$(soroban contract deploy \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_multisig.wasm \
  --source $SOURCE_ACCOUNT \
  --network $NETWORK)

echo "Contract deployed with ID: $CONTRACT_ID"

echo "Initializing contract..."
soroban contract invoke \
  --id $CONTRACT_ID \
  --source $SOURCE_ACCOUNT \
  --network $NETWORK \
  -- \
  initialize \
  --admin $ADMIN_ADDRESS \
  --signers "[\"$ADMIN_ADDRESS\"]" \
  --threshold 1

echo "Multisig Contract Initialization Complete."

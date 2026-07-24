#!/bin/bash

# MentorMinds Local Data Seeding Script
# This script creates sample escrows, sessions, and reviews for testing

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
BLUE='\033[0;34m'
NC='\033[0m' # No Color

# Configuration
NETWORK_NAME="standalone"

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

# Function to check if local environment is running
check_environment() {
    if [ ! -f "deployed/local.json" ] || [ ! -f "deployed/accounts.json" ]; then
        print_error "Local environment not set up. Please run 'npm run local:start' first."
        exit 1
    fi
    
    # Check if Stellar container is running
    if ! docker ps -q -f name=mentorminds-stellar | grep -q .; then
        print_error "Stellar container is not running. Please run 'npm run local:start' first."
        exit 1
    fi
    
    print_success "Local environment is running"
}

# Function to get contract and account addresses
get_addresses() {
    ESCROW_CONTRACT_ID=$(jq -r '.contracts.escrow.contract_id' deployed/local.json)
    VERIFICATION_CONTRACT_ID=$(jq -r '.contracts.verification.contract_id' deployed/local.json)
    ORACLE_CONTRACT_ID=$(jq -r '.contracts.oracle.contract_id' deployed/local.json)
    TIMELOCK_CONTRACT_ID=$(jq -r '.contracts.timelock.contract_id' deployed/local.json)
    
    ADMIN_ADDRESS=$(jq -r '.accounts.admin.address' deployed/accounts.json)
    MENTOR1_ADDRESS=$(jq -r '.accounts.mentor1.address' deployed/accounts.json)
    MENTOR2_ADDRESS=$(jq -r '.accounts.mentor2.address' deployed/accounts.json)
    LEARNER1_ADDRESS=$(jq -r '.accounts.learner1.address' deployed/accounts.json)
    LEARNER2_ADDRESS=$(jq -r '.accounts.learner2.address' deployed/accounts.json)
    
    print_status "Loaded contract and account addresses"
}

# Function to create sample escrows
create_escrows() {
    print_status "Creating sample escrows..."
    
    # Escrow 1: mentor1 and learner1 - Web Development session
    print_status "Creating escrow 1: Web Development (mentor1 -> learner1)..."
    local escrow1_id=$(soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_learner1 \
        --network $NETWORK_NAME \
        -- create_escrow \
        --mentor $MENTOR1_ADDRESS \
        --amount 1000000000 \
        --session_id "web-dev-001" \
        --metadata '{"title": "Web Development Basics", "duration": 3600, "category": "programming"}' \
        --time_lock 1640995200 | jq -r '.result')
    
    # Fund the escrow
    print_status "Funding escrow 1..."
    soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_learner1 \
        --network $NETWORK_NAME \
        -- fund_escrow \
        --escrow_id $escrow1_id \
        --amount 1000000000
    
    # Escrow 2: mentor2 and learner2 - Smart Contract Development
    print_status "Creating escrow 2: Smart Contract Development (mentor2 -> learner2)..."
    local escrow2_id=$(soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_learner2 \
        --network $NETWORK_NAME \
        -- create_escrow \
        --mentor $MENTOR2_ADDRESS \
        --amount 1500000000 \
        --session_id "smart-contract-001" \
        --metadata '{"title": "Smart Contract Development", "duration": 7200, "category": "blockchain"}' \
        --time_lock 1640995200 | jq -r '.result')
    
    # Fund the escrow
    print_status "Funding escrow 2..."
    soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_learner2 \
        --network $NETWORK_NAME \
        -- fund_escrow \
        --escrow_id $escrow2_id \
        --amount 1500000000
    
    # Escrow 3: mentor1 and learner2 - Rust Programming
    print_status "Creating escrow 3: Rust Programming (mentor1 -> learner2)..."
    local escrow3_id=$(soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_learner2 \
        --network $NETWORK_NAME \
        -- create_escrow \
        --mentor $MENTOR1_ADDRESS \
        --amount 800000000 \
        --session_id "rust-001" \
        --metadata '{"title": "Rust Programming Fundamentals", "duration": 5400, "category": "programming"}' \
        --time_lock 1640995200 | jq -r '.result')
    
    # Fund the escrow
    print_status "Funding escrow 3..."
    soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_learner2 \
        --network $NETWORK_NAME \
        -- fund_escrow \
        --escrow_id $escrow3_id \
        --amount 800000000
    
    # Store escrow IDs for later use
    echo "{
      \"escrow1\": \"$escrow1_id\",
      \"escrow2\": \"$escrow2_id\",
      \"escrow3\": \"$escrow3_id\"
    }" > deployed/seed_escrows.json
    
    print_success "Created 3 sample escrows"
}

# Function to create verification records
create_verifications() {
    print_status "Creating verification records..."
    
    # Verify mentor1
    print_status "Verifying mentor1..."
    soroban contract invoke \
        --id $VERIFICATION_CONTRACT_ID \
        --source local_admin \
        --network $NETWORK_NAME \
        -- verify_mentor \
        --mentor $MENTOR1_ADDRESS \
        --verification_data '{"skills": ["web-development", "javascript", "react"], "experience_years": 5, "rating": 4.8}' \
        --verification_level "verified"
    
    # Verify mentor2
    print_status "Verifying mentor2..."
    soroban contract invoke \
        --id $VERIFICATION_CONTRACT_ID \
        --source local_admin \
        --network $NETWORK_NAME \
        -- verify_mentor \
        --mentor $MENTOR2_ADDRESS \
        --verification_data '{"skills": ["blockchain", "smart-contracts", "rust"], "experience_years": 3, "rating": 4.9}' \
        --verification_level "premium"
    
    print_success "Created verification records for mentors"
}

# Function to create oracle price feeds
create_oracle_data() {
    print_status "Creating oracle price feeds..."
    
    # Set XLM price
    print_status "Setting XLM price feed..."
    soroban contract invoke \
        --id $ORACLE_CONTRACT_ID \
        --source local_admin \
        --network $NETWORK_NAME \
        -- update_price \
        --asset "XLM" \
        --price 10000000 \
        --timestamp $(date +%s)
    
    # Set session completion rates
    print_status "Setting session completion rate..."
    soroban contract invoke \
        --id $ORACLE_CONTRACT_ID \
        --source local_admin \
        --network $NETWORK_NAME \
        -- update_metric \
        --metric "session_completion_rate" \
        --value 9500 \
        --timestamp $(date +%s)
    
    print_success "Created oracle price feeds"
}

# Function to simulate completed sessions and reviews
create_sessions_and_reviews() {
    print_status "Simulating completed sessions and creating reviews..."
    
    local escrow_ids=$(cat deployed/seed_escrows.json)
    local escrow1_id=$(echo $escrow_ids | jq -r '.escrow1')
    local escrow2_id=$(echo $escrow_ids | jq -r '.escrow2')
    
    # Complete session 1 and create review
    print_status "Completing session 1 and creating review..."
    soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_mentor1 \
        --network $NETWORK_NAME \
        -- complete_session \
        --escrow_id $escrow1_id \
        --completion_proof '{"duration_minutes": 65, "topics_covered": ["html", "css", "javascript-basics"]}'
    
    # Release funds for escrow 1
    soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_learner1 \
        --network $NETWORK_NAME \
        -- release_funds \
        --escrow_id $escrow1_id
    
    # Create review for mentor1
    print_status "Creating review for mentor1..."
    soroban contract invoke \
        --id $VERIFICATION_CONTRACT_ID \
        --source local_learner1 \
        --network $NETWORK_NAME \
        -- create_review \
        --mentor $MENTOR1_ADDRESS \
        --learner $LEARNER1_ADDRESS \
        --session_id "web-dev-001" \
        --rating 5 \
        --review_text "Excellent teaching style! Very patient and knowledgeable." \
        --skills_taught '["html", "css", "javascript"]'
    
    # Complete session 2 and create review
    print_status "Completing session 2 and creating review..."
    soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_mentor2 \
        --network $NETWORK_NAME \
        -- complete_session \
        --escrow_id $escrow2_id \
        --completion_proof '{"duration_minutes": 125, "topics_covered": ["soroban-basics", "smart-contract-patterns", "security"]}'
    
    # Release funds for escrow 2
    soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_learner2 \
        --network $NETWORK_NAME \
        -- release_funds \
        --escrow_id $escrow2_id
    
    # Create review for mentor2
    print_status "Creating review for mentor2..."
    soroban contract invoke \
        --id $VERIFICATION_CONTRACT_ID \
        --source local_learner2 \
        --network $NETWORK_NAME \
        -- create_review \
        --mentor $MENTOR2_ADDRESS \
        --learner $LEARNER2_ADDRESS \
        --session_id "smart-contract-001" \
        --rating 4 \
        --review_text "Great content, but could use more practical examples." \
        --skills_taught '["smart-contracts", "rust", "soroban"]'
    
    print_success "Created completed sessions and reviews"
}

# Function to create a dispute scenario
create_dispute_scenario() {
    print_status "Creating a dispute scenario for testing..."
    
    local escrow_ids=$(cat deployed/seed_escrows.json)
    local escrow3_id=$(echo $escrow_ids | jq -r '.escrow3')
    
    # Create a dispute for escrow 3
    print_status "Creating dispute for escrow 3..."
    soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_learner2 \
        --network $NETWORK_NAME \
        -- create_dispute \
        --escrow_id $escrow3_id \
        --reason "Session cancelled by mentor last minute" \
        --evidence '{"messages": ["mentor cancelled 1 hour before session"], "refund_requested": true}'
    
    print_success "Created dispute scenario"
}

# Function to generate summary report
generate_summary() {
    print_status "Generating seeding summary..."
    
    local summary_file="deployed/seed_summary.json"
    
    # Get contract states
    local escrow_count=$(soroban contract invoke \
        --id $ESCROW_CONTRACT_ID \
        --source local_admin \
        --network $NETWORK_NAME \
        -- get_escrow_count | jq -r '.result')
    
    local verification_count=$(soroban contract invoke \
        --id $VERIFICATION_CONTRACT_ID \
        --source local_admin \
        --network $NETWORK_NAME \
        -- get_verified_mentors_count | jq -r '.result')
    
    # Create summary
    cat > $summary_file << EOF
{
  "seeded_at": "$(date -u +%Y-%m-%dT%H:%M:%SZ)",
  "network": "$NETWORK_NAME",
  "summary": {
    "total_escrows": $escrow_count,
    "verified_mentors": $verification_count,
    "completed_sessions": 2,
    "active_disputes": 1,
    "total_reviews": 2
  },
  "contracts": {
    "escrow": "$ESCROW_CONTRACT_ID",
    "verification": "$VERIFICATION_CONTRACT_ID",
    "oracle": "$ORACLE_CONTRACT_ID",
    "timelock": "$TIMELOCK_CONTRACT_ID"
  },
  "accounts": {
    "admin": "$ADMIN_ADDRESS",
    "mentor1": "$MENTOR1_ADDRESS",
    "mentor2": "$MENTOR2_ADDRESS",
    "learner1": "$LEARNER1_ADDRESS",
    "learner2": "$LEARNER2_ADDRESS"
  },
  "sample_data": {
    "escrows": "deployed/seed_escrows.json",
    "completed_sessions": ["web-dev-001", "smart-contract-001"],
    "disputed_sessions": ["rust-001"],
    "reviews_created": 2
  }
}
EOF
    
    print_success "Summary report generated: $summary_file"
}

# Function to display seeded data
display_seeded_data() {
    print_status "Seeded Data Overview:"
    echo ""
    
    print_status "📊 Summary:"
    local summary_file="deployed/seed_summary.json"
    if [ -f "$summary_file" ]; then
        echo "  Total Escrows: $(jq -r '.summary.total_escrows' $summary_file)"
        echo "  Verified Mentors: $(jq -r '.summary.verified_mentors' $summary_file)"
        echo "  Completed Sessions: $(jq -r '.summary.completed_sessions' $summary_file)"
        echo "  Active Disputes: $(jq -r '.summary.active_disputes' $summary_file)"
        echo "  Total Reviews: $(jq -r '.summary.total_reviews' $summary_file)"
    fi
    
    echo ""
    print_status "👥 Accounts:"
    echo "  Admin: $ADMIN_ADDRESS"
    echo "  Mentor 1: $MENTOR1_ADDRESS (Web Development)"
    echo "  Mentor 2: $MENTOR2_ADDRESS (Smart Contracts)"
    echo "  Learner 1: $LEARNER1_ADDRESS"
    echo "  Learner 2: $LEARNER2_ADDRESS"
    
    echo ""
    print_status "📋 Sample Sessions:"
    echo "  1. Web Development Basics - COMPLETED"
    echo "  2. Smart Contract Development - COMPLETED"
    echo "  3. Rust Programming Fundamentals - DISPUTED"
    
    echo ""
    print_status "🔗 Useful Commands:"
    echo "  Check escrow details: soroban contract invoke --id $ESCROW_CONTRACT_ID --network $NETWORK_NAME -- get_escrow --escrow_id <ID>"
    echo "  View mentor reviews: soroban contract invoke --id $VERIFICATION_CONTRACT_ID --network $NETWORK_NAME -- get_mentor_reviews --mentor <ADDRESS>"
    echo "  Check oracle prices: soroban contract invoke --id $ORACLE_CONTRACT_ID --network $NETWORK_NAME -- get_all_prices"
}

# Main execution
main() {
    print_status "Seeding MentorMinds local environment with sample data..."
    
    check_environment
    get_addresses
    create_verifications
    create_oracle_data
    create_escrows
    create_sessions_and_reviews
    create_dispute_scenario
    generate_summary
    display_seeded_data
    
    print_success "Local environment seeding complete!"
    echo ""
    print_status "Files created:"
    echo "  - deployed/seed_escrows.json: Sample escrow IDs"
    echo "  - deployed/seed_summary.json: Complete seeding summary"
    echo ""
    print_status "You can now test the application with realistic data!"
}

# Run main function
main "$@"

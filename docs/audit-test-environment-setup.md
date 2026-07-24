# Security Audit Test Environment Setup

**Version**: 1.0  
**Date**: March 25, 2026  
**Purpose**: Prepare test environment for third-party security auditors

---

## Overview

This document provides step-by-step instructions for setting up a complete test environment for security auditors to assess MentorMinds smart contracts. The environment includes deployed contracts on Stellar testnet, funded accounts, monitoring tools, and comprehensive documentation.

---

## Prerequisites

### System Requirements

| Requirement                       | Version | Installation                               |
| --------------------------------- | ------- | ------------------------------------------ |
| **Rust**                          | 1.70+   | `rustup install stable`                    |
| **wasm32-unknown-unknown target** | -       | `rustup target add wasm32-unknown-unknown` |
| **Soroban CLI**                   | Latest  | `cargo install --locked soroban-cli`       |
| **Node.js**                       | 18+     | Download from nodejs.org                   |
| **jq**                            | Latest  | `choco install jq` (Windows)               |

### Verification

```bash
# Verify installations
rustc --version        # Should show 1.70+
cargo --version
soroban --version
node --version         # Should show v18+
npm --version
```

---

## 1. Stellar Testnet Configuration

### 1.1 Configure Soroban for Testnet

```bash
# Add testnet network configuration
soroban config network add testnet \
  --rpc-url https://soroban-testnet.stellar.org:443 \
  --network-passphrase "Test SDF Network ; September 2015"
```

### 1.2 Generate Auditor Identities

Create separate identities for different roles:

```bash
# Generate admin identity
soroban config identity generate auditor-admin

# Generate learner identity
soroban config identity generate auditor-learner

# Generate mentor identity
soroban config identity generate auditor-mentor

# Generate treasury identity
soroban config identity generate auditor-treasury

# List all identities
soroban config identity list
```

### 1.3 Fund Testnet Accounts

```bash
# Get public addresses
ADMIN_ADDR=$(soroban config identity address auditor-admin)
LEARNER_ADDR=$(soroban config identity address auditor-learner)
MENTOR_ADDR=$(soroban config identity address auditor-mentor)
TREASURY_ADDR=$(soroban config identity address auditor-treasury)

echo "Admin: $ADMIN_ADDR"
echo "Learner: $LEARNER_ADDR"
echo "Mentor: $MENTOR_ADDR"
echo "Treasury: $TREASURY_ADDR"
```

**Fund each account via Stellar Laboratory**:

1. Visit: https://laboratory.stellar.org/#account-creator?network=test
2. Enter each address
3. Click "Create Account"
4. Each account receives 10,000 XLM (testnet)

**Alternative: Friendbot API**

```bash
# Fund via API (if friendbot is available)
curl "https://friendbot.stellar.org?addr=$ADMIN_ADDR"
curl "https://friendbot.stellar.org?addr=$LEARNER_ADDR"
curl "https://friendbot.stellar.org?addr=$MENTOR_ADDR"
curl "https://friendbot.stellar.org?addr=$TREASURY_ADDR"
```

### 1.4 Verify Balances

```bash
# Check account balances (use Stellar Expert or CLI)
# https://stellar.expert/explorer/testnet/account/<ADDRESS>

# Or use soroban (if balance command available)
soroban balance auditor-admin --network testnet
```

---

## 2. Contract Deployment to Testnet

### 2.1 Build All Contracts

```bash
# Clean previous builds
cargo clean

# Build all contracts for WASM target
cargo build --target wasm32-unknown-unknown --release

# Verify WASM files exist
ls -lh target/wasm32-unknown-unknown/release/*.wasm
```

Expected output:

```
mentorminds_escrow.wasm
mentorminds_verification.wasm
mentorminds_mnt_token.wasm
mentorminds_referral.wasm
```

### 2.2 Optimize WASM Files

```bash
# Optimize each contract
stellar contract optimize \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm

stellar contract optimize \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_verification.wasm

stellar contract optimize \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_mnt_token.wasm

stellar contract optimize \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_referral.wasm
```

Optimized files will be created with `.optimized` suffix.

### 2.3 Deploy Escrow Contract

```bash
# Deploy escrow
ESCROW_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm.optimized \
  --source auditor-admin \
  --network testnet)

echo "ESCROW_CONTRACT_ID=$ESCROW_ID"
```

**Record the contract ID**: `CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA`

### 2.4 Deploy Verification Contract

```bash
VERIFICATION_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_verification.wasm.optimized \
  --source auditor-admin \
  --network testnet)

echo "VERIFICATION_CONTRACT_ID=$VERIFICATION_ID"
```

### 2.5 Deploy MNT Token Contract

```bash
TOKEN_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_mnt_token.wasm.optimized \
  --source auditor-admin \
  --network testnet)

echo "MNT_TOKEN_CONTRACT_ID=$TOKEN_ID"
```

### 2.6 Deploy Referral Contract

```bash
REFERRAL_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_referral.wasm.optimized \
  --source auditor-admin \
  --network testnet)

echo "REFERRAL_CONTRACT_ID=$REFERRAL_ID"
```

### 2.7 Record Deployment

Save all contract IDs to `deployed/testnet-audit.json`:

```json
{
  "network": "testnet",
  "purpose": "Security Audit Environment",
  "deployed_at": "2026-03-25T00:00:00Z",
  "admin_address": "<ADMIN_ADDR>",
  "treasury_address": "<TREASURY_ADDR>",
  "contracts": {
    "escrow": "<ESCROW_ID>",
    "verification": "<VERIFICATION_ID>",
    "mnt_token": "<TOKEN_ID>",
    "referral": "<REFERRAL_ID>"
  }
}
```

---

## 3. Contract Initialization

### 3.1 Initialize Escrow Contract

```bash
# Create approved tokens array (USDC testnet example)
# USDC testnet: GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5

stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-admin \
  --network testnet \
  -- initialize \
  --admin "$ADMIN_ADDR" \
  --treasury "$TREASURY_ADDR" \
  --fee_bps 500 \
  --approved_tokens '["GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5"]' \
  --auto_release_delay_secs 86400
```

Parameters:

- `fee_bps = 500` → 5% platform fee
- `auto_release_delay_secs = 86400` → 24 hours (shortened for testing)

### 3.2 Initialize Verification Contract

```bash
stellar contract invoke \
  --id "$VERIFICATION_ID" \
  --source auditor-admin \
  --network testnet \
  -- initialize \
  --admin "$ADMIN_ADDR"
```

### 3.3 Initialize MNT Token Contract

```bash
stellar contract invoke \
  --id "$TOKEN_ID" \
  --source auditor-admin \
  --network testnet \
  -- initialize \
  --admin "$REFERRAL_ID"
```

Note: MNT token admin is set to referral contract so it can mint rewards.

### 3.4 Initialize Referral Contract

```bash
stellar contract invoke \
  --id "$REFERRAL_ID" \
  --source auditor-admin \
  --network testnet \
  -- initialize \
  --admin "$ADMIN_ADDR" \
  --mnt_token "$TOKEN_ID"
```

---

## 4. Test Scenario Setup

### 4.1 Create Test USDC Tokens

Since we need USDC for escrow testing, create a test asset:

```bash
# Register stellar asset contract for testing
USDC_TEST_ADDR="GBBD47IF6LWK7P7MDEVSCWR7DPUWV3NY3DTQEVFL4NAT4AQH3ZLLFLA5"

# Add to escrow approved tokens (if not already added)
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-admin \
  --network testnet \
  -- set_approved_token \
  --token "$USDC_TEST_ADDR" \
  --approved true
```

### 4.2 Mint Test Tokens for Learner

For testing purposes, we'll use the SAC (Stellar Asset Contract):

```bash
# Create a test SAC contract
USDC_CLIENT=$(stellar contract asset deploy \
  --asset "USDC:$(soroban config identity address auditor-admin)" \
  --source auditor-admin \
  --network testnet)

# Mint USDC to learner
stellar contract invoke \
  --id "$USDC_CLIENT" \
  --source auditor-admin \
  --network testnet \
  -- mint \
  --to "$LEARNER_ADDR" \
  --amount 1000000000
```

### 4.3 Verify Initial State

```bash
# Check escrow fee
stellar contract invoke \
  --id "$ESCROW_ID" \
  --network testnet \
  -- get_fee_bps
# Expected: 500

# Check auto-release delay
stellar contract invoke \
  --id "$ESCROW_ID" \
  --network testnet \
  -- get_auto_release_delay
# Expected: 86400

# Check treasury address
stellar contract invoke \
  --id "$ESCROW_ID" \
  --network testnet \
  -- get_treasury
# Expected: $TREASURY_ADDR

# Verify USDC is approved
stellar contract invoke \
  --id "$ESCROW_ID" \
  --network testnet \
  -- is_token_approved \
  --token "$USDC_TEST_ADDR"
# Expected: true
```

---

## 5. Pre-Configured Test Scenarios

### Scenario 1: Basic Escrow Flow

**Setup Script**: `scripts/audit-scenario-1.sh`

```bash
#!/bin/bash
# Create escrow
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-learner \
  --network testnet \
  -- create_escrow \
  --mentor "$MENTOR_ADDR" \
  --learner "$LEARNER_ADDR" \
  --amount 10000000 \
  --session_id "AUDIT_TEST_1" \
  --token "$USDC_TEST_ADDR" \
  --session_end_time "$(date +%s)"

# Release funds
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-learner \
  --network testnet \
  -- release_funds \
  --caller "$LEARNER_ADDR" \
  --escrow_id 1
```

### Scenario 2: Dispute and Resolution

**Setup Script**: `scripts/audit-scenario-2.sh`

```bash
#!/bin/bash
# Create escrow
ID=$(stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-learner \
  --network testnet \
  -- create_escrow \
  --mentor "$MENTOR_ADDR" \
  --learner "$LEARNER_ADDR" \
  --amount 10000000 \
  --session_id "AUDIT_DISPUTE_1" \
  --token "$USDC_TEST_ADDR" \
  --session_end_time "$(date +%s)")

# Open dispute (as mentor)
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-mentor \
  --network testnet \
  -- dispute \
  --caller "$MENTOR_ADDR" \
  --escrow_id "$ID" \
  --reason "NO_SHOW"

# Resolve dispute (as admin)
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-admin \
  --network testnet \
  -- resolve_dispute \
  --escrow_id "$ID" \
  --mentor_pct 50
```

### Scenario 3: Auto-Release

**Setup Script**: `scripts/audit-scenario-3.sh`

```bash
#!/bin/bash
# Create escrow with past end time
NOW=$(date +%s)
PAST_TIME=$((NOW - 100))

ID=$(stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-learner \
  --network testnet \
  -- create_escrow \
  --mentor "$MENTOR_ADDR" \
  --learner "$LEARNER_ADDR" \
  --amount 5000000 \
  --session_id "AUDIT_AUTO_1" \
  --token "$USDC_TEST_ADDR" \
  --session_end_time "$PAST_TIME")

# Wait for auto-release delay (24 hours in test, or advance ledger time)
# For testing, we can use ledger manipulation:
stellar ledger set --timestamp $((NOW + 86401)) --network testnet

# Trigger auto-release (permissionless)
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-admin \
  --network testnet \
  -- try_auto_release \
  --escrow_id "$ID"
```

### Scenario 4: Refund Flow

**Setup Script**: `scripts/audit-scenario-4.sh`

```bash
#!/bin/bash
# Create escrow
ID=$(stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-learner \
  --network testnet \
  -- create_escrow \
  --mentor "$MENTOR_ADDR" \
  --learner "$LEARNER_ADDR" \
  --amount 7500000 \
  --session_id "AUDIT_REFUND_1" \
  --token "$USDC_TEST_ADDR" \
  --session_end_time "$(date +%s)")

# Refund (as admin)
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source auditor-admin \
  --network testnet \
  -- refund \
  --escrow_id "$ID"
```

### Scenario 5: Referral Rewards

**Setup Script**: `scripts/audit-scenario-5.sh`

```bash
#!/bin/bash
REFERRER="$MENTOR_ADDR"
REFEREE="$LEARNER_ADDR"

# Register referral
stellar contract invoke \
  --id "$REFERRAL_ID" \
  --source auditor-admin \
  --network testnet \
  -- register_referral \
  --referrer "$REFERRER" \
  --referee "$REFEREE" \
  --is_mentor true

# Fulfill referral
stellar contract invoke \
  --id "$REFERRAL_ID" \
  --source auditor-admin \
  --network testnet \
  -- fulfill_referral \
  --referee "$REFEREE"

# Claim reward
stellar contract invoke \
  --id "$REFERRAL_ID" \
  --source auditor-mentor \
  --network testnet \
  -- claim_reward \
  --referrer "$REFERRER"

# Check balance
stellar contract invoke \
  --id "$TOKEN_ID" \
  --network testnet \
  -- balance \
  --id "$REFERRER"
```

---

## 6. Monitoring and Inspection Tools

### 6.1 Stellar Expert

**Primary Block Explorer**: https://stellar.expert/explorer/testnet

Useful URLs (replace `<CONTRACT_ID>`):

- Contract Viewer: `https://stellar.expert/explorer/testnet/contract/<CONTRACT_ID>`
- Transaction History: `https://stellar.expert/explorer/testnet/account/<CONTRACT_ID>/transactions`
- Events: `https://stellar.expert/explorer/testnet/contract/<CONTRACT_ID>/events`
- Storage: `https://stellar.expert/explorer/testnet/contract/<CONTRACT_ID>/storage`

### 6.2 Soroban RPC Queries

```bash
# Get contract data
stellar rpc get ledgers latest --network testnet

# Get events for contract
stellar events --id "$ESCROW_ID" --network testnet

# Query contract storage
stellar contract inspect \
  --id "$ESCROW_ID" \
  --network testnet \
  --key "ESC_CNT"
```

### 6.3 Custom Monitoring Script

Create `scripts/monitor-contracts.sh`:

```bash
#!/bin/bash

ESCROW_ID="<ESCROW_ID>"
NETWORK="testnet"

echo "=== MentorMinds Contract Monitor ==="
echo "Network: $NETWORK"
echo "Timestamp: $(date -u +"%Y-%m-%dT%H:%M:%SZ")"
echo ""

echo "Escrow Contract: $ESCROW_ID"
echo "Fee (bps): $(stellar contract invoke --id $ESCROW_ID --network $NETWORK -- get_fee_bps)"
echo "Treasury: $(stellar contract invoke --id $ESCROW_ID --network $NETWORK -- get_treasury)"
echo "Escrow Count: $(stellar contract invoke --id $ESCROW_ID --network $NETWORK -- get_escrow_count)"
echo ""

echo "Recent Events:"
stellar events --id "$ESCROW_ID" --network "$NETWORK" --limit 10
```

---

## 7. Audit Checklist for Auditors

### Pre-Audit Verification

- [ ] All contracts compile without warnings (`cargo clippy -- -D warnings`)
- [ ] All unit tests pass (`cargo test --workspace`)
- [ ] Contracts deployed to testnet successfully
- [ ] All contracts initialized with correct parameters
- [ ] Test accounts funded with sufficient XLM
- [ ] Approved token (USDC) configured and minted for testing

### Code Review Areas

#### Escrow Contract

- [ ] Authorization checks in all privileged functions
- [ ] Fee calculation accuracy and rounding behavior
- [ ] State machine transitions (Active → Released/Refunded/Resolved)
- [ ] Auto-release timestamp boundary conditions
- [ ] Dispute resolution percentage calculations
- [ ] Reentrancy protection
- [ ] Integer overflow/underflow protection
- [ ] Event emission completeness

#### Verification Contract

- [ ] Admin-only access controls
- [ ] Credential hash storage and retrieval
- [ ] Expiry logic and edge cases
- [ ] Revocation mechanism

#### MNT Token Contract

- [ ] Supply cap enforcement
- [ ] Minting authorization
- [ ] Transfer and approval logic
- [ ] Burn mechanism

#### Referral Contract

- [ ] Self-referral prevention
- [ ] Duplicate registration checks
- [ ] Reward calculation accuracy
- [ ] Cross-contract call safety (mint)

### Attack Vectors to Test

1. **Reentrancy**: Attempt reentrant calls during token transfers
2. **Overflow**: Test with i128::MAX values
3. **Underflow**: Test with zero/negative amounts
4. **Authorization Bypass**: Try calling admin functions without auth
5. **State Machine Exploits**: Attempt invalid state transitions
6. **Flash Loan Attacks**: Manipulate token balances mid-transaction
7. **Front-running**: Race condition on dispute resolution
8. **DOS**: Gas exhaustion attacks
9. **Oracle Manipulation**: Not applicable (no price oracles used)
10. **Cross-Contract Call Failures**: Referral → MNT mint failure modes

---

## 8. Known Issues and Limitations

### Documented Limitations

1. **Single Admin Key**: No multi-sig in v1.0 (planned for v1.1)
2. **No Emergency Pause**: Contracts lack global pause mechanism
3. **Symbol Length**: Dispute reasons limited to 6 characters
4. **Simple Allowance**: MNT token lacks expiration on approvals
5. **Admin-Controlled Verification**: Centralized process (by design)

### Accepted Risks

1. **Admin Key Compromise**: Mitigated by hardware wallet requirement
2. **Smart Contract Bugs**: Mitigated by audits and testing
3. **Token Freezing**: USDC issuer can freeze addresses (documented)
4. **Network Congestion**: Stellar TPS limits may affect UX

### Out-of-Scope for Audit

- Economic viability of fee structure
- Business logic decisions (e.g., 72h default auto-release)
- Off-chain governance processes
- Frontend/backend security (separate audits)

---

## 9. Post-Audit Actions

### After Audit Completion

1. **Receive Audit Report**:

   - Critical/High/Medium/Low findings
   - Remediation recommendations
   - Overall security assessment

2. **Triage Findings**:

   - Immediate action for Critical/High
   - Schedule Medium for next sprint
   - Accept or mitigate Low

3. **Implement Fixes**:

   - Create fix branches
   - Write regression tests
   - Internal code review

4. **Deploy Patches**:

   - Deploy to testnet first
   - Verify fixes with auditors
   - Mainnet deployment after sign-off

5. **Public Disclosure**:
   - Publish audit report (redacted if needed)
   - Blog post summarizing findings
   - Update SECURITY.md with lessons learned

---

## 10. Contact and Support

### Audit Coordination Team

| Role                         | Contact                    | Availability    |
| ---------------------------- | -------------------------- | --------------- |
| Lead Smart Contract Engineer | engineering@mentorminds.io | UTC 9-5         |
| Security Lead                | security@mentorminds.io    | 24/7 for urgent |
| CTO                          | cto@mentorminds.io         | By appointment  |

### Communication Channels

- **Email**: security@mentorminds.io (primary)
- **Telegram**: @mentorminds_audit (private group)
- **Discord**: #security-audit channel
- **GitHub**: Private fork for sensitive discussions

### Emergency Contacts

For critical issues discovered during audit:

1. Email security@mentorminds.io with "[CRITICAL]" in subject
2. Telegram @mentorminds_security
3. Do NOT disclose publicly until coordinated response

---

## Appendix A: Quick Reference Commands

```bash
# Build everything
cargo build --target wasm32-unknown-unknown --release

# Run all tests
cargo test --workspace

# Run specific test
cargo test -p mentorminds-escrow test_create_escrow_valid

# Check for vulnerabilities
cargo audit

# Format code
cargo fmt

# Lint code
cargo clippy -- -D warnings

# Deploy contract
stellar contract deploy --wasm <PATH> --source <IDENTITY> --network testnet

# Invoke contract
stellar contract invoke --id <ID> --source <IDENTITY> --network testnet -- <FUNCTION> [ARGS]

# View events
stellar events --id <CONTRACT_ID> --network testnet

# Check balance
soroban balance <IDENTITY> --network testnet
```

---

## Appendix B: Test Account Summary

_To be filled after deployment_

| Account  | Address | Purpose                 | Initial Balance |
| -------- | ------- | ----------------------- | --------------- |
| Admin    | TBD     | Contract administration | 10,000 XLM      |
| Treasury | TBD     | Fee collection          | 10,000 XLM      |
| Learner  | TBD     | Test escrow creation    | 10,000 XLM      |
| Mentor   | TBD     | Test session delivery   | 10,000 XLM      |

---

## Appendix C: Contract Deployment Summary

_To be filled after deployment_

| Contract     | Testnet ID | Mainnet ID (TBD) |
| ------------ | ---------- | ---------------- |
| Escrow       | TBD        | TBD              |
| Verification | TBD        | TBD              |
| MNT Token    | TBD        | TBD              |
| Referral     | TBD        | TBD              |

---

**End of Document**

For questions or clarifications, contact: security@mentorminds.io

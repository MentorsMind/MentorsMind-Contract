# MentorMinds Mainnet Deployment Runbook

**Status**: Pre-deployment  
**Last Updated**: 2026-03-25  
**Network**: Public Global Stellar Network ; September 2015

---

## Table of Contents

1. [Emergency Contacts & Escalation](#1-emergency-contacts--escalation)
2. [Admin Key Custody](#2-admin-key-custody)
3. [Pre-Deploy Checklist](#3-pre-deploy-checklist)
4. [Deployment Cost Estimate](#4-deployment-cost-estimate)
5. [Step-by-Step Deployment](#5-step-by-step-deployment)
6. [Post-Deploy Verification](#6-post-deploy-verification)
7. [Stellar Expert Links](#7-stellar-expert-links)
8. [Rollback Procedure](#8-rollback-procedure)

---

## 1. Emergency Contacts & Escalation

| Role | Name | Contact | Availability |
|------|------|---------|--------------|
| Lead Smart Contract Engineer | `<lead-engineer>` | `<email>` / `<telegram>` | 24/7 on deploy day |
| Platform Admin (multisig key holder 1) | `<admin-1>` | `<email>` / `<telegram>` | 24/7 on deploy day |
| Platform Admin (multisig key holder 2) | `<admin-2>` | `<email>` / `<telegram>` | 24/7 on deploy day |
| Security Auditor | `<auditor>` | `<email>` | Business hours |
| Stellar/Soroban Support | Stellar Discord #soroban | https://discord.gg/stellardev | Community hours |

**Escalation Path**:

1. Issue detected → Lead Engineer assesses severity (< 5 min)
2. If funds at risk → immediately execute [Rollback Procedure](#8-rollback-procedure)
3. Lead Engineer notifies both admin key holders
4. If rollback insufficient → contact Security Auditor
5. Post-incident report within 24 hours

---

## 2. Admin Key Custody

> **Hardware wallet is required for mainnet admin keys. Software wallets are not permitted.**

### Requirements

- Admin key **must** be stored on a hardware wallet (Ledger Nano X/S+ recommended)
- The hardware wallet device must be physically secured and accessible on deploy day
- A second hardware wallet with the same seed phrase must be stored in a separate secure location (offsite backup)
- Seed phrase must be written on metal (not paper) and stored in a fireproof safe

### Key Holders

| Key | Holder | Device | Location |
|-----|--------|--------|----------|
| Admin (deployer) | `<admin-1>` | Ledger Nano X | `<secure-location>` |
| Admin backup | `<admin-2>` | Ledger Nano X | `<offsite-location>` |
| Treasury | `<treasury-holder>` | Ledger Nano X | `<secure-location>` |

### Signing Setup

```bash
# Verify Ledger is recognized before deployment
stellar keys list

# Add the hardware wallet identity (Ledger must be unlocked with Stellar app open)
stellar keys add mainnet-admin --ledger

# Confirm the address matches the expected admin address
stellar keys address mainnet-admin
# Expected: <ADMIN_ADDRESS>
```

**Never** enter the seed phrase into any computer. All signing must happen on the hardware device.

---

## 3. Pre-Deploy Checklist

Complete every item and record sign-off before proceeding. **Do not deploy with any unchecked item.**

### 3.1 Security Audit

- [ ] External security audit completed and report received
- [ ] All Critical and High findings resolved
- [ ] Medium findings reviewed and accepted or resolved
- [ ] Audit report stored at `docs/audit-report-<date>.pdf`
- [ ] Auditor sign-off: `<auditor-name>` on `<date>`

### 3.2 Test Suite

```bash
# Run all unit tests — must show 0 failures
cargo test --workspace

# Expected output (all contracts):
# test result: ok. X passed; 0 failed; 0 ignored
```

- [ ] `cargo test --workspace` passes with 0 failures
- [ ] All snapshot tests match expected state
- [ ] Integration tests pass against testnet deployment
- [ ] Fee calculation tests verified (5% = 500 bps, max 10% = 1000 bps)
- [ ] Auto-release delay tests verified (default 72h = 259200s)

### 3.3 Testnet Validation

- [ ] All three contracts deployed and initialized on testnet
- [ ] Full session lifecycle tested end-to-end on testnet:
  - [ ] `create_escrow` → funds locked
  - [ ] `release_funds` → fee deducted, mentor paid
  - [ ] `dispute` → status transitions to Disputed
  - [ ] `resolve_dispute` → funds split correctly
  - [ ] `refund` → learner refunded
  - [ ] `try_auto_release` → triggers after delay
- [ ] Testnet contract IDs recorded in `deployed/testnet.json`
- [ ] Backend API tested against testnet contracts

### 3.4 Multisig Admin Setup

- [ ] Admin hardware wallet prepared and tested (Ledger + Stellar app)
- [ ] Admin address funded with minimum 10 XLM for deployment fees
- [ ] Treasury address confirmed and funded (separate from admin)
- [ ] Both admin key holders available and reachable on deploy day
- [ ] Admin address: `<ADMIN_ADDRESS>` (confirm matches hardware wallet)
- [ ] Treasury address: `<TREASURY_ADDRESS>` (confirm matches hardware wallet)

### 3.5 Infrastructure

- [ ] RPC endpoint confirmed: `https://mainnet.stellar.validationcloud.io/v1/<KEY>`
- [ ] `VALIDATION_CLOUD_KEY` environment variable set
- [ ] `stellar` CLI version confirmed: `stellar --version`
- [ ] `jq` installed (required by deploy script)
- [ ] `deployed/mainnet.json` does not exist yet (or is intentionally empty `{}`)
- [ ] Rollback plan reviewed by all key holders

### 3.6 Final Sign-Off

| Role | Name | Signature | Date |
|------|------|-----------|------|
| Lead Engineer | | | |
| Admin Key Holder 1 | | | |
| Admin Key Holder 2 | | | |

---

## 4. Deployment Cost Estimate

Soroban fees on mainnet consist of inclusion fees + resource fees. Estimates below are based on typical Soroban contract sizes and operations as of early 2026.

| Operation | Estimated Cost (XLM) |
|-----------|---------------------|
| Deploy `escrow` WASM (~60 KB) | ~2–4 XLM |
| Deploy `verification` WASM (~7 KB) | ~0.5–1 XLM |
| Deploy `mnt_token` WASM (~13 KB) | ~1–2 XLM |
| `initialize` escrow | ~0.1 XLM |
| `initialize` verification | ~0.05 XLM |
| `initialize` mnt_token | ~0.05 XLM |
| Buffer for retries / verification calls | ~1 XLM |
| **Total Estimate** | **~5–9 XLM** |

> Actual fees depend on network congestion and WASM size at build time. Fund the deployer account with at least **15 XLM** to be safe.

Check current fee estimates at [Stellar Expert Fee Stats](https://stellar.expert/explorer/public/network-activity).

---

## 5. Step-by-Step Deployment

### Prerequisites

```bash
# Confirm toolchain
rustup target list --installed | grep wasm32-unknown-unknown
stellar --version

# Set RPC key
export VALIDATION_CLOUD_KEY="<your-validation-cloud-api-key>"

# Confirm admin address matches hardware wallet
stellar keys address mainnet-admin
```

### Step 1 — Build All Contracts

```bash
cargo build --target wasm32-unknown-unknown --release

# Confirm WASM artifacts exist
ls -lh target/wasm32-unknown-unknown/release/*.wasm
```

Expected output:
```
mentorminds_escrow.wasm
mentorminds_mnt_token.wasm
mentorminds_verification.wasm
```

### Step 2 — Optimize WASM

```bash
stellar contract optimize \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm

stellar contract optimize \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_verification.wasm

stellar contract optimize \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_mnt_token.wasm
```

### Step 3 — Register Mainnet Network

```bash
stellar network add mainnet \
  --rpc-url "https://mainnet.stellar.validationcloud.io/v1/${VALIDATION_CLOUD_KEY}" \
  --network-passphrase "Public Global Stellar Network ; September 2015"
```

### Step 4 — Deploy Contracts

> Hardware wallet must be connected, unlocked, and Stellar app open for each signing prompt.

```bash
# Deploy escrow
ESCROW_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm \
  --source mainnet-admin \
  --network mainnet)
echo "ESCROW_ID=$ESCROW_ID"

# Deploy verification
VERIFICATION_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_verification.wasm \
  --source mainnet-admin \
  --network mainnet)
echo "VERIFICATION_ID=$VERIFICATION_ID"

# Deploy mnt_token
TOKEN_ID=$(stellar contract deploy \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_mnt_token.wasm \
  --source mainnet-admin \
  --network mainnet)
echo "TOKEN_ID=$TOKEN_ID"
```

Expected output for each: a 56-character contract ID starting with `C`, e.g.:
```
ESCROW_ID=CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA
```

**Record all three IDs immediately** in `deployed/mainnet.json`:

```json
{
  "network": "mainnet",
  "admin": "<ADMIN_ADDRESS>",
  "treasury": "<TREASURY_ADDRESS>",
  "deployed_at": "2026-03-25T00:00:00Z",
  "escrow": "<ESCROW_ID>",
  "verification": "<VERIFICATION_ID>",
  "mnt_token": "<TOKEN_ID>"
}
```

### Step 5 — Initialize Contracts

> Replace `<ADMIN_ADDRESS>`, `<TREASURY_ADDRESS>`, and `<USDC_ADDRESS>` with real mainnet addresses.
> USDC on Stellar mainnet: `GA5ZSEJYB37JRC5AVCIA5MOP4RHTM335X2KGX3IHOJAPP5RE34K4KZVN`

```bash
# Initialize escrow
# fee_bps=500 → 5% platform fee
# auto_release_delay_secs=259200 → 72 hours
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source mainnet-admin \
  --network mainnet \
  -- initialize \
  --admin <ADMIN_ADDRESS> \
  --treasury <TREASURY_ADDRESS> \
  --fee_bps 500 \
  --approved_tokens '["<USDC_ADDRESS>"]' \
  --auto_release_delay_secs 259200
```

Expected output: `null` (void return)

```bash
# Initialize verification
stellar contract invoke \
  --id "$VERIFICATION_ID" \
  --source mainnet-admin \
  --network mainnet \
  -- initialize \
  --admin <ADMIN_ADDRESS>
```

Expected output: `null`

```bash
# Initialize mnt_token
stellar contract invoke \
  --id "$TOKEN_ID" \
  --source mainnet-admin \
  --network mainnet \
  -- initialize \
  --admin <ADMIN_ADDRESS>
```

Expected output: `null`

### Step 6 — Approve Tokens

```bash
# Approve USDC for escrow
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source mainnet-admin \
  --network mainnet \
  -- set_approved_token \
  --token <USDC_ADDRESS> \
  --approved true
```

Expected output: `null`

### Step 7 — Record Deployment

```bash
# Use the deploy script to write metadata (or update manually)
./scripts/deploy.sh --network mainnet --identity mainnet-admin
```

Commit `deployed/mainnet.json` to the repository (the file is gitignored by default — ensure it is stored securely, e.g. in a private config repo or secrets manager).

---

## 6. Post-Deploy Verification

Run all checks immediately after deployment. **Do not announce the deployment until all pass.**

### 6.1 Escrow Contract

```bash
# Verify fee is 500 bps (5%)
stellar contract invoke \
  --id "$ESCROW_ID" \
  --network mainnet \
  -- get_fee_bps
# Expected: 500

# Verify USDC is approved
stellar contract invoke \
  --id "$ESCROW_ID" \
  --network mainnet \
  -- is_token_approved \
  --token <USDC_ADDRESS>
# Expected: true
```

### 6.2 Verification Contract

```bash
# Verify admin address is not verified (sanity check)
stellar contract invoke \
  --id "$VERIFICATION_ID" \
  --network mainnet \
  -- is_verified \
  --mentor <ADMIN_ADDRESS>
# Expected: false
```

### 6.3 MNT Token Contract

```bash
# Verify token name / symbol
stellar contract invoke \
  --id "$TOKEN_ID" \
  --network mainnet \
  -- name
# Expected: "MentorMinds Token" (or configured name)

stellar contract invoke \
  --id "$TOKEN_ID" \
  --network mainnet \
  -- symbol
# Expected: "MNT"
```

### 6.4 State Verification Checklist

- [ ] `get_fee_bps` returns `500`
- [ ] `is_token_approved` returns `true` for USDC
- [ ] `is_verified` returns `false` for a fresh address (not yet verified)
- [ ] All three contract IDs visible on Stellar Expert (see [Section 7](#7-stellar-expert-links))
- [ ] No unexpected events emitted during initialization
- [ ] `deployed/mainnet.json` contains all three contract IDs and correct metadata
- [ ] Backend environment variables updated with mainnet contract IDs
- [ ] Backend smoke test: create a minimal escrow via API and confirm on-chain state

---

## 7. Stellar Expert Links

After deployment, replace `<CONTRACT_ID>` with the actual IDs from `deployed/mainnet.json`.

| Contract | Stellar Expert URL |
|----------|--------------------|
| Escrow | `https://stellar.expert/explorer/public/contract/<ESCROW_ID>` |
| Verification | `https://stellar.expert/explorer/public/contract/<VERIFICATION_ID>` |
| MNT Token | `https://stellar.expert/explorer/public/contract/<TOKEN_ID>` |
| Admin Account | `https://stellar.expert/explorer/public/account/<ADMIN_ADDRESS>` |
| Treasury Account | `https://stellar.expert/explorer/public/account/<TREASURY_ADDRESS>` |

Use these links to:
- Monitor contract invocations and events in real time
- Verify contract WASM hash matches the deployed build
- Inspect storage entries and balances

---

## 8. Rollback Procedure

> Soroban contracts are immutable once deployed — there is no on-chain "pause" built into the current escrow contract. Rollback means stopping new escrows from being created and refunding active ones.

### 8.1 Immediate Response (< 5 minutes)

1. **Stop the backend API** from creating new escrows:
   ```bash
   # Set environment variable to disable escrow creation
   # (backend must check this flag before calling create_escrow)
   ESCROW_CREATION_ENABLED=false
   ```

2. **Alert all key holders** using the escalation path in [Section 1](#1-emergency-contacts--escalation).

3. **Do not revoke admin key** until the situation is assessed — the admin key is needed to issue refunds.

### 8.2 Pause New Escrows via Fee Update

As an emergency measure, set the fee to the maximum (1000 bps = 10%) to deter new sessions while the issue is investigated. This is a soft deterrent only.

```bash
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source mainnet-admin \
  --network mainnet \
  -- update_fee \
  --new_fee_bps 1000
```

### 8.3 Refund Active Escrows

The admin can refund any escrow in `Active` or `Disputed` status. Obtain the list of active escrow IDs from the backend database or by querying on-chain events.

```bash
# Refund a single escrow by ID
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source mainnet-admin \
  --network mainnet \
  -- refund \
  --escrow_id <ESCROW_ID_NUMBER>
```

Repeat for each active escrow. The `refund` function returns funds to the learner.

**Verification after each refund**:
```bash
stellar contract invoke \
  --id "$ESCROW_ID" \
  --network mainnet \
  -- get_escrow \
  --escrow_id <ESCROW_ID_NUMBER>
# status field must show "Refunded"
```

### 8.4 Contract Upgrade (if a fix is available)

If a patched contract is ready, follow the upgrade procedure from the README:

```bash
# 1. Build the patched WASM
cargo build --target wasm32-unknown-unknown --release

# 2. Upload the new WASM (get the new wasm_hash)
WASM_HASH=$(stellar contract upload \
  --source mainnet-admin \
  --network mainnet \
  --wasm target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm)
echo "New WASM hash: $WASM_HASH"

# 3. Invoke the contract's upgrade entrypoint (admin only)
stellar contract invoke \
  --id "$ESCROW_ID" \
  --source mainnet-admin \
  --network mainnet \
  -- upgrade \
  --new_wasm_hash "$WASM_HASH"
```

After upgrade:
- Do **not** call `initialize` again (the guard will panic)
- Verify existing escrows are readable: `get_escrow` on pre-upgrade IDs
- Confirm `dispute_reason` defaults to empty symbol on old records
- Confirm `resolved_at` defaults to `0` on old records
- Test `dispute`, `resolve_dispute`, and `try_auto_release` on a pre-upgrade escrow

### 8.5 Rollback Checklist

- [ ] Backend API escrow creation disabled
- [ ] All key holders notified
- [ ] Root cause identified
- [ ] All active escrows refunded (or upgrade deployed)
- [ ] Stellar Expert confirms no unexpected contract activity
- [ ] Post-incident report drafted within 24 hours
- [ ] Users notified via platform status page

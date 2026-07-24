# Threat Model - MentorMinds Smart Contracts

**Version**: 1.0  
**Date**: March 25, 2026  
**Status**: Draft - Ready for Audit

---

## Table of Contents

1. [Executive Summary](#1-executive-summary)
2. [System Overview](#2-system-overview)
3. [Trust Boundaries](#3-trust-boundaries)
4. [Threat Actors](#4-threat-actors)
5. [Threat Model by Contract](#5-threat-model-by-contract)
6. [Attack Surface Analysis](#6-attack-surface-analysis)
7. [Security Controls](#7-security-controls)
8. [Known Limitations](#8-known-limitations)
9. [Risk Assessment](#9-risk-assessment)

---

## 1. Executive Summary

This document provides a comprehensive threat model for the MentorMinds Stellar smart contract system, which consists of four contracts:

- **Escrow Contract**: Payment escrow for mentoring sessions
- **Verification Contract**: Mentor credential verification
- **MNT Token Contract**: ERC-20-like token for platform rewards
- **Referral Contract**: Referral reward system

### Key Security Properties

- **Confidentiality**: No sensitive data stored on-chain beyond public addresses
- **Integrity**: All state changes require proper authorization
- **Availability**: Permissionless auto-release ensures funds can be recovered
- **Non-repudiation**: All operations emit events with full audit trail

---

## 2. System Overview

### 2.1 Architecture

```
┌─────────────────┐
│   Backend API   │
│  (Node.js/TS)   │
└────────┬────────┘
         │
         ▼
┌─────────────────┐      ┌──────────────────┐
│  User Wallets   │─────▶│  Escrow Contract │
│ (Learner/Mentor)│      │  (Admin: Multisig)│
└─────────────────┘      └──────────────────┘
                                │
                                ▼
                         ┌──────────────────┐
                         │ Verification Ctr │
                         │  (Admin Only)    │
                         └──────────────────┘
                                ▲
                                │
                         ┌──────────────────┐
                         │  Referral Contract│
                         │  (Mints MNT)     │
                         └──────────────────┘
                                ▲
                                │
                         ┌──────────────────┐
                         │  MNT Token Ctr   │
                         │  (Admin: Referral)│
                         └──────────────────┘
```

### 2.2 Data Flow

1. **Escrow Creation**: Learner → Approve tokens → `create_escrow()` → Tokens locked
2. **Session Completion**: Learner/Admin → `release_funds()` → Fee to treasury, net to mentor
3. **Auto-Release**: Anyone → `try_auto_release()` → After delay period
4. **Dispute**: Mentor/Learner → `dispute()` → Admin resolves → `resolve_dispute()`
5. **Verification**: Admin → `verify_mentor()` → Stores credential hash
6. **Referral**: Admin registers → User fulfills → Referrer claims MNT rewards

---

## 3. Trust Boundaries

### 3.1 Trusted Components

| Component                  | Trust Level | Capabilities                                                                            |
| -------------------------- | ----------- | --------------------------------------------------------------------------------------- |
| **Admin Address**          | High        | Update fees, treasury, approve tokens, refund escrows, resolve disputes, verify mentors |
| **Treasury Address**       | Medium      | Receives platform fees (5% typical, max 10%)                                            |
| **Backend API**            | Medium-High | Initiates transactions on behalf of users, must protect private keys                    |
| **Users (Learner/Mentor)** | Low         | Can only act on their own escrows with proper authorization                             |

### 3.2 Trust Assumptions

1. **Admin will not**:

   - Set fees above 10% (enforced by code: `MAX_FEE_BPS = 1000`)
   - Steal funds from active escrows (cannot directly withdraw)
   - Manipulate dispute resolution arbitrarily (bounded by 0-100% split)

2. **Treasury will not**:

   - Attempt to access funds beyond allocated fees
   - Interfere with escrow operations

3. **Users must**:
   - Safeguard their wallet private keys
   - Verify transaction details before signing
   - Monitor their escrow status

---

## 4. Threat Actors

### 4.1 External Attackers

| Threat      | Capability                                        | Motivation             |
| ----------- | ------------------------------------------------- | ---------------------- |
| **Hacker**  | Smart contract exploitation, reentrancy, overflow | Steal locked funds     |
| **Phisher** | Social engineering, fake UI                       | Steal user credentials |
| **MEV Bot** | Transaction ordering, front-running               | Extract value          |

### 4.2 Malicious Insiders

| Threat             | Capability              | Motivation                          |
| ------------------ | ----------------------- | ----------------------------------- |
| **Rogue Admin**    | Admin key compromise    | Steal fees, manipulate disputes     |
| **Malicious User** | Legitimate user account | Game referral system, avoid payment |

### 4.3 Negligent Actors

| Threat                | Capability               | Impact             |
| --------------------- | ------------------------ | ------------------ |
| **Careless User**     | Lost keys, wrong address | Loss of funds      |
| **Incompetent Admin** | Misconfiguration         | Service disruption |

---

## 5. Threat Model by Contract

### 5.1 Escrow Contract

#### Entry Points

| Function               | Access         | Risk Level | Description                                |
| ---------------------- | -------------- | ---------- | ------------------------------------------ |
| `initialize()`         | Once           | Critical   | Sets admin, treasury, fee, approved tokens |
| `create_escrow()`      | Learner        | High       | Locks learner's tokens                     |
| `release_funds()`      | Learner/Admin  | High       | Releases funds, deducts fee                |
| `try_auto_release()`   | Permissionless | Medium     | Auto-releases after delay                  |
| `dispute()`            | Mentor/Learner | Medium     | Pauses release, requires admin             |
| `resolve_dispute()`    | Admin          | High       | Splits funds 0-100% to mentor              |
| `refund()`             | Admin          | High       | Returns funds to learner                   |
| `update_fee()`         | Admin          | Medium     | Changes fee (capped at 10%)                |
| `update_treasury()`    | Admin          | Low        | Changes fee recipient                      |
| `set_approved_token()` | Admin          | Medium     | Adds/removes token support                 |

#### Threat Scenarios

**T1.1 - Reentrancy Attack**

- **Threat**: Attacker attempts reentrant call during token transfer
- **Mitigation**: Soroban's synchronous execution model prevents reentrancy
- **Residual Risk**: LOW

**T1.2 - Integer Overflow/Underflow**

- **Threat**: Manipulate amounts via overflow
- **Mitigation**: Rust's checked arithmetic (`checked_add`, `checked_sub`)
- **Residual Risk**: LOW

**T1.3 - Unauthorized Access**

- **Threat**: Bypass authorization checks
- **Mitigation**: `require_auth()` on all privileged operations
- **Residual Risk**: LOW (depends on Soroban SDK security)

**T1.4 - Admin Key Compromise**

- **Threat**: Attacker gains admin private key
- **Impact**: Can steal fees, manipulate disputes, refund escrows
- **Mitigation**: Use hardware wallet, multi-sig (planned)
- **Residual Risk**: MEDIUM

**T1.5 - Flash Loan Attack**

- **Threat**: Manipulate token price during operation
- **Mitigation**: Fixed-fee percentage, no price oracle dependency
- **Residual Risk**: LOW

**T1.6 - Denial of Service**

- **Threat**: Prevent legitimate operations via gas exhaustion
- **Mitigation**: Simple storage model, bounded loops
- **Residual Risk**: LOW

### 5.2 Verification Contract

#### Entry Points

| Function                | Access | Risk Level | Description               |
| ----------------------- | ------ | ---------- | ------------------------- |
| `initialize()`          | Once   | Critical   | Sets admin                |
| `verify_mentor()`       | Admin  | Medium     | Stores credential hash    |
| `revoke_verification()` | Admin  | Low        | Deactivates verification  |
| `is_verified()`         | Public | Low        | Query verification status |

#### Threat Scenarios

**T2.1 - False Verification**

- **Threat**: Admin verifies unqualified mentor
- **Mitigation**: Governance/process controls (off-chain)
- **Residual Risk**: MEDIUM (business logic risk)

**T2.2 - Credential Hash Collision**

- **Threat**: Two different credentials produce same hash
- **Mitigation**: SHA-256 (32 bytes) makes collision infeasible
- **Residual Risk**: LOW

### 5.3 MNT Token Contract

#### Entry Points

| Function                        | Access       | Risk Level | Description          |
| ------------------------------- | ------------ | ---------- | -------------------- |
| `initialize()`                  | Once         | Critical   | Sets admin, metadata |
| `mint()`                        | Admin        | High       | Creates new tokens   |
| `burn()`                        | Token holder | Medium     | Destroys tokens      |
| `transfer()`                    | Token holder | Medium     | Moves tokens         |
| `approve()` / `transfer_from()` | Spender      | Medium     | Delegated transfers  |

#### Threat Scenarios

**T3.1 - Unlimited Minting**

- **Threat**: Admin mints excessive tokens
- **Mitigation**: Hard cap (`SUPPLY_CAP = 100M tokens`)
- **Residual Risk**: LOW

**T3.2 - Approval Race Condition**

- **Threat**: Spender uses allowance after owner revokes
- **Mitigation**: Standard ERC-20 allowance pattern
- **Residual Risk**: LOW (known behavior)

### 5.4 Referral Contract

#### Entry Points

| Function              | Access   | Risk Level | Description                             |
| --------------------- | -------- | ---------- | --------------------------------------- |
| `initialize()`        | Once     | Critical   | Sets admin, MNT token                   |
| `register_referral()` | Admin    | Medium     | Records referral relationship           |
| `fulfill_referral()`  | Admin    | Medium     | Marks referral complete, accrues reward |
| `claim_reward()`      | Referrer | High       | Mints MNT tokens to referrer            |

#### Threat Scenarios

**T4.1 - Self-Referral**

- **Threat**: User refers themselves to claim rewards
- **Mitigation**: Explicit check: `referrer == referee` panics
- **Residual Risk**: LOW

**T4.2 - Double Counting**

- **Threat**: Same referee registered twice
- **Mitigation**: Storage check prevents duplicate registration
- **Residual Risk**: LOW

**T4.3 - Reward Manipulation**

- **Threat**: Admin falsifies fulfillment
- **Mitigation**: Admin trust assumption, on-chain audit trail
- **Residual Risk**: MEDIUM

---

## 6. Attack Surface Analysis

### 6.1 Privileged Functions

| Contract     | Function               | Privilege | Impact if Compromised         |
| ------------ | ---------------------- | --------- | ----------------------------- |
| Escrow       | `update_fee()`         | Admin     | Fee increase up to 10%        |
| Escrow       | `update_treasury()`    | Admin     | Redirect future fees          |
| Escrow       | `resolve_dispute()`    | Admin     | Arbitrary fund split (0-100%) |
| Escrow       | `refund()`             | Admin     | Return funds to learner       |
| Escrow       | `set_approved_token()` | Admin     | Enable/disable tokens         |
| Verification | `verify_mentor()`      | Admin     | Grant verified status         |
| MNT Token    | `mint()`               | Admin     | Create tokens (up to cap)     |
| Referral     | `register_referral()`  | Admin     | Record referral               |
| Referral     | `fulfill_referral()`   | Admin     | Accrue rewards                |

### 6.2 External Dependencies

| Dependency                       | Type       | Risk if Compromised                    |
| -------------------------------- | ---------- | -------------------------------------- |
| **Soroban SDK**                  | Framework  | Critical - could affect all contracts  |
| **Stellar Network**              | Blockchain | Critical - consensus/security          |
| **Token Contracts (USDC, etc.)** | External   | Medium - could block escrow creation   |
| **Backend API**                  | Off-chain  | Medium - key management, user guidance |

### 6.3 State Variables

| Contract  | Variable        | Sensitivity | Attack Impact                     |
| --------- | --------------- | ----------- | --------------------------------- |
| Escrow    | `ADMIN`         | Critical    | Full control over admin functions |
| Escrow    | `TREASURY`      | Medium      | Fee redirection                   |
| Escrow    | `FEE_BPS`       | Medium      | Fee manipulation                  |
| Escrow    | Escrow balances | High        | Fund theft/manipulation           |
| MNT Token | Total supply    | High        | Inflation attack                  |
| Referral  | Pending rewards | Medium      | Reward manipulation               |

---

## 7. Security Controls

### 7.1 Authorization

- **User-level**: `require_auth()` on all user actions
- **Admin-level**: Single admin address (hardware wallet recommended)
- **No proxy patterns**: Direct contract calls only

### 7.2 Input Validation

- Amount validation: `amount > 0` checks
- Percentage bounds: `mentor_pct <= 100`
- Token approval: Whitelist mechanism
- Supply cap: Hard limit enforced

### 7.3 Error Handling

- Panic on invalid state (double-init, double-release)
- Checked arithmetic throughout
- Status machine enforcement (Active → Released/Refunded/Resolved)

### 7.4 Economic Controls

- Fee cap: Maximum 10% (1000 bps)
- Supply cap: 100M MNT tokens
- Auto-release delay: Configurable (default 72h)

### 7.5 Monitoring & Audit

- Events emitted for all state changes:
  - `created`, `released`, `auto_released`
  - `disp_opnd`, `disp_res` (dispute opened/resolved)
  - `refunded`, `verified`, `revoked`
  - `mint`, `burn`, `transfer`
- Persistent storage with TTL extension
- Test coverage: 50+ unit tests per contract

---

## 8. Known Limitations

### 8.1 Design Decisions

1. **Single Admin Key**:

   - Current implementation uses single admin address
   - Multi-sig wallet planned but not yet implemented
   - **Mitigation**: Hardware wallet requirement documented

2. **No Emergency Pause**:

   - Contracts lack global pause mechanism
   - **Rationale**: Soroban best practices favor explicit controls
   - **Mitigation**: Fee increase can deter new escrows; refunds available

3. **Simple Token Model**:

   - MNT token uses basic allowance without expiration
   - **Mitigation**: Acceptable for MVP; enhancement possible

4. **Admin-Controlled Verification**:
   - Centralized verification process
   - **Rationale**: Required for KYC/compliance
   - **Mitigation**: Process controls off-chain

### 8.2 Technical Constraints

1. **Storage TTL**:

   - Entries require TTL extension to persist
   - **Mitigation**: `extend_ttl()` calls on every access

2. **No Cross-Contract Calls**:

   - Referral contract mints MNT via client call
   - **Risk**: Tight coupling between contracts
   - **Mitigation**: Careful initialization order

3. **Symbol Length**:
   - Dispute reasons limited to 6 chars (Symbol type)
   - **Impact**: Limited expressiveness
   - **Mitigation**: Predefined reason codes

---

## 9. Risk Assessment

### 9.1 Risk Matrix

| Threat                 | Likelihood | Impact   | Risk Level | Priority |
| ---------------------- | ---------- | -------- | ---------- | -------- |
| Admin key compromise   | Medium     | Critical | HIGH       | P0       |
| Smart contract exploit | Low        | Critical | MEDIUM     | P1       |
| Reentrancy attack      | Low        | High     | LOW        | P2       |
| Oracle manipulation    | N/A        | N/A      | N/A        | N/A      |
| DOS via gas            | Low        | Medium   | LOW        | P2       |
| Phishing users         | Medium     | Medium   | MEDIUM     | P1       |
| Self-referral fraud    | Low        | Low      | LOW        | P3       |

### 9.2 Risk Treatment

**HIGH Priority (P0)**:

- Implement multi-sig admin wallet
- Require hardware wallet for admin operations
- Document key custody procedures

**MEDIUM Priority (P1)**:

- Complete external security audit
- Add emergency contact procedures
- Implement monitoring/alerting

**LOW Priority (P2-P3)**:

- Code optimization for gas efficiency
- Enhanced event logging
- User education materials

---

## 10. Recommendations for Auditors

### Focus Areas

1. **Authorization Logic**: Verify `require_auth()` cannot be bypassed
2. **Fee Calculation**: Ensure truncation doesn't create dust attacks
3. **Dispute Resolution**: Check for edge cases in percentage splits
4. **Auto-Release**: Validate timestamp handling and boundary conditions
5. **Cross-Contract Calls**: Verify referral → MNT mint interaction
6. **Upgrade Path**: Assess WASM upgrade mechanism safety

### Testing Requests

- Fuzz testing on amount calculations
- Formal verification of state machine transitions
- Gas profiling under worst-case scenarios
- Dependency vulnerability scan

---

## Appendix A: Contract Addresses

_To be filled post-deployment_

| Contract     | Network | Address |
| ------------ | ------- | ------- |
| Escrow       | Testnet | TBD     |
| Verification | Testnet | TBD     |
| MNT Token    | Testnet | TBD     |
| Referral     | Testnet | TBD     |

---

## Appendix B: Changelog

| Version | Date       | Author        | Changes       |
| ------- | ---------- | ------------- | ------------- |
| 1.0     | 2026-03-25 | Security Team | Initial draft |

# Trust Assumptions - MentorMinds Smart Contracts

**Version**: 1.0  
**Date**: March 25, 2026

---

## Overview

This document outlines the trust assumptions for users, developers, and auditors of the MentorMinds Stellar smart contract system. Understanding these assumptions is critical for assessing system security and making informed decisions about participation.

---

## 1. Central Authority (Admin) Trust Assumptions

### 1.1 Admin Capabilities

The admin address holds significant power over the escrow contract ecosystem:

| Capability                | Scope                                            | Constraints                                    |
| ------------------------- | ------------------------------------------------ | ---------------------------------------------- |
| **Update Platform Fee**   | Can change fee from 0% to 10%                    | Hard cap at 1000 bps (10%) enforced by code    |
| **Update Treasury**       | Can redirect fee recipient                       | No constraints; can be any address             |
| **Approve/Remove Tokens** | Controls which tokens can be used                | Must satisfy SEP-41 interface                  |
| **Refund Escrows**        | Can refund Active or Disputed escrows to learner | Cannot refund Released or Resolved escrows     |
| **Resolve Disputes**      | Can split disputed funds 0-100% to mentor        | Must be in Disputed status; percentage bounded |
| **Initialize Contract**   | Sets initial parameters                          | One-time call; cannot be re-initialized        |

### 1.2 What Users Must Trust

Users **MUST** trust that the admin will:

1. **Not abuse fee power**: While capped at 10%, the admin could theoretically raise fees to the maximum
2. **Act fairly in disputes**: Admin has discretion in dispute resolution (0-100% split)
3. **Safeguard admin key**: Key compromise would allow attacker to perform all admin actions
4. **Not collude with treasury**: Treasury address receives all fees; admin controls this address

### 1.3 What Users Do NOT Need to Trust

Users **DO NOT** need to trust that the admin will:

1. **Not steal principal**: Admin cannot directly withdraw funds from escrows
2. **Manipulate balances**: Token transfers follow deterministic fee logic
3. **Block auto-release**: Permissionless auto-release works regardless of admin action
4. **Change past agreements**: Terms set at escrow creation are immutable

### 1.4 Admin Key Security Requirements

**MANDATORY FOR MAINNET**:

- ✅ Hardware wallet required (Ledger Nano X/S+ recommended)
- ✅ Seed phrase stored on metal plate in fireproof safe
- ✅ Backup hardware wallet with same seed stored offsite
- ❌ Software wallets NOT permitted for mainnet admin
- ❌ Seed phrase NEVER entered into any computer

---

## 2. Backend System Trust Assumptions

### 2.1 Backend Capabilities

The MentorMinds backend (Node.js/TypeScript API) interacts with the smart contracts:

| Function                   | Description                                         | Trust Required |
| -------------------------- | --------------------------------------------------- | -------------- |
| **Transaction Submission** | Submits user-signed transactions to Stellar network | Medium         |
| **Key Management**         | May hold user keys if using custodial wallet        | High           |
| **UI/UX Guidance**         | Suggests gas prices, transaction parameters         | Medium         |
| **Database Tracking**      | Maintains off-chain index of escrows                | Low            |

### 2.2 What Users Must Trust

If using the **official MentorMinds platform**:

1. **Private key security** (if custodial): Backend properly secures user keys
2. **Transaction integrity**: Backend submits correct transactions as requested
3. **No front-running**: Backend doesn't exploit knowledge of pending transactions
4. **Data accuracy**: Backend correctly displays escrow status and balances

### 2.3 What Users Do NOT Need to Trust

Even when using the platform:

1. **Fund custody**: Users can hold their own keys and interact directly
2. **Contract execution**: Smart contracts enforce rules automatically
3. **Access to funds**: Users can always call `release_funds` or `try_auto_release` directly

### 2.4 Self-Custody Option

Users may **bypass the backend entirely** by:

- Using StellarX, Lobstr, or other Stellar wallets
- Interacting directly via Stellar CLI
- Calling contract methods through Stellar Laboratory

**Smart contracts are non-custodial** — the backend is optional for core functionality.

---

## 3. Third-Party Dependencies

### 3.1 Stellar Network

**Trust Assumptions**:

- ✅ Stellar validators maintain consensus honestly
- ✅ Soroban runtime executes contracts correctly
- ✅ Network remains available and decentralized

**Risks**:

- Network congestion could delay transactions
- Protocol upgrades could affect contract behavior
- Validator collusion (>67%) could rewrite history

**Mitigation**: Stellar's proven track record (operational since 2014)

### 3.2 Token Contracts (USDC, PYUSD, etc.)

**Trust Assumptions**:

- ✅ Approved tokens implement SEP-41 correctly
- ✅ Token contracts don't have hidden transfer restrictions
- ✅ Token issuers maintain peg/stability

**Risks**:

- Token contract bugs could block transfers
- Issuer could freeze addresses (USDC-specific)
- Depeg events could affect escrow value

**Mitigation**: Only approve reputable tokens; monitor issuer policies

### 3.3 Soroban SDK

**Trust Assumptions**:

- ✅ SDK correctly implements authorization checks
- ✅ No vulnerabilities in cryptographic primitives
- ✅ Storage TTL mechanism works as documented

**Risks**:

- SDK bugs could affect all Soroban contracts
- Breaking changes between versions

**Mitigation**: Use stable SDK versions; monitor Soroban security advisories

---

## 4. User Responsibilities

### 4.1 What Users Control

Users have **full control** over:

1. **Escrow Creation**: Choose mentor, amount, session end time
2. **Fund Release**: Decide when to release (after session completion)
3. **Dispute Initiation**: Open dispute if issues arise
4. **Wallet Security**: Safeguard private keys

### 4.2 What Users Cannot Control

Users **cannot**:

1. **Unilaterally retrieve funds**: Once locked in Active escrow
2. **Force release before conditions met**: Unless admin refunds
3. **Change fee percentage**: Set at initialization by admin
4. **Modify smart contract code**: Contracts are immutable post-deployment

### 4.3 Recommended User Practices

1. **Verify counterparty**: Check mentor verification status before creating escrow
2. **Set appropriate session_end_time**: Allow buffer for auto-release
3. **Monitor escrow status**: Track via Stellar Expert or platform UI
4. **Use approved tokens only**: Stick to USDC, PYUSD, or admin-approved tokens
5. **Save transaction hashes**: Record for dispute evidence if needed

---

## 5. Economic Incentives & Game Theory

### 5.1 Honest Behavior Incentives

| Actor        | Honest Incentive           | Dishonest Consequence                    |
| ------------ | -------------------------- | ---------------------------------------- |
| **Learner**  | Pay for completed sessions | Reputation damage; no future mentors     |
| **Mentor**   | Deliver quality sessions   | Dispute risk; loss of verified status    |
| **Admin**    | Fair dispute resolution    | Platform reputation; long-term viability |
| **Treasury** | Collect fees over time     | Killing platform kills fee revenue       |

### 5.2 Attack Economics

**Attacking as Learner**:

- Cost: Lose escrowed funds if dishonestly disputed
- Benefit: Potentially free session (if admin rules in favor)
- Risk: Mentor dispute + evidence → learner loses

**Attacking as Mentor**:

- Cost: Reputation loss; verification revocation
- Benefit: Payment for incomplete sessions (if admin rules in favor)
- Risk: Learner dispute + evidence → mentor loses

**Attacking as Admin**:

- Cost: Platform destruction; legal liability
- Benefit: Steal fees (capped at 10%) or manipulate one dispute
- Risk: Community backlash; potential legal action

### 5.3 Why Attacks Are Unlikely

1. **Reputation Capital**: Platform value exceeds single-session gains
2. **Transparent Audit Trail**: All actions visible on-chain
3. **Economic Alignment**: All parties benefit from successful sessions
4. **Limited Admin Power**: Cannot steal principal; only influence disputes

---

## 6. Trust Minimization Roadmap

### Current State (v1.0) - Centralized Admin

- Single admin address controls key functions
- Hardware wallet required for mainnet
- Full audit trail on-chain

### Near Term (v1.1) - Multi-Sig Admin

- Replace single admin with 2-of-3 or 3-of-5 multi-sig
- Time-lock for critical changes (fee updates)
- Community input on dispute resolution

### Medium Term (v2.0) - Decentralized Governance

- DAO governance for parameter changes
- Decentralized arbitration for disputes
- Community-curated token approval list

### Long Term Vision

- Fully non-custodial platform
- No privileged admin role
- Pure code-based governance

---

## 7. Comparison to Traditional Systems

### MentorMinds vs. PayPal Escrow

| Aspect                 | PayPal                  | MentorMinds                  |
| ---------------------- | ----------------------- | ---------------------------- |
| **Custody**            | PayPal holds funds      | Smart contract holds funds   |
| **Dispute Resolution** | PayPal decides (opaque) | Admin decides (transparent)  |
| **Fees**               | ~3-5% + fixed           | 0-10% (configurable)         |
| **Access**             | Permissioned (KYC)      | Permissionless (wallet only) |
| **Transparency**       | Private                 | Fully public on-chain        |

### MentorMinds vs. Direct Payment

| Aspect             | Direct Payment      | MentorMinds Escrow        |
| ------------------ | ------------------- | ------------------------- |
| **Timing**         | Immediate           | Delayed until release     |
| **Risk (Learner)** | High (prepayment)   | Low (funds protected)     |
| **Risk (Mentor)**  | Low (paid upfront)  | Medium (performance risk) |
| **Dispute Option** | Chargeback (costly) | On-chain resolution       |

---

## 8. Regulatory Considerations

### 8.1 What This Is

- **Software Infrastructure**: Smart contracts for conditional payments
- **Non-custodial**: Platform doesn't hold user funds directly
- **Tool, Not Financial Product**: Users control their own transactions

### 8.2 What This Is NOT

- ❌ Not a bank (no fractional reserve lending)
- ❌ Not a money transmitter (no custody)
- ❌ Not an investment contract (no profit expectation)
- ❌ Not insurance (no risk pooling)

### 8.3 Compliance Responsibilities

**Users**:

- Responsible for own tax reporting
- Must comply with local laws on cryptocurrency use
- Should verify mentor credentials independently

**Platform**:

- Terms of service prohibit illegal use
- Cooperation with law enforcement (if legally required)
- No guarantee of regulatory compliance in all jurisdictions

---

## 9. Worst-Case Scenarios

### Scenario 1: Admin Key Compromise

**What Happens**:

- Attacker gains control of admin private key
- Can resolve disputes dishonestly, update treasury

**User Impact**:

- Active escrows still protected (attacker can't steal directly)
- Disputed escrows at risk (attacker controls resolution)
- Future fees redirected to attacker's treasury

**Mitigation**:

- Switch to new admin address via contract upgrade
- Multi-sig reduces single point of failure
- Hardware wallet makes remote compromise harder

### Scenario 2: Smart Contract Bug

**What Happens**:

- Critical vulnerability discovered (e.g., authorization bypass)
- Attacker exploits and drains escrows

**User Impact**:

- Loss of locked funds
- No recourse for recovery

**Mitigation**:

- External security audit before mainnet
- Bug bounty program
- Upgrade mechanism for patches
- Insurance fund (future consideration)

### Scenario 3: Platform Shutdown

**What Happens**:

- MentorMinds team abandons project
- Backend API goes offline

**User Impact**:

- UI unavailable for creating escrows
- Cannot contact support

**Good News**:

- **Smart contracts remain operational**
- Users can still interact directly via CLI/Laboratory
- Funds not lost; auto-release still works
- Open-source code allows community fork

---

## 10. Summary: Trust Hierarchy

```
┌─────────────────────────────────────┐
│  TRUSTLESS (Code Enforced)          │
│  - Escrow terms immutable           │
│  - Auto-release permissionless      │
│  - Fee cap hard-coded               │
└─────────────────────────────────────┘
              ▲
              │
┌─────────────────────────────────────┐
│  MINIMAL TRUST (Economic Alignment) │
│  - Admin won't destroy platform     │
│  - Treasury won't kill fee revenue  │
│  - Users act rationally             │
└─────────────────────────────────────┘
              ▲
              │
┌─────────────────────────────────────┐
│  MODERATE TRUST (Security Practices)│
│  - Admin safeguards private key     │
│  - Backend protects user data       │
│  - Stellar network remains secure   │
└─────────────────────────────────────┘
```

---

## 11. Questions for Users to Consider

Before using MentorMinds, ask yourself:

1. ✅ Do I understand that the admin can resolve disputes but cannot steal my funds?
2. ✅ Am I comfortable with a 0-10% fee that the admin can adjust?
3. ✅ Have I verified my counterparty (mentor/learner) reputation?
4. ✅ Do I understand how to access funds if the platform goes offline?
5. ✅ Am I using a secure wallet (hardware wallet recommended)?
6. ✅ Have I recorded transaction hashes for my escrows?
7. ✅ Do I understand the auto-release delay period?

If you answered "no" to any question, seek clarification before proceeding.

---

## Appendix: Contact & Support

For questions about trust assumptions:

- Email: security@mentorminds.io
- Documentation: https://docs.mentorminds.io
- Community: Discord, Telegram

**Remember**: When in doubt, test with small amounts first.

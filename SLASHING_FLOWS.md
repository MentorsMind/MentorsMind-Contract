# Slashing Mechanism - Flow Diagrams

## Overview
This document provides visual representations of the slashing mechanism flows, authorization paths, and integration points.

---

## 1. High-Level Architecture

```
┌─────────────────────────────────────────────────────────────────┐
│                     MentorsMind Protocol                         │
├─────────────────────────────────────────────────────────────────┤
│                                                                   │
│  ┌──────────────┐       ┌──────────────┐      ┌──────────────┐ │
│  │   Multisig   │       │  Governance  │      │  Insurance   │ │
│  │    Admin     │       │   Contract   │      │     Pool     │ │
│  └──────┬───────┘       └──────┬───────┘      └──────▲───────┘ │
│         │                      │                      │         │
│         │    Authorization     │                      │         │
│         └──────────┬───────────┘                      │         │
│                    │                                  │         │
│                    ▼                                  │         │
│         ┌─────────────────────┐              Slashed │         │
│         │  Staking Contract   │              Tokens  │         │
│         │   (with Slashing)   ├──────────────────────┘         │
│         └─────────┬───────────┘                                │
│                   │                                             │
│                   │ Stake/Unstake                               │
│                   ▼                                             │
│         ┌─────────────────────┐                                │
│         │    MNT Token        │                                │
│         │  (Stellar Asset)    │                                │
│         └─────────────────────┘                                │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

---

## 2. Slashing Flow - Via Multisig

```
┌──────────┐         ┌──────────────┐         ┌──────────────┐         ┌──────────────┐
│  Admin   │         │  Multisig    │         │   Staking    │         │  Insurance   │
│          │         │   Contract   │         │   Contract   │         │     Pool     │
└────┬─────┘         └──────┬───────┘         └──────┬───────┘         └──────┬───────┘
     │                      │                        │                        │
     │ 1. Propose Slash     │                        │                        │
     ├─────────────────────>│                        │                        │
     │                      │                        │                        │
     │ 2. Sign Proposal     │                        │                        │
     ├─────────────────────>│                        │                        │
     │                      │                        │                        │
     │ 3. Sign (Reach       │                        │                        │
     │    Threshold)        │                        │                        │
     ├─────────────────────>│                        │                        │
     │                      │                        │                        │
     │                      │ 4. Execute Proposal    │                        │
     │                      ├───────────────────────>│                        │
     │                      │                        │                        │
     │                      │    5. Verify Approval  │                        │
     │                      │<───────────────────────┤                        │
     │                      │    (is_executed())     │                        │
     │                      │                        │                        │
     │                      │    6. Approval OK      │                        │
     │                      ├───────────────────────>│                        │
     │                      │                        │                        │
     │                      │                        │ 7. Calculate Slash     │
     │                      │                        │    Amount              │
     │                      │                        │                        │
     │                      │                        │ 8. Update Stake        │
     │                      │                        │    & Tier              │
     │                      │                        │                        │
     │                      │                        │ 9. Transfer Tokens     │
     │                      │                        ├───────────────────────>│
     │                      │                        │                        │
     │                      │                        │ 10. Record History     │
     │                      │                        │                        │
     │                      │                        │ 11. Emit Event         │
     │                      │                        │                        │
     │                      │    12. Success         │                        │
     │                      │<───────────────────────┤                        │
     │                      │                        │                        │
     │  13. Success         │                        │                        │
     │<─────────────────────┤                        │                        │
     │                      │                        │                        │
```

---

## 3. Slashing Flow - Via Governance

```
┌──────────┐         ┌──────────────┐         ┌──────────────┐         ┌──────────────┐
│  User    │         │ Governance   │         │   Staking    │         │  Insurance   │
│          │         │   Contract   │         │   Contract   │         │     Pool     │
└────┬─────┘         └──────┬───────┘         └──────┬───────┘         └──────┬───────┘
     │                      │                        │                        │
     │ 1. Create Proposal   │                        │                        │
     ├─────────────────────>│                        │                        │
     │                      │                        │                        │
     │ 2. Cast Vote         │                        │                        │
     ├─────────────────────>│                        │                        │
     │                      │                        │                        │
     │ 3. Vote (Others)     │                        │                        │
     ├─────────────────────>│                        │                        │
     │                      │                        │                        │
     │ 4. Finalize Proposal │                        │                        │
     │    (After Period)    │                        │                        │
     ├─────────────────────>│                        │                        │
     │                      │                        │                        │
     │                      │ 5. Execute Slash       │                        │
     │                      ├───────────────────────>│                        │
     │                      │                        │                        │
     │                      │    6. Verify Proposal  │                        │
     │                      │<───────────────────────┤                        │
     │                      │    (get_status())      │                        │
     │                      │                        │                        │
     │                      │    7. Status Executed  │                        │
     │                      ├───────────────────────>│                        │
     │                      │                        │                        │
     │                      │                        │ 8. Calculate Slash     │
     │                      │                        │    Amount              │
     │                      │                        │                        │
     │                      │                        │ 9. Update Stake        │
     │                      │                        │    & Tier              │
     │                      │                        │                        │
     │                      │                        │ 10. Transfer Tokens    │
     │                      │                        ├───────────────────────>│
     │                      │                        │                        │
     │                      │                        │ 11. Record History     │
     │                      │                        │                        │
     │                      │                        │ 12. Emit Event         │
     │                      │                        │                        │
     │                      │    13. Success         │                        │
     │                      │<───────────────────────┤                        │
     │                      │                        │                        │
     │  14. Success         │                        │                        │
     │<─────────────────────┤                        │                        │
     │                      │                        │                        │
```

---

## 4. Authorization Decision Tree

```
                     ┌─────────────────┐
                     │  slash() Called │
                     └────────┬─────────┘
                              │
                              ▼
                 ┌────────────────────────┐
                 │ Validate slash_bps     │
                 │ (0 < bps ≤ 5000)       │
                 └────────┬───────────────┘
                          │
                          ▼
            ┌─────────────────────────────┐
            │ Check multisig_proposal_id  │
            └────────┬────────────────────┘
                     │
         ┌───────────┴──────────┐
         │                      │
         ▼                      ▼
    ┌─────────┐           ┌─────────┐
    │ Present │           │  Absent │
    └────┬────┘           └────┬────┘
         │                     │
         ▼                     ▼
    ┌──────────────────┐  ┌────────────────────────┐
    │ Verify Multisig  │  │ Check governance_id    │
    │ is_executed()    │  └────────┬───────────────┘
    └────┬─────────────┘           │
         │              ┌──────────┴──────────┐
         ▼              ▼                     ▼
    ┌─────────┐    ┌─────────┐          ┌─────────┐
    │ Approved│    │ Present │          │  Absent │
    └────┬────┘    └────┬────┘          └────┬────┘
         │              │                     │
         │              ▼                     ▼
         │    ┌──────────────────┐    ┌──────────────┐
         │    │ Verify Governance│    │    REJECT    │
         │    │ get_status()     │    │ (No Approval)│
         │    └────┬─────────────┘    └──────────────┘
         │         │
         │         ▼
         │    ┌─────────┐
         │    │ Approved│
         │    └────┬────┘
         │         │
         └─────────┴─────────┐
                             ▼
                    ┌─────────────────┐
                    │  PROCEED WITH   │
                    │     SLASH       │
                    └─────────────────┘
```

---

## 5. Slash Amount Calculation Flow

```
┌─────────────────────────────────────────────────────────────────┐
│                    Slash Calculation Process                     │
└─────────────────────────────────────────────────────────────────┘

Input:
├─ current_stake: i128  (e.g., 1000 tokens)
├─ slash_bps: u32       (e.g., 1000 = 10%)
└─ current_tier: u32    (e.g., 2 = Silver)

                              │
                              ▼
                    ┌──────────────────┐
                    │  Calculate Slash │
                    │  slash_amount =  │
                    │  (stake * bps)   │
                    │    / 10,000      │
                    └────────┬─────────┘
                             │
                             ▼
                  Example: (1000 * 1000) / 10,000 = 100
                             │
                             ▼
                    ┌──────────────────┐
                    │  Calculate New   │
                    │  new_amount =    │
                    │  stake - slash   │
                    └────────┬─────────┘
                             │
                             ▼
                  Example: 1000 - 100 = 900
                             │
                             ▼
                    ┌──────────────────┐
                    │  Compute New Tier│
                    │  if amt >= 2000  │
                    │    tier = 3      │
                    │  elif amt >= 500 │
                    │    tier = 2      │
                    │  elif amt >= 100 │
                    │    tier = 1      │
                    │  else tier = 0   │
                    └────────┬─────────┘
                             │
                             ▼
                  Example: 900 >= 500 → tier = 2
                             │
                             ▼
                    ┌──────────────────┐
                    │  Update Storage  │
                    │  record.amount   │
                    │  record.tier     │
                    │  total_staked    │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │  Transfer to     │
                    │  Insurance Pool  │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │  Record in       │
                    │  SlashHistory    │
                    └────────┬─────────┘
                             │
                             ▼
                    ┌──────────────────┐
                    │  Emit Event      │
                    └──────────────────┘

Output:
├─ new_amount: 900 tokens
├─ new_tier: 2 (Silver, unchanged)
├─ slash_amount: 100 tokens
└─ insurance_received: 100 tokens
```

---

## 6. State Transitions

```
                        Mentor Stake States
                        
┌─────────────┐  stake()   ┌─────────────┐  unlock_at  ┌─────────────┐
│   NO STAKE  ├───────────>│   STAKED    ├────────────>│  UNLOCKED   │
└─────────────┘            │  (LOCKED)   │  expires    └──────┬──────┘
                           └──────┬──────┘                     │
                                  │                            │
                                  │ slash()                    │ unstake()
                                  ▼                            │
                           ┌─────────────┐                     │
                           │   SLASHED   │                     │
                           │  (LOCKED)   │                     │
                           └──────┬──────┘                     │
                                  │                            │
                                  │ unlock_at                  │
                                  │ expires                    │
                                  ▼                            │
                           ┌─────────────┐                     │
                           │  SLASHED &  │                     │
                           │  UNLOCKED   ├─────────────────────┘
                           └──────┬──────┘                     │
                                  │                            │
                                  │ unstake()                  │
                                  ▼                            ▼
                           ┌──────────────────────────────────────┐
                           │            NO STAKE                  │
                           └──────────────────────────────────────┘


Tier Transitions (Example: 1000 tokens, Silver)

     Tier 2 (Silver)
     1000 tokens
          │
          │ slash 10% (100 tokens)
          ▼
     Tier 2 (Silver)
     900 tokens
          │
          │ slash 50% (450 tokens)
          ▼
     Tier 1 (Bronze)
     450 tokens
          │
          │ slash 50% (225 tokens)
          ▼
     Tier 1 (Bronze)
     225 tokens
          │
          │ slash 50% (112 tokens)
          ▼
     Tier 1 (Bronze)
     113 tokens
          │
          │ slash 10% (11 tokens)
          ▼
     Tier 1 (Bronze)
     102 tokens
          │
          │ slash 2% (2 tokens)
          ▼
     Tier 0 (None)
     100 tokens
```

---

## 7. Cross-Contract Call Flow

```
┌────────────────────────────────────────────────────────────────────┐
│                    Cross-Contract Interactions                      │
└────────────────────────────────────────────────────────────────────┘

                         ┌──────────────┐
                         │   Staking    │
                         │   Contract   │
                         └──────┬───────┘
                                │
                   ┌────────────┼────────────┐
                   │            │            │
                   ▼            ▼            ▼
          ┌────────────┐  ┌──────────┐  ┌──────────┐
          │ Multisig   │  │Governance│  │Insurance │
          │  Admin     │  │ Contract │  │   Pool   │
          └─────┬──────┘  └────┬─────┘  └────┬─────┘
                │              │             │
                ▼              ▼             ▼
    ┌────────────────┐ ┌──────────────┐ ┌────────────┐
    │is_executed()   │ │get_proposal_ │ │transfer()  │
    │                │ │status()      │ │(via token) │
    │Returns: bool   │ │Returns: u32  │ │            │
    └────────────────┘ └──────────────┘ └────────────┘


Verification Flow:

    Staking.slash()
         │
         ├─────────────────────────────────┐
         │                                 │
         ▼                                 ▼
    Has multisig_id?                  Has gov_id?
         │                                 │
         ├─ Yes ─> invoke_contract()      ├─ Yes ─> invoke_contract()
         │         (multisig, "is_executed")│        (gov, "get_proposal_status")
         │              │                  │              │
         │              ▼                  │              ▼
         │         Returns bool            │         Returns u32
         │              │                  │              │
         │              ▼                  │              ▼
         │         if true ─> OK           │         if executed ─> OK
         │                                 │
         └────────┬────────────────────────┘
                  │
                  ▼
            At least one OK?
                  │
          ├───────┴───────┐
          │               │
          ▼               ▼
         Yes             No
          │               │
          ▼               ▼
      PROCEED         REJECT
```

---

## 8. Data Flow Diagram

```
┌─────────────────────────────────────────────────────────────────┐
│                        Data Flow                                 │
└─────────────────────────────────────────────────────────────────┘

Input Data (slash function)
┌────────────────────────────┐
│ caller: Address            │
│ mentor: Address            │
│ slash_bps: u32             │
│ slash_reason: Symbol       │
│ multisig_proposal_id: ?    │
│ governance_proposal_id: ?  │
└─────────────┬──────────────┘
              │
              ▼
┌─────────────────────────────┐
│  Authorization Validation   │
│  - Check multisig           │
│  - Check governance         │
└─────────────┬───────────────┘
              │
              ▼
┌─────────────────────────────┐
│   Read from Storage         │
│   - StakeRecord(mentor)     │
│   - TotalStaked             │
└─────────────┬───────────────┘
              │
              ▼
┌─────────────────────────────┐
│   Compute New Values        │
│   - slash_amount            │
│   - new_amount              │
│   - new_tier                │
└─────────────┬───────────────┘
              │
              ▼
┌─────────────────────────────┐
│   Write to Storage          │
│   - Update StakeRecord      │
│   - Update TotalStaked      │
│   - Append SlashHistory     │
└─────────────┬───────────────┘
              │
              ▼
┌─────────────────────────────┐
│   External Call             │
│   - Transfer to Insurance   │
└─────────────┬───────────────┘
              │
              ▼
┌─────────────────────────────┐
│   Emit Event                │
│   - SlashedEventData        │
└─────────────────────────────┘

Output
┌────────────────────────────┐
│ Updated stake record       │
│ Reduced tier (if needed)   │
│ Tokens in insurance pool   │
│ Slash history entry        │
│ Event emitted              │
└────────────────────────────┘
```

---

## 9. Error Handling Flow

```
                    slash() Entry
                         │
         ┌───────────────┼───────────────┐
         ▼               ▼               ▼
    Validation 1    Validation 2    Validation 3
    (slash_bps)     (approval)      (stake exists)
         │               │               │
    ┌────┴────┐     ┌────┴────┐     ┌────┴────┐
    │         │     │         │     │         │
    ▼         ▼     ▼         ▼     ▼         ▼
  Valid    Invalid  Valid   Invalid Found   Not Found
    │         │      │         │      │         │
    │         └──────┼─────────┼──────┼─────────┤
    │                │         │      │         │
    │                ▼         ▼      │         ▼
    │           SlashExceedsMax  │    │    NoStakeFound
    │           InvalidSlashBps  │    │
    │           NoMultisigApproval    │
    │                            │    │
    └────────────────────────────┴────┘
                    │
                    ▼
              Continue Execution
                    │
         ┌──────────┼──────────┐
         ▼          ▼          ▼
    Arithmetic  Storage   Transfer
    Operations  Updates   to Insurance
         │          │          │
    ┌────┴────┐     │     ┌────┴────┐
    │         │     │     │         │
    ▼         │     │     ▼         │
  Safe        │     │  Success      │
  (checked)   │     │               │
    │         │     │               │
    └─────────┴─────┴───────────────┘
                    │
                    ▼
               Success
```

---

## 10. Integration Sequence Diagram

```
System Initialization & Configuration

┌────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐  ┌──────────┐
│ Admin  │  │ Staking  │  │Insurance │  │ Multisig │  │Governance│
└───┬────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘  └────┬─────┘
    │            │               │             │             │
    │ Deploy     │               │             │             │
    ├───────────>│               │             │             │
    │            │               │             │             │
    │ initialize()│              │             │             │
    ├───────────>│               │             │             │
    │            │               │             │             │
    │ set_insurance_pool()       │             │             │
    ├───────────>│               │             │             │
    │            │               │             │             │
    │ set_multisig_admin()       │             │             │
    ├───────────>│               │             │             │
    │            │               │             │             │
    │ set_governance()           │             │             │
    ├───────────>│               │             │             │
    │            │               │             │             │
    │            │ add_allowed_call("slash")   │             │
    │            │<──────────────────────────────────────────┤
    │            │               │             │             │
    │  ✓ Ready   │               │             │             │
    │            │               │             │             │
```

---

## 11. Tier Transition Matrix

```
Current Tier │ After 10% Slash │ After 25% Slash │ After 50% Slash
─────────────┼─────────────────┼─────────────────┼────────────────
             │                 │                 │
Tier 0       │ Tier 0          │ Tier 0          │ Tier 0
(< 100)      │ (still < 100)   │ (still < 100)   │ (still < 100)
             │                 │                 │
Tier 1       │ Tier 0          │ Tier 0          │ Tier 0
(100)        │ (90 < 100)      │ (75 < 100)      │ (50 < 100)
             │                 │                 │
Tier 1       │ Tier 1          │ Tier 1          │ Tier 1
(200)        │ (180 >= 100)    │ (150 >= 100)    │ (100 >= 100)
             │                 │                 │
Tier 2       │ Tier 1          │ Tier 1          │ Tier 1
(500)        │ (450 < 500)     │ (375 < 500)     │ (250 < 500)
             │                 │                 │
Tier 2       │ Tier 2          │ Tier 2          │ Tier 1
(1000)       │ (900 >= 500)    │ (750 >= 500)    │ (500 = 500)
             │                 │                 │
Tier 3       │ Tier 2          │ Tier 2          │ Tier 2
(2000)       │ (1800 < 2000)   │ (1500 < 2000)   │ (1000 < 2000)
             │                 │                 │
Tier 3       │ Tier 3          │ Tier 3          │ Tier 2
(5000)       │ (4500 >= 2000)  │ (3750 >= 2000)  │ (2500 >= 2000)
```

---

## Summary

These flow diagrams illustrate:

1. **Architecture** - How slashing fits into the protocol
2. **Authorization** - Multisig and governance approval flows
3. **Calculation** - How slash amounts and tiers are computed
4. **State Transitions** - How stake states change over time
5. **Cross-Contract** - How contracts interact during slashing
6. **Data Flow** - How data moves through the system
7. **Error Handling** - How errors are caught and reported
8. **Integration** - How to set up the system
9. **Tier Transitions** - How tiers change with slashing
10. **Decision Trees** - How authorization decisions are made

Use these diagrams to understand the slashing mechanism at different levels of detail.

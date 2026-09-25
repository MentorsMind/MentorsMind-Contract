# Governance Attack Surface Analysis

**Issue:** #595  
**Date:** 2026-08-31  
**Scope:** Multisig, Timelock, Upgrade, and Core Governance contracts  
**Status:** Complete

---

## Table of Contents

1. [Executive Summary](#executive-summary)
2. [Threat Model](#threat-model)
3. [Contract-by-Contract Analysis](#contract-by-contract-analysis)
4. [Attack Vector Catalog](#attack-vector-catalog)
5. [Risk Severity Assessment](#risk-severity-assessment)
6. [Cross-Contract Attack Chains](#cross-contract-attack-chains)
7. [Mitigation Recommendations](#mitigation-recommendations)
8. [Dead Code Findings](#dead-code-findings)

---

## Executive Summary

This analysis covers five governance-adjacent contracts and the core governance module. We identified **17 distinct attack vectors** across the governance stack, ranging from Critical to Low severity. The most impactful findings are:

- **Critical:** `set_emergency_signers` in multisig_admin requires only single-signer auth, enabling a single compromised key to control the emergency signer set.
- **Critical:** `pause_guardian::activate_protections` has no auth check, allowing anyone to grief the system by repeatedly pausing it.
- **High:** `cancel_pending_upgrade` in upgrade_registry only requires admin auth, letting a compromised admin block all upgrades.
- **High:** `ExecuteCall` arguments are never forwarded to target contracts (functional bug).
- **Medium:** Governance voting commit-reveal, minimum holding period, and manipulation detection modules exist but are **unused** in the live voting path.

---

## Threat Model

### Adversary Classes

| Adversary | Capability | Goal |
|-----------|-----------|------|
| **Compromised Admin** | Single key control over admin-gated functions | Drain funds, block upgrades, manipulate governance |
| **Colluding Signers** | Control of M-of-N multisig signers | Execute arbitrary cross-contract calls, rollback protocol |
| **Malicious Proposer** | Can create governance proposals | Queue flooding, griefing, MEV extraction |
| **External Attacker** | No authorized keys | DoS via pause, frontrun execution, replay attacks |
| **Guardian Compromise** | Control of guardian multisig | Cancel legitimate operations, bypass timelock safety |

### Trust Boundaries

```
┌─────────────────────────────────────────────────┐
│                  Protocol Core                   │
│  (escrow, staking, treasury, lending_pool)       │
├─────────────┬───────────────┬───────────────────┤
│  Governance │   Multisig    │   Timelock        │
│  (voting)   │   (signers)   │   (delay)         │
├─────────────┼───────────────┼───────────────────┤
│  Upgrade    │   Guardian    │   Pause           │
│  Registry   │   Multisig    │   Guardian        │
└─────────────┴───────────────┴───────────────────┘
```

---

## Contract-by-Contract Analysis

### 1. Governance Contract (`contracts/governance/src/lib.rs`)

**Lines of code:** ~2827  
**Storage keys:** 20+ DataKey variants  
**Entry points:** 20+ public functions

#### State Machine

```
Active ──> Passed ──> Queued ──> Executed
  │          │
  ▼          ▼
Failed    Cancelled
```

Uses `StateMachine` trait from `shared` crate. Transitions validated via `transition_proposal_status` (line 322).

#### Key Security Patterns

| Pattern | Implementation | Assessment |
|---------|---------------|------------|
| Per-address proposal limit | `max_active_proposals_per_address = 3` (line 368) | Adequate for queue flooding |
| Minimum proposer balance | Checked at lines 657-671 | Prevents spam from empty accounts |
| Cancel cooldown | 7-day per (admin, action_type) (line 1172) | Prevents cancel abuse |
| Multi-sig escalation | Required after 3 cancels in 30 days (line 1191) | Strong anti-griefing |
| Time-weighted voting | Early=80%, Mid=100%, Late=110% (lines 74-84) | Discourages last-minute voting |
| ExecuteCall timelock | 7-day mandatory delay + 48h via timelock (line 993) | Dual-delay protection |

#### Vulnerabilities

**V-1: Permissionless `execute_proposal` (line 891)**  
- **Severity:** Medium  
- Anyone can call `execute_proposal` after voting succeeds and timelock expires.
- Combined with allowlisted `ExecuteCall`, an executor can trigger any allowlisted function.
- **Mitigation present:** Allowlist restricts targets. **Gap:** No executor reputation or stake requirement.

**V-2: `ExecuteCall` arguments never forwarded (line 1680-1681)**  
- **Severity:** High (functional bug)  
- `apply_action` passes `vec![env]` to the target regardless of the `args` field.
- Any `ExecuteCall` proposal invoking a function with parameters will fail.
- **Recommendation:** Forward `proposal.action.args` or remove `ExecuteCall` type.

**V-3: `GovConsensusEmergency` doesn't gate execution (lines 2215-2225)**  
- **Severity:** Medium  
- Emergency flag blocks `create_proposal` but not `execute_proposal`.
- Pre-emergency proposals can still execute during emergency state.
- **Recommendation:** Check emergency flag in `execute_proposal`.

**V-4: No global proposal queue limit**  
- **Severity:** Low  
- Per-address limit is 3, but no global cap. Many addresses can collectively flood the queue.

---

### 2. Multisig Admin Contract (`contracts/multisig_admin/src/lib.rs`)

**Lines of code:** ~1413  
**Storage keys:** 12 DataKey variants  
**Entry points:** 15+ public functions

#### Key Security Patterns

| Pattern | Implementation | Assessment |
|---------|---------------|------------|
| Threshold validation | Checked before execution (line 284) | Correct |
| Pre-execution marking | `executed = true` set before dispatch (line 287) | Prevents re-entrancy |
| Signer removal safety | Prevents removal below threshold (line 1230) | Correct |
| Proposal expiry | 7-day expiry (line 72) | Reasonable |

#### Vulnerabilities

**V-5: Arbitrary external contract call (line 319)**  
- **Severity:** Critical (by design)  
- `execute_action` invokes `env.invoke_contract::<()>(&proposal.target, &proposal.function, proposal.args)`.
- 3-of-5 colluding signers can call ANY function on ANY contract the multisig has admin over.
- **Mitigation present:** Threshold requirement. **Gap:** No function-level allowlist or call simulation.

**V-6: `set_emergency_signers` requires only single-signer auth (line 468)**  
- **Severity:** Critical  
- A single compromised signer can reset the entire 7-signer emergency set to attacker-controlled addresses.
- Comment at line 466 acknowledges this: "For simplicity in the DR path, this is callable by any current signer."
- **Recommendation:** Require M-of-N multisig approval for emergency signer changes.

**V-7: No proposal limit in multisig**  
- **Severity:** Medium  
- Unlike governance, no per-address or global proposal limit.
- A compromised signer can spam proposals to grief other signers.

**V-8: Expired proposals not cleaned from storage**  
- **Severity:** Low  
- Expired proposals persist in persistent storage, causing unbounded storage growth.

---

### 3. Timelock Contract (`contracts/timelock/src/lib.rs`)

**Lines of code:** ~940  
**Storage keys:** 8 DataKey variants  
**Entry points:** 7 public functions  
**Formal verification:** 4 Kani-proven invariants

#### Constants

| Constant | Value | Purpose |
|----------|-------|---------|
| `MIN_DELAY` | 48 hours | Minimum scheduling delay |
| `MAX_DELAY` | 30 days | Maximum scheduling delay |
| `OPERATION_EXPIRY_SECS` | 14 days | Operations expire after ready_at + 14d |
| `MIN_GUARDIAN_THRESHOLD` | 4 | Minimum guardian multisig threshold |
| `MIN_GUARDIAN_SIGNERS` | 7 | Minimum guardian signer count |
| `MAX_GUARDIAN_OVERRIDES_PER_PERIOD` | 3 | Max emergency cancels per 30 days |
| `GUARDIAN_VETO_PERIOD_SECS` | 48 hours | Community veto window |

#### Formal Verification Proofs (proofs.rs)

1. `ready_at > now` (lines 21-35)
2. Execute window correctness (lines 41-63)
3. Done is terminal (lines 69-79)
4. Cancel authorization: proposer OR admin (lines 86-108)

#### Vulnerabilities

**V-9: `execute` has no auth check (line 292)**  
- **Severity:** Medium  
- Any address can execute a ready, non-expired operation.
- Standard timelock design but enables frontrunning.
- **Recommendation:** Consider requiring proposer auth or adding a executor allowlist.

**V-10: `transfer_admin` has no timelock (line 498)**  
- **Severity:** High  
- Admin can be transferred instantly with a single `require_auth`.
- Governance contract requires 48-hour timelock for admin changes; timelock does not.
- **Recommendation:** Add minimum delay or require governance approval for admin transfer.

**V-11: Guardian override rate-limiting in `instance` storage (lines 616-642)**  
- **Severity:** Medium  
- `GuardianOverrideTimestamps` stored in `instance` storage, which does not persist across contract upgrades.
- After upgrade, guardian can immediately perform 3 more overrides regardless of previous usage.
- **Recommendation:** Move to `persistent` storage.

**V-12: No operation cancellation cleanup**  
- **Severity:** Low  
- Expired operations remain in persistent storage.
- An attacker could schedule many operations that expire, causing storage bloat.

---

### 4. Upgrade Registry Contract (`contracts/upgrade_registry/src/lib.rs`)

**Lines of code:** ~2131  
**Storage keys:** 20+ DataKey variants  
**Entry points:** 20+ public functions

#### Two Upgrade Paths

- **PATH A (recommended):** `schedule_upgrade` -> `execute_pending_upgrade` with timelock
- **PATH B (deprecated):** `upgrade_contract` with timelock enforcement

#### Key Security Patterns

| Pattern | Implementation | Assessment |
|---------|---------------|------------|
| M-of-N approval | Required for all upgrade operations | Strong |
| Version monotonicity | `new_version <= current` rejected (line 276) | Prevents downgrades |
| Validation caching | 5-minute cache for approval results (line 1402) | Performance optimization |
| Storage schema validation | Compatibility checking with gradual migration | Comprehensive |
| Emergency rollback | 4-of-7 threshold with WASM revert | Last resort safety |

#### Vulnerabilities

**V-13: `cancel_pending_upgrade` only requires admin auth (line 601)**  
- **Severity:** High  
- Compromised admin can cancel any pending upgrade, blocking M-of-N signers.
- **Recommendation:** Require M-of-N approval or multisig for cancellation.

**V-14: `set_emergency_signers` only requires admin auth (line 1069)**  
- **Severity:** High  
- Single point of failure for emergency signer configuration.
- **Recommendation:** Require M-of-N approval.

**V-15: Single `PendingUpgrade` slot (line 271)**  
- **Severity:** Medium  
- Only one upgrade can be pending at a time.
- An attacker could schedule a benign upgrade to block legitimate upgrades.

**V-16: Rollback restores admin from snapshot**  
- **Severity:** Medium  
- If snapshot captured during compromised admin era, rollback restores compromised admin.
- **Recommendation:** Add admin validation post-rollback or require manual admin re-confirmation.

**V-17: Validation cache ignores auth revocation (lines 1401-1434)**  
- **Severity:** Low  
- 5-minute cache returns cached approval without re-verifying `require_auth`.
- Theoretical risk if signer authority revoked between cache write and expiry.
- Soroban's auth model makes exploitation unlikely in practice.

---

### 5. Admin Rotation Coordinator (`contracts/admin_rotation_coordinator/src/lib.rs`)

**Lines of code:** 62  
**Storage keys:** 2 DataKey variants  
**Entry points:** 4 public functions

#### Vulnerabilities

**V-18: No threshold enforcement for batch rotation (line 47)**  
- **Severity:** High  
- `batch_propose_admin_change` requires only the coordinator's admin auth.
- No multisig or threshold gate. Single compromised key rotates ALL managed contracts.
- **Recommendation:** Add threshold check or require multisig approval.

**V-19: No undo/rollback for batch proposals**  
- **Severity:** Medium  
- Once `batch_propose_admin_change` is called, there is no way to cancel all proposals.
- Admin must separately cancel on each managed contract.

**V-20: No event emission**  
- **Severity:** Low  
- Batch rotations are unauditable on-chain.
- **Recommendation:** Emit events for each proposed admin change.

---

## Attack Vector Catalog

### AV-1: Admin Key Compromise -> Full Protocol Control

| Step | Action | Contract |
|------|--------|----------|
| 1 | Compromise governance admin key | External |
| 2 | `add_allowed_call` to allowlist a drain function | Governance |
| 3 | Create `ExecuteCall` proposal targeting the drain | Governance |
| 4 | Self-vote with large token holdings | Governance |
| 5 | `execute_proposal` after voting period + 7-day timelock | Governance |

**Mitigations present:** Allowlist, voting quorum, timelock.  
**Gap:** No multi-party approval required for allowlist changes.

### AV-2: Signer Collusion -> Emergency Rollback

| Step | Action | Contract |
|------|--------|----------|
| 1 | Compromise 3 multisig signer keys (3-of-5) | External |
| 2 | `propose_action` to add a 4th compromised signer | Multisig |
| 3 | Execute to gain 4-of-5 control | Multisig |
| 4 | `set_emergency_signers` with attacker-controlled addresses | Multisig (single auth!) |
| 5 | `propose_emergency_rollback` with old WASM | Multisig |
| 6 | `execute_emergency_rollback` to revert to vulnerable version | Multisig |

**Mitigations present:** Threshold checks on rollback.  
**Gap:** `set_emergency_signers` only requires single signer auth (V-6).

### AV-3: Guardian Bypass -> Timelock Cancellation

| Step | Action | Contract |
|------|--------|----------|
| 1 | Compromise governance admin | External |
| 2 | Schedule malicious operation via timelock | Timelock |
| 3 | Guardian should cancel, but is rate-limited (3/30d) | Timelock |
| 4 | Or: guardian not yet registered (bootstrap window) | Timelock |
| 5 | Operation executes after delay | Timelock |

**Mitigations present:** Guardian bootstrap enforces minimum 4-of-7.  
**Gap:** Bootstrap window (before guardian set) is unprotected.

### AV-4: Pause Guardian DoS

| Step | Action | Contract |
|------|--------|----------|
| 1 | Call `activate_protections()` (no auth required) | Pause Guardian |
| 2 | System pauses, blocking yield operations | Pause Guardian |
| 3 | Admin must manually unpause | External |
| 4 | Repeat | Pause Guardian |

**Mitigation:** Admin can unpause.  
**Gap:** No rate limiting on `activate_protections`.

### AV-5: Upgrade Blocking via Single Pending Slot

| Step | Action | Contract |
|------|--------|----------|
| 1 | Compromise admin or M-of-N signers | External |
| 2 | `schedule_upgrade` with benign WASM hash | Upgrade Registry |
| 3 | Legitimate upgrade cannot be scheduled (single slot) | Upgrade Registry |
| 4 | Wait for expiry or cancel at will | Upgrade Registry |

**Mitigation:** Admin can cancel.  
**Gap:** Single pending slot creates a blocking vector.

---

## Risk Severity Assessment

| ID | Vulnerability | Severity | Likelihood | Impact | Exploitability |
|----|--------------|----------|------------|--------|----------------|
| V-5 | Arbitrary external contract call | Critical | Low (by design) | Total loss | Requires M-of-N collusion |
| V-6 | `set_emergency_signers` single auth | Critical | Medium | Total DR control | Single key compromise |
| V-18 | No threshold for batch rotation | High | Medium | All contracts rotated | Single key compromise |
| V-13 | `cancel_pending_upgrade` admin-only | High | Medium | Upgrade blocking | Admin compromise |
| V-14 | `set_emergency_signers` admin-only | High | Medium | Emergency signer control | Admin compromise |
| V-10 | `transfer_admin` no timelock | High | Low | Instant admin takeover | Admin compromise |
| V-2 | ExecuteCall args not forwarded | High | High | ExecuteCall always fails | N/A (bug) |
| V-1 | Permissionless execute_proposal | Medium | Low | frontrunning | Anyone |
| V-3 | Emergency doesn't gate execution | Medium | Low | Execute during emergency | Pre-existing proposal |
| V-7 | No multisig proposal limit | Medium | Low | Signer griefing | Compromised signer |
| V-9 | Permissionless timelock execute | Medium | Low | Frontrunning | Anyone |
| V-11 | Override state in instance storage | Medium | Low | Rate-limit bypass post-upgrade | Upgrade event |
| V-15 | Single pending upgrade slot | Medium | Low | Upgrade blocking | Admin/signer |
| V-16 | Rollback restores old admin | Medium | Low | Admin compromise revival | Rollback event |
| V-4 | No global proposal limit | Low | Low | Queue flooding | Many accounts |
| V-8 | Expired proposals not cleaned | Low | Low | Storage bloat | Time |
| V-12 | Timelock ops not cleaned | Low | Low | Storage bloat | Time |
| V-17 | Validation cache ignores revocation | Low | Very Low | Auth bypass | Timing window |
| V-19 | No undo for batch proposals | Medium | Low | Irreversible rotation | Admin action |
| V-20 | No rotation events | Low | Low | Unauditable rotations | N/A |

---

## Cross-Contract Attack Chains

### Chain 1: Governance -> Multisig -> Timelock

```
Governance admin ──> add_allowed_call (timelock.transfer_admin)
    ──> ExecuteCall proposal ──> timelock.transfer_admin(attacker)
    ──> Attacker now controls timelock
    ──> Schedule malicious operation ──> Execute after delay
```

**Total steps:** 4  
**Required:** Governance admin key + waiting period

### Chain 2: Multisig Signer -> Emergency Set -> Protocol Rollback

```
Compromise 3 signers ──> propose_action (add 4th signer)
    ──> Execute ──> set_emergency_signers (attacker addresses)
    ──> propose_emergency_rollback ──> execute_emergency_rollback
    ──> Protocol reverted to vulnerable version
```

**Total steps:** 4  
**Required:** 3 signer keys (3-of-5)

### Chain 3: Pause DoS -> Governance Delay -> Execution Window

```
Attacker ──> activate_protections (no auth)
    ──> System paused ──> Admin unpause (delays governance)
    ──> Repeat to extend delay window
    ──> Pre-existing proposal executes during unpause window
```

**Total steps:** 2 (repeatable)  
**Required:** No keys

---

## Mitigation Recommendations

### Critical Priority

| ID | Recommendation | Contracts Affected |
|----|---------------|-------------------|
| M-1 | Require M-of-N approval for `set_emergency_signers` | multisig_admin, upgrade_registry |
| M-2 | Add rate limiting or auth to `activate_protections` | pause_guardian |
| M-3 | Forward ExecuteCall args or remove ExecuteCall type | governance |

### High Priority

| ID | Recommendation | Contracts Affected |
|----|---------------|-------------------|
| M-4 | Add timelock to `transfer_admin` | timelock |
| M-5 | Require M-of-N for `cancel_pending_upgrade` | upgrade_registry |
| M-6 | Require threshold for `batch_propose_admin_change` | admin_rotation_coordinator |
| M-7 | Move `GuardianOverrideTimestamps` to persistent storage | timelock |

### Medium Priority

| ID | Recommendation | Contracts Affected |
|----|---------------|-------------------|
| M-8 | Check emergency flag in `execute_proposal` | governance |
| M-9 | Add per-address proposal limit to multisig | multisig_admin |
| M-10 | Add executor auth or reputation to timelock `execute` | timelock |
| M-11 | Add undo/cancel for batch rotation proposals | admin_rotation_coordinator |
| M-12 | Add events to admin rotation coordinator | admin_rotation_coordinator |

### Low Priority

| ID | Recommendation | Contracts Affected |
|----|---------------|-------------------|
| M-13 | Add global governance proposal limit | governance |
| M-14 | Clean expired proposals from storage | multisig_admin, timelock |
| M-15 | Invalidate validation cache on auth revocation | upgrade_registry |

---

## Dead Code Findings

### Governance Voting Module (`shared/src/governance_voting.rs`)

The following features are implemented but **not wired** into the live governance voting path:

| Feature | Status | Risk |
|---------|--------|------|
| Commit-reveal voting | Unused | No MEV protection in live path |
| Minimum holding period | Unused | No minimum stake duration enforced |
| Random deadline extension | Unused | No frontrunning protection |
| Manipulation detection | Unused (only flags, doesn't block) | Late stakers not blocked |

**Recommendation:** Either integrate these into the governance `vote()` function or remove them to reduce attack surface and code maintenance burden.

---

## Appendix: Storage Layout Audit Notes

| Contract | Storage Type | Namespace Isolation | Assessment |
|----------|-------------|--------------------|----|
| governance | instance + persistent | Yes (NamespaceRoot) | Correct |
| multisig_admin | instance + persistent | Yes (NamespaceRoot) | Correct |
| timelock | instance + persistent | Yes (NamespaceRoot) | Correct |
| upgrade_registry | instance + persistent | Yes (NamespaceRoot) | Correct |
| admin_rotation_coordinator | instance only | No | Inconsistent |
| pause_guardian | instance + persistent | Yes (NamespaceRoot) | Correct |

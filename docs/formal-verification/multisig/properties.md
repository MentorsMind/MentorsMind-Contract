# Multisig Admin Contract Properties

This document provides a quick reference to all properties that must hold for the Multisig Admin contract.

---

## Safety Properties

| ID | Property | Status |
|----|----------|--------|
| SP1 | Threshold Validity | ✅ Defined |
| SP2 | Approval Uniqueness | ✅ Defined |
| SP3 | Execution Guard | ✅ Defined |
| SP4 | Single Execution | ✅ Defined |
| SP5 | Signer Authorization | ✅ Defined |

---

## Liveness Properties

| ID | Property | Status |
|----|----------|--------|
| LP1 | Proposal Progression | ✅ Defined |
| LP2 | Proposer Auto-Approval | ✅ Defined |
| LP3 | Cancellation Availability | ✅ Defined |

---

## Functional Correctness Properties

| ID | Property | Status |
|----|----------|--------|
| FC1 | Self-Targeted Operations | ✅ Defined |
| FC2 | External Contract Invocation | ✅ Defined |
| FC3 | Signer Addition Safety | ✅ Defined |
| FC4 | Signer Removal Safety | ✅ Defined |
| FC5 | Threshold Update Safety | ✅ Defined |

---

## Temporal Properties

| ID | Property | Status |
|----|----------|--------|
| TP1 | Expiry Enforcement | ✅ Defined |
| TP2 | Proposal Lifespan | ✅ Defined |

---

## Full Property Definitions

See [specifications.md](./specifications.md) for complete formal definitions of all properties.

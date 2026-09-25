# Timelock Controller Contract Properties

This document provides a quick reference to all properties that must hold for the Timelock Controller contract.

---

## Safety Properties

| ID | Property | Status |
|----|----------|--------|
| SP1 | Operation Uniqueness | ✅ Defined |
| SP2 | Delay Bounds Enforcement | ✅ Defined |
| SP3 | Temporal Execution Window | ✅ Defined |
| SP4 | Single Execution | ✅ Defined |
| SP5 | Cancellation Authorization | ✅ Defined |

---

## Liveness Properties

| ID | Property | Status |
|----|----------|--------|
| LP1 | Execution Availability | ✅ Defined |
| LP2 | View Function Correctness | ✅ Defined |
| LP3 | Expiry Detection | ✅ Defined |

---

## Functional Correctness Properties

| ID | Property | Status |
|----|----------|--------|
| FC1 | Operation Scheduling Correctness | ✅ Defined |
| FC2 | Operation Payload Immutability | ✅ Defined |
| FC3 | Nonce Monotonicity | ✅ Defined |

---

## Temporal Properties

| ID | Property | Status |
|----|----------|--------|
| TP1 | Delay Lower Bound (Safety) | ✅ Defined |
| TP2 | Delay Upper Bound (Flexibility) | ✅ Defined |
| TP3 | Expiry Enforcement | ✅ Defined |

---

## Full Property Definitions

See [specifications.md](./specifications.md) for complete formal definitions of all properties.

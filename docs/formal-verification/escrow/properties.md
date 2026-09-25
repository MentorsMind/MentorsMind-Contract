# Escrow Contract Properties

This document provides a quick reference to all properties that must hold for the Escrow contract.

---

## Safety Properties

| ID | Property | Status |
|----|----------|--------|
| SP1 | Fund Conservation | ✅ Defined |
| SP2 | No Fund Loss | ✅ Defined |
| SP3 | State Machine Integrity | ✅ Defined |
| SP4 | Double-Spend Prevention | ✅ Defined |
| SP5 | Authorization Correctness | ✅ Defined |

---

## Liveness Properties

| ID | Property | Status |
|----|----------|--------|
| LP1 | Auto-Release Availability | ✅ Defined |
| LP2 | Dispute Openability | ✅ Defined |
| LP3 | Admin Override | ✅ Defined |

---

## Functional Correctness Properties

| ID | Property | Status |
|----|----------|--------|
| FC1 | Fee Calculation Accuracy | ✅ Defined |
| FC2 | Partial Release Correctness | ✅ Defined |
| FC3 | Dispute Resolution Split | ✅ Defined |
| FC4 | Token Whitelist Enforcement | ✅ Defined |

---

## Temporal Properties

| ID | Property | Status |
|----|----------|--------|
| TP1 | Auto-Release Window Correctness | ✅ Defined |
| TP2 | Creation Timestamp Validity | ✅ Defined |

---

## Full Property Definitions

See [specifications.md](./specifications.md) for complete formal definitions of all properties.

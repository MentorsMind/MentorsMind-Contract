# Error Code Reference

## Overview

This document provides a comprehensive reference for all error codes in MentorsMind smart contracts. Each error includes its meaning, common causes, and recovery procedures.

## Table of Contents

1. [Shared Error Codes](#shared-error-codes)
2. [Contract-Specific Errors](#contract-specific-errors)
3. [Error Recovery Procedures](#error-recovery-procedures)
4. [Error Examples](#error-examples)

---

## Shared Error Codes

These error codes are used across multiple contracts and defined in `contracts/shared/src/lib.rs`.

### AlreadyInitialized (1)

**Meaning**: Contract has already been initialized.

**Causes**:

- Calling `initialize()` more than once
- Contract state already exists from previous deployment

**Recovery**:

1. Verify contract is already initialized
2. Skip initialization step
3. Call contract functions normally

**Example**:

```rust
// ❌ Error: AlreadyInitialized
initialize(env, admin_address)?;
initialize(env, admin_address)?; // Second call fails

// ✅ Correct: Initialize only once
initialize(env, admin_address)?;
```

---

### NotInitialized (2)

**Meaning**: Contract has not been initialized yet.

**Causes**:

- Calling contract functions before `initialize()`
- Contract state was cleared or reset

**Recovery**:

1. Call `initialize()` with admin address
2. Verify initialization succeeded
3. Retry the operation

**Example**:

```rust
// ❌ Error: NotInitialized
create_escrow(env, mentor, learner, amount)?; // Contract not initialized

// ✅ Correct: Initialize first
initialize(env, admin_address)?;
create_escrow(env, mentor, learner, amount)?;
```

---

### Unauthorized (3)

**Meaning**: Caller does not have permission to perform this operation.

**Causes**:

- Calling admin-only function without admin privileges
- Calling function without required authorization
- Signature validation failed

**Recovery**:

1. Verify caller has required role/permissions
2. Use correct account for operation
3. Check authorization requirements in contract documentation

**Example**:

```rust
// ❌ Error: Unauthorized
// Called by non-admin account
register_upgrade(env, contract_name, old_v, new_v, hash)?;

// ✅ Correct: Use admin account
// Switch to admin account and retry
register_upgrade(env, contract_name, old_v, new_v, hash)?;
```

---

### NotFound (4)

**Meaning**: Requested resource does not exist.

**Causes**:

- Escrow ID does not exist
- Stream ID not found
- Verification record missing
- Contract not deployed

**Recovery**:

1. Verify resource ID is correct
2. Check resource was created successfully
3. Query contract to list available resources

**Example**:

```rust
// ❌ Error: NotFound
get_escrow(env, 999)?; // Escrow 999 doesn't exist

// ✅ Correct: Use valid escrow ID
let escrows = list_escrows(env)?;
let escrow_id = escrows[0].id;
get_escrow(env, escrow_id)?;
```

---

### InvalidAmount (5)

**Meaning**: Amount is invalid (zero, negative, or exceeds limits).

**Causes**:

- Amount is zero or negative
- Amount exceeds maximum allowed
- Amount is below minimum required
- Insufficient balance

**Recovery**:

1. Verify amount is positive
2. Check amount is within valid range
3. Ensure sufficient balance available

**Example**:

```rust
// ❌ Error: InvalidAmount
create_escrow(env, mentor, learner, 0)?; // Zero amount
create_escrow(env, mentor, learner, -100)?; // Negative amount

// ✅ Correct: Use valid amount
create_escrow(env, mentor, learner, 1000)?;
```

---

### InvalidState (6)

**Meaning**: Operation is invalid for current state.

**Causes**:

- Escrow is not in correct status for operation
- Stream is already completed
- Dispute already exists
- State machine transition not allowed

**Recovery**:

1. Check current state of resource
2. Verify operation is valid for current state
3. Complete prerequisite operations first

**Example**:

```rust
// ❌ Error: InvalidState
// Escrow is already released
release_escrow(env, escrow_id)?;
release_escrow(env, escrow_id)?; // Already released

// ✅ Correct: Check state first
let escrow = get_escrow(env, escrow_id)?;
if escrow.status == EscrowStatus::Active {
    release_escrow(env, escrow_id)?;
}
```

---

### DuplicateEntry (7)

**Meaning**: Entry already exists (duplicate).

**Causes**:

- Learner already joined group escrow
- Mentor already subscribed to upgrade notifications
- Duplicate record creation attempt

**Recovery**:

1. Check if entry already exists
2. Skip creation if already exists
3. Update existing entry if needed

**Example**:

```rust
// ❌ Error: DuplicateEntry
subscribe_to_upgrades(env, contract_name, subscriber)?;
subscribe_to_upgrades(env, contract_name, subscriber)?; // Already subscribed

// ✅ Correct: Check before subscribing
if !is_subscribed(env, contract_name, subscriber)? {
    subscribe_to_upgrades(env, contract_name, subscriber)?;
}
```

---

### UnsupportedOperation (8)

**Meaning**: Operation is not supported by this contract.

**Causes**:

- Calling function that doesn't exist
- Operation disabled in current version
- Feature not implemented

**Recovery**:

1. Verify operation is supported
2. Check contract version
3. Use alternative operation if available

---

### Overflow (9)

**Meaning**: Arithmetic operation resulted in overflow.

**Causes**:

- Adding amounts that exceed i128 maximum
- Multiplying large numbers
- Fee calculation overflow

**Recovery**:

1. Use smaller amounts
2. Split operation into multiple transactions
3. Check contract limits

**Example**:

```rust
// ❌ Error: Overflow
let max_i128 = i128::MAX;
let amount = max_i128 + 1; // Overflow

// ✅ Correct: Use valid amounts
let amount = 1_000_000_000_000_000_000i128; // 1 billion tokens
```

---

### Underflow (10)

**Meaning**: Arithmetic operation resulted in underflow.

**Causes**:

- Subtracting more than available
- Negative result from calculation
- Insufficient balance

**Recovery**:

1. Verify sufficient balance
2. Check calculation logic
3. Ensure amounts are positive

**Example**:

```rust
// ❌ Error: Underflow
let balance = 100i128;
let withdrawal = 200i128;
let result = balance - withdrawal; // Underflow

// ✅ Correct: Verify balance first
if balance >= withdrawal {
    let result = balance - withdrawal;
}
```

---

## Contract-Specific Errors

### Escrow Contract

**Error Codes**: 1-20

| Code | Name                 | Meaning                               |
| ---- | -------------------- | ------------------------------------- |
| 1    | AlreadyInitialized   | Escrow contract already initialized   |
| 2    | NotInitialized       | Escrow contract not initialized       |
| 3    | Unauthorized         | Caller not authorized for operation   |
| 4    | EscrowNotFound       | Escrow ID does not exist              |
| 5    | InvalidAmount        | Amount is invalid                     |
| 6    | InvalidStatus        | Escrow status invalid for operation   |
| 7    | DisputeAlreadyExists | Dispute already opened                |
| 8    | NoActiveDispute      | No active dispute to resolve          |
| 9    | InvalidDisputeReason | Dispute reason exceeds 500 characters |
| 10   | SessionNotEnded      | Session has not ended yet             |

**Recovery Examples**:

```rust
// InvalidDisputeReason (9)
// ❌ Error: Reason too long
let reason = "a".repeat(501); // 501 characters
open_dispute(env, escrow_id, reason)?;

// ✅ Correct: Limit reason to 500 characters
let reason = "a".repeat(500); // 500 characters
open_dispute(env, escrow_id, reason)?;

// SessionNotEnded (10)
// ❌ Error: Session still active
let escrow = get_escrow(env, escrow_id)?;
if env.ledger().timestamp() < escrow.session_end_time {
    release_escrow(env, escrow_id)?; // Fails
}

// ✅ Correct: Wait for session to end
if env.ledger().timestamp() >= escrow.session_end_time {
    release_escrow(env, escrow_id)?;
}
```

### Verification Contract

**Error Codes**: 1-10

| Code | Name                | Meaning                                   |
| ---- | ------------------- | ----------------------------------------- |
| 1    | AlreadyInitialized  | Verification contract already initialized |
| 2    | NotInitialized      | Verification contract not initialized     |
| 3    | NotAdmin            | Caller is not admin                       |
| 4    | MentorNotVerified   | Mentor verification not found             |
| 5    | VerificationExpired | Mentor verification has expired           |
| 6    | InvalidCredential   | Credential hash is invalid                |
| 7    | InvalidExpiry       | Expiry time is in the past                |

**Recovery Examples**:

```rust
// MentorNotVerified (4)
// ❌ Error: Mentor not verified
let is_verified = is_verified(env, mentor_address)?; // Returns false
if is_verified {
    // Process verified mentor
}

// ✅ Correct: Verify mentor first
verify_mentor(env, mentor_address, credential_hash, expiry)?;
let is_verified = is_verified(env, mentor_address)?; // Returns true

// VerificationExpired (5)
// ❌ Error: Verification expired
let record = get_verification(env, mentor_address)?;
if env.ledger().timestamp() > record.expiry {
    // Verification is expired
}

// ✅ Correct: Renew verification
revoke_verification(env, mentor_address)?;
verify_mentor(env, mentor_address, new_credential_hash, new_expiry)?;
```

### Upgrade Registry Contract

**Error Codes**: 1-6

| Code | Name               | Meaning                        |
| ---- | ------------------ | ------------------------------ |
| 1    | AlreadyInitialized | Registry already initialized   |
| 2    | NotInitialized     | Registry not initialized       |
| 3    | NotAdmin           | Caller is not admin            |
| 4    | ContractNotFound   | Contract not found in registry |
| 5    | AlreadySubscribed  | Already subscribed to upgrades |
| 6    | NotSubscribed      | Not subscribed to upgrades     |

---

## Error Recovery Procedures

### General Recovery Steps

1. **Identify Error**
   - Check error code returned
   - Review error message
   - Check contract logs

2. **Understand Cause**
   - Refer to error documentation
   - Check contract state
   - Review transaction parameters

3. **Implement Fix**
   - Follow recovery procedure for error
   - Verify fix is correct
   - Test in local environment

4. **Retry Operation**
   - Execute corrected operation
   - Monitor for success
   - Verify state changes

### Common Recovery Patterns

**Pattern 1: State Validation**

```rust
// Before operation, validate state
let resource = get_resource(env, id)?;
if !is_valid_state_for_operation(&resource) {
    return Err(InvalidState);
}
perform_operation(env, id)?;
```

**Pattern 2: Authorization Check**

```rust
// Verify authorization before operation
let admin = get_admin(env)?;
if caller != admin {
    return Err(Unauthorized);
}
perform_admin_operation(env)?;
```

**Pattern 3: Amount Validation**

```rust
// Validate amounts before operation
if amount <= 0 || amount > MAX_AMOUNT {
    return Err(InvalidAmount);
}
transfer_amount(env, amount)?;
```

**Pattern 4: Existence Check**

```rust
// Verify resource exists before operation
if !resource_exists(env, id)? {
    return Err(NotFound);
}
operate_on_resource(env, id)?;
```

---

## Error Examples

### Example 1: Creating Escrow with Invalid Amount

```rust
// Scenario: User tries to create escrow with zero amount
let result = create_escrow(
    env,
    mentor_address,
    learner_address,
    0, // Invalid: zero amount
    session_id,
    token_address,
    session_end_time
);

// Result: Err(InvalidAmount)

// Recovery:
let result = create_escrow(
    env,
    mentor_address,
    learner_address,
    1_000_000_000, // Valid: positive amount
    session_id,
    token_address,
    session_end_time
);
// Result: Ok(escrow_id)
```

### Example 2: Releasing Escrow in Wrong Status

```rust
// Scenario: User tries to release escrow that's already released
let escrow = get_escrow(env, escrow_id)?;
// escrow.status = EscrowStatus::Released

let result = release_escrow(env, escrow_id);
// Result: Err(InvalidStatus)

// Recovery:
let escrow = get_escrow(env, escrow_id)?;
if escrow.status == EscrowStatus::Active {
    release_escrow(env, escrow_id)?;
} else {
    // Handle already released
    println!("Escrow already released");
}
```

### Example 3: Unauthorized Admin Operation

```rust
// Scenario: Non-admin tries to verify mentor
let result = verify_mentor(
    env,
    mentor_address,
    credential_hash,
    expiry
);
// Result: Err(Unauthorized)

// Recovery:
// 1. Use admin account
// 2. Or request admin to perform operation
let admin = get_admin(env)?;
if caller == admin {
    verify_mentor(env, mentor_address, credential_hash, expiry)?;
}
```

### Example 4: Dispute Reason Too Long

```rust
// Scenario: User tries to open dispute with very long reason
let long_reason = "a".repeat(501); // 501 characters
let result = open_dispute(env, escrow_id, long_reason);
// Result: Err(InvalidDisputeReason)

// Recovery:
let reason = "a".repeat(500); // Truncate to 500 characters
open_dispute(env, escrow_id, reason)?;
```

---

## Troubleshooting

For additional help:

- See TROUBLESHOOTING.md for common issues
- Check INTEGRATION_GUIDE.md for integration-specific errors
- Review contract source code for detailed error handling
- Contact support@mentorminds.io for assistance

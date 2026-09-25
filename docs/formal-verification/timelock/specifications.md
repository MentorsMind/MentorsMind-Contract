# Timelock Controller Contract Formal Specifications

## Overview

The Timelock Controller enforces a mandatory delay between scheduling and executing critical operations. It provides transparency and a window for stakeholders to react before changes take effect, serving as a crucial component of the protocol's security architecture.

---

## Contract Metadata

- **Contract**: `TimelockController`
- **Location**: `contracts/timelock/src/lib.rs`
- **Primary State**: `Operation`
- **Key Feature**: Collision-resistant operation IDs via SHA-256

---

## State Types

### Operation Record
```rust
pub struct Operation {
    pub proposer: Address,     // Who scheduled the operation
    pub target: Address,       // Contract to invoke
    pub function: Symbol,      // Function to call
    pub args: Vec<Val>,        // Function arguments
    pub ready_at: u64,         // Earliest execution timestamp
    pub done: bool,            // Execution flag
}
```

### Constants
```rust
pub const MIN_DELAY: u64 = 48 * 60 * 60;                    // 48 hours
pub const MAX_DELAY: u64 = 30 * 24 * 60 * 60;               // 30 days
pub const OPERATION_EXPIRY_SECS: u64 = 14 * 24 * 60 * 60;  // 14 days
pub const TIMESTAMP_TOLERANCE_SECS: u64 = 60;              // 1 minute
```

---

## Safety Properties

### SP1: Operation Uniqueness
**Property**: Operation IDs are collision-resistant via SHA-256 hash of full operation payload.

**ID Derivation**:
```rust
op_id = SHA256(
    proposer_xdr || 
    target_xdr || 
    function_xdr || 
    args_xdr || 
    ready_at_xdr || 
    nonce_xdr ||       // Monotonically increasing counter
    salt_xdr           // Caller-provided entropy
)
```

**Formal**:
```
∀ operations op1, op2:
  op1.id = op2.id ⟹ 
    (op1.proposer = op2.proposer) ∧
    (op1.target = op2.target) ∧
    (op1.function = op2.function) ∧
    (op1.args = op2.args) ∧
    (op1.ready_at = op2.ready_at) ∧
    (same nonce) ∧
    (op1.salt = op2.salt)
```

**Rationale**:
- Nonce prevents replay of identical operations
- Salt prevents op_id prediction by adversaries
- Committing to full payload prevents parameter substitution attacks

**Verification Strategy**:
- Collision test: generate many operations, verify no ID collisions
- Cryptographic assumption: SHA-256 collision resistance
- Salt variation test: same parameters + different salts → different IDs

### SP2: Delay Bounds Enforcement
**Property**: All operations have delays within `[MIN_DELAY, MAX_DELAY]`.

**Formal**:
```
schedule(op, delay) succeeds ⟹
  MIN_DELAY ≤ delay ≤ MAX_DELAY
```

**Verification Strategy**:
- Boundary tests: delay = MIN_DELAY - 1 (fails), MIN_DELAY (succeeds), MAX_DELAY (succeeds), MAX_DELAY + 1 (fails)
- Overflow test: delay = u64::MAX (fails)

### SP3: Temporal Execution Window
**Property**: Operations execute only within the valid time window.

**Valid Execution Window**:
```
ready_at + TOLERANCE ≤ now < ready_at + EXPIRY
```

**Formal**:
```
execute(op) succeeds ⟹
  (now ≥ op.ready_at + TIMESTAMP_TOLERANCE_SECS) ∧
  (now < op.ready_at + OPERATION_EXPIRY_SECS)
```

**Rationale**:
- `TOLERANCE` prevents premature execution due to timestamp rounding
- `EXPIRY` prevents stale operations from executing indefinitely

**Verification Strategy**:
- Boundary tests at key timestamps:
  - `now = ready_at` (fails: too early)
  - `now = ready_at + TOLERANCE - 1` (fails: too early)
  - `now = ready_at + TOLERANCE` (succeeds: exactly at boundary)
  - `now = ready_at + EXPIRY - 1` (succeeds: just before expiry)
  - `now = ready_at + EXPIRY` (fails: expired)

### SP4: Single Execution
**Property**: Each operation executes at most once.

**Formal**:
```
∀ operation op:
  execute(op) succeeds ⟹ op.done = true
  op.done = true ⟹ ∀ future attempts: execute(op) fails
```

**Implementation**:
```rust
if op.done {
    panic!("operation already done");
}
// ... execute ...
op.done = true;
env.storage().persistent().set(&DataKey::Op(operation_id), &op);
```

**Verification Strategy**:
- Double-execution test
- Reentrancy test (malicious target calls back)

### SP5: Cancellation Authorization
**Property**: Operations can be cancelled by proposer or admin.

**Formal**:
```
cancel(op, caller) succeeds ⟹
  (caller = op.proposer ∨ caller = admin) ∧
  ¬op.done
```

**Rationale**:
- Proposer can cancel their own operations
- Admin can emergency-cancel any operation

**Verification Strategy**:
- Unauthorized cancellation test
- Post-execution cancellation test (should fail)

---

## Liveness Properties

### LP1: Execution Availability
**Property**: If an operation is within its valid window and not done, execution is permissionless.

**Formal**:
```
(op.ready_at + TOLERANCE ≤ now < op.ready_at + EXPIRY) ∧
(¬op.done)
⟹ ∀ callers: can_execute(op)
```

**Rationale**: Permissionless execution ensures operations can't be censored by withholding execution.

### LP2: View Function Correctness
**Property**: `is_operation_ready` accurately reflects execution eligibility.

**Formal**:
```
is_operation_ready(op, now) ⟺
  (¬op.done) ∧
  (now ≥ op.ready_at + TIMESTAMP_TOLERANCE_SECS) ∧
  (now < op.ready_at + OPERATION_EXPIRY_SECS)
```

**Verification Strategy**:
- Compare `is_operation_ready` result with actual `execute` success/failure
- Property test: ∀ op, now: `is_operation_ready(op, now) ⟹ can_execute(op, now)`

### LP3: Expiry Detection
**Property**: `is_operation_expired` accurately identifies expired operations.

**Formal**:
```
is_operation_expired(op, now) ⟺
  (¬op.done) ∧
  (now ≥ op.ready_at + OPERATION_EXPIRY_SECS)
```

---

## Functional Correctness Properties

### FC1: Operation Scheduling Correctness
**Property**: Scheduled operations have correct `ready_at` timestamp.

**Formal**:
```
schedule(op, delay, now) ⟹
  op.ready_at = now + delay
```

**Overflow Handling**:
```rust
let ready_at = now.checked_add(delay).expect("timestamp overflow");
```

**Verification Strategy**:
- Arithmetic tests: verify `ready_at = now + delay`
- Overflow test: schedule with `delay = u64::MAX - now + 1` (panics)

### FC2: Operation Payload Immutability
**Property**: Once scheduled, operation parameters cannot be modified.

**Formal**:
```
∀ operation op after schedule:
  immutable(op.proposer) ∧
  immutable(op.target) ∧
  immutable(op.function) ∧
  immutable(op.args) ∧
  immutable(op.ready_at)
```

**Rationale**: Prevents parameter substitution attacks.

**Verification Strategy**:
- Code audit: no public functions modify operation fields
- Storage pattern: operations stored by ID, not mutable references

### FC3: Nonce Monotonicity
**Property**: Operation count (nonce) increments monotonically.

**Formal**:
```
schedule(op1) at nonce n1, schedule(op2) at nonce n2 ⟹
  (op1 scheduled before op2) ⟹ n1 < n2
```

**Implementation**:
```rust
let mut count: u64 = env.storage().instance().get(&DataKey::OpCount).unwrap_or(0);
count += 1;
env.storage().instance().set(&DataKey::OpCount, &count);
```

**Verification Strategy**:
- Concurrent scheduling test
- Overflow test: count approaching u64::MAX

---

## Temporal Properties

### TP1: Delay Lower Bound (Safety)
**Property**: No operation can execute sooner than MIN_DELAY after scheduling.

**Formal**:
```
schedule(op, delay, now) ∧ execute(op, now_exec) succeeds ⟹
  now_exec - now ≥ MIN_DELAY + TIMESTAMP_TOLERANCE_SECS
```

**Verification Strategy**:
- Time-travel test: schedule, immediately fast-forward to `now + MIN_DELAY - 1`, attempt execute (fails)
- Boundary test: fast-forward to `now + MIN_DELAY + TOLERANCE`, execute (succeeds)

### TP2: Delay Upper Bound (Flexibility)
**Property**: Operations can be scheduled up to MAX_DELAY in the future.

**Formal**:
```
schedule(op, MAX_DELAY, now) succeeds
```

### TP3: Expiry Enforcement
**Property**: Operations expire OPERATION_EXPIRY_SECS after ready_at.

**Formal**:
```
schedule(op, delay, now) ⟹
  op expires at: now + delay + OPERATION_EXPIRY_SECS
```

**Verification Strategy**:
- Time-travel test: schedule, fast-forward to `ready_at + EXPIRY - 1` (execute succeeds), fast-forward to `ready_at + EXPIRY` (execute fails)

---

## Attack Resistance Properties

### AR1: Collision Attack Prevention
**Property**: Adversaries cannot predict or collide operation IDs.

**Threat Model**:
1. **Collision Attack**: Find two operations with same ID
2. **Pre-image Attack**: Predict ID to front-run or censor
3. **Second Pre-image Attack**: Modify operation to match existing ID

**Defenses**:
- SHA-256 collision resistance (2^128 operations for birthday attack)
- Nonce prevents replay
- Salt (caller-provided entropy) prevents prediction

**Formal**:
```
P(collision) ≈ n^2 / 2^257 (birthday bound)
For n = 2^64 operations: P(collision) ≈ 2^-129 (negligible)
```

### AR2: Timing Manipulation Resistance
**Property**: Operation execution timing is determined by immutable `ready_at`.

**Attack Scenario**: Adversary manipulates timestamps to execute early or delay indefinitely.

**Defenses**:
- `ready_at` is immutable after scheduling
- Stellar ledger timestamps are consensus-based (platform guarantee)
- Tolerance and expiry bounds prevent gaming

### AR3: Front-Running Resistance
**Property**: Since execution is permissionless, front-running has no economic advantage.

**Rationale**: Any account can call `execute`; funds/effects go to `op.target`, not caller.

### AR4: Griefing Resistance
**Property**: Malicious operations can be cancelled by admin.

**Scenario**: Adversary schedules spam operations.

**Defense**:
```rust
cancel(op, admin) always succeeds if !op.done
```

---

## Data Integrity Properties

### DI1: Operation Storage Integrity
**Property**: Operations are stored persistently by ID and survive ledger archival.

**Storage Pattern**:
```rust
env.storage().persistent().set(&DataKey::Op(op_id), &op);
```

**Verification Strategy**:
- TTL test: verify operations persist across ledger bumps
- Storage key uniqueness: `DataKey::Op(id)` for each ID

### DI2: Admin Address Persistence
**Property**: Admin address is set once at initialization and persists.

**Formal**:
```
initialize(admin) ⟹
  ∀ future times: get_admin() = admin (unless transferred)
```

---

## Integration Properties (Cross-Contract)

### IP1: Treasury Buyback Integration
**Property**: Treasury's `buyback_and_burn` can only be called via timelock.

**Verification**:
- Treasury checks: `caller = registered_timelock`
- Timelock executes: `invoke_contract(&treasury, &buyback_and_burn, ...)`

**Property**:
```
treasury.buyback_and_burn(...) succeeds ⟹
  ∃ timelock operation op:
    op.target = treasury ∧
    op.function = "buyback_and_burn" ∧
    op executed via timelock
```

### IP2: Multisig + Timelock Composition
**Property**: Critical operations require multisig approval + timelock delay.

**Workflow**:
1. Multisig proposes action (e.g., update_fee)
2. Multisig threshold approvals met
3. Multisig executes → calls `timelock.schedule`
4. Timelock enforces MIN_DELAY
5. Anyone executes timelock operation after delay

**Verification**:
- Integration test: end-to-end workflow
- Verify delay cannot be bypassed

---

## Test Coverage Requirements

### Unit Tests
- ✅ Initialize with admin
- ✅ Schedule operation with valid delay
- ✅ Schedule with delay < MIN_DELAY (fails)
- ✅ Schedule with delay > MAX_DELAY (fails)
- ✅ Execute before ready_at (fails)
- ✅ Execute before ready_at + TOLERANCE (fails)
- ✅ Execute after ready_at + TOLERANCE (succeeds)
- ✅ Execute after expiry (fails)
- ✅ Execute operation twice (second fails)
- ✅ Cancel by proposer
- ✅ Cancel by admin
- ✅ Cancel by unauthorized (fails)
- ✅ Cancel after execution (fails)
- ✅ Different salts produce different op_ids

### Property Tests (Future)
- ⏳ Operation ID uniqueness over random inputs
- ⏳ Temporal correctness over random delays and execution times

### Integration Tests
- ⏳ Timelock + Treasury: schedule buyback_and_burn
- ⏳ Multisig + Timelock: multisig schedules via timelock

---

## Kani Harness Templates

### Template 1: Operation Uniqueness
```rust
#[cfg(kani)]
#[kani::proof]
fn verify_operation_uniqueness() {
    let env = Env::default();
    let contract = TimelockControllerClient::new(&env, &env.register_contract(None, TimelockController));
    
    contract.initialize(&Address::generate(&env));
    
    let proposer = Address::generate(&env);
    let target = Address::generate(&env);
    let function = Symbol::new(&env, "test");
    let args = Vec::new(&env);
    let delay = MIN_DELAY;
    
    // Two operations with different salts
    let salt1 = BytesN::from_array(&env, &[1u8; 32]);
    let salt2 = BytesN::from_array(&env, &[2u8; 32]);
    
    let id1 = contract.schedule(&proposer, &target, &function, &args, &delay, &salt1);
    let id2 = contract.schedule(&proposer, &target, &function, &args, &delay, &salt2);
    
    kani::assert(id1 != id2, "Different salts must produce different IDs");
}
```

### Template 2: Temporal Correctness
```rust
#[cfg(kani)]
#[kani::proof]
fn verify_temporal_execution() {
    let (env, contract, proposer, op_id) = setup_scheduled_operation();
    
    let now = env.ledger().timestamp();
    let op = contract.get_operation(&op_id);
    
    // Before ready_at + TOLERANCE: cannot execute
    if now < op.ready_at + TIMESTAMP_TOLERANCE_SECS {
        kani::assert(
            contract.try_execute(&op_id).is_err(),
            "Cannot execute before ready_at + TOLERANCE"
        );
    }
    
    // Within valid window: can execute
    if now >= op.ready_at + TIMESTAMP_TOLERANCE_SECS &&
       now < op.ready_at + OPERATION_EXPIRY_SECS {
        kani::assert(
            contract.try_execute(&op_id).is_ok(),
            "Must execute within valid window"
        );
    }
    
    // After expiry: cannot execute
    if now >= op.ready_at + OPERATION_EXPIRY_SECS {
        kani::assert(
            contract.try_execute(&op_id).is_err(),
            "Cannot execute after expiry"
        );
    }
}
```

---

## MIRAI Annotation Examples

```rust
#[pre(delay >= MIN_DELAY, "Delay must be at least MIN_DELAY")]
#[pre(delay <= MAX_DELAY, "Delay must be at most MAX_DELAY")]
#[post(result.is_ok() => storage_contains_operation(result.unwrap()), "Operation must be stored")]
pub fn schedule(
    env: Env,
    caller: Address,
    target: Address,
    function: Symbol,
    args: Vec<Val>,
    delay: u64,
    salt: BytesN<32>,
) -> Result<BytesN<32>, Error> {
    // implementation
}
```

---

## Formal Verification Roadmap

### Phase 1: Specifications (Current)
- ✅ Document all properties
- ✅ Define invariants
- ✅ Create test coverage matrix

### Phase 2: Tooling Setup
- ⏳ Configure Kani for timelock contract
- ⏳ Write proof harnesses

### Phase 3: Verification
- ⏳ Verify operation uniqueness (cryptographic assumption)
- ⏳ Verify temporal correctness
- ⏳ Verify single execution property

### Phase 4: Integration
- ⏳ Verify treasury integration property
- ⏳ Verify multisig composition property

---

## Security Considerations

### Delay Configuration Trade-offs
- **MIN_DELAY (48 hours)**: Balances security (reaction time) with usability
- **MAX_DELAY (30 days)**: Prevents indefinite scheduling (governance paralysis)
- **EXPIRY (14 days)**: Prevents stale operations while allowing reasonable execution window

### Emergency Response
- **Admin cancellation**: Provides escape hatch for malicious operations
- **Permissionless execution**: Ensures operations can't be censored
- **Expiry mechanism**: Prevents accumulation of stale operations

---

## References

- [Timelock Contract Source](../../../contracts/timelock/src/lib.rs)
- [Timelock Tests](../../../contracts/timelock/src/lib.rs#tests)
- [OpenZeppelin TimelockController (Solidity reference)](https://docs.openzeppelin.com/contracts/4.x/api/governance#TimelockController)
- [Compound Timelock (inspiration)](https://github.com/compound-finance/compound-protocol/blob/master/contracts/Timelock.sol)

---

**Last Updated**: 2026-07-24  
**Version**: 1.0.0  
**Status**: Ready for Verification Implementation

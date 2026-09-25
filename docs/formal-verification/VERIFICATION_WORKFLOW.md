# Formal Verification Workflow

This document provides step-by-step instructions for running formal verification on the MentorMinds contract suite.

---

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Tooling Setup](#tooling-setup)
3. [Running Verification](#running-verification)
4. [Interpreting Results](#interpreting-results)
5. [Adding New Properties](#adding-new-properties)
6. [CI Integration](#ci-integration)
7. [Troubleshooting](#troubleshooting)

---

## Prerequisites

### System Requirements
- **OS**: Linux (Ubuntu 20.04+), macOS (12+), or Windows (WSL2)
- **Rust**: 1.70+ (same as project requirement)
- **Memory**: 8GB+ RAM recommended
- **Disk**: 5GB+ free space

### Required Tools
```bash
# Rust toolchain
rustup update
rustup component add rust-src

# Soroban SDK (already installed if you can build the project)
cargo install soroban-cli
```

---

## Tooling Setup

### Option 1: Kani Rust Verifier (Recommended)

#### Installation
```bash
# Linux/macOS
cargo install --locked kani-verifier
cargo kani setup

# Windows (WSL2)
# Install from pre-built binary
curl -LO https://github.com/model-checking/kani/releases/latest/download/kani-verifier-x86_64-unknown-linux-gnu.tar.gz
tar -xzf kani-verifier-x86_64-unknown-linux-gnu.tar.gz
export PATH="$PWD/kani-verifier/bin:$PATH"
cargo kani setup
```

#### Verify Installation
```bash
cargo kani --version
# Expected output: kani-verifier 0.x.y
```

#### Configure for Soroban
Create `.kani/config.toml` in project root:
```toml
[verification]
# Increase unwinding for loops (default is 1)
unwind = 10

# Set timeout (in seconds)
timeout = 300

# Memory safety checks
memory-safety-checks = true

# Overflow checks (already enabled in Rust)
overflow-checks = true

[output]
# Detailed output
verbose = true

# Generate coverage reports
coverage = true
```

### Option 2: MIRAI (Abstract Interpretation)

#### Installation
```bash
cargo install mirai
```

#### Configure for Workspace
Add to `Cargo.toml`:
```toml
[profile.mirai]
inherits = "dev"

[dependencies]
mirai-annotations = { version = "1.12", optional = true }

[features]
mirai = ["mirai-annotations"]
```

### Option 3: Creusot (Deductive Verification)

> **Note**: Creusot integration is more complex and requires Why3/SMT solvers. See [Creusot documentation](https://github.com/creusot-rs/creusot) for setup.

---

## Running Verification

### Step 1: Build the Project
```bash
# Ensure the project builds without errors
cargo build --workspace --target wasm32-unknown-unknown --release
```

### Step 2: Run Unit Tests
```bash
# Run all tests to ensure baseline correctness
cargo test --workspace
```

### Step 3: Run Kani Verification

#### Verify a Single Contract
```bash
# Verify escrow contract
cd contracts/escrow
cargo kani --harness verify_fund_conservation

# Verify multisig contract
cd contracts/multisig_admin
cargo kani --harness verify_threshold_validity

# Verify timelock contract
cd contracts/timelock
cargo kani --harness verify_operation_uniqueness
```

#### Verify All Harnesses in a Contract
```bash
cd contracts/escrow
cargo kani
```

#### Verify Entire Workspace
```bash
# From project root
cargo kani --workspace
```

### Step 4: Run MIRAI Verification (Alternative)
```bash
# Set MIRAI as default verifier
export RUSTC_WRAPPER=mirai

# Run MIRAI on escrow contract
cd contracts/escrow
cargo mirai
```

---

## Interpreting Results

### Kani Output Examples

#### Success
```
VERIFICATION SUCCESSFUL
- All assertions passed
- No panics detected
- No undefined behavior found

Summary:
  Total checks: 42
  Successful: 42
  Failed: 0
  Coverage: 87.3%
```

#### Failure
```
VERIFICATION FAILED

Failed assertion: "Cannot execute before ready_at + TOLERANCE"
  File: contracts/timelock/src/lib.rs:178
  Counterexample:
    ready_at = 1000
    now = 1055
    TIMESTAMP_TOLERANCE_SECS = 60
    
Trace:
  1. schedule(delay=MIN_DELAY)
  2. env.ledger().timestamp() = 1055
  3. execute() called
  4. Assertion failed: now < ready_at + TOLERANCE
```

**Action**: Review the counterexample and fix the logic or adjust the property.

### MIRAI Output Examples

#### Success
```
MIRAI analysis completed successfully
- All preconditions satisfied
- All postconditions verified
- No unreachable code detected
```

#### Warning
```
Warning: Possible precondition violation
  Function: release_funds
  Line: 245
  Condition: escrow.status == EscrowStatus::Active
  
  Analysis suggests this function may be called with escrow.status = Released
  Consider adding runtime check or documenting assumption.
```

---

## Adding New Properties

### Step 1: Define the Property
Document in the appropriate `properties.md` file:
```markdown
### SP-NEW: Property Name
**Property**: Description of what must hold.

**Formal**:
```
∀ x: condition(x) ⟹ result(x)
```

**Verification Strategy**: How to verify this property.
```

### Step 2: Write a Kani Harness
Create a new test in the contract's `src/lib.rs`:

```rust
#[cfg(all(test, kani))]
mod verification {
    use super::*;
    
    #[kani::proof]
    fn verify_new_property() {
        let env = Env::default();
        // ... setup ...
        
        // Generate non-deterministic inputs
        let input = kani::any();
        
        // Execute operation
        let result = operation(&env, input);
        
        // Assert property
        kani::assert(
            property_holds(result),
            "Property description"
        );
    }
}
```

### Step 3: Run the New Harness
```bash
cargo kani --harness verify_new_property
```

### Step 4: Add to CI (see below)

---

## CI Integration

### GitHub Actions Workflow
Create `.github/workflows/formal-verification.yml`:

```yaml
name: Formal Verification

on:
  push:
    branches: [main, develop]
  pull_request:
    branches: [main, develop]

jobs:
  kani-verification:
    runs-on: ubuntu-latest
    steps:
      - uses: actions/checkout@v3
      
      - name: Install Rust
        uses: actions-rs/toolchain@v1
        with:
          toolchain: stable
          components: rust-src
      
      - name: Install Kani
        run: |
          cargo install --locked kani-verifier
          cargo kani setup
      
      - name: Verify Escrow Contract
        run: |
          cd contracts/escrow
          cargo kani --output-format json | tee kani-results.json
      
      - name: Verify Multisig Contract
        run: |
          cd contracts/multisig_admin
          cargo kani --output-format json | tee kani-results.json
      
      - name: Verify Timelock Contract
        run: |
          cd contracts/timelock
          cargo kani --output-format json | tee kani-results.json
      
      - name: Upload Results
        uses: actions/upload-artifact@v3
        with:
          name: verification-results
          path: '**/kani-results.json'
      
      - name: Check for Failures
        run: |
          if grep -q '"result": "FAILED"' **/kani-results.json; then
            echo "Verification failed!"
            exit 1
          fi
```

### GitLab CI Configuration
Create `.gitlab-ci.yml`:

```yaml
verification:
  stage: verify
  image: rust:latest
  before_script:
    - cargo install --locked kani-verifier
    - cargo kani setup
  script:
    - cd contracts/escrow && cargo kani
    - cd ../multisig_admin && cargo kani
    - cd ../timelock && cargo kani
  artifacts:
    reports:
      junit: verification-report.xml
  only:
    - main
    - merge_requests
```

---

## Troubleshooting

### Issue: Kani Installation Fails

**Symptom**: `cargo install kani-verifier` errors

**Solution**:
```bash
# Clear cargo cache
cargo clean
rm -rf ~/.cargo/registry

# Try with explicit version
cargo install --locked --version 0.34.0 kani-verifier

# Or use pre-built binary (Linux)
curl -LO https://github.com/model-checking/kani/releases/latest/download/kani-verifier-x86_64-unknown-linux-gnu.tar.gz
tar -xzf kani-verifier-x86_64-unknown-linux-gnu.tar.gz
export PATH="$PWD/kani-verifier/bin:$PATH"
```

### Issue: Verification Timeouts

**Symptom**: Kani runs for >5 minutes without completing

**Solution**:
```bash
# Increase timeout in .kani/config.toml
[verification]
timeout = 600

# Or reduce unwinding
[verification]
unwind = 5

# Or run with specific bounds
cargo kani --unwinding 5 --timeout 300
```

### Issue: Soroban SDK Incompatibility

**Symptom**: `error: cannot find macro 'contract' in this scope`

**Solution**:
```bash
# Kani doesn't fully support Soroban macros yet
# Use conditional compilation:

#[cfg(not(kani))]
#[contract]
pub struct MyContract;

#[cfg(kani)]
pub struct MyContract;

#[cfg_attr(not(kani), contractimpl)]
impl MyContract {
    // methods
}
```

### Issue: Out of Memory

**Symptom**: Kani crashes with OOM error

**Solution**:
```bash
# Reduce verification scope
cargo kani --harness specific_harness_only

# Or increase system memory
# Or use bounded verification with smaller unwind values
cargo kani --unwinding 3
```

### Issue: False Positives

**Symptom**: Kani reports failures for valid code

**Solution**:
```rust
// Add assumptions to constrain inputs
#[kani::proof]
fn my_proof() {
    let x = kani::any();
    kani::assume(x > 0); // Constrain to valid domain
    kani::assume(x < 1000);
    
    let result = my_function(x);
    kani::assert(result.is_ok());
}
```

### Issue: Verification Takes Too Long

**Strategies**:
1. **Break down properties**: Verify smaller properties independently
2. **Use stubs**: Replace complex external calls with stubs
3. **Bound loops**: Add `#[kani::unwind(n)]` attributes
4. **Parallelize**: Run harnesses in parallel

```bash
# Run harnesses in parallel
cargo kani --harness prop1 &
cargo kani --harness prop2 &
cargo kani --harness prop3 &
wait
```

---

## Best Practices

### 1. Start Small
- Begin with simple properties (e.g., arithmetic correctness)
- Gradually add complex properties (e.g., state machine invariants)

### 2. Incremental Verification
- Verify one function at a time
- Use stubs for unverified dependencies
- Build up to full contract verification

### 3. Property Decomposition
- Break complex properties into smaller sub-properties
- Verify each sub-property independently
- Compose results

### 4. Documentation
- Document all assumptions
- Explain why properties hold
- Note verification limitations

### 5. Continuous Integration
- Run verification on every commit
- Fail builds on verification failures
- Track verification coverage over time

---

## Verification Metrics

### Coverage Tracking
```bash
# Generate coverage report
cargo kani --coverage

# View coverage
open target/kani/coverage/index.html
```

### Property Catalog
Maintain a checklist of verified properties:

```markdown
## Escrow Contract
- [x] SP1: Fund Conservation
- [x] SP2: No Fund Loss
- [x] SP3: State Machine Integrity
- [ ] SP4: Double-Spend Prevention (in progress)
- [ ] SP5: Authorization Correctness (pending)
```

### Verification Performance
Track verification times to detect regressions:

| Property                  | Time (s) | Status |
|---------------------------|----------|--------|
| Escrow Fund Conservation  | 42       | ✅      |
| Multisig Threshold        | 18       | ✅      |
| Timelock Temporal         | 67       | ✅      |

---

## Advanced Techniques

### Symbolic Execution with KLEE (Future)
```bash
# Generate LLVM bitcode
cargo build --release
llvm-extract target/release/escrow.wasm -o escrow.bc

# Run KLEE
klee --max-time=600 escrow.bc
```

### SMT-Based Verification with Z3
```rust
// Use Z3 for arithmetic proofs
#[cfg(z3)]
fn verify_fee_calculation() {
    use z3::*;
    let cfg = Config::new();
    let ctx = Context::new(&cfg);
    let solver = Solver::new(&ctx);
    
    let amount = Int::new_const(&ctx, "amount");
    let fee_bps = Int::new_const(&ctx, "fee_bps");
    let platform_fee = Int::new_const(&ctx, "platform_fee");
    
    // Assert: platform_fee = (amount * fee_bps) / 10000
    solver.assert(&platform_fee._eq(&amount.mul(&[&fee_bps]).div(&Int::from_i64(&ctx, 10000))));
    
    // Check satisfiability
    assert_eq!(solver.check(), SatResult::Sat);
}
```

---

## Resources

### Documentation
- [Kani User Guide](https://model-checking.github.io/kani/)
- [MIRAI Documentation](https://github.com/facebookexperimental/MIRAI/blob/main/documentation/Overview.md)
- [Creusot Tutorial](https://github.com/creusot-rs/creusot/wiki)

### Research Papers
- [Formal Verification of Rust Programs](https://arxiv.org/abs/2010.12033)
- [Bounded Model Checking for Rust](https://arxiv.org/abs/2106.09535)
- [Smart Contract Verification](https://arxiv.org/abs/2008.02712)

### Community
- [Kani GitHub Discussions](https://github.com/model-checking/kani/discussions)
- [Rust Formal Methods Interest Group](https://rust-formal-methods.github.io/)

---

**Last Updated**: 2026-07-24  
**Version**: 1.0.0  
**Status**: Ready for Use

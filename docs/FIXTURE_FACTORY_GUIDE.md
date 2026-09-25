# Test Fixture Factory Guide

## Overview

The deterministic test fixture factory provides a unified, consistent way to set up cross-contract integration tests. It eliminates duplication of mock implementations and ensures all integration tests use compatible mock implementations.

## Problem Solved

Previously, each contract had its own ad-hoc `Fixture` struct and `setup()` function. Cross-contract tests had to manually wire up 5-8 contracts, repeating initialization boilerplate. The same `MockMNT` token was reimplemented 6+ times across different test modules with subtle differences in storage key schemes, making cross-contract mocking unreliable.

## Architecture

### Components

1. **Unified Mock Contracts** (`contracts/shared/src/test_fixture.rs`)
   - `MockMNT`: Standardized mock token implementation
   - `MockSnapshot`: Governance snapshot contract mock
   - `MockKYCRegistry`: KYC verification mock
   - `MockSanctions`: Sanctions screening mock

2. **Fixture Builder** (`tests/fixture_factory.rs`)
   - `FixtureBuilder`: Builder pattern for deterministic contract deployment
   - `Fixture`: Provides access to all deployed contracts and helper methods
   - `TestAddresses`: Deterministic test address generation
   - `FixtureConfig`: Configurable parameters for contract initialization

## Usage

### Basic Usage

```rust
use crate::fixture_factory::FixtureBuilder;

let env = soroban_sdk::Env::default();

// Build a basic fixture with MNT token and Escrow
let fixture = FixtureBuilder::new(&env)
    .deploy_mnt_token()
    .deploy_escrow()
    .build();

// Access contracts
let mnt_client = fixture.mnt_client();
let escrow_client = fixture.escrow_client();
```

### Full Stack Setup

```rust
let fixture = FixtureBuilder::new(&env)
    .deploy_mnt_token()
    .deploy_snapshot()
    .deploy_governance()
    .deploy_staking()
    .deploy_delegation()
    .deploy_verification()
    .deploy_escrow()
    .deploy_kyc_registry()
    .deploy_sanctions()
    .build();
```

### Custom Configuration

```rust
use crate::fixture_factory::{FixtureBuilder, FixtureConfig};

let config = FixtureConfig {
    fee_bps: 300, // 3%
    voting_period_secs: 3600, // 1 hour
    quorum_bps: 500, // 5%
    auto_release_delay_secs: 72 * 60 * 60, // 72 hours
    initial_token_supply: 1_000_000_000,
    ..Default::default()
};

let fixture = FixtureBuilder::new(&env)
    .with_config(config)
    .deploy_mnt_token()
    .deploy_escrow()
    .build();
```

### Helper Methods

```rust
// Verify a mentor (requires verification contract)
fixture.verify_mentor();

// Create an escrow with default parameters
let escrow_id = fixture.create_escrow(10_000);

// Advance ledger time
fixture.advance_time(3600);
```

## Migration Guide

### Before (Ad-hoc Fixture)

```rust
struct Fixture<'a> {
    env: Env,
    escrow: EscrowContractClient<'a>,
    escrow_id: Address,
    verif: VerificationContractClient<'a>,
    admin: Address,
    mentor: Address,
    learner: Address,
    treasury: Address,
    token: Address,
}

impl<'a> Fixture<'a> {
    fn new(env: &'a Env, fee_bps: u32) -> Self {
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.timestamp = 10_000);

        let admin = Address::generate(env);
        let mentor = Address::generate(env);
        let learner = Address::generate(env);
        let treasury = Address::generate(env);

        // --- Token ---
        let (token, sac) = create_token(env, &admin);
        sac.mint(&learner, &1_000_000);

        // --- Escrow ---
        let escrow_id = env.register_contract(None, EscrowContract);
        let escrow = EscrowContractClient::new(env, &escrow_id);
        let mut approved = Vec::new(env);
        approved.push_back(token.clone());
        escrow.initialize(&admin, &treasury, &fee_bps, &approved, &0u64, &None);

        // --- Verification ---
        let verif_id = env.register_contract(None, VerificationContract);
        let verif = VerificationContractClient::new(env, &verif_id);
        verif.initialize(&admin);

        Fixture { /* ... */ }
    }
}
```

### After (Fixture Factory)

```rust
use crate::fixture_factory::FixtureBuilder;

let fixture = FixtureBuilder::new(&env)
    .deploy_mnt_token()
    .deploy_verification()
    .deploy_escrow()
    .build();

// Access contracts via getter methods
let escrow_client = fixture.escrow_client();
let verif_client = fixture.verification_client().unwrap();
```

## Benefits

1. **Consistency**: All tests use the same mock implementations with identical storage schemes
2. **Reduced Duplication**: Eliminates 6+ duplicate MockMNT implementations
3. **Deterministic**: Address generation and initialization are predictable
4. **Composable**: Builder pattern allows flexible contract combinations
5. **Maintainable**: Single source of truth for mock contracts
6. **Type-Safe**: Client generation ensures correct function signatures

## Contract Coverage

The fixture factory supports the following contracts:

- **Core**: MNT Token, Escrow
- **Governance**: Governance, Timelock, Snapshot
- **Identity**: Verification, KYC Registry, Sanctions
- **DeFi**: Staking, Delegation
- **Reputation**: Reputation Contract

## Best Practices

1. **Use the fixture factory** for all new cross-contract integration tests
2. **Migrate existing tests** incrementally to use the fixture factory
3. **Remove duplicate mocks** from individual contract test files
4. **Configure appropriately** using `FixtureConfig` for test-specific parameters
5. **Use helper methods** like `verify_mentor()` and `create_escrow()` for common operations

## Examples

See `tests/fixture_factory_example.rs` for comprehensive examples of:
- Basic fixture setup
- Governance stack integration
- Full-stack deployment
- Custom address configuration

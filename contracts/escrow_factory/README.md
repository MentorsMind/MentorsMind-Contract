# MentorsMind Escrow Factory

A factory contract for deploying isolated escrow instances using the minimal proxy pattern. This contract improves security isolation and upgradeability by creating separate escrow contract instances for each mentoring session.

## Features

- **Factory Pattern**: Deploys new escrow contract instances on demand
- **Minimal Proxy**: Each deployed escrow is a minimal proxy (clone) of the implementation contract
- **Session Isolation**: Each session gets its own isolated escrow contract instance
- **Upgradeability**: Factory admin can upgrade the implementation for future deployments (existing escrows unaffected)
- **Pagination**: Efficient retrieval of all deployed escrows with pagination support
- **Events**: Emits events for escrow deployment and implementation upgrades

## Contract Functions

### Core Functions

- `initialize(admin, implementation_address)` - Initialize the factory
- `deploy_escrow(mentor, learner, amount, token, session_id) -> Address` - Deploy new escrow instance
- `get_escrow_address(session_id) -> Option<Address>` - Get escrow address by session ID
- `get_all_escrows(page, page_size) -> Vec<EscrowInfo>` - Get all escrows with pagination

### Admin Functions

- `upgrade_implementation(new_implementation)` - Upgrade implementation for future deployments
- `get_admin() -> Address` - Get admin address
- `get_implementation() -> Address` - Get current implementation address

### Query Functions

- `get_escrow_count() -> u64` - Get total number of deployed escrows

## Data Structures

### EscrowInfo
```rust
pub struct EscrowInfo {
    pub address: Address,      // Escrow contract address
    pub session_id: Symbol,     // Unique session identifier
    pub mentor: Address,        // Mentor address
    pub learner: Address,       // Learner address
    pub created_at: u64,        // Creation timestamp
}
```

## Events

- `escrow_deployed(session_id, contract_address)` - Emitted when new escrow is deployed
- `implementation_upgraded(new_implementation, timestamp)` - Emitted when implementation is upgraded

## Usage

1. Initialize the factory with admin and implementation contract addresses
2. Deploy new escrow instances using `deploy_escrow`
3. Look up escrow addresses using session IDs
4. Retrieve paginated lists of all escrows
5. Upgrade implementation when needed (only affects future deployments)

## Security Considerations

- Each escrow instance is isolated from others
- Factory admin controls implementation upgrades
- Session IDs must be unique to prevent conflicts
- Only admin can upgrade implementation contract

## Testing

Run the comprehensive test suite:

```bash
cargo test --package mentorminds-escrow-factory
```

The test suite covers:
- Factory initialization
- Escrow deployment and lookup
- Pagination functionality
- Implementation upgrades
- Error handling and edge cases

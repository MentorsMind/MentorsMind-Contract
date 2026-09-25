# Contributing to MentorMinds Contracts

Thank you for your interest in contributing to MentorMinds! This guide will help you set up a local development environment and understand our contribution process.

## 🚀 Quick Start

### Prerequisites

- **Docker** and **Docker Compose** installed
- **Node.js** 18+ 
- **Rust** 1.70+ with `wasm32-unknown-unknown` target
- **Soroban CLI** (latest version)

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
rustup target add wasm32-unknown-unknown

# Install Soroban CLI
cargo install --locked soroban-cli
```

### Local Development Setup

1. **Clone and setup the repository:**
```bash
git clone https://github.com/MentorsMind/MentorsMind-Contract.git
cd MentorsMind-Contract
```

2. **Start the local development environment:**
```bash
npm run local:start
```

This command will:
- Start a local Stellar node using Docker
- Create and fund 5 test accounts (admin, mentor1, mentor2, learner1, learner2)
- Build and deploy all smart contracts
- Save configuration to `deployed/local.json`

3. **Seed sample data (optional):**
```bash
npm run local:seed
```

This creates sample escrows, sessions, reviews, and a dispute scenario for testing.

4. **Check the status:**
```bash
npm run local:status
```

## 🛠️ Development Workflow

### Available Scripts

```bash
# Local environment management
npm run local:start    # Start local Stellar node and deploy contracts
npm run local:stop     # Stop the local Stellar node
npm run local:reset    # Reset the entire environment (stop, clean, start)
npm run local:seed     # Create sample data for testing
npm run local:status   # Check environment status
npm run local:logs     # View Stellar container logs

# Contract development
npm run build          # Build all contracts
npm run build:escrow   # Build only escrow contract
npm run optimize       # Optimize WASM files
npm run test          # Run all tests
npm run clean         # Clean build artifacts
```

### Service Endpoints

When the local environment is running, these services are available:

- **Horizon API**: http://localhost:8000
- **Stellar RPC**: http://localhost:8001  
- **Friendbot** (funding): http://localhost:8002
- **Soroban RPC**: http://localhost:8003

### Test Accounts

The setup creates these pre-funded accounts:

| Account | Role | Identity Name |
|---------|------|---------------|
| admin | Platform administration | `local_admin` |
| mentor1 | Web Development mentor | `local_mentor1` |
| mentor2 | Smart Contract mentor | `local_mentor2` |
| learner1 | Test learner | `local_learner1` |
| learner2 | Test learner | `local_learner2` |

Account details are saved in `deployed/accounts.json`.

## 📁 Project Structure

```
mentorminds-contracts/
├── contracts/              # Smart contracts
│   ├── verification/      # Mentor verification contract
│   ├── oracle/           # Price oracle contract
│   ├── timelock/         # Timelock contract
│   └── treasury/         # Treasury contract
├── escrow/               # Main escrow contract
├── scripts/              # Deployment and utility scripts
│   ├── setup-local.sh    # Local environment setup
│   ├── seed-local.sh     # Sample data creation
│   └── deploy.sh         # Production deployment
├── deployed/             # Generated configuration files
│   ├── local.json       # Local contract addresses
│   ├── accounts.json     # Test account details
│   └── seed_*.json       # Sample data files
├── tests/               # Integration tests
├── docker-compose.yml   # Local Stellar configuration
└── package.json         # NPM scripts and metadata
```

## 🔧 Contract Development

### Building Contracts

```bash
# Build all contracts
npm run build

# Build specific contract
npm run build:escrow

# Optimize WASM for deployment
npm run optimize
```

### Testing

```bash
# Run unit tests
npm run test

# Run tests for specific contract
cd escrow && cargo test
```

### Local Deployment

The local setup script automatically handles deployment, but you can manually deploy:

```bash
# Deploy escrow contract
soroban contract deploy \
  --wasm escrow/target/wasm32-unknown-unknown/release/mentorminds_escrow.wasm \
  --source local_admin \
  --network standalone

# Initialize contract
soroban contract invoke \
  --id <CONTRACT_ID> \
  --source local_admin \
  --network standalone \
  -- initialize \
  --admin <ADMIN_ADDRESS> \
  --platform_fee 5
```

## 🌐 Network Configuration

### Local Network

The local setup uses a standalone network with these settings:

- **Network Name**: `standalone`
- **Network Passphrase**: `Standalone Network ; February 2017`
- **Friendbot**: Available for instant funding

### Adding Other Networks

```bash
# Add testnet
soroban config network add testnet \
  --rpc-url https://soroban-testnet.stellar.org:443 \
  --network-passphrase "Test SDF Network ; September 2015"

# Add mainnet
soroban config network add mainnet \
  --rpc-url https://mainnet.stellar.org:443 \
  --network-passphrase "Public Global Stellar Network ; September 2015"
```

## 🧪 Testing with Sample Data

After running `npm run local:seed`, you'll have:

- **3 escrows** with different states (completed, disputed)
- **2 verified mentors** with different skill sets
- **2 completed sessions** with reviews
- **1 active dispute** for testing
- **Oracle price feeds** for XLM and metrics

### Useful Testing Commands

```bash
# Check escrow details
soroban contract invoke \
  --id $(jq -r '.contracts.escrow.contract_id' deployed/local.json) \
  --network standalone \
  -- get_escrow \
  --escrow_id <ESCROW_ID>

# View mentor reviews
soroban contract invoke \
  --id $(jq -r '.contracts.verification.contract_id' deployed/local.json) \
  --network standalone \
  -- get_mentor_reviews \
  --mentor <MENTOR_ADDRESS>

# Check oracle prices
soroban contract invoke \
  --id $(jq -r '.contracts.oracle.contract_id' deployed/local.json) \
  --network standalone \
  -- get_all_prices
```

## 🧪 Testing Guidelines

- Keep contract tests close to the behavior they exercise. Unit tests belong next to the contract crate, while cross-contract and end-to-end checks belong in `tests/`.
- Cover both the happy path and at least one failure case for every new control-flow branch.
- Prefer deterministic fixtures and explicit ledger timestamps so test output stays stable across runs.
- When a change affects storage layout, authorization, or upgrade behavior, add a regression test before merging.
- Use coverage reports as a signal, not a target; the real goal is to keep critical security and state-transition paths exercised.

## 📝 Commenting Guidelines

- Add inline comments only where the intent is not obvious from the code alone: state transitions, pricing math, authorization shortcuts, or storage TTL handling.
- Explain the assumption being made, not the line of code itself.
- Keep TODO and FIXME notes concrete, time-bound, and easy to search for during follow-up work.
- Remove comments once the implementation becomes self-explanatory or the assumption no longer applies.

## 🔍 Debugging

### Common Issues

1. **Docker port conflicts**: Ensure ports 8000-8003 are available
2. **Soroban CLI version**: Use the latest version matching contract dependencies
3. **Account funding**: Use friendbot for local accounts, Stellar Laboratory for testnet
4. **Contract storage**: Reset environment with `npm run local:reset` if needed

### Viewing Logs

```bash
# View Stellar container logs
npm run local:logs

# View specific service logs
docker-compose logs stellar
```

### Environment Reset

```bash
# Complete reset
npm run local:reset

# Manual cleanup
npm run local:stop
docker volume rm mentorminds-contract_stellar_data
rm -rf deployed/*.json
```

## 📝 Code Style

### Rust Contracts

- Use `cargo fmt` for formatting
- Use `cargo clippy` for linting
- Follow Soroban best practices
- Include comprehensive error messages
- Add inline documentation for public functions

### Shell Scripts

- Use `shellcheck` for validation
- Follow POSIX compatibility
- Include error handling with `set -e`
- Use descriptive variable names
- Add colored output for better UX

## 🤝 Contribution Process

1. **Fork** the repository
2. **Create** a feature branch: `git checkout -b feature/amazing-feature`
3. **Setup** local environment: `npm run local:start`
4. **Make** your changes
5. **Test** thoroughly: `npm run test && npm run local:seed`
6. **Commit** your changes with descriptive messages
7. **Push** to your fork: `git push origin feature/amazing-feature`
8. **Create** a Pull Request

### Pull Request Guidelines

- Include tests for new features
- Update documentation as needed
- Ensure all existing tests pass
- Describe the problem and solution clearly
- Link relevant issues in the description

## 🐛 Bug Reports

When reporting bugs, include:

- Environment details (OS, Docker version, etc.)
- Steps to reproduce
- Expected vs actual behavior
- Relevant logs and error messages
- Contract addresses (if applicable)

## 💡 Feature Requests

Feature requests should:

- Describe the use case clearly
- Explain why it's valuable
- Consider implementation complexity
- Suggest API design if applicable

## 📚 Resources

- [Soroban Documentation](https://soroban.stellar.org/docs)
- [Stellar Laboratory](https://laboratory.stellar.org/)
- [Soroban Examples](https://github.com/stellar/soroban-examples)
- [Stellar Discord](https://discord.gg/stellardev)

## 🆘 Getting Help

- Create an issue on GitHub
- Join the Stellar Discord
- Check existing documentation
- Review similar contracts in the ecosystem

---

Thank you for contributing to MentorMinds! Your contributions help make decentralized mentoring more accessible and secure. 🌟

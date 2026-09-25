# Formal Verification Specifications

This directory contains machine-verifiable specifications and protocol invariants for the MentorMinds contract suite, specifically targeting the escrow, multisig, and timelock contracts for future formal verification.

## Purpose

The specifications in this directory serve to:

1. **Explicitly document protocol invariants** that must hold across all execution paths
2. **Define safety and liveness properties** for critical contract operations
3. **Provide machine-readable specifications** compatible with formal verification tools (Kani, MIRAI, etc.)
4. **Establish security assumptions** and trust boundaries
5. **Enable automated verification** of critical protocol guarantees

## Directory Structure

```
formal-verification/
├── README.md                          # This file
├── INVARIANTS.md                      # Core protocol invariants
├── VERIFICATION_WORKFLOW.md           # How to run verification
├── escrow/
│   ├── specifications.md              # Human-readable escrow specs
│   ├── invariants.rs                  # Escrow invariants (Rust)
│   └── properties.md                  # Safety and liveness properties
├── multisig/
│   ├── specifications.md              # Human-readable multisig specs
│   ├── invariants.rs                  # Multisig invariants (Rust)
│   └── properties.md                  # Safety and liveness properties
├── timelock/
│   ├── specifications.md              # Human-readable timelock specs
│   ├── invariants.rs                  # Timelock invariants (Rust)
│   └── properties.md                  # Safety and liveness properties
└── tooling/
    ├── kani-setup.md                  # Kani Rust Verifier configuration
    └── templates/                     # Verification harness templates
```

## Verification Tools

The specifications are designed to be compatible with:

1. **[Kani Rust Verifier](https://github.com/model-checking/kani)** - Bounded model checking for Rust
2. **[MIRAI](https://github.com/facebookexperimental/MIRAI)** - Abstract interpretation for Rust
3. **[Creusot](https://github.com/creusot-rs/creusot)** - Deductive verification for Rust
4. **Manual auditing** - Human-readable specifications for security reviews

## Core Verification Goals

### Escrow Contract
- **Fund safety**: Funds can never be double-spent or lost
- **State consistency**: State transitions follow the defined state machine
- **Authorization**: Only authorized parties can trigger state changes
- **Accounting**: Platform fee calculations are correct and funds sum properly

### Multisig Contract
- **Threshold enforcement**: Actions execute only when threshold approvals are met
- **Authorization**: Only registered signers can approve proposals
- **Atomicity**: No partial executions or double-executions
- **Signer set consistency**: Add/remove operations maintain valid configurations

### Timelock Contract
- **Temporal safety**: Operations execute only after the delay period
- **Operation uniqueness**: Operation IDs are collision-resistant
- **Cancellation safety**: Only authorized parties can cancel operations
- **Execution atomicity**: Operations execute exactly once

## Security Assumptions

The specifications explicitly document:

1. **Cryptographic assumptions** (hash function collision resistance)
2. **Platform assumptions** (Stellar ledger integrity, timestamp monotonicity)
3. **Economic assumptions** (token standards, overflow protection)
4. **Access control assumptions** (authentication via `require_auth`)

## Quick Start

1. **Read the invariants**: Start with `INVARIANTS.md` for an overview
2. **Review contract-specific specs**: Navigate to each contract's directory
3. **Run verification workflow**: Follow `VERIFICATION_WORKFLOW.md`
4. **Add new properties**: Use templates in `tooling/templates/`

## Status

- ✅ Protocol invariants documented
- ✅ Security assumptions defined
- ✅ Escrow specifications complete
- ✅ Multisig specifications complete
- ✅ Timelock specifications complete
- ⏳ Kani harnesses (future work)
- ⏳ MIRAI annotations (future work)

## Contributing

When adding new verification properties:

1. Document the property in the contract's `properties.md`
2. Add Rust-level invariants to `invariants.rs`
3. Update `INVARIANTS.md` if it's a cross-contract invariant
4. Add test cases that demonstrate the property

## References

- [Kani Rust Verifier Documentation](https://model-checking.github.io/kani/)
- [MIRAI Documentation](https://github.com/facebookexperimental/MIRAI/blob/main/documentation/Overview.md)
- [Soroban Security Best Practices](https://soroban.stellar.org/docs/learn/security)
- [Stellar Documentation](https://developers.stellar.org/)

## License

MIT License - See repository root LICENSE file

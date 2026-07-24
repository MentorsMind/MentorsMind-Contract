# @mentorminds/contracts-sdk

This is the fully-typed TypeScript SDK for MentorMinds Soroban smart contracts.
It features auto-generated bindings mapping to Rust structures, avoiding manual XDR encoding.

## Installation

Since this is an internal workspace package, you can just depend on it locally:
```json
{
  "dependencies": {
    "@mentorminds/contracts-sdk": "workspace:*"
  }
}
```

## Usage

### Escrow Client

```typescript
import { EscrowClient, ContractError } from '@mentorminds/contracts-sdk';

const client = new EscrowClient({
  networkPassphrase: 'Test SDF Network ; September 2015',
  contractId: 'C...',
  rpcUrl: 'https://soroban-testnet.stellar.org:443',
  publicKey: 'G...',
  // optional signTransaction handler, etc.
});

// Example interaction
async function lockFunds() {
    try {
        const tx = await client.createSession({ ... });
        await tx.signAndSend();
    } catch (error) {
        if (error instanceof ContractError) {
            console.error('Contract panic:', error.message);
        }
    }
}
```

Available exported clients:
- `EscrowClient`
- `ReputationClient` (Verification Contract)
- `SessionRegistryClient` (Session NFT Contract)
- `TokenClient`
- `StakingClient`

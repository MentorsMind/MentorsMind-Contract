# Integration Guide

## Overview

This guide provides comprehensive instructions for integrating MentorsMind smart contracts with backend and frontend applications. It includes contract interfaces, authentication, error handling, and best practices.

## Table of Contents

1. [Contract Interfaces](#contract-interfaces)
2. [Integration Examples](#integration-examples)
3. [Authentication & Authorization](#authentication--authorization)
4. [Error Handling](#error-handling)
5. [Best Practices](#best-practices)
6. [Sample Code](#sample-code)

---

## Contract Interfaces

### Escrow Contract

**Contract Address**: Deployed on Stellar Soroban (testnet/mainnet)

**Core Functions**:

```rust
// Initialize contract
pub fn initialize(env: Env, admin: Address) -> Result<(), Error>

// Create escrow
pub fn create_escrow(
    env: Env,
    mentor: Address,
    learner: Address,
    amount: i128,
    session_id: Symbol,
    token_address: Address,
    session_end_time: u64
) -> Result<u64, Error>

// Release funds
pub fn release_escrow(env: Env, escrow_id: u64) -> Result<(), Error>

// Open dispute
pub fn open_dispute(
    env: Env,
    escrow_id: u64,
    reason: String
) -> Result<(), Error>

// Resolve dispute
pub fn resolve_dispute(
    env: Env,
    escrow_id: u64,
    mentor_pct: u32
) -> Result<(), Error>

// Get escrow details
pub fn get_escrow(env: Env, escrow_id: u64) -> Result<Escrow, Error>

// List escrows
pub fn list_escrows(env: Env) -> Result<Vec<Escrow>, Error>
```

### Verification Contract

**Core Functions**:

```rust
// Initialize contract
pub fn initialize(env: Env, admin: Address) -> Result<(), Error>

// Verify mentor
pub fn verify_mentor(
    env: Env,
    mentor: Address,
    credential_hash: BytesN<32>,
    expiry: u64
) -> Result<(), Error>

// Check if verified
pub fn is_verified(env: Env, mentor: Address) -> Result<bool, Error>

// Get verification record
pub fn get_verification(env: Env, mentor: Address) -> Result<VerificationRecord, Error>

// Revoke verification
pub fn revoke_verification(env: Env, mentor: Address) -> Result<(), Error>
```

### Upgrade Registry Contract

**Core Functions**:

```rust
// Initialize registry
pub fn initialize(env: Env, admin: Address) -> Result<(), Error>

// Register upgrade
pub fn register_upgrade(
    env: Env,
    contract_name: Symbol,
    old_version: u32,
    new_version: u32,
    changelog_hash: BytesN<32>
) -> Result<(), Error>

// Subscribe to upgrades
pub fn subscribe_to_upgrades(
    env: Env,
    contract_name: Symbol,
    subscriber: Address
) -> Result<(), Error>

// Get upgrade history
pub fn get_upgrade_history(
    env: Env,
    contract_name: Symbol
) -> Result<Vec<UpgradeRecord>, Error>
```

---

## Integration Examples

### JavaScript/TypeScript Integration

#### Setup

```typescript
import {
  Keypair,
  Networks,
  TransactionBuilder,
  Operation,
} from "@stellar/stellar-sdk";
import { ContractSpec, MethodOptions } from "@stellar/stellar-sdk/contract";

// Initialize Soroban client
const rpcUrl = "https://soroban-testnet.stellar.org:443";
const networkPassphrase = Networks.TESTNET_NETWORK_PASSPHRASE;

// Load contract
const contractId = "CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4";
const spec = new ContractSpec(contractAbi);
```

#### Create Escrow

```typescript
async function createEscrow(
  mentorAddress: string,
  learnerAddress: string,
  amount: bigint,
  sessionId: string,
  tokenAddress: string,
  sessionEndTime: number,
): Promise<string> {
  const keypair = Keypair.fromSecret(process.env.ADMIN_SECRET_KEY!);
  const account = await server.getAccount(keypair.publicKey());

  const transaction = new TransactionBuilder(account, {
    fee: "100",
    networkPassphrase,
  })
    .addOperation(
      Operation.invokeHostFunction({
        func: xdr.HostFunction.hostFunctionTypeInvokeContract([
          xdr.ContractIdPreimage.contractIdFromSourceAccount({
            networkId: xdr.Hash.fromXDR(Buffer.from(networkPassphrase, "utf8")),
            sourceAccount: keypair.publicKey(),
            salt: xdr.Uint64.fromString("0"),
          }),
        ]),
        args: [
          nativeToScVal("create_escrow"),
          nativeToScVal(mentorAddress),
          nativeToScVal(learnerAddress),
          nativeToScVal(amount),
          nativeToScVal(sessionId),
          nativeToScVal(tokenAddress),
          nativeToScVal(sessionEndTime),
        ],
      }),
    )
    .setTimeout(30)
    .build();

  const signedTx = transaction.sign(keypair);
  const response = await server.submitTransaction(signedTx);

  return response.id;
}
```

#### Release Escrow

```typescript
async function releaseEscrow(escrowId: bigint): Promise<string> {
  const keypair = Keypair.fromSecret(process.env.MENTOR_SECRET_KEY!);
  const account = await server.getAccount(keypair.publicKey());

  const transaction = new TransactionBuilder(account, {
    fee: "100",
    networkPassphrase,
  })
    .addOperation(
      Operation.invokeHostFunction({
        func: xdr.HostFunction.hostFunctionTypeInvokeContract([
          xdr.ContractIdPreimage.contractIdFromSourceAccount({
            networkId: xdr.Hash.fromXDR(Buffer.from(networkPassphrase, "utf8")),
            sourceAccount: keypair.publicKey(),
            salt: xdr.Uint64.fromString("0"),
          }),
        ]),
        args: [nativeToScVal("release_escrow"), nativeToScVal(escrowId)],
      }),
    )
    .setTimeout(30)
    .build();

  const signedTx = transaction.sign(keypair);
  const response = await server.submitTransaction(signedTx);

  return response.id;
}
```

#### Error Handling

```typescript
async function createEscrowWithErrorHandling(
  mentorAddress: string,
  learnerAddress: string,
  amount: bigint,
  sessionId: string,
  tokenAddress: string,
  sessionEndTime: number,
): Promise<{ success: boolean; escrowId?: string; error?: string }> {
  try {
    // Validate inputs
    if (amount <= 0n) {
      return {
        success: false,
        error: "InvalidAmount: Amount must be positive",
      };
    }

    if (sessionEndTime <= Math.floor(Date.now() / 1000)) {
      return {
        success: false,
        error: "InvalidTime: Session end time must be in future",
      };
    }

    const escrowId = await createEscrow(
      mentorAddress,
      learnerAddress,
      amount,
      sessionId,
      tokenAddress,
      sessionEndTime,
    );

    return { success: true, escrowId };
  } catch (error: any) {
    const errorCode = error.response?.extras?.result_codes?.operations?.[0];

    switch (errorCode) {
      case "op_inner":
        return { success: false, error: "Unauthorized: Check permissions" };
      case "op_no_destination":
        return { success: false, error: "NotFound: Invalid address" };
      default:
        return { success: false, error: `Error: ${error.message}` };
    }
  }
}
```

### Python Integration

#### Setup

```python
from stellar_sdk import Keypair, Network, TransactionBuilder, Server
from stellar_sdk.operation import InvokeHostFunction
from stellar_sdk.xdr import SCVal
import os

# Initialize Soroban client
rpc_url = 'https://soroban-testnet.stellar.org:443'
network_passphrase = Network.TESTNET_NETWORK_PASSPHRASE
server = Server(rpc_url)

contract_id = 'CAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAABSC4'
```

#### Create Escrow

```python
def create_escrow(
    mentor_address: str,
    learner_address: str,
    amount: int,
    session_id: str,
    token_address: str,
    session_end_time: int
) -> str:
    keypair = Keypair.random()
    account = server.load_account(keypair.public_key)

    transaction = (
        TransactionBuilder(
            account,
            base_fee=100,
            network_passphrase=network_passphrase
        )
        .add_text_memo('Create Escrow')
        .set_timeout(30)
        .build()
    )

    signed_tx = transaction.sign(keypair)
    response = server.submit_transaction(signed_tx)

    return response['id']
```

#### Error Handling

```python
def create_escrow_with_error_handling(
    mentor_address: str,
    learner_address: str,
    amount: int,
    session_id: str,
    token_address: str,
    session_end_time: int
) -> dict:
    try:
        # Validate inputs
        if amount <= 0:
            return {'success': False, 'error': 'InvalidAmount: Amount must be positive'}

        if session_end_time <= int(time.time()):
            return {'success': False, 'error': 'InvalidTime: Session end time must be in future'}

        escrow_id = create_escrow(
            mentor_address,
            learner_address,
            amount,
            session_id,
            token_address,
            session_end_time
        )

        return {'success': True, 'escrow_id': escrow_id}

    except Exception as error:
        error_msg = str(error)

        if 'Unauthorized' in error_msg:
            return {'success': False, 'error': 'Unauthorized: Check permissions'}
        elif 'NotFound' in error_msg:
            return {'success': False, 'error': 'NotFound: Invalid address'}
        else:
            return {'success': False, 'error': f'Error: {error_msg}'}
```

### Rust Integration

#### Setup

```rust
use soroban_sdk::{Address, Env, Symbol};
use stellar_sdk::Client;

pub struct EscrowClient {
    contract_id: Address,
    env: Env,
}

impl EscrowClient {
    pub fn new(contract_id: Address, env: Env) -> Self {
        EscrowClient { contract_id, env }
    }
}
```

#### Create Escrow

```rust
impl EscrowClient {
    pub fn create_escrow(
        &self,
        mentor: Address,
        learner: Address,
        amount: i128,
        session_id: Symbol,
        token_address: Address,
        session_end_time: u64,
    ) -> Result<u64, String> {
        // Call contract function
        let escrow_id: u64 = self.env
            .invoke_contract(
                &self.contract_id,
                &Symbol::new(&self.env, "create_escrow"),
                (&mentor, &learner, &amount, &session_id, &token_address, &session_end_time),
            );

        Ok(escrow_id)
    }
}
```

---

## Authentication & Authorization

### Signature-Based Authentication

All contract operations require cryptographic signatures:

```typescript
// 1. Create transaction
const transaction = new TransactionBuilder(account, {
  fee: "100",
  networkPassphrase,
})
  .addOperation(operation)
  .setTimeout(30)
  .build();

// 2. Sign with private key
const signedTx = transaction.sign(keypair);

// 3. Submit to network
const response = await server.submitTransaction(signedTx);
```

### Role-Based Access Control

Different operations require different roles:

| Operation       | Required Role     | Example                           |
| --------------- | ----------------- | --------------------------------- |
| create_escrow   | Mentor            | Mentor creates escrow for session |
| release_escrow  | Mentor or Admin   | Release funds after session       |
| open_dispute    | Mentor or Learner | Either party can open dispute     |
| resolve_dispute | Admin             | Only admin resolves disputes      |
| verify_mentor   | Admin             | Only admin verifies mentors       |

### Authorization Checks

```typescript
// Check if caller is authorized
async function checkAuthorization(
  operation: string,
  callerAddress: string,
): Promise<boolean> {
  const roles = await getRoles(callerAddress);

  switch (operation) {
    case "create_escrow":
      return roles.includes("mentor");
    case "release_escrow":
      return roles.includes("mentor") || roles.includes("admin");
    case "resolve_dispute":
      return roles.includes("admin");
    default:
      return false;
  }
}
```

---

## Error Handling

### Error Response Format

```typescript
interface ErrorResponse {
  code: number;
  message: string;
  details?: string;
}

// Example error responses
const errors = {
  InvalidAmount: { code: 5, message: "Amount is invalid" },
  Unauthorized: { code: 3, message: "Caller not authorized" },
  NotFound: { code: 4, message: "Resource not found" },
  InvalidState: { code: 6, message: "Invalid state for operation" },
};
```

### Error Handling Pattern

```typescript
async function executeWithErrorHandling<T>(
  operation: () => Promise<T>,
  operationName: string,
): Promise<{ success: boolean; data?: T; error?: string }> {
  try {
    const data = await operation();
    return { success: true, data };
  } catch (error: any) {
    const errorCode = error.response?.extras?.result_codes?.operations?.[0];
    const errorMessage = mapErrorCode(errorCode);

    console.error(`${operationName} failed: ${errorMessage}`);

    return {
      success: false,
      error: errorMessage,
    };
  }
}

function mapErrorCode(code: string): string {
  const errorMap: Record<string, string> = {
    op_inner: "Unauthorized: Check permissions",
    op_no_destination: "NotFound: Invalid address",
    op_underfunded: "InvalidAmount: Insufficient balance",
    op_line_full: "InvalidState: Account limit reached",
  };

  return errorMap[code] || "Unknown error occurred";
}
```

---

## Best Practices

### 1. Input Validation

Always validate inputs before sending to contract:

```typescript
function validateEscrowInput(
  amount: bigint,
  sessionEndTime: number,
): { valid: boolean; error?: string } {
  // Check amount
  if (amount <= 0n) {
    return { valid: false, error: "Amount must be positive" };
  }

  if (amount > BigInt("9223372036854775807")) {
    return { valid: false, error: "Amount exceeds maximum" };
  }

  // Check time
  const now = Math.floor(Date.now() / 1000);
  if (sessionEndTime <= now) {
    return { valid: false, error: "Session end time must be in future" };
  }

  if (sessionEndTime > now + 365 * 24 * 60 * 60) {
    return { valid: false, error: "Session end time too far in future" };
  }

  return { valid: true };
}
```

### 2. Transaction Confirmation

Always wait for transaction confirmation:

```typescript
async function submitAndWait(
  signedTx: Transaction,
  maxWaitTime: number = 30000,
): Promise<TransactionResponse> {
  const response = await server.submitTransaction(signedTx);

  // Poll for confirmation
  const startTime = Date.now();
  while (Date.now() - startTime < maxWaitTime) {
    try {
      const tx = await server.transactions().transaction(response.id).call();
      if (tx.successful) {
        return tx;
      }
    } catch (error) {
      // Transaction not yet confirmed
    }

    await new Promise((resolve) => setTimeout(resolve, 1000));
  }

  throw new Error("Transaction confirmation timeout");
}
```

### 3. Event Monitoring

Monitor contract events for state changes:

```typescript
async function monitorEscrowEvents(
  escrowId: bigint,
  callback: (event: EscrowEvent) => void,
): Promise<void> {
  const eventStream = server
    .events()
    .forContract(contractId)
    .cursor("now")
    .stream({
      onmessage: (event) => {
        const escrowEvent = parseEscrowEvent(event);
        if (escrowEvent.escrowId === escrowId) {
          callback(escrowEvent);
        }
      },
      onerror: (error) => {
        console.error("Event stream error:", error);
      },
    });

  return eventStream;
}
```

### 4. Rate Limiting

Implement rate limiting to avoid hitting contract limits:

```typescript
class RateLimiter {
  private requests: number[] = [];
  private maxRequests: number;
  private windowMs: number;

  constructor(maxRequests: number = 10, windowMs: number = 60000) {
    this.maxRequests = maxRequests;
    this.windowMs = windowMs;
  }

  async checkLimit(): Promise<boolean> {
    const now = Date.now();
    this.requests = this.requests.filter((t) => now - t < this.windowMs);

    if (this.requests.length >= this.maxRequests) {
      return false;
    }

    this.requests.push(now);
    return true;
  }
}
```

### 5. Retry Logic

Implement exponential backoff for retries:

```typescript
async function executeWithRetry<T>(
  operation: () => Promise<T>,
  maxRetries: number = 3,
  baseDelay: number = 1000,
): Promise<T> {
  for (let attempt = 0; attempt < maxRetries; attempt++) {
    try {
      return await operation();
    } catch (error) {
      if (attempt === maxRetries - 1) {
        throw error;
      }

      const delay = baseDelay * Math.pow(2, attempt);
      await new Promise((resolve) => setTimeout(resolve, delay));
    }
  }

  throw new Error("Max retries exceeded");
}
```

---

## Sample Code

### Complete Escrow Workflow

```typescript
// 1. Create escrow
const escrowId = await createEscrow(
  mentorAddress,
  learnerAddress,
  BigInt("1000000000"), // 1 billion stroops
  "session-123",
  tokenAddress,
  Math.floor(Date.now() / 1000) + 86400, // 24 hours from now
);

// 2. Monitor escrow status
const escrow = await getEscrow(escrowId);
console.log(`Escrow created: ${escrow.id}, Status: ${escrow.status}`);

// 3. Wait for session to end
await new Promise((resolve) =>
  setTimeout(
    resolve,
    (escrow.session_end_time - Math.floor(Date.now() / 1000)) * 1000,
  ),
);

// 4. Release funds
await releaseEscrow(escrowId);

// 5. Verify release
const updatedEscrow = await getEscrow(escrowId);
console.log(`Escrow released: Status: ${updatedEscrow.status}`);
```

---

## Support

For integration assistance:

- Review ERRORS.md for error codes
- Check EVENTS.md for event documentation
- See TROUBLESHOOTING.md for common issues
- Contact support@mentorminds.io for help

# Storage Strategy Guide

This document outlines the storage strategy for all MentorsMind contracts on Soroban, optimizing for ledger entry fees and performance.

## Storage Types Overview

Soroban provides three storage types:

1. **Persistent Storage**: Long-term data that survives ledger expiry
2. **Instance Storage**: Contract instance data, shared across all functions
3. **Temporary Storage**: Session-scoped data that auto-expires with TTL

## General Guidelines

| Storage Type | Use Case | TTL | Cost |
|--------------|----------|-----|------|
| Persistent | User data, historical records, critical state | Manual extension required | Highest |
| Instance | Config, admin address, frequently-read settings | Contract lifetime | Medium |
| Temporary | Session data, rate limiting, cache | Auto-expires | Lowest |

## Contract-Specific Strategies

### Credit Score Contract

| Key | Storage Type | Rationale |
|-----|--------------|-----------|
| Admin | Persistent | Critical config, needs to survive |
| EscrowContract | Persistent | External dependency reference |
| StakingContract | Persistent | External dependency reference |
| UserScore | Persistent | Long-term user reputation |
| UserBreakdown | Persistent | Long-term user data |
| LastUpdate | Temporary | Rate limiting per day, auto-expires |

**TTL Strategy**: Extend persistent entries on read operations.

### Streak Rewards Contract

| Key | Storage Type | Rationale |
|-----|--------------|-----------|
| Admin | Instance | Frequently read, contract config |
| MntToken | Instance | Frequently read, contract config |
| UserStreak | Persistent | Long-term user engagement data |
| Leaderboard | Persistent | Aggregate data, updated frequently |

**TTL Strategy**: Extend persistent entries on write operations.

### Staking Contract

| Key | Storage Type | Rationale |
|-----|--------------|-----------|
| Admin | Instance | Frequently read, contract config |
| MNTToken | Instance | Frequently read, contract config |
| Stake | Persistent | Long-term staking records |

**TTL Strategy**: Extend persistent entries on stake/unstake operations.

### Governance Contract

| Key | Storage Type | Rationale |
|-----|--------------|-----------|
| Admin | Instance | Frequently read, contract config |
| TOKEN | Instance | Frequently read, contract config |
| PROPOSAL_COUNT | Instance | Frequently read counter |
| VOTING_PERIOD_SECS | Instance | Config, rarely changes |
| QUORUM_BPS | Instance | Config, rarely changes |
| Proposal | Persistent | Historical governance records |
| Vote | Persistent | Voting records |
| VoteWeight | Persistent | Voting power records |

**TTL Strategy**: Extend persistent entries on vote/execute operations.

### Escrow Factory Contract

| Key | Storage Type | Rationale |
|-----|--------------|-----------|
| ADMIN | Persistent | Critical, needs manual TTL extension |
| IMPLEMENTATION | Persistent | Critical, needs manual TTL extension |
| ESCROW_MAPPING | Persistent | Session to address mapping |
| ESCROW_LIST | Persistent | Historical records |
| ESCROW_COUNT | Persistent | Counter for pagination |

**TTL Strategy**: Already implements TTL extensions on all operations.

### Upgrade Registry Contract

| Key | Storage Type | Rationale |
|-----|--------------|-----------|
| Admin | Instance | Contract config |
| UpgradeHistory | Persistent | Historical upgrade records |
| LatestVersion | Persistent | Current version tracking |
| Subscribers | Persistent | Subscription list |

**TTL Strategy**: Extend persistent entries on subscribe/register operations.

### Rate Limiter Contract

| Key | Storage Type | Rationale |
|-----|--------------|-----------|
| Admin | Instance | Contract config |
| CallCount | Temporary | Auto-expires with window |
| WindowStart | Temporary | Auto-expires with window |
| Whitelist | Instance | Frequently checked, admin config |

**TTL Strategy**: Temporary storage handles auto-expiry. No manual extension needed.

### Performance Bond Contract

| Key | Storage Type | Rationale |
|-----|--------------|-----------|
| Admin | Instance | Contract config |
| MntToken | Instance | Frequently read |
| InsurancePool | Instance | Config, rarely changes |
| Bond | Persistent | Long-term mentor bond records |

**TTL Strategy**: Extend persistent entries on bond operations.

## TTL Extension Best Practices

### When to Extend TTL

1. **Read Operations**: Extend entries that are frequently accessed
2. **Write Operations**: Extend entries being modified
3. **Batch Operations**: Extend all related entries together

### TTL Values

```rust
const THRESHOLD: u32 = 500_000;  // Extend when below this
const BUMP: u32 = 1_000_000;     // Extend by this amount
```

### Example Implementation

```rust
// On read
let data: MyType = env.storage().persistent().get(&key).unwrap();
env.storage().persistent().extend_ttl(&key, THRESHOLD, BUMP);

// On write
env.storage().persistent().set(&key, &data);
env.storage().persistent().extend_ttl(&key, THRESHOLD, BUMP);
```

## Migration Guide

When optimizing existing contracts:

1. **Audit Current Storage**: List all storage keys and their current type
2. **Identify Candidates**: Find keys that should use different storage
3. **Check Dependencies**: Ensure other contracts don't depend on storage type
4. **Test Migration**: Verify data integrity after migration
5. **Update Documentation**: Reflect changes in this guide

## Performance Metrics

Target: **20% reduction** in ledger entry fees after optimization.

### Measurement

1. Count total persistent entries before optimization
2. Count persistent entries after optimization
3. Calculate fee reduction: `(before - after) / before * 100`

### Expected Savings

| Contract | Persistent Before | Persistent After | Savings |
|----------|------------------|------------------|---------|
| Credit Score | 5 | 4 | 20% |
| Staking | 3 | 1 | 67% |
| Governance | 6 | 3 | 50% |
| Rate Limiter | 4 | 0 | 100% |

## Security Considerations

1. **Instance Storage**: Visible to all contract functions - don't store secrets
2. **Temporary Storage**: Auto-expires - ensure data loss is acceptable
3. **Persistent Storage**: Most expensive - use sparingly for critical data only

## Monitoring

Track storage usage with events:

```rust
env.events().publish(
    (symbol_short!("storage"), symbol_short!("updated")),
    (key_type, storage_type),
);
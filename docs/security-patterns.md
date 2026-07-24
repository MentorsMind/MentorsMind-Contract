# Security Patterns

This branch introduces the security modules requested in the linked issues:

- `contracts/rbac`: centralized role membership for `SUPER_ADMIN`, `ESCROW_ADMIN`, `DISPUTE_RESOLVER`, `ORACLE_ADMIN`, `ORACLE_FEEDER`, `KYC_OPERATOR`, and `SESSION_ORACLE`
- `contracts/dispute_evidence`: bounded evidence submission for disputed escrows with a 48-hour submission window and on-chain retrieval
- `contracts/shared/src/reentrancy_guard.rs`: reusable instance-storage lock for external-call paths
- `contracts/session_oracle`: dual-confirmation session completion tracking with dispute and expiry handling

The existing `escrow` contract in this repository already contains pre-existing merge damage. This branch keeps the new security modules isolated and integrates them into the cleaner contract surfaces without attempting a full escrow rewrite.

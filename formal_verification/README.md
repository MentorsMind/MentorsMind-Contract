# Economic Formal Verification

This model is the executable specification for the economic invariants shared by the contracts.

## Properties

- **Fund conservation:** `prior + inflows = current + outflows + fees`.
- **Reward fairness:** allocated rewards equal the declared reward within one base unit of integer rounding.
- **Temporal safety:** timestamps are monotonic and observations cannot exceed the configured age bound.
- **Incentive compatibility:** an exploitive strategy is not preferable after its expected detection penalty.
- **Market integrity:** secondary venues must have quorum and fewer than half may deviate from the liquidity-weighted median beyond the configured bound.

The Soroban implementation is in `contracts/shared/src/economic_verification.rs`. Checks are persisted and failed checks emit an `economic/violation` event for indexers and alerting systems.

Run the bounded proofs with Kani from this directory:

```text
cargo kani --output-format terse
```

The proofs cover arithmetic closure and transition predicates. They do not claim that arbitrary external contracts are honest; cross-contract callers must supply observed balances and venue observations to the runtime predicates.

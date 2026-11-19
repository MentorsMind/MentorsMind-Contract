# Governance Contract

An on-chain governance contract for the MentorMinds platform (crate `mentorminds-governance`), enabling token-weighted proposals, time-weighted voting, timelocked execution, arbitration/appeals, and market-control protection on Stellar/Soroban.

## Overview

- Proposals carry a typed `ProposalAction` (fee update, auto-release update, asset add, admin update, allowlisted cross-contract call, or emergency-rollback approval).
- Voting power comes from snapshot/delegation contracts and is time-weighted: 80% in the first third of the voting period, 100% mid, 110% late.
- Default quorum is 10% (1000 bps); custom (rollback) proposals require 30% (3000 bps); `ExecuteCall` proposals enforce a 7-day timelock delay via an external timelock contract.
- Anti-griefing: proposal deposits, minimum proposer balance, global and per-address active proposal caps, a proposer holding-period check against the staking contract, and a 7-day cancel cooldown with multi-sig escalation after 3 cancels in 30 days.
- Arbitration registry with dispute-independence checks, appeals, and round-robin arbitrator selection for disputes.
- Governance-level market protection (concentration regulation, competition audits, pricing-coordination detection) plus validator accountability, consensus emergency mode, and voter risk flags.

## Entry Points

### Initialization / Admin

- `fn initialize(admin: Address, mnt_token: Address, snapshot_contract: Address, delegation_contract: Address, voting_period_secs: Option<u64>, quorum_bps: Option<u32>, proposal_deposit: Option<i128>, min_proposer_balance: Option<i128>, max_active_proposals: Option<u32>)` — one-time setup; installs the storage namespace and writes config defaults (7-day voting period, 10% quorum, 3 max active proposals).
- `fn propose_admin_change(current_admin: Address, new_admin: Address) -> Result<(), Error>` — propose a new admin; effective after a 48-hour timelock.
- `fn accept_admin_change(new_admin: Address) -> Result<(), Error>` — new admin accepts once the timelock has elapsed.
- `fn cancel_admin_change(multisig: Address) -> Result<(), Error>` — multisig cancels a pending admin change.
- `fn set_timelock(timelock: Address)` — set the timelock contract address (admin auth).
- `fn add_allowed_call(admin: Address, target: Address, function: Symbol)` — allowlist a `(target, function)` pair for `ExecuteCall` proposals (admin only).
- `fn set_templates_contract(templates_contract: Address)` — set the templates contract used to pin allowlisted call hashes (admin auth).
- `fn set_staking_contract(admin: Address, staking_contract: Address)` — set the staking contract queried for the proposer's `staked_at` holding-period check (admin only).
- `fn set_multisig_admin(admin: Address, multisig_admin: Address)` — set the MultisigAdmin contract used for cancel escalation (admin only).

### Proposals

- `fn create_proposal(proposer: Address, title: Bytes, description_hash: BytesN<32>, action: ProposalAction) -> u32` — create a proposal; enforces allowlisting, holding period, and active-proposal caps; escrows the proposal deposit.
- `fn propose_rollback_review(proposer: Address, title: Bytes, description_hash: BytesN<32>, escrow_contract: Address, escrow_rollback_id: u32) -> u32` — open a governance review for an escrow emergency rollback and link it to the escrow contract.
- `fn cancel_proposal(proposal_id: u32, multisig_action_id: Option<u32>)` — admin cancels a non-executed proposal; subject to 7-day per-action cooldown and multi-sig escalation after 3 cancels in 30 days.

### Voting

- `fn vote(voter: Address, proposal_id: u32, support: bool)` — cast a single vote using delegation-adjusted snapshot power, time-window weight, and coordinated-timing/manipulation checks.
- `fn get_voting_window(proposal_id: u32) -> VotingWindow` — returns the current voting window (0/1/2) and weight bps (8000/10000/11000).
- `fn estimate_governance_vote_cost(proposal_id: u32, voter: Address) -> GasEstimate` — heuristic gas estimate (instructions, storage ops, cross-contract calls) for a vote.

### Execution

- `fn execute_proposal(proposal_id: u32)` — finalize after voting ends; applies the action or queues it into the timelock for `ExecuteCall` variants.
- `fn execute_queued_proposal(proposal_id: u32)` — execute a queued proposal after the timelock delay; callable only by the timelock contract.

### Arbitration

- `fn register_arbitrator(admin: Address, arbitrator: Address) -> Result<(), Error>` — register an arbitrator (admin only); returns `ArbitratorConflict` if dispute independence fails.
- `fn unregister_arbitrator(admin: Address, arbitrator: Address)` — deactivate and deregister an arbitrator (admin only).
- `fn migrate_arbitrators(admin: Address)` — migrate a legacy arbitrator list into the indexed registry (admin only).
- `fn set_arbitration_compensation(admin: Address, amount: i128)` — set arbitrator compensation amount (admin auth).
- `fn select_arbitrator(dispute_id: u64) -> Address` — deterministically select the next active arbitrator (round-robin by dispute id).
- `fn submit_appeal(appellant: Address, proposal_id: u32, reason: String)` — submit an appeal for a resolved proposal.
- `fn resolve_appeal(arbitrator: Address, proposal_id: u32)` — mark an appeal resolved and increment the arbitrator's case count (active arbitrators only).

### Market Protection / Safety

- `fn regulate_market_concentration(admin: Address, network_ids: Vec<Symbol>, network_session_counts: Vec<u32>, total_sessions: u32, independent_mentor_count: u32, total_active_mentors: u32) -> DecentralizationMonitoring` — compute HHI-based decentralization and a combined intervention decision (admin only).
- `fn enforce_competition_policies(admin: Address, barrier_signal_count: u32, price_timestamps: Vec<u64>, price_changes_bps: Vec<u32>) -> CompetitionAuditRecord` — update barrier tracking, detect pricing coordination, and run a competition audit (admin only).
- `fn restore_mkt_comp_balance(admin: Address)` — clear the active market-control intervention once the restoration cooldown has elapsed (admin only).
- `fn register_governance_validator(admin: Address, validator: Address)` — register a validator for accountability tracking (admin only).
- `fn assess_validator_alignment(validator: Address) -> IncentiveAlignmentScore` — score a validator's incentive alignment.
- `fn activate_governance_emergency(admin: Address)` / `fn deactivate_governance_emergency(admin: Address)` — toggle governance consensus emergency mode (admin only).
- `fn evaluate_vote_risk(voter: Address, proposal_id: u32, support: bool) -> TransactionIntent` — pre-vote risk check; flags blocked/critical accounts.
- `fn clear_voter_flag(admin: Address, voter: Address)` — clear a voter's suspicious-activity flag (admin only).

### Views

- `fn get_admin() -> Result<Address, Error>`; `fn get_pending_admin_change() -> Option<PendingAdminChange>`
- `fn is_call_allowed(target: Address, function: Symbol) -> bool`; `fn get_staking_contract() -> Option<Address>`
- `fn get_proposal(id: u32) -> Proposal`; `fn get_active_proposal_count() -> u32`; `fn get_proposals_by_status(status: ProposalStatus, offset: u32, limit: u32) -> Vec<u32>`
- `fn get_vote(id: u32, voter: Address) -> bool`; `fn get_vote_weight(id: u32, voter: Address) -> i128`
- `fn get_arbitrator(arbitrator: Address) -> ArbitratorRecord`; `fn get_arbitrator_count() -> u32`; `fn list_arbitrators_page(offset: u32, limit: u32) -> Vec<Address>`; `fn get_arbitration_compensation() -> i128`
- `fn get_appeal(proposal_id: u32) -> AppealRecord`
- `fn get_gov_market_protection() -> MarketProtectionRecord`; `fn get_competition_audit() -> CompetitionAuditRecord`; `fn get_gov_decentralization() -> DecentralizationMonitoring`; `fn get_gov_market_fairness() -> MarketFairness`
- `fn get_governance_validator(validator: Address) -> Option<ValidatorRecord>`; `fn is_governance_validator_ejected(validator: Address) -> bool`; `fn is_governance_emergency_active() -> bool`; `fn is_voter_flagged(voter: Address) -> bool`

## Storage

Instance-storage config symbols: `ADMIN`, `TOKEN`, `SNAPSHOT`, `PROP_CNT`, `VOT_PER`, `QRM_BPS`, `FEE_BPS`, `AUTO_REL`, `TMPLATES`, `PROP_DEP`, `MIN_P_BAL`, `MAX_ACT_P`, `TRES_BAL` (namespace scope `mm_gov`).

`DataKey` variants (persistent unless noted):

| Variant | Purpose |
|---|---|
| `NamespaceRoot` | Contract-isolated storage namespace root |
| `Proposal(u32)` | Proposal record by id |
| `ActiveProposalCount` / `PerAddressActiveProposalCount(Address)` | Global / per-address active proposal caps |
| `ProposalDeposit(u32)` | Escrowed deposit per proposal |
| `Vote(u32, Address)` / `VoteWeight(u32, Address)` / `VoteWeightMultiplier(u32, Address)` | Vote flag, weight, per-voter multiplier |
| `VoteTimestamps(u32)` | Rolling vote-time history for coordinated-timing detection |
| `WeightedVotesFor(u32)` / `WeightedVotesAgainst(u32)` | Time-weighted tallies for quorum |
| `ApprovedAsset(Address)` | Approved asset registry |
| `Timelock` | Timelock contract address |
| `CustomProposal(u32)` | Marks rollback-review proposals (30% quorum) |
| `AllowedCall(Address, Symbol)` | ExecuteCall allowlist |
| `PendingAdmin` | Pending two-step admin change |
| `DelegationContract` / `Delegate(Address)` | Delegation contract address / per-address delegate |
| `CancelCooldown(Address, ProposalAction)` / `CancelCount(Address)` / `CancelTimestamps(Address)` | Cancel anti-griefing state |
| `MultisigAdmin` | MultisigAdmin contract for cancel escalation |
| `Arbitrator(Address)` / `ArbitratorAt(u32)` / `ArbitratorCount` / `ArbitratorIndex(Address)` / `ArbitratorList` | Arbitrator registry |
| `ArbitratorCompensation` (instance) | Arbitrator compensation amount |
| `ArbitratorDisputeHistory(Address)` / `ArbitratorIndependenceFlag(Address)` | Dispute independence audit |
| `Appeal(u32)` | Appeal record by proposal id |
| `GovDecentralizationRecord` / `GovCompetitionRecord` / `GovMarketFairnessRecord` / `GovMarketProtectionRecord` | Cached market-protection state |
| `GovMarketControlActive` | Active intervention flag |
| `GovNetworkSessionCount(Symbol)` / `GovSegmentTotalSessions` / `GovIndependentMentorCount` / `GovTotalActiveMentors` / `GovBarrierSignalCount` | Market-protection inputs |
| `GovCompetitionAuditRecord` | Latest competition audit |
| `GovValidatorRecord(Address)` | Governance validator registration |
| `GovConsensusEmergency` | Emergency consensus flag |
| `GovVoterFlag(Address)` | Suspicious-voter flag |
| `StakingContract` | Staking contract for holding-period check |
| `ProposalsByStatus(ProposalStatus, u32)` / `ProposalStatusCount(ProposalStatus)` | Status-filtered proposal index |

## Events

Published via `shared::events` (`emit_governance_event`): `prop_created`, `vote_cast`, `prop_passed`, `prop_failed`, `prop_queued`, `prop_executed`, `prop_cancelled`, `prop_cxl_cd` (cancel with cooldown), `timelock_set`, `call_allowed`, `arb_registered`, `arb_unreg`, `appeal_sub`, `appeal_res`.

Published directly via `env.events().publish`:

- `(admin, proposed)` — admin change proposed.
- `(gov, coord)` — coordinated vote timing detected.
- `VoteManipulationAlert` — vote manipulation detected for a proposal.
- `(govmkt, intervention)` — market-control intervention triggered.
- `(govfair, coord_detected)` — pricing coordination detected.
- `(govcomp, barrier_found)` — competition barrier signals found.
- `(govmkt, restored)` — market competitive balance restored.
- `(govval, register)` — governance validator registered.
- `(govval, emer_on)` / `(govval, emer_off)` — consensus emergency toggled.
- `(govtx, flag_clr)` — voter suspicious-activity flag cleared.

## Cross-Contract Dependencies

| Contract | Address stored as | Functions called |
|---|---|---|
| MNT token | `TOKEN` | `balance`, `total_supply` |
| Snapshot | `SNAPSHOT` | `record_snapshot`, `get_total_supply_at`, `get_snapshot_balance` |
| Delegation | `DelegationContract` | `get_delegated_power_at_snapshot` |
| Templates | `TMPLATES` | `get_template_hash` |
| Staking | `StakingContract` | `get_stake` (proposer `staked_at` for holding period) |
| Timelock | `Timelock` | `schedule`; timelock auth required by `execute_queued_proposal` |
| MultisigAdmin | `MultisigAdmin` | `get_proposal`, `get_threshold`, `execute_action` (cancel escalation) |
| Escrow | passed to `propose_rollback_review` | `link_governance_rollback_review` |

## Testing

```bash
cargo test -p mentorminds-governance
```

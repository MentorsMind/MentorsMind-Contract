# Edit plan: enforce ProposalStatus StateMachine transitions

## Information gathered
- Located the governance contract at `contracts/governance/src/lib.rs`.
- The contract defines `impl StateMachine for ProposalStatus` with explicit valid transitions.
- In `execute_proposal()` the code directly mutates `proposal.status` in sequence:
  - sets `Failed` via `proposal.status = ProposalStatus::Failed`
  - later sets `Passed` via `proposal.status = ProposalStatus::Passed`
  - then sets `Queued` via `proposal.status = ProposalStatus::Queued`
- In `execute_queued_proposal()` it directly sets `proposal.status = ProposalStatus::Executed`.
- The helper/validation `ProposalStatus::is_valid_transition(...)` is currently not invoked anywhere in these flows, making the state machine dead code.

## Plan
### 1) Add a transition enforcement helper
- Add a private helper function in `impl GovernanceContract`:
  - `fn transition_proposal_status(env: &Env, proposal: &mut Proposal, to: ProposalStatus)`
  - It checks `ProposalStatus::is_valid_transition(env, &proposal.status, &to)`.
  - If invalid, it panics with a clear message.
  - On success, sets `proposal.status = to`.

### 2) Update `execute_proposal`
- Replace direct assignments with enforced transitions:
  - When quorum fails: use `transition_proposal_status(&env, &mut proposal, ProposalStatus::Failed)`.
  - When quorum passes: transition from `Active -> Passed`, then from `Passed -> Queued`.

### 3) Update `execute_queued_proposal`
- Replace direct assignment `proposal.status = ProposalStatus::Executed` with enforced transition `Queued -> Executed`.

### 4) (Optional but recommended) Update `cancel_proposal`
- If `cancel_proposal` currently allows transitions that are not covered by the state machine, either:
  - enforce cancellation transitions via the helper, or
  - update the state machine to reflect allowed cancellation.

### 5) Testing
- Run `cargo test` (or `cargo test -p governance` if workspace supports it) to ensure unit tests pass.
- Ensure the existing tests for full lifecycle/quorum failure still pass.

## Dependent files to edit
- `contracts/governance/src/lib.rs`

## Followup steps
- Run `cargo test` from repository root.
- If any compilation errors occur due to helper signature/lifetimes, fix them and re-run tests.

<ask_followup_question>
Confirm I should proceed to implement the helper + update `execute_proposal` and `execute_queued_proposal` to enforce StateMachine transitions (and also check/enforce `cancel_proposal` if needed). I will then run `cargo test`.
</ask_followup_question>


# Contributing to MentorMinds Contracts

Thank you for contributing to the MentorMinds Soroban contract suite. This
guide covers local setup, the development workflow, adding contracts, and
opening a pull request.

## Prerequisites

Install the following tools before starting:

- Rust 1.70 or newer, with `rustup`.
- The Soroban CLI, installed with `cargo install --locked soroban-cli`.
- Docker and Docker Compose, for the local Stellar environment.
- Node.js 18 or newer and pnpm, for the repository helper scripts.

Install the Rust target used by the contract build and confirm the tools are
available:

```bash
rustup target add wasm32v1-none
rustc --version
soroban --version
docker --version
pnpm --version
```

## Development Setup

Clone the repository and install JavaScript dependencies:

```bash
git clone https://github.com/MentorsMind/MentorsMind-Contract.git
cd MentorsMind-Contract
pnpm install
```

Start the local Stellar environment when you need to exercise deployed
contracts:

```bash
pnpm run local:start
pnpm run local:status
```

The setup script starts the Docker services, creates local test accounts, and
writes generated deployment data under `deployed/`. Seed data is optional:

```bash
pnpm run local:seed
```

Stop or reset the environment with:

```bash
pnpm run local:stop
pnpm run local:reset
```

## Build and Test

Build the workspace contracts for Soroban:

```bash
cargo build --target wasm32v1-none
```

For optimized release WASM, use the benchmark build or the existing package
script where applicable. Run the complete Rust test suite from the workspace
root:

```bash
cargo test --workspace
```

Before opening a PR, also format and lint Rust changes:

```bash
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

The test strategy and test locations are described in
[`docs/TESTING.md`](docs/TESTING.md).

## Running Benchmarks

Benchmarks measure CPU instructions, memory, storage operations, and WASM size
for selected contract entry points. From the repository root:

```bash
cargo build --target wasm32v1-none --release \
  -p mentorminds-escrow \
  -p mentorminds-staking \
  -p mentorminds-governance \
  -p mentorminds-timelock \
  -p mentorminds-upgrade-registry \
  -p mentorminds-dispute-evidence
cargo run -p mentorminds-benchmarks
```

Reports are written to `benchmarks/results/`. Read
[`benchmarks/README.md`](benchmarks/README.md) before updating a baseline;
baseline changes should be intentional and explained in the PR.

## Adding a New Contract

New contracts should follow the existing workspace structure and security
patterns. The minimum required components are:

- [ ] Add a crate under `contracts/<contract_name>/` with a `Cargo.toml` and
      `src/lib.rs`.
- [ ] Add the crate to the workspace members in the root `Cargo.toml`.
- [ ] Define typed storage keys with a `DataKey` enum and include a
      `DataKey::NamespaceRoot` entry. Follow the
      [DataKey::NamespaceRoot guide (Issue #53)](https://github.com/MentorsMind/MentorsMind-Contract/issues/53)
      and the [storage pattern documentation](docs/storage-guide.md).
- [ ] Define a `#[contracterror]` error enum for all expected contract
      failures.
- [ ] Add an `initialize` entrypoint with an initialization guard so it cannot
      be run twice.
- [ ] Choose the correct Soroban storage tier and bump TTLs for persistent or
      instance data according to the [storage pattern documentation](docs/storage-guide.md).
- [ ] Emit events for state-changing operations. Document the event topics and
      payloads in [`docs/events.md`](docs/events.md) (Issue #21).
- [ ] Add unit and integration tests for successful and failing paths.
- [ ] Add a benchmark suite entry when the contract has measurable critical
      entry points, following [`benchmarks/README.md`](benchmarks/README.md).

Keep storage keys append-only where possible. Changes to storage layout,
authorization, events, or public interfaces need regression tests and a clear
upgrade or migration explanation.

## Contribution Workflow

1. Create a focused branch from the default branch:

   ```bash
   git checkout -b feature/short-description
   ```

2. Make the smallest coherent change and add or update tests and docs.
3. Run the build, test, formatting, lint, and relevant benchmark commands.
4. Review the diff, generated files, and benchmark output before committing.
5. Push the branch to your fork and open a pull request against the default
   branch.

## Pull Request Checklist

Use this checklist in the PR description:

- [ ] The PR explains the problem, the implementation, and any security or
      storage implications.
- [ ] Tests were added or updated for the changed behavior.
- [ ] `cargo build --target wasm32v1-none` passes.
- [ ] `cargo test --workspace` passes.
- [ ] `cargo fmt --all -- --check` and the relevant Clippy checks pass.
- [ ] Benchmarks were run when contract performance or a benchmarked entry
      point changed; any baseline update is justified.
- [ ] Documentation and event schemas were updated where needed.
- [ ] No generated deployment artifacts, secrets, or unrelated formatting
      changes are included.
- [ ] Related issues are linked, including the relevant security or storage
      issue when applicable.

For questions or larger design changes, open an issue before implementation so
the contract interface and storage implications can be discussed early.

# MentorsMind Soroban Benchmarks

Measures CPU instruction count, memory usage, and WASM binary size for critical
contract entry points. CI fails on any metric regressing more than **10%** from
the recorded baseline.

## Running locally

```bash
# From workspace root

# 1. Build WASM binaries (populates wasm_bytes in the report)
cargo build \
  --target wasm32-unknown-unknown \
  --release \
  -p mentorminds-escrow \
  -p mentorminds-staking \
  -p mentorminds-governance \
  -p mentorminds-timelock

# 2. Run benchmarks (compare against baselines.json, exit 1 on regression)
cargo run -p mentorminds-benchmarks
```

Reports are written to `benchmarks/results/`:
- `report.json` — machine-readable per-function metrics
- `report.html` — human-readable table, open in a browser
- `bench.log` — captured in CI as an artifact

## Updating the baseline

The baseline should only be updated intentionally, not on every PR. Two ways:

**Option A — CI (recommended):** trigger the `Soroban Benchmarks` workflow
manually from the Actions tab with `update_baseline = true`. It runs the
benchmarks, copies `results/report.json` → `baselines.json`, and commits.

**Option B — local:**
```bash
cargo run -p mentorminds-benchmarks
cp benchmarks/results/report.json benchmarks/baselines.json
git commit benchmarks/baselines.json -m "chore(bench): update baselines"
```

## How it works

The harness uses `soroban-sdk` testutils `Env::budget()` to capture host-level
metrics:

```
env.budget().reset_default();   // zero the counters
contract_client.some_fn(...);   // the measured call
let cpu = env.budget().cpu_instruction_count();
let mem = env.budget().memory_bytes_count();
```

Each entry point gets its own fresh `Env` and contract fixture so measurements
are isolated — setup cost does not contaminate the measured function.

## Covered entry points

| Contract    | Entry Points |
|-------------|-------------|
| escrow      | `create_escrow`, `release_funds`, `dispute`, `resolve_dispute` |
| staking     | `stake`, `unstake`, `distribute_revenue_batch`, `claim_rewards` |
| governance  | `create_proposal`, `vote`, `execute_proposal` |
| timelock    | `schedule`, `execute` |

## Thresholds

| Metric | Regression gate | Alert |
|--------|----------------|-------|
| `cpu_instructions` | > 10% increase | — |
| `mem_bytes` | > 10% increase | — |
| `storage_reads` | > 10% increase | — |
| `storage_writes` | > 10% increase | — |
| `wasm_bytes` | > 10% increase | Hard alert if > 64 KB |

## Adding a new benchmark

1. Add a function to the relevant suite in `benchmarks/src/suites/`.
2. Push a new `BenchResult` to the `results` vec in that suite's `run()`.
3. Run locally to generate a `report.json`, then copy it to `baselines.json`.

## CI behaviour

The `Soroban Benchmarks` workflow runs on every PR that touches the benchmarked
contracts or the `benchmarks/` crate itself. It:

1. Builds WASM release binaries for size tracking.
2. Runs `cargo run -p mentorminds-benchmarks`.
3. Uploads `report.json`, `report.html`, and `bench.log` as artifacts (90-day
   retention).
4. Posts a summary table as a PR comment (updates the comment on re-runs).
5. Exits with code 1 and fails the check if any metric exceeds the 10% gate.

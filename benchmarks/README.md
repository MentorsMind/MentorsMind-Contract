# MentorsMind Soroban Benchmarks

Measures CPU instruction count, memory usage, and WASM binary size for critical
contract entry points. CI fails on any metric regressing more than **10%** from
the recorded baseline.

## Running locally

```bash
# From workspace root

# 1. Build WASM binaries (populates wasm_bytes in the report)
cargo build \
  --target wasm32v1-none \
  --release \
  -p mentorminds-escrow \
  -p mentorminds-staking \
  -p mentorminds-governance \
  -p mentorminds-timelock \
  -p mentorminds-upgrade-registry \
  -p mentorminds-dispute-evidence

# 2. Run benchmarks (compare against baselines.json, exit 1 on regression)
cargo run -p mentorminds-benchmarks
```

Reports are written to `benchmarks/results/`:
- `report.json` — machine-readable per-function metrics
- `report.html` — human-readable table with interactive trend charts
- `bench.log` — captured in CI as an artifact

Historical snapshots are stored in `benchmarks/history/` as
`YYYY-MM-DD_<short-sha>.json` and committed to the repo after each main-branch
run. The HTML report renders up to 30 of these as sparkline trend charts.

## Updating the baseline

The baseline should only be updated intentionally, not on every PR.

**Option A — CI (recommended):** Trigger the `Soroban Benchmarks` workflow
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

```rust
env.budget().reset_default();      // zero the counters
contract_client.some_fn(...);      // the measured call
let cpu = env.budget().cpu_instruction_count();
let mem = env.budget().memory_bytes_count();
```

Each entry point gets its own fresh `Env` and contract fixture so measurements
are isolated — setup cost does not contaminate the measured function.

## Historical tracking

After every successful run on `main` (push or nightly schedule), the benchmark
binary writes a timestamped record to `benchmarks/history/`. CI commits those
files automatically using the `stefanzweifel/git-auto-commit-action` step.

History files are named `YYYY-MM-DD_<short-sha>.json` and contain the full
`BenchResult` array plus run metadata (date, full SHA, ref name). The HTML
dashboard reads up to 30 of the most recent records to draw per-entry-point
CPU trend charts.

To bootstrap history on an existing repo, run `cargo run -p mentorminds-benchmarks`
locally (with `BENCH_DATE`, `GITHUB_SHA`, and `GITHUB_REF_NAME` set) and commit
the generated files:

```bash
export BENCH_DATE=$(date '+%Y-%m-%d')
export GITHUB_SHA=$(git rev-parse HEAD)
export GITHUB_REF_NAME=$(git branch --show-current)
cargo run -p mentorminds-benchmarks
git add benchmarks/history/
git commit -m "chore(bench): bootstrap performance history"
```

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
| `cpu_instructions` | > 10% increase | GitHub Actions annotation |
| `mem_bytes` | > 10% increase | GitHub Actions annotation |
| `storage_reads` | > 10% increase | GitHub Actions annotation |
| `storage_writes` | > 10% increase | GitHub Actions annotation |
| `wasm_bytes` | > 10% increase | Annotation + hard alert if > 64 KB |

## Regression alerts

When a regression is detected the benchmark binary:

1. Exits with code **1** — failing the CI check.
2. Emits `::error` GitHub Actions [workflow commands][wf-cmds] so each
   regression appears as an inline annotation in the PR diff view.
3. Writes a detailed **job summary** (visible on the Actions run page) listing
   every regressed metric with baseline, measured value, and percentage delta.

[wf-cmds]: https://docs.github.com/en/actions/writing-workflows/choosing-what-your-workflow-does/workflow-commands-for-github-actions

## CI behaviour

The `Soroban Benchmarks` workflow runs on:
- Every PR touching benchmarked contracts or `benchmarks/`
- Every push to `main` (same path filters)
- A **nightly schedule** at 03:00 UTC to catch drift not triggered by code changes

### Steps

1. Build WASM release binaries for size tracking.
2. Run `cargo run -p mentorminds-benchmarks`.
3. Upload `report.json`, `report.html`, and `bench.log` as artifacts (90-day retention).
4. **Commit history record** to `benchmarks/history/` (main/schedule only).
5. Post a summary table as a PR comment (updates on re-runs).
6. Exits with code 1 and fails the check if any metric exceeds the 10% gate.

## Adding a new benchmark

1. Add a function to the relevant suite in `benchmarks/src/suites/`.
2. Push a new `BenchResult` to the `results` vec in that suite's `run()`.
3. Run locally to generate a `report.json`, then copy it to `baselines.json`.
4. After merging, CI will start tracking the new entry point in history.

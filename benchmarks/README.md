# MentorsMind Soroban Benchmarks

This directory contains the performance benchmark harness for all MentorsMind smart contracts. It measures CPU instruction count, memory usage, and WASM binary size for each contract entry point using the Soroban SDK testutils budget API, then compares results against a committed baseline to catch regressions automatically in CI.

---

## Table of Contents

1. [Running the Benchmarks](#1-running-the-benchmarks)
2. [Understanding the Output](#2-understanding-the-output)
3. [Metrics Captured](#3-metrics-captured)
4. [The 10% Regression Gate](#4-the-10-regression-gate)
5. [Updating Baselines](#5-updating-baselines)
6. [The history/ Directory and Trend Charts](#6-the-history-directory-and-trend-charts)
7. [Adding a New Suite](#7-adding-a-new-suite)
8. [CI Integration](#8-ci-integration)

---

## 1. Running the Benchmarks

Run from the **workspace root** (not from inside `benchmarks/`):

```sh
cargo run -p mentorminds-benchmarks
```

The binary must be run from the workspace root so that relative paths to `benchmarks/baselines.json`, `benchmarks/history/`, and `benchmarks/results/` resolve correctly.

### Optional: include WASM sizes

WASM sizes are read from `target/wasm32v1-none/release/<crate_name>.wasm`. To populate them, build the contracts for the WASM target first:

```sh
cargo build --target wasm32v1-none --release
cargo run -p mentorminds-benchmarks
```

Without a prior WASM build the `wasm_bytes` column will show `0` / `N/A` and the WASM regression gate is skipped for that entry.

---

## 2. Understanding the Output

The run sequence is:

1. All suites execute and collect `BenchResult` records.
2. **`benchmarks/results/report.json`** — full machine-readable results for this run.
3. **`benchmarks/results/report.html`** — human-readable per-function table with trend charts (open in a browser).
4. **`benchmarks/results/gas_accuracy.json`** — gas estimation accuracy report.
5. **`benchmarks/history/<date>_<sha>.json`** — this run is appended to the history directory.
6. Regression check runs against `benchmarks/baselines.json`. The process exits with code `1` if any metric regressed; `0` otherwise.

Console output looks like this on a clean run:

```
── escrow ──
  create_escrow                  cpu=       890,000  mem=    15,680
  release_funds                  cpu=       650,000  mem=    12,240
  ...

✅  All metrics within 10% of baseline.
📚  History record written to benchmarks/history/2026-09-25_a1b2c3d4.json
```

On a regression:

```
❌  REGRESSIONS DETECTED (1 total):
  [escrow] create_escrow — cpu_instructions exceeded baseline by 15.3% (baseline=890000, measured=1026270)
```

---

## 3. Metrics Captured

Each `BenchResult` record carries the following fields:

| Field | Type | Description |
|---|---|---|
| `cpu_instructions` | `u64` | Soroban host CPU instruction count consumed by the call, measured via `env.budget().cpu_instruction_cost()`. |
| `mem_bytes` | `u64` | Memory bytes consumed by the call, measured via `env.budget().memory_bytes()`. |
| `storage_reads` | `u32` | Storage read operations (currently `0`; placeholder for a future Soroban SDK version that exposes per-call storage metrics). |
| `storage_writes` | `u32` | Storage write operations (same placeholder caveat). |
| `wasm_bytes` | `u64` | Compiled WASM binary size in bytes, read from `target/wasm32v1-none/release/<crate>.wasm`. `0` when the WASM target has not been built. |

**How measurement works:** before each call the harness resets the host budget with `env.budget().reset_default()`, runs the closure under test, then snapshots the counters. This isolates the measured function from setup costs.

**Gas accuracy:** the `gas_estimation` suite also compares estimated CPU against actual CPU for each operation. Estimates within 20% of actual pass; failures are reported separately in `gas_accuracy.json` and printed to stderr.

---

## 4. The 10% Regression Gate

After all suites run, `harness::check_regressions` loads `benchmarks/baselines.json` and checks every metric in the table above against the stored baseline. A regression is triggered when:

```
(measured - baseline) / baseline > 0.10
```

i.e., the measured value is **more than 10% higher** than the baseline. Improvements (lower values) never trigger a failure.

**Special cases:**

- If a metric's baseline value is `0`, that metric is skipped — it was not available when the baseline was recorded.
- **WASM size** additionally has a hard alert at **64 KB**: any WASM binary exceeding this limit prints a warning regardless of the percentage change.
- New entry points (present in results but missing from `baselines.json`) are logged as informational and do not fail the run. They will be included the next time you update the baseline.

---

## 5. Updating Baselines

**When to update:** after an intentional change that raises resource usage (e.g., a new feature that adds storage writes), or after an optimization that lowers it.

**How to update:**

1. Run the benchmarks and confirm the new numbers look correct:
   ```sh
   cargo run -p mentorminds-benchmarks
   ```
2. Copy `benchmarks/results/report.json` over `benchmarks/baselines.json`:
   ```sh
   # PowerShell
   Copy-Item benchmarks\results\report.json benchmarks\baselines.json

   # bash / macOS / Linux
   cp benchmarks/results/report.json benchmarks/baselines.json
   ```
3. Commit `baselines.json` together with the code change that caused the measurement shift. This keeps the baseline and the code in sync in the same commit.

**First-time setup:** if `benchmarks/baselines.json` does not exist when you run the harness, it is created automatically from the current results and the process exits `0`. Commit the generated file before pushing so CI has a baseline to check against.

---

## 6. The `history/` Directory and Trend Charts

Every benchmark run writes a snapshot to `benchmarks/history/` with the filename pattern:

```
YYYY-MM-DD_<short-git-sha>.json
```

For example: `2026-09-25_a1b2c3d.json`

The date comes from the `BENCH_DATE` environment variable (set by CI). The SHA comes from `GITHUB_SHA`. When running locally these fall back to `unknown-date` and `local` respectively, producing a file like `unknown-date_local.json`.

**File format:**

```json
{
  "date": "2026-09-25",
  "sha": "a1b2c3d4e5f6...",
  "ref_name": "main",
  "results": [ /* array of BenchResult objects, same schema as baselines.json */ ]
}
```

**Why it's committed:** history files are committed to the repository so trends persist across CI runs without relying on artifact retention windows.

**Trend charts:** `benchmarks/results/report.html` renders a Chart.js line graph for each `(contract, entry_point)` pair showing CPU instruction counts over the last 30 runs, sorted chronologically by filename. Charts only appear after **at least 2 history records** exist.

**Reading the charts:**

- A flat line is good — stable performance.
- A sudden upward spike indicates a regression was introduced around that commit.
- A downward step indicates an optimization landed.
- The X-axis labels show `YYYY-MM-DD (short-sha)` so you can correlate a data point directly to a commit.

---

## 7. Adding a New Suite

Follow the `escrow.rs` pattern in `benchmarks/src/suites/`.

### Step 1 — Create the suite file

Create `benchmarks/src/suites/<contract_name>.rs`. The minimal structure is:

```rust
extern crate std;

use crate::harness::{measure, wasm_size, BenchResult};
use mentorminds_<contract_name>::{MyContract, MyContractClient};
use soroban_sdk::{testutils::Address as _, Address, Env};

const CONTRACT: &str = "<contract_name>";       // used in BenchResult.contract
const WASM_CRATE: &str = "mentorminds_<contract_name>"; // maps to WASM filename

// ---------------------------------------------------------------------------
// Fixture — set up a clean environment for each measured call
// ---------------------------------------------------------------------------

struct Fixture {
    env: Env,
    contract_id: Address,
    // add any addresses / tokens your contract needs
}

impl Fixture {
    fn new() -> Self {
        let env = Env::default();
        env.mock_all_auths();

        let contract_id = env.register(MyContract, ());
        let admin = Address::generate(&env);

        // initialize the contract, mint tokens, etc.
        MyContractClient::new(&env, &contract_id).initialize(&admin);

        Fixture { env, contract_id }
    }

    fn client(&self) -> MyContractClient<'_> {
        MyContractClient::new(&self.env, &self.contract_id)
    }
}

// ---------------------------------------------------------------------------
// Suite entry point
// ---------------------------------------------------------------------------

pub fn run() -> Vec<BenchResult> {
    let wasm = wasm_size(WASM_CRATE);
    let mut results: Vec<BenchResult> = Vec::new();

    // --- my_entry_point ---
    {
        let f = Fixture::new();
        let snap = measure(&f.env, || {
            f.client().my_entry_point(/* args */);
        });
        results.push(BenchResult {
            contract: CONTRACT.into(),
            entry_point: "my_entry_point".into(),
            cpu_instructions: snap.cpu_instructions,
            mem_bytes: snap.mem_bytes,
            storage_reads: 0,
            storage_writes: 0,
            wasm_bytes: wasm,
        });
    }

    print_suite(&results);
    results
}

fn print_suite(results: &[BenchResult]) {
    println!("\n── {} ──", CONTRACT);
    for r in results {
        println!(
            "  {:30} cpu={:>12}  mem={:>10}",
            r.entry_point, r.cpu_instructions, r.mem_bytes
        );
    }
}
```

Key points from the `escrow.rs` pattern:
- Use a **fresh `Fixture`** for each measured entry point — don't share state between measurements.
- Call `env.mock_all_auths()` in `Fixture::new()` so auth checks don't interfere with measurements.
- Any **setup work** (e.g. calling `create_escrow` before benchmarking `release_funds`) goes **outside** the `measure(...)` closure so it is not counted.
- `wasm_size(WASM_CRATE)` is called once per suite and reused across all results in that suite.

### Step 2 — Register the suite in `mod.rs`

Add a `pub mod` line to `benchmarks/src/suites/mod.rs`:

```rust
pub mod <contract_name>;
```

### Step 3 — Call the suite in `main.rs`

Add a line to `run_all_suites()` in `benchmarks/src/main.rs`:

```rust
all.extend(suites::<contract_name>::run());
```

### Step 4 — Add the contract dependency to `benchmarks/Cargo.toml`

```toml
[dependencies.mentorminds-<contract-name>]
path = "../contracts/<contract_name>"
features = ["testutils"]
```

### Step 5 — Regenerate the baseline

Run the benchmarks once so the new entry points appear in `baselines.json`:

```sh
cargo run -p mentorminds-benchmarks
cp benchmarks/results/report.json benchmarks/baselines.json
```

Commit both `baselines.json` and your new suite file together.

---

## 8. CI Integration

The benchmark binary integrates with GitHub Actions automatically when the following environment variables are set by the runner:

| Variable | Purpose |
|---|---|
| `GITHUB_SHA` | Full commit SHA — embedded in the history filename and summary. |
| `GITHUB_REF_NAME` | Branch or tag name — embedded in the history record. |
| `BENCH_DATE` | Date string (e.g. `2026-09-25`) — used as the date prefix in the history filename. Set this in your workflow with `echo "BENCH_DATE=$(date -u +%F)" >> $GITHUB_ENV`. |
| `GITHUB_STEP_SUMMARY` | Path to the job summary file — the harness appends a Markdown results table automatically. |

**Regression annotations:** when regressions are found, the harness emits GitHub Actions `::error` workflow commands that surface as annotations directly in the PR diff view:

```
::error title=Performance Regression [escrow/create_escrow]::Metric `cpu_instructions` exceeded 10% baseline — baseline=890000, measured=1026270, delta=+15.3%
```

**Exit codes:**
- `0` — all metrics within baseline (or no baseline exists yet).
- `1` — one or more regressions detected, or gas estimation accuracy failures.

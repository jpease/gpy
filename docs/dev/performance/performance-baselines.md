# Performance Baselines

This document describes how to establish and maintain performance baselines for GPY to ensure that changes don't introduce performance regressions.

These baselines are engineering diagnostics. They are meant to help us compare GPY to earlier GPY revisions, detect regressions, and validate improvements. They are not promises about the exact latency any user will see on their own machine.

The hook-time `perf-canary` workflow follows the same philosophy. It compares a
small set of microbenchmarks against the last pushed commit on the same machine
so regressions are easier to catch early, while still tolerating ordinary
developer-machine noise. Treat it as a relative signal, not an absolute truth.

The project’s current `<10ms` target is for user-visible cached/instant
response behavior. It is not a claim that fresh full git or language
computation completes within `<10ms` in every repository or host condition.

## Overview

GPY's performance is critical to user experience since it runs on every prompt render. We maintain baselines for:

1. **Formatter hot paths** - Git status and language formatting (<5ms target)
2. **IPC roundtrip** - Agent communication latency (<1.5ms max budget)
3. **Git operations** - Repository status detection (<50ms target for cached, <500ms for cold)

## Running Benchmarks

GPY uses three different performance layers:

- **Microbenchmarks**: Criterion-based measurements of internal hot paths.
- **Budget checks**: Conservative regression gates used for CI and local verification.
- **Real-world oneshot benchmarks**: Pinned repository runs of `gpy-agent oneshot git --format json`.

Keep their claims separate when communicating performance.

## Interpreting Results Honestly

- Absolute timings will vary across machines, filesystems, kernel versions, thermal states, and background activity.
- CI budgets and local baselines are primarily regression tools, not marketing numbers.
- Real-world repository benchmarks can support comparative statements only when the fixtures, scope, and host conditions are explicitly documented.
- If a run shows unusually high `System` time, wide variance, or host-specific warnings, treat it as a measurement problem first and investigate before drawing product conclusions.

### Formatter Benchmarks (Rust)

The formatter benchmarks measure the performance of the code paths executed on every prompt render:

```bash
# Run all formatter benchmarks
cd gpy-agent
cargo bench --bench formatter_bench

# Run specific benchmark
cargo bench --bench formatter_bench -- git_status_ansi_short_branch

# Generate HTML report (saved to target/criterion/)
cargo bench --bench formatter_bench
open target/criterion/report/index.html
```

**Key metrics to track:**
- `git_status_ansi_short_branch` - Baseline git rendering (<500μs)
- `git_status_ansi_long_branch` - Long branch name rendering (<1ms)
- `git_status_ansi_truncated_branch` - Branch truncation overhead (should be minimal, <1ms)
- `language_ansi_single` - Single language rendering (<500μs)
- `language_ansi_multiple` - Multiple languages rendering (<1ms)

### Agent Cold-Start Benchmarks (Rust)

`Agent::new()` (`gpy-agent/src/agent/mod.rs`) registers global file watchers and
binds a Unix socket, which makes benchmarking the full constructor
impractical and non-hermetic in a Criterion harness. `agent_startup_bench.rs`
instead benches its dominant sub-steps individually, plus the
version-detection subprocess spawn path that also runs on the first prompt
render (via language detection) but is not itself part of `Agent::new`
(issue #335, first task of the perf-pass epic #334):

```bash
cd gpy-agent
cargo bench --bench agent_startup_bench
```

**Key metrics to track (baseline, Apple Silicon dev machine, 2026-07-02):**
- `config_load_from_file` - config parsing (`ConfigManager::new`'s
  `load_config_from_file` step) against a representative fixture in a tempdir
  — **16.4µs** mean
- `theme_manager_load` - `ThemeManager::new`, the exact call
  `Agent::init_theme_manager` makes — **40.7µs** mean
- `theme_export_write` - `write_theme_export_cache`'s render + atomic-write
  work, benched through the `write_theme_export_to_dir` explicit-directory
  seam into a tempdir — **164.6µs** mean
- `version_command_spawn` - `execute_version_command`'s spawn +
  own-process-group placement + reader thread + timeout wait + parse, using a
  stub `ReleaseSource` (`true` / `cmd /C exit 0`) so no real toolchain is
  shelled out to — **1.594ms** mean

These absolute numbers are machine-specific engineering diagnostics, not
promises about any user's host — see "Interpreting Results Honestly" above.
`version_command_spawn` in particular is dominated by OS process-creation
cost (fork/exec or `CreateProcess`), which varies significantly across
machines and CI runners; its budget carries much larger headroom than the
in-process benchmarks for that reason.

### IPC and Integration Benchmarks (Shell)

The shell-level benchmarks measure end-to-end performance including IPC:

```bash
# Run full benchmark suite (local mode)
./scripts/bench.sh

# Run with specific file signature mode
./scripts/bench.sh -m files

# Run CI mode with budget enforcement
./scripts/bench.sh --ci
```

**Key metrics to track:**
- IPC latency (full roundtrip to agent)
- Git status cold start (uncached)
- Language detection (with/without cache)

### Real-World Repository Benchmarks

The repository benchmark suite exercises the `gpy-agent oneshot git --format json` path against pinned open-source repositories:

```bash
cd gpy-agent
./benchmarks/run.sh --compare
```

By default this runs in `benchmark-mode` scope, which skips config/theme file I/O so the measurements reflect the git-status request path rather than user-specific config loading. To measure full CLI behavior instead:

```bash
cd gpy-agent
BENCH_SCOPE=user-config ./benchmarks/run.sh --compare
```

Use the default scope for comparative or public-facing git performance claims. Use `user-config` only for troubleshooting or end-to-end CLI investigations.

### Real-World Benchmark Protocol

For repeatable real-world runs on any developer machine:

```bash
cd gpy-agent
BENCH_DIR=/tmp/gpy-benchmarks ./benchmarks/setup-repos.sh
for repo in /tmp/gpy-benchmarks/*; do
  git -C "$repo" config core.fsmonitor false
done
BENCH_DIR=/tmp/gpy-benchmarks BENCH_RUNS=5 ./benchmarks/run.sh
```

Rules:
- Run this suite by itself, not alongside `cargo bench`, `just check`, or other heavy jobs.
- Recreate or reset fixtures before taking comparison-grade measurements.
- Verify benchmark repos are clean before publishing or saving baseline results.
- Prefer `/tmp` or another known-writable local path if `~/.cache` behavior differs across environments.

## Establishing Baselines

When adding new features or making significant changes, establish new baselines:

### 1. Run benchmarks in clean state

```bash
# Ensure clean build
cd gpy-agent
cargo clean
cargo build --release

# Run formatter benchmarks
cargo bench --bench formatter_bench

# Run integration benchmarks
cd ..
./scripts/bench.sh
```

### 2. Record baseline results

Create or update `tests/performance-baselines.json` in the project root:

```json
{
  "timestamp": "2026-05-15T00:00:00Z",
  "commit": "8e29d83",
  "benchmarks": {
    "formatter": {
      "git_status_ansi_short_branch": {
        "mean_ns": 562,
        "std_dev_ns": 10
      },
      "git_status_ansi_long_branch": {
        "mean_ns": 595,
        "std_dev_ns": 12
      },
      "git_status_ansi_truncated_branch": {
        "mean_ns": 645,
        "std_dev_ns": 12
      },
      "language_ansi_single": {
        "mean_ns": 394,
        "std_dev_ns": 8
      }
    },
    "ipc": {
      "roundtrip_latency_ms": 0.037,
      "std_dev_ms": 0.001
    },
    "git": {
      "status_cold_ms": 17.2,
      "status_cached_ms": 0.04
    }
  },
  "budgets": {
    "ipc_latency": {
        "max_ms": 1.5,
        "description": "IPC roundtrip must complete within 1.5ms"
    },
    "git_status_cold": {
      "max_ms": 750,
      "description": "Cold git status within 750ms (CI tolerance)"
    },
    "cache_write": {
      "max_ms": 25,
      "description": "Cache operations within 25ms"
    },
    "memory_usage": {
      "max_mb": 50,
      "description": "Agent memory footprint under 50MB"
    }
  },
  "regression_thresholds": {
    "major_regression_percent": 50,
    "measurement_variance_percent": 15
  }
}
```

### 3. Validate baselines

Run benchmarks multiple times to ensure consistency:

```bash
# Run 5 times and compare results
for i in {1..5}; do
  echo "Run $i"
  cargo bench --bench formatter_bench -- git_status_ansi_short_branch
done
```

Results should be within ±15% variance. If not, investigate:
- Background processes consuming CPU
- Thermal throttling
- Disk I/O contention
- Backup/indexing activity (for example Time Machine or Spotlight)
- Repo-local features such as broken `fsmonitor`
- Host-specific filesystem behavior on large fixtures

## Performance Budgets

The `scripts/bench.sh --ci` mode enforces these budgets (with 15% tolerance):

| Metric | Budget | Notes |
|--------|--------|-------|
| IPC Latency | 1.5ms | Full roundtrip including socket overhead |
| Git Status (cold) | 750ms | Includes filesystem operations |
| Cache Operations | 25ms | Write + invalidation |
| Formatter (git short) | 1ms | Fast path for common case |
| Formatter (git long) | 2ms | With truncation overhead |
| Memory Usage | 50MB | Agent resident memory |
| Config Load | 100ms | `config_load_from_file` (issue #335) |
| Theme Manager Load | 20ms | `theme_manager_load` (issue #335) |
| Theme Export Write | 30ms | `theme_export_write` (issue #335) |
| Version Command Spawn | 50ms | `version_command_spawn`, stub command (issue #335) |

## Detecting Regressions

### Automated Baseline Comparison

The performance baseline script can compare current measurements against saved baselines:

```bash
# Compare current baselines to saved values
./scripts/perf-baseline.sh --compare

# Example output:
# [INFO] Comparing current results to baseline...
# [INFO] Baseline file: tests/performance-baselines.json
#
# [INFO] Regression thresholds:
#   Major regression: >50%
#   Minor regression: >20%
#   Variance tolerance: ±15%
#
# [INFO] Budget compliance check:
# [✓] ipc_latency: 0.2ms / 1.5ms (13.3%) ✓
#   └─ IPC roundtrip latency budget
# [✓] git_status_cold: 45.2ms / 750.0ms (6.0%) ✓
#   └─ Git status scan (cold cache) budget
# [✓] cache_write: 2.1ms / 25.0ms (8.4%) ✓
#   └─ Cache write operation budget
#
# [✓] ✓ All measured metrics within budgets
```

**Exit codes:**
- `0`: All metrics within budgets
- `1`: One or more metrics exceed budgets or comparison failed

**Requirements:**
- `jq` must be installed for comparison (`sudo apt install jq` or `brew install jq`)
- `bc` must be installed for percentage calculations (usually pre-installed)

**Understanding the output:**

```bash
# Green checkmark: Metric is within budget
[✓] ipc_latency: 0.2ms / 1.5ms (13.3%) ✓
  └─ IPC roundtrip latency budget

# Red X: Metric exceeds budget (regression detected)
[✗] git_status_cold: 800ms / 750.0ms (106.7%) EXCEEDS BUDGET
  └─ Git status scan (cold cache) budget

# Warning: No baseline available yet
[⚠] cache_write: No baseline measurement available
  └─ Cache write operation budget
```

### Local Development Integration

Performance checks are integrated into the quality-check script for easy local validation:

```bash
# Run all quality checks including performance regression checks
./scripts/quality-check.sh --with-perf

# This will:
# 1. Run all standard quality checks (format, lint, tests)
# 2. Compare current performance against baselines in tests/performance-baselines.json
# 3. Exit non-zero if any budgets are exceeded
```

**Requirements:**
- `jq` must be installed (`sudo apt install jq` or `brew install jq`)
- `bc` must be installed (usually pre-installed on most systems)

**When to run:**
- Before committing performance-sensitive changes
- Before creating pull requests
- When updating formatter code paths
- After dependency upgrades

**What to do when it fails:**
1. Check which metric exceeded its budget in the output
2. Run the specific benchmark to get detailed results:
   ```bash
   ./scripts/perf-baseline.sh --rust  # For formatter benchmarks
   ./scripts/perf-baseline.sh --shell # For IPC benchmarks
   ```
3. Investigate the regression using profiling tools (see "Optimization Guidelines" below)
4. Either fix the regression or update the baseline if the change is justified

### CI Integration

**⏳ Performance regression checks are READY but not yet integrated into CI**

**Current Status:**
- Integration step is designed and documented
- Requires manual addition to `.github/workflows/ci.yml` (workflow permissions limitation)
- See `CI_INTEGRATION_GUIDE.md` in project root for complete instructions

**Recommended CI Setup:**

Add this step to `.github/workflows/ci.yml` in the `benchmark` job (after "Generate performance report"):

```yaml
- name: Performance baseline regression check
  continue-on-error: true  # Advisory mode while establishing baselines
  run: |
    echo "Running performance baseline comparison..."
    sudo apt-get update
    sudo apt-get install -y jq bc
    ./scripts/perf-baseline.sh --compare

    BASELINE_CHECK_EXIT=$?
    if [ $BASELINE_CHECK_EXIT -eq 0 ]; then
      echo "✅ All performance metrics within budgets"
      echo "PERF_BASELINE_STATUS=PASS" >> $GITHUB_ENV
    else
      echo "⚠️ Some performance metrics exceed budgets"
      echo "PERF_BASELINE_STATUS=WARN" >> $GITHUB_ENV
    fi
```

**Once integrated, it will:**
1. Run benchmarks as part of the `benchmark` job
2. Install jq and bc for comparison
3. Run `./scripts/perf-baseline.sh --compare`
4. Report results in build logs
5. Set `PERF_BASELINE_STATUS` environment variable (PASS/WARN)
6. Run in advisory mode initially (doesn't block builds)

**Future:** After 2-3 weeks of data collection, make it blocking by removing `continue-on-error: true`.

**Manual CI-mode benchmarks:**
You can also run benchmarks with budget enforcement locally:
```bash
./scripts/bench.sh --ci
# Exit code 0: All benchmarks within budget
# Exit code 1: One or more budgets exceeded
```

### Manual Comparison

Compare current results to baseline for deeper analysis:

```bash
# Run current benchmarks
cargo bench --bench formatter_bench > current-results.txt

# Compare to baseline using the script
./scripts/perf-baseline.sh --compare

# Or manually inspect the baseline file
jq '.baselines' tests/performance-baselines.json
```

**Red flags:**
- Mean time increase >50% (major regression)
- Standard deviation increase >100% (unstable performance)
- New allocations in hot paths (use `cargo flamegraph` or `heaptrack`)
- Metrics exceeding their defined budgets

## Best Practices

### When to Benchmark

**Always benchmark when:**
- Adding new formatters or segments
- Changing IPC protocol
- Modifying cache strategy
- Updating hot-path dependencies (serde, tokio, etc.)

**Optional for:**
- Documentation changes
- Test-only changes
- Non-performance config additions

### Optimization Guidelines

1. **Profile before optimizing** - Use `cargo flamegraph` to find actual bottlenecks
2. **Measure impact** - Run benchmarks before and after changes
3. **Avoid premature optimization** - Focus on algorithmic improvements first
4. **Test on target hardware** - Benchmarks on high-end dev machines don't reflect user experience

### Benchmark Hygiene

- Run benchmarks on idle system (close browsers, stop background tasks)
- Use release builds only (`cargo bench` does this automatically)
- Account for variance (±15% is normal)
- Don't commit `target/criterion/` directory
- Update baselines after major changes

## Troubleshooting

### Benchmarks are noisy (>20% variance)

**Causes:**
- Background processes
- Thermal throttling
- Swap activity

**Fix:**
```bash
# Close unnecessary applications
# Disable CPU frequency scaling (Linux)
echo performance | sudo tee /sys/devices/system/cpu/cpu*/cpufreq/scaling_governor

# Re-run benchmarks
cargo bench --bench formatter_bench
```

### CI benchmarks fail but local passes

**Causes:**
- Different hardware (slower CI runners)
- Concurrent CI jobs
- Cold caches

**Fix:**
- Increase budget tolerance in `tests/performance-baselines.json`
- Add warmup iterations
- Consider hardware-specific baselines

### Benchmarks take too long

**Causes:**
- Too many iterations
- Large sample sizes

**Fix:**
```rust
// In benchmark code, reduce sample size
c.bench_function("my_bench", |b| {
    b.iter(|| expensive_operation())
}).sample_size(10);  // Default is 100
```

## Additional Resources

- [Criterion.rs documentation](https://bheisler.github.io/criterion.rs/book/)
- [Rust Performance Book](https://nnethercote.github.io/perf-book/)
- [hyperfine](https://github.com/sharkdp/hyperfine) - Shell benchmark tool used in `bench.sh`

## See Also

- `scripts/bench.sh` - Shell-level benchmarking
- `gpy-agent/benches/` - Rust microbenchmarks
- `docs/ARCHITECTURE.md` - System design and performance considerations

# Performance Baselines

This document describes how to establish and maintain performance baselines for GPY to ensure that changes don't introduce performance regressions.

These baselines are engineering diagnostics. They are meant to help us compare GPY to earlier GPY revisions, detect regressions, and validate improvements. They are not promises about the exact latency any user will see on their own machine.

The `perf-canary` script follows the same philosophy. It compares a small set
of microbenchmarks against a base commit on the same machine (locally the last
pushed commit; in CI the pull request's base, in the `perf` job of
`pr-gate.yml`) so regressions are easier to catch early, while still tolerating
ordinary machine noise. Treat it as a relative signal, not an absolute truth.

The cached prompt render budget (`prompt_render`: 20 ms target, 50 ms max) is
for user-visible cached/instant response behavior. It is not a claim that
fresh full git or language computation fits that budget in every repository
or host condition; those have their own, larger budgets (`prompt_render_cold`,
`git_status_cold`).

## Overview

The agent and shell integration run on every prompt render. Budgets live in
`benchmarks/perf-budgets.json`; baselines cover:

1. **Formatter hot paths** - Git status and language formatting
2. **IPC roundtrip** - Agent communication latency (`ipc_latency`: 1 ms target, 1.5 ms max)
3. **Git operations** - Repository status (`git_status_cold`: 500 ms target)

## Running Benchmarks

GPY uses three different performance layers:

- **Microbenchmarks**: Criterion-based measurements of internal hot paths.
- **Budget checks**: Conservative regression gates used for CI and local verification.
- **Real-world oneshot benchmarks**: Pinned repository runs of `gpy-agent oneshot git --format json`.

Keep their claims separate when communicating performance.

## Interpreting Results

- Absolute timings will vary across machines, filesystems, kernel versions, thermal states, and background activity.
- CI budgets and local baselines are primarily regression tools.
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
render (via language detection) but is not itself part of `Agent::new`:

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
promises about any user's host. See "Interpreting Results" above.
`version_command_spawn` in particular is dominated by OS process-creation
cost (fork/exec or `CreateProcess`), which varies significantly across
machines and CI runners; its budget carries much larger headroom than the
in-process benchmarks for that reason.

### IPC and Integration Benchmarks

`./scripts/bench.sh --ci` rebuilds the release agent, runs `cargo bench` and enforces the budgets below.
`./scripts/ci-bench.sh` measures the end-to-end shell path (see
[benchmarking.md](benchmarking.md)).

```bash
./scripts/bench.sh --ci
```

**Key metrics to track:**
- IPC latency (full roundtrip to agent)
- Git status cold start (uncached)
- Language detection (with/without cache)

### Real-World Repository Benchmarks

The repository benchmark suite exercises the `gpy-agent oneshot git --format json` path against pinned open-source repositories:

```bash
./benchmarks/real-repos/run.sh --compare
```

By default this runs in `benchmark-mode` scope, which skips config/theme file I/O so the measurements reflect the git-status request path rather than user-specific config loading. To measure full CLI behavior instead:

```bash
BENCH_SCOPE=user-config ./benchmarks/real-repos/run.sh --compare
```

Use the default scope for comparative or public-facing git performance claims. Use `user-config` only for troubleshooting or end-to-end CLI investigations.

### Real-World Benchmark Protocol

For repeatable real-world runs on any developer machine:

```bash
BENCH_DIR=/tmp/gpy-benchmarks ./benchmarks/real-repos/setup-repos.sh
for repo in /tmp/gpy-benchmarks/*/; do
  git -C "$repo" config core.fsmonitor false
done
BENCH_DIR=/tmp/gpy-benchmarks BENCH_RUNS=5 ./benchmarks/real-repos/run.sh
```

Rules:
- Run this suite by itself, not alongside `cargo bench`, `just check`, or other heavy jobs.
- Recreate or reset fixtures before taking comparison-grade measurements.
- Verify benchmark repos are clean before publishing or saving baseline results.
- Prefer `/tmp` or another known-writable local path if `~/.cache` behavior differs across environments.

## Updating Budgets

When adding new features or making significant changes, measure first, then
update the budget file.

### 1. Run benchmarks in clean state

```bash
# Ensure clean build
cd gpy-agent
cargo clean
cargo build --release

# Run formatter benchmarks
cargo bench --bench formatter_bench
```

### 2. Record the budget

`benchmarks/perf-budgets.json` holds only what a script reads:

```json
{
  "budgets": {
    "ipc_latency": {
      "target_ms": 1.0,
      "max_ms": 1.5,
      "description": "IPC roundtrip latency budget"
    },
    "memory_usage": {
      "target_mb": 15.0,
      "max_mb": 50.0,
      "description": "Agent memory usage budget (RSS)"
    }
  },
  "regression_thresholds": {
    "measurement_variance_percent": 15.0
  }
}
```

`max_ms` (or `max_mb`) is the enforced limit; `target_ms` is the goal and is
documentation only. `measurement_variance_percent` is the tolerance both
scripts add on top of every `max_*`. `scripts/bench.sh --ci` and
`scripts/ci-bench.sh` read every key they use in one `jq` call
(`scripts/lib/bench-common.sh`) and exit 2, naming the problem, when `jq`, the
file or a key is missing; they never substitute a default. Set `PERF_BUDGETS_FILE`
to point either script at another file. Add a budget only together with the
benchmark that measures it.

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

`scripts/bench.sh --ci` enforces these budgets (plus the tolerance above), each
against the matching Criterion benchmark:

| Metric | Budget key | Benchmark |
|--------|------------|-----------|
| IPC Latency | `ipc_latency` | `ipc_roundtrip` |
| Git Status (cold) | `git_status_cold` | `git_status_cold` |
| Cache Write | `cache_write` | `cache_write_new_entry` |
| Cache Read | `cache_read` | `cache_read_hit` |
| Config Load | `config_validation` | `config_load_from_file` |
| Theme Manager Load | `theme_manager_load` | `theme_manager_load` |
| Theme Export Write | `theme_export_write` | `theme_export_write` |
| Version Command Spawn | `version_command_spawn` | `version_command_spawn`, stub command |
| Language Detection | `language_detection` | `lang_detect_5k` |

`scripts/ci-bench.sh` enforces the prompt-render budgets (`prompt_render*`),
the script-segment budgets (`script_segments_bash`, `script_segments_zsh`,
`script_segments_fish`, the shared `script_segments_shared` for every plugin
segment of a prompt due at once, and `script_segment_scan`), `git_status_cold`
on a synthetic repository, and the agent's resident memory (`memory_usage`,
`memory_long_running`); see [benchmarking.md](benchmarking.md). The
script-segment budgets are what plugin segments *add* to a prompt (paired
differences), not a whole render.

## Detecting Regressions

### CI Integration

`pr-gate.yml`'s `perf` job (`Performance budgets`) runs `./scripts/bench.sh --ci`
and is part of `gate-result`, so a Rust micro-benchmark over its budget blocks
the merge; `bench.sh --ci` also fails when two bench targets register the same
benchmark ID. The base-vs-head canary (`scripts/perf-canary.sh`, benchmarks
`canary_ipc_roundtrip` and `canary_git_status_cold`) and the strict
script-segment deadline run in the same job as advisory steps. `release.yml`'s
`validate` job runs `just bench-ci`, which adds the end-to-end budgets
(`ci-bench.sh`), so an exceeded budget blocks a release.

**Manual CI-mode benchmarks:**
You can also run benchmarks with budget enforcement locally:
```bash
./scripts/bench.sh --ci
# Exit code 0: All benchmarks within budget
# Exit code 1: One or more budgets exceeded, or cargo bench failed
# Exit code 2: jq, the baseline file or a budget key is missing
```

### Manual Comparison

Compare current results to baseline for deeper analysis:

```bash
# Run current benchmarks
cargo bench --bench formatter_bench > current-results.txt

# Or read the budgets
jq '.budgets' benchmarks/perf-budgets.json
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
- Increase budget tolerance in `benchmarks/perf-budgets.json`
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
- [hyperfine](https://github.com/sharkdp/hyperfine) - Shell benchmark tool used by `scripts/ci-bench.sh`

## See Also

- `scripts/bench.sh --ci` - Rust micro-benchmark budgets
- `gpy-agent/benches/` - Rust microbenchmarks
- [Architecture](../architecture.md) - System design and performance considerations

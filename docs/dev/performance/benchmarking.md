# Performance Benchmarking Methodology

This document defines the standard benchmark workflow so results are repeatable and comparable over time.

The goal of GPY benchmarking is diagnostic, not marketing. We use these runs to compare GPY against itself over time, catch regressions, and validate improvements under controlled conditions. Absolute timings will vary materially by machine, filesystem, thermal state, OS background activity, and toolchain details.

For day-to-day development, the repository also provides `just perf-canary`.
This runs a small same-machine comparison between `HEAD` and the last pushed
commit so relative regressions are easier to spot while they are still easy to
attribute. It is a diagnostic early warning, not a replacement for the full
benchmark workflow below.

## Worst-Case Policy

When GPY uses `<10ms` language, it means `<10ms` for the user-visible
cached/instant-response path, not `<10ms` for fresh full computation in all
cases. Fresh git or language recomputation on large or noisy repositories may
take materially longer and must be communicated separately.

## Environment Requirements

- Close heavy background workloads (IDE indexing, large downloads).
- Run on AC power, not on battery saver.
- Ensure `socat`, `jq`, and `hyperfine` are installed (for shell and CI benchmarks).
- If `sccache` causes permission errors, disable it for benchmarks with:
  - `RUSTC_WRAPPER=`

## Repeatable Run Protocol

Use this protocol whenever you want numbers that are defensible and comparable across reruns:

1. Start from a clean repo state.
```bash
git status --short
```

2. Confirm the benchmark host is quiet.
- No Time Machine backup
- No Spotlight or IDE indexing spike
- No large downloads, package installs, or other benchmark/test jobs

3. Run benchmarks sequentially, never in parallel.
- Do not overlap `just check`, `cargo bench`, `./scripts/bench.sh`, repo cloning, or `hyperfine` runs.
- Treat any run that overlapped other heavy work as invalid for comparison.

4. Use fresh, known-clean benchmark fixtures for the real-world suite.
```bash
cd gpy-agent
BENCH_DIR=/tmp/gpy-benchmarks ./benchmarks/setup-repos.sh
```

5. Disable repo-local fsmonitor for benchmark fixtures before timing them.
```bash
for repo in /tmp/gpy-benchmarks/*; do
  git -C "$repo" config core.fsmonitor false
done
```

6. Record the machine context with every saved result.
- machine model / CPU
- OS and kernel version
- filesystem type if relevant
- whether backup/indexing activity was confirmed idle
- benchmark scope (`benchmark-mode` or `user-config`)

7. Re-run suspicious results before treating them as meaningful.
- If `System` time is unexpectedly large
- If variance is wide
- If a repo emits fsmonitor or filesystem errors
- If results materially disagree with targeted A/B measurements

## Standard Benchmark Workflow

Run benchmarks in this order and record the outputs:

1) CI budget enforcement (authoritative)
```bash
RUSTC_WRAPPER= just bench-ci
```

2) Shell end-to-end timing (prompt latency)
```bash
just bench-shell
```

3) Rust micro-benchmarks (agent internals)
```bash
RUSTC_WRAPPER= just bench-rust
```

4) Variance run (stability over time)
```bash
just bench-variance
```

> **Host sensitivity note**: `just bench-variance` exercises the real-world repository path and is
> sensitive to Spotlight indexing, Time Machine backups, filesystem buffer state, and other background
> activity. On some developer machines the results are too noisy to be actionable. If you see wide
> variance, confirm the host is idle (no backup/indexing activity) before drawing conclusions. Treat
> the variance run as a qualitative signal, not a regression gate; use `just bench-ci` for budget
> enforcement.

5) Plugin discovery overhead budgets (0/10/50 plugins)
```bash
(cd gpy-agent && RUSTC_WRAPPER= cargo test --test plugin_performance_tests)
```

Optional:
- Huge repo stress: `just bench-huge`
- Baseline comparison/update: `just bench-baseline`
- Zsh-specific: `just bench-zsh`
- Same-machine pre-push canary: `just perf-canary`

## End-to-End Prompt & Memory Benchmark

`scripts/ci-bench.sh` measures the *user-visible* path: the real Fish
`fish_prompt` implementation and the live `gpy-agent` daemon. It complements the
Rust micro-benchmarks in `scripts/bench.sh` and is run automatically as part of
`just bench-ci`.

### What it measures

| Scenario            | What it exercises                                  | Budget key (`tests/performance-baselines.json`) |
| ------------------- | -------------------------------------------------- | ----------------------------------------------- |
| warm prompt         | Agent up, instant cache fresh (steady-state hit)   | `prompt_render`                                 |
| cold cache          | Instant cache cleared before every render          | `prompt_render_cold`                            |
| stale cache         | Instant cache aged (serve-stale path)              | `prompt_render_stale`                           |
| agent unavailable   | Daemon stopped (graceful fallback)                 | `prompt_render_degraded`                        |
| large repository    | Cold git scan on a 1,500-file synthetic repo       | `git_status_cold`                               |
| startup RSS         | Daemon RSS right after it becomes responsive       | `memory_usage`                                  |
| long-running RSS    | Peak daemon RSS under sustained prompt load        | `memory_long_running`                           |

### How it stays honest

- The prompt scenarios load the actual project init path (`fish/core/init.fish`)
  and call the real `fish_prompt`, with the checkout's `fish/functions` forced to
  the front of `fish_function_path` so a globally installed GPY can never be
  measured by mistake.
- Latency is reported as the **amortized per-render** cost: an in-process loop of
  renders is timed and divided by the render count, isolating render work from
  one-time Fish startup and GPY init. (macOS `date` has no `%N`, so amortization
  is the portable way to get per-render resolution.)
- The memory scenario starts a live daemon, confirms its PID and that its socket
  reports `Running and Responding`, samples RSS, then shuts it down cleanly.
- A broken measurement — non-zero exit, empty prompt output, unparseable timing,
  zero/empty RSS, or an unresponsive daemon — is a **hard failure in every mode**
  and can never be silently reported as a passing zero. Only a *valid* result
  that exceeds its budget is downgraded to a warning in local mode.
- All work happens in an isolated sandbox (`XDG_CACHE_HOME`, socket, and
  repositories under a temp dir), so it never touches your real cache or agent.

### Reproducible local commands

```bash
# Full run (budget overages are warnings locally; broken measurements still fail)
./scripts/ci-bench.sh

# Exactly what CI runs (budget overages fail; equivalent to the ci-bench step of
# `just bench-ci`)
CI=true ./scripts/ci-bench.sh --ci

# Faster smoke run while iterating on the harness itself
GPY_BENCH_RENDER_ITERS=20 GPY_BENCH_RUNS=3 GPY_BENCH_WARMUP=1 \
  GPY_BENCH_LONG_ITERS=8 ./scripts/ci-bench.sh
```

Tunable knobs (all optional): `GPY_BENCH_RENDER_ITERS` (renders per timed run,
default 100), `GPY_BENCH_RUNS` / `GPY_BENCH_WARMUP` (hyperfine runs/warmups),
`GPY_BENCH_LONG_ITERS` (long-running RSS iterations). A markdown summary is
written to `bench-results/prompt-memory-report.md`.

## Benchmark Scopes

GPY uses multiple benchmark classes. Keep their claims separate:

- `just bench-ci`
  Measures budget compliance for internal hot paths and shell integration,
  including the end-to-end prompt-latency and agent-memory budgets from
  `scripts/ci-bench.sh`. Use this to answer "did we violate our prompt-performance budgets?"
- `just bench-rust`
  Measures isolated agent internals (formatter, cache, IPC, git, language detection). Use this to localize regressions inside the Rust code.
- `just bench-variance`
  Measures stability over repeated runs on the standard synthetic large-repo fixture. Use this to check variance and identify noisy environments.
- `gpy-agent/benchmarks/run.sh`
  Measures the real-world `gpy-agent oneshot git --format json` path on pinned repositories.
  Default scope is `BENCH_SCOPE=benchmark-mode`, which adds `--benchmark-mode` and skips config/theme file I/O so results remain comparable to prior git-path baselines.
  Use `BENCH_SCOPE=user-config` only when you explicitly want full CLI/config behavior included.

Competitive or public-facing performance claims should cite the real-world suite, name the scope used, and describe the host conditions. Do not present a single local run as a universal user expectation.

## Recording Results

- Save raw outputs in a timestamped file under `docs/dev/performance/bench-results/`.
- Update `tests/performance-baselines.json` only when the change is intentional and verified.
- When recording the real-world suite, also capture:
  - `BENCH_SCOPE`
  - OS/kernel version
  - whether backup/indexing workloads were active
- If a budget exceeds, open a GitHub issue with:
  - benchmark name
  - measured value vs budget
  - reproduction command and environment

## Enabling Mechanisms for the `<10ms` Cached/Instant-Response Claim

The `<10ms` user-visible target is met by the following architectural mechanisms, not by raw computation speed:

1. **Instant cache** — On every prompt render, the agent first reads a stale filesystem-backed cache
   entry. If valid, it returns the cached result in `<1ms` while scheduling a background refresh.
   This is the primary path that makes `<10ms` achievable even for large repositories.

2. **Background refresh** — After returning the cached result, the agent runs the real git/language
   computation asynchronously. The updated value is stored for the next prompt.

3. **IPC hot path** — For prompts that hit the live in-memory cache, roundtrip IPC latency is
   `<0.2ms` (validated by `just bench-ci`).

If any of these mechanisms are bypassed (e.g. cold agent start, cache miss with no stale entry,
or `--no-cache` flag), the user-visible latency is bounded by fresh git/language computation time,
which can be `>50ms` on large or noisy repositories.

## Notes

- `just bench-ci` writes `bench-results.json` in the repo root.
- Use `./scripts/perf-baseline.sh --compare` for budget comparisons if needed.

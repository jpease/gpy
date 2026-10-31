# Performance Benchmarking Methodology

This document defines the standard benchmark workflow so results are repeatable and comparable over time.

These runs compare GPY against itself over time, catch regressions, and validate improvements under controlled conditions. Absolute timings vary by machine, filesystem, thermal state, OS background activity, and toolchain details.

For day-to-day development, the repository also provides `just perf-canary`.
This runs a small same-machine comparison between `HEAD` and the last pushed
commit so relative regressions are easier to spot while they are still easy to
attribute. It is a diagnostic early warning, not a replacement for the full
benchmark workflow below.

To compare GPY against Starship on pinned repositories and tool versions, use `just bench-compare`; its method and fairness rules are in [`benchmarks/compare-starship/README.md`](../../../benchmarks/compare-starship/README.md), and the published results are in [GPY vs Starship performance](../../user/performance-vs-starship.md).

## Worst-Case Policy

The cached prompt budget (`prompt_render` in `tests/performance-baselines.json`:
20 ms target, 50 ms max) applies to the user-visible cached/instant-response
path, not to fresh full computation in all cases. Fresh git or language recomputation on large or noisy repositories may
take longer and must be communicated separately.

## Environment Requirements

- Close heavy background workloads (IDE indexing, large downloads).
- Run on AC power, not on battery saver.
- Ensure `socat`, `jq`, and `hyperfine` are installed (for shell and CI benchmarks).
- If your configured compiler cache (`rustc-wrapper`) causes errors, disable it for benchmarks with:
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
- If results disagree with targeted A/B measurements

## Standard Benchmark Workflow

Run benchmarks in this order and record the outputs:

1) CI budget enforcement (authoritative)
```bash
RUSTC_WRAPPER= just bench-ci
```

2) Rust micro-benchmarks (agent internals)
```bash
RUSTC_WRAPPER= just bench-rust
```

3) Plugin discovery overhead (0/10/50 plugins; part of step 2, alone when
   checking the [plugin budgets](plugin-budgets.md))
```bash
(cd gpy-agent && RUSTC_WRAPPER= cargo bench --bench plugin_bench)
```

Optional:
- Same-machine canary against the last pushed commit: `just perf-canary` (CI runs it against the PR base in the non-gating `perf` job of `pr-gate.yml`, together with the budgets in `tests/performance-baselines.json`)

## End-to-End Prompt & Memory Benchmark

`scripts/ci-bench.sh` measures the *user-visible* path: the real Fish
`fish_prompt`, Bash `PS1` and Zsh `PROMPT` renders, plugin script segments
through the real adapters, and the live `gpy-agent` daemon. It complements the
Rust micro-benchmarks in `scripts/bench.sh` and is run automatically as part of
`just bench-ci`.

### What it measures

| Scenario            | What it exercises                                  | Budget key (`tests/performance-baselines.json`) |
| ------------------- | -------------------------------------------------- | ----------------------------------------------- |
| warm prompt         | Agent up, instant cache fresh (steady-state hit)   | `prompt_render`                                 |
| cold cache          | Instant cache cleared before every render          | `prompt_render_cold`                            |
| stale cache         | Instant cache aged (serve-stale path)              | `prompt_render_stale`                           |
| agent unavailable   | Daemon stopped (graceful fallback)                 | `prompt_render_degraded`                        |
| bash prompt | Agent up; bash/gpy.bash sourced once, __gpy_precmd + ${PS1@P} per render | `prompt_render_bash` |
| zsh prompt | Agent up; zsh/gpy.zsh sourced once, precmd_functions + ${(%)PROMPT} per render | `prompt_render_zsh` |
| script segments | What plugin script segments add to a prompt in each shell (see below) | `script_segments_bash`, `script_segments_zsh`, `script_segments_fish`, `script_segments_shared`, `script_segment_scan` |
| large repository    | Cold git scan on a 1,500-file synthetic repo       | `git_status_cold`                               |
| startup RSS         | Daemon RSS right after it becomes responsive       | `memory_usage`                                  |
| long-running RSS    | Peak daemon RSS under sustained prompt load        | `memory_long_running`                           |

### Measurement safeguards

- The prompt scenarios load the actual project init path (`fish/core/init.fish`)
  and call the real `fish_prompt`, with the checkout's `fish/functions` forced to
  the front of `fish_function_path` so a globally installed GPY can never be
  measured by mistake.
- Latency is reported as the **amortized per-render** cost: an in-process loop of
  renders is timed and divided by the render count, isolating render work from
  one-time Fish startup and GPY init. (macOS `date` has no `%N`, so amortization
  is the portable way to get per-render resolution.)
- The memory scenario starts a live daemon, confirms its PID and that its socket
  reports `state: running`, samples RSS, then shuts it down cleanly.
- A broken measurement — non-zero exit, empty prompt output, unparseable timing,
  zero/empty RSS, or an unresponsive daemon — is a **hard failure in every mode**
  and can never be silently reported as a passing zero. Only a *valid* result
  that exceeds its budget is downgraded to a warning in local mode.
- All work happens in an isolated sandbox (`XDG_CACHE_HOME`, socket, and
  repositories under a temp dir), so it never touches your real cache or agent.

### Script segments

The scenario renders a fixture plugin (`gpy-bench`, written into the sandbox)
through the agent's real theme export and each shell's real adapter:
`bash/core/g_segments.bash`, `zsh/core/script.zsh`, `fish/core/script.fish` and
the shared `g_runner.bash`. Its segments are `bench-user`, a username-like
segment cached for the session (`ttl_secs = 3600`); `bench-cwd`, which runs on
every command (`ttl_secs = 0`, input `pwd`); and `bench-scan`, whose
`require_files` never matches. A sandboxed agent of its own renders the only
built-in segment, `directory`, in every config. Each variant is paired with
`base`, the same prompt without plugin segments:

| Variant | Enabled after `directory` | What a due prompt pays | Budget key |
|---|---|---|---|
| `hit` | `bench-user` | a cache hit and its pill | per shell |
| `g` | `bench-user`, `bench-cwd` | a hit plus one uncached run | per shell |
| `idle` (Zsh, Fish) | as `g`, no command | an empty Enter: nothing runs (a Fish repaint calls only the `fish_prompt` function, a subset of this) | per shell |
| `expiry` | as `g`, `[plugin.bench-user] ttl_secs = 0` | every plugin segment due at once: two runs in one batch | `script_segments_shared` |
| `scan` | `bench-scan` | one scan child that misses, no run | `script_segment_scan` |
| `g-busybox` (Linux, when `busybox` is installed) | as `g`, `__gpy_script_sh` set to busybox | as `g`, under Alpine's `sh` | per shell |

`g` against `hit` is also reported, without a budget: that is the cost of one
uncached run. Shells: the first Bash 4.4 or later (`find_bench_bash`), `bash3`
(`/bin/bash` when it is older than 4.4, i.e. macOS 3.2, or
`GPY_BENCH_BASH3`), Zsh and Fish. The `bash3` rows go through the DEBUG trap
that Bash 3.2 to 4.3 installs once a script segment is listed, so its cost is
part of what they measure; the trap's own +5.94 µs per user command is the
spike's number (see below). Linux CI has no Bash older than 4.4, so `bash3`
runs on macOS only.

Method, after the spike #1006 harness `bench`:

- Every round visits every config in rotated order (round *r* starts at
  config *r* mod *k*) and takes `GPY_BENCH_G_PER_ROUND` samples of each
  (default 3). Round 0 is a warmup and is discarded; `GPY_BENCH_G_ROUNDS`
  rounds follow (default 5).
- A sample is a fresh shell with a clean environment (`env -i`). It sources
  the integration, draws the first prompt, then draws `GPY_BENCH_G_ITERS`
  more (default 20), each after a simulated command. Per prompt =
  (median of those samples − median of 0-prompt samples) / ITERS, so startup
  and the first prompt cancel out. Bash counts `\#` per top-level command,
  so its body is unrolled one line per prompt.
- A variant's result is the median, and IQR, over the rounds of its
  difference to `base` in the same round. The budget check uses that median
  (plus `measurement_variance_percent`).
- Load1 is recorded before and after every round.
  `bench-results/script-segments.tsv` keeps the per-round values.
- Preflight: before timing, each config draws three prompts. The scripts
  print their PID, so the run hard-fails unless every segment ran or hit
  exactly as its variant claims. A skipped script can never pass as cheap.

Canary check: `GPY_BENCH_G_DELAY=0.05 ./scripts/ci-bench.sh --ci` makes
`bench-cwd` sleep 50 ms per run, still under its 500 ms timeout. Every
variant that runs it then exceeds its budget, and the script exits 1.

The deadline itself (T + 50 ms grace + 30 ms at p99, with a hung and a
TERM-ignoring script) is a test, not a benchmark:
`tests/bash/script_segment_deadline.test.bash`. That p99 bound is the
quiet-host acceptance of ADR-0011 Spike 2; shared CI runners miss it on load
alone (macOS runners at load1 ~3: Bash 5 p50 206 ms, p99 286 ms). So the test
has two modes, set by `GPY_STRICT_DEADLINE`:

- `1` (the default outside CI): the strict p99 bound fails the test.
- `0` (the default when `CI` is set, so in the gating shell suite): the test
  prints the p99 as a `NOTE` and fails only on a real hang, any prompt over
  T + 500 ms. The hard checks hold in both modes: no prompt before T, the fast
  segment shown, both hung ones hidden, no script process left.

The non-gating Performance job in `.github/workflows/pr-gate.yml` runs the
test with `GPY_STRICT_DEADLINE=1` on ubuntu, writes the output to the step
summary and uploads it as `deadline-strict.txt`. Locally, on a quiet machine,
`bash tests/bash/script_segment_deadline.test.bash` runs strict.

Numbers and their sources:

- **Re-measured here** (`ci-bench.sh` defaults, 2026-10-26, macOS 27.0
  arm64 M4 Max, load1 median 4.8, not a quiet machine). Values are ms per due
  prompt, the median paired difference (IQR):

  | | Bash 5.3 | Bash 3.2 | Zsh 5.9 | Fish 4.9 |
  |---|---|---|---|---|
  | `hit` | +1.77 (1.77–1.81) | +1.81 (1.81–2.16) | +0.90 (0.88–1.00) | +0.63 (0.57–0.72) |
  | `g` | +10.16 (10.12–10.27) | +11.69 (11.57–11.72) | +15.26 (14.88–16.31) | +13.90 (13.73–13.97) |
  | `g` vs `hit` (one uncached run) | +8.39 | +9.66 | +14.39 | +13.19 |
  | `expiry` | +10.96 | +12.17 | +16.34 | +14.24 |
  | `scan` | +1.81 | +2.36 | +0.77 | +7.81 |
  | `idle` | – | – | +1.65 | +0.27 |

  Interpreter: `/bin/bash` (the export's macOS choice). Zsh and Fish pay one
  Bash child per run, about 5 ms on this host, and Fish pays another for the
  scan.
- **Reused from the PoC and spike #1006, not re-run:** the PoC built-in
  renders (35.88 / 13.13 / 20.67 ms) and the PoC G hit (1.5 to 2.8 ms) and
  one-process run (5.8 to 8.7 ms). Also: the runner's cost over
  `$(/bin/sh script)` (+0.67 to +2.41 ms on macOS, +0.4 to +0.9 ms on Linux),
  the clean environment (+0.07 to +0.30 ms), the scan child on Linux (+0.6 to
  +0.9 ms), and the Bash 3.2 DEBUG trap (+5.94 µs per command, P4). The
  benchmark measures the whole adapter path, not the runner against a bare
  `$( )`, so those figures are not re-derived here.
- **Linux and WSL:** not measured locally. The Linux figures come from CI:
  `release.yml`'s `validate` job (ubuntu-latest; `/bin/sh` is dash) runs
  `just bench-ci`, and with it this scenario. Busybox rows appear only where
  `busybox` is installed, which `.github/actions/install-shells` does not do
  today. No workflow runs `ci-bench.sh` under WSL (`wsl-smoke.yml` runs the
  smoke and contract suites only), so there are no WSL figures yet.

The budgets are set from the macOS run: `target_ms` ≈ the measured value,
`max_ms` ≈ 2.5× the target. That leaves room for shared runners, yet a
plugin that adds more than `max_ms` fails. Target change for review: a
`ttl_secs = 0` segment adds about 10 to 16 ms per command on macOS, more than
the Zsh render target (`prompt_render_zsh`, 13 ms). It stays a separate,
additive budget and does not raise the prompt targets.

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
`GPY_BENCH_LONG_ITERS` (long-running RSS iterations), and for script segments
`GPY_BENCH_G_ROUNDS` (5), `GPY_BENCH_G_PER_ROUND` (3), `GPY_BENCH_G_ITERS`
(20), `GPY_BENCH_BASH3` (a Bash older than 4.4) and `GPY_BENCH_G_DELAY` (the
canary check above). A markdown summary is written to
`bench-results/prompt-memory-report.md`; the script-segment rounds go to
`bench-results/script-segments.tsv`.

## Benchmark Scopes

GPY uses multiple benchmark classes. Keep their claims separate:

- `just bench-ci`
  Measures budget compliance for internal hot paths and shell integration,
  including the end-to-end prompt-latency and agent-memory budgets from
  `scripts/ci-bench.sh`. Use this to answer "did we violate our prompt-performance budgets?"
- `just bench-rust`
  Measures isolated agent internals (formatter, cache, IPC, git, language detection). Use this to localize regressions inside the Rust code.
- `gpy-agent/benchmarks/run.sh`
  Measures the real-world `gpy-agent oneshot git --format json` path on pinned repositories.
  Default scope is `BENCH_SCOPE=benchmark-mode`, which adds `--benchmark-mode` and skips config/theme file I/O so results remain comparable to prior git-path baselines.
  Use `BENCH_SCOPE=user-config` only when you explicitly want full CLI/config behavior included.

Competitive or public-facing performance claims should cite the real-world suite, name the scope used, and describe the host conditions. Do not present a single local run as a universal user expectation.

## Recording Results

- Attach raw outputs to the pull request that changes performance.
- Update `tests/performance-baselines.json` only when the change is intentional and verified.
- When recording the real-world suite, also capture:
  - `BENCH_SCOPE`
  - OS/kernel version
  - whether backup/indexing workloads were active
- If a budget exceeds, open a GitHub issue with:
  - benchmark name
  - measured value vs budget
  - reproduction command and environment

## Mechanisms Behind the Cached Prompt Budget

The `prompt_render` budget is met by the following architectural mechanisms, not by raw computation speed:

1. **Instant cache** — On every prompt render, the agent first reads a stale filesystem-backed cache
   entry. If valid, it returns the cached result (`cache_read` budget: 1 ms target) while scheduling a background refresh.
   This is the primary path that keeps large repositories inside the budget.

2. **Background refresh** — After returning the cached result, the agent runs the real git/language
   computation asynchronously. The updated value is stored for the next prompt.

3. **IPC hot path** — For prompts that hit the live in-memory cache, roundtrip IPC latency stays within the
   `ipc_latency` budget (1 ms target, 1.5 ms max; 0.037 ms measured on a dev machine, 2026-05-15).

If any of these mechanisms are bypassed (e.g. cold agent start, cache miss with no stale entry,
or `--no-cache` flag), the user-visible latency is bounded by fresh git/language computation time,
which can be `>50ms` on large or noisy repositories.

## Notes

- `just bench-ci` writes `bench-results.json` in the repo root.

# 2026-05-14 Single-Flight Cache Misses and Async Language Refresh

## Scope

This run validated two targeted prompt-path changes:

1. Move language refresh triggered by Git filesystem events off the synchronous Git event path.
2. Coalesce duplicate concurrent cache misses for a repository so only one expensive detection job runs per key.

## Environment

- Host: local macOS development machine
- Branch under test: `codex/perf-coalesce-refresh`
- Baseline: `origin/main` at `8680eed`
- Tool: `hyperfine`
- Runs: 15 measured runs, 3 warmup runs
- Fixture: synthetic Git repository under `/tmp` with 1,200 Rust source files and 200 Markdown files committed

The benchmark measured same-machine relative performance. Absolute timings are not portable to other machines.

## Harness

Two temporary Rust harness crates were built in release mode:

- `/tmp/gpy-perf-main`, depending on `/tmp/gpy-main-perf/gpy-agent`
- `/tmp/gpy-perf-current`, depending on this branch's `gpy-agent`

The harness exposed two commands:

- `git-event <repo>`: constructs an `AgentContext`, sends a Git `DebouncedEvent`, and times `handle_file_event`.
- `language-concurrent <repo>`: constructs one `LanguageHandler`, sends 8 concurrent `LanguageDetect` requests for the same repo, and measures wall time.

## Results

### Git Event Path

Command:

```bash
hyperfine --warmup 3 --runs 15 \
  --command-name main-git-event '/tmp/gpy-perf-main/target/release/gpy-perf-main git-event /tmp/gpy-perf-repo.h3Xs32' \
  --command-name current-git-event '/tmp/gpy-perf-current/target/release/gpy-perf-current git-event /tmp/gpy-perf-repo.h3Xs32'
```

| Variant | Mean | Std dev | Min | Max | Relative |
|:---|---:|---:|---:|---:|---:|
| `origin/main` | 141.1 ms | 27.3 ms | 62.5 ms | 195.6 ms | 1.00x |
| Branch | 27.2 ms | 3.5 ms | 23.4 ms | 32.9 ms | 5.19x faster |

### Concurrent Language Cache Misses

Command:

```bash
hyperfine --warmup 3 --runs 15 \
  --command-name main-lang-concurrent '/tmp/gpy-perf-main/target/release/gpy-perf-main language-concurrent /tmp/gpy-perf-repo.h3Xs32' \
  --command-name current-lang-concurrent '/tmp/gpy-perf-current/target/release/gpy-perf-current language-concurrent /tmp/gpy-perf-repo.h3Xs32'
```

| Variant | Mean | Std dev | Min | Max | Relative |
|:---|---:|---:|---:|---:|---:|
| `origin/main` | 136.7 ms | 8.7 ms | 126.7 ms | 163.6 ms | 1.00x |
| Branch | 57.4 ms | 1.8 ms | 54.5 ms | 61.2 ms | 2.38x faster |

## Interpretation

The Git event result confirms that synchronous language detection was a high-probability latency source on the file-event path. Moving that refresh to background work reduced measured event handling time by about 81% on this fixture while still updating the instant language cache and notifying clients after the refresh completes.

The concurrent language result confirms that duplicate same-repository cache misses were doing redundant detector work. The single-flight helper reduced wall time by about 58% for 8 simultaneous requests and also reduced variance.

## Caveats

- The fixture is synthetic and intentionally language-heavy.
- The harness measures internal paths directly rather than shell end-to-end prompt rendering.
- The Git event benchmark on `origin/main` reported statistical outliers; the gap was large enough to keep the result useful, but future work should also run the standard benchmark suite before changing permanent baselines.

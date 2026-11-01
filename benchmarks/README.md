# Benchmarks

Five tools answer five different questions. Budgets live in
[`perf-budgets.json`](perf-budgets.json); methodology and how to read results
are in [`docs/dev/performance/benchmarking.md`](../docs/dev/performance/benchmarking.md).

| Tool | Question it answers | Run | In CI |
|------|---------------------|-----|-------|
| `scripts/bench.sh --ci` | Do the Rust micro-benchmarks (IPC, git status, cache, config, theme, language detection) stay within their budgets? | `just bench-ci` | PR gate (`perf`) and release |
| `scripts/ci-bench.sh` | Does a real fish, Bash or Zsh prompt render, and the live daemon's memory, stay within the end-to-end budgets? | `just bench-ci` | Release |
| `scripts/perf-canary.sh` | Did this change slow a canary benchmark against the base commit, on the same machine? | `just perf-canary` | PR gate (`perf`, advisory) |
| `benchmarks/real-repos/run.sh` | How long does `gpy-agent oneshot git` take on pinned real repositories, compared with a saved run? | `just bench-repos` | No |
| `benchmarks/compare-starship/run.sh` | How does GPY's prompt latency compare with Starship's on the same repositories? | `just bench-compare` | No |
| `cargo bench` | Where inside the agent did a regression land? | `just bench-rust` | Through `bench.sh` |

`bench.sh` and `ci-bench.sh` exit 0 when every budget holds, 1 on an exceeded
budget or a failed benchmark, and 2 when `jq`, the budget file or a budget key
cannot be read. `PERF_BUDGETS_FILE` points either at another budget file.

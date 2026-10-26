# GPY Agent Performance Benchmarks

Reproducible performance benchmarks using real-world repositories.

These benchmarks are primarily a maintenance and diagnostics tool for GPY itself. We use them to compare one GPY revision to another, catch regressions, and validate improvements under controlled conditions. They are not guarantees about the exact prompt latency a user will see on a different machine.

## Quick Start

```bash
# 1. Setup benchmark repositories in a clean local path
BENCH_DIR=/tmp/gpy-benchmarks ./benchmarks/setup-repos.sh

# 2. Disable fsmonitor in benchmark fixtures
for repo in /tmp/gpy-benchmarks/*; do
  git -C "$repo" config core.fsmonitor false
done

# 3. Build the agent
cargo build --release

# 4. Run benchmarks (benchmark scope by default)
BENCH_DIR=/tmp/gpy-benchmarks ./benchmarks/run.sh

# 5. Save baseline for regression testing
BENCH_DIR=/tmp/gpy-benchmarks ./benchmarks/run.sh --baseline

# 6. Compare against baseline (after making changes)
BENCH_DIR=/tmp/gpy-benchmarks ./benchmarks/run.sh --compare

# Optional: measure full user-configured oneshot behavior instead
BENCH_DIR=/tmp/gpy-benchmarks BENCH_SCOPE=user-config ./benchmarks/run.sh
```

Run this suite on a quiet machine and never in parallel with other heavy tests or benchmarks.

## Benchmark Repositories

We use real-world open-source repositories of varying sizes:

| Repository | Size | Files | Description |
|------------|------|-------|-------------|
| **tiny-express** | ~234 files | ~0.02 MB index | Express.js web framework |
| **small-redis** | ~1.4k files | ~0.13 MB index | Redis in-memory database |
| **medium-kubernetes** | ~24k files | ~3.27 MB index | Kubernetes orchestrator |
| **large-tensorflow** | ~36k files | ~4.50 MB index | TensorFlow ML framework |

Exact timings depend on the host machine and environment.

## Why Real-World Repos?

Real-world repositories have characteristics that synthetic benchmarks miss:
- Diverse file types (.c, .h, .rs, .js, etc.)
- Deep directory structures (10+ levels)
- Real commit history
- Varied file sizes
- Complex git metadata
- .gitignore patterns
- Submodules and worktrees

## Configuration

Environment variables:

- `BENCH_DIR` - Where to store benchmark repos (default: `~/.cache/gpy-benchmarks`)
- `BENCH_RUNS` - Number of runs per benchmark (default: 5)
- `BENCH_SCOPE` - `benchmark-mode` (default, skips config/theme I/O) or `user-config`

## Scope

The real-world suite measures the `gpy-agent oneshot git --format json` path on real repositories.

- `BENCH_SCOPE=benchmark-mode`: adds `--benchmark-mode` and bypasses config/theme file I/O so the numbers reflect the git-status request path more directly.
- `BENCH_SCOPE=user-config`: measures full user-configured oneshot behavior, including config loading.

Use `benchmark-mode` for comparative performance claims and baseline tracking. Use `user-config` for troubleshooting or end-to-end CLI investigations.

## Repeatability Checklist

Before treating a run as comparison-grade:

- Confirm the machine is idle: no backups, indexing, package installs, or other benchmark/test jobs.
- Use freshly reset fixtures via `./benchmarks/setup-repos.sh`.
- Disable `core.fsmonitor` in the benchmark repos.
- Run the suite by itself.
- Record machine, OS/kernel, filesystem, and scope details with the result.

If a run shows unusually large `System` time or host warnings, rerun after fixing the environment before drawing conclusions.

## Baseline Format

The baseline file (`baseline.json`) stores performance data:

```json
{
  "timestamp": "2025-12-02T12:00:00Z",
  "git_commit": "<full commit sha>",
  "scope": "benchmark-mode",
  "kernel": "Darwin 24.1.0",
  "benchmarks": {
    "tiny-express":      { "mean": 0.0089, "stddev": 0.0002 },
    "small-redis":       { "mean": 0.0101, "stddev": 0.0003 },
    "medium-kubernetes": { "mean": 0.0245, "stddev": 0.0004 },
    "large-tensorflow":  { "mean": 0.0302, "stddev": 0.0005 }
  }
}
```

`mean` and `stddev` are in seconds and are written for all four repositories listed above. `--baseline` writes the file; `--compare` prints every repository and exits non-zero if the baseline has no entry for one of them (regenerate with `--baseline`). The values above are illustrative.

## Regression Testing

```bash
# Before making changes
./benchmarks/run.sh --baseline

# Make your changes
git checkout feature-branch
cargo build --release

# Check for regressions
./benchmarks/run.sh --compare
```

Output shows:
- `REGRESSION` - Performance degraded >10%
- `IMPROVEMENT` - Performance improved >10%
- `no change` - Within ±10%
- `MISSING from baseline` - No baseline entry for that repository (exit status 1)

## Maintenance

Benchmark repos are stored in `~/.cache/gpy-benchmarks` by default.

To update:
```bash
rm -rf ~/.cache/gpy-benchmarks
./benchmarks/setup-repos.sh
./benchmarks/run.sh --baseline  # Update baseline
```

The setup script is also safe to rerun without deleting the directory first. It will:
- fetch the pinned revision or branch head,
- hard-reset the working tree,
- remove untracked files,
- restore each benchmark repo to a clean state before measurement.

Note: on case-insensitive filesystems, some repositories may still report dirty paths after reset if upstream contains case-distinct filenames that collide locally. The old `large-linux` fixture hit that problem on macOS, so the current suite uses `large-tensorflow` instead. Verify cleanliness with `git -c core.fsmonitor=false status --short` before using any fixture for published comparisons.

## What Is Measured

The git segment runs a single `git status --porcelain=v2 --branch` subprocess
(branch, ahead/behind and file status in one call), so git's own fsmonitor,
untracked-cache and sparse-checkout support apply. GPY enables the untracked
cache for a repository unless the user has set `core.untrackedCache`.

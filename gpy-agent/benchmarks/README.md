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

These represent actual user workloads across a range of repository sizes, but the exact timings still depend on the host machine and environment.

## Why Real-World Repos?

Real-world repositories have characteristics that synthetic benchmarks miss:
- Diverse file types (.c, .h, .rs, .js, etc.)
- Deep directory structures (10+ levels)
- Real commit history
- Varied file sizes
- Complex git metadata
- .gitignore patterns
- Submodules and worktrees

These provide realistic performance measurements for actual user workloads.

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
  "timestamp": "2024-11-29T12:00:00Z",
  "git_commit": "9bcff3f",
  "scope": "benchmark-mode",
  "benchmarks": {
    "tiny-express": {
      "mean": 0.0089,
      "stddev": 0.0002
    },
    ...
  }
}
```

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
- 🔴 REGRESSION - Performance degraded >10%
- 🟢 IMPROVEMENT - Performance improved >10%
- ✓ No change - Within ±10%

## CI Integration

Add to your CI pipeline:

```yaml
- name: Performance regression test
  run: |
    ./benchmarks/setup-repos.sh
    cargo build --release
    ./benchmarks/run.sh --compare
```

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

## Historical Results

The table below is a historical benchmark snapshot, not a guaranteed current result. Re-run the suite on your benchmark host before citing these numbers publicly.
This snapshot predates the `large-tensorflow` swap and still used the old `large-linux` fixture.

**Historical Snapshot:** 2025-12-02 (1000 runs per test, native git v2 format, benchmark-mode scope)

| Repository | GPY | Starship | Difference | Status |
|------------|-----|----------|------------|--------|
| tiny-express | **25.4ms ± 0.9ms** | 27.3ms ± 1.4ms | **-6.96%** | 🟢 **Faster** |
| small-redis | **25.6ms ± 0.8ms** | 27.6ms ± 1.3ms | **-7.25%** | 🟢 **Faster** |
| medium-kubernetes | **33.1ms ± 1.4ms** | 35.0ms ± 1.1ms | **-5.43%** | 🟢 **Faster** |
| large-linux (historical) | **37.5ms ± 1.3ms** | 39.3ms ± 1.2ms | **-4.58%** | 🟢 **Faster** |

**CPU Time Breakdown:**
- User time: 6.8-16.7ms (scales with repo size)
- System time: 5.4-8.0ms (consistent overhead)

### Performance Characteristics

Treat the following as historical observations from a specific controlled benchmark host, not universal user-facing guarantees.

✅ **Strengths:**
- **Beats Starship**: 4.58-7.25% faster across all repo sizes (average: 6.06%, n=1000)
- **Highly scalable**: Sub-38ms performance even on 91k file repos
- **Predictable**: Consistent performance with tight variance (<5% CV)
- **Leverages native optimizations**: fsmonitor, untracked cache, sparse-checkout
- **Single subprocess**: Uses `git status --porcelain=v2 --branch`
- **Statistically significant**: All results p<0.001 with 1000 iterations

🎯 **Performance Leadership:**
- GPY now outperforms Starship despite using subprocess approach
- Native git optimizations provide better scalability than libgit2
- Efficient v2 format parsing minimizes post-subprocess overhead
- Average 6.06% performance advantage across all repository sizes (n=1000)

### Implementation Details

**Current architecture (Native Git):**
- Single `git status --porcelain=v2 --branch` call (combines branch + ahead/behind + file status)
- Parallel repo state checking (filesystem-based)
- Auto-enables untracked cache for repeat performance
- Functional parsing with structured result types

**Trade-offs:**
- ✅ Superior performance vs Starship (4-8% faster across all repo sizes)
- ✅ Leverages git's built-in features (fsmonitor, sparse-checkout, untracked cache)
- ✅ Native git optimizations outweigh subprocess overhead

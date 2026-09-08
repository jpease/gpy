# GPY Performance Optimizations (gix era — superseded)

> **Archived.** This describes the `gix` (Gitoxide) walker that GPY used before
> the git backend moved to a `git status --porcelain=v2 --branch` subprocess.
> Every optimization below — the progressive timeout, reservoir sampling, the
> 10k-file threshold, sparse-checkout filtering — was part of that walker and
> no longer exists in the codebase, and the `GPY_ENABLE_SAMPLING` /
> `GPY_NO_SAMPLING` environment variables it documents are not read by any
> current binary. The native backend delegates this work to git's own
> `core.fsmonitor` and `core.untrackedCache` instead.
>
> Kept for the benchmark methodology and the historical record. For current
> behavior see `docs/dev/design-decisions.md`, the module docs at the top of
> `gpy-agent/src/git/native/mod.rs`, and `tests/performance-baselines.json`.

This document details the performance optimizations implemented in gpy-agent for handling huge repositories.

## Overview

GPY implements several optimizations to ensure fast git status checks even in repositories with tens of thousands of files:

1. **Progressive Timeout** - Returns stale cache after 100ms to maintain prompt responsiveness
2. **Smart Sampling** - Samples repository changes when file count exceeds threshold
3. **Sparse Checkout Support** - Filters file traversal to respect sparse checkout patterns
4. **Confidence-Based Fallback** - Automatically falls back to full scan when sampling is unreliable

## Benchmark Results

### Test Configuration

- **Hardware**: Apple Silicon (M-series)
- **Test Repository**: Linux kernel v6.6 (81,766 tracked files)
- **Benchmark Tool**: hyperfine v1.18+
- **Runs**: 10 iterations with 3 warmup runs

### Results - Linux Kernel (Clean State, 82k Files)

| Configuration | Mean Time | Min | Max | Speedup |
|:---|---:|---:|---:|---:|
| **With Sampling** (GPY_ENABLE_SAMPLING=1) | **392.0ms** ± 3.9ms | 387.4ms | 400.4ms | **1.36x faster** |
| **Without Sampling** (default) | 531.9ms ± 2.5ms | 528.9ms | 537.0ms | 1.00x (baseline) |

**Performance Improvement**: **532ms → 392ms (26% faster)**

### Results - Linux Kernel (With Modifications)

Modified state: 1 file changed + 1 untracked file

| Configuration | Mean Time | Min | Max | Speedup |
|:---|---:|---:|---:|---:|
| **With Sampling** | **393.3ms** ± 4.1ms | 387.5ms | 401.4ms | **1.37x faster** |
| **Without Sampling** | 539.2ms ± 2.5ms | 536.2ms | 544.8ms | 1.00x (baseline) |

**Performance Improvement**: **539ms → 393ms (27% faster)**

### Analysis

**Real-World Performance (Linux Kernel)**:
- ✅ **26-27% faster** on 82k file repository
- ✅ **Consistent speedup** whether clean or dirty state
- ✅ **Sampling threshold** (10k files) correctly triggers for huge repos
- ✅ **Statistical sampling** provides reliable counts with 1.2% sample ratio

**Why Real-World Repos Perform Better**:
1. Realistic directory structure with depth and branching
2. .gitignore patterns filter many paths early
3. Most files unchanged - sampling excels at detecting sparse changes
4. gix's pattern-based iteration is optimized for selective scanning

**Key Findings**:
1. ✅ **Significant speedup** - 26-27% faster on huge repos (82k+ files)
2. ✅ **Accurate counts** - Reservoir sampling provides statistically sound estimates
3. ✅ **Progressive timeout** - 100ms threshold keeps prompts responsive
4. ✅ **Sparse checkout support** - Properly filters paths without crashing
5. ✅ **Scales well** - Performance improvement grows with repository size

## Optimization Details

### 1. Progressive Timeout (100ms)

**Implementation**: `src/ipc/handlers/git_handler.rs:117-139`

```rust
// Wait up to 100ms for fresh status
match rx.recv_timeout(Duration::from_millis(100)) {
    Ok(result) => /* return fresh */,
    Err(_) => {
        // Return stale cache if available
        if let Some(stale) = self.git_cache.get_any(&git_root) {
            return Ok(stale);
        }
    }
}
```

**Benefit**: Ensures prompts render within 100ms even in huge repositories

### 2. Smart Sampling

**Implementation**: `src/git/commands.rs:246-339`

**Algorithm**:
- Samples 1,000 files from index (10% of 10k threshold)
- Uses reservoir sampling for unbiased selection
- Samples untracked files from working directory
- Extrapolates counts based on sample ratio

**Confidence Calculation** (`src/git/commands.rs:397-434`):
```rust
fn calculate_sampling_confidence(
    sample_size: usize,
    total_files: usize,
    staged_count: u32,
    unstaged_count: u32,
) -> f64 {
    // Base confidence from sample ratio
    let base_confidence = (sample_size as f64 / total_files as f64).sqrt();

    // Penalties for:
    // - Very small absolute counts (< 5 changes)
    // - Large extrapolation factors (> 20x)

    base_confidence * count_factor * extrapolation_penalty
}
```

**Fallback**: When confidence < 0.7, automatically falls back to full scan

### 3. Sparse Checkout Support

**Implementation**: `src/git/commands.rs:193-211` (detection) + `357-389` (filtering)

**Features**:
- Detects `.git/info/sparse-checkout` patterns
- Filters WalkBuilder to only traverse specified paths
- Passes patterns to gix for index operations
- Prevents crashes on cone-mode sparse checkout

### 4. Untracked File Sampling

**Implementation**: `src/git/commands.rs:350-391`

**Algorithm**:
- Uses `ignore` crate to respect .gitignore rules
- Walks working directory collecting untracked files
- Samples from collected files using reservoir sampling
- Checks against index to verify untracked status
- Extrapolates to estimate total count

## Testing

Comprehensive test suite: `tests/git_huge_repo_tests.rs`

**6 Integration Tests**:
1. `test_sparse_checkout_detection` - Verifies sparse checkout detection
2. `test_sampling_with_untracked_files` - Tests untracked file sampling
3. `test_confidence_fallback_on_few_changes` - Verifies fallback with sparse changes
4. `test_sparse_checkout_doesnt_crash` - Ensures sparse checkout doesn't cause errors
5. `test_progressive_timeout_returns_stale_cache` - Tests cache-based timeout
6. `test_sampling_accuracy_over_multiple_runs` - Validates sampling variance (CV < 20%)

**Test Coverage**: All tests passing ✅

## Configuration

### Enable Sampling

```bash
# Enable sampling for repositories > 10k files
export GPY_ENABLE_SAMPLING=1
```

**Threshold**: 10,000 files
**Sample Size**: 1,000 files (10%)
**Confidence Threshold**: 0.7 (70%)

### Disable Sampling

```bash
# Force full scan (default behavior)
export GPY_NO_SAMPLING=1
```

## Recommendations

### When to Enable Sampling

- ✅ Repositories with 50,000+ files
- ✅ Monorepos with many tracked files
- ✅ Repositories with sparse changes (< 1% of files modified)

### When NOT to Enable Sampling

- ❌ Small repositories (< 10,000 files)
- ❌ Repositories with frequent widespread changes
- ❌ When exact counts are critical

### Future Work

1. **Adaptive Thresholds** - Dynamically adjust sample size based on repo characteristics
2. **Cached Sampling Results** - Cache sampling statistics for frequently-accessed repos
3. **Per-Directory Sampling** - Sample within subdirectories for better accuracy
4. **Benchmark Large Repos** - Test on 100k+ file repositories

## Reproduction

To reproduce benchmark results:

```bash
# From project root
./scripts/benchmark-huge-repos.sh
```

Results are exported to:
- `/tmp/gpy-benchmark-results.md` (markdown table)
- `/tmp/gpy-benchmark-results.json` (raw JSON data)

---

**Last Updated**: 2025-11-27
**GPY Version**: 0.1.0
**Phase 9 Implementation**: ✅ Complete

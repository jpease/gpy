# Performance Variance & <10ms Worst-Case Goal (gpy-dwo)

Historical investigation note: this document captures a pre-optimization snapshot from before the current language-detection fast path and CI benchmark suite. For current budgets and authoritative measurements, use `docs/dev/performance/benchmarking.md`, `docs/dev/performance/performance-baselines.md`, and `tests/performance-baselines.json`.

Decision update: GPY now interprets the `<10ms` goal as a user-visible
cached/instant-response target. It is no longer treated as a promise that fresh
full computation will complete within `<10ms` under worst-case repository or
host conditions.

## 📊 Performance Analysis

Measurements taken on a "huge repository" (10,000 files) using `scripts/variance-check.sh`.

### Cached Performance (Hot Agent)
Once the agent has initialized and cached Git status:

| Metric | Git Status IPC | Language Detection | Total Render Estimate |
|:---|:---|:---|:---|
| **Mean** | 0.1072 ms | 80.3638 ms | 80.4702 ms |
| **Min** | 0.0949 ms | 77.2700 ms | 77.3600 ms |
| **Max** | 0.1264 ms | 85.4800 ms | 85.5800 ms |
| **Stdev** | 0.0059 ms | 1.7676 ms | 1.7667 ms |
| **P95** | 0.1163 ms | 83.9200 ms | 84.0200 ms |

**Observations:**
- **Git Status** is successfully cached and serves in **<0.2ms**.
- **Language Detection** is the current bottleneck, taking **~80ms** even when "hot".
- **Variance** is very low (~2ms), indicating the logic is deterministic but slow.

### Worst-Case Performance (Cold Cache)
Initial request to a 10,000 file repository with a cold agent cache:

- **Latency**: >500ms (triggered client timeout).
- **Reason**: `hyperpolyglot` scans the entire directory tree synchronously during the IPC request.

---

## 🎯 Feasibility of <10ms Worst-Case

The target of <10ms under "worst conditions" is **achievable** but requires architectural changes:

### 1. Address the Language Bottleneck
Current language detection re-scans the directory on every request.
- **Finding**: The "Instant Cache" mechanism (`src/cache/instant_prompt.rs`) **only caches Git status**. It does not cache language detection results.
- **Result**: The prompt waits ~80ms for language detection even if Git status is instant.
- **Solution**: Implement a `DetectionCache` (similar to `GitStatusCache`) that persists results for a given `cwd`.
- **Impact**: Reduces mean "hot" latency from 80ms to **<1ms**.

### 2. "Instant Cache" for Cold Starts
The absolute worst case is the first render in a huge repo. No amount of optimization makes `git status` or `hyperpolyglot` scan 100k files in <10ms.
- **Solution**: GPY already has an "Instant Cache" (stale filesystem-backed cache).
- **Impact**: If the agent returns the *stale* result immediately while spawning a background scan, the worst-case (user-visible) latency remains **<5ms** (standard IPC + disk read).

---

## 🏁 Established Targets

Based on this investigation, we establish the following targets for GPY:

| Scenario | Target | Current | Status |
|:---|:---|:---|:---|
| **Hot Cache (Cached)** | < 2ms | ~80ms | 🔴 Bottleneck: Language |
| **Stale Cache (Async)** | < 10ms | ~80ms | 🔴 Bottleneck: Sync Detection |
| **Cold Cache (No data)** | < 50ms | > 500ms | 🔴 Bottleneck: Sync I/O |

## ✅ Recommendations
1. **Cache Language Detection**: Move detection results into a global cache.
2. **Background Language Refresh**: Use the file watcher to invalidate/refresh language results rather than doing it on-demand.
3. **Optimistic IPC**: If a scan is taking too long, return "Stale" or "Processing" rather than blocking.

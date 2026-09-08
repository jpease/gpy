# Caching Strategy

> **Archived.** This document describes historical caching strategies and may not reflect current implementations.

**Audience**: Contributors working on performance optimization or debugging cache behavior.

**Prerequisites**:
- Read [architecture.md](../dev/architecture.md) for system overview
- Read [modules.md](modules.md) for cache module references

This document explains GPY's caching architecture, the timing relationships between debouncing/cooldown/TTL, and how to tune cache behavior.

---

## Table of Contents

- [Overview](#overview)
- [Cache Policy System](#cache-policy-system)
- [Cache Layers](#cache-layers)
- [Timing Relationships](#timing-relationships)
- [Cache Invalidation](#cache-invalidation)
- [Configuration and Tuning](#configuration-and-tuning)
- [Debugging Cache Issues](#debugging-cache-issues)
- [Performance Characteristics](#performance-characteristics)

---

## Overview

### Why Cache?

GPY caches expensive operations to keep prompt rendering fast:

| Operation | Uncached | Cached | Improvement |
|-----------|----------|--------|-------------|
| Git status | 10-50ms | <1ms | **10-50x faster** |
| Language detection | 5-20ms | <1ms | **5-20x faster** |
| IPC round-trip | 1-2ms | N/A | Already fast |

**Goal**: Prompt renders in <50ms even with multiple segments enabled.

### Cache Architecture

GPY uses a **three-tier caching strategy**:

```
┌─────────────────────────────────────────────────────────────┐
│                    Timing Coordination                       │
│                                                              │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐     │
│  │  Debouncing  │  │   Cooldown   │  │     TTL      │     │
│  │   (100ms)    │  │   (150ms)    │  │ (30s/24h/5s) │     │
│  └──────────────┘  └──────────────┘  └──────────────┘     │
│       ▲                  ▲                   ▲              │
│       │                  │                   │              │
│       │                  │                   │              │
│  Prevents          Prevents            Expires old          │
│  redundant         redundant           cached data          │
│  signals           queries                                  │
└─────────────────────────────────────────────────────────────┘
         │                  │                   │
         ▼                  ▼                   ▼
┌─────────────────────────────────────────────────────────────┐
│                      Cache Layers                            │
│                                                              │
│  ┌────────────────┐  ┌──────────────────┐  ┌──────────────┐     │
│  │ GitStatusCache │  │ VersionStore     │  │  ThemeCache  │     │
│  │ policy TTL 30s │  │ policy TTL 24h   │  │  TTL: 5s     │     │
│  └──────────────┘  └──────────────┘  └──────────────┘     │
└─────────────────────────────────────────────────────────────┘
```

### Design Principles

1. **Cache aggressively**: Only re-compute when necessary
2. **Invalidate conservatively**: Prefer stale data over slow prompts
3. **Fail gracefully**: Cache miss should not break prompt
4. **Tune per-subsystem**: Different operations need different TTLs

---

## Cache Policy System

**Location**: `gpy-agent/src/cache/policy.rs`

GPY provides a unified `CachePolicy` abstraction that encapsulates timing parameters (TTL and cooldown) for different cache types. This ensures consistent timing semantics across all cache implementations.

### CachePolicy Structure

```rust
pub struct CachePolicy {
    ttl: Duration,      // Time-to-live: max age before stale
    cooldown: Duration, // Cooldown: min time before re-query
}
```

### Predefined Policies

The system provides three predefined policies tuned for different use cases:

#### 1. Git Status Policy

```rust
let policy = CachePolicy::git_status();
// TTL: 30 seconds
// Cooldown: 150ms
```

**Rationale**:
- **Short TTL (30s)**: Git repos change frequently during development
- **150ms cooldown**: Matches watcher throttle to prevent redundant queries after file watcher updates
- **Use case**: Caching `git status` results that change often

#### 2. Language Version Policy

```rust
let policy = CachePolicy::language_version();
// TTL: 24 hours
// Cooldown: 0ms
```

**Rationale**:
- **Long TTL (24h)**: Language versions change infrequently (only when upgrading tools)
- **No cooldown**: No file watcher involvement, no thrashing risk
- **Use case**: Caching expensive subprocess calls (`python --version`, etc.)

#### 3. Theme Policy

```rust
let policy = CachePolicy::theme();
// TTL: 5 seconds
// Cooldown: 100ms
```

**Rationale**:
- **Short TTL (5s)**: Enables hot-reload during theme development
- **100ms cooldown**: Prevents multiple reloads during multi-file theme edits
- **Use case**: Active - theme changes detected via file watcher trigger automatic reload

### Policy-Driven Caching

Caches can implement the `PolicyDriven` trait to declare their timing behavior:

```rust
pub trait PolicyDriven {
    fn policy(&self) -> CachePolicy;
}

impl PolicyDriven for GitStatusCache {
    fn policy(&self) -> CachePolicy {
        CachePolicy::git_status()
    }
}
```

### Compatibility Checking

The policy system validates that cooldown and debounce values are compatible:

```rust
let policy = CachePolicy::git_status(); // 150ms cooldown
let watcher_debounce = Duration::from_millis(100);

// Verify cooldown >= debounce for optimal performance
assert!(policy.is_compatible_with_debounce(watcher_debounce));
```

**Why this matters**: If cooldown < debounce, the cache may refresh before the debouncer even fires, defeating the purpose of debouncing.

### Benefits of Unified Policy System

1. **Single source of truth**: All timing parameters defined in one place
2. **Documented rationale**: Each policy explains why its values were chosen
3. **Type safety**: Compile-time guarantee that caches have timing parameters
4. **Testability**: Easy to test cache behavior with different policies
5. **Future extensibility**: Easy to add new cache types with appropriate policies

---

## Cache Layers

### Layer 1: Git Cache

**Location**: `gpy-agent/src/git/cache.rs`

**Purpose**: Cache git status results (branch, ahead/behind, file counts)

**Implementation**:
```rust
pub struct GitStatusCache {
    entries: Arc<Mutex<HashMap<PathBuf, CacheEntry>>>,
    policy: CachePolicy,
}

struct CacheEntry {
    status: RepositoryStatus,
    cached_at: Instant,
}
```

**Policy**:
- **TTL**: 30 seconds (from `CachePolicy::git_status()`)
- **Cooldown**: 150 ms (from `CachePolicy::git_status()`) after watcher events

**Cache Key**: Canonical repository root path (`/home/user/project`)

**Example**:
```rust
let cache = GitStatusCache::new();

if let Some(status) = cache.get(repo_path) {
    return Ok(status);
}

let status = load_repository_state(repo_path)?;
cache.set(repo_path, status.clone());
```

**Invalidation Triggers**:
1. TTL expiration (after 30 seconds)
2. File watcher detects `.git/HEAD`, `.git/index`, etc. changes
3. Explicit `invalidate(path)` call

### Layer 2: Language Cache

**Location**: `gpy-agent/src/language/cache.rs`

**Purpose**: Cache language detection results (name, version, icon, color)

**Implementation**:
```rust
pub struct LanguageCache {
    cache: Arc<Mutex<LruCache<PathBuf, CachedLanguageInfo>>>,
    ttl: Duration,
}

struct CachedLanguageInfo {
    info: Option<LanguageInfo>,
    cached_at: Instant,
}
```

**Policy**:
- **TTL**: 24 hours (from `CachePolicy::language_version()`)
- **Cooldown**: 0 ms (language detection runs on-demand)

**Cache Key**: Directory path where language was detected

**Example**:
```rust
// Check cache
if let Some(cached) = lang_cache.get(path) {
    return Ok(cached);
}

// Detect language
let info = detector.detect(path)?;
lang_cache.insert(path.to_path_buf(), info.clone());
```

**Invalidation Triggers**:
1. TTL expiration (after 24 hours)
2. File watcher detects `package.json`, `Cargo.toml`, etc. changes
3. Explicit `invalidate(path)` call

### Layer 3: Theme Cache

**Location**: `gpy-agent/src/theme/manager.rs`

**Purpose**: Cache theme configuration (colors, icons, separators) with hot-reload support

**Implementation**:
```rust
pub struct ThemeManager {
    theme: Arc<RwLock<ThemeConfig>>,
    theme_name: Arc<RwLock<String>>,
    theme_path: Arc<RwLock<Option<PathBuf>>>,
}
```

**Policy**:
- **TTL**: 5 seconds (from `CachePolicy::theme()`)
- **Cooldown**: 100ms (from `CachePolicy::theme()`)
- **Reload**: Automatic via file watcher

**Features**:
- Hot-reload theme on file change (no agent restart required)
- Thread-safe theme swapping via `RwLock`
- Automatic detection of theme file modifications

---

## Timing Relationships

### The Three Timing Mechanisms

GPY uses three distinct timing windows that work together:

```
Timeline (horizontal axis = time)
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━

File Change Detected (.git/HEAD modified)
│
├─[Debouncing: 100ms]──────────┐
│                               ▼
│                         Signal Sent (SIGUSR1)
│                               │
│                               ▼
│                         IPC Request Arrives
│                               │
├─[Cooldown: 150ms]────────────┤
│                               │
│  During cooldown:             │
│  • Skip git query             │
│  • Return cached value        ▼
│                         Cache Hit
│
├─[TTL: 30s]───────────────────────────────────────────────────┐
│                                                               │
│  After TTL expiration:                                        │
│  • Cache entry stale                                          │
│  • Next request re-queries git                                ▼
│                                                         Cache Miss
━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━
```

### 1. Debouncing (100ms)

**Purpose**: Coalesce rapid file events to prevent prompt flicker.

**Example scenario**:
```bash
# User runs: git commit -m "message"
# Git writes multiple files in quick succession:

t=0ms:    .git/HEAD modified       ─┐
t=5ms:    .git/index modified       │ Debouncing window
t=10ms:   .git/refs/heads/main      │ (100ms)
t=15ms:   .git/COMMIT_EDITMSG       │
...                                  │
t=100ms:  Debouncer expires         ─┘
t=101ms:  SIGUSR1 sent (once)
```

**Without debouncing**: Would send 4+ signals, causing prompt to flicker.

**With debouncing**: Single signal after events settle.

**Configuration**:
```rust
// Environment variable: GPY_DEBOUNCE_MS (default: 100)
let config = WatcherConfig::from_env();
let debounce_duration = config.debounce_duration();  // 100ms
```

**Location**: `gpy-agent/src/watcher/debouncer.rs`

### 2. Cooldown (150ms)

**Purpose**: Prevent redundant git queries when watcher triggers cache refresh.

**Example scenario**:
```bash
# User modifies file and immediately checks prompt:

t=0ms:    User runs: git add file.txt
t=5ms:    Watcher detects .git/index change
t=10ms:   Agent updates git cache
t=15ms:   SIGUSR1 sent to Fish shell
t=20ms:   IPC request arrives: "get git status"
t=21ms:   ✓ Cache hit (cooldown prevents re-query)

# Without cooldown:
t=21ms:   ✗ Cache miss → Re-run git status (redundant, slow)
```

**Race condition solved**:

The cooldown prevents this race:
1. Watcher updates cache at t=10ms
2. Signal triggers prompt render at t=20ms
3. Without cooldown: Cache already expired, re-queries git
4. With cooldown: Use recently-updated cache value

**Configuration**:
```rust
// Cooldown is defined by cache policy (not configurable via environment)
let policy = CachePolicy::git_status();
let cooldown = policy.cooldown();  // 150ms (compile-time constant)
```

**Location**: `gpy-agent/src/cache/policy.rs:130-150`

### 3. TTL (Time To Live)

**Purpose**: Expire stale cache entries that haven't been updated.

**Example scenario**:
```bash

# User leaves terminal idle for 2 minutes:

t=0s:     Cache entry created (git status cached)
t=30s:    TTL expires
t=31s:    Next IPC request → Cache miss → Re-query git
t=32s:    New cache entry created

# User actively working (prompt renders every 5s):
t=0s:     Cache created
t=5s:     Cache hit (TTL: 25s remaining)
t=10s:    Cache hit (TTL: 20s remaining)
...
t=30s:    TTL expires
t=31s:    Cache miss → Refresh
```

**Policy by subsystem**:

| Cache    | TTL      | Rationale |
|----------|----------|-----------|
| Git      | 30 seconds | Short TTL catches watcher misses and remote updates |
| Language | 24 hours | Version checks are expensive and change rarely |
| Theme    | 5 seconds | Enables hot-reload for rapid theme development iteration |

**Configuration**:
```rust
use gpy_agent::cache::CachePolicy;

let git_policy = CachePolicy::git_status();        // TTL 30s, cooldown 150ms
let lang_policy = CachePolicy::language_version(); // TTL 24h, cooldown 0ms
let theme_policy = CachePolicy::theme();           // TTL 5s, cooldown 100ms
```

### Why Different Values?

**Debounce (100ms)**:
- Too short (<50ms): Doesn't coalesce multi-file operations
- Too long (>200ms): Prompt updates feel sluggish
- Sweet spot: 100ms (fast enough, coalesces most git operations)

**Cooldown (150ms)**:
- Policy-defined via `CachePolicy::git_status()`
- Ensures cooldown ≥ debounce (100ms) + signal delivery buffer

**TTL (30s for git, 24h for language, 5s for theme)**:
- Git changes frequently: 30s keeps data fresh without thrashing
- Languages change rarely: 24h minimises subprocess calls
- Themes reload on demand: 5s enables hot-reload during development

---

## Cache Invalidation

### Invalidation Strategies

GPY uses three invalidation strategies:

#### 1. Time-Based Invalidation (TTL)

**When**: Entry exceeds TTL duration

**How**: Check timestamp on cache read
```rust
pub fn get(&self, path: &Path) -> Option<GitInfo> {
    let entry = self.cache.lock().ok()?.get(path)?;

    // Check if expired
    if entry.cached_at.elapsed() > self.ttl {
        self.cache.lock().ok()?.pop(path);  // Evict
        return None;
    }

    Some(entry.info.clone())
}
```

**Pros**:
- Simple to implement
- Predictable staleness window

**Cons**:
- May return stale data if TTL not yet expired
- Requires TTL tuning per use case

#### 2. Event-Based Invalidation

**When**: File watcher detects relevant change

**How**: Watcher callback invalidates cache explicitly
```rust
// In agent.rs event loop (line ~350)
match event.event {
    FileEvent::Git { ref path } => {
        // Invalidate git cache for this repo
        agent.git_cache.invalidate(path);

        // Update cache with fresh data
        if let Ok(info) = get_git_status(path) {
            agent.git_cache.insert(path.clone(), info);
        }

        // Signal clients
        agent.client_directory.signal_clients_for_repo(path, throttle_ms)?;
    }
}
```

**Pros**:
- Immediate invalidation on change
- No stale data (assuming watcher is reliable)

**Cons**:
- Watcher may miss events (rare)
- Requires file watching infrastructure

#### 3. Capacity-Based Eviction (LRU)

**When**: Cache exceeds capacity limit

**How**: Least-recently-used entry evicted
```rust
// LruCache automatically evicts oldest entry when full
let mut cache = LruCache::new(100);  // Max 100 entries
cache.put(path, info);  // Evicts LRU if at capacity
```

**Pros**:
- Bounded memory usage
- No configuration needed

**Cons**:
- May evict entry that would have been useful
- Doesn't consider staleness (only recency)

### Invalidation Priority

**Order of operations when cache needs updating**:

1. **Event-based invalidation** (highest priority)
   - File watcher detects change → Invalidate immediately
   - Example: `.git/HEAD` modified → Invalidate git cache

2. **TTL expiration** (medium priority)
   - Entry exceeds TTL → Mark stale on next read
   - Example: 60 seconds pass → Next request refreshes cache

3. **Capacity eviction** (lowest priority)
   - Cache full → Evict LRU entry
   - Example: 101st repo added → Evict oldest repo's cache

### Cooldown Window Exception

**Special case**: During cooldown window, skip invalidation from watcher events.

**Rationale**: Watcher already updated cache, don't query again.

```rust
pub fn get_with_cooldown(&self, path: &Path, cooldown: Duration) -> Option<GitInfo> {
    let entry = self.cache.lock().ok()?.get(path)?;

    // Check TTL expiration
    if entry.cached_at.elapsed() > self.ttl {
        return None;  // Expired
    }

    // During cooldown: Use cache even if normally would refresh
    Some(entry.info.clone())
}
```

**Timeline**:
```
t=0ms:    Watcher detects change → Updates cache
t=10ms:   SIGUSR1 sent
t=20ms:   IPC request: "get git status"
t=21ms:   Cooldown check: 21ms < 150ms → Return cached value
t=150ms:  Cooldown expires
t=151ms:  Next request: Normal TTL logic applies
```

---

## Configuration and Tuning

### Environment Variables

Only the watcher timing knobs remain user-configurable; cache TTL/cooldown values now live in `CachePolicy`:

```bash
# Debouncing (watcher event coalescing)
export GPY_DEBOUNCE_MS=100              # Default: 100ms

# Signal throttling (per-client rate limit)
export GPY_SIGUSR1_THROTTLE_MS=150      # Default: 150ms

# Note: Cache cooldown is now policy-driven (not configurable)
# Git cooldown: 150ms (defined in CachePolicy::git_status)
# Language cooldown: 0ms (defined in CachePolicy::language_version)

# Git cache TTL (not yet configurable, hardcoded in policy)
# Default: 30 seconds (defined in CachePolicy::git_status)

# Language cache TTL (not yet configurable, hardcoded in policy)
# Default: 24 hours (defined in CachePolicy::language_version)
```

**Applying configuration**:
```bash
# Set in Fish config
# Restart agent to apply debounce/throttle changes
gpy restart

# TTL/cooldown adjustments require editing CachePolicy constants
```

### Tuning Recommendations

#### For Slower Systems

If prompt feels sluggish on lower-end hardware:

```bash
# Longer debounce (reduce signal frequency)
export GPY_DEBOUNCE_MS=200

# Note: TTL and cooldown are now policy-driven (not configurable via env vars)
# To change them, modify CachePolicy in gpy-agent/src/cache/policy.rs
```

**Trade-off**: Less frequent updates, but faster prompts.

#### For Real-Time Updates

If you need instant prompt updates:

```bash
# Shorter debounce (faster signal delivery)
export GPY_DEBOUNCE_MS=50

# Note: TTL and cooldown are now policy-driven (not configurable via env vars)
# To change them, modify CachePolicy in gpy-agent/src/cache/policy.rs
```

**Trade-off**: More frequent git queries, slight CPU increase.

#### For Large Monorepos

If working in very large repositories (>100k files):

```bash
# Longer debounce (coalesce more events)
export GPY_DEBOUNCE_MS=300
```

**Trade-off**: Less real-time, but avoids expensive git status on huge repos.

## Debugging Cache Issues

### Symptom: Prompt Shows Stale Data

**Example**: Branch changed but prompt still shows old branch.

**Diagnostic steps**:

1. **Check TTL expiration**:
```bash
# Enable debug logging
export GPY_DEBUG_LOG=~/.cache/gpy/debug.log
gpy restart

# Trigger prompt render
fish_prompt

# Check log for cache hits/misses
tail -f ~/.cache/gpy/debug.log | grep "cache"
```

Expected output:
```
[git] Cache hit for /home/user/project (age: 15s)
[git] Cache miss for /home/user/other (not found)
```

2. **Verify file watcher is running**:
```bash
# Check if watcher detected change
tail -f ~/.cache/gpy/debug.log | grep "watcher"
```

Expected after `git checkout`:
```
[watcher] Event: Git { path: "/home/user/project/.git/HEAD" }
[watcher] Invalidating cache for /home/user/project
```

3. **Force cache invalidation**:
```bash
# Restart agent (clears all caches)
gpy restart
```

**Common causes**:
- TTL too long (solution: lower `CachePolicy::git_status()` TTL)
- File watcher not detecting change (solution: check watcher logs)
- Cooldown preventing refresh (solution: wait 150 ms, try again)

### Symptom: Prompt Updates Too Slowly

**Example**: Run `git add`, prompt doesn't update for 1-2 seconds.

**Diagnostic steps**:

1. **Measure debounce timing**:
```bash
# Check debounce window
echo $GPY_DEBOUNCE_MS

# Expected: 100 (ms)
# If higher: Reduce debounce for faster updates
```

2. **Check signal delivery**:
```bash
# Time from file change to prompt update
# In one terminal:
touch test.txt && date +%s%3N

# Watch prompt update, note time
date +%s%3N

# Difference should be <200ms (100ms debounce + 100ms render)
```

3. **Profile git query time**:
```bash
# Time a git status query
time git status --porcelain

# Should be <50ms
# If slower: Large repo, consider longer TTL or skip-worktree
```

**Common causes**:
- Debounce too long (solution: reduce `GPY_DEBOUNCE_MS`)
- Expensive git query (solution: consider raising `CachePolicy::git_status()` TTL)
- Too many file events (solution: add patterns to watcher ignore list)

### Symptom: High CPU Usage

**Example**: `gpy-agent` using 5-10% CPU constantly.

**Diagnostic steps**:

1. **Check for event storm**:
```bash
# Count watcher events per second
tail -f ~/.cache/gpy/debug.log | grep "watcher" | pv -l -i 1 -r > /dev/null

# Expected: <10 events/sec during normal work
# If higher: File watcher receiving too many events
```

2. **Identify noisy paths**:
```bash
# Most frequent event paths
grep "watcher" ~/.cache/gpy/debug.log | awk '{print $NF}' | sort | uniq -c | sort -rn | head

# Common culprits:
#   .git/objects/  (should be filtered)
#   node_modules/  (should be filtered)
#   build/         (add to ignore list)
```

3. **Tune debouncing**:
```bash
# Increase debounce to reduce signal frequency
export GPY_DEBOUNCE_MS=300
gpy restart
```

**Common causes**:
- Watching noisy directories (solution: add to ignore list in `watcher/mod.rs:254-272`)
- Too many git queries (solution: increase TTL)
- Background processes writing files (solution: increase debounce)

---

## Performance Characteristics

### Cache Hit Rates

**Typical hit rates** (measured in production):

| Cache | Hit Rate | Scenario |
|-------|----------|----------|
| Git | ~95% | Active development (prompt every 5s) |
| Git | ~60% | Infrequent use (prompt every 60s+) |
| Language | ~98% | Stable project (no package changes) |
| Language | ~70% | Frequent `npm install` |

**Formula**:
```
Hit Rate = (Cache Hits / Total Requests) × 100%
```

**Impact of hit rate**:
- **95% hit rate**: 10-50ms saved on 95% of prompts
- **60% hit rate**: Still saves significant time, but more queries
- **Below 50%**: Cache may not be providing much benefit

### Memory Usage

**Typical memory footprint** (100 repos, 50 dirs):

```
GitCache:         100 repos × 200 bytes  = 20 KB
LanguageCache:     50 dirs  × 100 bytes  =  5 KB
Internal overhead: (Mutex, Arc, LRU)     = ~5 KB
─────────────────────────────────────────────────
Total cache memory:                       ≈ 30 KB
```

**Agent total memory**: ~2-5 MB (mostly Tokio runtime overhead)

**Cache capacity impact**:
```
100 repos × 200 bytes =  20 KB
500 repos × 200 bytes = 100 KB
1000 repos × 200 bytes = 200 KB
```

**Recommendation**: Keep capacity at 100 unless working with many repos.

### Query Latency

**Typical query times** (measured on macOS, SSD):

| Operation | Uncached | Cached | Cache Benefit |
|-----------|----------|--------|---------------|
| Git status (small repo) | 10ms | <1ms | **10x faster** |
| Git status (large repo) | 50ms | <1ms | **50x faster** |
| Language detection | 5-20ms | <1ms | **5-20x faster** |
| IPC round-trip | 1-2ms | N/A | Already fast |

**Prompt render budget**: 50ms total

With caching:
- Git: 1ms
- Language: 1ms
- IPC: 2ms
- Fish rendering: 5ms
- **Total: ~9ms** ✅ (well under budget)

Without caching:
- Git: 30ms
- Language: 15ms
- IPC: 2ms
- Fish rendering: 5ms
- **Total: ~52ms** ❌ (over budget)

---

## Advanced Topics

### Cache Warming

**Strategy**: Pre-populate cache on agent startup.

**Why**: First prompt render is slow (cache cold), subsequent renders fast.

**Implementation** (future enhancement):
```rust
pub async fn warm_cache(agent: &Agent) {
    // Find all git repos in common directories
    let common_dirs = vec![
        PathBuf::from(env::var("HOME").unwrap()).join("Developer"),
        PathBuf::from("/workspace"),
    ];

    for dir in common_dirs {
        for repo in find_git_repos(&dir) {
            // Pre-populate cache
            if let Ok(info) = get_git_status(&repo) {
                agent.git_cache.insert(repo, info);
            }
        }
    }
}
```

**Trade-off**: Slower agent startup, but faster first prompt.

### Multi-Level Caching

**Future enhancement**: Add L2 disk-based cache for rarely-changed data.

**Use case**: Theme data, language versions, git remote URLs.

**Architecture**:
```
L1: Memory cache (LRU, fast)
    ├─ TTL: 30s
    └─ Capacity: 100 entries

L2: Disk cache (persistent, slower)
    ├─ TTL: 24 hours
    └─ Capacity: 1000 entries

On cache miss:
1. Check L1 (memory) → Miss
2. Check L2 (disk) → Hit → Populate L1
3. If L2 miss → Query source → Populate L2 and L1
```

**Benefit**: Survive agent restarts without cache invalidation.

---

## Next Steps

- **For module references**: See [modules.md](modules.md)
- **For segment development**: See [segment-development.md](../dev/segment-development.md)
- **For system architecture**: See [architecture.md](../dev/architecture.md)

---

## Summary

### Key Takeaways

1. **Three timing mechanisms**:
   - Debouncing (100 ms): Coalesce file events
   - Cooldown (150 ms): Skip redundant queries after watcher updates
   - TTL (30 s / 24 h / 5 s): Expire stale cache entries per policy

2. **Three cache layers**:
   - Git status cache (policy TTL 30 s): Frequent changes
   - Language version store (policy TTL 24 h): Infrequent changes
   - Theme cache (policy TTL 5 s): Hot-reload support active

3. **Three invalidation strategies**:
   - Time-based (TTL): Expire after duration
   - Event-based (watcher): Invalidate on file change
   - Capacity-based (LRU): Evict oldest entry when full

4. **Performance targets**:
   - Prompt render: <50ms
   - Cache hit rate: >90%
   - Memory usage: <10MB

5. **Tuning philosophy**:
   - Default watcher values work for most users (100 ms debounce / 150 ms throttle)
   - Tune for slower systems: Increase debounce, keep policy TTLs unless absolutely necessary
   - Tune for real-time: Decrease debounce, consider lowering policy TTLs if you can afford extra git work

# Reliability Investigation (gpy-c5v)

**Date**: 2026-01-15
**Scope**: Cache invalidation correctness, memory growth, watcher leaks.

## 1. Cache Invalidation

**Mechanism**:
The agent uses a hybrid approach for git status caching:
1.  **Incremental Updates**: When specific files change, `load_repository_state` is called with a path filter.
2.  **Full Fallback**: If the incremental update fails or returns empty results, the cache entry is invalidated, and a full scan is triggered.

**Findings**:
*   **Correctness Issue**: The incremental update logic (`agent/events.rs`) relies on the file watcher reporting specific modified files. However, global state changes like modifying `.gitignore` or `.git/config` affect the status of *other* files.
    *   If `.gitignore` is modified, the watcher reports `.gitignore`.
    *   `load_repository_state` updates the status of `.gitignore`.
    *   It **does not** re-evaluate other files that might have become ignored or unignored.
    *   **Result**: The cached aggregate status (e.g., "untracked count") may drift from reality until a full scan occurs (which only happens if the incremental path fails, which it won't for `.gitignore`).

**Recommendation**:
*   Identify "global impact" files (e.g., `.gitignore`, `.git/config`, `.git/exclude`).
*   Force a full scan/invalidation when these specific files change, bypassing the incremental logic.

## 2. Memory Growth

**Mechanism**:
The `ClientDirectory` (`ipc/registry.rs`) tracks active clients and notification throttling.

**Findings**:
*   **Client Registry**: Dead clients are actively pruned every 60 seconds. This prevents unbounded growth from zombie processes.
*   **Throttle Map Leak**: The `last_notify` map (`Mutex<HashMap<PathBuf, Instant>>`) tracks the last notification time for every repository path encountered.
    *   **Issue**: There is **no cleanup mechanism** for this map.
    *   **Impact**: If a user navigates to thousands of different directories over the agent's lifetime (e.g., crawling a large tree), this map will grow indefinitely.
    *   **Severity**: Low to Moderate. `PathBuf` + `Instant` is small, but unbounded growth is technically a leak.

**Recommendation**:
*   Implement a cleanup strategy for `last_notify`. For example, clear entries older than 1 hour during the pruning cycle.

## 3. Watcher Leaks & Race Conditions

**Mechanism**:
The `MultiRepoWatcher` (`watcher/multi_repo.rs`) uses reference counting to share watchers among clients.

**Findings**:
*   **Race Condition in `unregister_client`**: The unregistration process involves dropping the repository lock, stopping the watcher, and then re-acquiring the lock to remove the map entry.
    *   **Scenario**:
        1.  Thread A (unregister): Determines repo count is 0. Unlocks map.
        2.  Thread B (register): Locks map. Finds existing repo entry. Increments count to 1. Unlocks map.
        3.  Thread A: Locks watcher. Stops watching the repo.
        4.  Thread A: Locks map. Removes the repo entry.
    *   **Result**: Thread B successfully registered a client, but the underlying watcher has been stopped, and the repository entry is removed from the map. The client believes it is being watched but receives no updates.

**Recommendation**:
*   **Atomic Operation**: The check-and-remove logic needs to be atomic relative to registration.
*   **Re-verification**: After re-acquiring the lock in `unregister_client`, verify the client count is still 0. If not, re-enable the watcher (or abort removal).

## Summary of Action Items

1.  **Fix Cache Invalidation**: Add special handling for `.gitignore` and `.git/config` in `handle_file_event` to force full invalidation.
2.  **Fix Memory Leak**: Add `prune_throttle_map` to `ClientDirectory` and call it from the pruning timer.
3.  **Fix Watcher Race**: Refactor `unregister_client` to double-check client count before final removal/unwatching.

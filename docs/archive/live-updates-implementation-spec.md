# Live Updates Implementation Specification

> **Archived.** This is a historical implementation specification. Refer to the current agent implementation and test suites for active behavior.

## Overview

This document specifies the implementation of live updates for the GPY prompt system. Live updates enable terminal prompts to automatically refresh when git repository state changes, providing real-time feedback across all terminal sessions working in the same repository.

## Scope

- Target platforms: POSIX systems where Fish runs (Linux/macOS/BSD). Uses SIGUSR1; Windows is not supported (WSL best‑effort only).
- Non-goals: No network I/O; no heavyweight background services beyond the GPY agent. Prioritize speed, low CPU, and reliability.

## Goals

1. **Automatic cross-terminal updates**: When files change in a git repository, all terminals with prompts in that repository should update automatically
2. **Performance**: Updates should be fast and not impact prompt responsiveness
3. **Stability**: No infinite loops, no excessive repainting, graceful degradation if watcher fails
4. **Correctness**: Only relevant terminals should be notified (same repository)

## Architecture

### Components

1. **File Watcher** (`MultiRepoWatcher`)
   - Monitors filesystem changes in git repositories
   - Uses `notify` crate with debouncing
   - Maintains reference counting for watched directories

2. **Agent** (`Agent`)
   - Coordinates between watcher, cache, and client registry
   - Handles file events and triggers appropriate actions
   - Manages cache refresh and client notification

3. **Git Cache** (`GitStatusCache`)
   - Stores git repository status in memory
   - Provides fast lookups for prompt rendering
   - Thread-safe with Arc + Mutex

4. **Client Registry** (`ClientDirectory`)
   - Tracks registered Fish shell processes
   - Stores PID and current working directory for each client
   - Sends SIGUSR1 signals to notify clients of updates

5. **Fish Signal Handler** (`__gpy_on_usr1`)
   - Receives SIGUSR1 signals
   - Triggers prompt repaint via `commandline -f repaint`

## Implementation Details

### 1. What to Watch

**Decision**: Watch the git directory (resolved gitdir), not the entire working tree.

**Rationale**:
- Minimizes watcher overhead and CPU usage, especially in large repositories
- Git state changes relevant to the prompt (branch, index, HEAD, rebase/merge state) are reflected in the gitdir
- Default behavior keeps prompt fast and avoids over‑watching; untracked file create/remove is not a default trigger

**Worktrees**:
- `.git` may be a file that points to the real git directory. Resolve the gitdir via parsing that file or using `git rev-parse --git-common-dir`, and watch the resolved directory.

**Implementation**:
```rust
// In multi_repo.rs, register_client()
let git_root = Self::find_git_root(cwd)?; // repo working tree root
let git_dir = resolve_gitdir(&git_root)?; // handles worktrees
watcher.watch_directory(&git_dir)?;       // watch gitdir, not the entire working tree
```
Where `resolve_gitdir` resolves `.git` file indirection for worktrees.

### 2. Cache Strategy

**Decision**: Use **proactive cache refresh** with atomic replacement.

**Rationale**:
- **Problem with invalidate-only**: Cache is empty during refresh window, causing prompt to fall back to slow oneshot
- **Problem with invalidate-then-refresh**: Race condition where clients request between invalidate and refresh
- **Solution**: Directly replace old cache with new cache atomically

**Implementation**:
```rust
// In agent.rs, handle_file_event()
FileEvent::Git { path } => {
    if let Some(git_root) = MultiRepoWatcher::get_git_root(path) {
        // Do NOT call cache.invalidate() - this creates a race condition
        // Proactively refresh cache (atomic replacement)
        if let Ok(Some(status)) = load_repository_state(&git_root) {
            cache.set(git_root.clone(), status);  // Atomic replacement
            // Notify clients with fresh data ready
            registry.notify_sigusr1(Some(&git_root));
        }
    }
}
```

**Flow**:
1. File changes → watcher detects event
2. Load fresh git status (old cache still available)
3. Replace old cache with new cache atomically
4. Send SIGUSR1 to clients
5. Clients repaint → request from cache → get fresh data

### 3. Client Notification

**Decision**: Send SIGUSR1 with **git repository root path**, not the changed file path.

**Rationale**:
- `notify_sigusr1` filters clients by path matching
- If we pass `/repo/subdir/file.txt`, it won't match clients in `/repo`
- Passing `/repo` (git root) matches all clients anywhere in that repo

**Implementation**:
```rust
// WRONG - passes file path
registry.notify_sigusr1(Some(path.as_path()));  // path = /repo/foo.md

// CORRECT - passes git root
registry.notify_sigusr1(Some(&git_root));  // git_root = /repo
```

Canonicalize both client cwd and target path before matching to avoid symlink issues.

**Client matching logic** (in `ClientDirectory::notify_sigusr1`):
```rust
fn paths_related(client: &Path, target: &Path) -> bool {
    if client == target {
        return true;
    }
    client.starts_with(target) || target.starts_with(client)
}
```

This ensures:
- Client at `/repo` matches target `/repo` ✓
- Client at `/repo/subdir` matches target `/repo` ✓
- Client at `/other-repo` does NOT match target `/repo` ✗

### 4. Preventing Infinite Loops

**Problem**: Earlier attempts caused repaint loops where prompts continuously redrew.

**Lightweight solutions**:

#### 4.1 Debouncing
Use the internal DebounceEngine (100ms default) that coalesces rapid events on a background flush loop. Keep it simple; no extra timers or queues.

#### 4.2 Event Selection
Trigger updates based on simple path‑based checks focused on gitdir files (e.g., `HEAD`, `index`, `config`). Avoid broad working‑tree watches by default.

#### 4.3 Keep Work Minimal
On event: load fresh status, atomically `set` the cache, and notify once using the git root. No invalidate‑then‑refresh cycles.

### 5. Error Handling

**Principle**: Watcher failures should not break the prompt. Graceful degradation.

**Implementation**:
```rust
// In Agent::new()
let watcher = MultiRepoWatcher::new(callback)
    .ok()  // Convert Result to Option
    .map(Arc::new);

if watcher.is_none() {
    log_message("Warning: File watcher failed - agent works without live updates");
}
```

If watcher creation fails:
- Agent continues working normally
- Prompts still render fast via cache
- Updates still work via manual prompt refresh (user runs commands)
- Only missing feature: automatic cross-terminal updates

### 6. Logging Strategy

**Problem**: Excessive logging (especially via `log_message()`) can cause performance issues and contribute to feedback loops.

**Solution**: Use conditional debug logging
```rust
// Use debug_log! macro (only logs when GPY_DEBUG_LOG env var is set)
debug_log!("agent", "Git event for: {}", path.display());

// NOT this (always logs to file):
log_message(&format!("Git event for: {}", path.display()));
```

Reserve `log_message()` for:
- Startup/shutdown messages
- Critical errors
- Watcher creation success/failure

### 7. Optional: Untracked File Updates (Aggressive Mode)

The default design watches only the gitdir for minimum overhead. To support immediate cross‑terminal updates on untracked file create/remove, add an opt‑in “aggressive” mode.

**Activation**:
- Env var: `GPY_WATCH_WORKTREE=1` (recommended minimal toggle)
- Or config: `[watch] worktree = true`

**Watcher behavior**:
- Always watch the resolved gitdir (default behavior)
- If aggressive mode is enabled, also watch the repository working tree root recursively

```rust
// In multi_repo.rs, register_client()
let git_root = Self::find_git_root(cwd)?;
let git_dir = resolve_gitdir(&git_root)?;
w.watch_directory(&git_dir)?;         // always

if aggressive_mode_enabled() {
    w.watch_directory(&git_root)?;    // opt-in: also watch working tree
}
```

**Event detection** (in filesystem watcher loop):
- Keep simple path‑based triggers for gitdir files (`HEAD`, `index`, `config`)
- If aggressive mode is enabled and the path is outside `.git/` and not ignored:
  - Trigger on Create/Remove/Rename only

Pseudo‑code:
```rust
for (event in notify_events) {
    for path in event.paths {
        if is_in_gitdir(&path) && is_gitdir_file_of_interest(&path) {
            emit(FileEvent::Git { path });
            break;
        }
        if aggressive_mode_enabled()
            && !is_in_gitdir(&path)
            && !is_ignored(&path)
            && is_create_remove_rename(&event.kind)
        {
            emit(FileEvent::Git { path });
            break;
        }
    }
}
```

**Ignore filters**:
- Reuse `get_ignore_patterns()` to skip heavy paths (e.g., `node_modules/`, `target/`, build caches)
- Implementation may use cheap substring/segment checks; no glob engine required

**Debouncing and notifications**:
- Rely on existing 100ms DebounceEngine (no extra rate limiter)
- Continue to notify with the repository root path and use proactive cache `set`

## Testing Strategy

### Test 1: Single Terminal Update (default)
1. Open one terminal in a git repo
2. Stage a file: `git add foo.md` (or commit)
3. **Expected**: Prompt updates immediately in the same terminal
4. **Verify**: Index/branch/state indicators reflect the change

### Test 2: Cross-Terminal Update (default)
1. Open two terminals in the same git repo
2. In terminal A: `git add bar.md` (or commit)
3. **Expected**: Terminal B's prompt updates automatically (within 1 second)
4. **Verify**: Both terminals show updated git status without running any command in terminal B

### Test 3: Different Repositories
1. Open terminal A in `/repo1`
2. Open terminal B in `/repo2`
3. In terminal A: `touch foo.md`
4. **Expected**: Only terminal A updates, terminal B is unaffected
5. **Verify**: Terminal B's prompt does not change

### Test 4: Subdirectory Handling
1. Open terminal A at `/repo`
2. Open terminal B at `/repo/subdir`
3. In terminal A: `git add /repo/file.md`
4. **Expected**: Both terminals update (same repository)
5. **Verify**: Both show the change

### Test 5: No Infinite Loops
1. Open terminal in a git repo
2. Stage or commit a change: `git add test.md` or `git commit -m test`
3. **Expected**: Prompt repaints exactly once (maybe twice due to debouncing)
4. **Verify**: No continuous scrolling, prompt stabilizes immediately
5. **Duration**: Should complete within 1-2 seconds

### Test 6: Performance
1. Make rapid file changes: `for i in {1..10}; do touch file$i.md; done`
2. Stage or commit them in bursts
3. **Expected**: Prompts update but remain responsive
4. **Verify**: No lag, no hangs, final state is correct

### Test 7: Watcher Failure Graceful Degradation
1. Simulate watcher failure (e.g., too many open files)
2. **Expected**: Prompt still works, just without live updates
3. **Verify**: Commands still show updated git status

### Test 8: Aggressive Mode Untracked Updates (opt‑in)
1. Enable mode: `export GPY_WATCH_WORKTREE=1`
2. Open two terminals in the same repo
3. In terminal A: `touch added.md`
4. **Expected**: Terminal B updates within 1 second
5. **Verify**: Untracked `?` indicator appears without any command in terminal B

## Implementation Checklist

- [ ] Ensure `multi_repo.rs` resolves worktrees and watches the resolved gitdir
- [ ] Update `agent.rs` to use proactive cache refresh (`cache.set` instead of `invalidate`)
- [ ] Update `agent.rs` to pass `git_root` to `notify_sigusr1` instead of file `path`
- [ ] Keep debouncing simple (100ms DebounceEngine); no extra rate limiting
- [ ] Review logging (prefer `debug_log!`; reserve `log_message` for critical events)
- [ ] Validate tests reflect default gitdir‑only behavior (use git actions, not plain `touch`)
- [ ] Verify no infinite loops with multiple terminal sessions
- [ ] Document known limitations
- [ ] Add opt‑in aggressive mode: env/config toggle
- [ ] In `multi_repo.rs`, when enabled, also watch the working tree root
- [ ] In `filesystem.rs`, on aggressive mode: trigger on Create/Remove/Rename outside `.git/` and not ignored
- [ ] Reuse `get_ignore_patterns()` for cheap path ignores
- [ ] Add tests for aggressive mode untracked create/remove cross‑terminal updates
- [ ] Test agent restart scenario (kill running agent while shells are registered, restart, confirm recovery)
- [ ] Confirm SIGUSR1 handler does not interfere with Fish job control or other signals
- [ ] Test symlinked repository paths (clients in symlink vs. canonical path still receive updates)

## Known Limitations

1. **Network filesystems**: May have delayed or unreliable filesystem notifications
2. **Large repositories**: Initial watch setup may take longer
3. **Docker containers**: Filesystem events may not propagate correctly
4. **Maximum watched directories**: System limits on inotify watches (Linux)
5. **Windows**: Not supported (SIGUSR1 not available). WSL may work but is not guaranteed.

## Rollback Plan

If live updates cause issues in production:

1. Set watcher to `None` in `Agent::new()`:
   ```rust
   let watcher = None;  // Temporarily disable watcher
   ```

2. Rebuild and deploy: `./install-dev.fish`

3. This immediately disables live updates while keeping all other functionality

The system is designed to work perfectly well without the watcher - it's an optional enhancement.

## Future Enhancements

1. **Configurable debounce timing**: Allow users to tune responsiveness vs. CPU usage
2. **Smart caching**: Skip refresh if file change doesn't affect git status (e.g., modified but staged)
3. **Event kind filtering**: Optionally filter by `EventKind` for extra noise reduction (beyond the minimal checks above)
4. **Metrics**: Track watcher performance and notification latency

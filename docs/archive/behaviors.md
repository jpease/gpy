# Behaviors → Code Mapping

> **Archived.** This document is historical reference material and may not reflect current implementations.

**Purpose**: Map user-visible behaviors to specific code paths for better "what you see is what it does" legibility.

**Target Audience**: Developers, LLM agents, anyone debugging or extending GPY

**Last Updated**: 2025-01-15

---

## Table of Contents

- [Prompt Updates](#prompt-updates)
- [Live Updates](#live-updates)
- [Configuration](#configuration)
- [Performance](#performance)
- [Error Handling](#error-handling)

---

## Prompt Updates

### B1: Prompt Shows Current Git Status

**User-Visible Behavior**:
> "When I open a terminal in a git repository, the prompt shows my current branch, staged/unstaged file counts, and ahead/behind status."

**Trigger**: Fish shell starts or prompt repaints

**Effect**: Prompt displays git segment with branch and status indicators

**Code Path**:
1. `functions/fish_prompt.fish:60-120` - Main prompt orchestration
2. `segments/git.fish:segment_git_detect()` - Check if in git repo
3. `segments/git.fish:segment_git_render()` - Request git status
4. `core/ipc.fish:__gpy_request()` - Send IPC message to agent
5. `gpy-agent/src/ipc/server.rs:200-350` - Handle IPC request
6. `gpy-agent/src/git/status.rs:50-150` - Query git via gix library
7. `gpy-agent/src/git/cache.rs:40-80` - Check cache, return if fresh
8. `gpy-agent/src/formatter/fish_ansi.rs` - Format as ANSI codes
9. `segments/git.fish` - Render formatted response

**Configuration**:
- `git.enabled = true` (default) - Enable git detection
- `git.show_upstream = true` - Show ahead/behind counts
- `git.timeout_seconds = 10` - Git operation timeout

**Tests**:
- `gpy-agent/tests/git_status_tests.rs::test_git_status_clean_repo`
- `gpy-agent/tests/e2e_ipc_tests.rs::test_git_status_request`

**Performance**: 10-50ms (cold), < 1ms (cached)

---

### B2: Language/Tool Version Detection

**User-Visible Behavior**:
> "When I'm in a Rust project, the prompt shows the Rust logo and version. When I switch to a Node project, it shows Node version."

**Trigger**: Fish prompt renders, working directory contains language files

**Effect**: Language segment appears with icon and version

**Code Path**:
1. `segments/language.fish:segment_language_detect()` - Check if language files exist
2. `segments/language.fish:segment_language_render()` - Request language info
3. `core/ipc.fish:__gpy_request()` - Send "language" operation
4. `gpy-agent/src/language/detector.rs:80-200` - Detect via hyperpolyglot
5. `gpy-agent/src/language/version.rs:50-300` - Extract version from file/command
6. `gpy-agent/src/language/cache.rs:30-60` - Cache result (24h TTL)
7. `gpy-agent/src/formatter/fish_ansi.rs` - Format with icon and color

**Configuration**:
- `language.enabled = true` - Enable detection
- `language.show_versions = true` - Show version text
- `language.cache_ttl_hours = 24` - Version cache duration
- `language.enabled_languages = []` - Empty = detect all

**Tests**:
- `gpy-agent/tests/language_tests.rs::test_rust_detection`
- `gpy-agent/tests/language_tests.rs::test_node_version_detection`

**Performance**: 10-100ms (cold), < 1ms (cached)

**Special Notes**:
- Language version cache is **global** (shared across all requests)
- Location: `gpy-agent/src/language/version.rs:316` (OnceLock pattern)
- Cache key: `(language_name, cwd)` tuple
- Invalidation: Automatic after TTL expires

---

## Live Updates

### B3: Git Changes Trigger Instant Prompt Update

**User-Visible Behavior**:
> "When I run `git commit` in Terminal A, the prompt in Terminal B (same repo) updates within 200ms to show the new status."

**Trigger**: Git file modification (.git/HEAD, .git/index, etc.)

**Effect**: All Fish terminals in same repo receive SIGUSR1 and repaint

**Code Path**:
1. **File Change**: User runs `git commit` → writes to `.git/HEAD`
2. `gpy-agent/src/watcher/filesystem.rs:80-150` - notify crate detects change
3. `gpy-agent/src/watcher/mod.rs:252-336` - Filter: Accept .git/HEAD, ignore .git/objects
4. `gpy-agent/src/watcher/debouncer.rs:50-100` - Debounce (100ms)
5. `gpy-agent/src/agent.rs:1065-1130` - `handle_file_event()`
6. `gpy-agent/src/git/cache.rs` - Invalidate cache for repo
7. `gpy-agent/src/git/status.rs` - Refresh git status
8. `gpy-agent/src/git/cache.rs` - Update cache with fresh data
9. `gpy-agent/src/ipc/registry.rs:80-120` - Send SIGUSR1 to all clients in repo
10. **Fish Shell**: Receives SIGUSR1
11. `core/sigusr_handler.fish:__gpy_sigusr1_handler()` - Increment repaint trigger
12. `core/sigusr_handler.fish:__gpy_force_repaint()` - Call `commandline -f repaint`
13. **Prompt Redraws** with fresh git status

**Configuration**:
- `agent.enabled = true` - Agent must be running
- `agent.live_updates = true` - Enable filesystem watcher
- `git.enabled = true` - Git detection must be on
- `GPY_DEBOUNCE_MS` - Debounce interval (default 100ms)

**Tests**:
- `gpy-agent/tests/agent_daemon_tests.rs::test_file_watcher_triggers_signal`
- `tests/fish/e2e_live_updates_signal.test.fish`

**Performance**:
- File detection: 0-50ms (OS-dependent)
- Debounce wait: 100ms (configurable)
- Git query: 10-50ms
- Signal delivery: 2-5ms per client
- **Total**: ~200-300ms from file change to prompt update

**Critical Timing**:
- **Debounce (100ms)**: Prevents flicker from `git commit` writing multiple files
- **Cache Cooldown (150ms)**: Prevents redundant git queries when watcher + IPC both trigger

**Watched Files** (auto-trigger updates):
- ✅ `.git/HEAD` - Branch changes
- ✅ `.git/index` - Staged files
- ✅ `.git/refs/**` - Branch pointers
- ✅ `.git/MERGE_HEAD` - Merge state
- ✅ `.git/REBASE_HEAD` - Rebase state
- ❌ `.git/objects/**` - Too noisy
- ❌ `.git/logs/**` - Not user-visible changes

---

### B4: Clock Updates Every Minute (or Second)

**User-Visible Behavior**:
> "The clock in my prompt updates automatically every minute without me pressing Enter."

**Trigger**: Time-based interval (1s or 60s)

**Effect**: All registered Fish terminals receive SIGUSR1 and repaint

**Code Path**:
1. `gpy-agent/src/agent.rs:1207-1250` - Clock timer in `tokio::select!` loop
2. `tokio::time::interval()` - Sleep until next tick
3. **Alignment** (show_seconds=false only):
   - `agent.rs:200-250` - Calculate seconds until next minute boundary
   - Sleep exact duration to align with HH:MM:00
4. `gpy-agent/src/ipc/registry.rs` - `notify_sigusr1(None)` → all clients
5. **Fish**: SIGUSR1 → repaint → clock segment shows new time

**Configuration**:
- `clock.show_seconds = false` - 60s interval, aligned to minute boundaries
- `clock.show_seconds = true` - 1s interval, no alignment

**Tests**:
- `gpy-agent/tests/clock_timer_tests.rs::test_clock_timer_alignment`
- `gpy-agent/tests/clock_timer_tests.rs::test_sigusr1_delivery`

**Performance**:
- Tick overhead: < 1ms
- Signal delivery: ~2ms per client
- **Optimization**: Only sends signals if `clients.len() > 0`

**Special Notes**:
- **Minute alignment** (when show_seconds=false):
  - If agent starts at 14:32:47, first tick at 14:33:00 (13s wait)
  - Subsequent ticks every 60s exactly: 14:34:00, 14:35:00, etc.
  - Prevents clock "drift" over long uptimes

---

## Configuration

### B5: Config Changes Auto-Reload

**User-Visible Behavior**:
> "When I edit `~/.config/gpy/config.toml` and change `git.enabled = false`, my prompt instantly stops showing git status."

**Trigger**: Config file modification

**Effect**: Theme exports reload, Fish re-sources variables, segments update

**Code Path**:
1. **File Change**: User edits `~/.config/gpy/config.toml`
2. `gpy-agent/src/watcher/mod.rs:252-336` - Detect `*.toml` file change
3. `gpy-agent/src/agent.rs:1065-1130` - `handle_file_event(FileEvent::Config)`
4. `gpy-agent/src/config/loader.rs` - `invalidate_theme_cache()`
5. `gpy-agent/src/theme/manager.rs` - Reload config, regenerate Fish exports
6. **SIGUSR2 Sent** via theme manager callback
7. **Fish**: Receives SIGUSR2
8. `core/sigusr_handler.fish:__gpy_sigusr2_handler()` - Increment config trigger
9. `core/sigusr_handler.fish:__gpy_reload_config()` - Re-source theme
10. `gpy-agent theme export --format fish | source` - Load fresh variables
11. **Prompt Redraws** with new config

**Configuration-Dependent Behavior**:

| Setting Changed | Effect | Reload Type |
|----------------|--------|-------------|
| `git.enabled` | Git segment disappears/appears | Hot (SIGUSR2) |
| `language.show_versions` | Version text hidden/shown | Hot (SIGUSR2) |
| `ui.show_icons` | Icons replaced with text | Hot (SIGUSR2) |
| `ui.directory.max_length` | Directory truncation changes | Hot (SIGUSR2) |
| `agent.supervisor.*` | Supervisor behavior changes | Cold (restart) |
| `agent.enabled` | Agent startup/skip | Cold (restart) |

**Tests**:
- `gpy-agent/tests/config_hot_reload_tests.rs::test_sigusr2_sent_when_git_disabled`
- `gpy-agent/tests/config_hot_reload_tests.rs::test_theme_export_updates_enabled_segments`
- `gpy-agent/tests/cli_integration_tests.rs::test_config_hot_reload_git_toggle_full_scenario`

**Critical Design**:
- **SIGUSR2 is REQUIRED** even when disabling features
- Why: Fish needs to reload `__enabled_segments` to fix delimiter selection
- Without SIGUSR2: Fish keeps stale segment list → wrong delimiters

**Common Mistake**:
```rust
// ❌ WRONG: Skip signal when disabling git
if config.git.enabled {
    notify_sigusr2();  // Only send when enabled
}

// ✅ CORRECT: Always send signal on config change
notify_sigusr2();  // Send regardless of enabled/disabled
```

---

### B6: Theme Changes Apply Immediately

**User-Visible Behavior**:
> "When I change a color in my theme file, the prompt colors update instantly."

**Trigger**: Theme file modification (`.toml` in themes directory)

**Effect**: Same as config reload (SIGUSR2 path)

**Code Path**: Identical to B5 (config changes)

**Tests**:
- `gpy-agent/tests/theme_manager_tests.rs::test_theme_reload_on_file_change`

---

## Performance

### B7: Fast Prompt Rendering (< 50ms)

**User-Visible Behavior**:
> "When I press Enter, the new prompt appears instantly (< 50ms)."

**Optimization Strategies**:

**1. Caching** (Primary):
- Git status: 60s TTL, < 1ms cache hit
- Language version: 24h TTL, < 1ms cache hit
- Config/theme: Cached until file change

**Code**:
- `gpy-agent/src/git/cache.rs:40-80` - LRU cache (100 repos)
- `gpy-agent/src/language/cache.rs:30-60` - Version cache
- `gpy-agent/src/language/version.rs:316` - Global cache (OnceLock)

**2. IPC Optimization**:
- Unix socket: < 1ms round-trip
- Pre-allocated buffers: `Vec::with_capacity(256)` in `ipc/protocol.rs:18`
- Direct serialization: `serde_json::to_writer()` (no intermediate String)

**Code**:
- `gpy-agent/src/ipc/protocol.rs:16-26` - Optimized serialization
- `gpy-agent/src/ipc/transport.rs` - Buffered socket I/O

**3. Parallelization**:
- Git and language detection happen concurrently (separate IPC calls)
- Each segment can fail independently without blocking others

**4. Timeouts**:
- Git operations: 10s timeout (prevents hangs on slow filesystems)
- IPC requests: 5s timeout (prevents hung agent blocking prompt)

**Code**:
- `gpy-agent/src/git/commands.rs` - Timeout configuration
- `core/ipc.fish:__gpy_request()` - Client-side timeout

**5. Fallback**:
- If agent unavailable, fall back to oneshot mode or skip segment
- Never block prompt indefinitely

**Code**:
- `core/ipc.fish:__gpy_request()` - Fallback logic

**Benchmarks**:
- Cold (no cache): 20-50ms total
- Warm (cached): 5-15ms total
- Agent unavailable (oneshot): 50-150ms

**Tests**:
- Performance regression tests in CI
- `gpy-agent/tests/git_status_tests.rs` - Includes timing assertions

---

## Error Handling

### B8: Graceful Degradation on Errors

**User-Visible Behavior**:
> "If the agent crashes or git is slow, the prompt still works—just without that segment."

**Error Scenarios**:

**1. Agent Not Running**:
- Fish detects socket connection failure
- Falls back to oneshot mode OR hides segment
- User sees prompt without agent-dependent data

**Code**: `core/ipc.fish:__gpy_request()` - Error handling

**2. Git Timeout**:
- Git operation exceeds 10s
- Agent returns error
- Git segment hidden for this prompt

**Code**: `gpy-agent/src/git/commands.rs` - Timeout enforcement

**3. Invalid Repository**:
- User in corrupted git repo
- Agent returns error variant
- Segment hidden, no crash

**Code**: `gpy-agent/src/git/status.rs` - Error handling

**4. IPC Parse Error**:
- Malformed JSON from agent
- Fish logs error, skips segment
- Prompt renders without that data

**Code**: `core/ipc.fish` - JSON parsing with error check

**5. Permission Denied**:
- Can't read .git directory
- Security module rejects path
- Error logged, segment hidden

**Code**: `gpy-agent/src/security.rs` - Path validation

**Tests**:
- `gpy-agent/tests/git_status_tests.rs::test_git_status_invalid_path`
- `gpy-agent/tests/git_status_tests.rs::test_git_status_permission_denied`

**Error Logging**:
- Agent: `GPY_DEBUG_LOG=/tmp/gpy-debug.log gpy start`
- Debug: Check `/tmp/gpy-debug.log` for agent logs

---

## Cross-Cutting Concerns

### B9: Security Validation

**User-Visible Behavior**:
> "GPY won't follow symlinks to /etc or accept paths with null bytes."

**Validation Points**:

**1. All IPC Paths**:
- Every `cwd` parameter validated via `security::validate_path()`
- Rejects: `..`, null bytes, excessive length, suspicious patterns

**Code**: `gpy-agent/src/security.rs`

**2. Message Size**:
- 64KB maximum (prevents DoS)

**Code**: `gpy-agent/src/ipc/protocol.rs:92-100`

**3. Socket Permissions**:
- 0600 (user-only access)

**Code**: `gpy-agent/src/ipc/server.rs:150-180`

**4. Rate Limiting**:
- 50ms minimum per client (prevents IPC spam)

**Code**: `gpy-agent/src/ipc/server.rs` - ClientRateLimiter

**Tests**:
- `gpy-agent/tests/security_tests.rs` (if exists)
- Security audits in SECURITY.md

---

### B10: Signal Handling

**User-Visible Behavior**:
> "When I Ctrl-C the agent, it shuts down cleanly without leaving zombie processes."

**Signal Flow**:

**SIGUSR1** (Prompt Update):
1. Agent sends via `kill(pid, SIGUSR1)`
2. Fish kernel delivers to process
3. Fish trap: `trap '__gpy_sigusr1_handler' SIGUSR1`
4. Handler increments `$__gpy_repaint_trigger`
5. Variable change event triggers `__gpy_force_repaint`
6. Repaint calls `commandline -f repaint`

**Code**:
- `gpy-agent/src/ipc/registry.rs:80-120` - Sending side
- `core/sigusr_handler.fish` - Receiving side

**SIGUSR2** (Config Reload):
- Same mechanism, different handler (`__gpy_sigusr2_handler`)
- Triggers theme reload instead of repaint

**SIGTERM/SIGINT** (Shutdown):
1. OS delivers signal to agent
2. `tokio::signal::ctrl_c()` or `signal::unix::signal(SIGTERM)`
3. Agent shutdown sequence:
   - Stop IPC server
   - Stop file watcher
   - Close connections
   - Remove socket file

**Code**:
- `gpy-agent/src/agent.rs:1207-1250` - tokio::select! shutdown handling

**Tests**:
- `gpy-agent/tests/agent_daemon_tests.rs::test_graceful_shutdown`
- `tests/fish/e2e_live_updates_signal.test.fish`

---

## Appendix: Behavior Categories

### Synchronous Behaviors (< 50ms)
- Prompt rendering (B1, B2)
- Config variable access
- Fish function calls

### Asynchronous Behaviors (50-300ms)
- Live updates (B3, B4)
- Config reloads (B5, B6)
- File watcher events

### Background Behaviors (continuous)
- File watching
- Clock timer
- Cache expiration
- Client pruning (60s interval)

---

## How to Use This Document

**For Developers**:
- When adding new behavior, add entry here
- Map user-visible trigger → code path → tests
- Include configuration and performance notes

**For LLM Agents**:
- Reference specific behaviors (e.g., "See B3: Git Changes")
- Understand causal chains without reading all code
- Find relevant code quickly

**For Debugging**:
- User reports "prompt doesn't update after git commit" → See B3
- User asks "how to disable git segment" → See B1, B5
- Performance issue → See B7

---

**Last Updated**: 2025-01-15
**Maintainer**: GPY Team
**Related Docs**: [architecture.md](../dev/architecture.md), [modules.md](modules.md)

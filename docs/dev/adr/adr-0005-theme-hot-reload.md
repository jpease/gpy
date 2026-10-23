# ADR-0005: Theme and Config Hot-Reload

## Status

Accepted (2024-11)

Superseded in part by [ADR-0007](adr-0007-sigurg-doorbell-notifications.md): reloads are now a `<pid>.reload` flag file plus SIGURG instead of SIGUSR2.

Debounce revised (2026-10): both watchers now use 1 s. The debouncer's 1 s max-delay cap already flushed a 5 s window after about 1 s, so 5 s never coalesced a save session; it only spaced the flush ticks, delaying theme edits by up to 6 s. A reload of unchanged content rings nothing, so an editor rewriting the same bytes costs no extra doorbell. Each path has one watcher: `themes/` the theme watcher, the rest of the config dir (`config.toml`, palettes) the config watcher.

## Context

Users want to customize their prompt appearance (colors, icons, segments) without restarting the agent or shell. The editing experience should be:

- **Immediate**: See changes within seconds of saving
- **Safe**: Invalid configs don't break the prompt
- **Predictable**: Clear feedback when reload succeeds/fails

### Requirements

1. **Hot-reload**: Detect file changes and reload automatically
2. **Validation**: Reject invalid TOML with clear errors
3. **Fallback**: Keep old config if new one is invalid
4. **Notification**: Tell Fish to update prompt after reload

### User Workflow

> **Superseded by [ADR-0007](adr-0007-sigurg-doorbell-notifications.md):** reloads now use a `<pid>.reload` flag plus SIGURG; SIGUSR2 is unused.

```
1. User edits ~/.config/gpy/themes/custom.toml
2. Saves file
3. Agent detects change (within 1-5 seconds)
4. Agent reloads theme
5. Agent sends SIGUSR2 to Fish processes
6. Fish reloads theme variables
7. Next prompt shows new colors/icons
```

## Decision

Implement **hot-reload with filesystem watching** for both themes and configuration.

### Architecture

**Two separate watchers:**
1. **Theme Watcher**: Watches `~/.config/gpy/themes/*.toml`
2. **Config Watcher**: Watches `~/.config/gpy/config.toml`

**Shared Mechanism:**
- Both use `notify` crate with `WatchCoordinator`
- Both have 5-second debounce (editor save patterns)
- Both validate before applying
- Both notify Fish on success

### Reload Flow

> **Superseded by [ADR-0007](adr-0007-sigurg-doorbell-notifications.md):** reloads now use a `<pid>.reload` flag plus SIGURG; SIGUSR2 is unused.

```rust
// Theme reload
File Change → Debouncer (5s) → Load & Validate → Apply → SIGUSR2 to Fish

// Config reload
File Change → Debouncer (5s) → Load & Validate → Apply → Callback
                                                            ↓
                                              Update dependent state (theme switch, etc.)
```

### Theme Reload Implementation

```rust
// Watcher and poll-fallback reloads go through the agent's callback, which
// serializes the reload, refreshes caches, then rings the doorbell.
theme_manager.set_reload_callback(Arc::new(move |reload| {
    events::handle_theme_reload(&ctx, reload); // calls reload(), then notify_reload() if it changed
}));
```

### Config Reload with Callback

```rust
pub fn set_reload_callback(&self, callback: ConfigReloadCallback) {
    let mut cb_guard = self.reload_callback.lock().unwrap();
    *cb_guard = Some(callback);
}

// In reload:
if let Some(ref callback) = *reload_callback.lock().unwrap() {
    callback(&old_config, &new_config);
}
```

## Consequences

### Positive

> **Superseded by [ADR-0007](adr-0007-sigurg-doorbell-notifications.md):** reloads now use a `<pid>.reload` flag plus SIGURG; SIGUSR2 is unused.

1. **UX**: Feedback on changes without restarts
   - No need to restart agent
   - No need to restart shell
   - Rapid iteration on themes

2. **Safe**: Invalid configs don't break things
   - Validation before application
   - Old config kept on error
   - Clear error messages in logs

3. **Extensible**: Can trigger dependent updates
   - Config callback can switch themes
   - Can restart watchers with new settings
   - Can invalidate caches

4. **Consistent**: Same pattern for themes and config
   - Reusable `WatchCoordinator`
   - Similar debouncing behavior
   - Same notification mechanism (SIGUSR2)

### Negative

1. **Complexity**: Multiple watchers to coordinate
   - Theme watcher
   - Config watcher
   - Git filesystem watcher
   - Must not interfere with each other

2. **Timing**: Debouncing can feel slow
   - 5-second delay for themes/config
   - Users expect instant (< 1s)
   - Trade-off: responsiveness vs editor save patterns

3. **State Management**: Callback can create circular deps
   - Config callback can change theme
   - Theme change triggers watcher restart
   - Must avoid infinite loops

4. **Error Handling**: Silent failures possible
   - File watcher may fail to start
   - Callback may panic
   - Users don't see errors unless checking logs

### Mitigation Strategies

> **Superseded by [ADR-0007](adr-0007-sigurg-doorbell-notifications.md):** reloads now use a `<pid>.reload` flag plus SIGURG; SIGUSR2 is unused.

1. **Clear Logging**
   - Debug log all reload attempts
   - Log validation errors
   - Log callback execution

2. **User Feedback**
   - SIGUSR2 on success (prompt updates immediately)
   - Warning messages for validation errors
   - Troubleshooting guide in docs

3. **Safe Callbacks**
   - Callbacks wrapped in panic handler
   - Errors logged, not propagated
   - Old state preserved on failure

4. **Debounce Tuning**
   - 5s is conservative (handles multi-file saves)
   - Could be configurable in future
   - 1s might be better for single-file edits

## Implementation Details

### Debounce Period Selection

**Why 5 seconds for config/theme?**
- Editors often save multiple times (backup, swap files)
- Vim `:w` can trigger 2-3 writes
- 5s ensures we only reload once per save "session"

**Why 150ms for git watcher?**
- Git operations are fast (add, commit)
- No multi-file patterns
- Users want instant feedback

### SIGUSR2 Choice

> **Superseded by [ADR-0007](adr-0007-sigurg-doorbell-notifications.md):** reloads now use a `<pid>.reload` flag plus SIGURG; SIGUSR2 is unused.

**Why SIGUSR2 (not SIGUSR1)?**
- SIGUSR1 already used for prompt repaints (git changes)
- SIGUSR2 for theme reloads (less frequent)
- Semantically different: "repaint" vs "reload vars"

**Fish Handler:**
```fish
function __gpy_handle_sigusr2 --on-signal SIGUSR2
    __gpy_load_theme_vars  # Re-export Fish variables
    commandline -f repaint  # Repaint prompt
end
```

### Callback Pattern

**Why callbacks for config reload?**
- Config changes can affect multiple subsystems
- Theme switch: update ThemeManager
- Watcher toggle: restart filesystem watcher
- Language cache TTL: update version cache

**Callback receives both old and new config:**
```rust
type ConfigReloadCallback = Arc<dyn Fn(&Config, &Config) + Send + Sync>;
```

This allows diffing to only update what changed.

## Edge Cases Handled

- **File deleted**: Reload fails, keep old config
- **Invalid TOML**: Parse error logged, old config kept
- **Missing fields**: Validation error, old config kept
- **Partial save**: Debouncer waits for final save
- **Concurrent edits**: RwLock ensures atomic updates
- **Circular callback**: Agent tracks reload depth, prevents loops

## Performance Impact

- **Memory**: ~1MB per watcher (notify overhead)
- **CPU**: < 0.1% idle, ~2% during reload
- **Latency**: 5s debounce + 10-50ms reload + 5-20ms Fish repaint

## Related

- [ADR-0004](adr-0004-sigusr1-live-updates.md) - Signal-based updates
- [theme/manager.rs](../../../gpy-agent/src/theme/manager.rs) - Theme hot-reload
- [config/manager.rs](../../../gpy-agent/src/config/manager.rs) - Config hot-reload
- [watcher/](../../../gpy-agent/src/watcher/) - Filesystem watcher

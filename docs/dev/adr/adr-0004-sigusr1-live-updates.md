# ADR-0004: SIGUSR1 for Live Prompt Updates

## Status

Accepted (2024-11)

Superseded in part by [ADR-0007](adr-0007-sigurg-doorbell-notifications.md): the agent now sends SIGURG instead of SIGUSR1.

## Context

Users expect their prompt to update in real-time when the working directory changes:
- File modifications in a git repo should update git status indicators
- Creating/deleting files should trigger prompt refresh
- Branch switches should immediately reflect in prompt

Without live updates, users must press Enter or run a command to see the updated prompt.

### Requirements

1. **Responsive**: Update within 150-500ms of filesystem change
2. **Non-intrusive**: Don't interrupt user typing
3. **Efficient**: Don't poll constantly (battery drain)
4. **Safe**: Work with any Fish shell version

### Considered Alternatives

1. **Polling (check every N seconds)**
   - ✅ Simple to implement
   - ❌ Battery drain (continuous CPU usage)
   - ❌ Slow response (up to N seconds delay)
   - ❌ Unnecessary work when idle

2. **Fish Event Handlers (on directory change)**
   - ✅ Native Fish integration
   - ❌ Only triggers on `cd` commands
   - ❌ Misses file changes in current directory
   - ❌ No support for external changes

3. **SIGUSR1 Signal + Filesystem Watching**
   - ✅ Event-driven (no polling)
   - ✅ Fast response (< 500ms)
   - ✅ Battery efficient (idle when no changes)
   - ✅ Works across all shells
   - ⚠️ Requires signal handling in Fish

## Decision

Use **SIGUSR1 signal** to notify Fish shell of prompt updates, triggered by a **filesystem watcher** in the Rust agent.

### Architecture

```
File Change → Watcher (notify crate) → Debouncer (100ms) → Signal Sender
                                                                  ↓
                                                            SIGUSR1 → Fish Process
                                                                  ↓
                                                      Fish Trap Handler → Prompt Repaint
```

### Implementation Details

**Rust Agent (gpy-agent):**
- Uses `notify` crate for filesystem watching
- Watches `.git/` directory and worktree
- Debounces events (100ms) to avoid signal spam
- Sends SIGUSR1 to registered Fish processes
- Tracks PIDs in client registry

**Fish Shell (init.fish):**
```fish
function __gpy_handle_sigusr1 --on-signal SIGUSR1
    # Re-render prompt without adding newline
    commandline -f repaint
end
```

**Signal Safety:**
- SIGUSR1 is user-defined, safe to use
- Fish handles signals gracefully
- No data race (just triggers repaint)

**Debouncing:**
- 100ms default (configurable)
- Coalesces multiple rapid changes
- Per-repository tracking
- Minimum 50ms between signals per client

## Consequences

### Positive

1. **Instant Updates**: Prompt refreshes automatically
   - Git status updates on file save
   - Branch changes immediately visible
   - No need to press Enter

2. **Battery Efficient**: Event-driven, no polling
   - Watcher uses inotify (Linux) or FSEvents (macOS)
   - Agent sleeps when no changes
   - Minimal CPU usage

3. **User Experience**: Feels magical
   - "It just works" - no user action needed
   - Mirrors VS Code's file watching
   - Professional, polished UX

4. **Flexible**: Can trigger any Fish function
   - Not just prompt repaints
   - Could trigger custom user functions
   - Extensible for future features

### Negative

1. **Complexity**: More moving parts
   - Filesystem watcher code
   - Signal handling in Fish
   - PID tracking and cleanup
   - Debouncing logic

2. **Edge Cases**: Signals can be tricky
   - Must handle process crashes (stale PIDs)
   - Race conditions possible (rare)
   - Signal delivery not guaranteed

3. **Debugging**: Async behavior harder to debug
   - Updates happen "magically"
   - Timing issues non-deterministic
   - Must log signal delivery for troubleshooting

4. **Platform Differences**: Watcher behavior varies
   - Linux inotify vs macOS FSEvents
   - Different event granularity
   - WSL has some limitations

### Mitigation Strategies

1. **Robust PID Management**
   - Periodic cleanup of dead PIDs (every 60s)
   - Validate PID existence before signaling
   - Handle ESRCH (no such process) gracefully

2. **Comprehensive Logging**
   - Debug log all signal deliveries
   - Track watcher events
   - Log debouncer state

3. **Disable Option**: Respect user preference
   - `GPY_DISABLE_WATCHER=1` environment variable
   - Config option: `watcher.enabled = false`
   - Clear documentation

4. **Graceful Degradation**
   - If signals fail, fall back to manual updates
   - Show warning only once (not on every prompt)
   - Provide troubleshooting guide

## Performance Characteristics

**Latency Breakdown:**
- Filesystem event detected: 0-50ms
- Debouncer delay: 100ms (configurable)
- Signal delivery: < 1ms
- Fish repaint: 5-20ms
- **Total**: 105-170ms typical

**Resource Usage:**
- Memory: ~2MB for watcher
- CPU: < 0.1% when idle
- Battery: Negligible impact

## Edge Cases Handled

- **Multiple Fish instances**: Each PID tracked separately
- **Fish process crash**: PID cleanup detects and removes
- **Agent restart**: Clients re-register on next prompt
- **Debouncer batching**: Multiple rapid changes → single signal
- **Symlink changes**: Watcher follows symlinks correctly

## Security Considerations

- **Signal safety**: SIGUSR1 is safe, doesn't interrupt syscalls
- **PID validation**: Verify PID belongs to Fish process (via `/proc`)
- **No shell injection**: Signal only, no command execution
- **User isolation**: Only signal user's own processes

## Related

- [ADR-0001](adr-0001-hybrid-rust-fish-architecture.md) - Architecture overview
- [ADR-0006: Async Terminal Updates](adr-0006-async-terminal-updates.md) - Implementation details
- [watcher/](../../../gpy-agent/src/watcher/) - Watcher implementation

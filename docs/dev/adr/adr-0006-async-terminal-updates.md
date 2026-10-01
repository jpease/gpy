# ADR-0006: Async Terminal Updates

## Status

Accepted (2024-11)

## Context

This document explains the async postexec update mechanism that ensures Terminal A (where git commands are executed) receives prompt updates without blocking delays.

## The Problem

When you run a git command in Terminal A, a race condition occurs:

```
Timeline:
1. User runs: git add .                    (Terminal A)
2. Git modifies .git/index file
3. Command completes, Fish renders prompt   (Shows STALE git status - cached)
4. [50-200ms later] File watcher detects change
5. Agent sends SIGUSR1 to all terminals
6. Terminal B receives signal → updates     (Shows FRESH git status)
7. Terminal A receives signal → ignored     (Prompt already rendered with stale data)
```

**Result**: Terminal B (observer) updates immediately, but Terminal A (executor) shows stale data until the next command.

## Solution: Async Update with fish_postexec

After git commands that modify repository state, we:

1. **Immediately render the prompt** (no blocking delay)
2. **Spawn a background job** that waits for the SIGUSR1 signal
3. **When SIGUSR1 arrives**, trigger a prompt repaint
4. **If no signal arrives**, timeout after 250ms and cleanup

### Implementation

Located in `/core/ipc.fish`:

```fish
function __gpy_postexec_async_update --on-event fish_postexec
    # Only trigger for git commands that modify repository state
    set -l cmd (string match -r '^git\s+(add|commit|checkout|...)' -- $argv[1])

    if test -n "$cmd"
        # Set flag indicating we're waiting for update
        set -g __gpy_awaiting_git_update 1

        # Spawn background job that waits for SIGUSR1
        fish -c '
            for i in (seq 1 5)  # 5 × 50ms = 250ms timeout
                sleep 0.05
                if not set -q __gpy_awaiting_git_update
                    # Signal received! Repaint prompt
                    commandline -f repaint 2>/dev/null
                    exit 0
                end
            end
            # Timeout - cleanup
            set -e __gpy_awaiting_git_update
        ' &
        disown $last_pid 2>/dev/null
    end
end
```

### SIGUSR1 Handler Enhancement

```fish
# Signal handler sets a variable (cannot call commandline directly)
function __gpy_sigusr1_handler --on-signal SIGUSR1
    # Clear the awaiting flag (terminates background job)
    set -e __gpy_awaiting_git_update

    # Trigger repaint via variable change
    # This is the only reliable way to force prompt repaint in Fish
    set -g __gpy_repaint_trigger (math (set -q __gpy_repaint_trigger; and echo $__gpy_repaint_trigger; or echo 0) + 1)
end

# Variable change handler triggers the actual repaint
function __gpy_repaint_on_variable --on-variable __gpy_repaint_trigger
    commandline -f repaint 2>/dev/null
end
```

## User Experience

### Before (Baseline)
- Terminal A: Shows stale data until next command
- Terminal B: Updates immediately ✅
- Prompt feels: Instant, but data is stale

### After (With Async Update)
- Terminal A: Shows fresh data 50-250ms after command ✅
- Terminal B: Updates immediately ✅
- Prompt feels: Instant, then natural "flicker" to fresh data

### Compared to Alternatives

| Approach | Prompt Latency | Terminal A Updates | UX Quality |
|----------|----------------|-------------------|------------|
| **Blocking Delay** | +200ms | ✅ Yes | ⛔ Sluggish |
| **Cache Invalidation** | +5-10ms | ❌ No | ❌ No improvement |
| **Async Update** | 0ms | ✅ Yes (50-250ms later) | ✅ Natural |
| **Do Nothing** | 0ms | ❌ No | ✅ Standard |

## Git Commands Affected

### Commands that trigger async updates:
- `git add`, `git rm`, `git mv`
- `git commit`, `git commit --amend`
- `git checkout`, `git switch`
- `git merge`, `git rebase`, `git cherry-pick`, `git revert`
- `git pull`, `git push`, `git fetch`
- `git stash`, `git stash pop`
- `git reset`, `git restore`
- `git tag`
- `git branch -d/-D/-m/-M`

### Commands that DON'T trigger (read-only):
- `git log`, `git show`, `git diff`
- `git status`, `git branch` (without modifying flags)
- Any non-git commands

## Performance Impact

- **No blocking delay**: Prompt appears instantly
- **Background job overhead**: <1ms to spawn, <1ms CPU every 50ms for max 250ms
- **Memory overhead**: Negligible (one background Fish process per git command)
- **Prompt repaints**: 2× (initial + update), both non-blocking

## Testing

### Postexec Async Update Tests

Comprehensive test suite at `/tests/fish/postexec_async_update.test.fish`:

```bash
fish tests/fish/postexec_async_update.test.fish
```

Tests validate:
1. Modifying git commands set the awaiting flag
2. Read-only git commands don't set the flag
3. Non-git commands don't set the flag
4. All known modifying commands trigger correctly
5. Background job initializes properly
6. SIGUSR1 handler clears the flag

### SIGUSR1 Repaint Regression Tests

**Critical regression tests** originally lived in `sigusr1_repaint.test.fish`. Since [ADR-0007](adr-0007-sigurg-doorbell-notifications.md) replaced SIGUSR1 with the SIGURG doorbell, they are in `tests/fish/doorbell_signal.test.fish`:

```bash
fish tests/fish/doorbell_signal.test.fish
```

These tests prevent regressions to broken repaint patterns:

1. ✅ Verify SIGUSR1 handler exists and is registered
2. ✅ Verify variable change handler exists and is registered
3. ✅ Test the variable-based repaint pattern works
4. ✅ Verify SIGUSR1 handler clears awaiting update flag
5. ✅ Verify SIGUSR1 handler increments trigger variable
6. ✅ **Regression check**: Ensure we're NOT using broken patterns:
   - `emit fish_prompt` directly (doesn't work)
   - `commandline -f repaint` in signal handler (doesn't work)
   - Must use variable trigger pattern (only thing that works)

**Run both test suites:**
```bash
fish tests/fish/postexec_async_update.test.fish && \
fish tests/fish/doorbell_signal.test.fish
```

## Maintenance Notes

### Why the regex pattern matches specific commands

The postexec handler uses a regex to match git commands that modify state:

```fish
set -l cmd (string match -r '^git\s+(add|commit|checkout|...)' -- $argv[1])
```

**IMPORTANT**: This pattern must be kept in sync with git commands that actually modify `.git/` files. If new git commands are added that change repository state, update the regex.

### Why we use a background job instead of inline waiting

**Option A (Inline - Bad)**:
```fish
sleep 0.2  # Blocks the prompt!
commandline -f repaint
```

**Option B (Background - Good)**:
```fish
fish -c 'sleep 0.05 × 5 then repaint' &
```

The background job allows the prompt to render immediately while waiting for the signal asynchronously.

### Critical: How to Force Prompt Repaints in Fish

**IMPORTANT**: This section documents what works and doesn't work for forcing visual prompt updates. If regressions occur, refer to this information.

#### What DOESN'T Work ❌

These approaches **will NOT** cause an idle prompt to visually repaint:

1. **`emit fish_prompt`** - Triggers the event but doesn't redraw the visible prompt
   ```fish
   function handler --on-signal SIGUSR1
       emit fish_prompt  # ❌ Event fires, but prompt stays stale
   end
   ```

2. **`commandline -f repaint`** - From signal handlers, this fails silently
   ```fish
   function handler --on-signal SIGUSR1
       commandline -f repaint  # ❌ Doesn't work from signal context
   end
   ```

3. **`commandline -f force-repaint`** - Same issue as `repaint`
   ```fish
   function handler --on-signal SIGUSR1
       commandline -f force-repaint  # ❌ Still doesn't work
   end
   ```

4. **Terminal control sequences** - Don't trigger Fish prompt re-rendering
   ```fish
   echo -ne '\r\033[K'  # ❌ Clears line but doesn't call fish_prompt
   ```

#### What DOES Work ✅

The **only reliable approach** is the **variable change event pattern** (used by Hydro/Tide):

```fish
# Signal handler sets a variable
function __gpy_sigusr1_handler --on-signal SIGUSR1
    # Increment a counter to trigger the variable event
    set -g __gpy_repaint_trigger (math (set -q __gpy_repaint_trigger; and echo $__gpy_repaint_trigger; or echo 0) + 1)
end

# Variable change handler calls repaint
function __gpy_repaint_on_variable --on-variable __gpy_repaint_trigger
    commandline -f repaint  # ✅ This WORKS from variable event context
end
```

**Why this works**:
- Fish allows `commandline -f repaint` from `--on-variable` event handlers
- Fish does NOT allow it from `--on-signal` event handlers
- This is a fundamental Fish limitation, not a bug

**Reference implementations**:
- [Hydro](https://github.com/jorgebucaran/hydro) - Uses `--on-variable` for async git updates
- [Tide](https://github.com/IlanCosman/tide) - Uses `--on-variable` for async rendering
- [fish-async-prompt](https://github.com/acomagu/fish-async-prompt) - Uses SIGUSR1 + variable pattern

#### Historical Context (Preventing Regressions)

**Commit a0b2928** ("Clean & Green and clock live update working"):
- Added SIGUSR1 handler using `emit fish_prompt`
- This worked for clock updates because they happen at regular intervals
- **Did NOT work** for git updates at idle prompts

**What we tried that failed**:
1. Direct `commandline -f repaint` in signal handler
2. `commandline -f force-repaint` in signal handler
3. Conditional logic checking `commandline --is-valid`
4. Control sequences (`\r\033[K`) + `emit fish_prompt`

**Final working solution** (current implementation):
- SIGUSR1 handler → sets variable
- Variable change handler → calls `commandline -f repaint`
- This is the **only** pattern that works for idle prompt repaints

### Why we disown the background job

```fish
disown $last_pid 2>/dev/null
```

This prevents the background job from:
- Appearing in `jobs` output
- Interfering with job control (Ctrl-Z, fg, bg)
- Blocking shell exit

## Debugging

Enable debug logging:

```bash
# Terminal output shows when SIGUSR1 arrives
tail -f /tmp/gpy-fish-sigusr1.log

# Check if background jobs are accumulating
jobs | grep -c fish
```

If you see many background `fish` processes, it may indicate:
- The timeout isn't working (should auto-cleanup after 250ms)
- SIGUSR1 signals aren't arriving (file watcher issue)
- The disown command is failing

## Future Enhancements

Possible improvements:

1. **Adaptive timeout**: Measure file watcher latency and adjust timeout dynamically
2. **Command classification**: Use git's `--dry-run` flag to determine if command modifies state
3. **Visual feedback**: Show subtle indicator when waiting for async update
4. **Configurable delay**: Allow users to adjust the 50ms polling interval

## References

- Issue: "Terminal A doesn't update after git commands"
- Implementation: `/core/ipc.fish` (lines 538-625)
- Tests: `/tests/fish/postexec_async_update.test.fish`
- UX Analysis: Generated during implementation showing all alternatives considered

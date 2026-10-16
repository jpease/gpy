# ADR-0006: Async Terminal Updates and the Fish Repaint Pattern

## Status

Accepted (2024-11). Partly superseded:

- [ADR-0007](adr-0007-sigurg-doorbell-notifications.md) replaced SIGUSR1 with the SIGURG doorbell.
- The `fish_postexec` background job described in the original version of this ADR is gone. The terminal that ran the git command is a registered shell like any other, so the agent's SIGURG broadcast after it recomputes git status repaints that terminal too; no per-command waiter is needed.

The repaint pattern below is current and is what `fish/core/ipc.fish` implements.

## Context

When a git command runs in Terminal A, Fish renders A's next prompt before the agent's watcher has seen the change, so that prompt shows the cached (stale) status. Other terminals in the same repository are idle and must also redraw once the agent has fresh data. Both cases need a way to repaint an idle Fish prompt from a signal.

## Decision

Repaint from a signal through a variable-change event:

```fish
function __gpy_doorbell_handler --on-signal SIGURG
    # ... consume .reregister / .reload flag files ...
    set -g __gpy_repaint_trigger (math (set -q __gpy_repaint_trigger; and echo $__gpy_repaint_trigger; or echo 0) + 1)
end

function __gpy_repaint_on_variable --on-variable __gpy_repaint_trigger
    commandline -f force-repaint 2>/dev/null
end
```

Fish honours `commandline -f repaint`/`force-repaint` from an `--on-variable` handler but not from an `--on-signal` handler, so the signal handler only bumps a counter. The same pattern is used by [Hydro](https://github.com/jorgebucaran/hydro), [Tide](https://github.com/IlanCosman/tide) and [fish-async-prompt](https://github.com/acomagu/fish-async-prompt).

### Approaches that do not repaint an idle prompt

1. `emit fish_prompt` from the signal handler: the event fires but the visible prompt is not redrawn.
2. `commandline -f repaint` or `commandline -f force-repaint` directly in the signal handler: fails silently.
3. Terminal control sequences such as `\r\033[K`: clear the line without calling `fish_prompt`.

## Consequences

- Terminal A shows the stale status immediately, then repaints when the agent's doorbell arrives; the prompt never blocks waiting for the watcher.
- Every doorbell costs one prompt repaint per registered Fish shell; the agent throttles repaint doorbells (`GPY_SIGURG_THROTTLE_MS`).
- `tests/fish/doorbell_signal.test.fish` asserts that a ring bumps `__gpy_repaint_trigger`; `tests/fish/e2e_interactive_session.test.fish` asserts the visible repaint on a real pty.

## Related

- [ADR-0004](adr-0004-sigusr1-live-updates.md) - Watcher-driven live updates
- [ADR-0007](adr-0007-sigurg-doorbell-notifications.md) - SIGURG doorbell and flag files

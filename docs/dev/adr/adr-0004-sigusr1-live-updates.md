# ADR-0004: Signal-Driven Live Prompt Updates

## Status

Accepted (2024-11). Partly superseded:

- [ADR-0007](adr-0007-sigurg-doorbell-notifications.md): the agent sends SIGURG, not SIGUSR1, and Bash installs no handler.
- The original version of this ADR showed the Fish handler calling `commandline -f repaint` directly. That does not repaint an idle prompt from signal context; the handler bumps a variable and an `--on-variable` handler repaints, as described in [ADR-0006](adr-0006-async-terminal-updates.md).

The decision itself, a filesystem watcher in the agent that signals registered shells, is current.

## Context

A prompt that shows git state goes stale when files change while the shell sits idle, or when another terminal changes the repository. Without live updates the user has to press Enter to see the current state.

Requirements:

1. Update within a few hundred milliseconds of a filesystem change.
2. Never interrupt typing.
3. No constant polling while nothing changes.

### Alternatives considered

1. **Polling every N seconds**: simple, but costs CPU while idle and lags by up to N seconds.
2. **Shell event handlers on `cd`**: native, but miss changes made in the current directory or by other processes.
3. **Agent-side filesystem watcher plus a signal to each registered shell**: event-driven and idle when nothing changes; requires a signal handler in each shell.

## Decision

The agent watches registered repositories with the `notify` crate (FSEvents on macOS, inotify on Linux), debounces events, recomputes state, and signals the registered shell PIDs for that repository.

```
File change -> watcher (notify) -> debouncer -> recompute -> signal registered shells -> repaint
```

- Debounce window: `GPY_DEBOUNCE_MS`, default 100 ms.
- Minimum interval between repaint signals to one shell: `GPY_SIGUSR1_THROTTLE_MS`, default 150 ms.
- `GPY_DISABLE_WATCHER=1` disables the repository watcher.
- A periodic reconcile (default 45 s) catches changes the watcher missed, and polling backends exist for filesystems that deliver no events (see [Troubleshooting](../../user/troubleshooting.md#diagnostic-and-fallback-environment-variables)).

## Consequences

### Positive

- Idle Fish and Zsh prompts show repository changes without a keystroke.
- No work while nothing changes.

### Negative

- More moving parts: watcher, debouncer, PID registry and cleanup, a handler in each shell.
- Signal delivery is not guaranteed and watcher behaviour differs by platform; the reconcile pass and the per-prompt render bound how long a missed event can leave the prompt stale.
- Asynchronous behaviour is harder to debug than a synchronous render.

## Related

- [ADR-0001](adr-0001-hybrid-rust-fish-architecture.md) - Architecture overview
- [ADR-0006](adr-0006-async-terminal-updates.md) - Fish repaint pattern
- [ADR-0007](adr-0007-sigurg-doorbell-notifications.md) - SIGURG doorbell and flag files
- [watcher/](../../../gpy-agent/src/watcher/) - Watcher implementation

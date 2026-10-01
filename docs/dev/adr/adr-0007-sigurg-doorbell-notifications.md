# ADR-0007: SIGURG Doorbell and Flag Files for Shell Notifications

## Status

Accepted (2026-10). Supersedes in part [ADR-0004](adr-0004-sigusr1-live-updates.md) (the signal used for repaints) and [ADR-0005](adr-0005-theme-hot-reload.md) (the signal used for reloads).

## Context

The agent used three signals to talk to registered shells: SIGUSR1 (repaint), SIGUSR2 (reload theme/config) and SIGALRM (re-register after an agent restart). All three have a default disposition of *terminate*.

A shell that runs `exec fish` (or `exec bash` / `exec zsh`) keeps its PID and process start time, and `fish_exit` does not fire on `exec`, so the PID stays registered with the agent. Between the `exec` and the moment the new shell installs its handlers, any of those signals kills the shell and the terminal pane closes (jpease/gpy-archive#674).

## Decision

The agent sends exactly one signal to shells: **SIGURG**, whose default disposition is *ignore* on macOS and Linux. A process that has not yet installed a handler is unaffected.

Meaning travels in empty flag files in the existing shell tracking directory (`<runtime_root>/shells/`), next to the `<pid>` tracking file:

| File | Written by | Meaning |
|---|---|---|
| `<pid>.reregister` | agent, before SIGURG | forget the registration and register again |
| `<pid>.reload` | agent, before SIGURG | reload theme/config variables |

Each shell installs one SIGURG handler (fish `--on-signal SIGURG`, bash `trap ... URG`, zsh `TRAPURG`) which:

1. removes `<pid>.reregister` if present and re-registers;
2. removes `<pid>.reload` if present and reloads theme/config;
3. repaints, as before.

With no flag present, SIGURG is a plain repaint. Existence checks are shell builtins, so the common path forks nothing. Reload notifications are never throttled, so a written flag is always followed by a signal. The agent removes flags for dead PIDs; shells remove their own flags on exit. The agent restart marker file is gone: re-registration is driven by the flag. The IPC protocol version is bumped to 2.

`GPY_SIGUSR1_THROTTLE_MS` keeps its name and now throttles repaint notifications.

## Alternatives Considered

1. **Install no-op handlers as early as possible in each shell.** Rejected: the window between `exec` and the first line of user config still exists during the shell's own startup (fish reads its own configuration before ours runs), so a signal can still land on the default disposition.
2. **Unregister the PID in `install-dev` (or other tooling) before `exec`.** Rejected: covers one entry point only; users `exec` their shell from anywhere.
3. **Three distinct signals whose default is ignore.** The only candidates are SIGWINCH, SIGCONT and SIGCHLD, and each carries real shell semantics (terminal resize, job control, child reaping). Overloading them would misfire on ordinary events. One ignore-by-default signal plus flag files carries the same information safely.

## Consequences

### Positive

- A shell with no handler (mid-`exec`, mid-startup) survives every agent notification.
- One handler per shell instead of three.

### Negative

- Reload and re-register need a filesystem write before the signal.
- Protocol version 2 is incompatible with shell scripts expecting version 1; shells and agent must be upgraded together.

## References

- jpease/gpy-archive#674
- [ADR-0004](adr-0004-sigusr1-live-updates.md), [ADR-0005](adr-0005-theme-hot-reload.md)

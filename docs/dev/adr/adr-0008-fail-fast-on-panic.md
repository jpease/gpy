# ADR-0008: Fail Fast on Panic

## Status

Accepted (2026-10).

## Context

Shipped binaries are built with `panic = "abort"`: it is set in `[profile.release]` in `gpy-agent/Cargo.toml` (and in `gpy-agent/.cargo/config.toml`), and the `release-dist` profile inherits it. Until now nothing recorded why.

The agent runs several long-lived threads that nothing supervises: the watcher flush thread (`watcher/mod.rs`), the config and theme hot-reload poll threads (`watcher/hot_reload.rs`), and the event-dispatch threads (`agent/events/mod.rs`). Under unwinding, a panic on one of these threads ends that thread only. The daemon keeps answering IPC, so shells see a healthy agent, but live updates or hot reload have silently stopped, and nothing restarts them.

The code still carries handling written for unwinding: `JobGuard` (`ipc/handlers/mod.rs`) completes a single-flight job whose worker panicked, `run_reconcile_pass` (`agent/mod.rs`) logs a `JoinError`, and lock sites recover a poisoned mutex with `PoisonError::into_inner`.

## Decision

Keep `panic = "abort"` for release builds. Any panic ends the daemon at once.

Recovery is the shells' job, as it already is for any agent exit: Fish autostarts the agent (bounded by `GPY_AGENT_AUTOSTART_MAX_ATTEMPTS` and `GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS`), and the Bash and Zsh supervisors (`bash/core/supervisor.bash`, `zsh/core/supervisor.zsh`) start it again. A restarted agent asks tracked shells to re-register ([ADR-0007](adr-0007-sigurg-doorbell-notifications.md)), so live updates resume.

The unwinding-era handling stays. Cargo ignores the `panic` setting for tests and benchmarks, which always unwind, so `JobGuard`, the `JoinError` logging and the poison recovery still decide test behaviour. `PoisonError::into_inner` is also how a lock is taken without `unwrap()` or `expect()`, which the crate's lint policy denies.

## Alternatives Considered

1. **Switch release builds to `panic = "unwind"` and contain panics.** Rejected: the threads above are unsupervised, so a contained panic would leave a daemon that answers pings but no longer repaints prompts. A clean exit that the shells notice and recover from is better than a silent partial failure.
2. **Unwind and supervise every thread** (restart a thread when it dies). Rejected for now: it adds a restart policy to each subsystem, to handle a failure that a process restart already covers.

## Consequences

### Positive

- A panic produces one observable failure, a dead agent, which existing shell recovery handles.
- No half-alive daemon whose live updates have stopped.

### Negative

- A panic in any request handler also ends the daemon, together with every in-flight request. Shells fall back to their agent-down behaviour until the restart.
- A panic that recurs on startup makes a crash loop, bounded only by the shells' restart limits.

### Revisit when

Reopen this decision if a crash loop is observed in practice, or if the agent gains a supervisor for its threads.

## References

- [ADR-0007](adr-0007-sigurg-doorbell-notifications.md): re-registration after an agent restart
- `gpy-agent/Cargo.toml` `[profile.release]`

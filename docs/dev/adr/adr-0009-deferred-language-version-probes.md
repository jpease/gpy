# ADR-0009: Deferred Language Version Probes

## Status

Accepted (2026-10).

## Context

The language segment shows a version per displayed language. A version comes from the version cache (`language/version.rs`, keyed by language and project directory) or, on a miss, from a probe: running `mise` or the tool itself (`node --version`, a forwarded venv's `python`, and so on). Each probe is bounded by `VERSION_COMMAND_TIMEOUT` (5 s), and a `mise` timeout followed by the direct command makes 10 s.

Until now a `lang` request ran those probes inline. `LanguageHandler::handle_language_detection` wrote the instant-cache variants and built its reply through `build_language_display_info_at`, which probes any cold version and joins before returning. Detection itself was already bounded by `agent.timeout_seconds` and continued in the background on a timeout, but a cold version made the request wait for the probe, and a slow tool held a blocking-pool thread and a connection permit for the whole time.

## Decision

The request path never runs a version probe.

- `language/version.rs` has a `ProbePolicy` (`Spawn`, `CachedOnly`); `lookup_language_release_at` and `PythonVenv::lookup_version` return `ColdVersion` instead of probing under `CachedOnly`. `cached_language_display_info_at` (`language/display.rs`) builds the display entries from the caches and is `None` when any displayed version is cold. `build_language_display_info_at` keeps probing, for oneshot, the wizard and background work.
- On a cold version, `LanguageHandler` starts a probe job, one per project root (`SingleFlight`), that runs the probing build off the request path, writes the instant cache for the request's key and `prev_bg`, and rings the shells through `notify_if_changed`. That is the repaint path git status already uses (ADR-0006, ADR-0007), so idle Fish and Zsh prompts repaint and Bash updates at its next prompt.
- The request waits for the job only for what is left of its `agent.timeout_seconds` budget, which it shares with detection. If the job finishes in time the reply carries the versions; otherwise the request fails with a timeout, as a slow detection does, rather than return a version-less reply that would clobber a good prompt. A finished job has cached every result, success or failure, so the request then reads the caches with `settled_language_display_info_at`: a version that is cold again by then (expired, evicted, or read as stale after the wall clock stepped back) shows as none instead of failing an in-budget request.
- The instant cache takes resolved `LanguageInfo` entries, so writing it never probes. Each publish resolves versions once, instead of once per position and render context.

## Alternatives Considered

1. **Answer at once with the version missing and fill it in later.** Rejected: the renderer drops a version-less language unless the theme sets `show_symbol_without_version`, so the segment would vanish and reappear on every cold probe.
2. **A short fixed wait, like git status's 100 ms fast window.** Rejected for now: most probes finish in tens of milliseconds but tool shims are often slower, and git has a stale value to serve meanwhile where a cold version has nothing. Waiting for the remaining request budget keeps fast and moderately slow tools answering inline.

## Consequences

### Positive

- A `lang` request never waits on a version probe beyond `agent.timeout_seconds`.
- Concurrent requests for one project share one probe job.
- Writing the instant cache no longer probes, once per variant, inside the cache module.
- The watcher's language refreshes (`agent/events/mod.rs`) all run on the blocking pool, so a marker edit that invalidates the version cache never holds the event thread on a probe.

### Negative

- A cold probe slower than the budget costs one failed request before its result arrives through the repaint path.
- A request and a watcher refresh that find the same version cold at the same moment each probe it. Sharing the request's single-flight job would need a handle on the `LanguageHandler` inside the agent's event context; the duplicate is bounded by the probe timeout and the version cache absorbs every later lookup.

## References

- [ADR-0006](adr-0006-async-terminal-updates.md): repaint of idle prompts
- [ADR-0007](adr-0007-sigurg-doorbell-notifications.md): the SIGURG doorbell
- `gpy-agent/src/ipc/handlers/language_handler.rs`, `gpy-agent/src/language/display.rs`

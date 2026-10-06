<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Troubleshooting Guide

Shell-aware troubleshooting for GPY: quick diagnosis, a Fish/Zsh/Bash comparison at a glance, and the watcher/environment-variable behavior that governs live prompt updates.

**Last Updated:** 2026-07-07

This guide focuses on what isn't documented elsewhere. For exhaustive detail it links out rather than repeating:

- Install/PATH/binary problems → [Installation Guide](../INSTALL.md#troubleshooting)
- The full Bash feature/version matrix → [Bash Limitations](bash-limitations.md)
- Full `gpy` command syntax → [CLI Reference](cli-reference.md)
- Config file layout, socket priority, security settings → [Advanced Configuration](advanced-configuration.md)

---

## Quick Diagnosis

Try these first, in order, before digging into shell-specific sections below.

```bash
gpy doctor          # Health checks: agent, config TOML, theme, segments, PATH
gpy status           # Is the agent running? Socket path, client count
GPY_DEBUG_LOG=/tmp/gpy-debug.log gpy restart   # Restart with debug logging, then tail the file
```

`gpy doctor` and `gpy status` syntax/output are fully documented in the [CLI Reference](cli-reference.md#gpy-doctor). Debug logging, manual IPC pings via `socat`, and config reset are covered in [Advanced Configuration → Troubleshooting](advanced-configuration.md#troubleshooting).

---

## Shell Comparison at a Glance

Condensed from the full [Bash Limitations](bash-limitations.md) matrix, with Fish and Zsh added for context. Use this to decide "is this symptom expected for my shell?" before filing a bug.

| Feature | Fish | Zsh | Bash 5.x | Bash 3.x (macOS default) |
|---|---|---|---|---|
| Live updates | Full: idle prompt repaints in place | Full: idle prompt repaints in place (re-rendered in `TRAPURG`, #637) | Shown at the next prompt: readline cannot repaint an idle prompt (measured, see [Bash Limitations](bash-limitations.md#2-live-updates)) | Same as Bash 5.x |
| Config/theme hot-reload | Full | Full | Full | Full |
| Transient prompt | Yes | Yes | No | No |
| Duration segment precision | Millisecond | Millisecond (`EPOCHREALTIME`) | Millisecond (`EPOCHREALTIME`) | Not available — segment disabled |
| Relative performance | Fastest (A+) | A | B+ | B- (slowest) |
| Signal mechanism | One `--on-signal SIGURG` handler | Native `TRAPURG` function | No trap: SIGURG ignored, flags read at the next prompt (#678) | Same as Bash 5.x |
| Re-registers after an agent restart | Yes | Yes (#638) | At the next prompt (#638) | At the next prompt |
| Minimum version | 3.6+ | 5.8+ | 4.0+ (5.0+ recommended) | N/A (upgrade recommended) |

See [Bash Limitations](bash-limitations.md) for the exhaustive version-by-version breakdown (duration precision by Bash version, benchmark numbers, `PS1` escape-sequence differences, known issues list) — that document is authoritative for Bash; this table exists only to compare shells side by side.

---

## "My Prompt Isn't Updating Live" (Watcher Issues)

This is the most common class of complaint for a tool like GPY, and the underlying mechanism is not documented elsewhere, so it gets the most detail here.

### How live updates work

The agent watches your repo, `config.toml`, and active theme file for changes using OS-level filesystem notifications (`FSEvents` on macOS, `inotify` on Linux, via the `notify` crate). When something relevant changes, the agent sends `SIGURG` to registered shell processes. A plain `SIGURG` means "repaint the prompt" (git/language state changed); to ask for a theme/config reload or a re-registration the agent first writes an empty `<pid>.reload` or `<pid>.reregister` flag file in the runtime `shells/` directory. `SIGURG` is ignored by default, so a shell that has not installed its handler yet (for example right after `exec fish`) is never killed by a notification. See [ADR-0007](../dev/adr/adr-0007-sigurg-doorbell-notifications.md) for the current protocol and [ADR-0004](../dev/adr/adr-0004-sigusr1-live-updates.md) / [ADR-0005](../dev/adr/adr-0005-theme-hot-reload.md) for the original rationale.

### Why it can silently stop working

OS-level filesystem notifications are not 100% reliable in every environment. Confirmed failure modes:

- **Network-mounted or some Docker/VM-mounted filesystems** don't reliably propagate `inotify`/`FSEvents` events.
- **FSEvents outages on macOS**: in at least one confirmed case, `FSEventsd` delivered zero notifications to the watcher in a sandboxed session even though the daemon was running — a poll-based fallback exists specifically to route around this (issue #354).

When this happens, git status, language versions, and theme edits stop refreshing automatically. The prompt still updates on every new command (Fish/Zsh/Bash all re-invoke prompt rendering per-command regardless of the watcher), but *within* a long-idle session at a single prompt, nothing changes until you press Enter.

### Diagnostic and fallback environment variables

All of these are off/default unless explicitly set — setting none of them preserves current behavior.

| Variable | Effect | Verified at |
|---|---|---|
| `GPY_DISABLE_WATCHER` | Set to `1`/`true`/`yes`/`on` to skip creating the git/language repo file watcher entirely at agent startup. This *only* disables the git/language live-update watcher — it does **not** affect the separate theme or config hot-reload watchers below. Use this to rule the watcher in or out as the cause of a hang or high CPU. | `gpy-agent/src/agent/mod.rs` (`init_watcher`, `setup_hot_reload`) |
| `GPY_THEME_WATCH_POLL_MS` | Unset or `0` (default): disabled — theme reload relies solely on `FSEvents`/`inotify`. Set to a positive integer: spawns an additional poll thread that stats the theme file on that interval (floored at the watcher's debounce) and reloads on an mtime change, independent of OS notifications. | `gpy-agent/src/theme/manager.rs` (`start_poll_fallback`) |
| `GPY_CONFIG_WATCH_POLL_MS` | Same shape as the theme variable, for `config.toml`: unset/`0` = disabled (default); positive integer = poll interval in ms, floored at the 1-second config debounce. This is the fix if editing `config.toml` never applies until you manually `gpy restart`. | `gpy-agent/src/config/manager.rs` (`start_poll_fallback`, `parse_poll_interval_ms`) |
| `GPY_RECONCILE_INTERVAL_SECS` | Overrides the periodic git-status reconcile cadence (default 45 seconds — a backstop scan of all watched repos that catches changes the watcher missed, gated on `agent.live_updates`). Set to a positive integer to tighten it; `0`/unset/non-numeric falls back to 45s. Lowering this trades CPU for freshness on environments with unreliable watcher events. | `gpy-agent/src/agent/mod.rs` (`reconcile_interval_secs`) |
| `GPY_DEBOUNCE_MS` | Debounce window (default 100ms) for the git/language repo watcher: coalesces bursts of filesystem events (e.g. `git commit` touching many files) into a single reconcile. Rarely needs changing; lower it only if you need faster reaction to rapid successive changes and can tolerate more work per event. | `gpy-agent/src/watcher/mod.rs` (`WatcherConfig::from_env`, `parse_env`) |
| `GPY_SIGUSR1_THROTTLE_MS` | Minimum interval (default 150ms) between repaint notifications (`SIGURG`) sent to a shell; the name predates the switch to `SIGURG`. Reload and re-register notifications are never throttled. Raise it if a very active repo (e.g. a build script touching many files) causes visible prompt flicker from rapid-fire repaints; lower it if you need faster reaction and can tolerate more signal traffic. | `gpy-agent/src/watcher/mod.rs` (`WatcherConfig::from_env`, `parse_env`) |
| `GPY_WATCH_WORKTREE` | Default: worktree watching is **on**. Set to `0`/`false`/`no`/`off` to stop watching the working-tree files themselves (only `.git/` metadata is watched) — any other value forces it on. An explicit env var always wins over the `git.watch_worktree` config key at agent init; newly registered repos pick up the change, existing watches keep their setting until re-registration (e.g. `cd` or agent restart). Turn this off on very large or slow-to-stat working trees where per-file watching is too expensive. **Tradeoff:** with it off, pure working-tree changes (new untracked file, editing/removing a tracked file) touch nothing under `.git`, so they fire no event — dirty/untracked state then surfaces only on the next `.git` write (commit/stage/checkout) or the periodic reconcile scan (default 45s), not instantly. If that lag matters, lower `GPY_RECONCILE_INTERVAL_SECS` to bound it more tightly (at the cost of more frequent full rescans). | `gpy-agent/src/watcher/multi_repo.rs` (`set_watch_worktree`, `env_watch_worktree_override`) |

**Recommended recovery sequence** when live updates seem stuck:

1. `gpy status` — confirm the agent is actually running.
2. `GPY_DISABLE_WATCHER=1 gpy restart` briefly, to confirm the watcher (vs. something else) is implicated — if the prompt still doesn't update per-command after a `cd` or new shell, the problem isn't the watcher.
3. Re-enable the watcher and try the poll fallbacks: `GPY_THEME_WATCH_POLL_MS=2000`, `GPY_CONFIG_WATCH_POLL_MS=2000`, and/or a tighter `GPY_RECONCILE_INTERVAL_SECS=10`, then `gpy restart`.
4. If polling fixes it, your filesystem/OS notification channel is unreliable in this environment (common on network mounts, some container/VM setups, and — per issue #354 — even some plain local macOS sessions). Keep the poll var(s) set permanently in your shell config.

---

## Fish-Specific Issues

- **Signal handling**: Fish uses one native `--on-signal SIGURG` handler, which checks the reload/re-register flag files and then repaints; it is the most reliable of the three shells — see the comparison table above. If Fish's prompt isn't repainting on signal, the watcher section above is the more likely cause than Fish's signal handling itself.
- **Agent won't start / crash loop**: Fish runs a background supervisor loop (`__gpy_agent_supervisor_loop` in `fish/core/ipc.fish`) that health-checks the agent and restarts it. Relevant `config.toml` keys, in increasing order of aggressiveness:
  - `[agent.supervisor] enabled = false` — disable the restart loop entirely (useful while debugging a crash so it doesn't keep respawning under you).
  - `[agent.supervisor] check_interval_seconds` (default 30) — health-check cadence.
  - `[agent.supervisor] max_restart_attempts` (default 5) — restarts attempted before the supervisor gives up.
  - `[agent] enabled = false` — disable the agent entirely (falls back to oneshot mode; slower per-prompt but no background process). No supervisor is started either.

  The theme export (`gpy-agent theme export --format fish`) re-exports these as `GPY_AGENT_ENABLED`, `GPY_AGENT_SUPERVISOR_ENABLED`, `GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS` and `GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS` at shell start and on every config reload, so `config.toml` wins over a `set -gx` in your shell config (#657). If GPY seems completely disabled, check both `GPY_AGENT_ENABLED` and `GPY_AGENT_SUPERVISOR_ENABLED` in the environment fish starts with — `gpy_init.fish` skips loading entirely when *both* are `0` there.
- **Reproducing registration issues**: `tests/fish/e2e_interactive_session.test.fish` drives a real `fish -i` on a pseudo-terminal against a real agent (two shells, one agent, repaint with no keystroke); run it, or borrow its sandbox setup, to reproduce a registration problem outside your own session. Prompt content itself is asserted by the gating `tests/fish/e2e_prompt_content.test.fish`.
- **Abbreviations/completions**: shell completions are installed and regenerated automatically by the standard installers ([CLI Reference § Shell Completion](cli-reference.md#shell-completion)). If `gpy <TAB>` doesn't complete, confirm the `gpy` CLI binary itself is on `PATH` — completions require it.

---

## Zsh-Specific Issues

Zsh uses a native `TRAPURG` function and `precmd`/`preexec` hooks rather than Fish's `--on-signal`/`--on-event`, but the underlying contract (agent-rendered `directory`/`duration`/`git`/`language` segments, shell-rendered `clock`/`status`) is identical across Fish, Zsh, and Bash — see `docs/archive/fish-integration-contract.md` for the full shared contract if you're debugging segment rendering specifically.

Zsh is at parity with Fish for live updates: `TRAPURG` re-renders `PROMPT` and calls `zle reset-prompt`, so an idle prompt shows a git change with no keystroke (asserted on a real pty by `tests/zsh/e2e_git_live_content.test.zsh`), a restarted agent's re-register nudge re-registers the shell (`tests/zsh/e2e_reregister_after_restart.test.zsh`), and a dead agent is restarted from the next prompt by the periodic supervisor check. If you hit a Zsh-only issue, it's worth filing as a gap in this doc.

---

## Bash-Specific Issues

Bash support is functionally complete but has real, well-documented caveats: duration-segment precision varies by Bash version (disabled entirely on Bash 3.x, macOS's default), live updates are less reliable than Fish/Zsh due to Bash's `trap` mechanism, transient prompt isn't implemented, and overall performance is 20-30% slower than Zsh. macOS ships Bash 3.2 by default — most Bash-related reports trace back to that. See [Bash Limitations](bash-limitations.md) for the full version matrix, benchmarks, migration paths, and known-issues list; that document is authoritative and this guide won't restate it.

---

## IPC/Socket Issues

The agent communicates over a Unix domain socket, resolved in priority order (`$XDG_RUNTIME_DIR` → `$XDG_CACHE_HOME` → `~/.cache/gpy/gpy.sock`) — see [Advanced Configuration § Socket Location](advanced-configuration.md#socket-location) for the full priority list and how to find the active path with `gpy status`.

- **`GPY_AGENT_SOCKET_PATH`**: overrides socket resolution everywhere (client, server, CLI commands) — highest priority, useful when running a non-standard install or multiple agent instances side by side. Verified at `gpy-agent/src/ipc/client.rs`, `gpy-agent/src/ipc/server/handle.rs`, `gpy-agent/src/agent/lifecycle/mod.rs`, and `gpy-agent/src/commands/utils.rs`.
- **Manual ping**: use `socat` against the resolved socket to confirm the agent responds at the transport level, independent of the shell integration — example command in [Advanced Configuration § Test IPC Connection](advanced-configuration.md#test-ipc-connection).

---

## Install/PATH Issues

Binary-not-found errors, agent-won't-start, macOS Gatekeeper quarantine (`xattr -d com.apple.quarantine`), and the "Bash on macOS is too old" case are all covered in depth in the [Installation Guide § Troubleshooting](../INSTALL.md#troubleshooting). Start there for anything related to getting GPY installed and on `PATH` in the first place, before assuming it's a runtime/watcher issue.

`GPY_CONFIG_PATH` overrides the config file location if you need to point GPY at a non-standard config for debugging (verified at `gpy-agent/src/config/schema.rs`, `gpy-agent/src/config/mod.rs`, `gpy-agent/src/commands/utils.rs`); it's checked before the XDG/`HOME` fallback chain.

**Every new shell prints `No such file or directory` for a path ending at a space** (for example `/Users/alice`): installs made before #746 wrote the rc `source` line without quotes, so a `HOME` or `XDG_CONFIG_HOME` containing a space split the path. Re-running the installer does not rewrite an existing `# >>> gpy-init >>>` block, so edit that block by hand and wrap the path in double quotes (`source "/Users/alice smith/.config/gpy/zsh/gpy.zsh"`), or remove the block and re-run the installer. Current installers quote the path, and refuse (before writing anything) a config directory containing `"`, `$`, a backtick, a backslash or a newline.

---

## See Also

- [Bash Limitations](bash-limitations.md)
- [Advanced Configuration](advanced-configuration.md)
- [CLI Reference](cli-reference.md)
- [Installation Guide](../INSTALL.md)

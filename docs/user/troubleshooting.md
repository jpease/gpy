<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Troubleshooting Guide

Shell-aware troubleshooting for GPY: quick diagnosis and the watcher/environment-variable behavior that governs live prompt updates.

This guide focuses on what isn't documented elsewhere. For exhaustive detail it links out rather than repeating:

- Install/PATH/binary problems → [Installation Guide](../INSTALL.md#troubleshooting)
- What each shell supports → [Shell feature matrix](../INSTALL.md#shell-feature-matrix)
- Bash version caveats → [Bash Limitations](bash-limitations.md)
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

`gpy doctor` and `gpy status` syntax/output are fully documented in the [CLI Reference](cli-reference.md#gpy-doctor). Debug logging, the `gpy status` / `gpy debug paths` connection check, and config reset are covered in [Advanced Configuration → Troubleshooting](advanced-configuration.md#troubleshooting).

---

## Shell Comparison

Before filing a bug, check whether the symptom is expected for your shell in the [Shell feature matrix](../INSTALL.md#shell-feature-matrix). In particular, Bash shows live updates at the next prompt rather than repainting an idle one.

---

## "My Prompt Isn't Updating Live" (Watcher Issues)

This is the most common complaint, and the mechanism is not documented elsewhere, so it gets the most detail here.

### How live updates work

The agent watches your repo, `config.toml`, the active theme's files (an overlay and every theme it extends), and your user themes directory (where a new same-name theme overrides a plugin or built-in one) for changes using OS-level filesystem notifications (`FSEvents` on macOS, `inotify` on Linux, via the `notify` crate). When something relevant changes, the agent sends `SIGURG` to registered shell processes. A plain `SIGURG` means "repaint the prompt" (git/language state changed); to ask for a theme/config reload or a re-registration the agent first writes an empty `<pid>.reload` or `<pid>.reregister` flag file in the runtime `shells/` directory. `SIGURG` is ignored by default, so a shell that has not installed its handler yet (for example right after `exec fish`) is never killed by a notification. See [ADR-0007](../dev/adr/adr-0007-sigurg-doorbell-notifications.md) for the current protocol and [ADR-0004](../dev/adr/adr-0004-sigusr1-live-updates.md) / [ADR-0005](../dev/adr/adr-0005-theme-hot-reload.md) for the original rationale.

### Why it can silently stop working

OS-level filesystem notifications are not 100% reliable in every environment. Confirmed failure modes:

- **Network-mounted or some Docker/VM-mounted filesystems** don't reliably propagate `inotify`/`FSEvents` events.
- **FSEvents outages on macOS**: in at least one confirmed case, `FSEventsd` delivered zero notifications to the watcher in a sandboxed session even though the daemon was running. The poll-based fallbacks below route around this.

When this happens, git status, language versions, and theme edits stop refreshing automatically. The prompt still updates on every new command (Fish/Zsh/Bash all re-invoke prompt rendering per-command regardless of the watcher), but *within* a long-idle session at a single prompt, nothing changes until you press Enter.

### Diagnostic and fallback environment variables

Every diagnostic and fallback variable, with its default, is on [Environment Variables](environment-variables.md#agent-tuning-and-diagnostics); `gpy doctor` lists their effective values.
The recovery sequence below uses `GPY_DISABLE_WATCHER`, `GPY_THEME_WATCH_POLL_MS`, `GPY_CONFIG_WATCH_POLL_MS` and `GPY_RECONCILE_INTERVAL_SECS`.

**Recommended recovery sequence** when live updates seem stuck:

1. `gpy status` — confirm the agent is actually running.
2. `GPY_DISABLE_WATCHER=1 gpy restart` briefly, to confirm the watcher (vs. something else) is implicated — if the prompt still doesn't update per-command after a `cd` or new shell, the problem isn't the watcher.
3. Re-enable the watcher and try the poll fallbacks: `GPY_THEME_WATCH_POLL_MS=2000`, `GPY_CONFIG_WATCH_POLL_MS=2000`, and/or a tighter `GPY_RECONCILE_INTERVAL_SECS=10`, then `gpy restart`.
4. If polling fixes it, your filesystem/OS notification channel is unreliable in this environment (common on network mounts, some container/VM setups, and even some plain local macOS sessions). Keep the poll var(s) set permanently in your shell config.

### WSL

- **Polling on a Windows drive**: `GPY_WATCH_FORCE_POLL=1 gpy restart` starts the watcher on the polling backend (a 2-second stat walk, `GPY_WATCH_POLL_MS` to change it) without waiting for the 2-second health probe to notice a silent backend. Expect the periodic reconcile (45 s) and the repaint on each command to be what updates the prompt otherwise.

Install-time and filesystem notes: [Windows (WSL)](../INSTALL.md#windows-wsl).

---

## Fish-Specific Issues

- **Signal handling**: Fish uses one native `--on-signal SIGURG` handler, which checks the reload/re-register flag files and then repaints. If Fish's prompt isn't repainting on signal, the watcher section above is the more likely cause than Fish's signal handling itself.
- **Agent won't start / crash loop**: Bash, Zsh and Fish restart the agent the same way, with no background supervisor process. Each shell probes the agent on the prompt, at most once per `check_interval_seconds`; when the probe fails it runs `gpy-agent start` in the background. A crashed agent is restarted on the next checked prompt, not while the shell sits idle. A silent agent (connected, no answer) costs up to `GPY_IPC_TIMEOUT_MS` (150 ms by default) on the prompt that runs the check, and counts as down. `gpy-agent start` replaces an older-protocol agent and evicts one that is wedged and honours `shutdown`; a wedged agent that ignores `gpy-agent stop` is never killed by a shell, so stop it by hand (`gpy-agent stop`, then `kill <pid>`). A supervisor loop left running by an earlier Fish version is stopped at the first Fish prompt. Relevant `config.toml` keys, in increasing order of aggressiveness:
  - `[agent.supervisor] enabled = false` — disable only the restarts (useful while debugging a crash so it doesn't keep respawning under you). Each shell still starts the agent once at startup and registers with a running agent, so live updates keep working.
  - `[agent.supervisor] check_interval_seconds` — health-check cadence.
  - `[agent.supervisor] max_restart_attempts` — restarts a shell attempts before it gives up (a good probe resets the count).
  - `[agent] enabled = false` — disable the agent entirely (falls back to oneshot mode; slower per-prompt but no background process). No restart is attempted either.

  An exported `GPY_AGENT_ENABLED`, `GPY_AGENT_SUPERVISOR_ENABLED`, `GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS` or `GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS` wins over `config.toml`; unset, `config.toml` decides (the theme export never exports these names). If GPY seems completely disabled, check both `GPY_AGENT_ENABLED` and `GPY_AGENT_SUPERVISOR_ENABLED` in the environment fish starts with — `gpy_init.fish` skips loading entirely when *both* are off (`0`, `false`, `no` or `off`) there.
- **Abbreviations/completions**: shell completions are installed and regenerated automatically by the standard installers ([CLI Reference § Shell Completion](cli-reference.md#shell-completion)). If `gpy <TAB>` doesn't complete, confirm the `gpy` CLI binary itself is on `PATH` — completions require it.
- **`gpy: GPY needs Fish 3.6 or newer (this is X)` / `gpy[init]: GPY failed to load; using the fallback prompt`**: Fish below 3.6 cannot run GPY (it needs `string escape --json`), so the load stops with the first message and you get a plain fallback prompt until Fish is upgraded. The second message means a file under `~/.local/share/gpy/fish/core/` (`$XDG_DATA_HOME/gpy/fish/core/`) failed to load; Fish's own error (with the file and line) is printed just above it. Re-run `gpy setup` to restore the files, then open a new shell. In a working shell, `gpy-prompt-debug validate` checks that every file GPY needs is installed.

---

## Zsh-Specific Issues

Zsh uses a native `TRAPURG` function and `precmd`/`preexec` hooks rather than Fish's `--on-signal`/`--on-event`, but the underlying contract is identical across Fish, Zsh, and Bash: `directory`/`duration`/`git`/`language` are agent-rendered, and `clock`/`status` are agent-rendered when the theme sets a `format` and drawn locally by the shell otherwise or whenever the agent gives no reply. `hostname` and `username` are bundled script plugins (installed by `gpy setup` into `~/.local/share/gpy/plugins`) run by the shell, so if they are missing run `gpy setup` again.

Zsh is at parity with Fish for live updates: `TRAPURG` re-renders `PROMPT` and calls `zle reset-prompt`, so an idle prompt shows a git change with no keystroke and a restarted agent's re-register nudge re-registers the shell. A dead agent is restarted from the next prompt by the periodic supervisor check. If you hit a Zsh-only issue, please [report it](https://github.com/jpease/gpy/issues).

---

## Bash-Specific Issues

Bash differs in two ways: duration-segment precision varies by Bash version (disabled entirely on Bash 3.x, macOS's default), and live updates are shown at the next prompt rather than repainting an idle one. macOS ships Bash 3.2 by default, and most Bash-related reports trace back to that. See [Bash Limitations](bash-limitations.md) for the version details, migration paths, and known-issues list.

---

## Theme Fails to Load After an Upgrade

A file at `<config>/themes/<name>.toml` fully shadows the built-in theme of the same name, so an old copy keeps its old keys after GPY changes the theme format (for example `format` or `show_always` on `[segments.username]` / `[segments.hostname]`, removed in 0.7). When such a file fails to load, the error says it overrides the builtin theme `<name>` and lists the recovery commands. Fix it with:

```bash
gpy theme diff starship     # what differs from the current builtin
gpy theme migrate starship  # rewrite the removed keys (backs up the theme and config.toml first)
gpy theme reset starship    # or move the copy to starship.toml.bak and use the builtin
```

See [`gpy theme migrate`](cli-reference.md#gpy-theme-migrate-name) for the rewrite rules.

`gpy doctor` lists every such override under **Theme overrides**, overlays (files with `extends`) apart from full copies, and loads each one. For a full copy it suggests `gpy theme rebase <name>`; an overlay's load error is the chain error, with no fork hint. A broken override that is not the active theme is a warning with the same recovery commands, and does not make `gpy doctor` exit non-zero; a broken active theme fails as usual.

---

## IPC/Socket Issues

The agent communicates over a Unix domain socket, resolved in priority order (`$XDG_RUNTIME_DIR` → `$XDG_CACHE_HOME` → `~/.cache/gpy/gpy.sock`) — see [Advanced Configuration § Socket Location](advanced-configuration.md#socket-location) for the full priority list and how to find the active path with `gpy status`.

- **`GPY_AGENT_SOCKET_PATH`**: overrides socket resolution everywhere (client, server, CLI commands); see [Environment Variables](environment-variables.md#gpy_agent_socket_path).
- **`refusing socket directory`**: `gpy-agent start` only publishes its socket in a real directory you own. A symlinked directory, or one owned by another user (such as `/tmp` itself), is refused before anything is written there. A directory you own that is group- or world-writable is set to `0700` when it is GPY's default runtime directory, but refused and left unchanged when `GPY_AGENT_SOCKET_PATH` or `--socket` points into it: run `chmod 700` on it, or point `XDG_RUNTIME_DIR` or `GPY_AGENT_SOCKET_PATH` at a private directory.
- **Socket path too long**: a Unix socket path holds at most 103 bytes on macOS and 107 on Linux, so a long `$HOME`, `XDG_RUNTIME_DIR` or `GPY_AGENT_SOCKET_PATH` fails on macOS first. `gpy-agent start`, `stop` and `status` then exit with an `Error:` line naming the path's length and the limit, followed by a `Remediation:` line. Shorten `XDG_RUNTIME_DIR`, or set `GPY_AGENT_SOCKET_PATH` to a short path in a private directory, such as `/tmp/gpy-$USER/gpy.sock` after `mkdir -m 700 /tmp/gpy-$USER` (every shell and command must see the same value). `gpy debug paths` still prints the rejected path.
- **Segments missing on some renders**: when the agent is reached but answers later than `GPY_IPC_TIMEOUT_MS` (default 150 ms), or `GPY_IPC_COLD_TIMEOUT_MS` (default 500 ms) when the shell has nothing cached for the segment, the shell omits that segment for the render instead of forking a second, blocking `gpy-agent oneshot`. It appears on a later render once the agent's cache is warm. On a heavily loaded machine, raise `GPY_IPC_COLD_TIMEOUT_MS`. Only an agent that cannot be reached at all falls back to oneshot.
- **Connection check**: `gpy status` connects to the agent and reports whether it answered, and `gpy debug paths` prints the socket path every shell uses; see [Advanced Configuration § Test IPC Connection](advanced-configuration.md#test-ipc-connection).

---

## Install/PATH Issues

Binary-not-found errors, agent-won't-start, macOS Gatekeeper quarantine (`xattr -d com.apple.quarantine`), and the "Bash on macOS is too old" case are all covered in the [Installation Guide § Troubleshooting](../INSTALL.md#troubleshooting). Start there for anything related to getting GPY installed and on `PATH`, before assuming it's a runtime/watcher issue.

To point GPY at a non-standard config for debugging, set `GPY_CONFIG_PATH` ([Environment Variables](environment-variables.md#locations)).

**Every new shell prints `No such file or directory` for a path ending at a space** (for example `/Users/alice`): the `source` line in the `# >>> gpy-init >>>` block holds an unquoted path, so a `HOME` or `XDG_DATA_HOME` containing a space splits it. Re-running the installer does not rewrite an existing block, so edit it by hand and wrap the path in double quotes (`source "/Users/alice smith/.local/share/gpy/zsh/gpy.zsh"`), or remove the block and re-run the installer. The installers quote the path, and refuse (before writing anything) a data directory containing `"`, `$`, a backtick, a backslash or a newline.

---

## See Also

- [Bash Limitations](bash-limitations.md)
- [Advanced Configuration](advanced-configuration.md)
- [CLI Reference](cli-reference.md)
- [Installation Guide](../INSTALL.md)

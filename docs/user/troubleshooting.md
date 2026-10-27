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

`gpy doctor` and `gpy status` syntax/output are fully documented in the [CLI Reference](cli-reference.md#gpy-doctor). Debug logging, manual IPC pings via `socat`, and config reset are covered in [Advanced Configuration → Troubleshooting](advanced-configuration.md#troubleshooting).

---

## Shell Comparison

Before filing a bug, check whether the symptom is expected for your shell in the [Shell feature matrix](../INSTALL.md#shell-feature-matrix). In particular, Bash shows live updates at the next prompt rather than repainting an idle one.

---

## "My Prompt Isn't Updating Live" (Watcher Issues)

This is the most common complaint, and the mechanism is not documented elsewhere, so it gets the most detail here.

### How live updates work

The agent watches your repo, `config.toml`, the active theme's file, and your user themes directory (where a new same-name theme overrides a plugin or built-in one) for changes using OS-level filesystem notifications (`FSEvents` on macOS, `inotify` on Linux, via the `notify` crate). When something relevant changes, the agent sends `SIGURG` to registered shell processes. A plain `SIGURG` means "repaint the prompt" (git/language state changed); to ask for a theme/config reload or a re-registration the agent first writes an empty `<pid>.reload` or `<pid>.reregister` flag file in the runtime `shells/` directory. `SIGURG` is ignored by default, so a shell that has not installed its handler yet (for example right after `exec fish`) is never killed by a notification. See [ADR-0007](../dev/adr/adr-0007-sigurg-doorbell-notifications.md) for the current protocol and [ADR-0004](../dev/adr/adr-0004-sigusr1-live-updates.md) / [ADR-0005](../dev/adr/adr-0005-theme-hot-reload.md) for the original rationale.

### Why it can silently stop working

OS-level filesystem notifications are not 100% reliable in every environment. Confirmed failure modes:

- **Network-mounted or some Docker/VM-mounted filesystems** don't reliably propagate `inotify`/`FSEvents` events.
- **FSEvents outages on macOS**: in at least one confirmed case, `FSEventsd` delivered zero notifications to the watcher in a sandboxed session even though the daemon was running. The poll-based fallbacks below route around this.

When this happens, git status, language versions, and theme edits stop refreshing automatically. The prompt still updates on every new command (Fish/Zsh/Bash all re-invoke prompt rendering per-command regardless of the watcher), but *within* a long-idle session at a single prompt, nothing changes until you press Enter.

### Diagnostic and fallback environment variables

All of these are off or default unless set.

| Variable | Effect |
|---|---|
| `GPY_DISABLE_WATCHER` | Set to `1`/`true`/`yes`/`on` to skip creating the git/language repo file watcher entirely at agent startup. This *only* disables the git/language live-update watcher — it does **not** affect the separate theme or config hot-reload watchers below. Use this to rule the watcher in or out as the cause of a hang or high CPU. |
| `GPY_THEME_WATCH_POLL_MS` | Unset or `0` (default): disabled — theme reload relies solely on `FSEvents`/`inotify`. Set to a positive integer: spawns an additional poll thread that, on that interval (floored at the watcher's debounce), checks the file the active theme name currently resolves to (user, then plugin, then built-in) and reloads when its content changes or the file appears or disappears, independent of OS notifications. |
| `GPY_CONFIG_WATCH_POLL_MS` | Same shape as the theme variable, for `config.toml`: unset/`0` = disabled (default); positive integer = poll interval in ms, floored at the 1-second config debounce. This is the fix if editing `config.toml` never applies until you manually `gpy restart`. |
| `GPY_RECONCILE_INTERVAL_SECS` | Overrides the periodic git-status reconcile cadence (default 45 seconds — a backstop scan of all watched repos that catches changes the watcher missed, gated on `agent.live_updates`). Set to a positive integer to tighten it; `0`/unset/non-numeric falls back to 45s. Lowering this trades CPU for freshness on environments with unreliable watcher events. |
| `GPY_WATCH_FORCE_POLL` | `1`/`true`/`yes`/`on` starts the file watchers on the polling backend with no health probe, for a machine already known to deliver no filesystem events (a repository on a WSL `/mnt/<drive>`, a network mount). Default: start native and swap to polling only if the probe sees nothing. |
| `GPY_WATCH_PROBE_MS` | How long (default 2000 ms) the native watcher gets to deliver the probe's own event before it is replaced by polling. `0` disables the probe and keeps the native backend whatever it does. |
| `GPY_WATCH_POLL_MS` | Interval (default 2000 ms) of the polling backend, once armed or forced. Each tick stat-walks every watched root. |
| `GPY_DEBOUNCE_MS` | Debounce window (default 100ms) for the git/language repo watcher: coalesces bursts of filesystem events (e.g. `git commit` touching many files) into a single reconcile. Rarely needs changing; lower it only if you need faster reaction to rapid successive changes and can tolerate more work per event. |
| `GPY_SIGURG_THROTTLE_MS` | Minimum interval (default 150ms) between repaint notifications (`SIGURG`) sent to a shell. Reload and re-register notifications are never throttled. Raise it if a very active repo (e.g. a build script touching many files) causes visible prompt flicker from rapid-fire repaints; lower it if you need faster reaction and can tolerate more signal traffic. |
| `GPY_WATCH_WORKTREE` | Default: worktree watching is **on**. Set to `0`/`false`/`no`/`off` to stop watching the working-tree files themselves (only `.git/` metadata is watched) — any other value forces it on. An explicit env var always wins over the `git.watch_worktree` config key at agent init; newly registered repos pick up the change, existing watches keep their setting until re-registration (e.g. `cd` or agent restart). Turn this off on very large or slow-to-stat working trees where per-file watching is too expensive. **Tradeoff:** with it off, pure working-tree changes (new untracked file, editing/removing a tracked file) touch nothing under `.git`, so they fire no event — dirty/untracked state then surfaces only on the next `.git` write (commit/stage/checkout) or the periodic reconcile scan (default 45s), not instantly. If that lag matters, lower `GPY_RECONCILE_INTERVAL_SECS` to bound it more tightly (at the cost of more frequent full rescans). |

**Recommended recovery sequence** when live updates seem stuck:

1. `gpy status` — confirm the agent is actually running.
2. `GPY_DISABLE_WATCHER=1 gpy restart` briefly, to confirm the watcher (vs. something else) is implicated — if the prompt still doesn't update per-command after a `cd` or new shell, the problem isn't the watcher.
3. Re-enable the watcher and try the poll fallbacks: `GPY_THEME_WATCH_POLL_MS=2000`, `GPY_CONFIG_WATCH_POLL_MS=2000`, and/or a tighter `GPY_RECONCILE_INTERVAL_SECS=10`, then `gpy restart`.
4. If polling fixes it, your filesystem/OS notification channel is unreliable in this environment (common on network mounts, some container/VM setups, and even some plain local macOS sessions). Keep the poll var(s) set permanently in your shell config.

### WSL

Under Windows Subsystem for Linux, a repository on a Windows drive (`/mnt/c`, `/mnt/d`, ...) is on drvfs/9p, not on the Linux filesystem, and `inotify` may not report changes made to it. Symptoms are the ones above: git status stays stale at an idle prompt, and `git` is slower on every command.

- **Move the repository** into the Linux filesystem (`~/src/...`). Nothing else is as reliable.
- **Keep it on the Windows drive and poll**: `GPY_WATCH_FORCE_POLL=1 gpy restart` starts the watcher on the polling backend (a 2-second stat walk, `GPY_WATCH_POLL_MS` to change it) without waiting for the 2-second health probe to notice a silent backend. Expect the periodic reconcile (45 s) and the repaint on each command to be what updates the prompt otherwise.
- **Skip it**: `gpy config set` cannot set list values; edit `config.toml` and add `skip_paths = ["/mnt/c"]` under `[git]` to turn git detection off for everything on that drive.
- **Windows tools in the language segment**: WSL appends the Windows `PATH` to yours (interop). If the language segment reports a Windows-side tool or is slow, put `[interop]` / `appendWindowsPath = false` in `/etc/wsl.conf` and run `wsl --shutdown`. The agent runs `node`, `python3`, `mise` and the other version commands by bare name, so only an extensionless Windows file of that name can be picked up; this has not been traced on a real machine.
- **Icons show as boxes**: GPY cannot see the Windows terminal's fonts, so `gpy setup` reports them as undetectable and writes ASCII icons. Install a Nerd Font on Windows, select it in the terminal, then `gpy config set ui.show_icons true`.
- **`gpy config open` does nothing**: `xdg-open` is usually missing on WSL. Set `$EDITOR`, or install `wslu` (it provides `wslview`, which GPY prefers on WSL).
- **Keep `$HOME`, the cache and the runtime directory on the Linux filesystem.** The agent serialises its start with a file lock and keeps its socket there; neither is tested on a drvfs or network `$HOME`.

`scripts/wsl-smoke.sh` checks the font behaviour and `gpy-agent oneshot git` against a repository on each filesystem; run it inside your distribution (`cd gpy-agent && cargo build --locked`, then `scripts/wsl-smoke.sh`) to record a WSL run of your own. CI runs it, plus the cross-shell contract suite, in `.github/workflows/wsl-smoke.yml`.

---

## Fish-Specific Issues

- **Signal handling**: Fish uses one native `--on-signal SIGURG` handler, which checks the reload/re-register flag files and then repaints. If Fish's prompt isn't repainting on signal, the watcher section above is the more likely cause than Fish's signal handling itself.
- **Agent won't start / crash loop**: Fish runs a background supervisor loop (`__gpy_agent_supervisor_loop` in `fish/core/supervisor.fish`) that health-checks the agent and restarts it. An agent that is gone (no socket, nothing listening) is restarted at the next check; one that accepts the check but does not answer within a second is restarted only after two checks in a row, so a brief stall does not cost you a restart. Relevant `config.toml` keys, in increasing order of aggressiveness:
  - `[agent.supervisor] enabled = false` — disable only the background restart loop (useful while debugging a crash so it doesn't keep respawning under you). Each shell still tries a bounded per-prompt autostart and registers with a running agent, so live updates keep working.
  - `[agent.supervisor] check_interval_seconds` (default 30) — health-check cadence.
  - `[agent.supervisor] max_restart_attempts` (default 5) — restarts attempted before the supervisor gives up.
  - `[agent] enabled = false` — disable the agent entirely (falls back to oneshot mode; slower per-prompt but no background process). No supervisor is started either.

  The theme export (`gpy-agent theme export --format fish`) re-exports these as `GPY_AGENT_ENABLED`, `GPY_AGENT_SUPERVISOR_ENABLED`, `GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS` and `GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS` at shell start and on every config reload, so `config.toml` wins over a `set -gx` in your shell config. If GPY seems completely disabled, check both `GPY_AGENT_ENABLED` and `GPY_AGENT_SUPERVISOR_ENABLED` in the environment fish starts with — `gpy_init.fish` skips loading entirely when *both* are `0` or `false` there.
- **Abbreviations/completions**: shell completions are installed and regenerated automatically by the standard installers ([CLI Reference § Shell Completion](cli-reference.md#shell-completion)). If `gpy <TAB>` doesn't complete, confirm the `gpy` CLI binary itself is on `PATH` — completions require it.

---

## Zsh-Specific Issues

Zsh uses a native `TRAPURG` function and `precmd`/`preexec` hooks rather than Fish's `--on-signal`/`--on-event`, but the underlying contract is identical across Fish, Zsh, and Bash: `directory`/`duration`/`git`/`language` are agent-rendered, and `clock`/`status` are agent-rendered when the theme sets a `format` and drawn locally by the shell otherwise or whenever the agent gives no reply. `hostname` and `username` are bundled script plugins (installed by `gpy setup` into `~/.config/gpy/bundled-plugins`) run by the shell, so if they are missing run `gpy setup` again.

Zsh is at parity with Fish for live updates: `TRAPURG` re-renders `PROMPT` and calls `zle reset-prompt`, so an idle prompt shows a git change with no keystroke and a restarted agent's re-register nudge re-registers the shell. A dead agent is restarted from the next prompt by the periodic supervisor check. If you hit a Zsh-only issue, please [report it](https://github.com/jpease/gpy/issues).

---

## Bash-Specific Issues

Bash differs in two ways: duration-segment precision varies by Bash version (disabled entirely on Bash 3.x, macOS's default), and live updates are shown at the next prompt rather than repainting an idle one. macOS ships Bash 3.2 by default, and most Bash-related reports trace back to that. See [Bash Limitations](bash-limitations.md) for the version details, migration paths, and known-issues list.

---

## Theme Fails to Load After an Upgrade

A file at `<config>/themes/<name>.toml` fully shadows the built-in theme of the same name, so an old copy keeps its old keys after GPY changes the theme format. When such a file fails to load, the error says it overrides the builtin theme `<name>` and lists the recovery commands (`gpy theme diff|migrate|reset <name>`). Moving the file aside lets the built-in take over.

`gpy doctor` lists every such override under **Theme overrides** and loads each one. A broken override that is not the active theme is a warning with the same recovery commands, and does not make `gpy doctor` exit non-zero; a broken active theme fails as usual.

## IPC/Socket Issues

The agent communicates over a Unix domain socket, resolved in priority order (`$XDG_RUNTIME_DIR` → `$XDG_CACHE_HOME` → `~/.cache/gpy/gpy.sock`) — see [Advanced Configuration § Socket Location](advanced-configuration.md#socket-location) for the full priority list and how to find the active path with `gpy status`.

- **`GPY_AGENT_SOCKET_PATH`**: overrides socket resolution everywhere (client, server, CLI commands) — highest priority, useful when running a non-standard install or multiple agent instances side by side.
- **`refusing socket directory`**: `gpy-agent start` only publishes its socket in a real directory you own. A symlinked directory, or one owned by another user (such as `/tmp` itself), is refused before anything is written there. A directory you own that is group- or world-writable is set to `0700` when it is GPY's default runtime directory, but refused and left unchanged when `GPY_AGENT_SOCKET_PATH` or `--socket` points into it: run `chmod 700` on it, or point `XDG_RUNTIME_DIR` or `GPY_AGENT_SOCKET_PATH` at a private directory.
- **Socket path too long**: a Unix socket path holds at most 103 bytes on macOS and 107 on Linux, so a long `$HOME`, `XDG_RUNTIME_DIR` or `GPY_AGENT_SOCKET_PATH` fails on macOS first. `gpy-agent start`, `stop` and `status` then exit with an error naming the path's length and the limit. Shorten `XDG_RUNTIME_DIR`, or set `GPY_AGENT_SOCKET_PATH` to a short path in a private directory, such as `/tmp/gpy-$USER/gpy.sock` after `mkdir -m 700 /tmp/gpy-$USER` (every shell and command must see the same value). `gpy debug paths` still prints the rejected path.
- **Segments missing on some renders**: when the agent is reached but answers later than `GPY_IPC_TIMEOUT_MS` (default 150 ms), or `GPY_IPC_COLD_TIMEOUT_MS` (default 500 ms) when the shell has nothing cached for the segment, the shell omits that segment for the render instead of forking a second, blocking `gpy-agent oneshot`. It appears on a later render once the agent's cache is warm. On a heavily loaded machine, raise `GPY_IPC_COLD_TIMEOUT_MS`. Only an agent that cannot be reached at all falls back to oneshot.
- **Manual ping**: use `socat` against the resolved socket to confirm the agent responds at the transport level, independent of the shell integration — example command in [Advanced Configuration § Test IPC Connection](advanced-configuration.md#test-ipc-connection).

---

## Install/PATH Issues

Binary-not-found errors, agent-won't-start, macOS Gatekeeper quarantine (`xattr -d com.apple.quarantine`), and the "Bash on macOS is too old" case are all covered in the [Installation Guide § Troubleshooting](../INSTALL.md#troubleshooting). Start there for anything related to getting GPY installed and on `PATH`, before assuming it's a runtime/watcher issue.

`GPY_CONFIG_PATH` overrides the config file location if you need to point GPY at a non-standard config for debugging; it's checked before the XDG/`HOME` fallback chain.

**Every new shell prints `No such file or directory` for a path ending at a space** (for example `/Users/alice`): the `source` line in the `# >>> gpy-init >>>` block holds an unquoted path, so a `HOME` or `XDG_CONFIG_HOME` containing a space splits it. Re-running the installer does not rewrite an existing block, so edit it by hand and wrap the path in double quotes (`source "/Users/alice smith/.config/gpy/zsh/gpy.zsh"`), or remove the block and re-run the installer. The installers quote the path, and refuse (before writing anything) a config directory containing `"`, `$`, a backtick, a backslash or a newline.

---

## See Also

- [Bash Limitations](bash-limitations.md)
- [Advanced Configuration](advanced-configuration.md)
- [CLI Reference](cli-reference.md)
- [Installation Guide](../INSTALL.md)

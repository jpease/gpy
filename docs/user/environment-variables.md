<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Environment Variables

Every environment variable GPY reads, except installer-only `GPY_VERSION` (see [Installation](../INSTALL.md#environment-variables)). Config keys live in the [Configuration Reference](configuration-reference.md).

Every `GPY_*` flag accepts `1`/`true`/`yes`/`on` and `0`/`false`/`no`/`off`, case-insensitive. An unrecognised value is ignored and logged once, a positive-integer variable ignores `0` and garbage, and an empty value counts as unset. Where a variable has a matching config key, the precedence is the environment variable, then `config.toml`, then the built-in default.

`gpy doctor` lists each agent tunable's effective value and where it came from (the "Agent tuning" section).

---

## Locations

| Variable | Default | Effect |
|---|---|---|
| `GPY_CONFIG_PATH` | unset | Explicit path to `config.toml`, checked before the XDG and `$HOME` locations. An empty value is ignored. |
| `XDG_CONFIG_HOME` | `~/.config` | Config root `$XDG_CONFIG_HOME/gpy`, also where user themes, palettes and plugins live. |
| `XDG_CACHE_HOME` | `~/.cache` | Cache root `$XDG_CACHE_HOME/gpy`: theme exports, instant-prompt cache, the agent log. Also the runtime root when `XDG_RUNTIME_DIR` is unset. |
| `XDG_RUNTIME_DIR` | unset | Runtime root `$XDG_RUNTIME_DIR/gpy`: the socket and the shell registry. |
| `HOME` | passwd entry | Fallback for the config and cache roots when the XDG variables are unset. An empty `HOME` counts as unset. |
| `GPY_AGENT_SOCKET_PATH` | under the runtime root | Overrides the socket path alone. Every command and shell must see the same value. See [below](#gpy_agent_socket_path). |
| `GPY_BUNDLED_PLUGIN_DIR` | `$XDG_DATA_HOME/gpy/plugins` | Directory holding the bundled plugins (`username`, `hostname`). See [Plugin Framework](../dev/plugins.md#bundled-plugins). |
| `VISUAL`, `EDITOR` | platform editor | The editor `gpy config open` launches, in that order. |

### GPY_AGENT_SOCKET_PATH

Overrides the socket path alone; the runtime root is unchanged. Every command and every shell must see the same value, so export it before the shell integration loads. Every socket, default or override, also gets a `<socket>.lock` start lock next to it. Its directory must be a real directory you own (see [`gpy start`](cli-reference.md#gpy-start)).

`gpy debug paths` prints what the path variables resolve to on the current machine.

---

## Shell integration

Read by the shell scripts. Export them in `config.fish`, `.zshrc` or `.bashrc` before the GPY integration is sourced. A value that is not a non-negative integer falls back to the default.

| Variable | Default | Shells | Effect |
|---|---|---|---|
| `GPY_IPC_TIMEOUT_MS` | `150` | Fish, Zsh, Bash | How long a prompt render waits for the agent's reply, in milliseconds. A late reply omits that segment for the render. |
| `GPY_IPC_COLD_TIMEOUT_MS` | `500` | Fish, Zsh, Bash | The same wait when the prompt has nothing to show yet for a segment. Never shorter than `GPY_IPC_TIMEOUT_MS`. |
| `GPY_GIT_INSTANT_CACHE_TTL_SECONDS` | `5` | Fish, Zsh, Bash | Seconds the shell treats its git instant-cache entry as fresh. See [Prompt freshness](#prompt-freshness). |
| `GPY_LANGUAGE_CACHE_TTL_SECONDS` | `30` | Fish, Zsh, Bash | The same for the language instant-cache entry. |
| `GPY_SHOW_STATUS` | on | Fish, Zsh, Bash | `0` hides the standalone `✔`/`✖` exit-status indicator before the prompt symbol. Read on every render. See [below](#gpy_show_status). |
| `GPY_AGENT_ENABLED` | on | Fish, Zsh, Bash | Overrides `agent.enabled`. Off stops the shell from starting, registering with or supervising an agent. |
| `GPY_AGENT_SUPERVISOR_ENABLED` | on | Fish, Zsh, Bash | Overrides `agent.supervisor.enabled`. |
| `GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS` | from config | Fish, Zsh, Bash | Overrides `agent.supervisor.check_interval_seconds`; `0` checks on every prompt. Not range-checked. |
| `GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS` | from config | Fish, Zsh, Bash | Overrides `agent.supervisor.max_restart_attempts`; `0` never restarts. Not range-checked. |
| `GPY_DEBUG` | off | Fish, Zsh, Bash | `1` turns on the shell integration's own debug output, including logs of unrecognised variable values. |

### GPY_SHOW_STATUS

The indicator is never shown while the agent renders the prompt symbol (that symbol is already coloured by the exit status), so this matters mainly when the agent is unavailable. Fish, Zsh and Bash read it alike.

### Agent variables in the shells

A value you export for `GPY_AGENT_ENABLED`, `GPY_AGENT_SUPERVISOR_ENABLED`, `GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS` or `GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS` beats the matching key in `[agent]` or `[agent.supervisor]` of `config.toml`. The keys, their defaults and ranges are in the [Configuration Reference](configuration-reference.md#agent).

---

## Agent tuning and diagnostics

Read by the agent. Each is off, or at the default shown, unless set.

| Variable | Default | Effect |
|---|---|---|
| `GPY_PROFILE` | off | A true value logs `[PROFILE] <operation>: <ms>` lines for timed operations to stderr and the debug log. |
| `GPY_DEBUG_LOG` | unset | File path of the agent's trace log. An empty value leaves logging off and warns once. |
| `GPY_DISABLE_WATCHER` | off | A true value skips the git/language file watcher at agent startup. See [below](#gpy_disable_watcher). |
| `GPY_THEME_WATCH_POLL_MS` | off | Positive integer: also poll the active theme's files on this interval. `0` or unset: notifications only. |
| `GPY_CONFIG_WATCH_POLL_MS` | off | The same for `config.toml`, floored at the 1-second config debounce. The fix when editing the file never applies until `gpy restart`. |
| `GPY_RECONCILE_INTERVAL_SECS` | `45` | Seconds between reconcile scans of all watched repositories. See [Prompt freshness](#prompt-freshness). |
| `GPY_WATCH_FORCE_POLL` | off | A true value starts the watchers on the polling backend with no health probe. See [below](#watcher-backends). |
| `GPY_WATCH_PROBE_MS` | `2000` | Milliseconds the native watcher gets to deliver its probe event before polling replaces it. `0` disables the probe. |
| `GPY_WATCH_POLL_MS` | `2000` | Interval of the polling backend, once armed or forced. See [below](#watcher-backends). |
| `GPY_DEBOUNCE_MS` | `100` | Debounce window for the git/language watcher; coalesces bursts of events into one reconcile. |
| `GPY_SIGURG_THROTTLE_MS` | `150` | Minimum gap between repaint signals sent to one shell. Reload and re-register signals are never throttled. |
| `GPY_WATCH_WORKTREE` | on | Overrides `git.watch_worktree`. See [below](#gpy_watch_worktree). |

### GPY_DISABLE_WATCHER

Skips only the git/language repository watcher. It does not affect the theme and config hot-reload watchers. Use it to rule the watcher in or out as the cause of a hang or high CPU. See [Troubleshooting](troubleshooting.md#how-live-updates-work).

### Watcher backends

The agent starts native (`FSEvents` or `inotify`) and swaps to polling only if the probe sees nothing. Use `GPY_WATCH_FORCE_POLL` for a machine known to deliver no events, such as a repository on a WSL `/mnt/<drive>` or a network mount. Each poll tick stat-walks every watched root. Poll mode compares file contents only inside the git directory, and not in its `objects` store, so a working-tree file rewritten within the same second at the same size shows at the next prompt instead of live.

### GPY_WATCH_WORKTREE

An explicit recognised value always wins over `git.watch_worktree`; an unrecognised one is ignored with a warning and the config key decides. Newly registered repositories pick up the change; existing watches keep their setting until re-registration (a `cd` or an agent restart). Turn it off on very large or slow-to-stat working trees. With it off, pure working-tree changes touch nothing under `.git`, so dirty and untracked state surfaces only on the next `.git` write or the reconcile scan; lower `GPY_RECONCILE_INTERVAL_SECS` to bound that lag.

---

## Setup

Read by `gpy setup`.

| Variable | Default | Effect |
|---|---|---|
| `GPY_NERD_FONT` | detect | What to assume about icons instead of asking. See [below](#gpy_nerd_font). |
| `GPY_SHELL` | `$SHELL` | Which shell `gpy setup` wires when `--shell` is absent: `fish`, `zsh` or `bash`. The installers read it too ([Installation](../INSTALL.md#environment-variables)). |
| `GPY_SHARE_DIR` | beside the binary | Where `gpy setup` finds the `fish/`, `zsh/` and `bash/` trees, after `--from`. |

### GPY_NERD_FONT

- `1`, `true`, `yes`, `on` force a Nerd Font; `0`, `false`, `no`, `off` force none.
- `nerd` is a Nerd Font; `none` and `ascii` are none.
- `unknown` says fonts cannot be detected, which forces ASCII icons.
- `auto` detects.

Anything else is ignored with one warning and falls through to detection. A recognised value other than `auto` skips the confirmation prompt. Under WSL detection always reports `unknown` (the terminal's fonts are on the Windows side), so set this to answer for it. `gpy doctor` lists the effective value in its "Agent tuning" section.

---

## Prompt freshness

Four timers decide how stale a segment can be:

- **Shell, 5 s.** The shell shows a git entry the agent confirmed within `GPY_GIT_INSTANT_CACHE_TTL_SECONDS` (5 s) at once, and refreshes it in the background after that. The language entry uses `GPY_LANGUAGE_CACHE_TTL_SECONDS` (30 s).
- **Agent, 30 s and 150 ms.** The agent reuses its git status for 30 s from its last full scan, and never recomputes it within 150 ms of its previous update of any kind. Neither value is tunable.
- **Repaint signals, 150 ms.** `GPY_SIGURG_THROTTLE_MS` limits repaint signals only; it does not change the 150 ms status cooldown.
- **Reconcile, 45 s.** `GPY_RECONCILE_INTERVAL_SECS` sets how often the agent rescans every watched repository to catch events the watcher missed.

---

## Deprecated

The first three are read by the Fish integration only; they still take effect and warn once at the first prompt. The others are ignored, and Fish warns about each one that is set.

| Variable | Use instead |
|---|---|
| `GPY_SHOW_LANGUAGES` | `gpy enable language` / `gpy disable language` |
| `GPY_MINIMAL_SEGMENTS` | `gpy enable <name>` / `gpy disable <name>` |
| `GPY_TEST_SEGMENTS` | `gpy enable <name>` / `gpy disable <name>` |
| `GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS` | Ignored with a warning; use `[agent.supervisor]` in `config.toml`. |
| `GPY_SUPERVISOR_LONG_BACKOFF_SECONDS` | Ignored with a warning; use `[agent.supervisor]` in `config.toml`. |
| `GPY_AGENT_AUTOSTART_MAX_ATTEMPTS` | Ignored with a warning; use `[agent.supervisor]` in `config.toml`. |
| `GPY_AGENT_AUTOSTART_RATE_LIMIT_SECONDS` | Ignored with a warning; use `[agent.supervisor]` in `config.toml`. |

<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Advanced Configuration Guide

This guide covers advanced configuration options for GPY beyond the basic setup.

---

## Configuration Files

GPY uses TOML configuration files stored in `~/.config/gpy/`.

Every key, its type, default and range: [Configuration Reference](configuration-reference.md). Copy-paste recipes: [Configuration Examples](configuration-examples.md).

### Theme Configuration

**File:** `~/.config/gpy/themes/<name>.toml` (user themes) or the built-in
`config/themes/*.toml` shipped with GPY.

A theme file has exactly these sections: `[ui]` (the root prompt symbol
`root_prompt_icon`/`root_prompt_color`, `two_line` and other display keys, all
listed in [Theme Customization](theme-customization.md#theme-ui-keys); the non-root prompt symbol is `[segments.character]`
`success_symbol`/`success_color`), the four glyph tables `[ui.prompt_open]`,
`[ui.prompt_close]`, `[ui.segment_open]`, `[ui.segment_close]`, an optional
`[ui.recommended]` block that `gpy theme use --force` applies to
`config.toml`, and one `[segments.<id>]` table per segment (`clock`,
`duration`, `directory`, `status`, `character`, `git`, `hostname`,
`username`, `language`) holding that segment's colours and format template (`hostname` and `username` are bundled script plugins: colours, icons and `<variant>_*` keys only, with their settings under `[plugin.hostname]` / `[plugin.username]` in `config.toml`).
There is no `[ui.clock]`, `[ui.delimiters]` or `[ui.git]` in a theme file,
and no colours or templates in `config.toml`; the split is described in
[Configuration Reference](configuration-reference.md).

Start from the shipped default rather than a blank file:

```bash
gpy theme new mine --from default   # writes ~/.config/gpy/themes/mine.toml with extends = "default"
gpy theme use mine
```

The new file is an overlay: add only the keys you change, and the rest comes
from `default`. Add `--full` for a complete, independent copy instead.

Every field a theme accepts, with the colour syntaxes and template
placeholders, is in [Theme Customization](theme-customization.md).

---

## Environment Variables

### Agent Control

Turn the agent or its supervisor off in `config.toml`:

```toml
[agent]
# Disable the agent entirely (prompts render in oneshot mode)
enabled = false

[agent.supervisor]
# Disable automatic agent restart supervision
enabled = false
```

An environment variable wins over `config.toml`: `GPY_AGENT_ENABLED`,
`GPY_AGENT_SUPERVISOR_ENABLED`, `GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS`
and `GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS` override the matching config
keys, and the theme export never writes or exports any `GPY_*` variable (it
sets only non-exported `__gpy_cfg_*` values the shell reads when the variable
is unset). In Fish, set the variable before the GPY block in `config.fish`
(`gpy setup` appends that block, which sources `gpy_init.fish`), or in the
environment the shell starts with; the flag functions read it at call time. In
Bash and Zsh, export it before the GPY block in `.bashrc` or `.zshrc`.

```bash
# Disable file watcher (no live git status updates)
export GPY_DISABLE_WATCHER=1   # Bash, Zsh
set -gx GPY_DISABLE_WATCHER 1  # Fish
```

To hide the language segment, turn it off in the config rather than in the
shell: `gpy disable language` removes it from `ui.enabled_segments`. No
`GPY_LANGUAGE_ENABLED` or `GPY_GIT_ENABLED` variable is read.

### Debug Logging

```bash
# Enable debug logging to file
export GPY_DEBUG_LOG=/tmp/gpy-debug.log   # Bash, Zsh
set -gx GPY_DEBUG_LOG /tmp/gpy-debug.log  # Fish

# Then start agent
gpy start

# View logs
tail -f /tmp/gpy-debug.log
```

Every variable GPY reads: [Environment Variables](environment-variables.md).

---

## Performance Tuning

### Git Skip Paths

For large monorepos or network-mounted repositories, add them to the skip list:

```toml
# ~/.config/gpy/config.toml
[git]
skip_paths = [
    "/mnt/network/large-repo",
    "/home/user/massive-monorepo"
]
```

This prevents git operations in those directories, improving prompt responsiveness.

### Git Timeout

Reduce timeout for faster failure on slow repositories:

```toml
[git]
timeout_seconds = 2
```

### Git Branch Truncation

Limit the displayed length of git branch names to prevent long branch names from overwhelming the prompt:

```toml
[git]
max_branch_length = 30
```

When a branch name exceeds the configured length, it will be truncated with an ellipsis (…):

**Example:**
- Branch: `feature/implement-advanced-kubernetes-integration-with-monitoring`
- Displayed (with `max_branch_length = 30`): `feature/implement-advanced-k…`

**Use cases:**
- Teams with long branch naming conventions
- Narrow terminal widths

### Git Icon Set & Stash

Which indicators `icon_set` changes, and the glyphs each set uses: [`[git]`](configuration-reference.md#git) and [`[git.icons]`](configuration-reference.md#giticons).

`stash_enabled = false` skips the `git stash list` call, so `$stash` is always empty.

To override an individual glyph directly (independent of `icon_set`). A theme's own `*_icon` wins over these; icon precedence is described in the [Configuration Reference](configuration-reference.md#giticons):

```toml
[git.icons]
stash = "$"
detached = "@"
in_progress = "~"
```

### Disable Live Updates

If you don't need real-time git status updates on file changes:

```bash
export GPY_DISABLE_WATCHER=1   # Bash, Zsh
set -gx GPY_DISABLE_WATCHER 1  # Fish
```

This reduces system resource usage but requires manual prompt refresh to see git changes.

### Agent-Free Mode

For minimal overhead, run without the agent:

```toml
[agent]
enabled = false
```

Each prompt render will use oneshot mode, which is slower but requires no background process. This holds in Bash, Zsh and Fish alike: the prompt hook is still installed, but no agent is started, no restart check runs, and the shell never registers with an agent.

`[agent.supervisor] enabled = false` (`GPY_AGENT_SUPERVISOR_ENABLED=0`) is different: it only turns off restarts. Shells still start the agent once at startup; if it later crashes, it stays down until you restart it (`gpy restart`).

---

## Security Limits

The IPC guards (message size, connection rate limit, PID and path
validation) are fixed in the agent and are not configurable from
`config.toml`; there is no `[security]` section. Their current values are
64 KiB per message and 1000 connections per second by default, and
[SECURITY.md](../../SECURITY.md#trust-boundaries) summarises what they protect against.

---

## Live Clock Updates

The clock segment can update in real-time while you're at the prompt.

### Enable Live Clock (Updates Every Second)

**Theme Configuration:**
```toml
# ~/.config/gpy/themes/your-theme.toml
[segments.clock]
show_seconds = true
```

**Impact:**
- Prompt repaints every second while idle
- Uses ~2% CPU per terminal on modern systems
- Shows exact time in scrollback history

### Disable Live Clock (Updates Per Minute)

```toml
[segments.clock]
show_seconds = false  # Default
```

**Impact:**
- Prompt repaints only at minute boundaries (XX:00)
- Minimal CPU usage (~0.02% per terminal)
- Historical prompts show minute-level accuracy

---

## Troubleshooting

Agent status: [`gpy status`](cli-reference.md#gpy-status).

### Debug Mode

```bash
# Enable debug logging
export GPY_DEBUG_LOG=/tmp/gpy-debug.log   # Bash, Zsh
set -gx GPY_DEBUG_LOG /tmp/gpy-debug.log  # Fish

# Restart agent
gpy restart

# Monitor logs
tail -f /tmp/gpy-debug.log
```

### Test IPC Connection

`gpy status` connects to the agent and reports whether it answered; `gpy debug paths` prints the socket path every shell uses.

### Reset Configuration

```bash
mv ~/.config/gpy/config.toml ~/.config/gpy/config.toml.bak && gpy restart
```

With no `config.toml`, every key uses its built-in default. If you have no Nerd Font, run `gpy setup --redetect-icons` to pick icons again (`gpy setup` wrote `ui.show_icons = false` into the file you moved).

---

## Custom Themes

Create a new theme that builds on the shipped default (built-in themes live
with the installed binary, not under `~/.config/gpy/themes/`):

```bash
gpy theme new mytheme --from default
```

Add the colors and glyphs you want to change to the new file (it starts as
`extends = "default"`), then activate:

```bash
# Switch to your new theme
gpy theme use mytheme
```

The agent will automatically reload the theme configuration.

---

## Socket Location

The agent socket location follows this priority (the first that applies wins):

1. `--socket <path>` on `gpy-agent start`, `stop`, `status` and `config reload`, or `$GPY_AGENT_SOCKET_PATH` (empty counts as unset); every shell and command must see the same value
2. `$XDG_RUNTIME_DIR/gpy/gpy.sock` (if XDG_RUNTIME_DIR is set to an absolute path)
3. `$XDG_CACHE_HOME/gpy/gpy.sock` (if XDG_CACHE_HOME is set to an absolute path)
4. `~/.cache/gpy/gpy.sock`
5. `/tmp/gpy/gpy.sock` (no home directory)

There is no `config.toml` key for the socket. The agent only uses a socket directory that is a real directory you own; see [Troubleshooting § IPC/Socket Issues](troubleshooting.md#ipcsocket-issues).

To find the current socket:

```bash
gpy status | grep -i socket
```

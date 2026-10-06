<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Advanced Configuration Guide

This guide covers advanced configuration options for GPY beyond the basic setup.

**Last Updated:** 2026-04-08

---

## Configuration Files

GPY uses TOML configuration files stored in `~/.config/gpy/`.

### Main Configuration

**File:** `~/.config/gpy/config.toml`

```toml
[git]
# Skip git operations in these paths (improves performance for large repos)
skip_paths = [
    "/very/large/monorepo",
    "/network/mounted/repos"
]

# Timeout for git operations in seconds
timeout_seconds = 5

[language]
# Enable/disable language detection entirely. Default: true
enabled = true

# Enable/disable version detection for languages. Default: true
show_versions = true

# Cache TTL for language versions in hours. Default: 24
cache_ttl_hours = 24

# List of languages to detect (empty means all). Example: ["rust", "python"]
enabled_languages = []

# Display mode for languages: "icon" or "text". Default: "icon"
display = "icon"

# Filter detected languages: "all", "primary", or top N (e.g., "3"). Default: "all"
# - "all": Show all detected languages (after confidence threshold).
# - "primary": Show only the single most confident language.
# - "N" (e.g., "3"): Show the top N most confident languages.
filter = "all"

# Minimum confidence score (0.0-1.0) for a language to be displayed. Default: 0.1
# Languages with a confidence below this threshold will be hidden.
confidence_threshold = 0.1

[ui]
# Theme name (matches filename in themes/)
theme = "default"

[ui.directory]
# Directory segment display mode: "basename", "abbreviated", "truncated", or "full"
display = "basename"
# Path components kept when display = "truncated". Default: 3
truncation_length = 3
# Prefix shown before a truncated path (e.g. "…/"). Default: ""
truncation_symbol = ""
# Maximum displayed length of the rendered directory string
max_length = 80
# Anchor the path at the enclosing git repo root (affects "truncated"/"full"). Default: false
truncate_to_repo = false

```

`ui.directory.display` controls how much path context the directory segment shows:

- `basename` shows only the current directory name
- `abbreviated` shortens intermediate path components to their first character (a hidden directory keeps its leading dot plus one character, so `~/.config/fish` becomes `~/.c/fish`) while keeping the leaf directory
- `truncated` keeps the last `ui.directory.truncation_length` path components, prefixed with `ui.directory.truncation_symbol`
- `full` shows the whole path with your home directory shown as `~` (for example `~/work/project`)

`ui.directory.max_length` is applied after that rendering step. For example, a `full` path can still be shortened with an ellipsis if it exceeds the configured maximum. At `max_length` values of `1`–`3` there isn't room for both the 3-character `"..."` ellipsis and any real path content, so the ellipsis is dropped and the raw last `max_length` characters are shown instead — the cap is always honored exactly.

`ui.directory.truncate_to_repo` (default `false`) anchors the displayed path at
the enclosing git repository root: the repo folder becomes the leading
component and everything above it is dropped. It affects `truncated` and `full`
(not `basename`/`abbreviated`), and applies before `max_length`. Outside a git
repo it has no effect. A repo rooted at your home directory (e.g. a dotfiles
repo) is not used as an anchor, so paths under it still show as `~/…`. The Starship preset enables it via
`gpy theme use starship --apply-layout`.

### Theme Configuration

**File:** `~/.config/gpy/themes/<name>.toml` (user themes) or the built-in
`config/themes/*.toml` shipped with GPY.

A theme file has exactly these sections: `[ui]` (the theme's name and a few
display defaults), the four glyph tables `[ui.prompt_open]`,
`[ui.prompt_close]`, `[ui.segment_open]`, `[ui.segment_close]`, an optional
`[ui.recommended]` block that `gpy theme use --force` applies to
`config.toml`, and one `[segments.<id>]` table per segment (`clock`,
`duration`, `directory`, `status`, `character`, `git`, `hostname`,
`username`, `language`) holding that segment's colours and format template.
There is no `[ui.clock]`, `[ui.delimiters]` or `[ui.git]` in a theme file,
and no colours or templates in `config.toml`; the split is described in
[Configuration Reference](configuration-reference.md).

Start from the shipped default rather than a blank file:

```bash
gpy theme new mine --from default   # copies config/themes/default.toml to ~/.config/gpy/themes/mine.toml
gpy theme use mine
```

Every field a theme accepts, with the colour syntaxes and template
placeholders, is in [Theme Customization](theme-customization.md).

---

## Environment Variables

### Agent Control

```fish
# Disable agent entirely (use oneshot mode only)
set -gx GPY_AGENT_ENABLED 0

# Disable automatic agent restart supervision
set -gx GPY_AGENT_SUPERVISOR_ENABLED 0

# Disable file watcher (no live git status updates)
set -gx GPY_DISABLE_WATCHER 1

```

To hide the language segment, turn it off in `config.toml` rather than in
the shell: the theme export the shell sources on every start re-exports
`GPY_LANGUAGE_ENABLED` (and every other segment flag) from `config.toml`, so
a `set -gx` in `config.fish` is overwritten (#657).

```toml
[language]
enabled = false
```

### Debug Logging

```fish
# Enable debug logging to file
set -gx GPY_DEBUG_LOG /tmp/gpy-debug.log

# Then start agent
gpy start

# View logs
tail -f /tmp/gpy-debug.log
```

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
timeout_seconds = 2  # Default: 5
```

### Git Branch Truncation

Limit the displayed length of git branch names to prevent long branch names from overwhelming the prompt:

```toml
[git]
max_branch_length = 30  # Default: 0 (unlimited)
```

When a branch name exceeds the configured length, it will be truncated with an ellipsis (…):

**Example:**
- Branch: `feature/implement-advanced-kubernetes-integration-with-monitoring`
- Displayed (with `max_branch_length = 30`): `feature/implement-advanced-k…`

### Git Icon Set & Stash

Two `[git]` keys control the new state/stash/detached-HEAD indicators
(the four pre-existing status icons — staged/unstaged/untracked/conflicts —
are unaffected by either):

```toml
[git]
icon_set = "unicode"    # or "nerd_font"
stash_enabled = true     # default; set false to skip `git stash list` entirely
```

- `icon_set`: selects the glyph set for the stash (Unicode `≡` / Nerd Font
  `nf-fa-archive`), detached-HEAD (Unicode `➦` / Nerd Font `nf-fa-code_fork`),
  and in-progress (Unicode `↻` / Nerd Font `nf-fa-refresh`) indicators. An
  explicit override under `[git.icons]` (below) always wins regardless of
  `icon_set`.
- `stash_enabled`: when `false`, the agent skips the `git stash list`
  subprocess call entirely and `$stash` always renders empty — use this on
  very large repos where the extra call is undesirable.

To override an individual glyph directly (independent of `icon_set`):

```toml
[git.icons]
stash = "$"
detached = "@"
in_progress = "~"
```

**Use cases:**
- Teams with long branch naming conventions
- Narrow terminal widths
- Keeping prompt compact and readable

**Note:** Setting `max_branch_length = 0` (the default) disables truncation.

### Disable Live Updates

If you don't need real-time git status updates on file changes:

```fish
set -gx GPY_DISABLE_WATCHER 1
```

This reduces system resource usage but requires manual prompt refresh to see git changes.

### Agent-Free Mode

For minimal overhead, run without the agent:

```fish
set -gx GPY_AGENT_ENABLED 0
```

Each prompt render will use oneshot mode, which is slower (~20ms) but requires no background process.

---

## Security Limits

The IPC guards (message size, connection rate limit, PID and path
validation) are fixed in the agent and are not configurable from
`config.toml`; there is no `[security]` section. Their current values live in
`gpy-agent/src/security.rs` (64 KiB messages, 1000 connections per second by
default) and are covered in [SECURITY.md](../../SECURITY.md).

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

### Check Agent Status

```bash
gpy status
```

Shows:
- Whether agent is running
- Socket path
- Number of connected clients
- Configuration file locations

### Debug Mode

```fish
# Enable debug logging
set -gx GPY_DEBUG_LOG /tmp/gpy-debug.log

# Restart agent
gpy restart

# Monitor logs
tail -f /tmp/gpy-debug.log
```

### Test IPC Connection

```fish
# From Fish shell, test IPC manually
echo '{"op":"ping"}' | socat - UNIX-CONNECT:~/.cache/gpy/gpy.sock
# Should respond: {"ack":true}
```

### Performance Benchmarking

```fish
# Check agent status
gpy status

# Run diagnostics
gpy doctor
```

### Reset Configuration

```bash
# Backup current config
mv ~/.config/gpy ~/.config/gpy.backup

# Restart agent (will create new default config)
gpy restart
```

---

## Custom Themes

Create a new theme from the shipped default (built-in themes live with the
installed binary, not under `~/.config/gpy/themes/`, so copy through the CLI):

```bash
gpy theme new mytheme --from default
```

Edit colors and glyphs as desired, then activate:

```bash
# Switch to your new theme
gpy theme use mytheme
```

The agent will automatically reload the theme configuration.

---

## Socket Location

The agent socket location follows this priority:

1. `$XDG_RUNTIME_DIR/gpy/gpy.sock` (if XDG_RUNTIME_DIR is set)
2. `$XDG_CACHE_HOME/gpy/gpy.sock` (if XDG_CACHE_HOME is set)
3. `~/.cache/gpy/gpy.sock` (fallback)

To find the current socket:

```bash
gpy status | grep -i socket
```

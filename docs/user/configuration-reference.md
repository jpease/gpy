<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Configuration Reference

A complete field-by-field reference for `~/.config/gpy/config.toml`: every section, key, type, default, and valid range.

**Last Updated:** 2026-07-07

---

## Scope

This document covers `config.toml` only — the file that controls agent behavior, git detection, language detection, and top-level UI layout. It does **not** cover:

- **Theme files** (`~/.config/gpy/themes/*.toml`), which define colors, per-segment templates, and delimiter glyphs. See [Advanced Configuration](advanced-configuration.md) for a theme-file example and the theme-customization guide once it exists.
- **CLI commands** for reading/writing config values. See [CLI Reference](cli-reference.md).

`config.toml` has exactly four top-level sections: `[agent]`, `[git]`, `[language]`, and `[ui]`. There is no `[security]`, `[segments]`, or `[plugins]` section in `config.toml` — those either don't exist or live elsewhere (segment templates and colors live in theme files, under `[segments.<id>]`).

## File Location

The agent searches for a config file in this order and uses the first one found:

1. `$GPY_CONFIG_PATH` (explicit override; a relative path is resolved against the directory you run the command or start the agent from)
2. `$XDG_CONFIG_HOME/gpy/config.toml`
3. `~/.config/gpy/config.toml` (fallback when `XDG_CONFIG_HOME` is unset)
4. Built-in defaults, if no file exists anywhere above

There is no per-directory config: a `.gpy.toml` in the current directory is not read.

Every field below is optional — anything you omit falls back to its default, so a minimal or even empty `config.toml` is valid.

---

## Worked Example

A full config with realistic (non-default) values, annotated with why you'd change them:

```toml
# ~/.config/gpy/config.toml

[agent]
enabled = true
timeout_seconds = 3       # Lower than the default 5s: fail fast on a slow/stuck agent
live_updates = true

[agent.supervisor]
enabled = true
check_interval_seconds = 15  # Check more often than the default 30s on a flaky machine
max_restart_attempts = 3     # Give up sooner than the default 5 to avoid restart-looping

[git]
enabled = true
show_upstream = true
timeout_seconds = 3          # Default is 10s; lower it in CI containers with fast local repos
skip_paths = ["/mnt/network-share", "/home/me/vendor"]  # Skip git status in slow/irrelevant trees
max_branch_length = 30       # Truncate long branch names instead of the default unlimited (0)
max_ahead_behind = 50        # Cap the ahead/behind counter lower than the default 100
watch_worktree = true
icon_set = "nerd_font"       # Requires a patched Nerd Font; default is "unicode"
stash_enabled = false        # Skip the extra `git stash list` call on very large repos

[git.icons]
stash = "$"                  # Override a single glyph independent of icon_set
detached = "@"

[language]
enabled = true
show_versions = true
cache_ttl_hours = 6          # Refresh version info more often than the default 24h
enabled_languages = ["rust", "python", "node"]  # Only show these; default [] shows all
display = "text"             # Print names instead of icons; default is "icon"
filter = "primary"           # Show only the dominant language; default is "all"
detection_mode = "markers"   # Show a language only when project markers exist; default "content"
confidence_threshold = 0.3   # Raise above the default 0.1 to hide low-confidence noise

[ui]
show_icons = true
theme = "starship"           # Must match a theme file under ~/.config/gpy/themes/
palette = "nord"
enabled_segments = ["clock", "language", "directory", "git"]  # Drop "duration"

[ui.directory]
display = "truncated"        # Default is "basename" (current dir name only)
truncation_length = 4        # Keep more trailing path components than the default 3
truncation_symbol = "…/"
max_length = 60               # Lower than the default 80 for narrow terminals
truncate_to_repo = true       # Anchor at the git repo root instead of the filesystem root
```

---

## `[agent]`

Controls the background agent process that serves prompt data over IPC.

| Key | Type | Default | Range | Effect |
|---|---|---|---|---|
| `enabled` | bool | `true` | — | Enables background agent mode. When `false`, every prompt render falls back to oneshot mode (slower, ~20ms, no persistent process). |
| `timeout_seconds` | integer | `5` | `1`–`300` | IPC socket timeout in seconds before the shell gives up waiting on the agent and falls back to oneshot rendering. |
| `live_updates` | bool | `true` | — | Enables live prompt updates pushed via signals (e.g., when a file watcher detects a git change) instead of only refreshing on the next command. |

### `[agent.supervisor]`

Controls automatic restart of a crashed or unresponsive agent process.

| Key | Type | Default | Range | Effect |
|---|---|---|---|---|
| `enabled` | bool | `true` | — | Enables the supervisor. When `false`, a crashed agent stays down until manually restarted (`gpy restart`). |
| `check_interval_seconds` | integer | `30` | `5`–`3600` | How often the supervisor checks agent health. |
| `max_restart_attempts` | integer | `5` | `1`–`100` | Maximum consecutive restart attempts before the supervisor gives up and leaves the agent down. |

```toml
[agent]
enabled = true
timeout_seconds = 5
live_updates = true

[agent.supervisor]
enabled = true
check_interval_seconds = 30
max_restart_attempts = 5
```

---

## `[git]`

Controls git status detection: what's computed, how it's capped, and which icons represent it.

| Key | Type | Default | Range | Effect |
|---|---|---|---|---|
| `enabled` | bool | `true` | — | Enables git status detection entirely. When `false`, the git segment never renders and no git subprocess/libgit2 calls are made. |
| `show_upstream` | bool | `true` | — | Shows ahead/behind commit counts relative to the upstream branch. |
| `timeout_seconds` | integer | `10` | `1`–`600` | Timeout for the underlying git subprocess. The IPC handler also uses it (plus a small margin) as the ceiling it waits for a fresh status before falling back to stale/cached data. |
| `skip_paths` | array of strings | `[]` | must be absolute (`/...`) or home-relative (`~/...`) paths | Directories where git detection is skipped entirely — useful for huge monorepos or slow network mounts. Entries are matched component-wise after `~` expansion and symlink resolution, so `~/x`, `/tmp/x` and `/private/tmp/x` all work (`/tmp/foo` does not skip `/tmp/foobar`). Skipped directories get no git status, no instant-prompt cache and no watcher refresh, and `gpy-agent oneshot git` returns the disabled error. |
| `max_branch_length` | integer | `0` | `0`–`500` | Maximum displayed branch name length; `0` means unlimited. Longer names are truncated with an ellipsis. |
| `max_ahead_behind` | integer | `100` | `0` = unlimited | Ceiling on the ahead/behind commit counters. Counts above this value are clamped and rendered with a trailing `+`. |
| `watch_worktree` | bool | `true` | — | Watches the working tree (not just `.git`) so editing a tracked file updates the prompt instantly (honors `.gitignore`). Can be overridden at runtime with the `GPY_WATCH_WORKTREE` environment variable. **When disabled**, pure working-tree changes (new untracked file, editing/removing a tracked file — anything that touches nothing under `.git`) fire no event, so dirty/untracked state surfaces only on the next `.git` write or the periodic reconcile scan (default 45s), not instantly. Lower `GPY_RECONCILE_INTERVAL_SECS` to tighten that bound at the cost of more frequent rescans. |
| `icon_set` | string | `"unicode"` | `"unicode"`, `"nerd_font"` | Glyph set for the stash, detached-HEAD, and in-progress indicators only (does **not** affect the staged/unstaged/untracked/conflicts icons below). An explicit `[git.icons]` override always wins regardless of this setting. |
| `stash_enabled` | bool | `true` | — | Enables the `git stash list` subprocess call (and `$stash` rendering). Disabling it skips that extra call entirely — useful on very large repos. |

gpy enables git's `core.untrackedCache` (a status speed-up) in repositories where it is unset at every scope. It never changes a value you have set, locally or globally; set `core.untrackedCache` to `false` to opt out.

### `[git.icons]`

Semantic icon overrides. Each accepts any non-empty string without control characters (so multi-character glyphs and Nerd Font codepoints both work).

| Key | Default | Meaning |
|---|---|---|
| `ahead` | `"↑"` | Commits ahead of upstream |
| `behind` | `"↓"` | Commits behind upstream |
| `staged` | `"✚"` | Staged changes present |
| `unstaged` | `"✱"` | Unstaged changes present |
| `untracked` | `"?"` | Untracked files present |
| `conflicts` | `"✖"` | Merge conflicts present |
| `stash` | `"≡"` | Stash entries present |
| `detached` | `"➦"` | Detached HEAD state |
| `in_progress` | `"↻"` | Rebase/merge/cherry-pick in progress |

```toml
[git]
enabled = true
show_upstream = true
timeout_seconds = 10
skip_paths = []
max_branch_length = 0
max_ahead_behind = 100
watch_worktree = true
icon_set = "unicode"
stash_enabled = true

[git.icons]
ahead = "↑"
behind = "↓"
staged = "✚"
unstaged = "✱"
untracked = "?"
conflicts = "✖"
stash = "≡"
detached = "➦"
in_progress = "↻"
```

---

## `[language]`

Controls programming-language/tool detection: what's shown, how confident it must be, and how it's displayed.

Detection runs at the project root, so every subdirectory shows the project's languages: inside a git repository that is the repo root; outside git it is the nearest ancestor directory holding a project marker file (`package.json`, `Cargo.toml`, `go.mod`, …), searched up to but never including `$HOME`. A directory with no such ancestor is detected on its own.

| Key | Type | Default | Range | Effect |
|---|---|---|---|---|
| `enabled` | bool | `true` | — | Enables language detection entirely. |
| `show_versions` | bool | `true` | — | Enables version detection/display (e.g., `node 20.11.0`) alongside the language name/icon. |
| `cache_ttl_hours` | integer | `24` | `1`–`720` | How long detected version info is cached before being re-checked. |
| `enabled_languages` | array of strings | `[]` | any known language identifier | Restricts detection to this list; empty means detect all supported languages. |
| `display` | string | `"icon"` | `"icon"`, `"text"` | Shows a language icon (falling back to the name when no icon exists) or always shows the plain text name. |
| `filter` | string | `"all"` | `"all"`, `"primary"`, or a number (e.g. `"3"`) | `"all"` shows every detected language above the confidence threshold; `"primary"` shows only the most confident one; a number shows the top N. |
| `detection_mode` | string | `"content"` | `"content"`, `"markers"`, `"hybrid"` | `"content"` scans the directory's files and ranks by prevalence — zero-config but can be noisy. `"markers"` shows a language only when its project marker files/extensions are present (Starship-style, more conservative). `"hybrid"` runs the content scan but keeps only languages whose project marker *file* is also present, so a stray helper script (e.g. a lone `.py` file in a Rust repo) doesn't surface a language the repo isn't actually a project of, while still ranking survivors by prevalence. |
| `confidence_threshold` | float | `0.1` | `0.0`–`1.0` | Minimum confidence score a detected language must meet to be shown. |

### `[language.icons]`

A free-form map of language identifier to icon glyph (e.g. `rust = "🦀"`). Unlisted languages fall back to built-in defaults. Per-language color and style overrides (`rust_bg_color`, `java_style`, etc.) live in theme files under `[segments.language]`, not here — see [Advanced Configuration](advanced-configuration.md).

```toml
[language]
enabled = true
show_versions = true
cache_ttl_hours = 24
enabled_languages = []
display = "icon"
filter = "all"
detection_mode = "content"
confidence_threshold = 0.1

[language.icons]
rust = "🦀"
python = "🐍"
```

---

## `[ui]`

Controls top-level layout: theme selection and which segments render.

| Key | Type | Default | Range | Effect |
|---|---|---|---|---|
| `show_icons` | bool | `true` | — | Shows icons (language, git, etc.) instead of falling back to text-only rendering where supported. Setting it to `false` also drops the Nerd Font powerline caps (U+E0BA, U+E0BC, U+E0B4) and any private-use delimiter glyph the theme exports to the shell, leaving flat colored blocks that render in any font. Plain-text delimiters such as `[`/`]` are kept. |
| `theme` | string | `"default"` | any theme file stem under `~/.config/gpy/themes/` (no path separators, no `..`, no control characters) | Selects which theme file controls colors and segment templates. |
| `palette` | string | `"default"` | any palette file stem under `~/.config/gpy/palettes/` (same naming rules as `theme`) | Selects the named-color set (e.g. `nord`, `catppuccin-mocha`, `gruvbox-dark-medium`) used when a theme references colors by name. |
| `enabled_segments` | array of strings | `["clock", "duration", "language", "directory", "git"]` | segment names, alphanumeric/underscore/hyphen only | The segments to render, in order. Segment names not on the built-in list must still pass the naming check (a plugin can register additional segment names). |

Delimiters (prompt/segment open and close icons and colors) are not a `config.toml` setting — they're defined per-theme. See [Advanced Configuration](advanced-configuration.md) and `docs/dev/create-theme.md` to customize them.

### `[ui.directory]`

Controls how the directory segment renders the current path.

| Key | Type | Default | Range | Effect |
|---|---|---|---|---|
| `display` | string | `"basename"` | `"basename"`, `"abbreviated"`, `"truncated"`, `"full"` | `"basename"` shows only the current directory name; `"abbreviated"` shortens intermediate components to their first character (hidden directories keep the leading dot plus one character, e.g. `~/.c/fish`); `"truncated"` keeps the last `truncation_length` components; `"full"` shows the whole path with your home directory shown as `~` (e.g. `~/work/project`). |
| `truncation_length` | integer | `3` | `1`–`255` | Trailing path components kept when `display = "truncated"`. |
| `truncation_symbol` | string | `""` | no control characters | Prefix shown before a truncated path (e.g. `"…/"`). |
| `max_length` | integer | `80` | `1`–`1000` | Character cap applied to the rendered directory string after the display mode is resolved — even a `"full"` path can be shortened if it exceeds this. Truncation normally prefixes a 3-character `"..."` ellipsis before the kept tail; for `max_length` values of `1`–`3` (too small to fit an ellipsis plus any real content) the ellipsis is dropped and the raw last `max_length` characters are shown instead. |
| `truncate_to_repo` | bool | `false` | — | Anchors the displayed path at the enclosing git repo root (the repo folder becomes the leading component). Affects `"truncated"`/`"full"` only, applied before `max_length`. No effect outside a git repo, and a repo rooted at `$HOME` is ignored (paths show as `~/…`). |

```toml
[ui]
show_icons = true
theme = "default"
palette = "default"
enabled_segments = ["clock", "duration", "language", "directory", "git"]

[ui.directory]
display = "basename"
truncation_length = 3
truncation_symbol = ""
max_length = 80
truncate_to_repo = false
```

---

## Applying Changes

`config.toml` is hot-reloaded by the running agent — no restart needed for most edits. Every reload re-resolves which config file is in effect (the highest-priority one that exists), so the agent also follows a config file that you fix after it failed to load at startup, create at a higher-priority location, or delete — deleting the active `config.toml` returns the agent to the built-in defaults. A file that exists but is invalid keeps the last good configuration.

An edit made while the agent is stopped takes effect when the agent next starts. Shells that were open across the restart reload the new theme on their own. A new Fish shell that loaded the old theme at startup reloads it as soon as it registers with the agent.

Use the CLI to inspect or edit values without hand-editing the file:

```bash
gpy config show          # Show the full effective configuration
gpy config show git      # Show only the [git] section
gpy config get ui.theme
gpy config set git.timeout_seconds 5
gpy config open          # Open config.toml in $EDITOR
gpy config wizard        # Interactive TUI for common settings
```

Commands that change the config (`config set`, `theme use`, `palette use`, `enable`/`disable`, `lang versions`, `theme import --apply-layout`, the wizard) edit `config.toml` in place: they change only the keys they set, and keep your comments, key order, unknown keys, and unset defaults. A symlinked `config.toml` stays a symlink and its target is updated. `gpy-agent init --force` is the exception: it replaces the file.

See [CLI Reference](cli-reference.md) for full command details.

---

## See Also

- [Advanced Configuration](advanced-configuration.md) — theme files, environment variables, performance tuning, and troubleshooting.
- [CLI Reference](cli-reference.md) — `gpy config show/get/set/open/wizard` and every other `gpy` subcommand.

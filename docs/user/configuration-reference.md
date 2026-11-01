<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Configuration Reference

A complete field-by-field reference for `~/.config/gpy/config.toml`: every section, key, type, default, and valid range.

---

## Scope

This document covers `config.toml` only — the file that controls agent behavior, git detection, language detection, and top-level UI layout. It does **not** cover:

- **Theme files** (`~/.config/gpy/themes/*.toml`), which define colors, per-segment templates, and delimiter glyphs. See [Theme Customization](theme-customization.md) and [Advanced Configuration](advanced-configuration.md).
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
detection_mode = "markers"   # Show a language only when project markers exist; default "markers"
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
| `enabled` | bool | `true` | — | Enables background agent mode. When `false`, every prompt render falls back to oneshot mode (slower, no persistent process). |
| `timeout_seconds` | integer | `5` | `1`–`300` | IPC socket timeout in seconds before the shell gives up waiting on the agent and falls back to oneshot rendering. |
| `live_updates` | bool | `true` | — | Enables live prompt updates pushed via signals (e.g., when a file watcher detects a git change) instead of only refreshing on the next command. |

### `[agent.supervisor]`

Controls automatic restart of a crashed or unresponsive agent process. In every shell (Bash, Zsh, Fish) this flag governs restarts only: with `enabled = false` the shell still starts the agent once when it starts (as long as `[agent] enabled` is `true`), but a crashed agent stays down afterwards. Only `[agent] enabled = false` (`GPY_AGENT_ENABLED=0`) stops shells from starting, registering with, or supervising an agent.

`GPY_AGENT_ENABLED` and `GPY_AGENT_SUPERVISOR_ENABLED` follow one rule in Bash, Zsh and Fish, and the same table as `GPY_SHOW_STATUS` (default on): `1`, `true`, `yes` and `on` mean on, `0`, `false`, `no` and `off` mean off, case-insensitive with no surrounding whitespace. Any other value, including empty, counts as unset (logged once under `GPY_DEBUG=1`). Precedence is the environment variable, then `config.toml`, then the built-in default; the theme export never exports these names.

`GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS` and `GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS` follow the same precedence: any non-negative integer in the environment wins over `config.toml` and is not range-checked (`config.toml` itself accepts only 5..=3600 and 1..=100). `0` for the interval means every prompt, `0` for the attempts means never restart, and a non-integer counts as unset.

All three shells check the agent the same way. At startup they start the agent in the background if it is not reachable (a reachable agent on another protocol version counts as not reachable, and `gpy-agent start` replaces it), and never wait for it. Then, on the prompt and at most once per `check_interval_seconds`, the shell probes the agent; if the probe fails it runs `gpy-agent start` in the background, at most `max_restart_attempts` times per shell. A successful probe resets that count. Bash and Fish ping with the prompt's IPC budget (`GPY_IPC_TIMEOUT_MS`), so a silent agent costs up to that budget on the prompt that runs the check; Zsh uses `gpy-agent status`. A crashed agent is therefore restarted on the next checked prompt, not while the shell sits idle. No shell kills an agent: a wedged agent that ignores `gpy-agent stop` must be killed by hand.

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
| `enabled` | bool | `true` | — | Deprecated: use `gpy disable git` / `gpy enable git` (`ui.enabled_segments`). Still loads: `false` removes a listed `git` segment, so no git calls are made. `gpy doctor` warns when it is present. |
| `show_upstream` | bool | `true` | — | Shows ahead/behind commit counts relative to the upstream branch. |
| `timeout_seconds` | integer | `10` | `1`–`600` | Time budget for one git status: every git subprocess it runs (status, stash count, branch fallback, untracked-cache setup) shares it, and a subprocess still running when it ends is killed. The IPC handler also uses it (plus a small margin) as the ceiling it waits for a fresh status before falling back to stale/cached data. |
| `skip_paths` | array of strings | `[]` | must be absolute (`/...`) or home-relative (`~/...`) paths | Directories whose repositories get no git detection — useful for huge monorepos or slow network mounts. A repository is skipped when its root (the directory holding `.git`) is at or under an entry, so an entry can name a repository or a directory of repositories. An entry inside a repository (below its root) does not skip that repository, not even for prompts in that subdirectory. Entries are matched component-wise after `~` expansion and symlink resolution, so `~/x`, `/tmp/x` and `/private/tmp/x` all work (`/tmp/foo` does not skip `/tmp/foobar`). The running agent re-resolves entries every few seconds, so a skipped directory or symlink created, removed or retargeted after startup takes effect without a restart. Skipped repositories get no git status, no instant-prompt cache and no background refresh (watcher events or the periodic reconcile), and `gpy-agent oneshot git` returns the disabled error. Skipped repositories are not watched either: a new or removed language shows within 30 s, but a changed version in a project file (`.tool-versions`, `rust-toolchain.toml`, `.python-version`) shows only after `language.cache_ttl_hours` (default 24 h). A running agent stops watching a newly skipped repository at the next config reload or when its shells change directory. |
| `max_branch_length` | integer | `0` | `0`–`500` | Maximum displayed branch name length; `0` means unlimited. Longer names are truncated with an ellipsis. Branch names are shown whole by default; truncating hides information. |
| `max_ahead_behind` | integer | `100` | `0` = unlimited | Largest ahead/behind count displayed; larger counts show as `N+`. Display only: git still counts them all; `0` = no cap. The cap keeps a stale branch from drawing a five-digit counter. |
| `watch_worktree` | bool | `true` | — | Watches the working tree (not just `.git`) so editing a tracked file updates the prompt instantly (honors `.gitignore`). With the default `true` the agent watches the whole working tree of every repository you enter, recursively; a repository rooted at your home directory or `/` gets only its gitdir watched. Can be overridden at runtime with the `GPY_WATCH_WORKTREE` environment variable. **When disabled**, pure working-tree changes (new untracked file, editing/removing a tracked file — anything that touches nothing under `.git`) fire no event, so dirty/untracked state surfaces only on the next `.git` write or the periodic reconcile scan (default 45s), not instantly. Lower `GPY_RECONCILE_INTERVAL_SECS` to tighten that bound at the cost of more frequent rescans. |
| `icon_set` | string | `"unicode"` | `"unicode"`, `"nerd_font"` | Default glyphs for the stash, detached-HEAD, and in-progress indicators only (does **not** affect the staged/unstaged/untracked/conflicts icons below, the branch glyph, or the powerline caps: those follow `ui.show_icons` and need a patched font in either set). An explicit `[git.icons]` value always wins over the `icon_set` default. |
| `stash_enabled` | bool | `true` | — | Enables the `git stash list` subprocess call (and `$stash` rendering). Disabling it skips that extra call entirely — useful on very large repos. |

In repositories where `core.untrackedCache` is unset, the GPY agent adds git's untracked cache to the index, and git keeps it from then on. GPY never writes your git config, and a value you set yourself is respected. The one-shot fallback (`gpy-agent oneshot`) and `gpy` commands such as `gpy config wizard` do not do this. To opt out, run `git config --global core.untrackedCache keep` (leave git's default behaviour and stop GPY adding the cache) or set it to `false` (which also removes it).

### `[git.icons]`

Semantic icon overrides. Each accepts any non-empty string without control characters (so multi-character glyphs and Nerd Font codepoints both work). A key you leave out uses the default for `git.icon_set`; `gpy config get git.icons.<name>` prints the effective glyph.

Icon precedence, for git and language icons alike: `ui.show_icons` gates first; then a theme icon (`[segments.git]` `*_icon`, `[segments.language]` `<lang>_symbol`); then an icon you set in `config.toml` (`[git.icons]`, `[language.icons]`); then the built-in default. To change a theme's icon, use a theme overlay (see [Theme Customization](theme-customization.md)).

| Key | Default | With `icon_set = "nerd_font"` | Meaning |
|---|---|---|---|
| `ahead` | `"↑"` | `"↑"` | Commits ahead of upstream. Overrides the `icon_set` default when set. |
| `behind` | `"↓"` | `"↓"` | Commits behind upstream. Overrides the `icon_set` default when set. |
| `staged` | `"✚"` | `"✚"` | Staged changes present. Overrides the `icon_set` default when set. |
| `unstaged` | `"✱"` | `"✱"` | Unstaged changes present. Overrides the `icon_set` default when set. |
| `untracked` | `"?"` | `"?"` | Untracked files present. Overrides the `icon_set` default when set. |
| `conflicts` | `"✖"` | `"✖"` | Merge conflicts present. Overrides the `icon_set` default when set. |
| `stash` | `"≡"` | `""` | Stash entries present. Overrides the `icon_set` default when set. |
| `detached` | `"➦"` | `""` | Detached HEAD state. Overrides the `icon_set` default when set. |
| `in_progress` | `"↻"` | `""` | Rebase/merge/cherry-pick in progress. Overrides the `icon_set` default when set. |

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

Version probes run the tool in the project directory with the agent's environment (the PATH of the shell or service that started it), not each shell's. Project version files are honoured wherever the tool or shim on the agent's PATH reads them: mise (`.tool-versions`, `mise.toml`), and pyenv, rbenv, nodenv or asdf shims (`.python-version`, `.ruby-version`, `.node-version`, `.tool-versions`). Per-shell state is not seen: `nvm use` and `.nvmrc` (nvm is a shell function), a direnv-exported PATH, `PYENV_VERSION` or `asdf shell`. Virtualenvs are the exception: the shell sends `VIRTUAL_ENV` with each request.

| Key | Type | Default | Range | Effect |
|---|---|---|---|---|
| `enabled` | bool | `true` | — | Deprecated: use `gpy disable language` / `gpy enable language` (`ui.enabled_segments`). Still loads: `false` removes a listed `language` segment. `gpy doctor` warns when it is present. |
| `show_versions` | bool | `true` | — | Enables version detection/display (e.g., `node 20.11.0`) alongside the language name/icon. |
| `cache_ttl_hours` | integer | `24` | `1`–`720` | How long language version, mise and virtualenv lookups are cached. A tool upgrade that changes no version file shows after this period. |
| `enabled_languages` | array of strings | `[]` | any known language identifier | Restricts detection to this list; empty means detect all supported languages. Shown space-separated in `gpy config get`. |
| `display` | string | `"icon"` | `"icon"`, `"text"` | Shows a language icon (the theme's symbol, the `[language.icons]` entry, or the built-in icon, falling back to the name when there is none) or always shows the plain text name. Icons are drawn only when `ui.show_icons = true`. |
| `filter` | string | `"all"` | `"all"`, `"primary"`, or a number (e.g. `"3"`) | `"all"` shows every detected language above the confidence threshold; `"primary"` shows only the most confident one; a number shows the top N. The prompt draws at most three of them; JSON output lists them all. In `markers` mode every detected language has confidence 1.0, so `"primary"` picks the language with the most marker evidence (ties by name). |
| `detection_mode` | string | `"markers"` | `"markers"`, `"content"`, `"hybrid"` | `"markers"` (default) shows a language only when its project marker files/extensions are present (Starship-style, one directory read, predictable). `"content"` scans the directory's files and ranks by prevalence — zero-config but can be noisy and costs a full tree walk. `"hybrid"` runs the content scan but keeps only languages whose project marker *file* is also present, so a stray helper script (e.g. a lone `.py` file in a Rust repo) doesn't surface a language the repo isn't actually a project of, while still ranking survivors by prevalence. `"content"` and `"hybrid"` fall back to `"markers"` for a directory outside a git repository or with more than 20,000 files, so the same config can follow different rules per directory. |
| `confidence_threshold` | float | `0.1` | `0.0`–`1.0` | Minimum confidence score a detected language must meet to be shown. Confidence ranks languages only in `content` and `hybrid` modes. In `markers` mode every detected language has confidence 1.0, so the threshold filters nothing and `primary` picks the language with the most marker evidence (ties by name). |

### `[language.icons]`

A free-form map of language identifier to icon glyph (e.g. `rust = "🦀"`). Languages with no entry use a built-in Nerd Font icon (rust, python, node, go, java, ruby, swift, elixir, php, csharp, cpp, c, erlang, fish; they need a patched font and are drawn when `ui.show_icons = true`), and any other language shows its name. Icon precedence: `ui.show_icons` gates first, then a theme's `<lang>_symbol`, then an entry here, then the built-in icon; see [`[git.icons]`](#giticons). `gpy config get language.icons.<lang>` reads only what `config.toml` sets, so it reports an unset entry even when a built-in icon applies. Per-language color and style overrides (`rust_bg_color`, `java_style`, etc.) live in theme files under `[segments.language]`, not here — see [Advanced Configuration](advanced-configuration.md).

```toml
[language]
enabled = true
show_versions = true
cache_ttl_hours = 24
enabled_languages = []
display = "icon"
filter = "all"
detection_mode = "markers"
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
| `show_icons` | bool | `true` | — | Shows icons (language, git, etc.) instead of falling back to text-only rendering where supported. Setting it to `false` also drops the Nerd Font powerline caps (U+E0BA, U+E0BC, U+E0B4) and any private-use delimiter glyph the theme exports to the shell, leaving flat colored blocks that render in any font. Plain-text delimiters such as `[`/`]` are kept. A private-use status icon (`[segments.status]` `ok_icon`/`fail_icon`) is replaced by the plain `ok`/`err`, the directory's read-only marker is `ro` instead of 🔒 or a private-use `read_only_icon`, and a private-use hostname or username `icon` is dropped. |
| `theme` | string | `"default"` | any theme file stem under `~/.config/gpy/themes/` (no path separators, no `..`, no control characters) | Selects which theme file controls colors and segment templates. |
| `palette` | string | `"default"` | any palette file stem under `~/.config/gpy/palettes/` (same naming rules as `theme`) | Selects the named-color set (e.g. `nord`, `catppuccin-mocha`, `gruvbox-dark-medium`) used when a theme references colors by name. |
| `enabled_segments` | array of strings | `["clock", "duration", "language", "directory", "git"]` | segment names, alphanumeric/underscore/hyphen only; names that differ only by `-` vs `_` (e.g. `a-b` and `a_b`) may not both appear | The only switch for whether a segment renders (and whether the agent does its work), in order. The six built-ins are `clock`, `duration`, `language`, `directory`, `git` and `status` (`status` is off by default); `hostname` and `username` are bundled plugins. The prompt character is always drawn and is not listed. Names are trimmed and lowercased on load (`"Directory"` and `" git "` mean `directory` and `git`), and `gpy config get`, `gpy doctor` and every shell see that canonical form. A name listed more than once, in any spelling, keeps only its first position. Segment names not on the built-in list must still pass the naming check (a plugin can register additional segment names). |
| `transient_prompt` | bool | `true` | — | After Enter, collapses the prompt to the clock's time (when `clock` is enabled), the prompt symbol and the command, e.g. ` 12:46 AM ❯ nvim mix.exs`. Root shows `root_prompt_icon`. Fish 4.2+ and Zsh only; no effect in Bash or older Fish (Bash keeps the full prompt). With `false`, Fish 4.2+ and Zsh redraw the full prompt at Enter, so the clock still shows when Enter was pressed. |
| `retire_unmodified_themes` | bool | `true` | — | Read by `gpy setup` only. A file under `~/.config/gpy/themes/` named after a builtin theme (`default`, `text`, `starship`) replaces that builtin entirely, so a copy an older GPY or installer left there stops receiving fixes. When the file is byte-identical to a version of that builtin released up to 0.7.1, setup moves it to `<name>.toml.bak` and reports it; the builtin then applies. A file with any edit, even whitespace, and a symlink are left alone. Set `false` to keep every copy. |

Delimiters (prompt/segment open and close icons and colors) are not a `config.toml` setting — they're defined per-theme. See [Advanced Configuration](advanced-configuration.md) and `docs/dev/create-theme.md` to customize them.

### `[ui.directory]`

Controls how the directory segment renders the current path.

| Key | Type | Default | Range | Effect |
|---|---|---|---|---|
| `display` | string | `"basename"` | `"basename"`, `"abbreviated"`, `"truncated"`, `"full"` | `"basename"` shows only the current directory name; `"abbreviated"` shortens intermediate components to their first character (hidden directories keep the leading dot plus one character, e.g. `~/.c/fish`); `"truncated"` keeps the last `truncation_length` components; `"full"` shows the whole path with your home directory shown as `~` (e.g. `~/work/project`). |
| `truncation_length` | integer | `3` | `1`–`255` | Trailing path components kept when `display = "truncated"`. Applies only to `"truncated"`. |
| `truncation_symbol` | string | `""` | no control characters | Prefix shown before a truncated path (e.g. `"…/"`). Applies only to `"truncated"`. |
| `max_length` | integer | `80` | `1`–`1000` | Character cap applied to the rendered directory string after the display mode is resolved — even a `"full"` path can be shortened if it exceeds this. Truncation prefixes a one-character `…` before the kept tail; at `max_length = 1` the ellipsis is dropped and the last character is shown. Wide characters (CJK, emoji) take two columns. |
| `truncate_to_repo` | bool | `false` | — | Anchors the displayed path at the enclosing git repo root (the repo folder becomes the leading component). Affects `"truncated"`/`"full"` only, applied before `max_length`. No effect outside a git repo, and a repo rooted at `$HOME` is ignored (paths show as `~/…`). |

```toml
[ui]
show_icons = true
theme = "default"
palette = "default"
enabled_segments = ["clock", "duration", "language", "directory", "git"]
transient_prompt = true
retire_unmodified_themes = true

[ui.directory]
display = "basename"
truncation_length = 3
truncation_symbol = ""
max_length = 80
truncate_to_repo = false
```

---

## `[plugin.<segment>]`

Overrides for one script plugin segment (`entry_type = "script"`, see [Plugin Framework](../dev/plugins.md#script-segments)). `<segment>` is the segment name as enabled in `ui.enabled_segments`. Settings live here rather than in the theme so they survive a theme switch.

| Key | Type | Default | Range | Effect |
|---|---|---|---|---|
| `ttl_secs` | integer | the segment manifest's `ttl_secs` (5 unless the author set another) | `0`–`31536000` | Seconds a script result is reused. `0` runs the script on every prompt that needs it. |
| `timeout_ms` | integer | the manifest's `timeout_ms` (at most 500) | `1` up to the manifest's value | Lowers the script's time limit. A value above the manifest's is rejected. |
| any setting the manifest declares | bool, integer or string, as declared | the declared default | the declared `min`/`max` or `choices` | Passed to the script as `GPY_OPT_<key>`. |

Each table is typed when the config loads. Whether a key is declared, and whether its value is allowed, depends on the installed plugin, so `gpy config set` checks it against the segment manifest and every error names the key (`plugin.k8s.style: "loud" is not one of bold, dim`). Run `gpy config list plugin.` to see the keys your installed script segments accept.

```toml
[plugin.k8s]
ttl_secs = 30
style = "dim"
```

```bash
gpy config set plugin.k8s.ttl_secs 30
gpy config get plugin.k8s.style   # the override, or the declared default
```

### Bundled `hostname` and `username`

`hostname` and `username` are bundled script plugins (installed by `gpy setup` into `$XDG_DATA_HOME/gpy/plugins`, see [Bundled Plugins](../dev/plugins.md#bundled-plugins)), so their settings live here and no longer in the theme. These replace the theme keys `show_always` and `trim_at` (hostname) and `show_always` (username) of earlier versions; move them when upgrading, since the theme no longer reads them. `gpy theme migrate <theme>` does this for you (see [CLI reference](cli-reference.md#gpy-theme-migrate-name)).

| Key | Type | Default | Effect |
|---|---|---|---|
| `plugin.hostname.show_always` | bool | `false` | Show the host name in every session, not only over SSH. |
| `plugin.hostname.trim_at` | string | `"."` | Cut the host name at the first occurrence of this text; empty keeps it whole. |
| `plugin.username.show_always` | bool | `false` | Show the user name in every session, not only as root, under sudo or over SSH. |

```toml
[plugin.hostname]
show_always = true
trim_at = ""

[plugin.username]
show_always = true
```

The theme's `format` and `user_format` keys for these segments are gone. Style them with `[segments.hostname]` / `[segments.username]` (`bg_color`, `text_color`, `icon`) and the per-variant keys `<variant>_bg_color`, `<variant>_text_color` and `<variant>_icon`. `hostname` prints the variant `ssh` over SSH and `username` prints `root` as root, so `ssh_icon = "🌐 "` shows an icon only over SSH and `root_bg_color` colors a root session. `<variant>_icon` replaces the segment `icon` while that variant is shown; give it its own trailing space if you want a gap.

---

## Applying Changes

`config.toml` is hot-reloaded by the running agent — no restart needed for most edits. Every reload re-resolves which config file is in effect (the highest-priority one that exists), so the agent also follows a config file that you fix after it failed to load at startup, create at a higher-priority location, or delete — deleting the active `config.toml` returns the agent to the built-in defaults. A file that exists but is invalid keeps the last good configuration.

Every reload also re-reads the active palette file, so palette edits apply without a restart. A palette file that cannot be read, does not parse, or fails validation (for example a color reference cycle) keeps the last good palette and logs a warning; switching `ui.palette` to such a palette rejects the reload and keeps the previous configuration. When no earlier palette exists (at startup, or in `gpy-agent oneshot`), an unusable palette falls back to the builtin `default` palette, so palette colours keep rendering.

The active theme name is re-resolved the same way, in user > plugin > built-in order: creating `~/.config/gpy/themes/<name>.toml` for the active theme replaces a plugin or built-in theme of that name, and deleting it falls back again, without a restart. The agent watches your user themes directory for this, and a config reload applies it at once.

For an overlay (a theme file with `extends`), the agent watches the overlay and every theme file it extends, including a plugin theme it is based on, so editing any of them reloads the theme; a built-in base changes only with a new GPY version. A plugin installed later that provides an overlay's base takes over the base at the next theme reload, `gpy theme use`, or agent restart; with `GPY_THEME_WATCH_POLL_MS` set (see [Troubleshooting](troubleshooting.md)) the poll picks it up on its next tick.

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

Commands that change the config (`config set`, `theme use`, `palette use`, `enable`/`disable`, `theme import --apply-layout`, the wizard) edit `config.toml` in place: they change only the keys they set, and keep your comments, key order, unknown keys, and unset defaults. A symlinked `config.toml` stays a symlink and its target is updated.

See [CLI Reference](cli-reference.md) for full command details.

---

## See Also

- [Advanced Configuration](advanced-configuration.md) — theme files, environment variables, performance tuning, and troubleshooting.
- [CLI Reference](cli-reference.md) — `gpy config show/list/get/set/open/wizard` and every other `gpy` subcommand.

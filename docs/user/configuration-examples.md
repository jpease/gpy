<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Configuration Examples

A task-oriented cookbook: pick the goal you want, copy the snippet, adapt the values. For what each key means, its type, default, and valid range, see [Configuration Reference](configuration-reference.md).

**Last Updated:** 2026-07-07

---

## Goal: Switch to Nerd Font icons

`git.icon_set` controls the glyph set used for the stash, detached-HEAD, and in-progress indicators only — the four core git status icons (staged/unstaged/untracked/conflicts) are unaffected and stay Unicode either way:

```toml
# ~/.config/gpy/config.toml
[git]
icon_set = "nerd_font"
```

This requires a [patched Nerd Font](https://www.nerdfonts.com/) installed and selected in your terminal — without one, the glyphs render as boxes or blanks. The built-in `default` theme already ships some Nerd Font glyphs out of the box (e.g. `[segments.duration] icon = "󰑧"` in `config/themes/default.toml`); if your terminal font isn't patched, use `gpy theme use text` instead, which sticks to plain ASCII/Unicode throughout. To swap individual language icons for Nerd Font codepoints (instead of the default emoji), override `[language.icons]`:

```toml
[language.icons]
rust = ""
python = ""
```

---

## Goal: Apply a color palette to a theme

`config/themes/default.toml` (and the other built-in themes) reference colors by bare name — `bg_color = "black"`, `dirty_bg_color = "red"`, `clean_bg_color = "green"`, and so on. `config/palettes/nord.toml` redefines those same names to Nord's hex values (`black = "#2e3440"`, `red = "#bf616a"`, `green = "#a3be8c"`, ...). Selecting the Nord palette remaps every matching name the theme already uses, with no theme-file edits required:

```toml
# ~/.config/gpy/config.toml
[ui]
theme = "default"
palette = "nord"
```

This works for any built-in palette (`gpy palette list` shows `default`, `starship`, `nord`, `gruvbox-dark-medium`, and the four `catppuccin-*` variants) paired with any theme whose color fields use bare names rather than literal `#RRGGBB` hex — a hex value in a theme file always wins as-is, regardless of the active palette.

---

## Goal: Reorder or trim prompt segments

`ui.enabled_segments` is an ordered array — segments render left to right in the order listed, and anything you omit simply doesn't render:

```toml
# ~/.config/gpy/config.toml

# Minimal prompt: just where you are and git state
[ui]
enabled_segments = ["directory", "git"]
```

```toml
# Verbose dev prompt: clock first, then language/dir/git, duration last
[ui]
enabled_segments = ["clock", "language", "directory", "git", "duration"]
```

---

## Goal: Tune git detection for a large monorepo

Combine a few `[git]` keys to keep the prompt responsive in a huge or partially-vendored tree:

```toml
# ~/.config/gpy/config.toml
[git]
skip_paths = ["~/work/monorepo/vendor", "/mnt/network-share"]
max_ahead_behind = 20
stash_enabled = false
timeout_seconds = 3
```

`skip_paths` disables git detection entirely under those trees. `max_ahead_behind` clamps the ahead/behind counter (rendered with a trailing `+` past the cap) instead of walking the full divergent history. `stash_enabled = false` skips the extra `git stash list` subprocess call. `timeout_seconds` fails fast on a slow repo instead of stalling the prompt.

---

## Goal: Tune language detection

Two common variants, both starting from an allowlist so only languages you care about are ever considered:

```toml
# ~/.config/gpy/config.toml

# Clean single-language prompt: show only the dominant language
[language]
enabled_languages = ["rust", "python", "node"]
filter = "primary"
```

```toml
# Mixed-language repo with marker noise: only show a language when its
# project marker files are present, and raise the bar for what counts
[language]
enabled_languages = ["rust", "python", "node"]
detection_mode = "markers"
confidence_threshold = 0.3
```

`filter = "primary"` collapses detection down to the single most-confident language instead of listing every one detected. `detection_mode = "markers"` swaps content-scanning (which can pick up stray files in a polyglot repo) for Starship-style marker-file detection. `confidence_threshold` raises the bar above the default `0.1` to hide low-confidence noise either mode can still produce.

For prevalence ranking *and* noise control together, `detection_mode = "hybrid"` runs the content scan but keeps only languages whose project marker file is also present — a Rust repo with a stray `.py` helper script still ranks languages by how much of the repo they make up, but drops `python` since there's no `pyproject.toml`/`setup.py`/etc. backing it.

---

## Goal: Recolor individual languages in the language segment

Per-language color overrides live in the theme file, not `config.toml` — any canonical language identifier works as `<lang>_bg_color` / `<lang>_text_color` under `[segments.language]`, matched case-insensitively (including aliases like `node`/`nodejs`/`javascript`/`js`, which all resolve to the `node` canonical name):

```toml
# ~/.config/gpy/themes/mytheme.toml
[segments.language]
bg_color = "black"     # fallback for any language without its own override
text_color = "white"

rust_bg_color = "red"
python_bg_color = "blue"
swift_bg_color = "orange"
```

Confirmed canonical language identifiers include `rust`, `python`, `node`, `go`, `java`, `ruby`, `swift`, `elixir`, `php`, `csharp`, `cpp`, `c`, `erlang`, and `fish`.

---

## Goal: Build a powerline-style prompt

The built-in `default` theme already renders as a powerline chain of colored blocks joined by triangle glyphs — each segment's `format` string draws its own leading/trailing separator via `$sep_gap`/`$sep_close`, and `[ui.segment_open]`/`[ui.segment_close]` set the glyph used between segments generally. Reuse the same pattern in a custom theme with your own glyph and colors:

```toml
# ~/.config/gpy/themes/mytheme.toml
[ui.prompt_open]
icon = ""
icon_color = "transparent"
bg_color = "transparent"

[ui.segment_open]
icon = ""
icon_color = "match_bg"
bg_color = "transparent"

[ui.segment_close]
icon = ""
icon_color = "match_bg"
bg_color = "transparent"

[segments.directory]
format = "[](fg:$bg bg:default)[ $path]($style)([$sep_gap]($style))([$sep_close](fg:$bg bg:default))"
bg_color = "blue"
text_color = "black"
```

`icon_color = "match_bg"` paints the separator glyph the same color as the segment's own background, so consecutive segments read as one continuous chain rather than boxed-in blocks. `transparent` (also valid for `icon_color`/`bg_color`) means "no color / terminal default." Validate with `gpy theme validate mytheme` before activating.

---

## See Also

- [Configuration Reference](configuration-reference.md) — every `config.toml` key, type, default, and valid range.
- [Advanced Configuration](advanced-configuration.md) — environment variables, performance tuning, security settings, and troubleshooting.
- [Create a Theme](../dev/create-theme.md) — the full theme-file schema, including the git segment's 6-bucket color-state model.
- The theme-customization guide, once it exists.

<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Theme Customization

How to change the way your GPY prompt looks, from the zero-effort wizard up to hand-editing a theme file.

**Last Updated:** 2026-07-07

---

## Overview

GPY separates "look" into two layers:

- **Theme** — structure: which segments render, their icons, separators, and format strings (`~/.config/gpy/themes/<name>.toml`).
- **Palette** — color: a named-color remap applied on top of a theme (`~/.config/gpy/palettes/<name>.toml`).

This guide is ordered by effort, cheapest first. Stop as soon as you get the look you want:

1. [The wizard](#1-easiest-the-interactive-wizard) — no file editing at all.
2. [Quick swaps via CLI](#2-quick-swaps-via-cli) — switch built-in themes/palettes.
3. [Palettes as the color layer](#3-palettes-as-the-color-layer) — understand and import palettes.
4. [Light hand-editing](#4-light-hand-editing) — fork a theme, tweak a few fields.
5. [Going further](#5-going-further) — the full schema and copy-paste recipes.

---

## 1. Easiest: the interactive wizard

```bash
gpy config wizard
```

A full-screen TUI with three pickers — Theme, Palette, Segments — plus a live preview of the resulting prompt that updates as you move the cursor. Nothing is written to disk until you save.

**Keys** (verified against `gpy-agent/src/commands/wizard/keys.rs`):

| Key | Action |
|---|---|
| `Tab` | Move to the next section (Theme → Palette → Segments → Theme …) |
| `Shift+Tab` | Move to the previous section |
| `Up` / `Down` | Move the cursor within the focused section |
| `Space` / `Enter` | Toggle or select the highlighted item |
| `s` | Save the current selection and exit |
| `q` / `Esc` | Quit — prompts to confirm discarding changes if anything is unsaved |

> `docs/user/cli-reference.md`'s prose for `gpy config wizard` only mentions `Tab`, `Space`/`Enter`, `s`, and `q` — it omits `Shift+Tab` and `Up`/`Down`. Both exist and work; this table reflects the actual key handler.

This is the fastest way to try combinations — pick a theme, cycle palettes over it, toggle segments on/off, and watch the preview — before touching any TOML.

---

## 2. Quick swaps via CLI

If you already know which built-in theme or palette you want, skip the wizard and set it directly. Both commands update `config.toml` and reload the running agent.

```bash
# Themes
gpy theme list             # see what's installed, active one marked with *
gpy theme use starship     # switch structure/layout
gpy theme show             # print the active theme name

# Palettes
gpy palette list            # see what's installed, active one marked with *
gpy palette use nord        # swap colors only, theme structure unchanged
gpy palette show             # print the active palette name
```

Only three themes ship in the box: `default`, `starship`, `text`. `text` is a plain-ASCII/Unicode fallback for terminals without a patched font — see [Configuration Examples: Switch to Nerd Font icons](configuration-examples.md#goal-switch-to-nerd-font-icons).

`gpy theme use <name>` also accepts `--force`: it applies that theme's *recommended* settings (segment layout, palette, language detection mode) on top of your current config, but only for values you haven't already customized by hand. Without `--force`, switching themes leaves everything else as-is.

Before writing changes, `gpy theme use` validates the candidate theme's segment format templates against the prospective active palette. If any format template fails validation (unsupported syntax or an unknown color role not present in the prospective palette), activation fails and your current `config.toml` is preserved untouched.

```bash
gpy theme use starship --force   # adopt starship's recommended layout + palette too
```

---

## 3. Palettes as the color layer

### What a palette actually does

A theme file's color fields (`bg_color`, `text_color`, `clean_bg_color`, and so on) accept three kinds of value:

- a **bare name** (`"black"`, `"red"`, `"green"`) — looked up by name in the *active palette*
- an ANSI index (`0`–`255`)
- a literal **hex** value (`"#RRGGBB"`)

The built-in `default` theme uses bare names throughout. Selecting a different palette remaps every name the theme already uses — no theme-file edits required. A hex value in a theme always wins as a literal; palettes can't override it. This is the same mechanism documented in [Configuration Examples: Apply a color palette to a theme](configuration-examples.md#goal-apply-a-color-palette-to-a-theme) — see that recipe for the exact `config.toml` snippet.

### Built-in palettes

```bash
gpy palette list
```

Ships with: `default`, `starship`, `nord`, `gruvbox-dark-medium`, `catppuccin-latte`, `catppuccin-mocha`, `catppuccin-macchiato`, `catppuccin-frappe`.

### Validating a palette

```bash
gpy palette validate               # validate the active palette
gpy palette validate nord          # validate a named palette
gpy palette validate ~/.config/gpy/palettes/mine.toml   # validate a file directly
```

### Importing a base16/base24 scheme

If you already have a color scheme from the [tinted-theming](https://github.com/tinted-theming) (base16/base24) ecosystem, import it directly as a GPY palette instead of retyping colors by hand:

```bash
gpy palette import <scheme.yaml> [--name <name>] [--force]
```

Verified against `gpy-agent/src/commands/palette.rs` and `gpy-agent/src/import/base16/`:

- `<scheme.yaml>` — a base16 or base24 scheme file. GPY's parser accepts both the modern nested layout (`system:`, `name:`, `variant:`, then a `palette:` block of `baseNN: "hex"` entries) and the legacy flat layout (`scheme:`/`name:` plus top-level `baseNN:` keys). `base00`–`base0F` cover base16; base24 adds `base10`–`base17`.
- `--name <name>` — overrides the palette name. If omitted, GPY uses the scheme's declared `name`/`scheme` field, falling back to `imported` if the scheme declares none.
- `--force` — overwrite an existing palette of the same name; without it, the import errors if the destination already exists.

The imported palette is written to `~/.config/gpy/palettes/<name>.toml`. Activate it the same way as any built-in palette:

```bash
gpy palette import ~/Downloads/gruvbox-dark-hard.yaml --name gruvbox-hard
gpy palette use gruvbox-hard
```

If the derived name would be unsafe (e.g. contains path separators), the import fails with an error rather than writing outside the palettes directory.

---

## 4. Light hand-editing

Once you want something a built-in theme/palette combination can't give you — one different icon, one segment recolored — fork the default theme instead of authoring one from scratch.

```bash
gpy theme new mytheme
```

Already like a specific theme (built-in, user, or plugin) and just want to tweak it? Clone it instead of starting from the default:

```bash
gpy theme new mytheme --from starship
```

Theme names must be valid configuration names (non-empty, non-whitespace, containing no `/`, `\`, `..`, or control characters). This copies the chosen theme (the default, or the one named by `--from`) to `~/.config/gpy/themes/mytheme.toml` and prints the path to edit plus the activation command. Open it in your editor:

```bash
$EDITOR ~/.config/gpy/themes/mytheme.toml
```

### Common small edits

**Change a segment's icon:**

```toml
# before
[segments.duration]
icon = "󰑧"

# after
[segments.duration]
icon = "⏱"
```

**Recolor a segment's background:**

```toml
# before
[segments.directory]
bg_color = "cyan"
text_color = "black"

# after
[segments.directory]
bg_color = "#1d3557"
text_color = "#f1faee"
```

**Toggle icon display globally:**

```bash
gpy config set ui.show_icons false
```

`show_icons` lives in `config.toml`, not the theme file — it's a display toggle that applies across whichever theme is active, so you don't need to touch the theme file for it.

> A related trap: `config.toml` has no `ui.delimiter_style`/`ui.delimiters` keys (a dead, never-rendered config surface was removed in #377). Segment separators are theme-only — edit the theme file's `[ui.segment_open]` / `[ui.segment_close]` tables or a segment's own `format` string instead — see [Configuration Examples: Build a powerline-style prompt](configuration-examples.md#goal-build-a-powerline-style-prompt) for a worked example.

After editing, validate before activating:

```bash
gpy theme validate mytheme
gpy theme use mytheme
```
`gpy theme validate` checks theme schema fields, icon glyphs, and segment format templates across all eight supported agent-rendered segments (`git`, `language`, `directory`, `duration`, `character`, `clock`, `hostname`, `username`), reporting the failing field, the invalid value, and remediation guidance if something's wrong (bad color name, empty icon glyph, malformed TOML). Both `theme validate` and `theme use` share this validator.
### Already using Starship?

If you're migrating from [Starship](https://starship.rs/) and want to start from your existing look rather than the GPY default theme, `gpy theme import` converts a `starship.toml` into a matching GPY palette + theme pair in one step:

```bash
gpy theme import ~/.config/starship.toml --name my-prompt --apply-layout
gpy palette use my-prompt
gpy theme use my-prompt
```

See [Migrating from Starship](migrating-from-starship.md) for the full walkthrough — config layout differences, common pitfalls, and uninstalling Starship afterward — or [Starship Importer](../dev/starship-import.md) for the flag list, module-by-module mapping table, and known lossy conversions.

---

## 5. Going further

Once you're past small tweaks — designing a theme from scratch, styling every git state bucket, or theming a plugin segment — the light-editing workflow above isn't the right reference anymore. Two docs cover that ground:

- **[Create a Theme](../dev/create-theme.md)** — the complete theme-file schema: every `[ui]` field, the full `[segments.<id>]` table, the git segment's 6-bucket color-state model (`Conflicts > InProgress > Modified > UntrackedOnly > AheadBehind > Clean`), per-state `git_style` attributes, and how plugin segments are styled the same way as built-in ones.
- **[Configuration Examples](configuration-examples.md)** — copy-paste recipes: a full powerline-style prompt, per-language segment colors, Nerd Font icon swaps, and more.

For what every `config.toml` key does (as opposed to theme-file fields), see [Configuration Reference](configuration-reference.md).

---

## See Also

- [Migrating from Starship](migrating-from-starship.md) — switching from Starship: what carries over, config layout differences, and common pitfalls.
- [Configuration Reference](configuration-reference.md) — every `config.toml` key, type, default, and range.
- [Configuration Examples](configuration-examples.md) — task-oriented cookbook of copy-paste config/theme snippets.
- [CLI Reference](cli-reference.md) — complete `gpy` command reference, including `gpy theme`/`gpy palette` subcommands and `gpy config wizard`.
- [Create a Theme](../dev/create-theme.md) — the full theme-file schema for authoring a theme from scratch.

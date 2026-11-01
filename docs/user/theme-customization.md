<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Theme Customization

How to change the way your GPY prompt looks, from the zero-effort wizard up to hand-editing a theme file.

---

## Terms

- **Theme** — prompt structure: which segments render, their icons, separators, and format templates (`~/.config/gpy/themes/<name>.toml`). In `config.toml`, `ui.theme` is the theme file stem.
- **Palette** — the named colours a theme uses (`~/.config/gpy/palettes/<name>.toml`). In `config.toml`, `ui.palette` is the palette file stem. Swapping palettes recolours the prompt without changing its layout.
- **Segment** — a single piece of prompt information. Built-ins (`clock`, `directory`, `duration`, `git`, `language`, `status`) are rendered by the agent. Bundled script plugins (`hostname`, `username`) and user plugins are managed by the plugin framework. The prompt character is styled by `[segments.character]`.
- **Overlay** — a theme file that starts with `extends = "<base>"` and overrides only the keys you want to change, inheriting everything else from the base theme.
- **Starship** — names three separate things in GPY: the `starship` built-in theme (`config/themes/starship.toml`), the `starship` built-in palette (`config/palettes/starship.toml`), and the Starship importer (`gpy theme import`) that converts a Starship configuration file into a GPY theme and palette pair.

---

## Overview

Theme precedence is user (`~/.config/gpy/themes/`) > plugin > builtin (`default`, `text`, `starship`).

- **[Recommended: an overlay](#recommended-an-overlay)** — run the wizard or `gpy theme new <name>`, which write an overlay, and edit that overlay:
  - [1. Easiest: the interactive wizard](#1-easiest-the-interactive-wizard) — no file editing at all.
  - [4. Light hand-editing](#4-light-hand-editing) — extend an existing theme, tweak a few fields.
- **[Other ways](#other-ways)** — alternatives for specific needs:
  - [3. Palettes as the color layer](#3-palettes-as-the-color-layer) — for colour only.
  - [2. Quick swaps via CLI](#2-quick-swaps-via-cli) — `theme use --force` for layout.
  - [Already using Starship?](#already-using-starship) — `theme import` for Starship users.
  - [Your copy of a builtin theme after an upgrade](#your-copy-of-a-builtin-theme-after-an-upgrade) — `--full` copies for a frozen theme.
- **[5. Going further](#5-going-further)** — the full schema and copy-paste recipes.

---

## Recommended: an overlay

The recommended path for customising a prompt is to run the interactive wizard or `gpy theme new <name>`, which write an overlay file (`~/.config/gpy/themes/<name>.toml` extending a base theme), and edit that overlay. Overlays keep only your overrides in the file while inheriting fixes and additions from the base theme when GPY upgrades.

### 1. Easiest: the interactive wizard

```bash
gpy config wizard
```

A full-screen TUI with three pickers — Theme, Palette, Segments — plus a live preview of the resulting prompt that updates as you move the cursor. Nothing is written to disk until you save.

`Tab` moves between the three pickers, the arrow keys move the cursor, `Space`/`Enter` selects or toggles, `s` saves and `q` quits. Highlighting the `clock`, `duration`, `language`, `directory` or `git` segment enables keys for its options, and `?` lists every key that applies. The full key map is in the [CLI Reference](cli-reference.md#gpy-config-wizard).

This is the fastest way to try combinations — pick a theme, cycle palettes over it, toggle segments on/off, and watch the preview — before touching any TOML.

---

### 4. Light hand-editing

Once you want something a built-in theme/palette combination can't give you — one different icon, one segment recolored — start a theme of your own that builds on an existing one instead of authoring one from scratch.

```bash
gpy theme new mytheme
```

Already like a specific theme (built-in, user, or plugin) and just want to tweak it? Build on it instead of the default:

```bash
gpy theme new mytheme --from starship
```

Theme names must be valid configuration names (non-empty, non-whitespace, containing no `/`, `\`, `..`, or control characters). This writes `~/.config/gpy/themes/mytheme.toml` as an **overlay** of the chosen theme (the default, or the one named by `--from`) and prints the path to edit, the theme it extends, and the activation command. The new file holds a single line:

```toml
extends = "starship"
```

Add only the keys you want to change; everything else comes from the base, including later fixes to it when you upgrade GPY. Tables merge key by key, so `[segments.clock]` with one `time_format` line keeps every other clock key of the base. To remove a key of the base, list it in `unset` (`unset = ["segments.character.format"]`). If the base is a plugin theme, the output names the plugin: removing or disabling it leaves your theme unable to load.

To customize a theme under its own name, `gpy theme save` with that theme active writes a self-named overlay (`starship.toml` with `extends = "starship"`), which still receives the builtin's updates; `gpy config wizard` does the same for the options it edits. (For frozen copies that do not inherit from a base, see [Frozen copies (`--full`)](#your-copy-of-a-builtin-theme-after-an-upgrade) under Other ways.)

Open the file in your editor:

```bash
$EDITOR ~/.config/gpy/themes/mytheme.toml
```

#### Common small edits

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

**Change the prompt symbol:**

The prompt symbol is `[segments.character]`: `success_symbol` and `success_color` (and `error_symbol`/`error_color` after a failed command). Shells that draw the symbol themselves, for example while the agent is down, use the same two values. `ui.root_prompt_icon` and `ui.root_prompt_color` are the separate, root-only symbol. `ui.prompt_icon` and `ui.prompt_color` are deprecated: they only override the shell-drawn fallback, so set the `segments.character` keys instead.

```toml
[segments.character]
success_symbol = "λ"
success_color = "cyan"
```

**Change the read-only marker:**

A directory format's `$read_only` shows when the working directory is not writable. It is the theme's `read_only_icon`, or 🔒 when unset; with `ui.show_icons = false` it is `ro`, and a Nerd Font `read_only_icon` gives way to `ro` too.

```toml
[segments.directory]
format = "[$path]($style)[ $read_only](red)"
read_only_icon = "RO"
```

**Toggle icon display globally:**

```bash
gpy config set ui.show_icons false
```

`show_icons` lives in `config.toml`, not the theme file. It applies to whichever theme is active. A theme's icons (`[segments.git]` `*_icon`, `[segments.language]` `<lang>_symbol`) win over `[git.icons]` and `[language.icons]` in `config.toml`; to change one, use a theme overlay (`gpy theme new`).

**Pill formats for `status`, and `hostname` / `username` styling:**

Like `clock`, `directory`, `duration`, `git` and `language`, `status` is drawn by the agent from a `format` string in the theme, so a themed pill reads the same in every shell. `hostname` and `username` are bundled script plugins instead: they have no `format` (or `user_format`) key, and the plugin framework draws their pills. Their `show_always` and `trim_at` settings live in `config.toml` under `[plugin.hostname]` / `[plugin.username]` (see the [Configuration Reference](configuration-reference.md#bundled-hostname-and-username)). In the theme you style them with `bg_color`, `text_color`, `icon`, and per-variant keys `<variant>_bg_color`, `<variant>_text_color` and `<variant>_icon`: `hostname` prints the variant `ssh` over SSH (`ssh_icon`), `username` prints `root` as root (`root_bg_color`, `root_text_color`, `root_icon`). The built-in `default` theme gives `status` a single leading space inside the pill:

```toml
[segments.status]
format = "([$sep_open](fg:$bg bg:default))[ $symbol]($style)([$sep_gap]($style))([$sep_close](fg:$bg bg:default))"
```

`$symbol` is the segment's icon (for `status`, its `ok_icon`/`fail_icon`, or `✔`/`✖` when unset). It is written as-is, so give an icon its own trailing space (`icon = "🌐 "`) and a missing icon leaves no gap; the same holds for a `<variant>_icon` such as `ssh_icon`. `$style` and `$bg` come from the segment's `text_color` and `bg_color` (for `status`, from `ok_*` or `fail_*` according to the previous command's exit status), and `$sep_open` is left out on the first pill in the prompt. A theme without a `format` for `status` still renders: the shell draws a plain pill, with the same single leading space. The shell draws that pill too whenever the agent gives no reply (down, disabled, or an older agent answering with an error), so the pill never disappears. The plain pill always puts a space between the icon and the text, so an icon without a trailing space looks different there.

> A related trap: `config.toml` has no `ui.delimiter_style`/`ui.delimiters` keys. Segment separators are theme-only — edit the theme file's `[ui.segment_open]` / `[ui.segment_close]` tables or a segment's own `format` string instead — see [Configuration Examples: Build a powerline-style prompt](configuration-examples.md#goal-build-a-powerline-style-prompt) for a worked example.

After editing, validate before activating:

```bash
gpy theme validate mytheme
gpy theme use mytheme
```
`gpy theme validate` checks theme schema fields, icon glyphs, and segment format templates across all nine supported agent-rendered segments (`git`, `language`, `directory`, `duration`, `character`, `clock`, `hostname`, `username`, `status`), reporting the failing field, the invalid value, and remediation guidance if something's wrong (bad color name, empty icon glyph, malformed TOML). It also checks the free-form style strings (`<lang>_style` overrides and `[segments.git.git_style]` values), naming the offending field (e.g. `segments.language.rust_style`) when one cannot be parsed or uses a color the palette cannot resolve. Format templates and style strings may not contain terminal control characters (ESC, BEL, carriage return, …); newline is the only control character a format may carry, and the agent strips any other control character from template text at render time. Both `theme validate` and `theme use` share this validator.

`gpy theme validate` and `gpy doctor` also print warnings for a theme that loads but has parts that do nothing; they never fail validation:

- a key the theme sets that GPY does not read (`unknown key `segments.clock.time_fromat` (ignored)`);
- a `[segments.<id>]` section that no built-in segment, bundled plugin (`username`, `hostname`) or installed plugin provides (usually a typo such as `[segments.gitt]`);
- a `[segments.language]` colour key that is not `<lang>_bg_color` or `<lang>_text_color` (`rust_bgcolor` is ignored);
- an enabled `directory`, `duration`, `git` or `language` segment with no `format`, which renders nothing (for example after `unset = ["segments.git.format"]`);
- a control character in a git icon override (`staged_icon`, `unstaged_icon`, `untracked_icon`, `conflicts_icon`, `stash_icon`), which is removed when the segment is drawn;
- a theme that sets the deprecated `ui.prompt_icon` or `ui.prompt_color` (set `segments.character.success_symbol` or `success_color` instead).

`gpy palette use` checks the active theme's segment templates against the new palette and warns about each segment that would render empty, then switches anyway.

---

## Other ways

Other ways to customize the prompt, in order of when to reach for them:

1. **Palettes for colour only**: use a palette when you want to change colours without altering prompt structure or segment templates.
2. **`theme use --force` for layout**: use `theme use --force` when you want to adopt a built-in theme's recommended segment layout, directory display, and palette in one step.
3. **`theme import` for Starship users**: use `theme import` when migrating from Starship to convert an existing `starship.toml` into a matching GPY theme and palette.
4. **`--full` copies for a frozen theme**: use a full copy (`--full`) when you want a completely independent, frozen theme that will not receive upstream updates when GPY is upgraded.

### 3. Palettes as the color layer

Use a palette when you want to change colours without altering prompt structure or segment templates.

#### What a palette actually does

A theme file's color fields (`bg_color`, `text_color`, `clean_bg_color`, and so on) accept the same colors as a format template's style string, plus a few theme-only keywords:

- a **bare name** (`"black"`, `"red"`, `"green"`) — looked up by name in the *active palette*. Every ANSI spelling a style string accepts works here: `magenta`/`purple`, bright variants as `bright_cyan`, `bright-cyan` or `brcyan`, and `gray`/`grey` for bright black. The palette role names `orange` and `brown` are accepted too.
- an ANSI index (`0`–`255`)
- a literal **hex** value (`"#RRGGBB"`, exactly one `#`)
- `transparent` (or `default`) for the terminal default color, and `match_bg`/`match_text` to borrow the neighbouring segment's background or text color in delimiter colors

GPY stores each color in one canonical spelling (for example `bright-magenta` becomes `brmagenta`), so every shell and the agent read the same value; `gpy theme validate` rejects anything else, such as `##112233`.

The built-in `default` theme uses bare names throughout. Selecting a different palette remaps every name the theme already uses, with no theme-file edits. A hex value in a theme always wins as a literal; palettes can't override it. See [Configuration Examples: Apply a color palette to a theme](configuration-examples.md#goal-apply-a-color-palette-to-a-theme) for the `config.toml` snippet.

#### Built-in palettes

```bash
gpy palette list
```

Ships with: `default`, `starship`, `nord`, `gruvbox-dark-medium`, `catppuccin-latte`, `catppuccin-mocha`, `catppuccin-macchiato`, `catppuccin-frappe`.

For the role vocabulary, palette file format, and the `starship` palette, see [Color Palettes](palettes.md).

#### Validating a palette

```bash
gpy palette validate               # validate the active palette
gpy palette validate nord          # validate a named palette
gpy palette validate ~/.config/gpy/palettes/mine.toml   # validate a file directly
```

#### Importing a base16/base24 scheme

If you already have a color scheme from the [tinted-theming](https://github.com/tinted-theming) (base16/base24) ecosystem, import it directly as a GPY palette instead of retyping colors by hand:

```bash
gpy palette import <scheme.yaml> [--name <name>] [--force]
```

- `<scheme.yaml>` — a base16 or base24 scheme file. GPY's parser accepts both the modern nested layout (`system:`, `name:`, `variant:`, then a `palette:` block of `baseNN: "hex"` entries) and the legacy flat layout (`scheme:`/`name:` plus top-level `baseNN:` keys). `base00`–`base0F` cover base16; base24 adds `base10`–`base17`.
- `--name <name>` — overrides the palette name. If omitted, GPY uses the scheme's `slug` field when present, else its `name`/`scheme` field (falling back to `imported` if the scheme declares none). The result is lowercased and runs of characters that are not letters or digits become a single `-`; non-ASCII letters are kept (`Rosé Pine` without a slug becomes `rosé-pine`).
- `--force` — overwrite an existing palette of the same name; without it, the import errors if the destination already exists.

The imported palette is written to `~/.config/gpy/palettes/<name>.toml`. Activate it the same way as any built-in palette:

```bash
gpy palette import ~/Downloads/gruvbox-dark-hard.yaml --name gruvbox-hard
gpy palette use gruvbox-hard
```

If the derived name would be unsafe (e.g. contains path separators), the import fails with an error rather than writing outside the palettes directory.

---

### 2. Quick swaps via CLI

Use `theme use --force` when you want to adopt a built-in theme's recommended segment layout, directory display, and palette in one step. If you already know which built-in theme or palette you want, skip the wizard and set it directly. Both commands update `config.toml` and reload the running agent.

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

`gpy theme use <name>` also accepts `--force`: it applies every *recommended* setting of that theme (segment layout, directory display, language detection mode, palette) on top of your current config, overwriting your values, and prints each change as `old → new` (for example `ui.palette: nord → starship`). To undo a change, set the old value back with `gpy config set` (or `gpy enable`/`gpy disable` for the segment list). `--force` never changes `ui.show_icons`, which `gpy setup` detects; a theme's `[ui.recommended] show_icons` is deprecated and ignored. Without `--force`, switching themes leaves everything else as-is.

Before writing changes, `gpy theme use` validates the candidate theme's segment format templates against the prospective active palette. If any format template fails validation (unsupported syntax or an unknown color role not present in the prospective palette), activation fails and your current `config.toml` is preserved untouched.

```bash
gpy theme use starship --force   # adopt starship's recommended layout + palette too
```

---

### Already using Starship?

Use `theme import` when migrating from Starship to convert an existing `starship.toml` into a matching GPY theme and palette. If you're migrating from [Starship](https://starship.rs/) and want to start from your existing look rather than the GPY default theme, `gpy theme import` converts a `starship.toml` into a matching GPY palette + theme pair in one step:

```bash
gpy theme import ~/.config/starship.toml --name my-prompt --apply-layout
gpy palette use my-prompt
gpy theme use my-prompt
```

See [Migrating from Starship](migrating-from-starship.md) for the full walkthrough — config layout differences, common pitfalls, and uninstalling Starship afterward — or [Starship Importer](../dev/starship-import.md) for the flag list, module-by-module mapping table, and known lossy conversions.

---

### Your copy of a builtin theme after an upgrade

Use a full copy (`--full`) when you want a completely independent, frozen theme that will not receive upstream updates when GPY is upgraded:

```bash
gpy theme new mytheme --from starship --full
# or to freeze an active theme:
gpy theme save mytheme --full
```

`--full` writes the whole theme: the base's file verbatim when the base is a full theme, or the merged result when it is an overlay.

A **full** file `~/.config/gpy/themes/<name>.toml` (one without `extends`) with the name of a builtin theme (`default`, `text`, `starship`) completely replaces the builtin, so it keeps its old keys while the builtin moves on. Older GPY versions wrote such copies from `gpy config wizard`, `gpy theme new` and `gpy theme save`; today these write overlays (above), which hold only your changes and pick up the builtin's updates. A stale copy can fail to load after an upgrade (the 0.7 release moved `show_always`/`trim_at` and the `format` keys of `username` and `hostname` out of the theme). These commands deal with it, none of which overwrites a copy without a backup:

```bash
gpy theme diff starship     # '-' = only in the builtin, '+' = only in your copy
gpy theme migrate starship  # rewrite keys removed by breaking changes, backing up first
gpy theme rebase starship   # turn a modified copy into an overlay that keeps only your changes
gpy theme reset starship    # or drop the copy: moved to starship.toml.bak
```

`migrate` moves the removed settings into `config.toml` (`[plugin.username]`, `[plugin.hostname]`) without replacing values already there, and checks that the result loads. `rebase` shows the overlay first and writes it only with `--write`. Use `reset` when you have no customizations worth keeping; use `diff` to see what you would lose.

---

## Theme `[ui]` keys

Every key a theme accepts under `[ui]`. These are theme keys, not `config.toml` keys.

| Key | Type | Effect |
|---|---|---|
| `two_line` | bool | Puts the prompt character on its own line below the other segments. |
| `add_newline` | bool | Prints a blank line before each prompt. |
| `root_prompt_icon` | string | Prompt symbol shown when running as root. |
| `root_prompt_color` | color | Color of the root prompt symbol. |
| `prompt_open`, `prompt_close` | glyph table | Delimiters drawn at the start and end of the whole prompt; each takes `fg`/`bg` colors (`[ui.prompt_open]`, `[ui.prompt_close]`). |
| `segment_open`, `segment_close` | glyph table | Default delimiters drawn around each segment, with `fg`/`bg` colors (`[ui.segment_open]`, `[ui.segment_close]`). |
| `prompt_icon` | string | Deprecated; use `[segments.character] success_symbol` / `success_color` instead. |
| `prompt_color` | color | Deprecated; use `[segments.character] success_symbol` / `success_color` instead. |
| `recommended` | table | `[ui.recommended]`: settings the theme suggests (`enabled_segments`, `directory`, `language_detection`, `palette`), applied to `config.toml` only by `gpy theme use <name> --force`. `[ui.recommended] show_icons` is deprecated and ignored; `ui.show_icons` is set by `gpy setup`. |

`default` and `starship` set `two_line`, and every bundled theme sets the root prompt keys: [`config/themes/`](../../config/themes/).

---

## 5. Going further

To design a theme from scratch, style every git state bucket, or theme a plugin segment, use these:

- **[Create a Theme](../dev/create-theme.md)** — the complete theme-file schema: every `[ui]` field, the full `[segments.<id>]` table, the git segment's 6-bucket color-state model (`Conflicts > InProgress > Modified > UntrackedOnly > AheadBehind > Clean`, where `Modified` is the dirty state: `dirty_*` colours and `git_style.modified`), per-state `git_style` attributes, and how plugin segments are styled the same way as built-in ones.
- **[Configuration Examples](configuration-examples.md)** — copy-paste recipes: a full powerline-style prompt, per-language segment colors, Nerd Font icon swaps, and more.

For what every `config.toml` key does (as opposed to theme-file fields), see [Configuration Reference](configuration-reference.md).

---

## See Also

- [Migrating from Starship](migrating-from-starship.md) — switching from Starship: what carries over, config layout differences, and common pitfalls.
- [Configuration Reference](configuration-reference.md) — every `config.toml` key, type, default, and range.
- [Configuration Examples](configuration-examples.md) — task-oriented cookbook of copy-paste config/theme snippets.
- [CLI Reference](cli-reference.md) — complete `gpy` command reference, including `gpy theme`/`gpy palette` subcommands and `gpy config wizard`.
- [Create a Theme](../dev/create-theme.md) — the full theme-file schema for authoring a theme from scratch.

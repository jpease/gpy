# Starship Importer

`gpy theme import <starship.toml>` converts a [Starship](https://starship.rs/) configuration
into a GPY palette and prompt theme so you can adopt GPY without starting from scratch.

## CLI reference

```
gpy theme import <path/to/starship.toml> [--name <name>] [--force] [--stdout] [--apply-layout]
```

| Flag | Description |
|---|---|
| `<path>` | Required. Path to the source `starship.toml`. |
| `--name <name>` | Base name for the emitted artifacts. Defaults to the file stem (e.g. `starship` from `starship.toml`), then `starship-import` if the stem is empty. |
| `--force` | Overwrite existing files of the same name, or intentionally shadow a builtin/plugin theme or palette of that name. |
| `--stdout` | Print both artifacts to stdout (with `# ---- palette ----` / `# ---- theme ----` headers) instead of writing files. Useful for inspection. |
| `--apply-layout` | Also write the derived `enabled_segments` order into the active `config.toml`. Default: print the suggested order only. |

### Output (normal mode)

Two files are written to the user config directories:

```
~/.config/gpy/palettes/<name>.toml
~/.config/gpy/themes/<name>.toml
```

After writing, the command prints activation hints:

```
gpy palette use <name>
gpy theme use <name>
# recommended segment order (paste into config.toml, or re-run with --apply-layout):
enabled_segments = ["directory", "git", "language", "duration", "character"]
```

## Activation after import

Run both commands, palette first, to apply the imported look (`gpy theme use` validates the theme against the active palette, so a theme that references colors from the imported palette fails if the palette is not active yet):

```bash
gpy palette use <name>
gpy theme use <name>
```

If you also want to adopt the imported segment order, either:

- Paste the printed `enabled_segments` line into `~/.config/gpy/config.toml`, or
- Re-run with `--apply-layout` to write it automatically.

## Module mapping

The importer translates Starship modules to GPY segments. Modules that have no
GPY equivalent are skipped with a warning.

Every segment starts from the builtin `starship` preset
(`config/themes/starship.toml`), which encodes Starship's default rendering.
A module the source configures replaces its segment; a module the source
leaves out keeps the preset's look. For example, a `starship.toml` that only
sets `[character]` still imports the default git, directory, duration,
hostname, username, and language formats. The preset's `[ui]` and
`[ui.recommended]` are not copied.

| Starship module(s) | GPY segment | Notes |
|---|---|---|
| `directory` | `directory` | `format`, `style`, `read_only`, `truncation_symbol` → GPY `format`. `$style` is inlined with the module's literal style value. |
| `git_branch` + `git_status` + `git_state` | `git` | Composed into one GPY git `format`. `$all_status` → `$status`; Starship per-flag variables → GPY `$status`. When only one of `git_branch` / `git_status` is present, the other half uses Starship's default `format` and `style`; when both are absent the preset's git format applies. `git_state` is lossy (GPY has no state overlay); produces a warning. |
| `cmd_duration` | `duration` | `min_time` (ms) → `show_if_exceeds_ms`; `format` / `style` → GPY duration `format`. |
| `character` | `character` | `success_symbol` / `error_symbol` (e.g. `[❯](bold green)`) → GPY `success_symbol`, `error_symbol`, `success_color`, `error_color`, `format`. |
| `rust`, `python`, `nodejs`, `golang`, `java`, `ruby`, `php`, `swift`, `elixir`, `c`, `cpp`, `csharp`, `erlang`, … | `language` | Starts from the preset's language theme (format `via [$symbol( v$version)]($attr fg:$color) `, per-language colors, symbols, `enabled_languages`). Per-language `style` → palette entry + `{lang}_bg_color` (foreground color, `fg:` prefix optional, palette aliases resolved to the selected Starship palette's value) + `{lang}_style` (attributes, exposed as `$attr`). GPY has one shared language format, so a style's `bg:` color is dropped with a `LossyMapping` warning, and a module's own `format` is not carried over (also a `LossyMapping` warning). Per-language `symbol` overrides are lossy (they live in `[language.icons]`, not the prompt theme): each produces a warning and drops the preset's symbol for that language so `[language.icons]` applies. |
| `time` | `clock` | `time_format` (strftime) → GPY `time_format` (`"12"` or `"24"`) + `show_seconds` on a best-effort basis. Non-representable formats produce a warning. |
| `username`, `hostname`, `kubernetes`, `aws`, `docker_context`, `package`, `memory_usage`, `battery`, custom modules, and everything else | — | Skipped with a warning: `module 'X' has no GPY equivalent; omitted from layout`. |

## Style translation

Starship encodes color in `style` fields and inline `($style)` references.
GPY's template grammar accepts the same token vocabulary (`bold`, `fg:`, `bg:`,
named/hex/256-index colors, `prev_fg`, `prev_bg`), so the importer:

1. Replaces `$style` and `$*_style` references in `format` strings with the
   module's literal style value (e.g. `style = "bold purple"` → `(bold purple)`).
2. Drops Starship format variables not in GPY's vocabulary for that segment,
   with a warning per dropped variable name.
3. Passes all other tokens through unchanged — escapes (`\[`, `\]`) and
   conditional groups `(...)` use the same grammar on both sides.

Palette-alias names inside `format`-inlined styles survive as bare color names;
the emitted palette defines them and the renderer resolves them. Language and
character colors are different: GPY stores them as concrete `ColorSpec` values,
so the importer reads the style with the template engine's own `parse_style`
(`fg:`/`bg:` prefixes, case-insensitive tokens) and resolves an alias through the
selected `[palettes.*]` table at import time (`peach` → `#fab387`). A palette
entry that shadows a builtin color name (`red = "#f38ba8"`) is not applied.
A character style's `bg:` color is carried into the shared character `format`
when the success and error styles agree on it; otherwise it is dropped with a
`LossyMapping` warning.

#### Per-language style attributes

A Starship language `style` carries a color plus optional attributes (e.g.
`[java] style = "red dimmed"`). The importer splits them:

- the **color** token → `{lang}_bg_color` (the `$color` source), and
- the **attribute** tokens → `{lang}_style`, exposed in the language `format` as
  `$attr` (composed as `($attr fg:$color)`). When the attributes are exactly
  `bold` (the renderer default) no `{lang}_style` is emitted; a bare-color style
  emits `{lang}_style = ""` to suppress the default.

Supported attributes and their fish-ansi SGR codes:

| Attribute | `Attr` variant | SGR |
|---|---|---|
| `bold` | `Attr::Bold` | `1` |
| `dimmed` | `Attr::Dimmed` | `2` |
| `italic` | `Attr::Italic` | `3` |
| `underline` | `Attr::Underline` | `4` |
| `blink` | `Attr::Blink` | `5` |
| `inverted` | `Attr::Inverted` | `7` |
| `hidden` | `Attr::Hidden` | `8` |
| `strikethrough` | `Attr::Strikethrough` | `9` |

> **Encoder note:** all encoders map every attribute. The fish-ansi encoder
> emits SGR codes directly; the zsh encoder uses native prompt escapes for
> bold (`%B`), underline (`%U`), and standout/inverted (`%S`), and raw SGR
> wrapped in `%{…%}` for the rest (no zsh escape exists for them). Bash/zsh
> live output uses the shell-agnostic ANSI encoder, which already maps all
> eight (#232).

## Palette emission

The importer determines the active Starship palette from the top-level
`palette = "x"` key and the corresponding `[palettes.x]` table. Each
`name → value` pair is color-validated; invalid values are dropped with a
warning. Colors discovered from module `style` fields (language colors) are
merged into the same palette so the emitted artifacts are self-contained.
Palette roles that the preset's language colors reference (`orange`,
`bright_magenta`, `bright_green`, …) are added with the `starship` palette's
values when the source palette does not define them.

If the source defines no palette (no `palette =` key), the importer emits a
palette from discovered colors only and warns:

```
source defined no palette; emitting colors discovered from modules only
```

## Warnings and lossy imports

Warnings are printed grouped to stderr after the import, with a final summary
line. The command exits 0 even on a lossy import.

Warning categories:

| Category | Meaning |
|---|---|
| **Unsupported modules** | Starship module has no GPY equivalent; omitted from layout. |
| **Unrepresentable options** | Option cannot be expressed in a GPY artifact (e.g. no palette defined, palette reference with no matching table). |
| **Invalid colors** | A color value failed validation and was dropped from the palette. |
| **Lossy mappings** | Construct maps only approximately (e.g. per-language symbol overrides, `git_state` integration, fine-grained `time_format`). |

Example stderr output:

```
Unsupported modules:
  - module 'kubernetes' has no GPY equivalent; omitted from layout
Lossy mappings:
  - rust: per-language symbol is lossy (lives in [language.icons]); skipped
imported with 2 warning(s)
```

## What is lossy

These Starship constructs cannot be fully represented in a GPY artifact:

- **Per-language symbols** (`symbol = " "` in `[rust]`, etc.) — GPY stores
  icon overrides in `[language.icons]` config, not in the prompt theme. A
  warning is emitted; manually set them via `gpy config set language.icons.rust`.
- **`git_state`** — Starship's git-state overlay (rebasing, merging, etc.) has
  no direct GPY equivalent; the module is translated to a lossy git format warning.
- **`remote_branch` / per-flag git variables** — Starship's fine-grained git
  status variables are collapsed into GPY's `$status` token.
- **Fine-grained `time_format`** — arbitrary strftime strings are mapped to
  `"12"` or `"24"` on a best-effort basis; complex formats produce a warning.
- **Right-prompt**, **custom modules**, **conditional visibility rules** — out
  of scope; each unsupported module produces an "unsupported" warning.

## Error handling

| Situation | Behavior |
|---|---|
| Source file not found or unreadable | Hard error; non-zero exit. |
| Invalid TOML in source file | Hard error with parse location; non-zero exit. |
| Output files exist without `--force` | Error listing conflicting paths and suggesting `--force`; non-zero exit. |
| Name matches a builtin or plugin theme/palette without `--force` (e.g. the default name `starship` from `starship.toml`) | Error naming the shadowed artifact and suggesting `--name <other>` or `--force`; nothing is written; non-zero exit. |
| Per-construct translation problems | Warning collected; import continues; exit 0. |

## Migration from Starship (#88)

If you are migrating from Starship, the typical workflow is:

```bash
# 1. Import your existing config
gpy theme import ~/.config/starship.toml --name my-prompt --apply-layout

# 2. Activate both artifacts
gpy palette use my-prompt
gpy theme use my-prompt

# 3. Review warnings and fix lossy mappings manually
#    e.g. for per-language icon overrides:
gpy config set language.icons.rust ""
```

The imported theme approximates your Starship look. Palette-alias colors (e.g.
`mauve`, `frost`) resolve at render time through the active palette, so the
visual fidelity depends on having both the theme and palette active.

For the full migration walkthrough — config layout differences, common
pitfalls, and uninstalling Starship afterward — see
[Migrating from Starship](../user/migrating-from-starship.md).

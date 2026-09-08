# Color Palettes

GPY separates *which colors exist* (the palette) from *which colors are used where*
(the prompt theme). Swapping a palette changes every color in the prompt without
touching the theme's layout, symbols, or structure.

## Role vocabulary

GPY names palette slots after their ANSI semantic meaning. Each name maps 1:1 to a
base24 standard slot (tinted-theming `base24/styling.md`):

| GPY role | base24 slot | base24 meaning |
|---|---|---|
| `black` | base00 | ANSI 0 / default background |
| `bright_black` | base03 | ANSI 8 |
| `white` | base05 | ANSI 7 / default foreground |
| `bright_white` | base07 | ANSI 15 |
| `red` | base08 | ANSI 1 red |
| `orange` | base09 | ANSI 16 orange |
| `yellow` | base0A | ANSI 3 yellow |
| `green` | base0B | ANSI 2 green |
| `cyan` | base0C | ANSI 6 cyan |
| `blue` | base0D | ANSI 4 blue |
| `magenta` | base0E | ANSI 5 magenta |
| `brown` | base0F | ANSI 17 dark-red / brown |
| `bright_red` | base12 | bright red |
| `bright_yellow` | base13 | bright yellow |
| `bright_green` | base14 | bright green |
| `bright_cyan` | base15 | bright cyan |
| `bright_blue` | base16 | bright blue |
| `bright_magenta` | base17 | bright magenta |
| `bg` / `surface` / `overlay` / `fg` *(optional)* | base01/02/04/06, base10/11 | background and foreground shades for powerline themes |

`orange` (base09) and `brown` (base0F) are the only names that go beyond the 16
standard ANSI names. Every other role is already familiar from terminal color
configuration.

`purple` is also accepted in a template as a literal color — it renders the
terminal's ANSI magenta (SGR 35) — but, unlike the `magenta` role, it does **not**
track the active palette. Only `magenta` recolors on a palette swap; use it (not
`purple`) when you want a scheme to control the color.

**base16 vs base24.** base16 schemes define slots base00–0F only; the bright slots
(base12–17) are absent. When importing a base16 scheme the importer derives the
bright roles from their base08–0F counterparts so the full role set is always
populated.

## Off-ANSI language colors

The starship theme colors three languages with 256-color indices that have no
direct ANSI equivalent. GPY maps each to the nearest unused standard base24 slot
so the color is consistent, scheme-aware, and named:

| Language | Vanilla 256-index | GPY role | Rationale |
|---|---|---|---|
| Swift | `202` (orange) | `orange` (base09) | Exact semantic match |
| PHP | `147` (light purple) | `bright_magenta` (base17) | 147 ≈ bright magenta/violet |
| C++ / C | `149` (yellow-green) | `bright_green` (base14) | 149 ≈ bright green |

Under the `starship` palette (see below) those slots carry 202/147/149, producing
output byte-identical to vanilla Starship. Under any scheme palette they carry the
scheme's value, so all 13 language indicators recolor when you swap palettes.

## Palette file format

Palettes live in `~/.config/gpy/palettes/` as TOML files:

```toml
name = "nord"
description = "Nord — an arctic, north-bluish color palette"
[colors]
black         = "#2e3440"
bright_black  = "#4c566a"
white         = "#d8dee9"
bright_white  = "#eceff4"
red           = "#bf616a"
orange        = "#d08770"
yellow        = "#ebcb8b"
green         = "#a3be8c"
cyan          = "#88c0d0"
blue          = "#81a1c1"
magenta       = "#b48ead"
brown         = "#5e81ac"
bright_red    = "#bf616a"
bright_yellow = "#ebcb8b"
bright_green  = "#a3be8c"
bright_cyan   = "#88c0d0"
bright_blue   = "#81a1c1"
bright_magenta = "#b48ead"
```

Color values may be:

- Hex strings (`"#rrggbb"` or `"rrggbb"` — the `#` is optional)
- Standard ANSI names (`"red"`, `"bright_cyan"`, …) — useful in the `starship`
  palette where several slots map to the terminal's own ANSI colors
- 256-color indices (`"202"`, `"147"`, …) — preserved for vanilla Starship parity

## The dedicated `starship` palette

`starship` is a first-class builtin palette that reproduces vanilla Starship's
colors. It is automatically paired with the `starship` theme: running
`gpy theme use starship --force` activates the theme *and* sets `ui.palette =
"starship"`. Without `--force` the theme switches but your current palette is
kept, giving you "Starship structure + my Catppuccin colors" out of the box.

The three off-ANSI slots carry Starship's exact 256-indices (`orange = "202"`,
`bright_magenta = "147"`, `bright_green = "149"`). The nine ANSI-name languages
(Rust, Python, Node.js, Go, Java, Ruby, Elixir, C#, Erlang) use standard ANSI
names so the terminal's own colors apply — identical to vanilla Starship.

To swap away from the starship colors while keeping the starship layout:

```bash
gpy palette use catppuccin-mocha   # or nord, gruvbox-dark-medium, …
```

The `ui.palette` field in `~/.config/gpy/config.toml` is all that changes.

## Builtin scheme palettes

GPY ships six palettes generated from their official tinted-theming base16/base24
ports. Values are copied from the source repos, not eyeballed:

| Palette name | Scheme | Variant |
|---|---|---|
| `catppuccin-latte` | Catppuccin | Light |
| `catppuccin-frappe` | Catppuccin | Dark |
| `catppuccin-macchiato` | Catppuccin | Dark |
| `catppuccin-mocha` | Catppuccin | Dark |
| `nord` | Nord | Dark |
| `gruvbox-dark-medium` | Gruvbox | Dark (medium) |

List all available palettes (builtin + user) with:

```bash
gpy palette list
```

## Importing a scheme

Any of the ~230+ [tinted-theming](https://github.com/tinted-theming/home)
base16/base24 scheme YAML files can be imported directly:

```bash
gpy palette import ~/path/to/scheme.yaml
```

The importer:

1. Parses the YAML (the constrained base16/base24 subset — no runtime YAML
   dependency; the file is converted to GPY TOML once at import time).
2. Maps every slot through the §1 role table (1:1, no special cases).
3. Writes `~/.config/gpy/palettes/<slug>.toml`.

Flags:

| Flag | Description |
|---|---|
| `<file>` | Required. Path to a `.yaml` or `.yml` scheme file. |
| `--name <name>` | Override the palette name (defaults to the scheme's `name` field, slugified). |
| `--force` | Overwrite an existing palette of the same name. |

After importing, activate the palette:

```bash
gpy palette use <name>
```

## Swap to a scheme — walkthrough

```bash
# 1. Check what is available
gpy palette list

# 2. Switch to a builtin
gpy palette use catppuccin-mocha

# 3. Or import any tinted-theming scheme
#    (download from https://github.com/tinted-theming/schemes)
gpy palette import ~/Downloads/base16-ocean.yaml
gpy palette use base16-ocean

# 4. Verify
gpy palette show

# 5. Return to default (vanilla Starship colors, if using the starship theme)
gpy palette use starship
```

Swapping a palette takes effect immediately on the next prompt render — no agent
restart required. The prompt theme and all segment settings remain unchanged.

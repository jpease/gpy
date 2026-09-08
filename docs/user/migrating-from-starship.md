<!-- SPDX-License-Identifier: GPL-3.0-or-later -->

# Migrating from Starship

A practical guide for Starship users switching to GPY: what carries over automatically, what config layout changes, and what to check by hand afterward.

**Last Updated:** 2026-09-07

---

## The short version

```bash
# 1. Install GPY (see docs/INSTALL.md) but don't remove Starship's init line yet.

# 2. Convert your existing starship.toml into a GPY palette + theme
gpy theme import ~/.config/starship.toml --name my-prompt --apply-layout

# 3. Activate both artifacts
gpy theme use my-prompt
gpy palette use my-prompt

# 4. Compare the two prompts side by side, then remove Starship's init
#    line from your shell rc file once you're happy with the result.
```

`gpy theme import` prints warnings for anything it couldn't translate exactly — read them before deleting your old `starship.toml`. The rest of this guide explains what those warnings mean and what else differs.

---

## What carries over automatically

The importer (`gpy-agent/src/import/starship/`, exercised by `gpy-agent/tests/starship_import_golden_tests.rs`) translates:

- **Colors** — your active `[palettes.x]` table, plus any colors it finds inline in module `style` fields, become a GPY palette.
- **Directory, git, duration, and character segments** — `format`/`style` fields map onto GPY's equivalent segment `format` strings, including conditional groups (`(...)`) and style tokens (`bold`, `fg:`, `bg:`, `prev_fg`/`prev_bg`).
- **Language segments** — `rust`, `python`, `nodejs`, `golang`, `java`, `ruby`, `php`, `swift`, `elixir`, `c`, `cpp`, `csharp`, `erlang`, and others fold into one GPY `language` segment, with per-language colors and attributes preserved.
- **Clock** — `time_format` maps to GPY's `"12"`/`"24"` setting on a best-effort basis.
- **Segment order** — pass `--apply-layout` and the derived `enabled_segments` order is written straight into `config.toml`; without it, the command just prints the suggested order.

For the full module-by-module mapping table and style-translation rules, see [Starship Importer](../dev/starship-import.md) — that document is the mechanical reference; this one is the walkthrough.

## What does not carry over

Some Starship constructs have no GPY equivalent and are dropped with a warning rather than silently guessed at:

| Starship feature | What happens in GPY |
|---|---|
| `kubernetes`, `aws`, `docker_context`, `package`, `memory_usage`, `battery`, `username`, `hostname`, custom `[custom.x]` modules | Skipped entirely — no equivalent segment exists yet. |
| `git_state` (rebasing/merging/cherry-picking overlay) | Folded into the git segment's `$status` token; the distinct state overlay is lost. |
| Per-flag git status variables (`$conflicted`, `$ahead`, `$behind`, etc.) | Collapsed into GPY's single `$status` token. |
| Per-language `symbol` overrides (e.g. `[rust] symbol = " "`) | Not part of the theme — set them manually after import: `gpy config set language.icons.rust ""`. |
| Right-hand prompt | Not supported; GPY renders one left-aligned prompt line. |
| Arbitrary `time_format` strftime strings | Approximated as `"12"` or `"24"`; anything finer-grained produces a warning. |

`gpy theme import` prints every dropped or approximated construct as a warning grouped by category (unsupported modules, unrepresentable options, invalid colors, lossy mappings) and exits `0` even when warnings fire — a lossy import is not a failure, but it does mean the emitted theme is an approximation you should read through once.

## Config layout differences

| | Starship | GPY |
|---|---|---|
| Config file(s) | One file: `~/.config/starship.toml` (or `$STARSHIP_CONFIG`) | Two layers: `~/.config/gpy/themes/<name>.toml` (structure) + `~/.config/gpy/palettes/<name>.toml` (color), selected from `~/.config/gpy/config.toml` |
| Color reuse | Palette is one table inside the same file | Palette is a separate, independently swappable file — switch colors without touching layout, or vice versa |
| Rendering | Synchronous, in-process per prompt render | Background agent (`gpy-agent`) over a Unix-socket IPC connection, with the shell falling back to a degraded oneshot render if the agent isn't running |
| Live updates | Recomputes on every prompt render | File-watches the repo and pushes updates between renders (`SIGUSR1`); see [Configuration Reference](configuration-reference.md) for the debounce/TTL knobs |

The two-layer theme/palette split is why `gpy theme import` writes *two* files instead of one — see [Theme Customization](theme-customization.md) for how to swap either layer independently afterward.

## Common pitfalls

- **Both prompts fight for the same line.** Don't remove Starship's `eval "$(starship init <shell>)"` line until you've activated the imported GPY theme/palette and are happy with it — otherwise you'll be debugging two prompt systems writing to the same terminal at once. Once you're satisfied, delete that line from `config.fish`/`.zshrc`/`.bashrc` (see [Installation Guide](../INSTALL.md#2-install-shell-files) for where GPY's own init lines live).
- **`--name` collisions.** `gpy theme import` refuses to overwrite an existing theme/palette of the same name unless you pass `--force`. If you already have a `starship`-named theme (GPY ships one as a built-in preset — see [Theme Customization](theme-customization.md#2-quick-swaps-via-cli)), pick a different `--name` for your imported one to avoid confusion between "GPY's Starship-look preset" and "my personal imported Starship config."
- **Missing icons after import.** Per-language `symbol` overrides don't survive the import (see table above). If a language segment's icon looks wrong or generic, set it explicitly: `gpy config set language.icons.<lang> "<icon>"`.
- **Agent not running.** GPY's live-update behavior depends on `gpy-agent` running in the background. Run `gpy status` to check; `gpy --version` and `gpy status` are also the two fields you'll be asked for in any bug report (see the issue template) if something looks broken after migrating.
- **Bash users:** GPY's Bash support is more limited than Starship's — see [Bash Limitations](bash-limitations.md) before assuming full feature parity if Bash is your primary shell.

## Uninstalling Starship

Once you've verified the GPY prompt looks right:

1. Remove Starship's init line from your shell rc file.
2. Optionally uninstall the `starship` binary itself (see Starship's own docs — this isn't a GPY-managed step).
3. Keep your original `starship.toml` around until you're confident you won't need to re-run the importer with different flags (e.g. `--stdout` to inspect output without writing files, or a different `--name`).

---

## See Also

- [Starship Importer](../dev/starship-import.md) — full CLI flag reference, module-by-module mapping table, and style-translation rules.
- [Theme Customization](theme-customization.md) — how theme and palette selection works once you're past the initial import.
- [Configuration Reference](configuration-reference.md) — every `config.toml` key, including the live-update/debounce settings that have no Starship equivalent.
- [Bash Limitations](bash-limitations.md) — read this if Bash is your primary shell.

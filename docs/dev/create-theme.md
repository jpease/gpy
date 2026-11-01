# Create a Theme

How to create a community theme for GPY.

If you want plugin-specific styling details, also read [Build a Segment Plugin](build-segment-plugin.md).

## The Fastest Way

Use the built-in scaffolder:

```bash
gpy theme new mytheme
```

Or build on another existing theme (builtin, user, or plugin) instead of the default theme:

```bash
gpy theme new mytheme --from starship
```

That creates `~/.config/gpy/themes/mytheme.toml` as an **overlay** of the base:

```toml
extends = "starship"
```

Add only the keys your theme changes; everything else comes from the base, which keeps receiving updates. Tables merge key by key at every depth, arrays replace the base's array, and `unset = ["segments.character.format"]` removes a key of the base. If the base is a plugin theme, your theme depends on that plugin. For a theme that must stand alone (to publish it, or to freeze it against later releases), add `--full`: the file is then the base's whole theme, verbatim when the base is a full theme or merged when it is an overlay.

Then edit it, validate it, and activate it:

```bash
gpy theme validate mytheme
gpy theme use mytheme
```

## Theme File Structure

A full theme is a TOML file with:

- top-level UI settings under `[ui]`
- per-segment styling under `[segments.<segment-id>]`

An overlay (a file with a top-level `extends = "<theme>"`) needs neither table; it holds only what it changes, and its merged result must be a valid full theme.

Start from the generated theme or from [templates/theme.plugin-segment.toml](templates/theme.plugin-segment.toml).

Example:

```toml
[ui]
root_prompt_icon = "❯"
root_prompt_color = "red"

[segments.character]
success_symbol = "❯"
success_color = "white"

[segments.directory]
bg_color = "cyan"
text_color = "black"

[segments.git]
bg_color = "white"
text_color = "black"

[segments.acme-tools]
icon = "⚙"
bg_color = "#1d3557"
text_color = "#f1faee"
```

## Styling Built-In and Plugin Segments

The same pattern works for both:

- built-in segment styling lives under `[segments.<built-in-id>]`
- plugin segment styling lives under `[segments.<plugin-segment-id>]`

That means community themes do not need a separate mechanism for plugin segments. If a plugin exposes a segment id, theme authors style it the same way they style `git`, `clock`, or `directory`.

Useful built-in references:

- [`config/themes/default.toml`](../../config/themes/default.toml)
- [`fish/segments/directory.fish`](../../fish/segments/directory.fish)

## Git Segment: State, Stash, Detached HEAD & `git_style`

The `[segments.git]` table supports a 6-bucket color-state model, in
precedence order (highest first): `Conflicts > InProgress > Modified >
UntrackedOnly > AheadBehind > Clean`. Each bucket has its own optional
`*_bg_color`/`*_text_color` pair; any unset pair falls back to the segment's
top-level `bg_color`/`text_color`:

```toml
[segments.git]
format = "[](fg:prev_bg bg:$bg)[ $symbol $branch]($style)([ $state]($style))([ $ahead_behind]($style))([ $status]($style))([ $stash]($style))([$sep_close](fg:$bg bg:default))"
bg_color = "white"
text_color = "black"

clean_bg_color = "green"
clean_text_color = "black"
dirty_bg_color = "red"        # "Modified" bucket — backward-compatible name
dirty_text_color = "black"
ahead_behind_bg_color = "yellow"
ahead_behind_text_color = "black"
conflicts_bg_color = "red"
conflicts_text_color = "black"
untracked_only_bg_color = "cyan"    # distinct from dirty_* by default
untracked_only_text_color = "black"
in_progress_bg_color = "magenta"    # distinct from conflicts_* by default
in_progress_text_color = "white"
```

`$state` renders an in-progress merge/rebase/cherry-pick/etc. indicator (with
step/total progress for an interactive rebase, e.g. `↻ REBASING 3/5`) and is
empty for every other state. `$stash` renders a stash count (e.g. `≡2`) and is
empty when there are no stashes; it also requires `[git] stash_enabled = true`
in `config.toml` (the default — see [Advanced Configuration](../user/advanced-configuration.md)
for the `[git]` `icon_set`/`stash_enabled` config keys). Detached HEAD is
rendered as a distinct `$symbol` glyph whenever `[ui] show_icons` is true (no
more `HEAD@<sha>` smuggled into `$branch` — `$branch` always renders the plain
branch/SHA text); no extra config is needed beyond including `$symbol` in
`format`. Both `format` strings above collapse `$state`/`$stash` to nothing
when empty, so a clean, unstashed repo renders identically to a theme that
predates this feature.

Per-element text colors (`branch_text_color`, `arrows_text_color`,
`staged_text_color`, `unstaged_text_color`, `untracked_text_color`,
`conflicts_indicator_text_color`, `state_text_color`) take precedence over the
per-state colors above when set, letting a theme recolor one piece (e.g. the
branch name) independent of the active color state.

For bold/dimmed/italic emphasis per state — independent of color — add a
`[segments.git.git_style]` subtable keyed by the lower-snake-case state name
(`clean`, `ahead_behind`, `untracked_only`, `modified` — the dirty state, also
set by the `dirty_*` colours — `in_progress`, `conflicts`), mirroring the per-language `styles` mechanism:

```toml
[segments.git.git_style]
conflicts = "bold"
in_progress = "italic"
```

An unset entry defaults to no extra attribute (`$style` still carries the
resolved `fg:`/`bg:` colors either way).

## Recommended Theme Workflow

1. Create the theme with `gpy theme new <name>` (an overlay of `default`; `--from <base>` picks another base, `--full` writes a full theme).
2. Edit `~/.config/gpy/themes/<name>.toml`.
3. Add styling for the segments you care about.
4. Validate with `gpy theme validate <name>`.
5. Activate with `gpy theme use <name>`.
6. Edit iteratively and rely on hot reload while developing.

## Good Theme Authoring Practices

Prefer:

- clear defaults for all commonly used built-in segments
- explicit styling for plugin segments your theme is meant to showcase
- legible foreground/background combinations
- icons that remain readable in common terminal fonts

Avoid:

- relying on undocumented internal shell variables
- assuming all users have the same Nerd Font coverage
- shipping themes that only work when a plugin is installed but provide no fallback styling

## Validation

Use:

```bash
gpy theme validate <theme-name>
```

Or validate a file directly:

```bash
gpy theme validate ~/.config/gpy/themes/mytheme.toml
```

If validation fails, GPY should report:

- the failing field
- the invalid value
- remediation guidance

## Troubleshooting

Theme not found:

- confirm the file exists under `~/.config/gpy/themes/`
- confirm the active config uses the same theme name

Invalid colors:

- use named colors, ANSI `0-255`, or `#RRGGBB`

Icons fail validation:

- remove empty or invalid glyphs
- use visible printable characters only

Plugin segment does not look right:

- confirm the plugin segment id matches the table name exactly
- confirm the plugin segment actually reads theme variables for the keys you set

## Themes Shipped in a Plugin

A plugin's `themes/` directory may hold full themes or overlays. A plugin overlay must extend a builtin theme (`extends = "starship"`) and always gets the embedded builtin, never a user or another plugin's theme of that name; `gpy plugin validate` checks it.

Users who customize a plugin theme with `gpy theme new <name> --from <plugin-theme>`, `gpy theme save` or `gpy config wizard` get an overlay that extends it, so their theme depends on your plugin: `gpy` names the plugin when it writes the file, and the theme stops loading if the plugin is removed or disabled. Renaming or removing a shipped theme therefore breaks such overlays. `--full` gives users an independent copy instead.

## Compatibility Notes

Theme authors can rely on:

- TOML theme files remaining the canonical format
- `[segments.<segment-id>]` remaining the extension point for segment styling
- `gpy theme validate` remaining the supported way to check correctness before release

For plugin contract and versioning policy, see [Plugin Framework](plugins.md).

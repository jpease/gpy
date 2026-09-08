# Build a Segment Plugin

This guide is the reference workflow for community contributors who want to add a custom prompt segment without modifying GPY itself.

If you want the contract details first, read [Plugin Framework](plugins.md).

## Before You Start

The simplest reference implementations are existing first-party Fish segments:

- [`fish/segments/clock.fish`](../../fish/segments/clock.fish)
- [`fish/segments/directory.fish`](../../fish/segments/directory.fish)

They are not shipped as standalone external plugins, but they use the same detect/render contract that plugin segments use at runtime. For most plugin authors, `clock` is the best minimal reference and `directory` is a good reference for theme-driven rendering.

## Plugin Layout

Create this directory structure:

```text
~/.config/gpy/plugins/<plugin-id>/
├── plugin.toml
└── segments/
    └── <segment-id>.fish
```

Example:

```text
~/.config/gpy/plugins/acme-tools/
├── plugin.toml
└── segments/
    └── acme-tools.fish
```

## Step 1: Create `plugin.toml`

Start from [templates/plugin.toml](templates/plugin.toml):

```toml
id = "acme-tools"
name = "Acme Tools"
version = "0.1.0"
api_version = "v1"
entry_type = "file"
provided_segments = ["acme-tools"]
description = "Example plugin segment for Acme tooling status"

[compatibility_constraints]
min_gpy_version = "0.1.0"
```

Guidance:

- `id` identifies the plugin package.
- `provided_segments` lists the segment ids this plugin contributes.
- `api_version = "v1"` means the plugin targets the current public plugin contract.
- `min_gpy_version` is recommended for community plugins. It gives users a clear diagnostic when the plugin needs a newer GPY.

## Step 2: Implement the Segment File

Create `~/.config/gpy/plugins/<plugin-id>/segments/<segment-id>.fish`.

Minimal example:

```fish
function segment_acme-tools_detect
    command -sq acme
end

function segment_acme-tools_render --argument-names is_last
    set -l bg_color (set -q __color_acme_tools_bg; and echo $__color_acme_tools_bg; or echo blue)
    set -l fg_color (set -q __color_acme_tools_fg; and echo $__color_acme_tools_fg; or echo white)
    set -l icon (set -q __icon_acme_tools; and echo $__icon_acme_tools; or echo "A")

    gpy_section_start $bg_color $fg_color "$icon acme"
    gpy_section_end $bg_color $is_last
end
```

Contract:

- `segment_<segment-id>_detect`
  - exit `0` to show the segment
  - exit non-zero to hide it
- `segment_<segment-id>_render`
  - prints the rendered segment
  - receives `is_last` as its first argument: `true` when the segment is the
    last one in the prompt, empty otherwise
  - optionally receives `is_first` as its second argument, in the same shape:
    `true` when the segment is the first one, empty otherwise

Supported public rendering helpers:

- `gpy_section_start`
- `gpy_section_append`
- `gpy_section_end`
- `gpy_section_standalone`

Those helper names are the supported shell helper surface for plugin authors. Avoid depending on undocumented `__gpy_*` internals.

## Step 3: Validate Discovery

Run:

```bash
gpy plugin new <plugin-id> --segment <segment-id>
gpy plugin validate ~/.config/gpy/plugins/<plugin-id>
gpy plugin list
gpy segments
```

You should see:

- the plugin listed by `gpy plugin list`
- the segment id listed by `gpy segments`

## Step 4: Enable the Segment

Run:

```bash
gpy enable <segment-id>
```

Then verify the config contains the segment under `ui.enabled_segments`:

```bash
gpy config get ui.enabled_segments
```

## Step 5: Add Theme Styling

Add a segment table to your theme:

```toml
[segments.acme-tools]
icon = "A"
bg_color = "#1d3557"
text_color = "#f1faee"
```

Then validate and activate:

```bash
gpy theme validate
gpy theme use <theme-name>
```

For a full theme workflow, see [Create a Theme](create-theme.md).

## Design Guidance

Prefer these patterns:

- Keep `detect` cheap.
- Keep `render` deterministic and fast.
- Use theme variables instead of hardcoded colors and icons.
- Gracefully degrade when the backing tool is absent.
- Cache or precompute expensive state outside prompt render if the plugin needs heavier logic.

Avoid these patterns:

- Running slow network calls during render.
- Printing extra logging or debugging text from render functions.
- Depending on GPY private shell internals beyond the documented detect/render contract.

## Troubleshooting

Plugin not discovered:

- confirm the manifest is at `~/.config/gpy/plugins/<plugin-id>/plugin.toml`
- confirm `provided_segments` includes the segment id
- run `gpy plugin validate <path>`

Segment not rendering:

- confirm `gpy enable <segment-id>` was run
- confirm `segment_<segment-id>_detect` returns `0`
- confirm `segment_<segment-id>_render` prints output

Colors or icons not applying:

- confirm the theme has `[segments.<segment-id>]`
- run `gpy theme validate`
- inspect a simple built-in segment like [`fish/segments/directory.fish`](../../fish/segments/directory.fish) for the expected theme-variable pattern

## Performance Expectations

Plugin discovery budgets are documented in [performance/plugin-budgets.md](performance/plugin-budgets.md).

Runtime expectations for segment authors:

- detection should usually be sub-millisecond
- rendering should avoid subprocess storms
- prompt rendering should remain effectively instantaneous to the user

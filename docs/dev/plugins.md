# Plugin Framework

This document defines the v1 plugin contract used by GPY for community-extensible segments.

For an end-to-end contributor workflow (plugin + theme), see:
- [Build a Segment Plugin](build-segment-plugin.md)
- [Create a Theme](create-theme.md)
- [Plugin + Theme Authoring Guide](plugin-theme-authoring.md)
- [Plugin Manifest Template](templates/plugin.toml)
- [Theme Template with Plugin Segment Styling](templates/theme.plugin-segment.toml)

## Manifest (plugin.toml)

Plugins are discovered from:
- User root: `$XDG_CONFIG_HOME/gpy/plugins/<plugin-id>/plugin.toml`
- Bundled root: `$GPY_BUNDLED_PLUGIN_DIR/<plugin-id>/plugin.toml` (optional)

Required fields:
- `id`
- `name`
- `version`
- `api_version`
- `entry_type` (`file`) — only `file` plugins are loadable by the current
  runtime. `command` is reserved and is rejected by `gpy plugin validate` and
  plugin discovery (reported as an "unsupported entry_type" diagnostic).
- `provided_segments` (array)

Optional fields:
- `description`
- `compatibility_constraints.min_gpy_version`

Identifier normalization/validation:
- `id`: lowercase, `[a-z0-9-]+`
- `provided_segments`: lowercase, `[a-z0-9_-]+`

Implementation: `gpy-agent/src/plugin/manifest.rs`.

## What Is Stable in v1

The following are public plugin-contract guarantees in `api_version = "v1"`:

- manifest file name and location: `plugin.toml` under a plugin root
- required manifest fields and validation rules documented here
- plugin discovery precedence rules
- the segment runtime contract:
  - `segment_<id>_detect`
  - `segment_<id>_render`
- the supported shell rendering helpers:
  - `gpy_section_start`
  - `gpy_section_append`
  - `gpy_section_end`
  - `gpy_section_standalone`
- user configuration through `ui.enabled_segments`
- theme styling through `[segments.<segment-id>]`

The following are intentionally not public plugin API guarantees:

- private shell variable names used internally by GPY to source plugin files
- internal registry implementation details beyond the documented behavior
- undocumented theme variable naming conventions

If you are writing a community plugin, target the documented contract, not GPY internals.

## Registry Behavior

Discovery is deterministic and isolated:
- First-party metadata entries are always included (`gpy-core`)
- User plugins override bundled plugins by plugin id
- Invalid plugins become diagnostics and do not block healthy plugins
- Output ordering is stable (sorted by plugin id)

Implementation: `gpy-agent/src/plugin/registry.rs`.

## Segment Runtime Contract

Plugin segments use the same Fish detect/render contract as built-ins:
- detect: `segment_<id>_detect`
- render: `segment_<id>_render`

By convention, segment files live at:
- `<plugin-root>/segments/<segment-id>.fish`

Theme export emits `__gpy_plugin_segment_file_<segment_token>` variables.
`fish/core/init.fish` uses these variables to source plugin segment files when a segment is enabled.

That export variable naming is an implementation detail, not a plugin-author API. Plugin authors should rely on the manifest + segment-file contract, not on private shell variables.

## Segment Ordering Semantics

`ui.enabled_segments` is authoritative.

Behavior:
- Built-in segments keep their canonical order when toggled
- Plugin segments are appended and preserve user-config order
- Segment IDs are no longer hardcoded to built-ins

## First-Party vs Community Plugins

Built-in segments are represented in registry metadata as a first-party plugin entry (`gpy-core`) to unify discovery and tooling UX.

Important distinction:
- First-party entries are metadata only
- Built-in runtime execution still uses existing optimized local code paths

For community examples, the best current references are first-party segment implementations such as:

- [`fish/segments/clock.fish`](../../fish/segments/clock.fish)
- [`fish/segments/directory.fish`](../../fish/segments/directory.fish)

These are good reference implementations, but they are not distributed as standalone external plugins.

## Versioning Policy

GPY uses semantic versioning for the project, and the plugin contract is versioned separately through `api_version`.

Rules:

- community plugins should currently declare `api_version = "v1"`
- backward-compatible GPY changes may add capabilities without breaking `v1`
- if GPY needs a breaking plugin-contract change, it must introduce a new `api_version`
- a plugin that requires new GPY behavior should set `compatibility_constraints.min_gpy_version`

Practical meaning:

- a GPY `0.1.x` to later `0.1.y` upgrade should not break a valid `v1` plugin
- if GPY ever needs to break the `v1` plugin contract, it should do so explicitly with a new plugin API version rather than silently changing `v1`

## Compatibility Guarantees

For `api_version = "v1"`, GPY aims to preserve:

- manifest parsing semantics for documented fields
- deterministic discovery and precedence behavior
- runtime loading of declared segment files
- config-based enable/disable behavior via `ui.enabled_segments`
- theme styling lookup via segment id

Compatibility failure mode:

- incompatible or invalid plugins should fail with diagnostics
- they should not crash the agent
- they should not block healthy plugins from loading

## Guidance for Community Authors

Recommended:

- always set `min_gpy_version`
- keep plugin ids and segment ids stable once released
- treat segment ids as user-facing configuration API
- document any theme keys your segment expects beyond `icon`, `bg_color`, and `text_color`

Avoid:

- depending on undocumented shell variables
- assuming bundled first-party metadata implies an external plugin package exists
- overloading one plugin with unrelated segment ids unless they clearly belong together

## Validation Workflow

Use CLI checks during authoring:
- `gpy plugin validate <plugin-path-or-id>`
- `gpy theme validate [theme-name-or-path]`
- `gpy doctor`

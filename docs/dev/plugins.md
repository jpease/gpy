# Plugin Framework

This document defines the v1 plugin contract for community segments.

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
- `entry_type`:
  - `file`: one Fish file per segment, loaded by Fish only (see
    [Segment Runtime Contract](#segment-runtime-contract)).
  - `script`: one POSIX `sh` script per segment, declared in
    `segments/<segment>.toml` and run by every shell (see
    [Script Segments](#script-segments)).
  - `command` is reserved and is rejected by `gpy plugin validate` and plugin
    discovery (reported as an "unsupported entry_type" diagnostic).
- `provided_segments` (array)

Optional fields:
- `description`
- `compatibility_constraints.min_gpy_version`: enforced. `gpy plugin validate`
  fails, and discovery skips the plugin with a diagnostic, when the running GPY
  is older.

Identifier normalization/validation:
- `id`: lowercase, `[a-z0-9-]+`
- `provided_segments`: lowercase, `[a-z0-9_-]+`
- Each segment name maps to one shell variable token: the name with `-` turned
  into `_` (no other change, so `my--seg` → `my__seg`, `seg_` → `seg_`). Two
  distinct names with the same token (`a-b` and `a_b`) are rejected: in one
  manifest, across discovered plugins (the later plugin by id is dropped with a
  diagnostic), in `ui.enabled_segments`, and among a theme's `[segments.<name>]`
  sections.

Implementation: `gpy-agent/src/plugin/manifest.rs`. JSON schema:
[`gpy-agent/schemas/plugin-manifest.json`](../../gpy-agent/schemas/plugin-manifest.json),
generated from the Rust types (a test fails when they drift).

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

Target the documented contract, not GPY internals.

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

Theme export emits `__gpy_plugin_segment_file_<segment_token>` variables, where
`<segment_token>` is the segment name lowercased with every `-` turned into `_`
and nothing else changed (`my--seg` → `my__seg`, `seg_` → `seg_`); names whose
tokens collide are rejected (see Identifier normalization above).
`fish/core/init.fish` uses these variables to source plugin segment files when a segment is enabled.
Plugin segment files are Fish scripts sourced with your full user privileges; they are Fish-only, so these variables are exported for Fish only.

That export variable naming is an implementation detail, not a plugin-author API. Plugin authors should rely on the manifest + segment-file contract, not on private shell variables.

## Script Segments

A plugin with `entry_type = "script"` provides, for each segment in
`provided_segments`, a manifest at `<plugin-root>/segments/<segment>.toml` and
a POSIX `sh` script it names. No `.fish` file is needed. Bash, Zsh and Fish
run the same script; the shell adapters that do so are tracked separately, so
this section is the contract they implement. JSON schema:
[`gpy-agent/schemas/script-segment-manifest.json`](../../gpy-agent/schemas/script-segment-manifest.json).
Implementation: `gpy-agent/src/plugin/script.rs` and `settings.rs`.

```toml
# segments/k8s.toml
description = "Kubernetes context"
command = "bin/k8s.sh"           # inside the plugin root, run by /bin/sh
inputs = ["is_ssh"]              # GPY_IS_SSH; no `pwd`, so session scope
env = ["KUBECONFIG"]             # forwarded and part of the cache key
ttl_secs = 5                     # default 5; 0 runs on every due prompt
timeout_ms = 300                 # default and maximum 500
require_env = ["KUBECONFIG"]

[settings.style]
type = "string"
default = "bold"
choices = ["bold", "dim"]
description = "Text weight"
```

### Fields

| Field | Type | Default | Rule |
|---|---|---|---|
| `command` | string | required | Path relative to the plugin root, without `..`; must be a regular file inside the root after symlinks. Run as `/bin/sh <command>`. |
| `description` | string | none | Shown by `gpy plugin validate`. |
| `inputs` | array | `[]` | Public shell values, below. Only declared inputs are set and keyed. |
| `env` | array | `[]` | Extra variables forwarded to the script and keyed in the cache. Names match `^[A-Za-z_][A-Za-z0-9_]*$` (ASCII); `GPY_*` is reserved. |
| `ttl_secs` | integer | `5` | `0`–`31536000`. Seconds a result is reused; `0` runs the script on every due prompt. |
| `timeout_ms` | integer | `500` | `1`–`500`: an author may only lower it. |
| `require_files` | array | `[]` | Every name must exist as a file in the working directory or an ancestor. On Linux the search stops at the mount that holds the working directory; on macOS it goes up to `/`. |
| `require_folders` | array | `[]` | Every name must exist as a folder in the working directory or an ancestor, with the same stop as `require_files`. |
| `require_extensions` | array | `[]` | Every extension (no leading dot) must match a file in the working directory itself. |
| `require_env` | array | `[]` | Every variable must be set and non-empty. |
| `require_command` | string | none | A script, like `command`, that must exit 0. It runs last, only after the other conditions hold, under the same runner and deadline. |
| `settings` | table | `{}` | Typed settings, below. |

Every `require_*` condition must hold (all of them, unlike Starship's
`detect_*`, where any one is enough). The file, folder and extension
conditions are checked in a child process with a 30 ms deadline; a timeout
hides the segment for that prompt. Unknown keys, duplicate list entries and
invalid names are load errors that name the file and key.

### Inputs: public shell state

| Input | Variable | Value |
|---|---|---|
| `pwd` | `GPY_PWD` | The shell's working directory. Declaring it also runs the script there and makes the cache per folder. |
| `is_root` | `GPY_IS_ROOT` | `1` as root, else `0`. |
| `is_sudo` | `GPY_IS_SUDO` | `1` inside a `sudo` session, else `0`. |
| `is_ssh` | `GPY_IS_SSH` | `1` over SSH, else `0`. |
| `cmd_duration` | `GPY_CMD_DURATION` | The last command's duration in milliseconds; empty before the first command. |
| `prompt_icons` | `GPY_PROMPT_ICONS` | The glyph mode: `nerd` (Nerd Font icons) or `ascii`. |

The names are the shells' own state variables (`__gpy_is_root`,
`__gpy_is_sudo`, `__gpy_is_ssh`, `__gpy_cmd_duration`, `__prompt_icons`)
without their private prefix.

**Cache scope** follows from `inputs`: without `pwd` a result is reused for
the whole shell session (while the declared inputs, `env` values and settings
are unchanged, and for at most `ttl_secs`); with `pwd` it is reused per
folder. There is no device-wide scope and no cache file. A segment runs at
most once per command: on the first prompt, after a command, or when the
working directory changed, and never on a repaint.

### Settings

`[settings.<key>]` declares a typed setting. Keys match `^[a-z_][a-z0-9_]*$`
(at most 64 bytes); `ttl_secs` and `timeout_ms` are reserved.

| `type` | Fields | Script sees |
|---|---|---|
| `bool` | `default` | `true` / `false` |
| `int` | `default`, optional `min`, `max` (inclusive) | the decimal number |
| `string` | `default`, optional `choices` | the string (no control characters) |

Each declaration may have a `description`. A default outside the range or
choices, `min` above `max`, or a repeated choice is a load error naming
`settings.<key>`. The value reaches the script as `GPY_OPT_<key>`. Users
override `ttl_secs`, `timeout_ms` (lower only) and any declared setting in
`config.toml` under `[plugin.<segment>]`, with the same checks; `gpy config
get/set/list` reach them as `plugin.<segment>.<key>` (see the
[configuration reference](../user/configuration-reference.md#pluginsegment)).
Settings never go in the theme.

### Output protocol

- Line 1 is the segment text. Empty output hides the segment.
- Line 2, optional, names a colour variant matching `^[a-z0-9_]{1,32}$`. The
  theme styles it with `<variant>_bg_color` and `<variant>_text_color` in
  `[segments.<segment>]`; an unknown or unstyled variant uses the segment's
  colours.
- More than 4096 bytes of output hides the segment.
- NUL is unsupported: output after the first NUL is undefined.
- C0 controls and DEL are stripped. Output that still holds a C1 control
  (U+0080 to U+009F) hides the segment. The text is then escaped for the
  shell once, before it is cached; GPY never stores raw output.
- stderr is discarded. A non-zero exit hides the segment.

GPY draws the colours, caps and separators: a script segment is a pill styled
by `[segments.<segment>]` (`icon`, `bg_color`, `text_color`, `open`, `close`
and the variant colours). The theme's `format` templates are not available to
script segments.

Theme export writes these styles as shell variables named from the segment
token (`<tok>`) and the variant: `__gpy_segment_<tok>_{id,bg_color,text_color,icon}`,
`__gpy_segment_<tok>_{open,close}_{icon,icon_color,bg_color}`,
`__gpy_segment_<tok>_variant_<variant>_{bg,text}_color`, and the flat aliases
`__color_<tok>_{bg,fg}`, `__color_<tok>_<variant>_{bg,fg}` and `__icon_<tok>`.
Segment names and variants both allow `_`, so two sections can name the same
variable: segment `a` with an `open` cap and segment `a_open` with an `icon`
(`__gpy_segment_a_open_icon`), segment `a` with variant `x` and segment
`a_variant_x` (`__gpy_segment_a_variant_x_bg_color`), or segment `a` with
variant `b_c` and segment `a_b` with variant `c` (`__color_a_b_c_bg`). Theme
validation rejects such a theme and names both fields, for example
`"segments.a.open.icon" and "segments.a_open.icon" both export shell variable
"__gpy_segment_a_open_icon"`. Export skips a section whose variables collide
with an earlier one's in sorted order, so no variable is assigned twice.

### Environment and process rules

- **Environment.** The script starts from a clean environment: `PATH`
  (without empty, `.` or other relative entries), `HOME`, `USER`, `LOGNAME`,
  `LANG`, every set `LC_*`, `TERM`, `TZ` and
  `TMPDIR` (each only when set), plus the declared `env` names, plus the
  `GPY_*` inputs and `GPY_OPT_*` settings the runner sets. Nothing else is
  passed: no credentials, `SSH_AUTH_SOCK` or exported shell functions unless
  declared, and a `GPY_OPT_*` exported by the user never reaches it. `/bin/sh`
  itself adds `PWD`, `SHLVL` and `_`. A `require_command` script gets the same
  environment and the same process rules as `command`.
- **Working directory**: the plugin root, or the shell's directory when `pwd`
  is declared.
- **stdin** is `/dev/null`; the script never gets the terminal.
- **Process group.** The script runs in its own process group (or session).
  On timeout or Ctrl-C the group gets TERM, then CONT, then KILL after a 50 ms
  grace (or as soon as the script exits). Processes the script leaves behind
  after a normal exit are killed the same way. A `setsid` or daemonizing
  escapee is outside this guarantee; do not start background jobs.
- **Timeout.** A script still running after `timeout_ms` is killed and the
  segment is hidden for that prompt. All script segments of one prompt share
  one 500 ms deadline; a segment not finished before it is skipped for that
  prompt and due again on the next.
- **Circuit breaker.** Two consecutive timeouts (of the script,
  `require_command` or the trigger scan) or over-cap outputs disable the
  segment for that shell, with one line on stderr. A success, a completed
  scan, the reload doorbell or a new shell resets it. Deadline-skipped runs,
  Ctrl-C and a non-zero exit do not count.

### Trust

A script runs as you, once per command that passes its triggers, including in
untrusted directories whose names and files it receives through `pwd`.
Install a script plugin only if you would trust it as a shell startup file.

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
- a breaking change to the `v1` contract ships as a new plugin API version

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
- document the colour variants your script prints, so themes can style `<variant>_bg_color` and `<variant>_text_color`

Avoid:

- depending on undocumented shell variables
- assuming bundled first-party metadata implies an external plugin package exists
- overloading one plugin with unrelated segment ids unless they clearly belong together

## Validation Workflow

Use CLI checks during authoring:
- `gpy plugin validate <plugin-path-or-id>`
- `gpy theme validate [theme-name-or-path]`
- `gpy doctor`

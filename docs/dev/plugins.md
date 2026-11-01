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
- Bundled root: `$GPY_BUNDLED_PLUGIN_DIR/<plugin-id>/plugin.toml`, default
  `$XDG_CONFIG_HOME/gpy/bundled-plugins` (see [Bundled Plugins](#bundled-plugins))

Required fields:
- `id`
- `name`
- `version`
- `api_version`
- `entry_type`:
  - `file`: one Fish file per segment, loaded by Fish only (see
    [Segment Runtime Contract](#segment-runtime-contract)).
  - `script`: one POSIX `sh` script per segment, declared in
    `segments/<segment>.toml`, run by Bash, Zsh and Fish (see
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

## Bundled Plugins

The `username` and `hostname` segments are ordinary script plugins that ship
with GPY, not built-in code. Their sources are the `plugins/` tree of the
repository (`plugins/username`, `plugins/hostname`); there is no built-in
renderer, IPC request or `gpy-agent oneshot` mode for either.

- **Where they live.** Discovery reads the bundled root from
  `$GPY_BUNDLED_PLUGIN_DIR`, default `$XDG_CONFIG_HOME/gpy/bundled-plugins`
  (`~/.config/gpy/bundled-plugins`). A user plugin with the same id in
  `plugins/` overrides a bundled one.
- **How they get there.** Release archives (`gpy-release.tar.gz`,
  `gpy-release.zip`) and `gpy-shell-files.tar.gz` carry `plugins/`, and the
  Homebrew formula installs it under `share/gpy/plugins`. `gpy setup` (run by
  `install.sh`, `install-oneline.sh`, `install-dev.fish` and by hand after a
  Homebrew install) replaces the bundled root with the source's `plugins/`
  tree and clears the group and world write bits, because script segments
  refuse a script that others can write (see [Trust](#trust)). Given `--from`
  or `GPY_SHARE_DIR`, setup also keeps a copy in `share/gpy/plugins` beside
  the binary. `gpy uninstall` removes the bundled root.
- **Settings.** Both are configured under `config.toml`
  `[plugin.hostname]` (`show_always`, `trim_at`) and `[plugin.username]`
  (`show_always`), not in the theme.
- **Styling.** Theme `[segments.hostname]` / `[segments.username]` take the
  usual style keys plus per-variant keys: `<variant>_bg_color`,
  `<variant>_text_color` and `<variant>_icon` (for example `ssh_icon` for the
  hostname's `ssh` variant, `root_icon` for the username's `root` variant).
  The old `format` and `user_format` keys no longer exist.

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
a POSIX `sh` script it names. No `.fish` file is needed.

The contract is defined and validated (`gpy plugin validate`, `gpy config`, the
JSON schema below), and theme export writes every enabled script segment for
Bash, Zsh and Fish (see [Theme export](#theme-export)); each shell runs the
exported scripts with its own runner (`bash/core`, `zsh/core`, `fish/core`)
under the rules below. JSON schema:
[`gpy-agent/schemas/script-segment-manifest.json`](../../gpy-agent/schemas/script-segment-manifest.json).
Implementation: `gpy-agent/src/plugin/script.rs`, `settings.rs` and
`gpy-agent/src/theme/export/script_segments.rs`.

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
| `command` | string | required | Path relative to the plugin root, without `..`; must be a regular file inside the root after symlinks, at a UTF-8 path without control characters, and pass the [ownership rule](#theme-export). Run by `/bin/sh`. |
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
  `TMPDIR` (each only when set), plus each declared `env` name that is set as
  a variable in the interactive shell (exported or not; an empty value is
  passed, an unset name is not), plus the `GPY_*` inputs and `GPY_OPT_*`
  settings the runner sets. Nothing else is
  passed: no credentials, `SSH_AUTH_SOCK` or exported shell functions unless
  declared, and a `GPY_OPT_*` exported by the user never reaches it. `/bin/sh`
  itself adds `PWD`, `SHLVL` and `_`. A `require_command` script gets the same
  environment and the same process rules as `command`.
- **How it starts.** The interpreter runs as `sh -c PROLOGUE SCRIPT` with an
  empty environment (`exec -c`): the prologue reads the `export NAME='value'`
  lines on descriptor 3, closes it, and sources the script, so `$0` is the
  script path as with `sh SCRIPT`. Values are never in any argv; only the
  names travel as arguments of the runner.
- **Working directory**: the plugin root, or the shell's directory when `pwd`
  is declared (folder scope). For folder scope the runner gets an empty
  directory argument and keeps the working directory it inherited, so `$PWD`
  never appears in an argv.
- **stdin** is `/dev/null`; the script never gets the terminal.
- **Process group.** `require_command` and `command` run one after the other
  in one process group, under one deadline. On timeout or Ctrl-C the group
  gets TERM, then CONT, then KILL after a 50 ms grace (or as soon as the
  script exits). Processes the script leaves behind after a normal exit get
  TERM and CONT, then KILL within the grace. A `setsid` or daemonizing
  escapee is outside this guarantee; do not start background jobs.
- **Timeout.** All due segments of one prompt start together and each is
  killed `timeout_ms` (at most 500) after that shared start, so the run takes
  at most 500 ms plus the grace. A timed-out segment is hidden for that
  prompt. The runner takes at most 40 due segments per prompt; the rest stay
  hidden.
- **Results.** A non-zero exit, a failed `require_command` and output holding
  a C1 control hide the segment, and that empty result is cached for
  `ttl_secs` like a success. A timeout, over-cap output or Ctrl-C hides it
  without caching. A failed trigger hides it and drops its cache entry.
- **Circuit breaker.** Each segment has two counters: consecutive scan
  timeouts and consecutive run failures (timeout or over-cap output). When
  either reaches two, the segment is disabled for that shell, with one line
  on stderr. A completed scan resets the scan counter, a successful run the
  run counter; the reload doorbell or a new shell resets both. Ctrl-C, a
  non-zero exit and a failed `require_command` do not count.

### Theme export

Theme export writes what a shell needs to run each enabled script segment
without calling GPY, to Bash, Zsh and Fish alike. Only names in
`ui.enabled_segments` are written, in that order; a built-in name (even if a
plugin provides it), a name no script plugin provides and a second spelling of
an already written token are skipped. Export resolves `command` and
`require_command` again, with the containment and ownership checks, and
applies `[plugin.<segment>]`; a segment that fails either is left out (one
debug log line) and never runs.

**Ownership rule.** The script, and every directory from it up to and
including the plugin root, must be owned by you or root and must not be group-
or world-writable (fix with `chmod go-w`). Every canonical ancestor above the
root, up to `/`, must be owned by you or root and not group- or
world-writable, unless it is sticky (like `/tmp`), where the kernel stops
others renaming your root, or group-writable only through your own
user-private group (named like you, no other member; umask 002 `$HOME`). Replacing a directory needs write access to its
parent, so a writable non-sticky ancestor would let another user swap the
plugin after export; the error names that ancestor. A symlinked plugin root
is judged by the physical location it points to. Another user who can write
there could make the segment run their code as you on every command.
Discovery, `gpy plugin validate` and export all refuse a script that fails it.

The variable names are private to GPY's integrations, like
`__gpy_plugin_segment_file_*`; plugin authors must not rely on them. `<tok>` is
the segment token. Every per-segment suffix is one word without `_`, so no
two segments, and no segment and the two globals, share a variable.

| Variable | Kind | Value |
|---|---|---|
| `__gpy_script_sh` | scalar | Absolute interpreter path, run as `sh`: `/bin/sh`, or on macOS the shell `/private/var/select/sh` links to, when it is an executable regular file owned by root or you and not writable by others (this skips the `/bin/sh` launcher). |
| `__gpy_script_<tok>_name` | scalar | The segment name. |
| `__gpy_script_<tok>_root` | scalar | Canonical plugin root: the working directory unless `pwd` is an input. |
| `__gpy_script_<tok>_command` | scalar | Canonical absolute `command` path. |
| `__gpy_script_<tok>_reqcommand` | scalar | Canonical absolute `require_command` path, or empty. |
| `__gpy_script_<tok>_inputs` | list | Declared inputs, manifest spelling (`pwd`, `is_root`, ...), manifest order. |
| `__gpy_script_<tok>_env` | list | Declared `env` names, manifest order. |
| `__gpy_script_<tok>_opts` | list | `GPY_OPT_<key>=<value>` for every declared setting, override applied, sorted by key. Split at the first `=`. |
| `__gpy_script_<tok>_ttl` | scalar | Effective `ttl_secs`, decimal. |
| `__gpy_script_<tok>_timeout` | scalar | Effective `timeout_ms`, decimal. |
| `__gpy_script_<tok>_scope` | scalar | `session` or `folder`. |
| `__gpy_script_<tok>_reqfiles` | list | `require_files`. |
| `__gpy_script_<tok>_reqfolders` | list | `require_folders`. |
| `__gpy_script_<tok>_reqexts` | list | `require_extensions`, without the dot. |
| `__gpy_script_<tok>_reqenv` | list | `require_env`. |
| `__gpy_script_<tok>_key` | scalar | The static part of the cache key: `<n>:<field>` for the plugin id, the plugin version, the segment name, the `command` path and then each `opts` entry, concatenated, where `<n>` is the field's length in UTF-8 bytes. |
| `__gpy_script_segments` | list | Tokens of the written segments, assigned after their variables. |

Each segment's variables are written in table order, then
`__gpy_script_segments`; `__gpy_script_sh` comes first. Both globals are
assigned by every export, with or without script segments. A list is a Bash
indexed array (`name=("a" "b")`), a Zsh array (`typeset -g -a name=("a" "b")`)
or a Fish list (`set -g name "a" "b"`); an empty list has no elements. Every
value is double-quoted, with `\`, `$` and `"` (and the backtick in Bash and
Zsh) escaped once, so sourcing it never expands anything. Values hold no
control characters: export refuses a path, version or setting value with one.

An integration unsets `__gpy_script_segments` before it first sources the
export, reads per-segment variables only for tokens in that list, and checks
each name it builds against `^[A-Za-z_][A-Za-z0-9_]*$` before indirection. A
segment dropped by a later export keeps its old `__gpy_script_<tok>_*`
variables until the shell exits; it is no longer in the list, so nothing reads
them. Before each export is sourced, the integration unsets the style
variables (`__gpy_segment_<tok>_*`, `__color_<tok>_*`, `__icon_<tok>` and
`__icon_<tok>_*`) of the segments listed by the previous export, so a style
field the new theme drops does not linger.

### Trust

A script runs as you, once per command that passes its triggers, including in
untrusted directories whose names and files it receives through `pwd`.
Install a script plugin only if you would trust it as a shell startup file.

Containment is enforced at export, not at run time. Export runs the script
path checks (canonical path inside the canonical plugin root, regular file)
and the ownership rule: the script and every directory from it up to the
plugin root must be owned by you or root and not be group- or
world-writable, and no canonical ancestor of the root may be replaceable by
another user. The shells then run the exported absolute path as is.
Swapping the file or a directory on its path after that needs your own write
access (or write access to an ancestor, which the rule excludes), so a
run-time re-check would only guard against code that already
runs as you. See [SECURITY.md](../../SECURITY.md#script-segment-trust-model) for the full
model.

### Runner

Bash runs `bash/core/g_runner.bash` inside `$( )` in the interactive shell.
Zsh and Fish run a byte-identical copy in one Bash child per prompt: the first
Bash 4 or later on `PATH`, else `/bin/bash`, resolved once per shell. With no
Bash at all, script segments are disabled (one warning, shown only with
`GPY_DEBUG`/`GPY_VERBOSE`).

- **Once per command.** Triggers are evaluated in this order before the
  script: `require_env` (set and non-empty, no fork), then
  `require_files`/`require_folders`/`require_extensions` in one scan child,
  then the cache, then `require_command` in the run itself. A repaint (Fish
  SIGURG, Zsh `reset-prompt`, a Bash redraw) reuses the stored text.
- **Trigger scan.** Names are matched literally, never as globs. Files and
  folders are looked up in the working directory and its parents through
  `../` paths, so the search follows physical parents (not a symlinked
  `$PWD`'s logical ones); on Linux it stops at the mount holding the
  directory. Extensions must match a regular file in the working directory.
  The scan child has 30 ms, then SIGKILL.
- **Cache in shell variables.** Keyed by `__gpy_script_<tok>_key` plus the
  current values of the declared inputs and `env` names, expiring after
  `ttl_secs` (`0` = no cache). Ages come from a clock in the same shell:
  `EPOCHSECONDS` or `printf %(%s)T` in Bash, `$SECONDS` in Bash 3.2 to 4.1;
  `EPOCHSECONDS` in Zsh; the mtime of `/` plus its age in Fish. No file is
  ever written.
- **Output.** Capped at 4096 bytes, C0/DEL stripped independently of the
  locale, hidden on a C1 control, escaped for the shell, then stored. Raw
  output is never kept.
- **Ctrl-C** ends a run at once with a Bash 4 or later runner. Fish keeps the
  terminal as the runner child's stdin, so the child is the foreground group
  and gets the INT (the scripts still get `/dev/null`). With only `/bin/bash`
  3.2 (stock macOS), Ctrl-C ends the run at the script's timeout; the run
  still ends and the group is killed. Ctrl-C never counts toward the breaker.

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
- `username` and `hostname` are the exception: they are real script plugins
  (see [Bundled Plugins](#bundled-plugins)) and no longer part of `gpy-core`

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

## Example Plugins

[`docs/dev/examples/plugins/`](examples/plugins/) holds script plugins you can
copy into `~/.config/gpy/plugins/`; each passes `gpy plugin validate`:

| Plugin | Segment | Teaches |
|---|---|---|
| [`k8s-context`](examples/plugins/k8s-context/) | `k8s` | A segment that is not built in. `env = ["KUBECONFIG"]` and no `pwd`, so the result is cached once per session; reads the kubeconfig instead of running `kubectl`; prints the variant `prod`. |
| [`username-example`](examples/plugins/username-example/) | `whoami-user` | The `is_root` input and the `root` variant. |
| [`hostname-example`](examples/plugins/hostname-example/) | `host-name` | A typed setting (`trim_at` as `GPY_OPT_trim_at`) and the `is_ssh` input. |

All use a `ttl_secs` of 5. A long TTL hides changes (a context switch shows up
only after the TTL), so raise it only for a value that really is static.

## Authoring Script Plugins

`gpy plugin new <id> [--segment <segment>]` writes `plugin.toml`,
`segments/<segment>.toml` and `bin/<segment>.sh`. The script is POSIX `sh`: it
declares what it reads in `inputs` and `env`, starts no background jobs,
prints at most 4 KiB and exits fast.

`gpy plugin validate <path-or-id>`:

- parses each segment manifest against the published schema and checks that
  `command` and `require_command` stay inside the plugin root;
- lints each script with `shellcheck -s sh`, else `dash -n`; macOS `sh -n` is
  not used because it is Bash 3.2 and accepts bashisms. A failure fails the
  validation. When neither tool is installed, a warning says the script was
  not linted;
- prints each segment's description, command, inputs, env, cache scope and
  times;
- warns about `$VAR` uses the manifest does not provide. The scan is
  heuristic (quoting is tracked, `eval` and indirect expansion are not) and
  never fails validation.

`gpy plugin list` shows each script segment's cache scope, `ttl_secs`,
`timeout_ms` and description. `gpy doctor` runs each enabled script segment
once with the runner's rules (clean environment, working directory, the
`timeout_ms` kill) and prints the run time and result; it does not evaluate
the `require_*` triggers or use the cache.

## Validation Workflow

Use CLI checks during authoring:
- `gpy plugin validate <plugin-path-or-id>`
- `gpy theme validate [theme-name-or-path]`
- `gpy doctor` (times enabled script segments)

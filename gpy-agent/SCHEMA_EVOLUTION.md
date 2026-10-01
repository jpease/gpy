# GPY Agent Schema Evolution

This document tracks backward-compatibility of GPY's public schemas: the agent
config file (`~/.config/gpy/config.toml`), the IPC request/response protocol,
and the CLI output formats.

---

## Versioning Rules

| Change type | Version component | Examples |
|---|---|---|
| **MAJOR** | `X`.y.z | Field type change, field rename, field removal, IPC enum variant removed, schema version bump |
| **MINOR** | x.`Y`.z | New optional config field with a serde default, new IPC operation, new CLI subcommand, new optional response field |
| **PATCH** | x.y.`Z` | Bug fix that does not alter schema shape, documentation update, internal refactor |

### Backward-compatibility guarantee

* **MINOR additions** — older config files deserialize unchanged; the new field
  takes its serde default. Older agent binaries ignore unknown JSON fields in IPC
  responses (unrecognised keys are silently dropped by serde's `deny_unknown_fields`
  opt-in, which GPY does **not** use globally).
* **MAJOR changes** — require a schema-version bump in `gpy-agent/src/ipc/protocol.rs`
  and corresponding Fish-side negotiation.

---

## Schema Change Log

Changes are listed newest-first within each release group.

---

### IPC protocol — single SIGURG doorbell for shell notifications (BREAKING, #674)

| Property | Value |
|---|---|
| Scope | Agent → shell notification channel (`ClientDirectory::notify_repaint*`, `notify_reload`, restart nudge) |
| Change type | BREAKING — `PROTOCOL_VERSION` 1 → 2 |
| IPC protocol | Message/Response enums unchanged; the signal channel changed. The agent now sends only SIGURG (default disposition: ignore) and carries meaning in empty flag files under `<runtime_root>/shells/`: `<pid>.reload` (reload theme/config) and `<pid>.reregister` (forget registration and register again), each written before the signal. The old runtime-root restart marker file is no longer written. |
| Config schema | No change |

**What changed.** A shell that `exec`s itself keeps its PID and stays
registered, so the agent's former per-purpose signals (all default-terminate)
could kill it before it installed handlers. SIGURG is ignored by default, so an
unprepared shell is unaffected; shells built for protocol 1 no longer receive
repaint/reload notifications and must be upgraded together with the agent.

---

### Config schema — bounded supervisor ints (BREAKING, #597)

| Property | Value |
|---|---|
| Scope | `AgentSettings.supervisor.check_interval_seconds`, `.max_restart_attempts` |
| Change type | BREAKING — field type change (raw `u64`/`u32` → bounded newtypes `types::SupervisorCheckInterval`/`types::SupervisorMaxRestartAttempts`), plus a narrowed accepted-value set |
| IPC protocol | Unchanged |
| Config schema | A previously-accepted value is now rejected: `check_interval_seconds = 0` or `max_restart_attempts = 0` used to load successfully (silently coerced to the default by `apply_defaults`'s `use_if_zero` hack) and now fails config load with a validation error. Every other value in each field's now-enforced range (5..=3600 seconds; 1..=100 attempts) round-trips unchanged — this range already matched what `validate_config`'s separate `validate_supervisor_interval`/`validate_supervisor_attempts` checks enforced, so only the `0` sentinel and any value already outside that range change behavior. |

**What changed.** These two fields were raw, unvalidated integers with a
loader-level special case: an explicit `0` was silently replaced with the
real default before validation ran, while any other out-of-range value
(`1`..`4` for the interval, for instance) reached `validate_config` and
hard-failed. Two invalid values, handled inconsistently. They are now
bounded newtypes that validate at deserialize time via
`#[serde(try_from = "...")]`, matching every other bounded config field
(`AgentTimeout`, `GitTimeout`, `CacheTtlHours`, etc.) — none of which treat a
magic sentinel value as "use the default" instead of erroring. The
loader-level zero-coercion and the separate `validate_config` range checks
were both removed as redundant with the newtype's own validation.

**Backward compatibility.** A config file that never set these keys (relying
on their defaults) is unaffected. A config file that set them to any value
in the existing valid range (5..=3600 / 1..=100) is unaffected. A config
file with `check_interval_seconds = 0` or `max_restart_attempts = 0` — which
used to load silently — now fails to load with a clear error naming the
field and its valid range; such a file must be edited to use a real value.
No migration tooling is provided for this narrow case.

---

### [Unreleased] — MINOR additions + BREAKING shell-contract change

#### IPC protocol — `virtual_env` field on `LanguageDetect` (MINOR)

| Property | Value |
|---|---|
| Scope | `Message::LanguageDetect`, `ShellIpcMessage` (`{"op":"lang",...}` shorthand), fish/bash/zsh `lang` payload builders |
| Change type | MINOR — new optional `virtual_env: Option<String>`, `#[serde(default, skip_serializing_if = "Option::is_none")]` |
| IPC protocol | Additive — older clients that never send it deserialize unchanged (`None`); agents ignore it when absent. `PROTOCOL_VERSION` unchanged (1 at the time) |
| Config schema | No change |

**What changed.** The agent daemon never inherits an interactive shell's activated
`VIRTUAL_ENV`, so the Python detector's bare `python --version` resolved the daemon's
*global* interpreter rather than the project's venv. Detection now prefers, in order: a
forwarded `$VIRTUAL_ENV` (new field), then a project-local `.venv`/`venv` directory, then
the bare `python` fallback (unchanged). A located venv's version is read from `pyvenv.cfg`
(no subprocess), falling back to invoking the venv's interpreter. The filesystem `.venv`
detection needs no shell change and applies to every render path; the forwarded field only
enriches venvs activated outside the project dir or under a non-standard name. To keep the
background refresh (which has no IPC request context) consistent with the synchronous reply,
the handler stashes the forwarded venv per repo root (`language/venv.rs`). Threaded through
fish/bash/zsh `lang` payload builders, mirroring the existing `prev_bg` plumbing.

#### IPC protocol — `is_first` field (MINOR)

| Property | Value |
|---|---|
| Scope | `Message::RepositoryStatus`, `Message::LanguageDetect`, `Message::DirectoryRequest`, `Message::DurationRequest`, `RequestMeta`, `RenderContext`'s segment position (originally `RenderContext::with_is_first`; folded into `formatter::SegmentPosition` by #586), `$sep_open` template variable |
| Change type | MINOR — new field, `#[serde(default)]`, all four segment-render `Message` variants |
| IPC protocol | Additive — new optional `is_first: bool` field alongside the existing `is_last`; older clients/agents that never send it are unaffected (defaults to `false`, i.e. today's behavior) |
| Config schema | Additive (theme files only — see below) |

**What changed.** Mirrors the existing `is_last`/`$sep_close` mechanism for the *opening* edge
of the segment chain instead of the closing one: a new `formatter::IsFirst` newtype (parallel
to `IsLast`), a `$sep_open` template variable resolved by the `duration`/`directory`/`git`/
`language` resolvers (`None` when first — suppressing the opening powerline cap — a glyph
otherwise), and `is_first: bool` on the four `Message` variants whose theme formats can use
`$sep_open`. `Message::CharacterRequest`/`HostnameRequest`/`UsernameRequest` do not gain the
field — their formats have no unconditional opening-cap literal to make conditional. Whichever
segment ends up first in the shell's enabled-and-rendering chain is determined purely by
position (mirroring `fish_prompt.fish`'s `current_idx -eq 1` check for `is_last`), never by
segment identity — fixes the wizard/theme inconsistency where disabling the `clock` segment
left the new first segment (e.g. `duration`) still rendering an opening cap that only made
sense when something preceded it.

Threaded through Fish, Bash, and Zsh (mirroring each shell's existing `is_last` plumbing) and
into the wizard's live preview (`commands/wizard/preview.rs`), which also gained a fixed-time
demo render for the `clock` segment (previously unrendered in the preview, since clock has no
agent-side template).

**#401 follow-up (resolved).** The on-disk instant-prompt cache used to serve `git`/`language`
segments instantly on shell startup (`cache/instant_prompt.rs`, `fish/core/ipc.fish`'s
`__gpy_read_instant_cache`) originally only had `git`/`git_last`/`lang`/`lang_last` cache
variants — no `is_first`-aware ones. `write_git`/`write_language_variants` now render and write
all four `is_last`×`is_first` combinations unconditionally (`git`, `git_last`, `git_first`,
`git_first_last`; same for `lang`), and the Fish/Bash/Zsh readers (`__gpy_cache_variant_suffix`,
mirroring the Rust `variant_suffix` helper) pick the file matching the segment's actual
position, so a `git`/`language` segment that is first renders correctly from a cold cache
instead of self-correcting after the first live repaint. The `gpy-agent oneshot` subcommands
also gained a `--first` flag (mirroring `--not-last`), threaded through the Fish/Bash/Zsh
`__gpy_oneshot_fallback`/`__gpy_fallback_oneshot` daemon-unreachable path, alongside a matching
fix for a pre-existing gap where the Bash/Zsh `git`/`language` segments never threaded
`is_last`/`is_first` into `__gpy_request` at all (the Fish implementation already did).

#### Theme schema — `$sep_open` in default theme format strings (MINOR)

| Property | Value |
|---|---|
| Scope | `config/themes/default.toml`: `segments.duration.format`, `segments.directory.format`, `segments.git.format`, `segments.language.format` |
| Change type | MINOR — format-string content change, no schema shape change |
| Config schema | Unchanged — `format` was already a free-form template string |

**What changed.** Replaced the unconditional opening-cap glyph `[](fg:$bg bg:default)` with
`([$sep_open](fg:$bg bg:default))` in the four built-in pill-style segments. A custom theme
with its own `format` string is unaffected unless it opts into `$sep_open`. `starship.toml`
and `text.toml` don't use the powerline pill-cap style at all and are untouched.

#### Theme schema — git content-visibility flags (MINOR, #410)

| Property | Value |
|---|---|
| Scope | `theme::model::GitTheme.show_branch`, `.show_ahead_behind`, `.show_stash`; `[segments.git]` TOML keys of the same names; `GitResolver`'s `branch`/`ahead_behind`/`stash` variable resolvers |
| Change type | MINOR — three new optional `Option<bool>` fields, each `#[serde(default, skip_serializing_if = "Option::is_none")]` |
| IPC protocol | Unchanged |
| Config schema | Additive (theme files only) — a theme predating #410 omits all three; each deserializes as `None`, which the resolver treats as `true` (the prior always-on behavior), so no existing theme changes output |

**What changed.** `GitTheme` previously had only one content toggle (`show_counts`); branch
name, ahead/behind arrows, and the stash indicator were rendered unconditionally. Each now has
an independent `Option<bool>` flag honored by the corresponding `GitResolver` variable
(returning `None` — which collapses the format's optional `(…)` group — when the flag is
`Some(false)`). The config wizard exposes these as a per-segment multi-select (`b`/`a`/`t` keys
while the `git` segment is highlighted), persisted through the per-field theme-override save
mechanism (#407). Unset defaults to on, so this is purely additive.

#### Config schema — removed dead `ui.delimiter_style`/`ui.delimiters` (BREAKING, #377)

| Property | Value |
|---|---|
| Scope | `UiSettings.delimiter_style`, `UiSettings.delimiters`, `config::DelimiterPreset`, `config::DelimiterSettings`, `config::DelimiterStyle`, `ui.delimiter_style`/`ui.delimiters.*` `gpy config get/set/show` keys |
| Change type | BREAKING — config schema fields removed |
| IPC protocol | Unchanged |
| Config schema | Removal — a `config.toml` with `[ui]` `delimiter_style` or a `[ui.delimiters]` table now fails validation on unknown-field strictness (if enabled) or is silently ignored (fields are simply no longer deserialized into `UiSettings`) |

**What changed.** `ui.delimiter_style` (`DelimiterPreset`: theme/ascii/nerd/powerline/none) and
`[ui.delimiters]` (`DelimiterSettings`: per-preset `first`/`start`/`end`/`last` glyph overrides)
round-tripped through `gpy config get/set/show` but were never read anywhere in the render
path — the actual rendered delimiters (prompt/segment open/close icons and colors) always
came from the theme file's `[ui.prompt_open]`/`[ui.prompt_close]`/`[ui.segment_open]`/
`[ui.segment_close]` (`theme::model::DelimiterConfig`, a separate, still-supported type),
wired through `theme::export`. Rather than wire a second, config.toml-level override path for
a purely visual/style concern — themes already own colors, icons, and other visual
properties, and `config.toml` has no other precedent for a config.toml-level style override
that competes with the theme file for the same rendered value — the dead schema was removed
outright. To customize delimiters, edit or create a theme file (`gpy theme new`) instead.
Follow-up: cloning an existing theme's values into a new theme (rather than only the blank
default template) is tracked separately.

#### IPC protocol — `username` request/response (MINOR, #252)

| Property | Value |
|---|---|
| Scope | `Message::UsernameRequest`, `Response::Username`, `username` shell op |
| Change type | MINOR — new IPC operation, new enum variants, all new fields `#[serde(default)]` |
| IPC protocol | Additive — new `Message`/`Response` variants; no existing variant, field, or arm altered |
| Config schema | Additive (theme files only — see below) |

**What changed.** Added `Message::UsernameRequest { username, format, is_last, prev_bg }`
and `Response::Username { username }`, mirroring the `hostname` request path through
`ShellIpcMessage` parsing (`"username"` op), `validate_message_content`
(`validate_username_request`), the `HandlerRegistry` router, and the `fish-ansi` renderer.
Like `hostname` (#259), the client resolves the effective username (`$USER`, which is
`root` under root/sudo) and gates root/sudo visibility (`segment_username_detect`) before
sending the request; the agent does not derive the username itself and only echoes the
client-supplied value back after validating it as a bounded string (max 256 chars) with
the existing null-byte/control-character checks. Older shells and agents interoperate
unchanged: the new `username` field on `ShellIpcMessage` is `#[serde(default)]`, and
clients that never send `{"op":"username",...}` are unaffected.

#### Theme schema — username segment (MINOR, #252)

| Property | Value |
|---|---|
| Scope | `SegmentThemes.username: UsernameTheme`, `config/themes/default.toml`, `config/themes/starship.toml` |
| Change type | MINOR — new optional theme field, all sub-fields `#[serde(default)]` |
| IPC protocol | See above |
| Config schema | Additive (theme files only) |

**What changed.** Added `SegmentThemes.username: UsernameTheme` (`gpy-agent/src/config/mod.rs`),
registered alongside `hostname`/`character`/`status`. `UsernameTheme` fields: `format`
(`Option<String>`, default `None` — pure-shell fast path; `Some` selects the agent-rendered
Starship-compatible template path), `icon` (`Option<Icon>`, default `None`), `bg_color`
(`ColorSpec`, default `red` via `default_color_red()` — a root/sudo warning pill; pure-shell
path only, ignored when `format` is set), `text_color` (`ColorSpec`, default `white`), and
`show_always` (`bool`, default `false` — root/sudo only, matching Starship's `show_always`
default). Existing theme TOML deserializes unchanged; the segment is opt-in (not in any
theme's default `enabled_segments`). Exported to shells as `__color_username_bg`,
`__color_username_fg`, `__username_show_always` (`0`/`1`), `__username_format` (presence flag
only — never the raw format string, for injection/`set -u` safety), and `__icon_username`.
The Starship importer maps a `[username]` module onto `UsernameTheme` (`$user` → `$username`,
`$style` inlined from `style_root`, `show_always` passthrough).

#### IPC protocol — `hostname` request/response (MINOR, #259)

| Property | Value |
|---|---|
| Scope | `Message::HostnameRequest`, `Response::Hostname`, `hostname` shell op |
| Change type | MINOR — new IPC operation, new enum variants, all new fields `#[serde(default)]` |
| IPC protocol | Additive — new `Message`/`Response` variants; no existing variant, field, or arm altered |
| Config schema | Unchanged |

**What changed.** Added `Message::HostnameRequest { hostname, format, is_last, prev_bg }`
and `Response::Hostname { hostname }`, mirroring the existing `character` request path
through `ShellIpcMessage` parsing (`"hostname"` op), `validate_message_content`, the
`HandlerRegistry` router, and `EndpointHandle::route_request_secure` dispatch. Per #259,
the client resolves the hostname string and gates SSH-only visibility before sending the
request; the agent does not derive the hostname itself and there is no `is_ssh` field
(this supersedes that part of the original design doc). The agent only echoes the
client-supplied hostname back after validating it as a bounded string (max 256 chars,
same limit as `key`) with the existing null-byte/control-character checks. Older shells
and agents interoperate unchanged: the new `hostname` field on `ShellIpcMessage` is
`#[serde(default)]`, and clients that never send `{"op":"hostname",...}` are unaffected.
Nothing renders the hostname segment yet — the resolver/renderer lands in a follow-up
change.

#### Theme schema — hostname segment (MINOR, #257/#258)

| Property | Value |
|---|---|
| Scope | `SegmentThemes.hostname: HostnameTheme`, `config/themes/default.toml` |
| Change type | MINOR — new optional theme field, all sub-fields `#[serde(default)]` |
| IPC protocol | Unchanged |
| Config schema | Additive (theme files only) |

**What changed.** Added `SegmentThemes.hostname: HostnameTheme` (`gpy-agent/src/config/mod.rs`),
registered alongside `clock`/`character`/`status`. `HostnameTheme` fields: `format`
(`Option<String>`, default `None` — pure-fish fast path; `Some` selects the agent-rendered
Starship-compatible template path), `icon` (`Option<Icon>`, default `None`, matching
Starship's no-icon default), `bg_color`/`text_color` (`ColorSpec`, defaults `black`/`white`,
pure-fish path only — ignored when `format` is set), `trim_at` (`String`, default `"."` via
`default_hostname_trim_at()` in `gpy-agent/src/config/defaults.rs` — trims the hostname at the
first delimiter match, e.g. FQDN → short name; empty string disables trimming, applies to both
render paths), and `show_always` (`bool`, default `false` — SSH-only visibility, matching
Starship's `ssh_only = true` default). `config/themes/default.toml` gained a `[segments.hostname]`
block (`bg_color = "black"`, `text_color = "white"`) so a user who opts in gets sensible pill
colors; `hostname` was **not** added to any theme's default `enabled_segments` — the segment
remains opt-in only. Older theme files deserialize unchanged: an absent `[segments.hostname]`
table falls back to `HostnameTheme::default()`, which is behaviorally identical to the segment
being invisible (gated by `enabled_segments` membership, not this struct). No IPC or shell-contract
change from this entry — see the IPC entry above for the wire-protocol addition.

#### Theme schema — per-language style attributes (MINOR, #231)

| Field | `[segments.language] <lang>_style` (string, e.g. `java_style = "dimmed"`) |
| Change type | MINOR — new optional field, serde default (empty map → `get_style` returns `"bold"`) |

Older theme files deserialize unchanged: with no `<lang>_style` keys the `styles`
map is empty and every language keeps the previously hardcoded `bold` attribute.

---

#### SP4 — base16/base24 palette color schemes (MINOR, #230)

| Property | Value |
|---|---|
| Scope | `[ui.recommended].palette`, new color roles, builtin palettes, `gpy palette import` |
| Change type | MINOR — all additions are optional with serde defaults; no existing field altered |
| IPC protocol | Unchanged |
| Config schema | Additive only |

**What changed.**

- **New optional config field:** `[ui.recommended].palette` (string, serde default `""`).
  Themes may recommend a paired palette by setting this field; it is applied when the
  user runs `gpy theme use <name> --force`. Existing theme files that omit this field
  deserialize unchanged; the empty default is treated as "no recommendation".

- **New standard color roles:** `orange` (base09) and `brown` (base0F). Both are
  added to `config/palettes/default.toml` as identity values. Templates and palette
  files that reference these names now resolve correctly. Existing themes do not
  reference these roles, so palettes that omit them are unaffected; only the
  starship theme references them, and it ships paired with palettes that define them.

- **New builtin palettes:** `starship`, `catppuccin-latte`, `catppuccin-frappe`,
  `catppuccin-macchiato`, `catppuccin-mocha`, `nord`, `gruvbox-dark-medium`.
  Registered in `PaletteManager::insert_builtin_palettes`. Selecting any of these
  via `ui.palette` requires no user-created file.

- **New CLI command:** `gpy palette import <scheme.yaml>` converts a
  base16/base24 tinted-theming scheme into a GPY palette TOML and writes it to
  `~/.config/gpy/palettes/<name>.toml`. Flags: `--name`, `--force`. No IPC
  change; purely a CLI-layer addition.

**Backward compatibility.** Existing config files deserialize without change.
Existing themes reference only standard ANSI names, so palettes that omit
`orange`/`brown` are unaffected; a theme that references a role absent from the
active palette renders that segment empty (a palette-name miss is a hard error,
not an ANSI fallback). The IPC protocol is unchanged. No migration is required.

---

#### Theme schema — Starship-parity git/language fields (MINOR)

| Property | Value |
|---|---|
| Scope | `ThemeConfig` `[segments.git]` and `[segments.language]` |
| Change type | MINOR — new optional fields, each with a serde default |
| IPC protocol | Unchanged |
| Config schema | Additive (theme files only) |

**What changed.** `[segments.git]` gains `show_counts` plus per-category status icon
overrides (`staged_icon`, `unstaged_icon`, `untracked_icon`, `conflicts_icon`).
`[segments.language]` gains `show_symbol_without_version` and `enabled_languages`. All
are `Option`/defaulted: older theme files deserialize unchanged and the defaults
preserve prior rendering (counts shown, config git icons, versionless languages hidden,
no theme-level language allow-list). The renderer reads these from the active theme and
falls back to `config.git` / `config.language`. The `starship` preset sets them to match
native Starship's `git_status` and language modules. No IPC change and no migration
required.

#### Starship importer — no schema change (PATCH, #186)

| Property | Value |
|---|---|
| Scope | New `gpy theme import` CLI command |
| Change type | PATCH — additive CLI command only; no config, IPC, or schema change |
| IPC protocol | Unchanged |
| Config schema | Unchanged |

**What changed.** The `gpy theme import <starship.toml>` command was added. It is a
CLI-only operation: it reads a `starship.toml` and emits two artifacts in existing
TOML formats — a palette file (`PaletteConfig`, established in SP1 #198) and a theme
file (`ThemeConfig`, existing format). No new config fields, IPC operations, or schema
versions are introduced. No migration is required.

#### Shell-contract: retired `__color_*` exports + `__gpy_*_format` toggles (BREAKING, #199)

| Property | Value |
|---|---|
| Scope | `gpy-agent theme export` output (Fish/Zsh/Bash) |
| Change type | BREAKING — shell-contract change (pre-production; no compat shim per SP2 epic) |
| IPC protocol | Unchanged |
| Config schema | Unchanged |

**What changed.** For the five segments that now have a confirmed agent render path
(`directory`, `duration`, `character`, `git`, `language`), `gpy-agent theme export`
no longer emits their per-segment `__color_*` shell variables or the
`__gpy_*_format` toggle variables. These were consumed by the now-deleted
shell-side colour renderers. Removed exports:

- `__color_directory_bg`, `__color_directory_fg`
- `__color_duration_bg`, `__color_duration_fg`
- `__color_git_bg`, `__color_git_fg`, and per-element git colour variables
- `__color_language_bg`, `__color_language_fg`, and per-language colour overrides
- `__gpy_directory_format`, `__gpy_duration_format`, `__gpy_character_format`

**What is NOT changed.** The following exports are unchanged and continue to be emitted:

- `clock`: `__color_clock_bg`, `__color_clock_fg` (clock remains shell-rendered)
- `status`: `__color_status_ok_bg/fg`, `__color_status_fail_bg/fg` (status remains shell-rendered)
- Plugin/custom segments: `__gpy_segment_<name>_*`, `__color_<name>_bg/fg`
- Delimiter/powerline colours: `__segment_delimiter_color/bg`, `__prompt_open/close_color/bg`, `__prompt_base_bg/fg`
- All icon exports (`__icon_*`)
- All layout/threshold exports (`__duration_threshold_ms`, `__time_format`, etc.)

**`ColorSpec` segment fields** (e.g., `git.bg_color`, `directory.bg_color`) were
**not** removed from the config schema or Rust structs. They still feed the
resolvers' style derivation and are consumed by the kept shell-side segments.
The `GitTextElement` struct and per-element git colours are retained for the
upcoming SP3 theme-importer (#186).

**Backward compatibility.** This change affects the shell-side theme export contract
only. The IPC protocol is unchanged. Config files deserialize without error.
Shells referencing the removed variables will silently receive empty/unset values;
remove those references.

#### Palette rendering activation — behavior change (PATCH, #199)

| Property | Value |
|---|---|
| Scope | Agent rendering behavior; no schema shape change |
| Change type | PATCH — behavior change only (schema unchanged since #198 added `ui.palette`) |
| IPC protocol | Unchanged |

**What changed.** `config.ui.palette` now actively affects rendering.
`Color::Named` in template expressions resolves through the active palette first
(Starship semantics) and falls back to the standard ANSI name on a miss.
The instant-prompt cache renders with the active palette; a `ui.palette` change
invalidates and refreshes the cache identically to a theme change.
Template colour validation also resolves through the active palette and rejects
colours absent from both palette and the standard ANSI name set.

The config schema is unchanged from SP1 (#198). No migration required.

---

#### `[ui] palette` field — config schema (MINOR, #198)

| Property | Value |
|---|---|
| Field path | `[ui].palette` |
| Rust type | `PaletteName` (newtype over `String`) |
| TOML type | string |
| Serde default | `"default"` (via `PaletteName::default()`) |
| Validation | Non-empty string; accepted values are palette names discoverable via `gpy palette list` |

**What changed.** The `[ui]` table in `config.toml` gained an optional `palette`
string field that selects the active named-color palette. When absent the field
defaults to `"default"`, which maps to the bundled identity palette
(`config/palettes/default.toml`).

**Backward compatibility.** Existing config files that omit `[ui].palette`
deserialize without error; `serde(default)` supplies `PaletteName::default()`
(`"default"`). No migration is required.

**Example config addition:**

```toml
[ui]
palette = "nord"   # select a user palette at ~/.config/gpy/palettes/nord.toml
```

**Builtin `default` palette.** The bundled palette is an identity map of the
16 standard ANSI color names. Color values use the validator-accepted ANSI
spellings — `magenta`, `bright_black`, `bright_red`, etc. (underscores, not
hyphens; no `purple` alias).

**New CLI commands.** Alongside the config field, the `gpy palette` subcommand
was added to the `gpy` and `gpy-agent` binaries:

| Command | Description |
|---|---|
| `gpy palette list` | List discovered palettes (builtin + user) |
| `gpy palette show` | Print the active palette name |
| `gpy palette use <name>` | Write `ui.palette = "<name>"` into config |
| `gpy palette validate [target]` | Validate a palette (active if omitted, else name or file path) |

**Doctor integration.** `gpy doctor` now reports the active palette and warns
when `ui.palette` refers to a palette that cannot be found.

---

## IPC Protocol

The IPC schema (`gpy-agent/schemas/message.json`, `response.json`) is versioned
separately via `protocol_version` in `gpy-agent/src/ipc/protocol.rs`. The
`ui.palette` addition does not alter the IPC protocol; palette selection is
resolved inside the agent at startup and exposed through the `theme export`
response, which already carries named-color context.

---

## Required Checks Before a Schema Change

Before merging any schema change, verify:

```bash
# Full quality gate — must be green
./scripts/quality-check.sh

# Individual schema-focused tests
(cd gpy-agent && cargo nextest run --test config_tests)
(cd gpy-agent && cargo nextest run schema)
(cd gpy-agent && cargo test --doc)
```

### Schema Change Checklist

- [ ] Identify the change type (patch/minor/major)
- [ ] Update the schema tests if needed
- [ ] Run the full compatibility suite
- [ ] Update this document
- [ ] Bump the version per the semver rules above
- [ ] Record the compatibility notes in `CHANGELOG.md`

A breaking schema change requires a major version bump.

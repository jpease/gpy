# ADR-0012: Theme Overlays

## Status

Proposed (2026-10). Needs maintainer approval: it alters the structure of the theme file schema, one of the changes the project's agent rules reserve for explicit approval ("Altering configuration schema structure"). Nothing here is implemented; the follow-up items at the end are separate work.

## Context

A theme name resolves to one file and nothing else is merged. `ThemeManager::resolve_path` (`gpy-agent/src/theme/manager.rs:626-647`) returns `<config>/themes/<name>.toml` when that joined path exists, else the winning plugin or builtin entry from `discover_available_themes` (`manager.rs:535-545`). Precedence is user > plugin > builtin (`config/discovery.rs:21-29`). Ties go to the later candidate (`discovery.rs:88-96`), and plugins are inserted sorted by id (`plugin/registry.rs:303-304`), so among plugins the lexically greatest id wins. `config::loader::load_theme_from_path` (`config/loader.rs:336-370`) reads that file, or the embedded builtin with the same stem. `theme::parse` (`theme/parse.rs:8-39`) parses the string, requires `[segments]` and `[ui]` and deserializes the whole `ThemeConfig`. On failure it re-parses the string for spans and runs #1084's per-key check on plugin tables (`parse.rs:41-52`, `theme/locate.rs`).

So a user file named like a builtin replaces the builtin outright, and every user customisation is a fork. Ways forks are created today:

- The wizard: `wizard::save` calls `ThemeManager::save_user_theme(selected_theme, pending_theme)` (`commands/wizard/save.rs:38-46`, `manager.rs:594-618`). That serializes the whole `ThemeConfig` with `toml::to_string_pretty`, defaults included and comments lost. The wizard edits six keys (`commands/wizard/state/mod.rs:336-364`): `segments.clock.time_format` and `show_seconds`, `segments.duration.show_milliseconds`, `segments.git.show_branch`, `show_ahead_behind` and `show_stash`.
- `gpy theme new <name> [--from <base>]` copies `DEFAULT_THEME_CONTENT` or the base's raw source (`commands/theme.rs:221-255`, `ThemeManager::theme_source_content`).
- `gpy theme save [name]` writes the active theme's raw source (`theme.rs:263-312`). With a builtin active, that is a byte-identical copy of the builtin. With a user file active and no name, it writes that file's own content back over it.
- `gpy theme import` writes a full theme built from the builtin `starship` preset (`import/starship/emit.rs:171-185`).
- Hand edits and copies from older installers.

The failure that started epic #1083: after 0.6.0 → 0.7.1, `gpy theme use starship` failed on a July `starship.toml` copy. The copy still set `format` and `show_always` on `username` and `hostname`, which #1003 turned into plugin segments. Plugin segment tables accept only style keys (`SCHEMA_EVOLUTION.md`, "Compatibility guarantees"; `theme/model/mod.rs:49-50,74-89`), so the stale keys are now errors. The rest of the copy (about 200 lines) also stopped receiving every later fix to the builtin.

Sibling items of the same epic have shipped and are listed under `[Unreleased]` in `CHANGELOG.md`. They treat the symptoms:

- #1084: parse errors name the offending key.
- #1085/#1086: load and doctor hints for shadowing copies.
- #1088: retire copies byte-identical to a released builtin.
- #1089: `gpy theme diff|reset|migrate`.

The shared helpers are in `theme/user_copy.rs`. This ADR addresses the cause: customising a theme should record only what the user changed.

### Prior art

Helix themes ([docs](https://docs.helix-editor.com/themes.html#inheritance), [loader](https://github.com/helix-editor/helix/blob/master/helix-view/src/theme.rs), read 2026-10-27): a theme sets `inherits = "<name>"` plus the keys it overrides. The loader merges the parent's `toml::Value` with the child's (top level at depth 1, `[palette]` at depth 2). A file may inherit from a same-named file in a lower-priority directory, and cycles are detected with a set of visited paths. There is no way to remove an inherited key.

TypeScript `tsconfig.json` `extends` ([docs](https://www.typescriptlang.org/tsconfig/extends.html), read 2026-10-27): the base is loaded, then overridden by the inheriting file. `files`, `include` and `exclude` from the inheriting file overwrite the base's, and circular `extends` is not allowed.

## Decision

Proposed: **overlay themes**. A user theme may name a base with `extends` and contain only the keys it changes. The loader merges it over the base before the existing typed step. The wizard, `theme new` and `theme save` write overlays. Existing full copies keep working unchanged and are converted only on request.

### 1. Format and location

An overlay is an ordinary theme file in the same places as today (`<config>/themes/<name>.toml`, or a plugin's `themes/` directory, section 3), with a top-level `extends` key:

```toml
# ~/.config/gpy/themes/starship.toml
extends = "starship"
unset = ["segments.character.format"]   # the shell draws the character locally

[segments.clock]
time_format = "24"
```

- **Told apart by `extends` alone.** A file with a top-level `extends` string is an overlay. A file without it is a full theme with today's rules (`[segments]` and `[ui]` required). No file extension, directory or header comment carries meaning, so `gpy theme list`, completion, `use` and `validate` keep one namespace. A non-string `extends` is an error.
- **`unset`** (optional, overlay only) is an array of dotted key paths to remove from the base. Each path component must match `^[A-Za-z0-9_-]+$`. `unset` is needed for three reasons:
  - Many theme fields are `Option`s whose absence means something. With `format` unset, the shell renders `clock`, `status` and `character` locally (`model/mod.rs:274-275,302,423-425`).
  - TOML has no null.
  - An empty string is a valid value (the `starship` builtin sets `icon = ""`).
- Both keys are removed before the merged table is deserialized. `unset` in a file without `extends` does nothing, and `gpy theme validate` reports it. Making it a load error would reject a file that loads today.

### 2. Merge semantics

`theme::parse` is split so the typed step takes a table:

- `parse_table(contents, source) -> toml::Table` does today's syntax parse, with its errors.
- `from_table(table, source) -> ThemeConfig` runs the `[segments]`/`[ui]` check, typed deserialization and `validation::validate_theme_config`.

A full theme still goes through `parse`, which runs both steps on one string and keeps today's span-based diagnostics unchanged. For an overlay, the loader runs `parse_table` on each file in the chain and merges in this order:

1. Resolve the base to its merged table (recursively).
2. Canonicalise the language table (below).
3. Remove each `unset` path.
4. Merge the overlay over the result.
5. Run `from_table` on the merged table.

- **Tables merge deeply at every depth.** Theme nesting is three levels deep (`[segments.clock.open]`, `[ui.recommended.directory]`, `[segments.language.symbols]`). Helix's depth-1 merge would make `[segments.clock] time_format = "24"` drop every other clock key. Inline tables are tables. When either side is not a table, the overlay value replaces the base value. A type mismatch is left to `from_table`, which reports it (below).
- **Arrays replace.** The arrays in themes are ordered lists (`ui.recommended.enabled_segments`, `segments.language.enabled_languages`). Appending would reorder or duplicate entries, and append semantics would need a second syntax for removal. tsconfig `extends` likewise overwrites `files` and `include`.
- **One spelling for `[segments.language]`.** `LanguageTheme` accepts per-language keys in two spellings and reads both into the same maps (`config/language_icons.rs:87-110`): flat (`rust_symbol`, `java_style`, `rust_bg_color`) or nested under `symbols`, `styles` and `overrides`. The `starship` builtin uses flat keys (repository `config/themes/starship.toml:157-188`). Without canonicalisation, an overlay's nested `rust_symbol` and the base's flat one would both survive, and which one wins would depend on key order.
  - Before `unset` and merge, every file's language table is rewritten to the nested form with the deserializer's own rule: `*_symbol` goes to `symbols`, `*_style` to `styles`, and any other key that is not a field goes to `overrides`. `unset` paths into the table are rewritten the same way.
  - The rule lives next to the deserializer and is tested against it.
- **Removing and replacing.** `unset` removes a leaf or a whole table. Unset-then-set replaces a table: `unset = ["segments.language.symbols"]` plus a `[segments.language.symbols]` table gives exactly the overlay's symbols, whichever spelling the base uses.
  - An `unset` path the base lacks (after canonicalisation) is ignored at load, and `gpy theme validate` warns about it. Making it an error would turn an upstream removal into the same upgrade break this ADR removes.
- **Plugin segment tables and `[ui]`** follow the same rules; there is no per-table special case. Variant styles are flat keys (`root_text_color`), so they merge per key. An overlay may style a plugin segment its base does not mention. `[ui.recommended]` is inherited, so `gpy theme use <overlay> --force` applies the base's recommendations unless the overlay changes them.
- **Chains.** `extends` may name another overlay.
  - The resolved chain is at most 4 `extends` hops. An overlay of a user overlay of a plugin overlay of a builtin needs 3; a 5th hop is an error.
  - Each visited file's canonicalised path (an embedded builtin by its name) is recorded. A repeat is a cycle, reported as an error naming the chain, e.g. `a (user) → b (user) → a (user)`.
  - Names match discovered file stems exactly (section 3). So on a case-insensitive filesystem, `extends = "Starship"` cannot reach `starship.toml` under a second path string.
  - Every hot reload re-reads the chain, which is why it is bounded.
- **Validation runs on the merged result only.** An overlay is not a theme on its own, so the existing checks run on the merged table: `[segments]`/`[ui]` present, typed deserialization, `validate_theme_config`, and template validation in `theme use`. Each file in the chain is first parsed alone for TOML syntax, with its own path and span, as today. The checks specific to overlays:
  - `extends` is a valid name (section 5).
  - `unset` paths are well-formed.
  - The chain resolves.
- **Error attribution.** The key path of a failing typed parse is not available from the error:
  - `toml::de::Error` exposes only `message()` and `span()`. Its key path is a private field, printed only when the error has no span or input (toml 1.1.8 `src/de/error.rs:5-10,63-80,159-160`).
  - A merged `toml::Table` has no source text to take spans from.

  So attribution finds the failing key structurally. It runs only when `from_table` fails, as today's re-parse does, so a successful load pays nothing for it. It works in three steps:

  1. **Find the key**, on the merged table.
     - **Plugin tables:** #1084's per-key check, changed to take a `toml::Table` instead of source text. Each key of each untyped `[segments.<id>]` table is deserialized alone into `PluginSegmentTheme` (`locate.rs:37-55`).
     - **Typed tables:** these are the seven `[segments.<builtin>]` tables (`locate.rs:12-20`) and `[ui]`. Each table is deserialized alone, then each direct key of the first failing table alone. A one-key table is valid input because the containers default every missing field: `UiTheme` and every typed segment struct are `#[serde(default)]` (`model/mod.rs`), and `LanguageTheme`'s deserializer starts from its default (`language_icons.rs:74`). The result is a path like `segments.clock.time_format`.
     - **Table-valued keys:** a failing key whose value is a table gives the path to that table. Examples: a delimiter, which requires `icon` (`config/mod.rs:317-326`); `[ui.recommended]`; the language maps.
     - **Validation errors:** a `validate_theme_config` error names a plugin segment (`config/validation/theme.rs:26-79`) and gives `segments.<id>`.
  2. **Find the file.** The merge is replayed over the per-file tables, recording the last file to write each leaf; an `unset` clears the entries under its path. A leaf path maps to one file, and a table path to every file that writes a leaf under it.
  3. **Find the line.** Each named file is re-parsed with `toml_edit::Document`, and the key's span gives line and column, as `locate::position` does (`locate.rs:57-72`). Canonical language paths are mapped back to the file's own spelling first.

  If no single key fails alone (a failure that needs two keys together), the error names the table and the chain. Format, in the repo's style:

  ```text
  Invalid theme 'starship'
    Diagnostic: segments.username.format: unknown key for a plugin segment, set at ~/.config/gpy/themes/starship.toml:7 (chain: starship (user) → starship (builtin))
    Remediation: Remove the key, or run `gpy theme migrate starship`.
  ```

  A key set by an embedded builtin is a GPY bug, and the error says so.

### 3. Precedence and resolution

- **An `extends` in a user file resolves like `gpy theme use <name>`**: user > plugin > builtin, through the same discovery, with one rule for the shadowing case: **a file never resolves to itself.**
  - When the name is the file's own name, resolution starts below the file's own source (for a user file: plugin, then builtin). So the wizard can write `starship.toml` with `extends = "starship"`, `ui.theme` stays `starship`, and the user keeps receiving the builtin. Helix allows the same.
  - If nothing lies below, the error says `extends 'x' names this theme and no plugin or builtin theme 'x' exists`.
  - Resolution compares the name with discovered file stems exactly. It never joins the name to a directory and tests `exists()`, as `resolve_path` does (`manager.rs:629-632`).
  - Resolution needs every candidate for a name, not only the winner `merge_candidate` keeps.
- **User overlays may extend user, plugin and builtin themes.** `mine.toml` with `extends = "starship"` gets the user's `starship.toml` if one exists (overlay or full), which is what `gpy theme use starship` would show.
- **Plugin overlays resolve straight to the embedded builtin.** An `extends` in a plugin theme must name a builtin and resolves to the embedded content, skipping user and plugin sources even when a file with that name exists there.
  - A plugin theme is a shipped artifact. Depending on a user file would make it render differently per user, and depending on another plugin's theme creates a dependency the manifest cannot declare (`plugin/manifest.rs` has no dependency field).
  - A non-builtin name is an error at load and from `gpy plugin validate`. `gpy plugin validate` does not check plugin theme files at all today (`commands/plugin.rs`), so this, together with the merged-result checks, is new validation work.
- **Builtins stay full themes.** They are embedded, tested by the golden tests, and the base of everything else. Making `text` an overlay of `default` is out of scope.
- **A full copy with a builtin's name keeps shadowing it**, as today. The hints from #1085 and the conversion in section 4 address it. Several plugins providing one name keep today's tie rule: the lexically greatest plugin id wins (Context).
- **Validating by path.** `gpy theme validate ./x.toml` resolves the file's `extends` as a user theme named after its stem. The exception is a file in a plugin's `themes/` directory (the directory's parent holds `plugin.toml`), which gets the plugin rule. So a plugin author's `gpy theme validate plugins/x/themes/y.toml` does not accept a non-builtin base.

### 4. Migration of existing full copies

- **No automatic conversion.** Converting can change what the user sees, because keys the copy lacks are then inherited from the base (below). Full copies keep loading as today.
- **Pristine copies: #1088.** A copy byte-identical to a released builtin carries no intent, and `gpy setup` moves it to `.bak`. That stays the first step and needs no overlay.
- **Modified copies: a new command, `gpy theme rebase <name> [--onto <base>]`.** It rewrites a full theme as an overlay of `<base>`.
  - `--onto` defaults to the same name's next lower source (the shadowing case). For other names it is required, and the command suggests the builtin with the fewest differing keys.
  - The command prints the result and a report and writes nothing unless `--write` is given. `--write` does what `migrate` does (`commands/theme_migrate.rs:79,88`): `user_copy::copy_aside` first, then `fs_util::write_atomically` over the original. A failed write leaves the copy in place, and the watcher sees one content change.
- **Comparison on normalised values.** The copy's raw text cannot be compared with the base's:
  - A wizard copy is `toml::to_string_pretty(ThemeConfig)` (`manager.rs:603`). It has every serde default, and the language maps are written as nested tables (`language_icons.rs:33-41`).
  - The builtins write the same keys flat.

  So rebase deserializes the copy and the merged base into typed `ThemeConfig`s and serializes both with the same serializer: `C = Table::try_from(copy)` and `B = Table::try_from(base)`. It then decides for each key the copy's file actually contains, mapped to its normalised path (the language rule of section 2 maps flat to nested):
  - **Equal** in C and B: dropped from the file.
  - **Absent from B and equal to the field's value when unset:** dropped. This holds for the `Option` fields read through an accessor that supplies a default: `ClockTheme::time_format()`, `show_leading_zero()` and `show_seconds()`, and `GitTheme::show_counts()`, `show_branch()`, `show_ahead_behind()` and `show_stash()` (`model/mod.rs:338-352,640-660`). They are listed once next to the model, with a test against the accessors. Fields where `None` has its own meaning (`format`) are never dropped this way.
  - **Otherwise:** kept in the file's own spelling, with its comments (`toml_edit`).

  A key with no normalised path (ignored at load today) is dropped and reported.
- **It never generates `unset` on its own.** Normalised paths where C and B differ but the copy's file sets no key are inherited after the rebase. Each such key was either added upstream after the copy was made (which the user should now receive; `ssh_icon` on `hostname` is one) or deleted by the user. The copy cannot tell them apart, and #1088 ships hashes, not old contents, so there is no common ancestor for a three-way merge. The report lists those keys. `--unset-missing` turns all of them into `unset` entries, for users who deleted keys on purpose.
- **Check before write.** The new overlay's merged result must load, and `Table::try_from` of it must equal C except at the reported inherited paths. Otherwise nothing is written. A copy that does not load (stale keys, the #1003 case) is refused with a hint to run `gpy theme migrate <name>` (#1089) first: migrate repairs keys, rebase removes duplication.
- **#1089 on overlays.**
  - `gpy theme diff <name>` shows an overlay against its base's merged result. `reset` moves an overlay aside like a full copy. Both accept any user overlay, not only the builtin names they accept today (`commands/theme_override.rs:73-78`).
  - `migrate` rules are key rewrites on a raw table, and they also rewrite `unset`. A `Rename` rewrites a matching `unset` path (`hostname.icon` → `ssh_icon`, `commands/theme_migrate_rules.rs:84-89`), and the `unset` entry for a removed or moved key is dropped. Left alone, the old path would be ignored at load and the user's intent lost.
  - `user_copy::recovery_hint` adds `gpy theme rebase` once it exists.
- **Overlays reduce, not remove, upgrade breaks.** An overlay that sets a key a later release rejects still fails, as the July copy would have if it had held only those keys. But it fails only on keys the user actually wrote, the error gives the file and line (section 2), and `migrate` covers BREAKING renames.

### 5. Hot reload, export, import, writers, schema and security

- **Hot reload.**
  - `ThemeLocation` (`manager.rs:30-60`) becomes the resolved chain, with one entry per file (path, source, on disk).
  - The fingerprint covers every link's identity and bytes. Editing the base or the overlay is a `Reloaded::Content`. A change in the chain (an overlay appears, a base moves to another source) is a `Reloaded::Source` with a re-arm.
  - `watch_paths` is the union over the chain. The user themes directory is already always watched (`manager.rs:417-429`), so the new case is a user overlay of a plugin theme, which adds that plugin's `themes/` directory and file.
  - The poll fallback re-resolves on every tick (`manager.rs:670-683`) and checks one file through the shared `PollTarget::path_of` (`watcher/hot_reload.rs:53-66`), which `config/manager.rs:346-354` also uses. That shared type changes: `path_of` returns a list of paths, fingerprinted together, and the config manager returns a list of one.
  - An embedded base changes only with a new binary, so a running agent keeps the old base until it restarts, as builtins do today.
  - Installing a plugin that provides a self-named overlay's name moves the base from the builtin to that plugin. The plugin's directory is not watched before then, so the switch takes effect at the next reload, `gpy theme use` or agent restart, as it does today for a name with no user file. `gpy theme list` shows the current base.
- **`gpy-agent theme export` and the shells** consume the merged `ThemeConfig` and are unchanged. The IPC protocol and the Fish integration contract are untouched.
- **Palettes.** Overlays do not affect palette resolution; `ui.recommended.palette` is inherited like any key.
- **`gpy theme import`** keeps writing full themes. Its output is a translation of the user's `starship.toml` against the preset, its `[ui]` is built from defaults rather than the preset's (`emit.rs:204-222`), and its header says to re-import to regenerate. Revisit once overlays ship.
- **`theme new`, `theme save` and the wizard write overlays.**
  - **`theme new`.** `gpy theme new <name> --from <base>` writes `extends = "<base>"`. `new <name>` means `--from default`. Unlike today's copy of the embedded `DEFAULT_THEME_CONTENT` (`theme.rs:231-234`), it therefore picks up a user `default.toml` if one exists. The output names the base it used.
  - **`theme save <name>`** writes `extends = "<active>"`, with today's overwrite prompt.
  - **`theme save` with no name, or with the active theme's name:**
    - If the active theme resolves to a user file (overlay or full copy), nothing is written, and the command says the theme already is that file. A self-named skeleton there would resolve below the file and silently drop every customisation.
    - Otherwise (a builtin or plugin theme is active), it writes a self-named overlay skeleton, `extends = "<active>"`. The confirm prompt no longer says "with its current configuration" (`theme.rs:279-285`).
  - **`--full`** (both commands) writes the effective theme as a full file. When the base is a full theme, that is the base's raw source (today's output, comments kept). When the base is an overlay, it is the merged result, serialized. `save --full` to the active user file is refused like the plain form; `save <other> --full` freezes it under another name.
  - **Check before write.** Every write by `new` and `save` is checked first: the new file's chain must resolve and load. This catches, for example, `save starship` from an overlay `mine` that extends `starship`, which would create a cycle.
  - **The wizard** writes only keys whose effective value changed.
    - For each of its six keys, it compares the value read through the field or its accessor (e.g. `ClockTheme::show_seconds()`) in the pending theme and in the base. It does not compare serialized fields: edits accumulate on the pending theme (`wizard/state/mod.rs:286-295`), and a toggle sets `Some(..)` (`:336-364`). A key toggled twice is therefore `Some(default)` where the builtin has `None`, which would serialize as a change.
    - With no user file, it writes a new self-named overlay with the differing keys.
    - In an existing overlay, it sets differing keys and removes keys now equal to the overlay's base, in place with `toml_edit`, keeping comments.
    - In a full copy, it sets the changed keys in place. It never converts a full copy; it prints the `rebase` hint instead.
- **Schema and versioning** (`gpy-agent/SCHEMA_EVOLUTION.md`).
  - For loading, `extends` and `unset` are new optional top-level keys: **MINOR**. There is one exception. Unknown keys are ignored today ("Compatibility guarantees"), so a file that already has a top-level `extends` or `unset` changes meaning. Such a file is unlikely and is not special-cased; the CHANGELOG entry names it. Every other existing theme loads and behaves as before.
  - Older GPY versions cannot read an overlay (it has no `[segments]`). A downgrade is not supported for theme files.
  - The changed output of `theme new`, `theme save` and the wizard is a CLI behaviour change. It is recorded as **Breaking:** in `CHANGELOG.md`, because a script that expected a full copy gets an overlay.
  - There is no published JSON schema for themes (`gpy-agent/schemas/` covers IPC and plugin manifests). The published form is the builtins in `config/themes/` and the theme docs, which gain an overlay section.
- **Security.**
  - `extends` is a name, never a path. It must pass `discovery::validate_config_name` (no `/`, `\`, `..` or control characters, `config/types.rs:403-414`).
  - It is resolved only through theme discovery (user themes directory, plugin `themes/` directories, embedded builtins), never joined to the overlay's directory.
  - No new path validation logic, filesystem location or external process is introduced.
  - The chain bound and the cycle check limit the work an adversarial file can cause per reload.
  - `unset` paths are data matched against table keys, not filesystem paths.

## Alternatives Considered

### Status quo plus #1088 and #1089

Retire pristine copies, give users `diff`/`reset`/`migrate`, and explain shadowing in errors and `doctor`. These have shipped and are needed in any case. Rejected as the end state: every wizard save, `theme new` and `theme save` still creates a fork, so the next breaking theme change finds new stale copies.

### Always-full copies with a version stamp

Write `# gpy-theme-base = "starship@0.7.1"` (or a key) into copies and warn when the builtin has changed since. This detects staleness, which #1088's hash table and #1085's hint already do for the common cases, but leaves the user to merge by hand. Rejected.

### Patch or diff files

Store a textual patch against the builtin, or JSON-Patch-style operations. A line patch fails on any upstream reformatting or comment edit. An operation list is a second language over TOML, with no editor support. An overlay is itself a TOML theme fragment that users can read and edit. Rejected.

### Three-way merge on upgrade

Keep full copies and merge upstream changes into them on upgrade, using the released version the copy started from as the ancestor. This needs the content of every released builtin in the binary (#1088 ships hashes only), a conflict UI for a prompt tool, and file rewrites in `setup` or at agent start. The result is still a fork. Rejected.

### Overrides in `config.toml` applied to any theme

A `[theme_overrides]` table applied over whichever theme is active. Colours chosen for one theme (the `starship` flat look) would apply to another (the `default` powerline), so it cannot replace per-theme customisation. Some wizard keys (clock `time_format`/`show_seconds`, git `show_*`) are arguably behaviour, not style, and ADR-0011 item 4 moved plugin settings to `config.toml` for that reason. Whether these keys move too is a separate question, not decided here.

### Helix-style shallow merge, or deep merge without `unset`

A depth-1 merge makes a one-key overlay drop its siblings (section 2). Without `unset`, an overlay cannot set an `Option` field back to `None`, which `format` on `clock`, `status` and `character` uses to mean "render in the shell".

## Consequences

### Positive

- A customised theme receives every later builtin fix to keys it does not set. The wizard's typical file shrinks from a full serialized theme to a few lines.
- Upgrade breaks shrink to the keys the user wrote, and errors point at the overlay's file and line.
- `ui.theme`, `gpy theme use` and completion keep using the builtin's name; customising a theme needs no rename.
- Plugin authors can ship variants of a builtin as overlays.

### Negative

- Loading becomes a chain: several files per theme, chain-aware reload and watching, and attribution through the merge, in code that every prompt path uses.
- The cost of a load rises and has not been measured.
  - Today a user file short-circuits resolution with no plugin discovery (`manager.rs:629-632`), and `oneshot` loads the theme on every request that is not JSON (`agent/oneshot.rs:217-233`).
  - A self-named user overlay, the wizard's default output, must look below the user source. That means `discover_plugins()`, which reads every plugin manifest (`manager.rs:654-664`), then parsing the embedded base (about 200 lines for `starship`) and merging.
  - The poll fallback pays the same cost on every tick, plus reading the overlay to find its `extends`.
  - Discovery runs once per load and is shared by every hop. If measurement shows the cost is over budget, the daemon can cache parsed builtin tables for the life of the process, and plugin theme candidates until the next reload or `theme use`. `oneshot` runs one process per request, so it cannot cache and pays the full cost each time.
- A user cannot see the effective theme by opening one file; `gpy theme diff` and `theme save <name> --full` show it.
- Inheritance can surprise: a builtin change the user did not want now applies automatically. `--full` and full copies remain for users who want a frozen theme.
- `theme new`, `theme save` and the wizard change their output, a **Breaking:** CLI entry.

## Follow-up work items

1. **Overlay loader.** A new `theme/overlay.rs`, used by every entry point: `ThemeManager::load_source`, `config::loader::load_theme(_from_path)` (`oneshot`), and `theme validate` by name and by path. `ThemeManager::builtin` stays full-only.
   - The split of `theme::parse` into `parse_table` and `from_table`.
   - `extends`/`unset` parsing and checks, and the language-table canonicalisation.
   - Chain resolution: the "never itself" rule, exact stem matching, every candidate for a name, and plugin overlays resolving to the embedded builtin.
   - Deep merge, cycle detection on canonicalised paths, and the 4-hop limit.
   - The three-step attribution, with `locate`'s per-key check taking a `toml::Table`.
   - Tests: merge rules, arrays, `unset`, flat versus nested language keys, a self-named overlay, a user→plugin→builtin chain, a cycle including a case-variant name, depth, and attribution to the overlay line for a plugin key, a typed key and a table-valued key.
   - Performance: measure a self-named overlay, with and without an installed plugin, against `theme_manager_load` (`tests/performance-baselines.json:100-104`) for the daemon and `prompt_render_degraded` (`:45-49`) for the `oneshot` path, which has no budget of its own. Add the caches from Consequences only if over budget.
2. **Chain-aware hot reload.** `ThemeLocation` as a chain, a combined fingerprint, the watch-path union including a plugin base's directory, `PollTarget::path_of` as a list (shared with the config manager), and `Reloaded::Source` on chain changes. Tests in `theme_manager_tests.rs`: editing the base and the overlay each reload; an overlay appearing over the active builtin switches the source.
3. **Writers emit overlays.**
   - The wizard's effective-value comparison and in-place `toml_edit` writes replace `save_user_theme`'s full serialization.
   - `theme new` and `theme save` write overlays: `save` writes nothing when the active theme is a user file, `--full` follows the rule above, and every write is checked to load first.
   - Docs: `docs/user/cli-reference.md`; `docs/user/theme-customization.md`, which describes `theme new` as a copy (`:126-138`) and has "Your copy of a builtin theme after an upgrade" (`:210-220`); `docs/dev/create-theme.md`; `docs/dev/plugin-theme-authoring.md`; and the "Compatibility guarantees" section of `gpy-agent/SCHEMA_EVOLUTION.md` (overlay keys, files that already have `extends`/`unset`, no downgrade).
   - A CHANGELOG **Breaking:** entry.
4. **`gpy theme rebase <name> [--onto <base>] [--write] [--unset-missing]`.** The conversion in Decision section 4: the normalised comparison, the list of `Option` fields with defaulting accessors, the `copy_aside` + `write_atomically` write, the before-write check, the refusal and `migrate` hint for copies that do not load, and `recovery_hint` updated to name the command. CLI tests on a modified 0.7 `starship` copy, a wizard-written copy (its defaults and nested language keys must drop) and a pristine copy.
5. **Overlay-aware listing and diagnostics.**
   - `user_copy::shadowing_copy` and `annotate_load_error` treat any user file with a builtin's name as a fork and append "may have been written by an older GPY" (`user_copy.rs:38-82`). A self-named overlay gets the chain error from section 2 instead.
   - `gpy doctor` (`doctor_themes.rs:13-17`) and `gpy setup` (`setup_themes.rs:45-50`, "Kept your theme files that override builtins") list overlays separately from full copies. `doctor` suggests `rebase` for full copies that shadow a builtin and reports chain errors.
   - `gpy theme list` shows an overlay's base (`starship (user, extends builtin starship)`).
   - `gpy plugin validate` gains plugin theme validation, including the builtin-only `extends`.
   - `gpy theme validate` warns on `unset` paths the base lacks and applies the plugin rule to files in a plugin's `themes/` directory.
6. **#1089 commands on overlays.** `diff` and `reset` accept user overlays (`theme_override.rs:73-78`), and `diff` compares against the base's merged result. `migrate` rewrites and drops `unset` paths along with keys. Tests for each.

## Decisions needing maintainer approval

1. **Theme file schema:** new top-level keys `extends` (string, a theme name) and `unset` (array of dotted key paths). A file with `extends` is an overlay and need not contain `[segments]` or `[ui]`.
2. **Merge semantics:** deep table merge at every depth; arrays replace; `[segments.language]` and the `unset` paths into it canonicalised to the nested spelling before the merge; `unset` applied to the base before the overlay merges; unknown `unset` paths ignored at load and warned about by `validate`; validation on the merged result only.
3. **Resolution:** a user file's `extends` resolves user > plugin > builtin by exact stem. A file never resolves to itself (a self-named overlay extends the next lower source). A plugin overlay must name a builtin and resolves to the embedded one. Chains have at most 4 hops, and cycles are errors.
4. **Writers:** the wizard (comparing effective values), `theme new` (no `--from` means `--from default`) and `theme save` write overlays by default. `theme save` never rewrites the active theme's user file. `--full` writes the effective theme as a full file. This is a **Breaking:** CLI change. `theme import` keeps writing full themes.
5. **Migration:** no automatic conversion. A new `gpy theme rebase` command (name to confirm) compares normalised typed values, never generates `unset` unless `--unset-missing` is given, and writes only with `--write`, after `copy_aside`. `migrate` rules also rewrite `unset` paths.
6. **Classification:** the theme loading change is MINOR under `SCHEMA_EVOLUTION.md`, except that files that already have a top-level `extends` or `unset` change meaning. Older GPY versions are not expected to read overlays.

## References

- `gpy-agent/src/theme/manager.rs` (`ThemeLocation`, `resolve_path`, `discover_available_themes`, `save_user_theme`, watching), `gpy-agent/src/theme/parse.rs`, `gpy-agent/src/theme/locate.rs`, `gpy-agent/src/theme/model/mod.rs`, `gpy-agent/src/theme/user_copy.rs`
- `gpy-agent/src/config/loader.rs`, `gpy-agent/src/config/discovery.rs`, `gpy-agent/src/config/language_icons.rs`, `gpy-agent/src/watcher/hot_reload.rs`
- `gpy-agent/src/commands/theme.rs`, `gpy-agent/src/commands/theme_migrate.rs`, `gpy-agent/src/commands/theme_override.rs`, `gpy-agent/src/commands/wizard/`
- [ADR-0005](adr-0005-theme-hot-reload.md): theme hot reload; [ADR-0011](adr-0011-plugin-shell-coverage.md) item 4: plugin settings in `config.toml`
- [`gpy-agent/SCHEMA_EVOLUTION.md`](../../../gpy-agent/SCHEMA_EVOLUTION.md)
- Helix theme inheritance: <https://docs.helix-editor.com/themes.html#inheritance>, <https://github.com/helix-editor/helix/blob/master/helix-view/src/theme.rs>
- TypeScript `extends`: <https://www.typescriptlang.org/tsconfig#extends>

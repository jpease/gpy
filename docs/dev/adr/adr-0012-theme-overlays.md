# ADR-0012: Theme Overlays

## Status

Accepted (2026-10-27). The maintainer approved all six decisions at the end of this document as written, and chose overlays as the writers' default from the start (no opt-in release). The ADR alters the structure of the theme file schema, which needs explicit maintainer approval ([ADR-0011](adr-0011-plugin-shell-coverage.md) Decision item 9 lists it). Implementation is tracked as separate work items (the follow-ups at the end).

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

Accepted: **overlay themes**. A user theme may name a base with `extends` and contain only the keys it changes. The loader merges it over the base before the existing typed step. The wizard, `theme new` and `theme save` write overlays. Existing full copies keep working unchanged and are converted only on request.

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
- `from_table(table, source) -> ThemeConfig` runs the `[segments]`/`[ui]` check, typed deserialization and `validation::validate_theme_config`. Its error keeps a validation failure as the structured `ValidationError` (field path and kind), which attribution reads (below). Today `From<ValidationError> for Error` flattens it into `Error::Config`'s message string (`config/validation/mod.rs:154-161`); attribution must not depend on downcasting that error's `source`.

The loader runs `parse_table` once on each file, which it needs anyway to read `extends`. A full theme's table then goes straight to `from_table`, so a full-theme load still parses its text once, as today. Only when `from_table` fails is the text parsed again for spans, as `reparse_for_diagnostic` does today (`parse.rs:41-52`), so a full theme keeps today's diagnostics unchanged. `theme::parse` stays, as `parse_table` followed by `from_table`, for callers that hold only a string. For an overlay, the loader merges the per-file tables in this order:

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
- **Validation runs on the merged result only.** An overlay is not a theme on its own, so the existing checks run on the merged table: `[segments]`/`[ui]` present, typed deserialization, `validate_theme_config`, and template validation in `theme use` and `theme validate`. Each file in the chain is first parsed alone for TOML syntax, with its own path and span, as today. The checks specific to overlays:
  - `extends` is a valid name (section 5).
  - `unset` paths are well-formed.
  - The chain resolves.
- **Error attribution.** The key path of a failing typed parse is not available from the error:
  - `toml::de::Error` exposes only `message()` and `span()`. Its key path is a private field, printed only when the error has no span or input (toml 1.1.8 `src/de/error.rs:5-10,63-80,159-160`).
  - A merged `toml::Table` has no source text to take spans from.

  So attribution finds the failing key structurally. It runs only when a check on the merged table fails, as today's re-parse does, so a successful load pays nothing for it. It works in three steps:

  1. **Find the key**, on the merged table.
     - **`[segments]` and `[ui]` missing:** the check that both are present tables (`parse.rs:20-29`) gives the path `segments` or `ui`. That covers `unset = ["ui"]` and `segments = "x"`, both of which step 2 records.
     - **Other `[segments]` entries:** every entry of `[segments]` that is not one of the seven typed tables (`locate.rs:12-20`) is first deserialized alone into `PluginSegmentTheme`. A non-table value such as `segments.foo = 1` fails here and gives `segments.foo`; #1084's check skips non-tables today (`locate.rs:33`). For a failing table, #1084's per-key check, changed to take a `toml::Table` instead of source text, deserializes each key alone into `PluginSegmentTheme` (`locate.rs:37-55`) and gives `segments.<id>.<key>`.
     - **Typed tables:** these are the seven `[segments.<builtin>]` tables and `[ui]`. Each table is deserialized alone, then each direct key of the first failing table alone. A one-key table is valid input because the containers default every missing field: `UiTheme` and every typed segment struct are `#[serde(default)]` (`model/mod.rs`), and `LanguageTheme`'s deserializer starts from its default (`language_icons.rs:74`). The result is a path like `segments.clock.time_format`.
     - **Table-valued keys:** a failing key whose value is a table gives the path to that table. Examples: a delimiter, which requires `icon` (`config/mod.rs:317-326`), and `[ui.recommended]`.
     - **Language maps:** `segments.language.overrides`, `symbols` and `styles` are descended into instead. Each entry is deserialized alone as the map's value type (`ColorSpec` for `overrides`, `String` for the others, `language_icons.rs:33-41`). So a bad `rust_bg_color` gives `segments.language.overrides.rust_bg_color`, which step 3 maps back to a flat key.
     - **Validation errors:** attribution reads the structured `ValidationError` (`config/validation/theme.rs:26-79`). Every path it maps is a key of the merged `[segments]` table, so a name the error normalised is mapped back to the raw keys first:
       - An invalid section name gives `segments.<id>` (`:56-59`).
       - A section that duplicates another ignoring case names only the later key in byte order (`:52-53,60-66`). With the `username` section every builtin has (`config/themes/default.toml:125`), an overlay's `[segments.Username]` sorts first, so the error names the builtin's `segments.username`. Attribution therefore finds every `[segments]` key whose `SegmentName::new` value (trimmed and lowercased, `plugin/manifest.rs:58-71`) equals the named key's, and maps all of them.
       - A bad variant colour gives `segments.<id>.<key>` (`:40`).
       - The shell-variable and token collisions have the field path `segments` only (`:31-37,72-78`); their kind names the two colliding sections or fields (`first` and `second`), and attribution maps both. A shell-variable collision's fields are raw paths such as `segments.a.open.icon` (`theme/export/plugin_segments.rs:188-196`). A collision with a built-in theme export has no `first` and maps only the plugin field. A token collision's `first` and `second` are normalised `SegmentName`s (`plugin/manifest.rs:129-132`): `[segments.Foo-bar]` against `foo_bar` names `foo-bar`, which is not a key. Each is mapped back to every `[segments]` key that normalises to it, as for a duplicate.
     - **Template errors:** `validate_segment_templates` returns a `ValidationError` whose field path is `segments.<segment>.format` or a style key (`segments.git.git_style.<key>`, `segments.language.<lang>_style`; `config/validation/templates.rs:83,110`). Today `theme use` and `theme validate` flatten it into a message (`commands/theme.rs:158-159,467-468`). Both instead pass it, with the chain, to the same attribution, so a bad `format` in an overlay names the overlay's file and line. A flat language path is first mapped to the nested form (`segments.language.styles.<lang>_style`).
  2. **Find the file.** The merge is replayed over the per-file tables, from the bottom of the chain up, keeping a record of which file last changed each path:
     - **Writes.** Where a file's table meets a base table, the replay recurses. Anywhere else the file's value is put at the path: a leaf, an array, a table the base lacks, or a value replacing one of the other kind (a table over a string, a string over a table). The records at and beneath the path are cleared, and the file is recorded for the path itself and for every path inside its value, tables included. So an empty `[segments.username.open]` over `default`, which fails because a delimiter needs `icon` (`theme/model/mod.rs:80`, `config/mod.rs:317-319`), is recorded at `segments.username.open`.
     - **`unset` entries.** Each entry clears the records at and beneath its path and is itself recorded for that path, as that file's `unset` entry.
     - **Lookup.** A failing path maps to every record at or beneath it, plus the nearest record above it. For a leaf, that is the file that set it, or the replacing value or `unset` entry above it. Example: `default`'s `[segments.clock.open]` holds only `icon` (`config/themes/default.toml:59-60`). An overlay with `unset = ["segments.clock.open.icon"]` leaves that delimiter without its required `icon`. The failing path is the table `segments.clock.open`, and the only record at or beneath it is the overlay's `unset` entry.
     - The error names the user and plugin files among the records mapped to the failing paths (for a collision or duplicate, every key mapped in step 1). It names an embedded builtin only when no user or plugin file is among them. When no record maps at all, it names the chain only.
  3. **Find the line.** Each named file is re-parsed with `toml_edit::Document`. For a write, the key's span gives line and column, as `locate::position` does (`locate.rs:57-72`). For an `unset` record, the span of that element of the file's `unset` array does. Canonical language paths, including those in `unset` entries, are mapped back to the file's own spelling first.

  If no single key fails alone (a failure that needs two keys together), the error names the table, the files mapped to it and the chain. Format, in the repo's style:

  ```text
  Invalid theme 'starship'
    Diagnostic: segments.username.format: unknown key for a plugin segment, set at ~/.config/gpy/themes/starship.toml:7 (chain: starship (user) → starship (builtin))
    Remediation: Remove the key, or run `gpy theme migrate starship`.
  ```

  The error calls the failure a GPY bug only when at least one record maps to the failing paths and every mapped record comes from an embedded builtin, that is, when a builtin wrote or unset something on them and no user or plugin file in the chain did. A failure with no mapped record is reported with the chain and no such wording.

### 3. Precedence and resolution

- **An `extends` in a user file resolves like `gpy theme use <name>`**: user > plugin > builtin, through the same discovery, with one rule for the shadowing case: **a file never resolves to itself.**
  - When the name is the file's own name, resolution starts below the file's own source (for a user file: plugin, then builtin). So the wizard can write `starship.toml` with `extends = "starship"`, `ui.theme` stays `starship`, and the user keeps receiving the builtin. Helix allows the same.
  - If nothing lies below, the error says `extends 'x' names this theme and no plugin or builtin theme 'x' exists`.
  - Resolution compares the name with discovered file stems exactly. It never joins the name to a directory and tests `exists()`, as `resolve_path` does (`manager.rs:629-632`).
  - **A file's own name is its on-disk stem**, never the name it was requested by. The top-level theme still resolves through `resolve_path`, so on a case-insensitive filesystem `ui.theme = "Starship"` loads `starship.toml`, as it does today. The loader then takes that file's own name and its path for the cycle check from the directory listing:
    - the entry whose stem equals the requested name exactly; or, if none does,
    - the single entry whose stem equals it ignoring case, compared with Rust's `str::to_lowercase` on both (Unicode lowercase mapping, no normalisation; a case-insensitive filesystem normally holds at most one such entry).
    - So the file's `extends = "starship"` resolves below it, exactly as under `ui.theme = "starship"`, and is not reported as a cycle.
    - Names may be any Unicode except separators, `..` and control characters (`config/types.rs:408-414`), and APFS and HFS+ also ignore Unicode normalisation, so an NFC request can open an NFD file name that neither rule matches. If no entry, or more than one, matches, a full theme loads as today (it needs no own name), and an overlay is an error: `theme 'X' opened <path>, whose file name differs; set ui.theme to the file's name`. GPY adds no Unicode normalisation dependency for this.
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

  So rebase deserializes the copy and the merged base into typed `ThemeConfig`s, serializes both with the same serializer and normalises the result: `C = N(Table::try_from(copy))` and `B = N(Table::try_from(base))`.
  - **`N` treats a defaulted `Option` as unset.** It removes each `Option` field that is read through an accessor supplying a default when its value equals that default. These are `ClockTheme::time_format()`, `show_leading_zero()` and `show_seconds()`, and `GitTheme::show_counts()`, `show_branch()`, `show_ahead_behind()` and `show_stash()` (`model/mod.rs:338-352,640-660`). An absent field and one set to its default then compare equal, as the accessors read them. The list is kept once next to the model, with a test against the accessors. Fields where `None` has its own meaning (`format`) are not in it.

  It then decides for each key the copy's file actually contains, mapped to its normalised path (the language rule of section 2 maps flat to nested):
  - **Equal** in C and B, including absent from both: dropped from the file. A wizard copy's `show_seconds = false` (`wizard/state/mod.rs:345-348`) over a base with no `show_seconds` is one: `N` removes it from C.
  - **Otherwise:** kept in the file's own spelling, with its comments (`toml_edit`).

  A key with no normalised path (ignored at load today) is dropped and reported.
- **It never generates `unset` on its own.** Paths where C and B differ but the copy's file sets no key are inherited after the rebase. Each such key was either added upstream after the copy was made (which the user should now receive; `ssh_icon` on `hostname` is one) or deleted by the user. The copy cannot tell them apart, and #1088 ships hashes, not old contents, so there is no common ancestor for a three-way merge.
  - **Inherited paths are taken at the topmost level.** An inherited path is a normalised path where C and B differ, the copy's file sets no key at or beneath it, and its parent is the root or holds a key the file sets. So a copy that has `[segments.clock]` but not `default`'s `open` delimiter in it, or that has `[segments]` but no `[segments.hostname]` plugin section, has the one inherited path `segments.clock.open` or `segments.hostname`, not one per leaf inside it.
  - The report lists those paths. `--unset-missing` turns each of them into one `unset` entry at that level, for users who deleted keys on purpose. Leaf-level entries would leave `open = {}`, which fails because a delimiter needs `icon` (`config/mod.rs:317-319`), or an empty `hostname` table that C does not have.
- **Check before write.** The new overlay's merged result must load, and `N(Table::try_from(merged))` must equal C except at and beneath the reported inherited paths; with `--unset-missing` it must equal C everywhere. Both sides go through `N`, so a key dropped as a default is absent from both and passes. Otherwise nothing is written. A copy that does not load (stale keys, the #1003 case) is refused with a hint to run `gpy theme migrate <name>` (#1089) first: migrate repairs keys, rebase removes duplication.
- **#1089 on overlays.**
  - `diff` and `reset` accept any user overlay, not only the builtin names they accept today (`commands/theme_override.rs:73-78`). `reset` moves an overlay aside like a full copy.
  - `gpy theme diff <name>` on an overlay compares two merged results, the base's and the overlay's, each as `N(Table::try_from(..))` serialized with the same serializer. Only keys whose effective value the overlay changes appear, never the base's other 200 lines. On a full copy it keeps today's text diff of the raw source against the builtin (`theme_override.rs:27-32`).
  - `reset` reports what the name resolves to afterwards instead of always "The builtin theme '{name}' is in use again" (`theme_override.rs:60`): the builtin, the plugin theme and its plugin id, or nothing (`no theme 'mine' remains`).
  - **`reset` checks the whole active chain.** Today it compares only the name with `ui.theme` (`theme_override.rs:69-71`). Under overlays the moved file can be any link of the active theme's chain: with active `mine2` extending a user overlay `mine`, `reset mine` breaks `mine2`'s `extends`. So `reset` resolves and loads the active theme with the file left out. If that fails (nothing remains under the name, an `extends` no longer resolves, or the result does not load), it refuses, names the active theme and the failure, and says to run `gpy theme use <other>` first. Otherwise the agent could not load the active theme, and `oneshot` would silently fall back to `default` (`agent/oneshot.rs:221-230`). If the file is in the active chain and the theme still loads, `reset` moves it, says the active theme changed, and reloads the agent.
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
  - **`theme new`.** `gpy theme new <name> --from <base>` writes `extends = "<base>"`. `new <name>` means `--from default`. Unlike today's copy of the embedded `DEFAULT_THEME_CONTENT` (`theme.rs:231-234`), it therefore picks up a user `default.toml` if one exists. The output names the base it used, and when the base is a plugin theme it also names the plugin the new theme now depends on (Consequences).
  - **`theme save <name>`** writes `extends = "<active>"`, with today's overwrite prompt.
  - **`theme save` with no name, or with the active theme's name:**
    - If the active theme resolves to a user file (overlay or full copy), nothing is written, and the command says the theme already is that file. A self-named skeleton there would resolve below the file and silently drop every customisation.
    - Otherwise (a builtin or plugin theme is active), it writes a self-named overlay skeleton, `extends = "<active>"`. The confirm prompt no longer says "with its current configuration" (`theme.rs:279-285`).
  - **`--full`** (both commands) writes the effective theme as a full file. When the base is a full theme, that is the base's raw source (today's output, comments kept). When the base is an overlay, it is the merged result, serialized. `save --full` to the active user file is refused like the plain form; `save <other> --full` freezes it under another name.
  - **Check before write.** Every write by `new` and `save` is checked first: the new file's chain must resolve and load. This catches, for example, `save starship` from an overlay `mine` that extends `starship`, which would create a cycle.
  - **The wizard** writes only keys whose effective value changed.
    - It compares values as they are read: through the field or its accessor (e.g. `ClockTheme::show_seconds()`), never as serialized fields. Edits accumulate on a pending theme seeded from the selected theme's merged result (`wizard/mod.rs:60,85`, `wizard/state/mod.rs:286-295`), and a toggle sets `Some(..)` (`:336-364`). A key toggled twice is therefore `Some(default)` where the builtin has `None`, which would serialize as a change.
    - A key is changed when its value in the pending theme differs from the seed. Only changed keys are written, so a key the file already sets is left alone even if it now equals its base.
    - **No user file** (a builtin or plugin theme is selected): the seed is the new overlay's parent. The wizard writes a self-named overlay with the changed keys.
    - **An overlay:** each changed key is compared with the overlay's parent's merged result after the overlay's own `unset` entries. If `unset` has an entry for exactly that key, the wizard first drops the entry and compares with the parent without it. The key is then set if it differs and removed from the file if it is equal. So toggling a key the overlay unsets back to the parent's value removes the `unset` entry instead of leaving the toggle with no effect. Edits are made in place with `toml_edit`, keeping comments.
    - **A full copy** has no parent: the changed keys are set in place. The wizard never converts a full copy; it prints the `rebase` hint instead.
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
- Upgrade breaks shrink to the keys the user wrote, and errors point at the overlay's file and line, including template errors from `theme use` and `theme validate`.
- `ui.theme`, `gpy theme use` and completion keep using the builtin's name; customising a theme needs no rename.
- Plugin authors can ship variants of a builtin as overlays.

### Negative

- Loading becomes a chain: several files per theme, chain-aware reload and watching, and attribution through the merge, in code that every prompt path uses.
- The cost of a load rises and has not been measured.
  - Today a user file short-circuits resolution with no plugin discovery (`manager.rs:629-632`), and `oneshot` loads the theme on every request that is not JSON (`agent/oneshot.rs:217-233`).
  - A full theme costs what it does today: its text is parsed once, and the table read to find `extends` goes straight to `from_table` (section 2).
  - A self-named user overlay, the wizard's default output, must look below the user source. That means `discover_plugins()`, which reads every plugin manifest (`manager.rs:654-664`), then parsing the embedded base (about 200 lines for `starship`) and merging.
  - The poll fallback pays the same cost on every tick, plus reading the overlay to find its `extends`.
  - Discovery runs once per load and is shared by every hop. If measurement shows the cost is over budget, the daemon can cache parsed builtin tables for the life of the process, and plugin theme candidates until the next reload or `theme use`. `oneshot` runs one process per request, so it cannot cache and pays the full cost each time.
- A user cannot see the effective theme by opening one file; `gpy theme diff` shows what an overlay changes, and `theme save <name> --full` writes the whole effective theme.
- Inheritance can surprise: a builtin change the user did not want now applies automatically. `--full` and full copies remain for users who want a frozen theme.
- A user theme based on a plugin theme now depends on that plugin. Today `theme new --from <plugin theme>` makes an independent copy (`theme.rs:231-232`). As an overlay, uninstalling or disabling the plugin leaves its `extends` unresolved, so the theme no longer loads and `oneshot` falls back to `default` (`agent/oneshot.rs:221-230`). The same holds for `theme save <name>` with a plugin theme active. `--full` keeps today's independent copy.
- `theme new`, `theme save` and the wizard change their output, a **Breaking:** CLI entry.

## Follow-up work items

1. **Overlay loader.** A new `theme/overlay.rs`, used by every entry point: `ThemeManager::load_source`, `config::loader::load_theme(_from_path)` (`oneshot`), and `theme validate` by name and by path. `ThemeManager::builtin` stays full-only.
   - The split of `theme::parse` into `parse_table` and `from_table`. A full theme runs `from_table` on the table already parsed and re-parses its text for spans only on failure. `from_table` returns validation failures as a structured `ValidationError`.
   - `extends`/`unset` parsing and checks, and the language-table canonicalisation.
   - Chain resolution: the "never itself" rule with a file's own name taken from its on-disk stem (exact, else ignoring case by `str::to_lowercase`, else an error for an overlay), exact stem matching, every candidate for a name, and plugin overlays resolving to the embedded builtin.
   - Deep merge, cycle detection on canonicalised paths, and the 4-hop limit.
   - The three-step attribution: `locate`'s per-key check taking a `toml::Table`, non-table `[segments]` entries, the `[segments]`/`[ui]`-present check, per-entry checks of the language maps, both sides of a collision with normalised names mapped back to raw keys, template errors from `theme use` and `theme validate`, and the merge replay that records writes (including every table a file introduces), replacing values and `unset` entries and clears the records beneath them.
   - Tests: merge rules, arrays, `unset`, flat versus nested language keys, a self-named overlay, a user→plugin→builtin chain, a case-variant `extends` does not resolve, a case-variant `ui.theme` whose file extends its own name, an overlay whose requested name matches no directory entry, and depth. Attribution tests: to the overlay line for a plugin key, a typed key, a table-valued key, a non-table plugin entry, a flat language key and a bad `format` caught by template validation; to an empty `[segments.username.open]` table over `default`; to the `unset` element for `unset = ["segments.clock.open.icon"]` and for `unset = ["ui"]` over `default`; to both sections of a collision between an overlay section and a base section, for an overlay `[segments.Username]` over a builtin (duplicate ignoring case) and an overlay `[segments.Foo-bar]` over a base `foo_bar` (token collision); and "GPY bug" wording only when at least one builtin record maps to the failing paths and no user or plugin record does.
   - Performance: measure a self-named overlay, with and without an installed plugin, against `theme_manager_load` (`tests/performance-baselines.json:100-104`) for the daemon and `prompt_render_degraded` (`:45-49`) for the `oneshot` path, which has no budget of its own. Add the caches from Consequences only if over budget.
2. **Chain-aware hot reload.** `ThemeLocation` as a chain, a combined fingerprint, the watch-path union including a plugin base's directory, `PollTarget::path_of` as a list (shared with the config manager), and `Reloaded::Source` on chain changes. Tests in `theme_manager_tests.rs`: editing the base and the overlay each reload; an overlay appearing over the active builtin switches the source.
3. **Writers emit overlays.**
   - The wizard's effective-value comparison (changed keys only; against the overlay's parent after its `unset` entries, dropping an `unset` entry for exactly the key) and in-place `toml_edit` writes replace `save_user_theme`'s full serialization. Tests: a key toggled twice writes nothing, and toggling a key the overlay unsets back to the parent's value removes the `unset` entry.
   - `theme new` and `theme save` write overlays: `save` writes nothing when the active theme is a user file, `--full` follows the rule above, every write is checked to load first, and the output names a plugin the new theme depends on.
   - Docs: `docs/user/cli-reference.md`; `docs/user/theme-customization.md`, which describes `theme new` as a copy (`:126-138`) and has "Your copy of a builtin theme after an upgrade" (`:210-220`); `docs/dev/create-theme.md`; `docs/dev/plugin-theme-authoring.md`; and the "Compatibility guarantees" section of `gpy-agent/SCHEMA_EVOLUTION.md` (overlay keys, files that already have `extends`/`unset`, no downgrade).
   - A CHANGELOG **Breaking:** entry.
4. **`gpy theme rebase <name> [--onto <base>] [--write] [--unset-missing]`.** The conversion in Decision section 4: the normalised comparison, the normalisation `N` over the `Option` fields with defaulting accessors (used by both the comparison and the before-write check), inherited paths taken at the topmost level and unset there by `--unset-missing`, the `copy_aside` + `write_atomically` write, the before-write check, the refusal and `migrate` hint for copies that do not load, and `recovery_hint` updated to name the command. CLI tests on a modified 0.7 `starship` copy, a wizard-written copy (its defaults, including a toggled `show_seconds = false`, and its nested language keys must drop, and the check must pass), a copy without a delimiter table or plugin section the base has (with `--unset-missing`, one `unset` entry for the table and the check passes) and a pristine copy.
5. **Overlay-aware listing and diagnostics.**
   - `user_copy::shadowing_copy` and `annotate_load_error` treat any user file with a builtin's name as a fork and append "may have been written by an older GPY" (`user_copy.rs:38-82`). A self-named overlay gets the chain error from section 2 instead.
   - `gpy doctor` (`doctor_themes.rs:13-17`) and `gpy setup` (`setup_themes.rs:45-50`, "Kept your theme files that override builtins") list overlays separately from full copies. `doctor` suggests `rebase` for full copies that shadow a builtin and reports chain errors.
   - `gpy theme list` shows an overlay's base (`starship (user, extends builtin starship)`).
   - `gpy plugin validate` gains plugin theme validation, including the builtin-only `extends`.
   - `gpy theme validate` reports `unset` in a file without `extends` (section 1), warns on `unset` paths the base lacks, and applies the plugin rule to files in a plugin's `themes/` directory.
6. **#1089 commands on overlays.** `diff` and `reset` accept user overlays (`theme_override.rs:73-78`). `diff` on an overlay compares the base's and the overlay's merged results, normalised and serialized the same way; on a full copy it keeps today's raw text diff. `reset` reports what the name resolves to afterwards, refuses when the active theme would not load without the moved file (anywhere in its chain), and reloads the agent when the file was in the active chain. `migrate` rewrites and drops `unset` paths along with keys. Tests for each, including `reset mine` with active `mine2` extending `mine`.

## Decisions approved by the maintainer

1. **Theme file schema:** new top-level keys `extends` (string, a theme name) and `unset` (array of dotted key paths). A file with `extends` is an overlay and need not contain `[segments]` or `[ui]`.
2. **Merge semantics:** deep table merge at every depth; arrays replace; `[segments.language]` and the `unset` paths into it canonicalised to the nested spelling before the merge; `unset` applied to the base before the overlay merges; unknown `unset` paths ignored at load and warned about by `validate`; validation on the merged result only.
3. **Resolution:** a user file's `extends` resolves user > plugin > builtin by exact stem. A file never resolves to itself (a self-named overlay extends the next lower source), and a file's own name is its on-disk stem, matched exactly or else ignoring case by `str::to_lowercase`; an overlay whose own name matches no single entry is an error. A plugin overlay must name a builtin and resolves to the embedded one. Chains have at most 4 hops, and cycles are errors.
4. **Writers:** the wizard (writing only changed keys, compared by effective value with an overlay's parent after its `unset` entries), `theme new` (no `--from` means `--from default`) and `theme save` write overlays by default, so a theme based on a plugin theme depends on that plugin. `theme save` never rewrites the active theme's user file. `--full` writes the effective theme as a full file. This is a **Breaking:** CLI change. `theme import` keeps writing full themes.
5. **Migration:** no automatic conversion. A new `gpy theme rebase` command (name to confirm) compares normalised typed values, with defaulted `Option` fields treated as unset in both the comparison and the before-write check. It never generates `unset` unless `--unset-missing` is given, which unsets each inherited path at its topmost level, and writes only with `--write`, after `copy_aside`. `migrate` rules also rewrite `unset` paths. `reset` refuses when the active theme would not load without the moved file, wherever that file sits in the active chain.
6. **Classification:** the theme loading change is MINOR under `SCHEMA_EVOLUTION.md`, except that files that already have a top-level `extends` or `unset` change meaning. Older GPY versions are not expected to read overlays.

## References

- `gpy-agent/src/theme/manager.rs` (`ThemeLocation`, `resolve_path`, `discover_available_themes`, `save_user_theme`, watching), `gpy-agent/src/theme/parse.rs`, `gpy-agent/src/theme/locate.rs`, `gpy-agent/src/theme/model/mod.rs`, `gpy-agent/src/theme/user_copy.rs`
- `gpy-agent/src/config/loader.rs`, `gpy-agent/src/config/discovery.rs`, `gpy-agent/src/config/language_icons.rs`, `gpy-agent/src/watcher/hot_reload.rs`
- `gpy-agent/src/commands/theme.rs`, `gpy-agent/src/commands/theme_migrate.rs`, `gpy-agent/src/commands/theme_override.rs`, `gpy-agent/src/commands/wizard/`
- [ADR-0005](adr-0005-theme-hot-reload.md): theme hot reload; [ADR-0011](adr-0011-plugin-shell-coverage.md) item 4: plugin settings in `config.toml`
- [`gpy-agent/SCHEMA_EVOLUTION.md`](../../../gpy-agent/SCHEMA_EVOLUTION.md)
- Helix theme inheritance: <https://docs.helix-editor.com/themes.html#inheritance>, <https://github.com/helix-editor/helix/blob/master/helix-view/src/theme.rs>
- TypeScript `extends`: <https://www.typescriptlang.org/tsconfig#extends>

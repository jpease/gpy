# ADR-0012: Theme Overlays

## Status

Proposed (2026-10). Needs maintainer approval: it alters the structure of the theme file schema, one of the changes the project's agent rules reserve for explicit approval ("Altering configuration schema structure"). Nothing here is implemented; the follow-up items at the end are separate work.

## Context

A theme name resolves to one file and nothing else is merged. `ThemeManager::resolve_path` (`gpy-agent/src/theme/manager.rs:625-644`) returns `<config>/themes/<name>.toml` when it exists, else the winning plugin or builtin entry from `discover_available_themes` (`manager.rs:534-542`), whose precedence is user > plugin > builtin (`config/discovery.rs:21-29`, ties go to the later candidate, `discovery.rs:88-96`). `config::loader::load_theme_from_path` (`config/loader.rs:340-359`) reads that file, or the embedded builtin with the same stem, and `theme::parse` (`theme/parse.rs:12-45`) requires `[segments]` and `[ui]` and deserializes the whole `ThemeConfig`.

So a user file named like a builtin replaces the builtin outright, and every user customisation is a fork. Ways forks are created today:

- The wizard: `wizard::save` calls `ThemeManager::save_user_theme(selected_theme, pending_theme)` (`commands/wizard/save.rs:38-46`, `manager.rs:597-615`), which serializes the whole `ThemeConfig` with `toml::to_string_pretty`, defaults included and comments lost. The wizard edits six keys (`commands/wizard/state/mod.rs:336-364`): `segments.clock.time_format` and `show_seconds`, `segments.duration.show_milliseconds`, `segments.git.show_branch`, `show_ahead_behind` and `show_stash`.
- `gpy theme new <name> [--from <base>]` copies `DEFAULT_THEME_CONTENT` or the base's raw source (`commands/theme.rs:221-255`, `ThemeManager::theme_source_content`).
- `gpy theme save [name]` writes the active theme's raw source, so `gpy theme save` with a builtin active writes a byte-identical copy of the builtin (`theme.rs:263-312`).
- `gpy theme import` writes a full theme built from the builtin `starship` preset (`import/starship/emit.rs:171-185`).
- Hand edits and copies from older installers.

The failure that started epic #1083: after 0.6.0 → 0.7.1, `gpy theme use starship` failed on a July `starship.toml` copy that still set `format` and `show_always` on `username` and `hostname`, which #1003 turned into plugin segments. Plugin segment tables accept only style keys (`SCHEMA_EVOLUTION.md`, "Compatibility guarantees"; `theme/model/mod.rs:49-50,74-89`), so the stale keys are now errors. The rest of the copy (about 200 lines) also stopped receiving every later fix to the builtin.

Sibling items of the same epic treat the symptoms: #1084 (parse errors name the offending key), #1085/#1086 (load and doctor hints for shadowing copies), #1088 (retire copies byte-identical to a released builtin), #1089 (`gpy theme diff|reset|migrate`). The shared helpers are in `theme/user_copy.rs`. This ADR addresses the cause: customising a theme should record only what the user changed.

### Prior art

Helix themes ([docs](https://docs.helix-editor.com/themes.html#inheritance), [loader](https://github.com/helix-editor/helix/blob/master/helix-view/src/theme.rs), read 2026-10-27): `inherits = "<name>"` plus the overriding keys. The loader merges the parent's `toml::Value` with the child's (top level at depth 1, `[palette]` at depth 2), lets a file inherit from a same-named file in a lower-priority directory, and detects cycles with a set of visited paths. It has no way to remove an inherited key.

TypeScript `tsconfig.json` `extends` ([docs](https://www.typescriptlang.org/tsconfig/extends.html), read 2026-10-27): the base is loaded, then overridden by the inheriting file; `files`, `include` and `exclude` from the inheriting file overwrite the base's, and circular `extends` is not allowed.

## Decision

Proposed: **overlay themes**. A user theme may name a base with `extends` and contain only the keys it changes; the loader merges it over the base before the existing typed parse. The wizard, `theme new` and `theme save` write overlays. Existing full copies keep working unchanged and are converted only on request.

### 1. Format and location

An overlay is an ordinary theme file in the same places as today (`<config>/themes/<name>.toml`, or a plugin's `themes/` directory, section 3), with a top-level `extends` key:

```toml
# ~/.config/gpy/themes/starship.toml
extends = "starship"
unset = ["segments.character.format"]   # the shell draws the character locally

[segments.clock]
time_format = "24"
```

- **Told apart by `extends` alone.** A file with a top-level `extends` string is an overlay; a file without it is a full theme with today's rules (`[segments]` and `[ui]` required). No file extension, directory or header comment carries meaning, so `gpy theme list`, completion, `use` and `validate` keep one namespace. A non-string `extends` is an error.
- **`unset`** (optional, overlay only) is an array of dotted key paths to remove from the base. It is needed because many theme fields are `Option`s whose absence means something (`format` unset: the shell renders `clock`, `status` and `character` locally, `model/mod.rs:274-275,302,423-425`), TOML has no null, and an empty string is a valid value (the `starship` builtin sets `icon = ""`). Each path component must match `^[A-Za-z0-9_-]+$`.
- Both keys are removed before the merged table is deserialized. `unset` in a file without `extends` does nothing and is reported by `gpy theme validate`; making it a load error would reject a file that loads today.

### 2. Merge semantics

Merging happens on `toml::Table` values, before `theme::parse`'s typed step, in this order: resolve the base to its merged table (recursively), remove each `unset` path, then merge the overlay's tables over the result.

- **Tables merge deeply at every depth.** Theme nesting is three levels deep (`[segments.clock.open]`, `[ui.recommended.directory]`, `[segments.language.symbols]`); Helix's depth-1 merge would make `[segments.clock] time_format = "24"` drop every other clock key. Inline tables are tables. When either side is not a table, the overlay value replaces the base value; a type mismatch is left to the typed parse, which reports it (below).
- **Arrays replace.** The arrays in themes are ordered lists (`ui.recommended.enabled_segments`, `segments.language.enabled_languages`), where appending would reorder or duplicate entries, and append semantics would need a second syntax for removal. tsconfig `extends` likewise overwrites `files`/`include`.
- **Removing and replacing.** `unset` removes a leaf or a whole table. Unset-then-set replaces a table: `unset = ["segments.language.symbols"]` plus a `[segments.language.symbols]` table gives exactly the overlay's symbols. An `unset` path the base does not have is ignored at load and reported as a warning by `gpy theme validate`; making it an error would turn an upstream removal into the same upgrade break this ADR removes.
- **Plugin segment tables and `[ui]`** follow the same rules; there is no per-table special case. Variant styles are flat keys (`root_text_color`), so they merge per key. An overlay may style a plugin segment its base does not mention. `[ui.recommended]` is inherited, so `gpy theme use <overlay> --force` applies the base's recommendations unless the overlay changes them.
- **Chains.** `extends` may name another overlay. The resolved chain is at most 4 `extends` hops (an overlay of a user overlay of a plugin overlay of a builtin needs 3); the 5th is an error. A cycle is detected by recording each visited file path (embedded builtins by name) and is an error naming the chain, e.g. `a (user) → b (user) → a (user)`. Every hot reload re-reads the chain, which is why it is bounded.
- **Validation runs on the merged result only.** An overlay is not a theme on its own, so the existing checks (`[segments]`/`[ui]` present, typed deserialization, `validation::validate_theme_config`, template validation in `theme use`) run on the merged table. Each file in the chain is first parsed alone for TOML syntax, with its own path and span, as today. Overlay-only checks: `extends` is a valid name (section 5), `unset` paths are well-formed, and the chain resolves.
- **Error attribution.** The merge records, for each leaf it writes, which file set it and where (toml 1.1's `toml::de::DeTable::parse` gives spanned keys and values). A typed error on the merged table is mapped through that record to the file and line that set the failing key, using the key path #1084 makes the parse error carry. Format, in the repo's style:

  ```text
  Invalid theme 'starship'
    Diagnostic: segments.username.format: unknown key for a plugin segment, set at ~/.config/gpy/themes/starship.toml:7 (chain: starship (user) → starship (builtin))
    Remediation: Remove the key, or run `gpy theme migrate starship`.
  ```

  A key set by an embedded builtin is a GPY bug and says so. Without a key path (some serde errors carry none), the error names the whole chain.

### 3. Precedence and resolution

- **`extends = <name>` resolves like `gpy theme use <name>`**: user > plugin > builtin, through the same discovery, with one rule for the shadowing case: **a file never resolves to itself.** When the name is the file's own name, resolution starts below the file's own source (for a user file: plugin, then builtin). So the wizard can write `starship.toml` with `extends = "starship"`, `ui.theme` stays `starship`, and the user keeps receiving the builtin. Helix allows the same. If nothing lies below, the error says `extends 'x' names this theme and no plugin or builtin theme 'x' exists`.
- **User overlays may extend user, plugin and builtin themes.** `mine.toml` with `extends = "starship"` gets the user's `starship.toml` if one exists (overlay or full), which is what `gpy theme use starship` would show.
- **Plugin overlays may extend builtins only.** A plugin theme is a shipped artifact: depending on a user file makes it render differently per user, and depending on another plugin's theme creates a dependency the manifest cannot declare (`plugin/manifest.rs` has no dependency field). An `extends` from a plugin theme that names a non-builtin is an error from `gpy plugin validate` and at load.
- **Builtins stay full themes.** They are embedded, tested by the golden tests, and the base of everything else; making `text` an overlay of `default` is out of scope.
- **A full copy with a builtin's name keeps shadowing it**, as today; the hints from #1085 and the conversion in section 4 address it. Several plugins providing one name keep today's tie rule (later candidate wins, `discovery.rs:88-96`).
- A file validated by path (`gpy theme validate ./x.toml`) resolves its `extends` as if it were a user theme named after its stem.

### 4. Migration of existing full copies

- **No automatic conversion.** Converting can change what the user sees, because keys the copy lacks are then inherited from the base (below). Full copies keep loading as today.
- **Pristine copies: #1088.** A copy byte-identical to a released builtin carries no intent and is moved to `.bak` by `gpy setup`. That stays the first step and needs no overlay.
- **Modified copies: a new command, `gpy theme rebase <name> [--onto <base>]`.** It rewrites a full theme as an overlay of `<base>`: keys whose value equals the base's merged value are dropped, the rest are kept, and comments on kept keys are preserved (`toml_edit`). `--onto` defaults to the same name's next lower source (the shadowing case); for other names it is required, and the command suggests the builtin with the fewest differing keys. It prints the result and a report and writes nothing unless `--write`, which moves the original aside with `user_copy::move_aside` first.
- **It never generates `unset` on its own.** A key in the base but missing from the copy is either a key added upstream after the copy was made (which the user should now receive; `ssh_icon` on `hostname` is one) or one the user deleted. The copy cannot tell them apart, and #1088 ships hashes, not old contents, so there is no common ancestor for a three-way merge. The report lists those keys; `--unset-missing` turns all of them into `unset` entries for users who deleted keys on purpose.
- **Check before write.** The merged result of the new overlay must deserialize, and its serialized `ThemeConfig` must equal the copy's except at the reported inherited keys; otherwise nothing is written. A copy that does not load (stale keys, the #1003 case) is refused with the hint to run `gpy theme migrate <name>` (#1089) first: migrate repairs keys, rebase removes duplication.
- **#1089 on overlays.** `gpy theme diff <name>` shows the overlay against its base's merged result; `reset` moves an overlay aside like a full copy; `migrate` rules are key rewrites on a raw table and apply to overlays unchanged. `user_copy::recovery_hint` adds `gpy theme rebase` once it exists.
- **Overlays reduce, not remove, upgrade breaks.** An overlay that sets a key a later release rejects still fails, as the July copy would have if it had held only those keys. It fails on the keys the user actually wrote, with the file and line from section 2, and `migrate` covers BREAKING renames.

### 5. Hot reload, export, import, schema and security

- **Hot reload.** `ThemeLocation` (`manager.rs:30-60`) becomes the resolved chain: one entry per file (path, source, on disk). The fingerprint covers every link's identity and bytes, so editing the base or the overlay is a `Reloaded::Content` and a change in the chain (an overlay appears, a base moves to another source) is a `Reloaded::Source` with a re-arm. `watch_paths` is the union over the chain: the user themes directory is always watched already (`manager.rs:414-426`, `watcher/mod.rs:161-168`), so the new case is a user overlay of a plugin theme, which adds that plugin's `themes/` directory and file. The poll fallback's single `path_of` (`manager.rs:669-680`) becomes a list. An embedded base changes only with a new binary, so a running agent keeps the old base until it restarts, as builtins do today.
- **`gpy-agent theme export` and the shells** consume the merged `ThemeConfig` and are unchanged; the IPC protocol and the Fish integration contract are untouched.
- **Palettes.** Overlays do not affect palette resolution; `ui.recommended.palette` is inherited like any key.
- **`gpy theme import`** keeps writing full themes. The output is a translation of the user's `starship.toml` against the preset, its `[ui]` is built from defaults rather than the preset's (`emit.rs:204-222`), and its header says to re-import to regenerate. Revisit once overlays ship.
- **`theme new`, `theme save` and the wizard write overlays.** `gpy theme new <name> --from <base>` writes `extends = "<base>"`, and `new <name>` without `--from` writes `extends = "default"`; `--full` keeps today's copy. `gpy theme save` with no name writes a self-named overlay skeleton instead of a pristine copy; `gpy theme save <name>` writes `extends = "<active>"`; `--full` writes the merged result as a full theme. The wizard diffs `toml::Table::try_from(pending)` against `toml::Table::try_from(base)` (same serializer on both sides, so defaults cancel and a toggle switched back produces no key) and writes only the changed leaves: into a new self-named overlay, or into an existing overlay or full copy in place with `toml_edit`, keeping its comments. It never converts a full copy; it prints the `rebase` hint instead.
- **Schema and versioning** (`gpy-agent/SCHEMA_EVOLUTION.md`). For loading, `extends` and `unset` are new optional top-level keys and every existing theme loads and behaves as before: **MINOR**. Older GPY versions cannot read an overlay (no `[segments]`); a downgrade is not supported for theme files. The changed output of `theme new`, `theme save` and the wizard is a CLI behaviour change, recorded as **Breaking:** in `CHANGELOG.md` because a script that expected a full copy gets an overlay. There is no published JSON schema for themes (`gpy-agent/schemas/` covers IPC and plugin manifests); the published form is the builtins in `config/themes/` and the theme docs, which gain an overlay section.
- **Security.** `extends` is a name, never a path: it must pass `discovery::validate_config_name` (no `/`, `\`, `..` or control characters, `config/types.rs:403-414`) and is resolved only through theme discovery (user themes directory, plugin `themes/` directories, embedded builtins), never joined to the overlay's directory. No new path validation logic, filesystem location or external process is introduced. The chain bound and cycle check bound the work an adversarial file can cause per reload. `unset` paths are data matched against table keys, not filesystem paths.

## Alternatives Considered

### Status quo plus #1088 and #1089

Retire pristine copies, give users `diff`/`reset`/`migrate`, explain shadowing in errors and `doctor`. Needed in any case and already in flight. Rejected as the end state: every wizard save, `theme new` and `theme save` still creates a fork, so the next breaking theme change finds new stale copies.

### Always-full copies with a version stamp

Write `# gpy-theme-base = "starship@0.7.1"` (or a key) into copies and warn when the builtin changed since. Detects staleness, which #1088's hash table and #1085's hint already do for the common cases, but leaves the user to merge by hand. Rejected.

### Patch or diff files

Store a textual patch against the builtin, or JSON-Patch-style operations. A line patch fails on any upstream reformatting or comment edit; an operation list is a second language over TOML with no editor support. An overlay is itself a TOML theme fragment that users can read and edit. Rejected.

### Three-way merge on upgrade

Keep full copies and merge upstream changes into them on upgrade, using the released version the copy started from as the ancestor. Needs the content of every released builtin in the binary (#1088 ships hashes only), a conflict UI for a prompt tool, and file rewrites in `setup` or at agent start; the result is still a fork. Rejected.

### Overrides in `config.toml` applied to any theme

A `[theme_overrides]` table applied over whichever theme is active. Colours chosen for one theme (the `starship` flat look) would apply to another (the `default` powerline), so it cannot replace per-theme customisation. Some wizard keys (clock `time_format`/`show_seconds`, git `show_*`) are arguably behaviour, not style, and ADR-0011 item 4 moved plugin settings to `config.toml` for that reason; whether these move too is a separate question, not decided here.

### Helix-style shallow merge, or deep merge without `unset`

Depth-1 merge makes a one-key overlay drop its siblings (section 2). Without `unset`, an overlay cannot get an `Option` field back to `None`, which `format` on `clock`, `status` and `character` uses to mean "render in the shell".

## Consequences

### Positive

- A customised theme receives every later builtin fix to keys it does not set. The wizard's typical file shrinks from a full serialized theme to a few lines.
- Upgrade breaks shrink to keys the user wrote, and errors point at the overlay's file and line.
- `ui.theme`, `gpy theme use` and completion keep using the builtin's name; no rename is needed to customise.
- Plugin authors can ship variants of a builtin as overlays.

### Negative

- Loading becomes a chain: several files per theme, a provenance record and chain-aware reload, watching and errors, in code that every prompt path uses (daemon and `oneshot`). The cost per `oneshot` load is one more small TOML parse and a table merge per hop, not measured.
- A user cannot see the effective theme by opening one file; `gpy theme diff` and `theme save --full` are the way to inspect it.
- Inheritance can surprise: a builtin change the user did not want now applies automatically. `--full` and full copies remain for users who want a frozen theme.
- `theme new`, `theme save` and the wizard change their output, a **Breaking:** CLI entry.

## Follow-up work items

1. **Overlay loader.** A new `theme/overlay.rs`: `extends`/`unset` parsing and checks, chain resolution with the "never itself" rule and the plugin restriction (this needs discovery to list every candidate for a name, not only the winner `merge_candidate` keeps), deep merge, cycle and 4-hop limits, the provenance record and error attribution on top of #1084's key path. Used by every entry point: `ThemeManager::load_source`, `config::loader::load_theme(_from_path)` (`oneshot`), `theme validate` by name and by path, and `ThemeManager::builtin` stays full-only. Tests: merge rules, arrays, unset, self-named overlay, user→plugin→builtin chain, cycle, depth, attribution to the overlay line. Measure `oneshot` with an overlay against `tests/performance-baselines.json`.
2. **Chain-aware hot reload.** `ThemeLocation` as a chain, combined fingerprint, watch-path union including a plugin base's directory, poll fallback over several paths, `Reloaded::Source` on chain changes. Tests in `theme_manager_tests.rs`: editing the base and the overlay each reload; an overlay appearing over the active builtin switches the source.
3. **Writers emit overlays.** The wizard's table diff and in-place `toml_edit` writes replace `save_user_theme`'s full serialization; `theme new --from`, `theme new` and `theme save` write overlays, with `--full` for today's behaviour. Docs: `cli-reference.md`, `docs/dev/create-theme.md`, `plugin-theme-authoring.md`; CHANGELOG **Breaking:** entry.
4. **`gpy theme rebase <name> [--onto <base>] [--write] [--unset-missing]`.** The conversion in Decision section 4, with the before-write equality check, `move_aside` backup, the refusal and `migrate` hint for copies that do not load, and `recovery_hint` updated to name it. CLI tests on a modified 0.7 `starship` copy and on a pristine copy.
5. **Listing and diagnostics.** `gpy theme list` shows an overlay's base (`starship (user, extends builtin starship)`); `gpy doctor` (#1085/#1086) suggests `rebase` for full copies that shadow a builtin and reports overlay chain errors; `gpy plugin validate` rejects plugin overlays of non-builtins; `gpy theme validate` warns on `unset` paths the base lacks; #1089's `diff` handles overlays.

## Decisions needing maintainer approval

1. Theme file schema: new top-level keys `extends` (string, a theme name) and `unset` (array of dotted key paths); a file with `extends` is an overlay and need not contain `[segments]` or `[ui]`.
2. Merge semantics: deep table merge at every depth, arrays replace, `unset` applied to the base before the overlay merges, unknown `unset` paths ignored at load and warned by `validate`, validation on the merged result only.
3. Resolution: `extends` resolves user > plugin > builtin, a file never resolves to itself (self-named overlays extend the next lower source), plugin overlays may extend builtins only, chains of at most 4 hops, cycles are errors.
4. Writers: the wizard, `theme new` and `theme save` write overlays by default with `--full` for full copies (a **Breaking:** CLI change); `theme import` keeps writing full themes.
5. Migration: no automatic conversion; a new `gpy theme rebase` command (name to confirm) that never generates `unset` unless `--unset-missing` is given and writes only with `--write`.
6. Classification: theme loading change MINOR under `SCHEMA_EVOLUTION.md`; older GPY versions are not expected to read overlays.

## References

- `gpy-agent/src/theme/manager.rs` (`ThemeLocation`, `resolve_path`, `discover_available_themes`, `save_user_theme`, watching), `gpy-agent/src/theme/parse.rs`, `gpy-agent/src/theme/model/mod.rs`, `gpy-agent/src/theme/user_copy.rs`
- `gpy-agent/src/config/loader.rs`, `gpy-agent/src/config/discovery.rs`, `gpy-agent/src/commands/theme.rs`, `gpy-agent/src/commands/wizard/`
- [ADR-0005](adr-0005-theme-hot-reload.md): theme hot reload; [ADR-0011](adr-0011-plugin-shell-coverage.md) item 4: plugin settings in `config.toml`
- [`gpy-agent/SCHEMA_EVOLUTION.md`](../../../gpy-agent/SCHEMA_EVOLUTION.md)
- Epic jpease/gpy-archive#1083 and its items #1084, #1085, #1086, #1088, #1089; #1003 (username/hostname as plugins)
- Helix theme inheritance: <https://docs.helix-editor.com/themes.html#inheritance>, <https://github.com/helix-editor/helix/blob/master/helix-view/src/theme.rs>
- TypeScript `extends`: <https://www.typescriptlang.org/tsconfig#extends>

# ADR-0010: Single Source of Truth for Config Keys

## Status

Proposed (2026-10).

## Context

A config key is declared twice. The `Config` structs in `gpy-agent/src/config/mod.rs` hold the field, its serde default and a `///` doc comment. `CONFIG_KEYS` in `gpy-agent/src/config/metadata.rs` holds the dotted name, a `get` closure, an optional `set` closure that parses the CLI string through the field's validating newtype, and a one-line description for `gpy config list`. `gpy config get/set/list` read only the registry. `gpy config show` and the loader read only the structs.

Tests already catch drift between the two:

- `every_config_leaf_has_registry_entry` and `every_registry_key_is_a_config_leaf` compare the registry with the leaves of a serialized `Config::default()`.
- `reference_doc_matches_registry` and `cli_reference_common_keys_are_registered` tie `docs/user/configuration-reference.md` and `docs/user/cli-reference.md` to the registry.
- `config_list_matches_registry` ties `gpy config list` to it.
- `shipped_config_toml_sets_only_known_keys` covers `config/config.toml` and `config/config.example.toml`.

Ghostty (`src/config/Config.zig`) declares each field once and generates its CLI flag and docs from that declaration. This ADR measures what the same approach would cost gpy.

### Measurements

All runs were on 2026-10-23 at `origin/main` 00e0427 (macOS arm64, rustc 1.99.1, cargo-nextest 0.9.149). Each ran in a scratch worktree (`git worktree add --detach ../scratch origin/main`, removed afterwards, nothing pushed) with `unset CARGO_TARGET_DIR` and `cargo nextest run --features test-support`.

| Measurement | Result |
|---|---|
| Registry size | `CONFIG_KEYS` is 363 lines for 42 keys. 33 are `ConfigKeyMeta` literals of 6 to 20 lines each, 16 of them 9 lines. 9 use the existing `git_icon_key!` macro (16 lines). 3 are read-only. |
| Add one boolean key `ui.scratch_flag` | 5 files, +19 lines (`git status --short`, `git diff --numstat`): the field and its `Default` (+4); `ui_refresh_required`'s exhaustive destructure (+3, forced by E0027); two `UiSettings` literals in `tests/config_tests.rs` (+2, E0063); the `CONFIG_KEYS` entry (+9, forced by `every_config_leaf_has_registry_entry`); and a reference-table row (+1, forced by `reference_doc_matches_registry`). The suite then passed: 2544 run, 2 skipped. `just lint` was clean. |
| Option C spike, 3 keys, no new crate | Deleted the entries for `agent.timeout_seconds`, `git.enabled` and `ui.directory.display`. `get`/`set` fell back to walking `toml::Table::try_from(&config)`: parse the string by the leaf's TOML type, insert it, and deserialize `Config` again so the newtypes' `try_from` checks run. `metadata.rs` +98 / -43: an 88-line fixed block plus hooks, against 29 registry lines and a 13-line helper removed. |
| C: pinned error text | `cli_error_messages_tests` passed. All three `set_config_value_error_messages` cases on spiked keys changed: `Invalid u64 value: abc` → `Invalid integer value: abc`; `Invalid agent timeout: 0` → `Invalid agent.timeout_seconds '0': agent timeout must be between 1 and 300`; the `ui.directory.display sideways` message became serde's ``unknown variant `sideways`, expected one of ...``. `git.enabled maybe` was unchanged. |
| C: other behaviour | `get language.confidence_threshold` printed `0.10000000149011612` instead of `0.1` (an `f32` widened through TOML's `f64`). `set ui.directory.display FULL` and `Truncated` succeed today and failed under the spike (`unknown variant 'FULL'`), because `FromStr` lowercases and serde's `rename_all` does not. `git.icon_set` and `language.display` have the same shape (from reading the code). Four tests failed: the error-text test and three guards written against `CONFIG_KEYS`. `gpy config list` would lose the spiked keys and their descriptions. |
| `schemars` (docs only) | 1.2.3, MIT. The derive adds 6 crates (`schemars`, `schemars_derive`, `serde_derive_internals`, `dyn-clone`, `ref-cast`, `ref-cast-impl`), all MIT or Apache-2.0, which `gpy-agent/deny.toml` allows. `syn` 3 is already in the lock. It turns `///` comments into JSON Schema descriptions and provides no get/set. Not built. |

Not measured: build time, binary size and latency, because three keys cannot tell the options apart. `cargo deny check` with `schemars` was not run either.

The spike is the worst case for option C. Its generic code is a fixed cost, while the savings grow with each key converted (6 to 20 registry lines each). Extrapolated to all 42 keys it would probably delete more than it adds, but that extrapolation was not measured. Line count is therefore not the case against C.

Registry descriptions and the `///` comments on the same fields mostly differ in wording. That is expected: one serves rustdoc readers and the other `gpy config list` users. Nothing requires them to match, and two are actually wrong. The doc comment on `LanguageSettings.detection_mode` lists `content` and `markers` but not `hybrid`, which `types::DetectionMode` has. `ui.show_icons` is described in both places as "Show icons for languages", but it also switches the git icons (`formatter/git_resolver.rs:52`, `:88`) and the separator glyphs (`formatter/separator.rs:62`).

## Decision

Proposed: option A. Keep `CONFIG_KEYS` and its guards.

- The registry costs 1 file and 9 of the 19 lines of a new boolean key, and forgetting it fails a test that names the key.
- Option C is rejected for what it changes, not for its size. It changes the CLI error text, prints widened floats, rejects mixed-case enum input that works today, and needs the registry guards rewritten. Each of these could be special-cased, but every special case is per-type metadata, which is what the registry already holds.
- Options B and D were rejected on reasoning and not spiked. The add-key run bounds what they could save: at most the registry edit, 1 of 5 files and 9 of 19 lines. The field, its `Default`, the exhaustive destructure and the struct literals would still be forced by the compiler, because those name fields directly. At 42 keys that saving does not justify a macro DSL (B) or a new proc-macro crate (D).
- B, C and D each cross the project's autonomy boundaries, which require owner approval before altering configuration schema structure, adding cross-module dependencies, or adding a dependency:
  - B moves the settings structs into macro syntax.
  - C makes `get`/`set` depend on the serialized TOML shape and the loader's deserialization path.
  - D adds a crate, and `schemars` would add 6.

The owner records the decision on the ADR's pull request. Option A has nothing to implement, so no follow-up epic. Revisit if the key count roughly doubles, if descriptions are to feed generated docs, or if another front end (a TUI editor, a JSON Schema) needs the key list.

## Alternatives Considered

### Option A

**Keep the registry and the existing guards** (this proposal). Uniform key families can use a small `macro_rules!` such as `git_icon_key!`. One cheaper addition was considered and declined: a unit test that each registry description and its `///` comment are non-empty and do not contradict each other. Non-empty doc comments are already enforced (`missing_docs = "warn"` under `warnings = "deny"`), and a contradiction cannot be detected mechanically. The two defects above are better fixed by editing the text. Add such a test only if drift reports appear.

### Option B

**A declarative `macro_rules!` declaring each leaf once.** It would generate the struct fields with serde attributes and docs, `Default` and the registry entries. Each leaf would carry its parser and error label, so today's error text could be kept. Not spiked. It would put the eight settings structs in a DSL that `rustfmt` cannot format, and the macro would have to pass struct attributes and the hand-written `Default` impls through. It would save only the registry edit.

### Option C

**Generic get/set over the serialized tree**, with descriptions from doc comments, possibly through `schemars`. Spiked on three keys (see Measurements). Its line count would probably improve with more keys, but the behaviour changes would not go away without per-type special cases. It would also need a description source for `gpy config list`, and the 4 guard tests rewritten.

### Option D

**A proc-macro derive** reading `///` docs and per-field attributes. It needs its own `proc-macro = true` crate, and `gpy-agent` is a single crate with no Cargo workspace, so it would add a crate, `Cargo.toml` and `Cargo.lock` changes, and a direct `syn`/`quote` dependency. Not spiked. It would save the same as B, at the highest maintenance cost of the four options.

## Consequences

### Positive

- No code, dependency or `Config` schema change, so no autonomy-boundary approval is needed.
- `gpy config` error text, case-insensitive enum input and float output stay as they are.
- A key missing from the registry or the reference tables still fails a named test.

### Negative

- Each new key still costs 6 to 20 registry lines in a second file.
- Registry descriptions and `///` comments can still drift without a test failing. The two known defects need a separate docs fix.

## References

- [ADR-0009](adr-0009-deferred-language-version-probes.md): the layout this ADR follows
- `gpy-agent/src/config/metadata.rs` (`CONFIG_KEYS`, guards), `gpy-agent/src/config/mod.rs`, `gpy-agent/tests/cli_error_messages_tests.rs`
- [Configuration reference](../../user/configuration-reference.md)
- Ghostty `src/config/Config.zig`: <https://github.com/ghostty-org/ghostty/blob/main/src/config/Config.zig>
- schemars: <https://graham.cool/schemars/>, <https://crates.io/crates/schemars>

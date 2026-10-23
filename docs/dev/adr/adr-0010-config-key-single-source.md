# ADR-0010: Single Source of Truth for Config Keys

## Status

Proposed (2026-10).

## Context

A config key exists in two places. The `Config` structs in `gpy-agent/src/config/mod.rs` define the field, its serde default and a `///` doc comment. `CONFIG_KEYS` in `gpy-agent/src/config/metadata.rs` gives the same key a dotted name, a `get` closure, an optional `set` closure that parses the CLI string through the field's validating newtype, and a one-line description. `gpy config get`, `gpy config set` and `gpy config list` read only the registry; `gpy config show` and the config file loader read only the structs.

Tests already catch the two lists drifting apart:

- `every_config_leaf_has_registry_entry` and `every_registry_key_is_a_config_leaf` (`config/metadata.rs`) compare the registry with the leaves of a serialized `Config::default()`.
- `reference_doc_matches_registry` compares the key tables in `docs/user/configuration-reference.md` with the registry, and `cli_reference_common_keys_are_registered` checks the "Common Keys" list in `docs/user/cli-reference.md`.
- `config_list_matches_registry` (`tests/cli_integration_tests.rs`) checks that `gpy config list` prints the registry.
- `shipped_config_toml_sets_only_known_keys` and `shipped_config_toml_matches_builtin_defaults` (`tests/config_loader_tests.rs`) check `config/config.toml` and `config/config.example.toml`.

Nothing checks the registry's `description` strings against the `///` doc comments on the fields.

Ghostty (`src/config/Config.zig`) shows the alternative: each config field is declared once, and its CLI flag and documentation are generated from that declaration. This ADR asks whether gpy should move to a single declaration per key, and measures what that would cost before choosing.

### Measurements

All runs were on 2026-10-23, on macOS arm64 with rustc 1.99.1 and cargo-nextest 0.9.149, at `origin/main` 00e0427. Commands run from the repository root unless a `cd` is shown. The scratch worktree was deleted afterwards and nothing from it was committed or pushed.

#### Size of the registry today

```sh
f=gpy-agent/src/config/metadata.rs
s=$(grep -n '^pub const CONFIG_KEYS' $f | cut -d: -f1)
e=$(awk -v s=$s 'NR>s && /^\];/{print NR; exit}' $f)
echo $((e-s+1)); sed -n "${s},${e}p" $f | grep -c 'ConfigKeyMeta {'
sed -n "${s},${e}p" $f | grep -c 'git_icon_key!('; sed -n "${s},${e}p" $f | grep -c 'set: None'
```

Observed: the `CONFIG_KEYS` array is 363 lines (56-418) for 42 keys: 33 written out as `ConfigKeyMeta` literals and 9 through the existing `git_icon_key!` `macro_rules!` (a 16-line macro, lines 34-49). Three keys are read-only (`set: None`): `git.skip_paths`, `language.enabled_languages` and `ui.enabled_segments`. A written-out entry runs 6 to 20 lines: 6 for a read-only key, 9 for most booleans (16 of the 33), and more for a newtype key, which names its parser and error label.

#### Cost of adding one key today

A throwaway `ui.scratch_flag: bool` (default `true`, modelled on `ui.transient_prompt`) was added in a scratch worktree. After each edit the build or tests were rerun, and the next edit was whatever they demanded:

```sh
git worktree add --detach ../gpy-adr0010-scratch origin/main
cd ../gpy-adr0010-scratch/gpy-agent && unset CARGO_TARGET_DIR
cargo nextest run --features test-support --no-run          # after each code edit
cargo nextest run --features test-support --no-fail-fast    # after the build passes
cd .. && just lint && git status --short && git diff --numstat
git worktree remove --force ../gpy-adr0010-scratch
```

| Step | Edit | What forced the next edit |
|---|---|---|
| 1 | Field plus `Default` entry in `UiSettings` (`config/mod.rs`, +4) | `error[E0027]: pattern does not mention field 'scratch_flag'` at `src/agent/events/mod.rs:794` and `:802` |
| 2 | `ui_refresh_required`'s two destructures and its comparison (`agent/events/mod.rs`, +3) | `error[E0063]: missing field 'scratch_flag' in initializer of 'UiSettings'` at `tests/config_tests.rs:114` and `:263` |
| 3 | Both struct literals (`tests/config_tests.rs`, +2) | Full suite: 2544 run, 2543 passed, 1 failed: `every_config_leaf_has_registry_entry` |
| 4 | A `ConfigKeyMeta` entry (`config/metadata.rs`, +9) | `reference_doc_matches_registry`: `registered keys without a table row: ["ui.scratch_flag"]` |
| 5 | A row in the `[ui]` table (`docs/user/configuration-reference.md`, +1) | Full suite: 2544 run, 2544 passed, 2 skipped; `just lint` clean |

`git status --short` listed 5 modified files and `git diff --numstat` showed 19 added lines. The registry entry was 9 of those 19 lines and 1 of the 5 files. The compiler forced the two `UiSettings` consumers, and tests forced the registry and the reference row. No test or compiler error asked for the edits a feature usually also gets: `gpy-agent/SCHEMA_EVOLUTION.md` and `CHANGELOG.md` entries, the two shipped `config/*.toml` files and the "Common Keys" list in `docs/user/cli-reference.md`.

Secondary proxy, how widely an existing key spreads:

```sh
git grep -l transient_prompt | wc -l     # 18
git grep -lw transient_prompt | wc -l    # 15
```

The 3 extra files in the first count match only as a substring: `fish/core/init.fish` names `fish_transient_prompt`, and `fish/functions/fish_prompt.fish` and `zsh/core/init.zsh` use `__gpy_transient_prompt`. Of the other 15 files, those a new key of this shape must change are `config/mod.rs`, `config/metadata.rs`, `agent/events/mod.rs`, `tests/config_tests.rs` and `docs/user/configuration-reference.md` (the five from the scratch run). `theme/export/segments.rs`, `agent/events/tests.rs`, `tests/fish/transient_prompt.test.fish` and `tests/zsh/transient_prompt.test.zsh` implement or test the feature itself. The rest only mention the key: `config/config.toml`, `config/config.example.toml`, `docs/INSTALL.md`, `docs/user/bash-limitations.md`, `docs/user/cli-reference.md` and `gpy-agent/SCHEMA_EVOLUTION.md`. A single source of truth would remove the `metadata.rs` edit and, if it generated the reference table, the reference row. It would leave the other three forced edits, the feature files and the mentions.

#### Registry descriptions against doc comments

The registry's description and the first doc-comment paragraph of the same field were compared for all 42 keys. The script reads the `///` lines above each `pub` field of the seven nested settings structs and compares them with the `description:` strings and `git_icon_key!` arguments, ignoring a trailing period:

```sh
python3 - <<'EOF'
import re
mod = open('gpy-agent/src/config/mod.rs').read(); meta = open('gpy-agent/src/config/metadata.rs').read()
structs = {'agent': 'AgentSettings', 'agent.supervisor': 'SupervisorSettings', 'git': 'GitSettings',
           'git.icons': 'GitIcons', 'language': 'LanguageSettings', 'ui': 'UiSettings', 'ui.directory': 'DirectorySettings'}
docs = {}
for prefix, name in structs.items():
    doc = []
    for line in re.search(r'pub struct ' + name + r' \{(.*?)\n\}', mod, re.S).group(1).split('\n'):
        l = line.strip()
        if l.startswith('///'): doc.append(l[3:].strip())
        elif l.startswith('pub '): docs[prefix + '.' + l.split()[1].rstrip(':')] = ' '.join(doc); doc = []
        elif not l.startswith('#['): doc = []
reg = dict(re.findall(r'key: "([^"]+)",.*?description: "([^"]+)"', meta[:meta.index('pub const DYNAMIC')], re.S))
reg.update({'git.icons.' + k: v for k, v in re.findall(r'git_icon_key!\((\w+), "([^"]+)"\)', meta)})
same = [k for k in reg if docs.get(k, '').rstrip('.') == reg[k]]
print(len(reg), 'keys:', len(same), 'identical,', len(reg) - len(same), 'different')
EOF
```

Observed: 11 identical and 31 different. Most differences are a doc comment that adds a range or a default (`(5..=3600)`, `(default: "↑")`), but some disagree with the code. `language.detection_mode`'s doc comment lists `content` and `markers`, while the registry and `types::DetectionMode` also have `hybrid`. `agent.enabled` reads "Enable/disable the background agent" in the registry and "Enable background agent mode" in the doc comment. Generating `gpy config list` text from doc comments would first need an editing pass over all 31.

#### Spike: option C on three keys, without a new crate

In a fresh scratch worktree, the `CONFIG_KEYS` entries for `agent.timeout_seconds` (a range-checked newtype), `git.enabled` (a bool) and `ui.directory.display` (an enum) were deleted. `get_config_value`/`set_config_value` fell back to a walk over `toml::Table::try_from(&config)` for any key the registry no longer had. `set` parsed the CLI string by the type of the existing leaf (bool through the existing `parse_bool_field`, integer, float, string; arrays and tables read-only), inserted it into the table, and deserialized the whole `Config` again, so the newtypes' `#[serde(try_from)]` checks ran. A throwaway unit test printed the results, and then the full suite ran:

```sh
cd gpy-agent && unset CARGO_TARGET_DIR
cargo nextest run --features test-support --no-capture -E 'test(spike_option_c_probe)'
cargo nextest run --features test-support --no-fail-fast
cd .. && git diff --numstat   # after deleting the probe test
```

<details>
<summary>The spike's added code in <code>config/metadata.rs</code> (doc comments and one <code>#[allow]</code> omitted)</summary>

Hooks: `if let Some(value) = tree_get(config, key) { return value; }` after the registry loop in `get_config_value`; the same with `tree_set` in `set_config_value`; `validate_config_key` also accepts `tree_leaf(&mut config_table(&Config::default())?, key).is_some()`; and `parse_bool_field` takes `field_path: &str` instead of `&'static str`. The throwaway `spike_option_c_probe` test compared `tree_get` with each remaining entry's `get` for `Config::default()` and for a config with `language.confidence_threshold = 0.3`, `git.icon_set = nerd_font`, `language.filter = 3` and `git.skip_paths = ["/a", "/b c"]`, then printed `set_config_value` results for the inputs in the table below.

```rust
fn config_table(config: &Config) -> Result<toml::Table> {
    toml::Table::try_from(config)
        .map_err(|e| Error::config(format!("Failed to serialize config to TOML: {e}")))
}

fn tree_leaf<'t>(table: &'t mut toml::Table, key: &str) -> Option<(&'t mut toml::Table, String)> {
    if key.starts_with(LANGUAGE_ICONS_PREFIX) {
        return None;
    }
    let mut parts: Vec<&str> = key.split('.').collect();
    let name = parts.pop()?;
    let mut parent = table;
    for part in parts {
        parent = parent.get_mut(part)?.as_table_mut()?;
    }
    match parent.get(name)? {
        toml::Value::Table(_) => None,
        _ => Some((parent, name.to_owned())),
    }
}

fn render_leaf(value: &toml::Value) -> String {
    match value {
        toml::Value::String(s) => s.clone(),
        toml::Value::Array(items) => items.iter().map(render_leaf).collect::<Vec<_>>().join(" "),
        other => other.to_string(),
    }
}

fn tree_get(config: &Config, key: &str) -> Option<Result<String>> {
    let mut table = match config_table(config) {
        Ok(table) => table,
        Err(e) => return Some(Err(e)),
    };
    let (parent, name) = tree_leaf(&mut table, key)?;
    parent.get(&name).map(|leaf| Ok(render_leaf(leaf)))
}

fn tree_set(config: &mut Config, key: &str, value: &str) -> Option<Result<()>> {
    let mut table = match config_table(config) {
        Ok(table) => table,
        Err(e) => return Some(Err(e)),
    };
    let (parent, name) = tree_leaf(&mut table, key)?;
    let parsed = match parent.get(&name)? {
        toml::Value::Boolean(_) => parse_bool_field(value, key).map(toml::Value::Boolean),
        toml::Value::Integer(_) => value
            .parse::<i64>()
            .map(toml::Value::Integer)
            .map_err(|_| invalid_value(key, "integer value", value)),
        toml::Value::Float(_) => value
            .parse::<f64>()
            .map(toml::Value::Float)
            .map_err(|_| invalid_value(key, "number", value)),
        toml::Value::String(_) => Ok(toml::Value::String(value.to_owned())),
        _ => Err(ValidationError {
            field_path: key.to_owned(),
            kind: ValidationErrorKind::ReadOnlyKey,
        }
        .into()),
    };
    let parsed = match parsed {
        Ok(parsed) => parsed,
        Err(e) => return Some(Err(e)),
    };
    parent.insert(name, parsed);
    Some(
        toml::Value::Table(table)
            .try_into::<Config>()
            .map(|updated| *config = updated)
            .map_err(|e| {
                let reason = e.message().to_owned();
                Error::config(format!("Invalid {key} '{value}': {reason}"))
            }),
    )
}
```

</details>

Lines: `metadata.rs` +98 / -43 (net +55). The removals were the three registry entries (29 lines: 11 + 9 + 9), the now-unused `parse_directory_display` helper (13) and one signature line. The additions were the tree walk, render, parse and deserialize code (88 lines in one block), three-line hooks in `get_config_value`, `set_config_value` and `validate_config_key`, and the widened `parse_bool_field` signature. For three keys, the generic code cost more lines than the registry entries it removed.

Error text pinned by tests:

| Command | Today | Spike |
|---|---|---|
| `set git.enabled maybe` | `Invalid boolean value: maybe` | unchanged |
| `set agent.timeout_seconds abc` | `Invalid u64 value: abc` | `Invalid integer value: abc` |
| `set agent.timeout_seconds 0` | `Invalid agent timeout: 0` | `Invalid agent.timeout_seconds '0': agent timeout must be between 1 and 300` |
| `set ui.directory.display sideways` | `Invalid directory display mode: unrecognized directory display mode: sideways` | ``Invalid ui.directory.display 'sideways': unknown variant `sideways`, expected one of `basename`, `abbreviated`, `truncated`, `full` `` |

`tests/cli_error_messages_tests.rs` still passed. It only requires one `Error:` line naming the bad input, for `config set git.enabled maybe` and `config get nope.key`. The exact strings pinned by `set_config_value_error_messages` did not survive: all three of its cases on spiked keys changed. The full suite ran 2545 tests: 2541 passed and 4 failed. The failures were `set_config_value_error_messages` and the three registry guards (`every_config_leaf_has_registry_entry`, `reference_doc_matches_registry`, `cli_reference_common_keys_are_registered`), which are written against `CONFIG_KEYS` and would have to be rewritten against the tree. `config_list_matches_registry` passed only because it compares `gpy config list` with `CONFIG_KEYS`, which no longer had the three keys. `commands::config::list` iterates `CONFIG_KEYS`, so the three keys would drop out of the listing along with their descriptions. That follows from the code; the listing was not run.

`get` over the tree matched the registry for every remaining key on both a default and a customised config, except `language.confidence_threshold`. That `f32` serializes through TOML's `f64` and printed `0.10000000149011612` instead of `0.1` (and `0.30000001192092896` instead of `0.3`).

The spike also changes which inputs are accepted (from reading the code, not run): `DirectoryDisplay::from_str`, used by `set` today, lowercases its input, but serde's `rename_all = "lowercase"` does not, so `gpy config set ui.directory.display FULL` would stop working. `git.icon_set` and `language.display` have the same shape.

Build time, binary size and latency were not measured: three keys cannot tell the options apart.

#### `schemars`, from its public documentation

No spike was run with `schemars`; that would add a dependency and needs owner approval. From crates.io and the schemars documentation (<https://graham.cool/schemars/>, read 2026-10-23):

- `schemars` 1.2.3 (2026-10-12) is MIT, with MSRV 1.74. Its default features `derive` and `std` pull in `schemars_derive` 1.2.3 (MIT), which needs `syn` 3, `quote`, `proc-macro2` and `serde_derive_internals` 0.30. `schemars` itself also needs `dyn-clone` and `ref-cast` (which needs `ref-cast-impl`), all `MIT OR Apache-2.0`.
- `gpy-agent/deny.toml` allows both licences (`[licenses] allow` lists MIT and Apache-2.0) and already skip-trees `syn` and `proc-macro2`. `syn` 3.0.9 is already in `Cargo.lock` (through `serde_derive`, `clap_derive`, `thiserror-impl`, `tokio-macros`), so the derive would add 6 crates (`schemars`, `schemars_derive`, `serde_derive_internals`, `dyn-clone`, `ref-cast`, `ref-cast-impl`) but no second `syn`. `cargo deny check` was not run with it.
- The derive uses `///` doc comments as each property's `description` and honours `#[serde(default, try_from, transparent, rename_all)]`. Every type in the `Config` tree would need `JsonSchema`: `Config` and its 7 nested settings structs, `LanguageIcons`, `GitIconSet` and the 17 `types::` newtypes and enums. `LanguageFilter` and `Icon` implement `Deserialize` by hand, so they would need `#[schemars(with = ...)]` or a hand-written impl.
- `schemars` produces a JSON Schema. It does not provide get/set or CLI parsing, so it would cover only the `description` column of option C.

## Decision

Proposed: option A. Keep `CONFIG_KEYS` and the guards that already exist.

- The registry is 1 file and 9 of the 19 lines a new boolean key costs. If it is forgotten, `every_config_leaf_has_registry_entry` fails with the key's name, and the reference row is checked the same way.
- Option C, the one design that keeps ordinary structs and adds neither a macro nor a crate, added 55 net lines for three keys. It changed all three pinned error strings it touched, would drop those keys from `gpy config list`, printed a widened float and would reject spellings the CLI accepts today. It meets neither part of the bar set when this ADR was filed: a net deletion and identical error text.
- Options B and D would remove the registry edit too, but not the field-plus-`Default` edit or the exhaustive-destructure and struct-literal edits the compiler forced in the scratch run. Both add macro machinery, B inside the crate and D as a new crate, and neither was spiked.

The owner records the decision on the ADR's pull request. If option A is accepted there is nothing to implement, so no follow-up epic is filed. Signs that should reopen this ADR: the key count roughly doubles; descriptions are to feed generated documentation; or a further front end (a TUI editor, a JSON Schema for editors) needs the key list.

## Alternatives Considered

### Option A

**Keep the registry and the existing guards.** Each key keeps its hand-written `ConfigKeyMeta` (or a small `macro_rules!` like `git_icon_key!` for a uniform family). The tests listed under Context check structs, registry, reference tables and shipped configs against each other. Cost: 6 to 20 registry lines per key, and descriptions that can drift from doc comments without failing a test (31 of 42 differ today). This is the proposed decision.

### Option B

**A declarative `macro_rules!` that declares each leaf once.** One macro invocation per settings struct would generate the struct with its serde attributes and doc comments, its `Default`, and the matching `CONFIG_KEYS` entries. Each leaf would state its type, default, CLI parser, error label and description, and the error label is enough to keep today's error text. `git_icon_key!` already does this for one uniform family (16 lines for 9 keys). Not spiked. Costs, from reading the code: the eight settings structs move into macro syntax, which `rustfmt` does not format and rust-analyzer handles less well; struct-level attributes such as `#[expect(clippy::struct_excessive_bools)]` and the hand-written `Default` impls must pass through the macro; and adding a key still forces the destructure in `ui_refresh_required` and the struct literals in `tests/config_tests.rs`, because those name the fields. Rejected: it moves the registry's lines into a harder-to-read macro and saves the one edit the tests already enforce.

### Option C

**Generic get/set over the serialized tree.** Walk `toml::Table::try_from(&config)` for `get`. For `set`, parse the string by the leaf's TOML type, merge it into the tree and deserialize, so the validating newtypes run through serde. Take descriptions from doc comments, perhaps through `schemars`. The spike above measured +98 / -43 lines for three keys and changed error text, and it showed the follow-on work: a description source for `gpy config list` (a second table, or `schemars` plus editing the 31 differing doc comments); per-type error labels if the exact text matters; special cases for `f32` rendering and case-insensitive `FromStr` parsing; and rewriting the four registry guards. Rejected: no net deletion, and the error text changes.

### Option D

**A proc-macro derive** (for example `#[derive(ConfigKeys)]` on the settings structs, reading `///` docs and a `#[config(parse = ..., label = ...)]` attribute). A proc macro must live in its own `proc-macro = true` crate. `gpy-agent` is a single crate with no Cargo workspace, so this would add a crate, a `Cargo.toml`, `Cargo.lock` changes and a dependency on `syn`/`quote`. `syn` 3 is already in the lock, so the cost is mainly maintaining the macro, not compile time. It could generate everything option B generates while keeping ordinary struct syntax, but adding a key would still force the same compiler edits. Not spiked. Rejected: the most machinery of the four options, for the same saving as B.

## Consequences

### Positive

- No code, dependency or `Config` schema change.
- Today's exact `gpy config set` error text, case-insensitive enum input and `0.1`-style float output stay as they are.
- Leaving a key out of the registry or the reference tables keeps failing a named test.

### Negative

- Every new key keeps paying 6 to 20 registry lines in a second file.
- Registry descriptions and `///` doc comments can keep drifting without a test noticing; 31 of 42 differ today.

## References

- [ADR-0009](adr-0009-deferred-language-version-probes.md): layout this ADR follows
- `gpy-agent/src/config/metadata.rs` (`CONFIG_KEYS`, `git_icon_key!` and the registry guards), `gpy-agent/src/config/mod.rs` (the settings structs)
- `gpy-agent/tests/cli_error_messages_tests.rs`, `gpy-agent/tests/cli_integration_tests.rs` (`config_list_matches_registry`), `gpy-agent/tests/config_loader_tests.rs`
- [Configuration reference](../../user/configuration-reference.md)
- Ghostty `src/config/Config.zig`: <https://github.com/ghostty-org/ghostty/blob/main/src/config/Config.zig>
- schemars: <https://graham.cool/schemars/>, <https://crates.io/crates/schemars>

# Schema Evolution

Rules for changing the formats GPY reads and writes. Individual changes are
recorded in [CHANGELOG.md](../CHANGELOG.md), not here.

## Contracts covered

| Contract | Defined by | Published form |
|---|---|---|
| IPC protocol | `src/ipc/protocol/mod.rs`, `Response` in `src/ipc/mod.rs` | `schemas/message.json`, `schemas/response.json` |
| Agent config | `src/config/` | `~/.config/gpy/config.toml`; keys in [configuration-reference.md](../docs/user/configuration-reference.md) |
| Themes and palettes | `src/theme/model/`, `src/palette/` | builtins in `config/themes/`, `config/palettes/` |
| Plugin manifests | `src/plugin/` | `schemas/plugin-manifest.json`, `schemas/script-segment-manifest.json` |
| Shell contract | `src/theme/export/` | variables printed by `gpy-agent theme export` |
| CLI | `src/bin/gpy.rs`, `src/main.rs`, `src/commands/` | [cli-reference.md](../docs/user/cli-reference.md) |

## Change classes

Every change to a contract is exactly one of these. The class is recorded in
`CHANGELOG.md`, where BREAKING changes are marked **Breaking:**.

| Class | Meaning | Examples |
|---|---|---|
| BREAKING | Existing input, output or a shipped client stops working or changes meaning | Removing or renaming a field, op, format value or exported variable; changing a field's type; narrowing accepted values; a `PROTOCOL_VERSION` bump |
| MINOR | Additive; existing input loads and behaves as before | New optional field with a serde default; new IPC op; new optional request or response field; new CLI subcommand or flag |
| PATCH | No change in shape or accepted values | Bug fix, rendering fix, documentation |

The crate version in `Cargo.toml` follows Semantic Versioning over these
classes. Before 1.0, a BREAKING change bumps the minor component.

## Compatibility guarantees

- **Config and theme files.** Unknown keys are ignored when loading (GPY
  config and theme types do not use `deny_unknown_fields`); `gpy doctor`
  reports unknown config keys. A MINOR field takes its serde default when
  absent, so older files load unchanged.
- **Plugin manifests.** `plugin.toml` and `segments/<id>.toml` reject unknown
  keys. A theme's `[segments.<plugin-segment>]` table accepts only style keys.
- **IPC requests.** Every request is one newline-terminated JSON object with an
  `op` field. Unknown fields are ignored. An unknown `op` gets
  `{"error":"... Unknown shell operation: <op>"}`. An unrecognised `format`
  falls back to `json`.
- **IPC replies.** A `format: "json"` reply is a flat object (`{"status":"ok"}`,
  `{"error":"..."}`, `{"branch":...}`); only the agent status reply is wrapped,
  as `{"AgentStatus":{...}}`, in every format. Other formats reply with one line
  of rendered text, with newlines encoded as `\x1e` (`REPLY_NEWLINE`).
- **Version skew.** A new optional request field must keep its old meaning when
  absent, and an old agent must degrade sensibly when it ignores the field.
  Shells treat a reply starting with `{"error"` as no reply and draw their
  local fallback, so a new op sent to an old agent is MINOR.

## Protocol version

`PROTOCOL_VERSION` in `src/ipc/protocol/mod.rs` is currently `2`. The agent
reports it in the status reply as `protocol_version`; the shells compare it
for equality with the version they were built for and restart a mismatched
agent. There is no negotiation, so a bump forces every running shell to restart
its agent.

Bump it only when an existing message, reply or notification changes in a way
a shipped shell would misread. Do not bump it for MINOR additions.

Version 2 defines the notification channel: the agent sends only SIGURG
(ignored by default) and carries meaning in empty flag files under
`<runtime_root>/shells/`, `<pid>.reload` and `<pid>.reregister`, each written
before the signal.

## Change checklist

1. Classify the change as BREAKING, MINOR or PATCH using the table above.
2. Give new config, theme and request fields a serde default that preserves
   current behaviour; do not add `deny_unknown_fields` to config, theme or IPC
   types.
3. Update the published form: `schemas/*.json`, `config/themes/`,
   `config/palettes/`, and the user docs for the contract.
4. Regenerate the plugin manifest schemas with
   `GPY_UPDATE_SCHEMAS=1 cargo nextest run --features test-support schema_tests`
   and review the diff.
5. Run the schema tests from `gpy-agent/`:

   ```bash
   cargo nextest run --features test-support --test schema_validation_tests \
     --test ipc_protocol_tests --test config_tests --test theme_schema_tests
   cargo nextest run --features test-support --lib -E 'test(/schema_tests/)'
   ```

6. Add a `CHANGELOG.md` entry under `[Unreleased]`, prefixed **Breaking:** for
   a BREAKING change, stating what an existing config, theme or shell must do.

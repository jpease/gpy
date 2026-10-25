# GPY Agent IPC Protocol Schemas

This directory contains JSON Schema files that define the IPC protocol between Fish shell and the GPY Agent.

## Schema Files

- **`message.json`**: Defines all message types that can be sent from Fish shell to the Agent
- **`response.json`**: Defines the replies the Agent writes for `format: "json"` requests (flat objects, `{"status":"ok"}`, `{"error":"..."}`; only `AgentStatus` is wrapped, as `{"AgentStatus":{...}}`, and a status request gets that JSON in every format). Any other format replies with one line of rendered text

## Plugin Manifest Schemas

- **`plugin-manifest.json`**: `plugin.toml`.
- **`script-segment-manifest.json`**: `segments/<id>.toml` of a script plugin.

Both are generated from the Rust types; `plugin::schema_tests` fails when they
drift (regenerate with `GPY_UPDATE_SCHEMAS=1 cargo nextest run schema_tests`).

## Protocol Formats

Every message is one JSON object with an `op` field naming the operation:

```json
{"op":"git","cwd":"/path/to/repo","format":"ansi"}
```

## Validation

The schemas can be used with standard JSON Schema validators to ensure protocol compliance. `gpy-agent/tests/schema_validation.rs` checks them against the code: the `format` enum against `Format`, the ops against the `into_wire_message` dispatch table, and real agent replies against `response.json`.

An unrecognized `format` silently falls back to `json`.

## Version Compatibility

These schemas correspond to protocol version 1 (see `gpy-agent/src/ipc/protocol/mod.rs`).

## Usage

External tools can use these schemas to:

1. Validate IPC messages before sending them to the Agent
2. Generate client libraries for other shells (Bash, Zsh)
3. Test protocol compliance
4. Generate documentation

## Examples

See the doc comments in `gpy-agent/src/ipc/mod.rs` for examples of each message and response type.

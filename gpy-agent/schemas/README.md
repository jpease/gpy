# GPY Agent IPC Protocol Schemas

This directory contains JSON Schema files that define the IPC protocol between Fish shell and the GPY Agent.

## Schema Files

- **`message.json`**: Defines all message types that can be sent from Fish shell to the Agent
- **`response.json`**: Defines the replies the Agent writes for `format: "json"` requests (flat objects, `{"status":"ok"}`, `{"error":"..."}`; only `AgentStatus` is wrapped). Any other format replies with one line of rendered text

## Protocol Formats

The protocol supports two message formats for backward compatibility:

### Fish Shell Format (Legacy)

Messages use an object with an `op` field to specify the operation:

```json
{"op":"git","cwd":"/path/to/repo","format":"ansi"}
```

### Native Rust Format

Messages use Rust enum serialization:

```json
{"RepositoryStatus":{"path":"/path/to/repo","format":"json"}}
```

## Validation

The schemas can be used with standard JSON Schema validators to ensure protocol compliance. `gpy-agent/tests/schema_validation.rs` checks them against the code: the `format` enum against `Format`, the ops against the `into_wire_message` dispatch table, and real agent replies against `response.json`.

In the `op` form an unrecognized `format` silently falls back to `json`; in the native form it rejects the message. `bash-source` and `zsh-source` parse but are not implemented.

## Version Compatibility

These schemas correspond to protocol version 1 (see `gpy-agent/src/ipc/protocol.rs`).

## Usage

External tools can use these schemas to:

1. Validate IPC messages before sending them to the Agent
2. Generate client libraries for other shells (Bash, Zsh)
3. Test protocol compliance
4. Generate documentation

## Examples

See the doc comments in `gpy-agent/src/ipc/mod.rs` for comprehensive examples of each message and response type.

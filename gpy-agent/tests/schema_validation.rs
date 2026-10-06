//! Schema validation tests
//!
//! This test suite validates that all JSON examples in the documentation
//! can be successfully serialized/deserialized and match the protocol schemas.

#![allow(clippy::missing_panics_doc)]
#![allow(clippy::unwrap_used)]

use gpy_agent::config::types::ClientPid;
use gpy_agent::ipc::{Message, Response};
use gpy_agent::security::SafePath;

// ── Message deserialization — Fish shell wire format ──────────────────────────

#[test]
fn test_msg_deserialize_repository_status_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let cases: &[(&[u8], &str)] = &[
        (br#"{"op":"git","cwd":"."}"#, "RepositoryStatus (minimal)"),
        (
            br#"{"op":"git","cwd":"/home/user/repo","format":"ansi","is_last":false}"#,
            "RepositoryStatus (full)",
        ),
    ];
    for (input, label) in cases {
        let result = deserialize_message(input);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_msg_deserialize_language_detect_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let cases: &[(&[u8], &str)] = &[
        (br#"{"op":"lang","cwd":"."}"#, "LanguageDetect (minimal)"),
        (
            br#"{"op":"lang","cwd":"/home/user/project","format":"fish-source","is_last":true}"#,
            "LanguageDetect (full)",
        ),
    ];
    for (input, label) in cases {
        let result = deserialize_message(input);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_msg_deserialize_client_registration_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let pid = std::process::id();

    {
        let json = format!(r#"{{"op":"register","pid":{pid},"cwd":"/home/user/project"}}"#);
        let result = deserialize_message(json.as_bytes());
        assert!(
            result.is_ok(),
            "Failed to deserialize RegisterClient (with cwd): {:?}",
            result.err()
        );
    }
    {
        let json = format!(r#"{{"op":"register","pid":{pid}}}"#);
        let result = deserialize_message(json.as_bytes());
        assert!(
            result.is_ok(),
            "Failed to deserialize RegisterClient (without cwd): {:?}",
            result.err()
        );
    }
    {
        let json = format!(r#"{{"op":"unregister","pid":{pid}}}"#);
        let result = deserialize_message(json.as_bytes());
        assert!(
            result.is_ok(),
            "Failed to deserialize UnregisterClient: {:?}",
            result.err()
        );
    }
    {
        let json = format!(r#"{{"op":"workspace","pid":{pid},"cwd":"/home/user/new-project"}}"#);
        let result = deserialize_message(json.as_bytes());
        assert!(
            result.is_ok(),
            "Failed to deserialize WorkspaceUpdate: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_msg_deserialize_simple_ops_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let cases: &[(&[u8], &str)] = &[
        (br#"{"op":"ping"}"#, "Ping"),
        (br#"{"op":"status"}"#, "Status"),
        (br#"{"op":"theme","key":"prompt_close"}"#, "ThemeQuery"),
        (br#"{"op":"shutdown"}"#, "Shutdown"),
        (br#"{"op":"config_reload"}"#, "ConfigReload"),
    ];
    for (input, label) in cases {
        let result = deserialize_message(input);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

// ── Response deserialization ──────────────────────────────────────────────────

#[test]
fn test_response_deserialize_repository_status() {
    let cases: &[(&str, &str)] = &[
        (
            r#"{"RepositoryStatus":{"branch":"main","ahead":2,"behind":0,"ahead_capped":false,"behind_capped":false,"staged":3,"unstaged":1,"untracked":2,"conflicts":0,"state":"clean"}}"#,
            "RepositoryStatus (with changes)",
        ),
        (
            r#"{"RepositoryStatus":{"branch":"feature/new-feature","ahead":0,"behind":0,"ahead_capped":false,"behind_capped":false,"staged":0,"unstaged":0,"untracked":0,"conflicts":0,"state":"clean"}}"#,
            "RepositoryStatus (clean)",
        ),
        (
            r#"{"RepositoryStatus":{"branch":"feature/complex-merge","ahead":0,"behind":0,"ahead_capped":false,"behind_capped":false,"staged":0,"unstaged":2,"untracked":0,"conflicts":1,"state":"rebasing"}}"#,
            "RepositoryStatus (rebase)",
        ),
    ];
    for (json, label) in cases {
        let result: Result<Response, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_response_deserialize_language() {
    let cases: &[(&str, &str)] = &[
        (
            r##"{"Language":{"languages":[{"name":"Rust","version":"1.70.0","color":"#dea584"}]}}"##,
            "Language (single)",
        ),
        (
            r##"{"Language":{"languages":[{"name":"JavaScript","version":"20.10.0","color":"#f1e05a"},{"name":"TypeScript","version":"5.3.3","color":"#3178c6"}]}}"##,
            "Language (multiple)",
        ),
        (r#"{"Language":{"languages":[]}}"#, "Language (none)"),
    ];
    for (json, label) in cases {
        let result: Result<Response, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

#[test]
fn test_response_deserialize_simple_variants() {
    let cases: &[(&str, &str)] = &[
        (r#""Ack""#, "Ack"),
        (
            r#"{"AgentStatus":{"version":"0.1.0","protocol_version":1,"watched_repos":5,"registered_clients":3,"cache_entries":12}}"#,
            "AgentStatus",
        ),
        (r#"{"ThemeValue":{"value":""}}"#, "ThemeValue (delimiter)"),
        (
            r##"{"ThemeValue":{"value":"#ff6b6b"}}"##,
            "ThemeValue (color)",
        ),
        (
            r#"{"Error":{"message":"Failed to detect language: directory not found"}}"#,
            "Error (generic)",
        ),
        (
            r#"{"Error":{"message":"IPC connection failed: socket timeout"}}"#,
            "Error (IPC)",
        ),
    ];
    for (json, label) in cases {
        let result: Result<Response, _> = serde_json::from_str(json);
        assert!(
            result.is_ok(),
            "Failed to deserialize {label}: {:?}",
            result.err()
        );
    }
}

// ── Round-trip tests ──────────────────────────────────────────────────────────

#[test]
fn test_message_roundtrip() {
    let pid_val = std::process::id();
    let pid = ClientPid::new(pid_val).unwrap();

    let messages = vec![
        Message::RepositoryStatus {
            path: SafePath::new("/home/user/repo").unwrap(),
            format: gpy_agent::formatter::Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
        },
        Message::LanguageDetect {
            path: SafePath::new("/home/user/project").unwrap(),
            format: gpy_agent::formatter::Format::FishSource,
            is_last: true,
            is_first: false,
            prev_bg: None,
            virtual_env: None,
        },
        Message::RegisterClient {
            pid,
            cwd: Some(SafePath::new("/home/user/project").unwrap()),
            shell: None,
            shell_version: None,
        },
        Message::UnregisterClient { pid },
        Message::WorkspaceUpdate {
            pid,
            cwd: SafePath::new("/home/user/new-project").unwrap(),
        },
        Message::Ping,
        Message::Status,
        Message::ThemeQuery {
            key: "prompt_close".to_owned(),
        },
        Message::Shutdown,
        Message::ConfigReload,
    ];

    for msg in messages {
        let json = serde_json::to_string(&msg).expect("Failed to serialize message");
        let deserialized: Message =
            serde_json::from_str(&json).expect("Failed to deserialize message");
        let json2 = serde_json::to_string(&deserialized).expect("Failed to re-serialize message");
        assert_eq!(json, json2, "Roundtrip mismatch for message: {msg:?}");
    }
}

#[test]
fn test_response_roundtrip() {
    let responses = vec![
        Response::RepositoryStatus(gpy_agent::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 2,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 3,
            unstaged: 1,
            untracked: 2,
            conflicts: 0,
            state: gpy_agent::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        }),
        Response::Language {
            languages: vec![gpy_agent::ipc::LanguageInfo {
                name: "Rust".to_owned(),
                version: Some("1.70.0".to_owned()),
                color: gpy_agent::config::types::ColorSpec::new("#dea584").unwrap(),
            }],
        },
        Response::Ack,
        Response::AgentStatus {
            version: "0.1.0".to_owned(),
            protocol_version: 1,
            watched_repos: 5,
            registered_clients: 3,
            cache_entries: 12,
        },
        Response::ThemeValue {
            value: String::new(),
        },
        Response::Error {
            message: "Test error".to_owned(),
        },
    ];

    for resp in responses {
        let json = serde_json::to_string(&resp).expect("Failed to serialize response");
        let deserialized: Response =
            serde_json::from_str(&json).expect("Failed to deserialize response");
        let json2 = serde_json::to_string(&deserialized).expect("Failed to re-serialize response");
        assert_eq!(json, json2, "Roundtrip mismatch for response: {resp:?}");
    }
}

// ── Protocol validation ───────────────────────────────────────────────────────

#[test]
fn test_protocol_validation_catches_errors() {
    use gpy_agent::ipc::protocol::deserialize_message;

    // Above ClientPid::MAX (4_194_304): rejected at parse time by range
    // validation. 2_000_000 used to fail here too, but only because
    // `ClientPid::new` ran a liveness probe at construction; #578 moved
    // liveness checking to route time (`route_request_secure`), so
    // deserialization alone no longer rejects an in-range-but-dead PID.
    let result = deserialize_message(br#"{"op":"register","pid":5000000}"#);
    assert!(result.is_err(), "Should reject out-of-range PID");

    let long_path = "a".repeat(5000);
    let json = format!(r#"{{"op":"git","cwd":"{long_path}"}}"#);
    let result_long = deserialize_message(json.as_bytes());
    assert!(result_long.is_err(), "Should reject overly long path");

    let result_missing = deserialize_message(br#"{"op":"workspace","pid":12345}"#);
    assert!(
        result_missing.is_err(),
        "Should reject missing required field"
    );

    let result_unknown = deserialize_message(br#"{"op":"invalid_operation"}"#);
    assert!(result_unknown.is_err(), "Should reject unknown operation");
}

// ── Published JSON schema files (`schemas/*.json`) ────────────────────────────
//
// No JSON-Schema validator crate is a dev-dependency (and none may be added),
// so `validate` below implements the small draft-07 subset the schemas use.

mod schema_files {
    #![allow(clippy::indexing_slicing)]
    #![allow(
        clippy::too_many_lines,
        reason = "sample tables and a schema-subset validator read best undivided"
    )]

    use gpy_agent::config::Config;
    use gpy_agent::formatter::{Format, RenderContext, SegmentPosition, create_formatter};
    use gpy_agent::git::{RepositoryState, RepositoryStatus};
    use gpy_agent::ipc::protocol::{deserialize_message, serialize_response};
    use gpy_agent::ipc::{LanguageInfo, Response};
    use gpy_agent::theme::ThemeConfig;
    use serde_json::Value;
    use std::collections::BTreeSet;
    use std::str::FromStr;

    fn load(name: &str) -> Value {
        let path = format!("{}/schemas/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    /// Source of `ipc/protocol.rs`: the op table and `WireMessage` live there.
    fn protocol_source() -> String {
        let path = format!("{}/src/ipc/protocol.rs", env!("CARGO_MANIFEST_DIR"));
        std::fs::read_to_string(path).unwrap()
    }

    fn one_of(schema: &Value) -> &Vec<Value> {
        schema["oneOf"].as_array().unwrap()
    }

    /// Every `Format` variant. The match has no wildcard, so adding a variant
    /// fails to compile here until it is listed (and then the schema test
    /// fails until `message.json` lists it too).
    fn all_formats() -> Vec<Format> {
        let all = vec![
            Format::Json,
            Format::Ansi,
            Format::BashPrompt,
            Format::ZshPrompt,
            Format::Fish,
            Format::FishSource,
            Format::BashSource,
            Format::Zsh,
            Format::ZshSource,
        ];
        for format in &all {
            match format {
                Format::Json
                | Format::Ansi
                | Format::BashPrompt
                | Format::ZshPrompt
                | Format::Fish
                | Format::FishSource
                | Format::BashSource
                | Format::Zsh
                | Format::ZshSource => {}
            }
        }
        all
    }

    /// Collect every `format` property schema reachable from `value`.
    fn collect_format_properties<'a>(value: &'a Value, out: &mut Vec<&'a Value>) {
        match value {
            Value::Object(map) => {
                if let Some(format) = map.get("properties").and_then(|p| p.get("format")) {
                    out.push(format);
                }
                for child in map.values() {
                    collect_format_properties(child, out);
                }
            }
            Value::Array(items) => {
                for child in items {
                    collect_format_properties(child, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn schema_format_enum_matches_format_type() {
        let schema = load("message.json");

        // Every `format` property must point at the one shared enum.
        let mut sites = Vec::new();
        collect_format_properties(&schema, &mut sites);
        assert!(
            sites.len() >= 16,
            "expected many format sites: {}",
            sites.len()
        );
        for site in &sites {
            assert_eq!(
                site["$ref"], "#/definitions/Format",
                "inline format: {site}"
            );
        }

        let listed: Vec<&str> = schema["definitions"]["Format"]["enum"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for value in &listed {
            assert!(
                Format::from_str(value).is_ok(),
                "message.json lists `{value}`, which Format::from_str rejects"
            );
        }
        let listed_set: BTreeSet<&str> = listed.iter().copied().collect();
        assert_eq!(
            listed_set.len(),
            listed.len(),
            "duplicate format enum value"
        );
        let formats: BTreeSet<String> = all_formats().iter().map(ToString::to_string).collect();
        let listed_owned: BTreeSet<String> = listed.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(listed_owned, formats, "format enum != Format values");
        for variant in <Format as clap::ValueEnum>::value_variants() {
            assert!(formats.contains(&variant.to_string()));
        }
    }

    /// Ops accepted by `ShellIpcMessage::into_wire_message`, parsed from the
    /// `"op" =>` arms of its dispatch table so no hand-kept list can drift.
    fn ops_from_code() -> BTreeSet<String> {
        let source = protocol_source();
        let table = source
            .split("match self.op.as_str() {")
            .nth(1)
            .unwrap()
            .split("_ => Err(Error::ipc(format!(\"Unknown shell operation")
            .next()
            .unwrap();
        let ops: BTreeSet<String> = regex::Regex::new(r#"(?m)^\s{12}"([a-z_]+)" =>"#)
            .unwrap()
            .captures_iter(table)
            .map(|c| c[1].to_owned())
            .collect();
        assert!(ops.len() >= 17, "op table parse found only {ops:?}");
        ops
    }

    /// `WireMessage` variant names (the native form), parsed from the enum.
    fn native_variants_from_code() -> BTreeSet<String> {
        let source = protocol_source();
        let body = source
            .split("enum WireMessage {")
            .nth(1)
            .unwrap()
            .split("\nimpl WireMessage")
            .next()
            .unwrap();
        let variants: BTreeSet<String> = regex::Regex::new(r"(?m)^    ([A-Z][A-Za-z]+)(?: \{|,)$")
            .unwrap()
            .captures_iter(body)
            .map(|c| c[1].to_owned())
            .collect();
        assert!(
            variants.len() >= 17,
            "variant parse found only {variants:?}"
        );
        variants
    }

    #[test]
    fn schema_lists_every_shell_op() {
        let schema = load("message.json");
        let mut schema_ops = BTreeSet::new();
        let mut schema_native = BTreeSet::new();
        for entry in one_of(&schema) {
            if let Some(op) = entry["properties"]["op"]["const"].as_str() {
                assert!(schema_ops.insert(op.to_owned()), "duplicate op entry {op}");
            } else if let Some(name) = entry["const"].as_str() {
                schema_native.insert(name.to_owned());
            } else {
                let keys = entry["required"].as_array().unwrap();
                assert_eq!(keys.len(), 1, "native entry must name one variant");
                schema_native.insert(keys[0].as_str().unwrap().to_owned());
            }
        }
        assert_eq!(
            schema_ops,
            ops_from_code(),
            "message.json ops != into_wire_message ops"
        );
        assert_eq!(
            schema_native,
            native_variants_from_code(),
            "message.json native variants != WireMessage variants"
        );
    }

    /// Draft-07 subset: `$ref`, `oneOf`, `type`, `const`, `enum`, `required`,
    /// `properties`, `additionalProperties: false`, `items`, `minimum`,
    /// `maximum`, `maxLength`.
    fn validate(root: &Value, schema: &Value, value: &Value) -> bool {
        if let Some(reference) = schema.get("$ref").and_then(Value::as_str) {
            let mut target = root;
            for part in reference.trim_start_matches("#/").split('/') {
                target = &target[part];
            }
            return validate(root, target, value);
        }
        if let Some(branches) = schema.get("oneOf").and_then(Value::as_array)
            && branches.iter().filter(|b| validate(root, b, value)).count() != 1
        {
            return false;
        }
        if let Some(ty) = schema.get("type") {
            let names: Vec<&str> = match ty {
                Value::String(s) => vec![s.as_str()],
                other => other
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(Value::as_str)
                    .collect(),
            };
            let ok = names.iter().any(|name| match *name {
                "string" => value.is_string(),
                "integer" => value.is_i64() || value.is_u64(),
                "boolean" => value.is_boolean(),
                "null" => value.is_null(),
                "object" => value.is_object(),
                "array" => value.is_array(),
                // An unsupported type keyword rejects everything, failing loudly.
                _ => false,
            });
            if !ok {
                return false;
            }
        }
        if schema.get("const").is_some_and(|c| c != value) {
            return false;
        }
        if let Some(allowed) = schema.get("enum").and_then(Value::as_array)
            && !allowed.contains(value)
        {
            return false;
        }
        if let Some(number) = value.as_i64() {
            if schema
                .get("minimum")
                .and_then(Value::as_i64)
                .is_some_and(|m| number < m)
            {
                return false;
            }
            if schema
                .get("maximum")
                .and_then(Value::as_i64)
                .is_some_and(|m| number > m)
            {
                return false;
            }
        }
        if let (Some(text), Some(max)) = (
            value.as_str(),
            schema.get("maxLength").and_then(Value::as_u64),
        ) && u64::try_from(text.chars().count()).unwrap() > max
        {
            return false;
        }
        if let Some(object) = value.as_object() {
            if let Some(required) = schema.get("required").and_then(Value::as_array)
                && required
                    .iter()
                    .any(|k| !object.contains_key(k.as_str().unwrap()))
            {
                return false;
            }
            let props = schema.get("properties").and_then(Value::as_object);
            for (key, child) in object {
                match props.and_then(|p| p.get(key)) {
                    Some(sub) => {
                        if !validate(root, sub, child) {
                            return false;
                        }
                    }
                    None if schema.get("additionalProperties") == Some(&Value::Bool(false)) => {
                        return false;
                    }
                    None => {}
                }
            }
        }
        if let (Some(items), Some(sub)) = (value.as_array(), schema.get("items"))
            && items.iter().any(|item| !validate(root, sub, item))
        {
            return false;
        }
        true
    }

    fn json(text: &str) -> Value {
        serde_json::from_str(text).unwrap()
    }

    #[test]
    fn message_schema_validates_real_messages() {
        let schema = load("message.json");
        // One shell-form sample per op; keys must cover `ops_from_code`.
        let shell: &[(&str, &str)] = &[
            (
                "git",
                r#"{"op":"git","cwd":".","format":"ansi","is_last":true,"is_first":false,"prev_bg":"blue"}"#,
            ),
            (
                "lang",
                r#"{"op":"lang","cwd":".","format":"fish-source","virtual_env":"/v"}"#,
            ),
            (
                "directory",
                r#"{"op":"directory","cwd":".","format":"zsh"}"#,
            ),
            (
                "clock",
                r#"{"op":"clock","shell":"zsh","format":"bash-prompt"}"#,
            ),
            (
                "duration",
                r#"{"op":"duration","duration_ms":1500,"format":"ansi"}"#,
            ),
            (
                "character",
                r#"{"op":"character","success":true,"format":"zsh-prompt"}"#,
            ),
            (
                "hostname",
                r#"{"op":"hostname","hostname":"box","format":"ansi"}"#,
            ),
            (
                "username",
                r#"{"op":"username","username":"me","format":"ansi"}"#,
            ),
            ("ping", r#"{"op":"ping"}"#),
            ("status", r#"{"op":"status"}"#),
            ("shutdown", r#"{"op":"shutdown"}"#),
            ("config_reload", r#"{"op":"config_reload"}"#),
            (
                "register",
                r#"{"op":"register","pid":123,"cwd":".","shell":"fish","shell_version":"4.0"}"#,
            ),
            ("unregister", r#"{"op":"unregister","pid":123}"#),
            ("workspace", r#"{"op":"workspace","pid":123,"cwd":"."}"#),
            ("theme", r#"{"op":"theme","key":"git.branch"}"#),
            ("latency_stats", r#"{"op":"latency_stats"}"#),
        ];
        let sampled: BTreeSet<String> = shell.iter().map(|(op, _)| (*op).to_owned()).collect();
        assert_eq!(sampled, ops_from_code(), "add a sample for the new op");
        for (op, text) in shell {
            assert!(
                validate(&schema, &schema, &json(text)),
                "schema rejects {op}: {text}"
            );
            assert!(
                deserialize_message(text.as_bytes()).is_ok(),
                "agent rejects {op}: {text}"
            );
        }
        let native: &[&str] = &[
            r#"{"RepositoryStatus":{"path":".","format":"ansi"}}"#,
            r#"{"LanguageDetect":{"path":".","virtual_env":null}}"#,
            r#"{"DirectoryRequest":{"path":"."}}"#,
            r#"{"ClockRequest":{"shell":"bash"}}"#,
            r#"{"DurationRequest":{"duration_ms":5}}"#,
            r#"{"CharacterRequest":{"success":false}}"#,
            r#"{"HostnameRequest":{"hostname":"h"}}"#,
            r#"{"UsernameRequest":{"username":"u"}}"#,
            r#"{"RegisterClient":{"pid":123,"cwd":null,"shell":"zsh","shell_version":null}}"#,
            r#"{"UnregisterClient":{"pid":123}}"#,
            r#"{"WorkspaceUpdate":{"pid":123,"cwd":"."}}"#,
            r#""Ping""#,
            r#""Status""#,
            r#"{"ThemeQuery":{"key":"k"}}"#,
            r#""LatencyStats""#,
            r#""Shutdown""#,
            r#""ConfigReload""#,
        ];
        for text in native {
            assert!(
                validate(&schema, &schema, &json(text)),
                "schema rejects {text}"
            );
            assert!(
                deserialize_message(text.as_bytes()).is_ok(),
                "agent rejects {text}"
            );
        }
        // The retired format name is rejected by the schema (and, in native
        // form, by the agent).
        let stale = json(r#"{"op":"git","cwd":".","format":"fish-ansi"}"#);
        assert!(!validate(&schema, &schema, &stale));
    }

    fn render_json(response: &Response) -> Value {
        let config = Config::default();
        let theme = ThemeConfig::default();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let text = create_formatter(Format::Json)
            .unwrap()
            .render(response, &ctx)
            .unwrap();
        json(&text)
    }

    #[test]
    fn response_schema_validates_json_replies() {
        let schema = load("response.json");
        let repo = |state, rebase_progress| RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 2,
            behind: 1,
            ahead_capped: false,
            behind_capped: false,
            staged: 3,
            unstaged: 4,
            untracked: 5,
            conflicts: 0,
            state,
            stash_count: 1,
            detached: false,
            rebase_progress,
        };
        let rebase = Some(gpy_agent::git::RebaseProgress { step: 1, total: 3 });
        let replies = vec![
            (
                "git",
                render_json(&Response::RepositoryStatus(repo(
                    RepositoryState::Clean,
                    None,
                ))),
            ),
            (
                "git (rebasing)",
                render_json(&Response::RepositoryStatus(repo(
                    RepositoryState::Rebasing,
                    rebase,
                ))),
            ),
            (
                "lang",
                render_json(&Response::Language {
                    languages: vec![LanguageInfo {
                        name: "Rust".to_owned(),
                        version: None,
                        color: gpy_agent::config::types::ColorSpec::new("#dea584").unwrap(),
                    }],
                }),
            ),
            (
                "directory",
                render_json(&Response::Directory {
                    cwd: "/tmp".to_owned(),
                    read_only: false,
                }),
            ),
            ("ping/ack", render_json(&Response::Ack)),
            (
                "error",
                render_json(&Response::Error {
                    message: "boom".to_owned(),
                }),
            ),
            (
                "theme",
                render_json(&Response::ThemeValue {
                    value: "red".to_owned(),
                }),
            ),
            (
                "duration",
                render_json(&Response::Duration { duration_ms: 3 }),
            ),
            (
                "character",
                render_json(&Response::Character { success: true }),
            ),
            (
                "hostname",
                render_json(&Response::Hostname {
                    hostname: "h".to_owned(),
                }),
            ),
            (
                "username",
                render_json(&Response::Username {
                    username: "u".to_owned(),
                }),
            ),
            (
                "clock",
                render_json(&Response::Clock {
                    shell: gpy_agent::shell::Shell::Zsh,
                }),
            ),
            (
                "latency_stats",
                render_json(&Response::LatencyStatsResult {
                    min_ms: 1,
                    max_ms: 15,
                    avg_ms: 3,
                    sample_count: 42,
                }),
            ),
            (
                "status",
                // The socket sends AgentStatus through the native serializer.
                json(
                    &String::from_utf8(
                        serialize_response(&Response::AgentStatus {
                            version: "1.0.0".to_owned(),
                            protocol_version: 2,
                            watched_repos: 1,
                            registered_clients: 2,
                            cache_entries: 3,
                        })
                        .unwrap(),
                    )
                    .unwrap(),
                ),
            ),
        ];
        for (label, reply) in &replies {
            assert!(
                validate(&schema, &schema, reply),
                "response.json rejects {label}: {reply}"
            );
        }
    }

    #[test]
    fn response_schema_ack_and_error_shapes() {
        let schema = load("response.json");
        let find = |needle: &str| {
            one_of(&schema)
                .iter()
                .find(|b| b["title"].as_str().unwrap().contains(needle))
                .unwrap()
        };
        let ack = find("Ack");
        assert_eq!(ack["required"], json(r#"["status"]"#));
        assert_eq!(ack["properties"]["status"]["const"], "ok");
        let error = find("Error");
        assert_eq!(error["required"], json(r#"["error"]"#));
        assert_eq!(error["properties"]["error"]["type"], "string");

        // The retired enum-wrapped shapes must not validate.
        for stale in [
            r#""Ack""#,
            r#"{"Error":{"message":"x"}}"#,
            r#"{"RepositoryStatus":{"branch":"m"}}"#,
        ] {
            assert!(
                !validate(&schema, &schema, &json(stale)),
                "schema accepts stale {stale}"
            );
        }
    }
}

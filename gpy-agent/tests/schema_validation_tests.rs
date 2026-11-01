//! Documented JSON requests parse and the daemon's replies match the protocol schemas.

#![allow(clippy::unwrap_used)]

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
        let msg = format!("Failed to deserialize {label}");
        let message = deserialize_message(input).expect(&msg);
        assert!(matches!(
            message,
            gpy_agent::ipc::Message::RepositoryStatus { .. }
        ));
    }
}

#[test]
fn test_msg_deserialize_language_detect_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let cases: &[(&[u8], &str)] = &[
        (br#"{"op":"lang","cwd":"."}"#, "LanguageDetect (minimal)"),
        (
            br#"{"op":"lang","cwd":"/home/user/project","format":"ansi","is_last":true}"#,
            "LanguageDetect (full)",
        ),
    ];
    for (input, label) in cases {
        let msg = format!("Failed to deserialize {label}");
        let message = deserialize_message(input).expect(&msg);
        assert!(matches!(
            message,
            gpy_agent::ipc::Message::LanguageDetect { .. }
        ));
    }
}

#[test]
fn test_msg_deserialize_client_registration_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let pid = std::process::id();

    {
        let json = format!(r#"{{"op":"register","pid":{pid},"cwd":"/home/user/project"}}"#);
        let message = deserialize_message(json.as_bytes())
            .expect("Failed to deserialize RegisterClient (with cwd)");
        assert!(matches!(
            message,
            gpy_agent::ipc::Message::RegisterClient { .. }
        ));
    }
    {
        let json = format!(r#"{{"op":"register","pid":{pid}}}"#);
        let message = deserialize_message(json.as_bytes())
            .expect("Failed to deserialize RegisterClient (without cwd)");
        assert!(matches!(
            message,
            gpy_agent::ipc::Message::RegisterClient { .. }
        ));
    }
    {
        let json = format!(r#"{{"op":"unregister","pid":{pid}}}"#);
        let message =
            deserialize_message(json.as_bytes()).expect("Failed to deserialize UnregisterClient");
        assert!(matches!(
            message,
            gpy_agent::ipc::Message::UnregisterClient { .. }
        ));
    }
    {
        let json = format!(r#"{{"op":"workspace","pid":{pid},"cwd":"/home/user/new-project"}}"#);
        let message =
            deserialize_message(json.as_bytes()).expect("Failed to deserialize WorkspaceUpdate");
        assert!(matches!(
            message,
            gpy_agent::ipc::Message::WorkspaceUpdate { .. }
        ));
    }
}

#[test]
fn test_msg_deserialize_simple_ops_fish() {
    use gpy_agent::ipc::protocol::deserialize_message;

    let cases: &[(&[u8], &str)] = &[
        (br#"{"op":"ping"}"#, "Ping"),
        (br#"{"op":"status"}"#, "Status"),
        (br#"{"op":"shutdown"}"#, "Shutdown"),
        (br#"{"op":"config_reload"}"#, "ConfigReload"),
    ];
    for (input, label) in cases {
        let msg = format!("Failed to deserialize {label}");
        let message = deserialize_message(input).expect(&msg);
        assert!(matches!(
            message,
            gpy_agent::ipc::Message::Ping
                | gpy_agent::ipc::Message::Status
                | gpy_agent::ipc::Message::Shutdown
                | gpy_agent::ipc::Message::ConfigReload
        ));
    }
}

#[test]
fn test_protocol_validation_catches_errors() {
    use gpy_agent::ipc::protocol::deserialize_message;

    // Above ClientPid::MAX (4_194_304): rejected at parse time. In-range dead
    // PIDs are only rejected at route time, not here.
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

// No JSON-Schema validator crate is a dev-dependency, so `validate` below
// implements the draft-07 subset the schemas use.

mod schema_files {
    #![allow(
        clippy::too_many_lines,
        reason = "sample tables and a schema-subset validator read best undivided"
    )]

    use gpy_agent::config::Config;
    use gpy_agent::formatter::{Format, RenderContext, SegmentPosition, create_formatter};
    use gpy_agent::git::{RepositoryState, RepositoryStatus};
    use gpy_agent::ipc::protocol::deserialize_message;
    use gpy_agent::ipc::wire::Op;
    use gpy_agent::ipc::{AgentStatus, LanguageInfo, Response};
    use gpy_agent::theme::ThemeConfig;
    use serde_json::Value;
    use std::collections::BTreeSet;
    use std::str::FromStr;

    fn load(name: &str) -> Value {
        let path = format!("{}/schemas/{name}", env!("CARGO_MANIFEST_DIR"));
        let text = std::fs::read_to_string(&path).unwrap();
        serde_json::from_str(&text).unwrap()
    }

    fn one_of(schema: &Value) -> &Vec<Value> {
        schema["oneOf"].as_array().unwrap()
    }

    /// Every `Format` variant; the wildcard-free match forces new variants to be
    /// listed here and in `message.json`.
    fn all_formats() -> Vec<Format> {
        let all = vec![
            Format::Json,
            Format::Ansi,
            Format::BashPrompt,
            Format::ZshPrompt,
        ];
        for format in &all {
            match format {
                Format::Json | Format::Ansi | Format::BashPrompt | Format::ZshPrompt => {}
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
            sites.len() >= 7,
            "expected many format sites: {}",
            sites.len()
        );
        for site in &sites {
            assert_eq!(
                site["$ref"], "#/definitions/Format",
                "inline format: {site}"
            );
        }

        let listed: Vec<&str> = schema
            .pointer("/definitions/Format/enum")
            .and_then(serde_json::Value::as_array)
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap())
            .collect();
        for value in &listed {
            let msg = format!("message.json lists `{value}`, which Format::from_str rejects");
            let format = Format::from_str(value).expect(&msg);
            assert_eq!(&format.to_string(), value);
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

    /// Ops accepted by `ShellIpcMessage::into_message`: every [`Op`].
    fn ops_from_code() -> BTreeSet<String> {
        Op::ALL.iter().map(|op| op.as_str().to_owned()).collect()
    }

    #[test]
    fn schema_lists_every_shell_op() {
        let schema = load("message.json");
        let mut schema_ops = BTreeSet::new();
        for entry in one_of(&schema) {
            let op = entry
                .pointer("/properties/op/const")
                .and_then(serde_json::Value::as_str)
                .unwrap();
            assert!(schema_ops.insert(op.to_owned()), "duplicate op entry {op}");
        }
        assert_eq!(
            schema_ops,
            ops_from_code(),
            "message.json ops != into_message ops"
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
                r#"{"op":"lang","cwd":".","format":"ansi","virtual_env":"/v"}"#,
            ),
            (
                "directory",
                r#"{"op":"directory","cwd":".","format":"zsh-prompt"}"#,
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
                "exit_status",
                r#"{"op":"exit_status","success":false,"format":"zsh-prompt","is_last":true,"is_first":true,"prev_bg":"red"}"#,
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
        ];
        let sampled: BTreeSet<String> = shell.iter().map(|(op, _)| (*op).to_owned()).collect();
        assert_eq!(sampled, ops_from_code(), "add a sample for the new op");
        for (op, text) in shell {
            assert!(
                validate(&schema, &schema, &json(text)),
                "schema rejects {op}: {text}"
            );
            let msg = format!("agent rejects {op}: {text}");
            let _ = deserialize_message(text.as_bytes()).expect(&msg);
        }
        for text in [
            r#""Ping""#,
            r#"{"Ping":null}"#,
            r#"{"RepositoryStatus":{"path":"."}}"#,
        ] {
            assert!(
                !validate(&schema, &schema, &json(text)),
                "schema accepts {text}"
            );
            assert!(
                deserialize_message(text.as_bytes()).is_err(),
                "agent accepts {text}"
            );
        }
        // The retired format name is rejected by the schema and the agent.
        let stale = json(r#"{"op":"git","cwd":".","format":"fish-ansi"}"#);
        assert!(!validate(&schema, &schema, &stale));
        // `success` is a bare boolean.
        let bad = r#"{"op":"character","success":"yes"}"#;
        assert!(
            !validate(&schema, &schema, &json(bad)),
            "schema accepts {bad}"
        );
    }

    fn render_json(response: &Response) -> Value {
        let config = Config::default();
        let theme = ThemeConfig::default();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let text = create_formatter(Format::Json)
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
                "duration",
                render_json(&Response::Duration { duration_ms: 3 }),
            ),
            (
                "character",
                render_json(&Response::Character { success: true }),
            ),
            (
                "exit_status",
                render_json(&Response::ExitStatus { success: false }),
            ),
            (
                "clock",
                render_json(&Response::Clock {
                    shell: gpy_agent::shell::Shell::Zsh,
                }),
            ),
            (
                "status",
                render_json(&Response::AgentStatus(AgentStatus {
                    version: "1.0.0".to_owned(),
                    protocol_version: 2,
                    watched_repos: 1,
                    registered_clients: 2,
                    cache_entries: 3,
                    pid: Some(4321),
                })),
            ),
        ];
        for (label, reply) in &replies {
            assert!(
                validate(&schema, &schema, reply),
                "response.json rejects {label}: {reply}"
            );
        }
        let mut bad_status = replies
            .iter()
            .find(|(label, _)| *label == "status")
            .map(|(_, reply)| reply.clone())
            .unwrap();
        bad_status
            .get_mut("AgentStatus")
            .and_then(Value::as_object_mut)
            .expect("status reply has an AgentStatus object")
            .insert("pid".to_owned(), Value::String("x".to_owned()));
        assert!(
            !validate(&schema, &schema, &bad_status),
            "response.json accepts a non-integer pid: {bad_status}"
        );
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
        assert_eq!(ack.get("required"), Some(&json(r#"["status"]"#)));
        assert_eq!(
            ack.pointer("/properties/status/const")
                .and_then(serde_json::Value::as_str),
            Some("ok")
        );
        let error = find("Error");
        assert_eq!(error.get("required"), Some(&json(r#"["error"]"#)));
        assert_eq!(
            error
                .pointer("/properties/error/type")
                .and_then(serde_json::Value::as_str),
            Some("string")
        );

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

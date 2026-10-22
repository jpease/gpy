//! JSON output formatter

use crate::Result;
use crate::formatter::{Formatter, RenderContext};
use crate::ipc::Response;
use serde_json::json;

/// Serializes responses to JSON.
pub struct JsonFormatter;

impl Formatter for JsonFormatter {
    fn render(&self, response: &Response, _ctx: &RenderContext<'_>) -> Result<String> {
        let json_value = match response {
            // Delegate to `Serialize` so no field can be silently dropped.
            Response::RepositoryStatus(status) => serde_json::to_value(status).map_err(|e| {
                crate::Error::ipc(format!("failed to serialize repository status: {e}"))
            })?,
            Response::Language { languages } => json!({
                "languages": serde_json::to_value(languages).map_err(|e| {
                    crate::Error::ipc(format!("failed to serialize languages: {e}"))
                })?,
            }),
            Response::Ack => json!({"status": "ok"}),
            // Wire shape is `{"AgentStatus":{…}}`, the same in every format.
            Response::AgentStatus { .. } => serde_json::to_value(response)
                .map_err(|e| crate::Error::ipc(format!("failed to serialize agent status: {e}")))?,
            Response::Directory { cwd, read_only } => json!({"cwd": cwd, "read_only": read_only}),
            Response::Clock { shell } => json!({"shell": shell.as_str()}),
            Response::Duration { duration_ms } => json!({"duration_ms": duration_ms}),
            Response::Character { success } | Response::ExitStatus { success } => {
                json!({"success": success})
            }
            Response::Hostname { hostname, is_ssh } => {
                json!({"hostname": hostname, "is_ssh": is_ssh})
            }
            Response::Username { username } => json!({"username": username}),
            Response::Error { message } => json!({"error": message}),
        };

        serde_json::to_string(&json_value)
            .map_err(|e| crate::Error::ipc(format!("JSON serialization failed: {e}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::formatter::SegmentPosition;
    use crate::ipc::Response;
    use crate::theme::ThemeConfig;

    fn create_test_context() -> (Config, ThemeConfig) {
        let config = Config::default();
        let theme = ThemeConfig::default();
        (config, theme)
    }

    /// Looks up `pointer` in `value`, panicking with the pointer when absent.
    fn field(value: &serde_json::Value, pointer: &str) -> serde_json::Value {
        value
            .pointer(pointer)
            .cloned()
            .unwrap_or_else(|| panic!("missing JSON field {pointer}"))
    }

    #[test]
    fn test_json_formatter_repository_status() {
        let formatter = JsonFormatter;
        let (config, theme) = create_test_context();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let response = Response::RepositoryStatus(crate::git::RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 2_u32,
            behind: 1_u32,
            ahead_capped: false,
            behind_capped: false,
            staged: 3_u32,
            unstaged: 4_u32,
            untracked: 5_u32,
            conflicts: 0_u32,
            state: crate::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        });

        let output = formatter.render(&response, &ctx).expect("render failed");

        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid JSON output");

        assert_eq!(field(&parsed, "/branch"), "main");
        assert_eq!(field(&parsed, "/ahead"), 2_i32);
        assert_eq!(field(&parsed, "/behind"), 1_i32);
        assert_eq!(field(&parsed, "/staged"), 3_i32);
        assert_eq!(field(&parsed, "/unstaged"), 4_i32);
        assert_eq!(field(&parsed, "/untracked"), 5_i32);
        assert_eq!(field(&parsed, "/conflicts"), 0_i32);
        assert_eq!(field(&parsed, "/state"), "clean");
    }

    #[test]
    fn test_json_formatter_repository_status_includes_every_field() {
        let formatter = JsonFormatter;
        let (config, theme) = create_test_context();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let response = Response::RepositoryStatus(crate::git::RepositoryStatus {
            branch: "feature/thing".to_owned(),
            ahead: 7_u32,
            behind: 9_u32,
            ahead_capped: true,
            behind_capped: true,
            staged: 1_u32,
            unstaged: 2_u32,
            untracked: 3_u32,
            conflicts: 4_u32,
            state: crate::git::RepositoryState::Rebasing,
            stash_count: 6_u32,
            detached: true,
            rebase_progress: Some(crate::git::RebaseProgress {
                step: 2_u32,
                total: 5_u32,
            }),
        });

        let output = formatter.render(&response, &ctx).expect("render failed");
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid JSON output");

        assert_eq!(field(&parsed, "/branch"), "feature/thing");
        assert_eq!(field(&parsed, "/ahead"), 7_i32);
        assert_eq!(field(&parsed, "/behind"), 9_i32);
        assert_eq!(field(&parsed, "/ahead_capped"), true);
        assert_eq!(field(&parsed, "/behind_capped"), true);
        assert_eq!(field(&parsed, "/staged"), 1_i32);
        assert_eq!(field(&parsed, "/unstaged"), 2_i32);
        assert_eq!(field(&parsed, "/untracked"), 3_i32);
        assert_eq!(field(&parsed, "/conflicts"), 4_i32);
        assert_eq!(field(&parsed, "/state"), "rebasing");
        assert_eq!(field(&parsed, "/stash_count"), 6_i32);
        assert_eq!(field(&parsed, "/detached"), true);
        assert_eq!(field(&parsed, "/rebase_progress/step"), 2_i32);
        assert_eq!(field(&parsed, "/rebase_progress/total"), 5_i32);
    }

    #[test]
    fn test_json_formatter_language() {
        let formatter = JsonFormatter;
        let (config, theme) = create_test_context();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let response = Response::Language {
            languages: vec![crate::ipc::LanguageInfo {
                name: "Rust".to_owned(),
                version: Some("1.75.0".to_owned()),
                color: crate::config::types::ColorSpec::new("#dea584").unwrap(),
            }],
        };

        let output = formatter.render(&response, &ctx).expect("render failed");

        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid JSON output");

        assert!(field(&parsed, "/languages").is_array());
        assert_eq!(field(&parsed, "/languages/0/name"), "Rust");
        assert_eq!(field(&parsed, "/languages/0/version"), "1.75.0");
        assert_eq!(field(&parsed, "/languages/0/color"), "#dea584");
    }

    #[test]
    fn test_json_formatter_ack() {
        let formatter = JsonFormatter;
        let (config, theme) = create_test_context();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let response = Response::Ack;
        let output = formatter.render(&response, &ctx).expect("render failed");

        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid JSON output");

        assert_eq!(field(&parsed, "/status"), "ok");
    }

    fn pinned_cases() -> Vec<(Response, &'static str)> {
        vec![
            (
                Response::RepositoryStatus(crate::git::RepositoryStatus {
                    branch: "main".to_owned(),
                    ahead: 2,
                    behind: 1,
                    ahead_capped: false,
                    behind_capped: false,
                    staged: 3,
                    unstaged: 4,
                    untracked: 5,
                    conflicts: 0,
                    state: crate::git::RepositoryState::Clean,
                    stash_count: 0,
                    detached: false,
                    rebase_progress: None,
                }),
                r#"{"ahead":2,"ahead_capped":false,"behind":1,"behind_capped":false,"branch":"main","conflicts":0,"detached":false,"rebase_progress":null,"staged":3,"stash_count":0,"state":"clean","unstaged":4,"untracked":5}"#,
            ),
            (
                Response::Language {
                    languages: vec![
                        crate::ipc::LanguageInfo {
                            name: "Rust".to_owned(),
                            version: Some("1.75.0".to_owned()),
                            color: crate::config::types::ColorSpec::new("#dea584").unwrap(),
                        },
                        crate::ipc::LanguageInfo {
                            name: "Python".to_owned(),
                            version: None,
                            color: crate::config::types::ColorSpec::new("#3776ab").unwrap(),
                        },
                    ],
                },
                r##"{"languages":[{"color":"#dea584","name":"Rust","version":"1.75.0"},{"color":"#3776ab","name":"Python","version":null}]}"##,
            ),
            (Response::Ack, r#"{"status":"ok"}"#),
            (
                Response::AgentStatus {
                    version: "1.0.0".to_owned(),
                    protocol_version: 2,
                    watched_repos: 1,
                    registered_clients: 2,
                    cache_entries: 3,
                },
                r#"{"AgentStatus":{"cache_entries":3,"protocol_version":2,"registered_clients":2,"version":"1.0.0","watched_repos":1}}"#,
            ),
            (
                Response::Directory {
                    cwd: "~/src".to_owned(),
                    read_only: true,
                },
                r#"{"cwd":"~/src","read_only":true}"#,
            ),
            (
                Response::Clock {
                    shell: crate::shell::Shell::Zsh,
                },
                r#"{"shell":"zsh"}"#,
            ),
            (
                Response::Duration { duration_ms: 1500 },
                r#"{"duration_ms":1500}"#,
            ),
            (
                Response::Character { success: false },
                r#"{"success":false}"#,
            ),
            (
                Response::ExitStatus { success: true },
                r#"{"success":true}"#,
            ),
            (
                Response::Hostname {
                    hostname: "box".to_owned(),
                    is_ssh: true,
                },
                r#"{"hostname":"box","is_ssh":true}"#,
            ),
            (
                Response::Username {
                    username: "me".to_owned(),
                },
                r#"{"username":"me"}"#,
            ),
            (
                Response::Error {
                    message: r#"boom "q""#.to_owned(),
                },
                r#"{"error":"boom \"q\""}"#,
            ),
        ]
    }

    /// Byte-exact output for every `Response` variant; extend when a variant is added.
    #[test]
    fn json_formatter_pins_every_variant() {
        let (config, theme) = create_test_context();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        for (response, expected) in pinned_cases() {
            let output = JsonFormatter
                .render(&response, &ctx)
                .expect("render failed");
            assert_eq!(output, expected, "{response:?}");
        }
    }
}

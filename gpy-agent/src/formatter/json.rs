//! JSON output formatter

use crate::Result;
use crate::formatter::{Formatter, RenderContext};
use crate::ipc::Response;
use serde_json::{Map, Value};

/// JSON format formatter
///
/// Serializes responses to JSON using `serde_json`.
pub struct JsonFormatter;

impl Formatter for JsonFormatter {
    #[expect(
        clippy::too_many_lines,
        reason = "mapping enum variants to JSON objects requires explicit field construction"
    )]
    fn render(&self, response: &Response, _ctx: &RenderContext<'_>) -> Result<String> {
        let json_value = match response {
            // `RepositoryStatus` derives `Serialize`, so delegate to it directly
            // rather than hand-mirroring its fields here: a hand-built `Map`
            // silently drops any field this arm forgets to list (this used to
            // miss `stash_count`, `detached`, and `rebase_progress`).
            Response::RepositoryStatus(status) => serde_json::to_value(status).map_err(|e| {
                crate::Error::ipc(format!("failed to serialize repository status: {e}"))
            })?,
            Response::Language { languages } => {
                let json_langs: Vec<Value> = languages
                    .iter()
                    .map(|lang| {
                        let mut map = Map::new();
                        map.insert("name".to_owned(), Value::String(lang.name.clone()));
                        map.insert(
                            "version".to_owned(),
                            lang.version
                                .as_ref()
                                .map_or(Value::Null, |v| Value::String(v.clone())),
                        );
                        map.insert("color".to_owned(), Value::String(lang.color.to_string()));
                        Value::Object(map)
                    })
                    .collect();

                let mut outer = Map::new();
                outer.insert("languages".to_owned(), Value::Array(json_langs));
                Value::Object(outer)
            }
            Response::Ack => {
                let mut map = Map::new();
                map.insert("status".to_owned(), Value::String("ok".to_owned()));
                Value::Object(map)
            }
            Response::AgentStatus {
                version,
                protocol_version,
                watched_repos,
                registered_clients,
                cache_entries,
            } => {
                let mut map = Map::new();
                map.insert("version".to_owned(), Value::String(version.clone()));
                map.insert(
                    "protocol_version".to_owned(),
                    Value::Number((*protocol_version).into()),
                );
                map.insert(
                    "watched_repos".to_owned(),
                    Value::Number((*watched_repos).into()),
                );
                map.insert(
                    "registered_clients".to_owned(),
                    Value::Number((*registered_clients).into()),
                );
                map.insert(
                    "cache_entries".to_owned(),
                    Value::Number((*cache_entries).into()),
                );
                Value::Object(map)
            }
            Response::ThemeValue { value } => {
                let mut map = Map::new();
                map.insert("value".to_owned(), Value::String(value.clone()));
                Value::Object(map)
            }
            Response::LatencyStatsResult {
                min_ms,
                max_ms,
                avg_ms,
                sample_count,
            } => {
                let mut map = Map::new();
                map.insert("min_ms".to_owned(), Value::Number((*min_ms).into()));
                map.insert("max_ms".to_owned(), Value::Number((*max_ms).into()));
                map.insert("avg_ms".to_owned(), Value::Number((*avg_ms).into()));
                map.insert(
                    "sample_count".to_owned(),
                    Value::Number((*sample_count).into()),
                );
                Value::Object(map)
            }
            Response::Directory { cwd, read_only } => {
                let mut map = Map::new();
                map.insert("cwd".to_owned(), Value::String(cwd.clone()));
                map.insert("read_only".to_owned(), Value::Bool(*read_only));
                Value::Object(map)
            }
            Response::Clock { shell } => {
                let mut map = Map::new();
                map.insert("shell".to_owned(), Value::String(shell.as_str().to_owned()));
                Value::Object(map)
            }
            Response::Duration { duration_ms } => {
                let mut map = Map::new();
                map.insert(
                    "duration_ms".to_owned(),
                    Value::Number((*duration_ms).into()),
                );
                Value::Object(map)
            }
            Response::Character { success } => {
                let mut map = Map::new();
                map.insert("success".to_owned(), Value::Bool(*success));
                Value::Object(map)
            }
            Response::Hostname { hostname, is_ssh } => {
                let mut map = Map::new();
                map.insert("hostname".to_owned(), Value::String(hostname.clone()));
                map.insert("is_ssh".to_owned(), Value::Bool(*is_ssh));
                Value::Object(map)
            }
            Response::Username { username } => {
                let mut map = Map::new();
                map.insert("username".to_owned(), Value::String(username.clone()));
                Value::Object(map)
            }
            Response::Error { message } => {
                let mut map = Map::new();
                map.insert("error".to_owned(), Value::String(message.clone()));
                Value::Object(map)
            }
        };

        serde_json::to_string(&json_value)
            .map_err(|e| crate::Error::ipc(format!("JSON serialization failed: {e}")))
    }
}

#[cfg(test)]
#[allow(clippy::indexing_slicing)]
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

    #[test]
    /// # Panics
    ///
    /// Panics if the formatter fails to render the repository status response.
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

        // Verify it's valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid JSON output");

        // Verify fields are present
        assert_eq!(parsed["branch"], "main");
        assert_eq!(parsed["ahead"], 2_i32);
        assert_eq!(parsed["behind"], 1_i32);
        assert_eq!(parsed["staged"], 3_i32);
        assert_eq!(parsed["unstaged"], 4_i32);
        assert_eq!(parsed["untracked"], 5_i32);
        assert_eq!(parsed["conflicts"], 0_i32);
        assert_eq!(parsed["state"], "clean");
    }

    #[test]
    /// # Panics
    ///
    /// Panics if the formatter fails to render the repository status response,
    /// or if any of `RepositoryStatus`'s 13 fields is missing/wrong afterward.
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

        assert_eq!(parsed["branch"], "feature/thing");
        assert_eq!(parsed["ahead"], 7_i32);
        assert_eq!(parsed["behind"], 9_i32);
        assert_eq!(parsed["ahead_capped"], true);
        assert_eq!(parsed["behind_capped"], true);
        assert_eq!(parsed["staged"], 1_i32);
        assert_eq!(parsed["unstaged"], 2_i32);
        assert_eq!(parsed["untracked"], 3_i32);
        assert_eq!(parsed["conflicts"], 4_i32);
        assert_eq!(parsed["state"], "rebasing");
        assert_eq!(parsed["stash_count"], 6_i32);
        assert_eq!(parsed["detached"], true);
        assert_eq!(parsed["rebase_progress"]["step"], 2_i32);
        assert_eq!(parsed["rebase_progress"]["total"], 5_i32);
    }

    #[test]
    /// # Panics
    ///
    /// Panics if the formatter fails to render the language response.
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

        // Verify it's valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid JSON output");

        // Verify structure
        assert!(parsed["languages"].is_array());
        assert_eq!(parsed["languages"][0]["name"], "Rust");
        assert_eq!(parsed["languages"][0]["version"], "1.75.0");
        assert_eq!(parsed["languages"][0]["color"], "#dea584");
    }

    #[test]
    /// # Panics
    ///
    /// Panics if the formatter fails to render the acknowledgement response.
    fn test_json_formatter_ack() {
        let formatter = JsonFormatter;
        let (config, theme) = create_test_context();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let response = Response::Ack;
        let output = formatter.render(&response, &ctx).expect("render failed");

        // Verify it's valid JSON
        let parsed: serde_json::Value = serde_json::from_str(&output).expect("invalid JSON output");

        // Ack response should indicate success status
        assert_eq!(parsed["status"], "ok");
    }
}

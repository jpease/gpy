//! The v2 IPC wire, defined once: request op names and the JSON reply codec.
//!
//! Requests are flat objects keyed by `op` ([`Op`]). Replies are flat objects
//! too, except the status reply, which is `{"AgentStatus":{…}}`; control ops
//! reply in JSON whatever `format` the request names.
//! `SCHEMA_EVOLUTION.md` says when a change here is a breaking one.

use super::{AgentStatus, Response};
use serde::Deserialize;
use serde_json::{Value, json};
use std::str::FromStr;

/// Wire form of [`Response::Ack`] as [`encode_json`] renders it.
///
/// Shell clients match on this shape, so fixtures modelling a live agent must send it.
pub const ACK_WIRE_JSON: &str = r#"{"status":"ok"}"#;

/// Every request `op` a shell client may send; the only list of op names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    /// `git`: repository status segment.
    Git,
    /// `lang`: language detection segment.
    Lang,
    /// `directory`: directory segment.
    Directory,
    /// `clock`: clock segment.
    Clock,
    /// `duration`: command duration segment.
    Duration,
    /// `character`: the final prompt symbol segment.
    Character,
    /// `exit_status`: the previous command's exit status segment.
    ExitStatus,
    /// `ping`: liveness check.
    Ping,
    /// `status`: agent health and counters.
    Status,
    /// `shutdown`: stop the agent.
    Shutdown,
    /// `config_reload`: reload configuration from disk.
    ConfigReload,
    /// `register`: register a shell process for live updates.
    Register,
    /// `unregister`: unregister a shell process.
    Unregister,
    /// `workspace`: report a registered client's working directory.
    Workspace,
}

impl Op {
    /// Every op, in schema order.
    pub const ALL: [Self; 14] = [
        Self::Git,
        Self::Lang,
        Self::Directory,
        Self::Clock,
        Self::Duration,
        Self::Character,
        Self::ExitStatus,
        Self::Ping,
        Self::Status,
        Self::Shutdown,
        Self::ConfigReload,
        Self::Register,
        Self::Unregister,
        Self::Workspace,
    ];

    /// The op's name on the wire.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Git => "git",
            Self::Lang => "lang",
            Self::Directory => "directory",
            Self::Clock => "clock",
            Self::Duration => "duration",
            Self::Character => "character",
            Self::ExitStatus => "exit_status",
            Self::Ping => "ping",
            Self::Status => "status",
            Self::Shutdown => "shutdown",
            Self::ConfigReload => "config_reload",
            Self::Register => "register",
            Self::Unregister => "unregister",
            Self::Workspace => "workspace",
        }
    }

    /// The request line for an op that carries no fields: `ping`, `status`,
    /// `shutdown` and `config_reload`.
    #[must_use]
    pub fn bare_request(self) -> String {
        json!({"op": self.as_str()}).to_string()
    }
}

impl FromStr for Op {
    type Err = crate::Error;

    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|op| op.as_str() == name)
            .ok_or_else(|| crate::Error::invalid(format!("Unknown shell operation: {name}")))
    }
}

/// Encode a reply as the JSON line body the daemon writes for `format=json`
/// (and for every control op): flat objects, `{"status":"ok"}` for
/// [`Response::Ack`], `{"error":…}` for [`Response::Error`], and
/// `{"AgentStatus":{…}}` for [`Response::AgentStatus`].
///
/// # Errors
///
/// Returns an error if a payload fails to serialize.
pub fn encode_json(response: &Response) -> crate::Result<String> {
    let json_value = match response {
        // Delegate to `Serialize` so no field can be silently dropped.
        Response::RepositoryStatus(status) => serde_json::to_value(status)?,
        Response::Language { languages } => json!({
            "languages": serde_json::to_value(languages)?,
        }),
        Response::Ack => json!({"status": "ok"}),
        Response::AgentStatus(status) => json!({"AgentStatus": serde_json::to_value(status)?}),
        Response::Directory { cwd, read_only } => json!({"cwd": cwd, "read_only": read_only}),
        Response::Clock { shell } => json!({"shell": shell.as_str()}),
        Response::Duration { duration_ms } => json!({"duration_ms": duration_ms}),
        Response::Character { success } | Response::ExitStatus { success } => {
            json!({"success": success})
        }
        Response::Error { message } => json!({"error": message}),
    };

    Ok(serde_json::to_string(&json_value)?)
}

/// Decode the reply to a control op (`ping`, `shutdown`, `config_reload`,
/// `status`) into [`Response::Ack`], [`Response::Error`] or
/// [`Response::AgentStatus`].
///
/// Extra keys on an ack are tolerated.
///
/// # Errors
///
/// Returns an error when the text is none of the three wire forms.
pub fn decode_control_reply(text: &str) -> crate::Result<Response> {
    let body = text.trim_end_matches(['\n', '\r']);
    let value = serde_json::from_str::<Value>(body)
        .map_err(|e| crate::Error::ipc(format!("Failed to parse agent response: {e}")))?;
    if value.get("status").and_then(Value::as_str) == Some("ok") {
        return Ok(Response::Ack);
    }
    if let Some(message) = value.get("error").and_then(Value::as_str) {
        return Ok(Response::Error {
            message: message.to_owned(),
        });
    }
    if let Some(status) = value.get("AgentStatus") {
        return AgentStatus::deserialize(status)
            .map(Response::AgentStatus)
            .map_err(|e| crate::Error::ipc(format!("Failed to parse agent response: {e}")));
    }
    Err(crate::Error::ipc(format!(
        "Failed to parse agent response: not a control reply: {body}"
    )))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ipc::LanguageInfo;

    fn sample_status() -> AgentStatus {
        AgentStatus {
            version: "1.0.0".to_owned(),
            protocol_version: 2,
            watched_repos: 1,
            registered_clients: 2,
            cache_entries: 3,
            pid: Some(4321),
        }
    }

    /// One sample per `Response` variant; the wildcard-free match forces a new
    /// variant to be listed here and given a golden line in [`goldens`].
    fn one_of_each_variant() -> Vec<Response> {
        let all = vec![
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
            Response::Language {
                languages: vec![
                    LanguageInfo {
                        name: "Rust".to_owned(),
                        version: Some("1.75.0".to_owned()),
                        color: crate::config::types::ColorSpec::new("#dea584").unwrap(),
                    },
                    LanguageInfo {
                        name: "Python".to_owned(),
                        version: None,
                        color: crate::config::types::ColorSpec::new("#3776ab").unwrap(),
                    },
                ],
            },
            Response::Directory {
                cwd: "~/src".to_owned(),
                read_only: true,
            },
            Response::Duration { duration_ms: 1500 },
            Response::Clock {
                shell: crate::shell::Shell::Zsh,
            },
            Response::Character { success: false },
            Response::ExitStatus { success: true },
            Response::Ack,
            Response::AgentStatus(sample_status()),
            Response::Error {
                message: r#"boom "q""#.to_owned(),
            },
        ];
        for response in &all {
            match response {
                Response::RepositoryStatus(_)
                | Response::Language { .. }
                | Response::Directory { .. }
                | Response::Duration { .. }
                | Response::Clock { .. }
                | Response::Character { .. }
                | Response::ExitStatus { .. }
                | Response::Ack
                | Response::AgentStatus(_)
                | Response::Error { .. } => {}
            }
        }
        all
    }

    /// The golden wire line of each sample from [`one_of_each_variant`], in order.
    fn goldens() -> [&'static str; 10] {
        [
            r#"{"ahead":2,"ahead_capped":false,"behind":1,"behind_capped":false,"branch":"main","conflicts":0,"detached":false,"rebase_progress":null,"staged":3,"stash_count":0,"state":"clean","unstaged":4,"untracked":5}"#,
            r##"{"languages":[{"color":"#dea584","name":"Rust","version":"1.75.0"},{"color":"#3776ab","name":"Python","version":null}]}"##,
            r#"{"cwd":"~/src","read_only":true}"#,
            r#"{"duration_ms":1500}"#,
            r#"{"shell":"zsh"}"#,
            r#"{"success":false}"#,
            r#"{"success":true}"#,
            ACK_WIRE_JSON,
            r#"{"AgentStatus":{"cache_entries":3,"pid":4321,"protocol_version":2,"registered_clients":2,"version":"1.0.0","watched_repos":1}}"#,
            r#"{"error":"boom \"q\""}"#,
        ]
    }

    /// Byte-exact output for every `Response` variant; extend when a variant is added.
    #[test]
    fn every_response_variant_has_a_pinned_wire_line() {
        let samples = one_of_each_variant();
        let expected = goldens();
        assert_eq!(samples.len(), expected.len(), "a sample lacks its golden");
        for (response, golden) in samples.iter().zip(expected) {
            assert_eq!(
                encode_json(response).expect("encode failed"),
                golden,
                "{response:?}"
            );
        }
    }

    #[test]
    fn control_replies_decode_from_their_wire_form() {
        assert_eq!(
            decode_control_reply(ACK_WIRE_JSON).expect("ack decodes"),
            Response::Ack
        );
        assert_eq!(
            decode_control_reply("{\"status\":\"ok\"}\r\n").expect("ack with CRLF decodes"),
            Response::Ack
        );
        assert_eq!(
            decode_control_reply(r#"{"error":"x"}"#).expect("error decodes"),
            Response::Error {
                message: "x".to_owned()
            }
        );
        let status = encode_json(&Response::AgentStatus(sample_status())).expect("encode failed");
        assert_eq!(
            decode_control_reply(&format!("{status}\n")).expect("status decodes"),
            Response::AgentStatus(sample_status())
        );

        for serde_form in [
            r#""Ack""#,
            r#"{"Error":{"message":"x"}}"#,
            r#"{"AgentStatus":{"version":"9.9.9"}}"#,
            "not json",
        ] {
            assert!(
                decode_control_reply(serde_form).is_err(),
                "{serde_form} must be rejected"
            );
        }
    }
}

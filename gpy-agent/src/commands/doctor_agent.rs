//! Doctor checks for an agent that answers: is it this binary's version, and
//! can it still produce a git segment. An agent that outlives an upgrade keeps
//! answering pings while its git requests fail, so a ping is not enough.

use crate::agent::lifecycle::AgentClient;
use crate::commands::output;
use crate::ipc::AgentStatus;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

/// How the running agent's version compares with this binary's.
#[derive(Debug, PartialEq, Eq)]
enum VersionVerdict {
    /// The same version.
    Current,
    /// The agent is older than this binary: it predates an upgrade.
    Older,
    /// The agent is newer than this binary: the binary was downgraded or an
    /// older copy comes first in `PATH`.
    Newer,
    /// The versions differ and one is not a `major.minor.patch` version.
    Different,
}

/// The numeric `major.minor.patch` core of `version`, ignoring any
/// pre-release or build suffix.
fn version_core(version: &str) -> Option<(u64, u64, u64)> {
    let core = version.split(['-', '+']).next()?;
    let mut parts = core.split('.').map(|part| part.parse::<u64>().ok());
    let triple = (parts.next()??, parts.next()??, parts.next()??);
    parts.next().is_none().then_some(triple)
}

/// Compare the version the agent reports with `this_version`.
fn compare_versions(running: &str, this_version: &str) -> VersionVerdict {
    if running == this_version {
        return VersionVerdict::Current;
    }
    match (version_core(running), version_core(this_version)) {
        (Some(agent), Some(binary)) => match agent.cmp(&binary) {
            Ordering::Less => VersionVerdict::Older,
            Ordering::Greater => VersionVerdict::Newer,
            Ordering::Equal => VersionVerdict::Different,
        },
        _ => VersionVerdict::Different,
    }
}

/// What the agent's reply to a git request says about its health.
#[derive(Debug, PartialEq, Eq)]
enum GitVerdict {
    /// The agent produced a segment.
    Healthy,
    /// The agent replied `{"error":…}`; carries the first line of the message.
    Failed(String),
    /// The reply was not a JSON object.
    Unreadable,
}

/// Judge the reply line the agent sent for a `git` request.
fn judge_git_reply(reply: &str) -> GitVerdict {
    let Ok(serde_json::Value::Object(fields)) = serde_json::from_str::<serde_json::Value>(reply)
    else {
        return GitVerdict::Unreadable;
    };
    fields.get("error").map_or(GitVerdict::Healthy, |error| {
        let text = error
            .as_str()
            .map_or_else(|| error.to_string(), str::to_owned);
        let first = text
            .lines()
            .map(str::trim)
            .find(|line| !line.is_empty())
            .unwrap_or("(empty error message)");
        GitVerdict::Failed(first.to_owned())
    })
}

/// The agent log's path as the path helpers resolve it, for the remediation.
fn agent_log_hint(cache_dir: Option<PathBuf>) -> String {
    cache_dir.map_or_else(
        || "the agent log (agent.log in the GPY cache directory)".to_owned(),
        |dir| dir.join("agent.log").display().to_string(),
    )
}

/// Print the version check for an agent that answered with `status`.
fn report_version(status: &AgentStatus, this_version: &str) {
    output::step("Checking agent version");
    let running = status.version.as_str();
    match compare_versions(running, this_version) {
        VersionVerdict::Current => output::ok(format_args!("Agent is current ({running})")),
        VersionVerdict::Older => {
            output::warn(format_args!(
                "Running agent is {running} but this gpy is {this_version}: the agent is outdated"
            ));
            output::detail("Remediation: run `gpy restart` to start the installed agent.");
        }
        VersionVerdict::Newer => {
            output::warn(format_args!(
                "Running agent is {running}, newer than this gpy ({this_version})"
            ));
            output::detail(
                "Remediation: run `gpy restart`, and check that the first `gpy` in your PATH is the one you installed last.",
            );
        }
        VersionVerdict::Different => {
            output::warn(format_args!(
                "Running agent is {running} but this gpy is {this_version}"
            ));
            output::detail("Remediation: run `gpy restart` to start the installed agent.");
        }
    }
}

/// Print the outcome of asking the agent for the git segment of `cwd`.
fn report_git(client: &AgentClient, maybe_cwd: Option<&Path>) {
    output::step("Checking agent git query");
    let Some(cwd) = maybe_cwd else {
        output::ok("Skipped (cannot read the current directory)");
        return;
    };
    if crate::git::find_repo_root(cwd).is_none() {
        output::ok("Skipped (not in a git repository)");
        return;
    }
    match client.git_reply(cwd).map(|reply| judge_git_reply(&reply)) {
        Ok(GitVerdict::Healthy) => output::ok("Agent answers git queries"),
        Ok(GitVerdict::Failed(error)) => {
            output::warn(format_args!(
                "Agent failed a git query for {}: {error}",
                cwd.display()
            ));
            output::detail(format_args!(
                "Remediation: run `gpy restart`, then check the agent log ({}).",
                agent_log_hint(crate::paths::gpy_cache_dir().ok())
            ));
        }
        Ok(GitVerdict::Unreadable) => {
            output::warn("Agent sent an unreadable reply to a git query");
            output::detail(format_args!(
                "Remediation: run `gpy restart`, then check the agent log ({}).",
                agent_log_hint(crate::paths::gpy_cache_dir().ok())
            ));
        }
        Err(err) => {
            output::warn(format_args!("Could not query the agent for git: {err}"));
            output::detail("Remediation: run `gpy restart`.");
        }
    }
}

/// Run the checks that need an answering agent. They only warn, as every
/// prompt still renders (from `gpy-agent oneshot`) while the agent is
/// outdated or failing, so this always returns `true`.
pub(super) fn check_answering_agent(client: &AgentClient, status: &AgentStatus) -> bool {
    report_version(status, crate::VERSION);
    report_git(client, std::env::current_dir().ok().as_deref());
    true
}

#[cfg(test)]
mod tests {
    use super::{
        GitVerdict, VersionVerdict, agent_log_hint, compare_versions, judge_git_reply, version_core,
    };
    use std::path::PathBuf;

    #[test]
    fn version_core_reads_three_numeric_parts_and_drops_suffixes() {
        assert_eq!(version_core("0.10.4"), Some((0, 10, 4)));
        assert_eq!(version_core("1.2.3-rc.1"), Some((1, 2, 3)));
        assert_eq!(version_core("1.2.3+build5"), Some((1, 2, 3)));
        assert_eq!(version_core("1.2"), None);
        assert_eq!(version_core("1.2.3.4"), None);
        assert_eq!(version_core("1.x.3"), None);
        assert_eq!(version_core(""), None);
    }

    #[test]
    fn versions_compare_numerically_not_lexically() {
        assert_eq!(
            compare_versions("0.10.4", "0.10.4"),
            VersionVerdict::Current
        );
        assert_eq!(compare_versions("0.9.0", "0.10.4"), VersionVerdict::Older);
        assert_eq!(compare_versions("0.10.4", "0.10.3"), VersionVerdict::Newer);
    }

    #[test]
    fn unparseable_or_suffix_only_differences_are_reported_as_different() {
        assert_eq!(compare_versions("dev", "0.10.4"), VersionVerdict::Different);
        assert_eq!(
            compare_versions("0.10.4-rc.1", "0.10.4"),
            VersionVerdict::Different
        );
    }

    #[test]
    fn an_error_reply_is_failed_with_its_first_line() {
        let reply = r#"{"error":"Unable to read current working directory: Operation not permitted\nsecond line"}"#;
        assert_eq!(
            judge_git_reply(reply),
            GitVerdict::Failed(
                "Unable to read current working directory: Operation not permitted".to_owned()
            )
        );
    }

    #[test]
    fn a_segment_reply_is_healthy() {
        assert_eq!(
            judge_git_reply(r#"{"branch":"main","ahead":2}"#),
            GitVerdict::Healthy
        );
        assert_eq!(judge_git_reply("{}"), GitVerdict::Healthy);
    }

    #[test]
    fn a_non_object_reply_is_unreadable() {
        assert_eq!(judge_git_reply("not json"), GitVerdict::Unreadable);
        assert_eq!(judge_git_reply("[1]"), GitVerdict::Unreadable);
        assert_eq!(judge_git_reply(""), GitVerdict::Unreadable);
    }

    #[test]
    fn a_blank_or_non_string_error_still_names_something() {
        assert_eq!(
            judge_git_reply(r#"{"error":"  \n"}"#),
            GitVerdict::Failed("(empty error message)".to_owned())
        );
        assert_eq!(
            judge_git_reply(r#"{"error":42}"#),
            GitVerdict::Failed("42".to_owned())
        );
    }

    #[test]
    fn the_log_hint_names_agent_log_in_the_cache_dir() {
        assert_eq!(
            agent_log_hint(Some(PathBuf::from("/home/u/.cache/gpy"))),
            PathBuf::from("/home/u/.cache/gpy")
                .join("agent.log")
                .display()
                .to_string()
        );
        assert!(agent_log_hint(None).contains("agent.log"));
    }
}

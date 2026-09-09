//! Fish shell source formatter
//!
//! Produces Fish `set -g` assignments that mirror the legacy
//! `serialize_response_emit_fish` helper.

use std::fmt::Write;

use crate::Result;
use crate::formatter::{Formatter, RenderContext, truncate_branch};
use crate::ipc::Response;
use crate::shell::Shell;
use crate::theme::export::escape_shell_value;

/// Format responses as Fish shell assignments.
pub struct FishSourceFormatter;

impl Formatter for FishSourceFormatter {
    fn render(&self, response: &Response, ctx: &RenderContext<'_>) -> Result<String> {
        let mut output = String::new();

        match response {
            Response::RepositoryStatus(status) => {
                render_repository_status(&mut output, status, ctx);
            }
            Response::Language { .. } => render_languages(&mut output, response),
            // The fish-source path has no directory, duration, character, hostname,
            // or username variable contract; the agent emits nothing and the shell
            // renders those segments locally.
            Response::Directory { .. }
            | Response::Clock { .. }
            | Response::Duration { .. }
            | Response::Character { .. }
            | Response::Hostname { .. }
            | Response::Username { .. } => {}
            Response::Ack => render_ack(&mut output),
            Response::AgentStatus { .. } => render_agent_status(&mut output, response),
            Response::ThemeValue { .. } => render_theme_value(&mut output, response),
            Response::LatencyStatsResult { .. } => render_latency_stats(&mut output, response),
            Response::Error { .. } => render_error(&mut output, response),
        }

        Ok(output)
    }
}

fn render_repository_status(
    output: &mut String,
    status: &crate::git::RepositoryStatus,
    ctx: &RenderContext<'_>,
) {
    let truncated_branch = truncate_branch(&status.branch, ctx.config.git.max_branch_length.get());
    let escaped_branch = fish_escape(&truncated_branch);
    let escaped_state = fish_escape(status.state.as_str());

    let _ = writeln!(output, "set -g __gpy_git_branch {escaped_branch}");
    let _ = writeln!(output, "set -g __gpy_git_ahead {}", status.ahead);
    let _ = writeln!(output, "set -g __gpy_git_behind {}", status.behind);
    let _ = writeln!(output, "set -g __gpy_git_staged {}", status.staged);
    let _ = writeln!(output, "set -g __gpy_git_unstaged {}", status.unstaged);
    let _ = writeln!(output, "set -g __gpy_git_untracked {}", status.untracked);
    let _ = writeln!(output, "set -g __gpy_git_conflicts {}", status.conflicts);
    let _ = writeln!(output, "set -g __gpy_git_state {escaped_state}");
}

fn render_languages(output: &mut String, response: &Response) {
    if let Response::Language { languages } = response {
        output.push_str("set -g __gpy_lang_names");
        for lang in languages {
            let escaped = fish_escape(&lang.name);
            let _ = write!(output, " {escaped}");
        }
        output.push('\n');

        output.push_str("set -g __gpy_lang_versions");
        for lang in languages {
            if let Some(version) = &lang.version {
                let escaped = fish_escape(version);
                let _ = write!(output, " {escaped}");
            } else {
                output.push_str(" \"\"");
            }
        }
        output.push('\n');

        output.push_str("set -g __gpy_lang_colors");
        for lang in languages {
            let escaped = fish_escape(lang.color.as_str());
            let _ = write!(output, " {escaped}");
        }
        output.push('\n');
    }
}

fn render_ack(output: &mut String) {
    output.push_str("set -g __gpy_ack 1\n");
}

fn render_agent_status(output: &mut String, response: &Response) {
    if let Response::AgentStatus {
        version,
        protocol_version,
        watched_repos,
        registered_clients,
        cache_entries,
    } = response
    {
        // Export protocol version for version negotiation in Fish shell
        let escaped_version = fish_escape(version);
        let _ = writeln!(output, "set -g __gpy_agent_version {escaped_version}");
        let _ = writeln!(output, "set -g __gpy_protocol_version {protocol_version}");
        let _ = writeln!(output, "set -g __gpy_status_watched_repos {watched_repos}");
        let _ = writeln!(
            output,
            "set -g __gpy_status_registered_clients {registered_clients}"
        );
        let _ = writeln!(output, "set -g __gpy_status_cache_entries {cache_entries}");
    }
}

fn render_theme_value(output: &mut String, response: &Response) {
    if let Response::ThemeValue { value } = response {
        let escaped = fish_escape(value);
        let _ = writeln!(output, "set -g __gpy_theme_value {escaped}");
    }
}

fn render_latency_stats(output: &mut String, response: &Response) {
    if let Response::LatencyStatsResult {
        min_ms,
        max_ms,
        avg_ms,
        sample_count,
    } = response
    {
        let _ = writeln!(output, "set -g __gpy_latency_min_ms {min_ms}");
        let _ = writeln!(output, "set -g __gpy_latency_max_ms {max_ms}");
        let _ = writeln!(output, "set -g __gpy_latency_avg_ms {avg_ms}");
        let _ = writeln!(output, "set -g __gpy_latency_sample_count {sample_count}");
    }
}

fn render_error(output: &mut String, response: &Response) {
    if let Response::Error { message } = response {
        let escaped = fish_escape(message);
        let _ = writeln!(output, "set -g __gpy_error {escaped}");
    }
}

/// Quote a value for a Fish `set -g` assignment.
///
/// Routes through the shared, regression-tested [`escape_shell_value`] (doubles
/// backslashes first, neutralizes `$`, escapes `"`) and wraps the result in
/// double quotes. The former single-quote-doubling idiom (`'{}'` with `'` →
/// `''`) was a SQL/POSIX habit Fish does not share: inside a single-quoted
/// string Fish reads `''` as two adjacent strings, so `'it''s'` collapses to
/// `its`, and a value ending in `\` leaves the closing quote escaped and aborts
/// sourcing of the whole block. See #424.
fn fish_escape(value: &str) -> String {
    format!("\"{}\"", escape_shell_value(value, Shell::Fish))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;
    use crate::formatter::SegmentPosition;
    use crate::ipc::LanguageInfo;
    use crate::theme::ThemeConfig;

    fn ctx() -> (Config, ThemeConfig) {
        (Config::default(), ThemeConfig::default())
    }

    #[test]
    /// # Panics
    ///
    /// Panics if the formatter fails to render the repository status response.
    fn repository_status_matches_legacy_output() {
        let formatter = FishSourceFormatter;
        let (config, theme) = ctx();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let response = Response::RepositoryStatus(crate::git::RepositoryStatus {
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
        });

        let output = formatter.render(&response, &ctx).expect("render failed");
        assert!(output.contains("set -g __gpy_git_branch \"main\""));
        assert!(output.contains("set -g __gpy_git_ahead 2"));
        assert!(output.contains("set -g __gpy_git_conflicts 0"));
    }

    #[test]
    /// # Panics
    ///
    /// Panics if the formatter fails to render the language response.
    fn language_output_matches_legacy() {
        let formatter = FishSourceFormatter;
        let (config, theme) = ctx();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let response = Response::Language {
            languages: vec![
                LanguageInfo {
                    name: "Rust".to_owned(),
                    version: Some("1.75.0".to_owned()),
                    color: crate::config::types::ColorSpec::new("#dea584").unwrap(),
                },
                LanguageInfo {
                    name: "Go".to_owned(),
                    version: None,
                    color: crate::config::types::ColorSpec::new("#00acd7").unwrap(),
                },
            ],
        };

        let output = formatter.render(&response, &ctx).expect("render failed");
        assert!(output.contains("set -g __gpy_lang_names \"Rust\" \"Go\""));
        assert!(output.contains("set -g __gpy_lang_versions \"1.75.0\" \"\""));
        assert!(output.contains("set -g __gpy_lang_colors \"#dea584\" \"#00acd7\""));
    }

    #[test]
    /// # Panics
    ///
    /// Panics if the formatter fails to render the error response.
    fn error_response_sets_variable() {
        let formatter = FishSourceFormatter;
        let (config, theme) = ctx();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let response = Response::Error {
            message: "boom".to_owned(),
        };

        let output = formatter.render(&response, &ctx).expect("render failed");
        assert_eq!(output.trim(), "set -g __gpy_error \"boom\"");
    }

    /// A branch name containing an apostrophe must survive sourcing intact.
    ///
    /// The old single-quote-doubling idiom turned `it's` into `its` because
    /// Fish does not read `''` as an escaped quote inside a single-quoted
    /// string. Regression for #424.
    ///
    /// # Panics
    ///
    /// Panics if the formatter fails to render the repository status response.
    fn status_with(branch: &str) -> String {
        let formatter = FishSourceFormatter;
        let (config, theme) = ctx();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let response = Response::RepositoryStatus(crate::git::RepositoryStatus {
            branch: branch.to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: crate::git::RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        });
        formatter.render(&response, &ctx).expect("render failed")
    }

    #[test]
    /// # Panics
    ///
    /// Panics if the apostrophe is dropped or mis-escaped in the rendered output.
    fn apostrophe_in_branch_is_preserved() {
        let output = status_with("feat/it's-broken");
        assert!(
            output.contains("set -g __gpy_git_branch \"feat/it's-broken\""),
            "apostrophe dropped or mis-escaped: {output}"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if a trailing backslash is not doubled in the rendered output.
    fn trailing_backslash_does_not_break_sourcing() {
        // A lone trailing backslash must be doubled so it cannot escape the
        // closing quote and abort the whole `set -g` block.
        let output = status_with("wip\\");
        assert!(
            output.contains("set -g __gpy_git_branch \"wip\\\\\""),
            "trailing backslash not doubled: {output}"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if an embedded `$` is not neutralized in the rendered output.
    fn embedded_dollar_is_neutralized() {
        // `$USER` must be inert data, not a Fish variable expansion.
        let output = status_with("feat/$USER");
        assert!(
            output.contains("set -g __gpy_git_branch \"feat/\\$USER\""),
            "dollar not neutralized: {output}"
        );
    }
}

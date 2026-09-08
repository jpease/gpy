//! Fish shell argument formatter
//!
//! Produces space-separated flags that mirror the legacy `format_git_status_fish`
//! and `format_languages_as_fish_args` helpers.
//!
//! This implementation is also re-exported as [`crate::formatter::ZshFormatter`]:
//! Zsh's shell-argument rendering needs the exact same space-separated
//! `--flag value` shape, so there is nothing Zsh-specific to implement
//! separately.

use crate::Result;
use crate::formatter::{Formatter, RenderContext};
use crate::ipc::Response;

/// Format responses as Fish shell arguments.
pub struct FishFormatter;

impl Formatter for FishFormatter {
    fn render(&self, response: &Response, _ctx: &RenderContext<'_>) -> Result<String> {
        match response {
            Response::RepositoryStatus(status) => {
                let parts = [
                    "--branch",
                    status.branch.as_str(),
                    "--ahead",
                    &status.ahead.to_string(),
                    "--behind",
                    &status.behind.to_string(),
                    "--staged",
                    &status.staged.to_string(),
                    "--unstaged",
                    &status.unstaged.to_string(),
                    "--untracked",
                    &status.untracked.to_string(),
                    "--conflicts",
                    &status.conflicts.to_string(),
                    "--state",
                    status.state.as_str(),
                ];

                Ok(parts.join(" "))
            }
            Response::Language { languages } => {
                if languages.is_empty() {
                    return Ok(String::new());
                }

                let mut parts = Vec::with_capacity(languages.len().saturating_mul(6));

                for lang in languages {
                    parts.push("--lang".to_owned());
                    parts.push(lang.name.clone());
                    parts.push("--version".to_owned());
                    parts.push(lang.version.clone().unwrap_or_default());
                    parts.push("--color".to_owned());
                    parts.push(lang.color.to_string());
                }

                Ok(parts.join(" "))
            }
            // Other response types do not have a Fish args representation
            _ => Ok(String::new()),
        }
    }
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
        let formatter = FishFormatter;
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
        assert_eq!(
            output,
            "--branch main --ahead 2 --behind 1 --staged 3 --unstaged 4 --untracked 5 \
             --conflicts 0 --state clean"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if the formatter fails to render the language response.
    fn language_output_matches_legacy() {
        let formatter = FishFormatter;
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
        assert_eq!(
            output,
            "--lang Rust --version 1.75.0 --color #dea584 \
             --lang Go --version  --color #00acd7"
        );
    }

    #[test]
    /// # Panics
    ///
    /// Panics if the formatter fails to render the language response.
    fn language_empty_returns_empty_string() {
        let formatter = FishFormatter;
        let (config, theme) = ctx();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);

        let response = Response::Language { languages: vec![] };
        let output = formatter.render(&response, &ctx).expect("render failed");
        assert!(output.is_empty());
    }

    #[test]
    /// # Panics
    ///
    /// Panics if either formatter fails to render the repository status response.
    fn zsh_formatter_output_matches_fish_formatter() {
        use crate::formatter::ZshFormatter;

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

        let fish_output = FishFormatter
            .render(&response, &ctx)
            .expect("render failed");
        let zsh_output = ZshFormatter.render(&response, &ctx).expect("render failed");
        assert_eq!(fish_output, zsh_output);
    }
}

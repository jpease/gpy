//! In-process live preview facts (git/language); no agent IPC.

use crate::config::{Config, LanguageSettings};
use crate::git::{CompleteStatus, RepositoryStatus};
use crate::ipc::LanguageInfo;
use crate::language::detector::{DetectedLanguage, Detector};
use crate::language::display::{DisplayRequest, Versions, display_info};
use crate::theme::ThemeConfig;
use std::path::Path;
use std::time::Duration;

/// Live and sample facts for the wizard preview.
pub struct PreviewFacts {
    /// `None` outside a repo, when git is disabled or skipped for `cwd`, or
    /// when git status failed or timed out.
    pub(crate) repository_status: Option<RepositoryStatus>,
    pub(crate) languages: Vec<LanguageInfo>,
    /// Detection result `languages` is built from, kept so a theme or
    /// `show_versions` change rebuilds them without rescanning the directory.
    detected: Vec<DetectedLanguage>,
    /// Theme name and `show_versions` that `languages` was built for.
    language_key: (String, bool),
    /// Fixed sample: there is no real "last command" in the wizard.
    pub(crate) sample_duration_ms: u64,
    /// Fixed sample for the character segment.
    pub(crate) sample_character_success: bool,
}

impl PreviewFacts {
    /// Facts with no repository or languages, for the preview tests.
    #[cfg(test)]
    pub(crate) const fn empty() -> Self {
        Self {
            repository_status: None,
            languages: Vec::new(),
            detected: Vec::new(),
            language_key: (String::new(), false),
            sample_duration_ms: 128,
            sample_character_success: true,
        }
    }

    /// Rebuild the language entries when the theme or `show_versions` changed
    /// since they were built; colours, allow-lists and versions depend on both.
    pub(crate) fn refresh_languages(
        &mut self,
        cwd: &Path,
        theme_name: &str,
        theme: &ThemeConfig,
        settings: &LanguageSettings,
    ) {
        if self.language_key.0 == theme_name && self.language_key.1 == settings.show_versions {
            return;
        }
        self.languages = language_entries(&self.detected, cwd, theme, settings);
        theme_name.clone_into(&mut self.language_key.0);
        self.language_key.1 = settings.show_versions;
    }
}

fn language_entries(
    detected: &[DetectedLanguage],
    cwd: &Path,
    theme: &ThemeConfig,
    settings: &LanguageSettings,
) -> Vec<LanguageInfo> {
    let request = DisplayRequest::new(detected, theme, settings).with_cwd(Some(cwd));
    display_info(&request, Versions::Probe).unwrap_or_default()
}

/// Gather preview facts for `cwd`, honoring the same config settings as the prompt.
///
/// A git status failure or timeout (bounded by `git.timeout_seconds`) only
/// leaves the preview without git facts.
pub fn gather(cwd: &Path, config: &Config, theme: &ThemeConfig) -> PreviewFacts {
    // Canonical, like the IPC `SafePath`, so symlinked `git.skip_paths` entries
    // match; they gate the repository root, as in the agent.
    let canonical_cwd = std::fs::canonicalize(cwd).unwrap_or_else(|_| cwd.to_path_buf());
    let git_allowed = crate::git::find_repo_root(&canonical_cwd)
        .is_some_and(|root| !config.git.is_path_skipped(&root));
    let repository_status = if git_allowed {
        crate::git::status::load_repository_state(
            cwd,
            config.git.max_ahead_behind,
            config.git.stash_enabled,
            Duration::from_secs(config.git.timeout_seconds.get()),
        )
        .inspect_err(|error| crate::debug_log!("wizard", "git status unavailable: {error}"))
        .ok()
        .flatten()
        .map(|CompleteStatus { status, .. }| {
            if config.git.show_upstream {
                status
            } else {
                status.without_upstream()
            }
        })
    } else {
        None
    };

    let detected = Detector::detect_directory_bounded(cwd, config.language.detection_mode);
    let languages = language_entries(&detected, cwd, theme, &config.language);

    PreviewFacts {
        repository_status,
        languages,
        detected,
        language_key: (
            config.ui.theme.as_str().to_owned(),
            config.language.show_versions,
        ),
        sample_duration_ms: 128,
        sample_character_success: true,
    }
}

#[cfg(test)]
#[allow(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::missing_panics_doc,
    clippy::missing_errors_doc
)]
mod tests {
    use super::*;
    use crate::config::types::DetectionMode;
    use std::process::Command;
    use tempfile::TempDir;

    /// Minimal git repo with one commit on `main`.
    fn init_temp_repo() -> TempDir {
        let dir = TempDir::new().expect("failed to create temp dir");
        let path = dir.path();

        let run = |args: &[&str]| {
            let status = Command::new("git")
                .args(args)
                .current_dir(path)
                .env("GIT_AUTHOR_NAME", "Test")
                .env("GIT_AUTHOR_EMAIL", "test@example.com")
                .env("GIT_COMMITTER_NAME", "Test")
                .env("GIT_COMMITTER_EMAIL", "test@example.com")
                .status()
                .expect("failed to spawn git");
            assert!(status.success(), "git {args:?} failed");
        };

        run(&["init", "--initial-branch=main"]);
        std::fs::write(path.join("README.md"), "hello\n").expect("failed to write README");
        run(&["add", "README.md"]);
        run(&["commit", "-m", "initial commit"]);

        dir
    }

    #[test]
    fn gather_inside_git_repo_returns_status() {
        let repo = init_temp_repo();
        let config = Config::default();
        let theme = ThemeConfig::default();

        let facts = gather(repo.path(), &config, &theme);

        let status = facts
            .repository_status
            .expect("repository_status should be Some inside a git repo");
        assert_eq!(status.branch, "main");
    }

    #[test]
    fn gather_outside_git_repo_returns_none() {
        let dir = TempDir::new().expect("failed to create temp dir");
        let config = Config::default();
        let theme = ThemeConfig::default();

        let facts = gather(dir.path(), &config, &theme);

        assert!(facts.repository_status.is_none());
    }

    #[test]
    fn gather_respects_git_skip_paths() {
        let repo = init_temp_repo();
        let mut config = Config::default();
        config.git.skip_paths = vec![repo.path().to_string_lossy().into_owned()];
        let theme = ThemeConfig::default();

        let facts = gather(repo.path(), &config, &theme);

        assert!(facts.repository_status.is_none());
    }

    #[test]
    fn gather_detects_language_from_marker_file() {
        let dir = TempDir::new().expect("failed to create temp dir");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n")
            .expect("failed to write Cargo.toml");
        let mut config = Config::default();
        config.language.detection_mode = DetectionMode::Markers;
        let theme = ThemeConfig::default();

        let facts = gather(dir.path(), &config, &theme);

        assert!(
            facts.languages.iter().any(|lang| lang.name == "rust"),
            "expected a 'rust' entry, got {:?}",
            facts.languages.iter().map(|l| &l.name).collect::<Vec<_>>()
        );
    }

    #[test]
    fn sample_facts_are_stable() {
        let dir = TempDir::new().expect("failed to create temp dir");
        let config = Config::default();
        let theme = ThemeConfig::default();

        let facts = gather(dir.path(), &config, &theme);

        assert_eq!(facts.sample_duration_ms, 128);
        assert!(facts.sample_character_success);
    }
}

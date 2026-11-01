//! In-process live preview facts (git/language); no agent IPC.

use crate::config::Config;
use crate::git::{CompleteStatus, RepositoryStatus};
use crate::ipc::LanguageInfo;
use crate::language::detector::Detector;
use crate::language::display::{DisplayRequest, Versions, display_info};
use crate::theme::ThemeConfig;
use std::path::Path;

/// Live and sample facts for the wizard preview.
pub struct PreviewFacts {
    /// `None` outside a repo or when git is disabled or skipped for `cwd`.
    pub(crate) repository_status: Option<RepositoryStatus>,
    pub(crate) languages: Vec<LanguageInfo>,
    /// Fixed sample: there is no real "last command" in the wizard.
    pub(crate) sample_duration_ms: u64,
    /// Fixed sample for the character segment.
    pub(crate) sample_character_success: bool,
}

/// Gather preview facts for `cwd`, honoring the same config settings as the prompt.
///
/// # Errors
///
/// Returns an error if git status collection fails; no repo is `Ok`.
pub fn gather(cwd: &Path, config: &Config, theme: &ThemeConfig) -> crate::Result<PreviewFacts> {
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
        )
        .map_err(|e| crate::Error::config(format!("Git status failed: {e}")))?
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

    let languages = {
        let detected = Detector::detect_directory_bounded(cwd, config.language.detection_mode);
        let request = DisplayRequest::new(&detected, theme, &config.language).with_cwd(Some(cwd));
        display_info(&request, Versions::Probe).unwrap_or_default()
    };

    Ok(PreviewFacts {
        repository_status,
        languages,
        sample_duration_ms: 128,
        sample_character_success: true,
    })
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

        let facts = gather(repo.path(), &config, &theme).expect("gather should not error");

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

        let facts = gather(dir.path(), &config, &theme).expect("gather should not error");

        assert!(facts.repository_status.is_none());
    }

    #[test]
    fn gather_respects_git_skip_paths() {
        let repo = init_temp_repo();
        let mut config = Config::default();
        config.git.skip_paths = vec![repo.path().to_string_lossy().into_owned()];
        let theme = ThemeConfig::default();

        let facts = gather(repo.path(), &config, &theme).expect("gather should not error");

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

        let facts = gather(dir.path(), &config, &theme).expect("gather should not error");

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

        let facts = gather(dir.path(), &config, &theme).expect("gather should not error");

        assert_eq!(facts.sample_duration_ms, 128);
        assert!(facts.sample_character_success);
    }
}

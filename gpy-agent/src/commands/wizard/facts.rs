//! In-process live preview facts (git/language/hostname/username); no agent IPC.

use crate::config::Config;
use crate::git::{CompleteStatus, RepositoryStatus};
use crate::ipc::LanguageInfo;
use crate::language::detector::Detector;
use crate::theme::ThemeConfig;
use std::path::Path;

/// Live and sample facts for the wizard preview.
pub struct PreviewFacts {
    /// `None` outside a repo or when git is disabled.
    pub(crate) repository_status: Option<RepositoryStatus>,
    pub(crate) languages: Vec<LanguageInfo>,
    pub(crate) hostname: String,
    pub(crate) username: String,
    /// Fixed sample: there is no real "last command" in the wizard.
    pub(crate) sample_duration_ms: u64,
    /// Fixed sample for the character segment.
    pub(crate) sample_character_success: bool,
    /// Render hostname as over SSH, since the preview always shows it.
    pub(crate) sample_is_ssh: bool,
}

/// Gather preview facts for `cwd`, honoring the same config settings as the prompt.
///
/// # Errors
///
/// Returns an error if git status collection fails; no repo is `Ok`.
pub fn gather(cwd: &Path, config: &Config, theme: &ThemeConfig) -> crate::Result<PreviewFacts> {
    let repository_status = if config.git.enabled {
        crate::git::status::load_repository_state(
            cwd,
            None,
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

    let languages = if config.language.enabled {
        let detected = Detector::detect_directory_bounded(cwd, config.language.detection_mode);
        crate::language::display::build_language_display_info_at(
            &detected,
            theme,
            &config.language,
            Some(cwd),
            None,
        )
    } else {
        Vec::new()
    };

    Ok(PreviewFacts {
        repository_status,
        languages,
        hostname: local_hostname(),
        username: local_username(),
        sample_duration_ms: 128,
        sample_character_success: true,
        sample_is_ssh: true,
    })
}

/// Best-effort hostname; falls back to a placeholder.
#[cfg(unix)]
fn local_hostname() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "localhost".to_owned())
}

#[cfg(not(unix))]
fn local_hostname() -> String {
    "localhost".to_owned()
}

/// Best-effort username from `$USER`.
fn local_username() -> String {
    std::env::var("USER").unwrap_or_else(|_| "user".to_owned())
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
    fn gather_respects_git_disabled() {
        let repo = init_temp_repo();
        let mut config = Config::default();
        config.git.enabled = false;
        let theme = ThemeConfig::default();

        let facts = gather(repo.path(), &config, &theme).expect("gather should not error");

        assert!(
            facts.repository_status.is_none(),
            "config gate should win over repo presence"
        );
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
    fn gather_respects_language_disabled() {
        let dir = TempDir::new().expect("failed to create temp dir");
        std::fs::write(dir.path().join("Cargo.toml"), "[package]\nname = \"x\"\n")
            .expect("failed to write Cargo.toml");
        let mut config = Config::default();
        config.language.enabled = false;
        config.language.detection_mode = DetectionMode::Markers;
        let theme = ThemeConfig::default();

        let facts = gather(dir.path(), &config, &theme).expect("gather should not error");

        assert!(facts.languages.is_empty());
    }

    #[test]
    fn local_hostname_is_non_empty() {
        assert_ne!(local_hostname(), "");
    }

    #[test]
    fn local_username_is_non_empty() {
        assert_ne!(local_username(), "");
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

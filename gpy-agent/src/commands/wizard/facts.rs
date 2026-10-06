//! In-process computation of live preview facts (git/language/hostname/username).
//!
//! Also computes representative sample facts for segments with no meaningful
//! wizard-session value (duration, character). No IPC, no agent dependency —
//! the only IPC client in this codebase (`ipc/client.rs`) is test-only and
//! speaks a protocol incompatible with the real agent, so the wizard computes
//! facts itself in-process instead of round-tripping to `gpy-agent`.
//! Separately, hostname and username are not computed anywhere else in this
//! crate: `agent/oneshot.rs`'s hostname/username handlers just echo values
//! Fish supplies client-side, so this module adds the first in-process lookup
//! for both.

use crate::config::Config;
use crate::git::{CompleteStatus, RepositoryStatus};
use crate::ipc::LanguageInfo;
use crate::language::detector::Detector;
use crate::theme::ThemeConfig;
use std::path::Path;

/// Live and representative-sample facts available for the wizard preview.
///
/// This is populated by [`gather`] and consumed by [`super::preview::render_preview_line`].
pub struct PreviewFacts {
    /// `None` when `cwd` isn't inside a git repository (or git is disabled via
    /// config), mirroring `load_repository_state`'s own `Option` return.
    pub(crate) repository_status: Option<RepositoryStatus>,
    pub(crate) languages: Vec<LanguageInfo>,
    pub(crate) hostname: String,
    pub(crate) username: String,
    /// Representative sample: duration has no truthful "last command" value
    /// inside a wizard session, so this is a fixed, clearly-plausible
    /// stand-in fed through the real rendering pipeline rather than a
    /// separate fake-rendering path.
    pub(crate) sample_duration_ms: u64,
    /// Representative sample: same rationale as `sample_duration_ms`, for the
    /// character segment's success/failure state.
    pub(crate) sample_character_success: bool,
}

/// Gather preview facts for `cwd`.
///
/// Uses `config` for the same settings (`git.enabled`, `git.max_ahead_behind`,
/// `git.stash_enabled`, `git.show_upstream`, `language.enabled`,
/// `language.detection_mode`) the real prompt render path honors.
///
/// # Errors
///
/// Returns an error only if git status collection itself errors (e.g. a
/// filesystem I/O failure) — being outside a repository, or git being
/// disabled via config, is `Ok` with `repository_status: None`, not an error.
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
    })
}

/// Best-effort local hostname lookup.
///
/// No existing in-process hostname source exists in this crate (Fish
/// supplies it in the real IPC request path) — this uses `sysinfo` (already a
/// workspace dependency for PID lookups; see `gpy-agent/Cargo.toml`'s
/// `sysinfo` entry, which is Unix-only, matching this crate's Unix/WSL-only
/// support). Falls back to a fixed placeholder rather than erroring, since
/// the wizard preview should still render without a real hostname.
#[cfg(unix)]
fn local_hostname() -> String {
    sysinfo::System::host_name().unwrap_or_else(|| "localhost".to_owned())
}

/// Non-Unix fallback: `sysinfo` is a Unix-only dependency in this crate (see
/// `Cargo.toml`'s `[target."cfg(unix)".dependencies]`), and GPY otherwise only
/// targets Unix and WSL, so there is no real lookup to perform here.
#[cfg(not(unix))]
fn local_hostname() -> String {
    "localhost".to_owned()
}

/// Best-effort local username lookup via the `USER` environment variable
/// (set on every Unix shell this project targets: GPY runs on Unix and WSL,
/// with no native-Windows IPC support).
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

    /// Initialize a minimal git repository with one commit at `dir`, on branch
    /// `main`.
    ///
    /// Mirrors the "temp git repo via `git` subprocess" pattern used by
    /// `tests/fixtures::TestRepo` (unreachable from this crate's unit tests,
    /// which compile separately from the `tests/` integration binary), kept
    /// deliberately minimal since this module only needs a repo to exist.
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

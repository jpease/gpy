//! Python virtualenv resolution and version reading.
//!
//! The agent daemon never inherits an interactive shell's activated
//! `VIRTUAL_ENV`, so bare `python --version` resolves the daemon's global
//! interpreter. This module lets the Python segment prefer a project venv:
//! a forwarded `VIRTUAL_ENV`, else a project-local `.venv`/`venv` directory.

use std::path::{Path, PathBuf};

use crate::cache::ttl_map::TtlMap;

/// Candidate venv directory names probed under a project root, in order.
const VENV_DIR_NAMES: [&str; 2] = [".venv", "venv"];

/// Resolve the venv directory to use for Python version detection.
///
/// Precedence: a forwarded, existing `virtual_env`; then `<cwd>/.venv`, then
/// `<cwd>/venv`. Returns `None` when no venv directory exists.
#[must_use]
pub fn resolve_python_venv(cwd: &Path, virtual_env: Option<&Path>) -> Option<PathBuf> {
    if let Some(forwarded) = virtual_env
        && is_venv_dir(forwarded)
    {
        return Some(forwarded.to_path_buf());
    }
    VENV_DIR_NAMES
        .iter()
        .map(|name| cwd.join(name))
        .find(|candidate| is_venv_dir(candidate))
}

/// A directory is treated as a venv when it holds either a `pyvenv.cfg` or a
/// Python interpreter under the platform's bin directory.
fn is_venv_dir(dir: &Path) -> bool {
    dir.join("pyvenv.cfg").is_file() || venv_python_binary(dir).is_file()
}

/// Path to the venv's Python interpreter (`bin/python` on Unix,
/// `Scripts\python.exe` on Windows).
#[must_use]
pub fn venv_python_binary(venv_dir: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        venv_dir.join("Scripts").join("python.exe")
    }
    #[cfg(not(windows))]
    {
        venv_dir.join("bin").join("python")
    }
}

/// Cap on `pyvenv.cfg` reads: a single-line-to-few-line file by
/// construction, so 4 `KiB` comfortably covers any legitimate file while
/// bounding a maliciously or accidentally huge one.
const PYVENV_CFG_READ_CAP: usize = 4_096;

/// Read the `version = X.Y.Z` line from `<venv_dir>/pyvenv.cfg`.
///
/// Returns `None` when the file is absent, unreadable, or has no `version` key.
#[must_use]
pub fn read_pyvenv_cfg_version(venv_dir: &Path) -> Option<String> {
    let contents =
        crate::fs_util::read_small_file(&venv_dir.join("pyvenv.cfg"), PYVENV_CFG_READ_CAP).ok()?;
    parse_pyvenv_cfg_version(&contents)
}

/// Parse a `pyvenv.cfg` file's contents for its `version = ...` line.
#[must_use]
fn parse_pyvenv_cfg_version(contents: &str) -> Option<String> {
    contents.lines().find_map(|line| {
        let (key, value) = line.split_once('=')?;
        (key.trim() == "version").then(|| value.trim().to_owned())
    })
}

/// Read the version reported by a venv: `pyvenv.cfg` first (no subprocess),
/// then invoking the venv's Python interpreter as a fallback.
///
/// Returns `None` when the directory is not a usable venv.
#[must_use]
pub fn read_venv_version(venv_dir: &Path) -> Option<String> {
    if let Some(version) = read_pyvenv_cfg_version(venv_dir) {
        return Some(version);
    }
    let python = venv_python_binary(venv_dir);
    python
        .is_file()
        .then(|| crate::language::version::detect_python_binary_version(&python))
        .flatten()
}

/// Max repos tracked by the forwarded-venv stash.
const VENV_STASH_CAPACITY: usize = 256;

/// How long a stashed forwarded venv stays authoritative for its repo.
///
/// Shares [`crate::language::version`]'s TTL knob via [`TtlMap`] rather than
/// a fixed duration of its own (#589) — venv stashing exists to support
/// Python version-detection accuracy, the same config domain
/// `config.language.cache_ttl_hours` already governs, so a change to that
/// setting now also reaches this stash instead of the two silently drifting
/// apart.
static VENV_STASH: TtlMap<PathBuf, PathBuf> = TtlMap::new(VENV_STASH_CAPACITY);

fn stash_key(repo_root: &Path) -> PathBuf {
    repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf())
}

/// Record the forwarded venv for a repo so later background detection renders consistently.
///
/// The background detection job (which has no IPC request context) then renders the same
/// interpreter as the synchronous handler — preventing a version flip-flop on the next
/// refresh.
///
pub fn stash_project_venv(repo_root: &Path, venv: &Path) {
    VENV_STASH.insert(stash_key(repo_root), venv.to_path_buf());
}

/// Fetch a repo's stashed forwarded venv, dropping it if the TTL lapsed.
#[must_use]
pub fn stashed_project_venv(repo_root: &Path) -> Option<PathBuf> {
    VENV_STASH.get(&stash_key(repo_root))
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use std::time::{Duration, SystemTime};

    fn write_pyvenv(dir: &Path, version: &str) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(
            dir.join("pyvenv.cfg"),
            format!("home = /x\nversion = {version}\n"),
        )
        .unwrap();
    }

    #[test]
    fn reads_version_from_pyvenv_cfg() {
        let tmp = tempfile::tempdir().unwrap();
        write_pyvenv(tmp.path(), "3.11.9");
        assert_eq!(
            read_pyvenv_cfg_version(tmp.path()).as_deref(),
            Some("3.11.9")
        );
    }

    #[test]
    fn missing_pyvenv_cfg_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(read_pyvenv_cfg_version(tmp.path()), None);
    }

    #[test]
    fn parse_pyvenv_cfg_version_table() {
        let cases: &[(&str, Option<&str>)] = &[
            ("home = /x\nversion = 3.11.9\n", Some("3.11.9")),
            ("version=3.11.9", Some("3.11.9")),
            ("version =   3.11.9  \n", Some("3.11.9")),
            ("home = /x\n", None),
            ("", None),
            ("not-a-key-value-line\n", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_pyvenv_cfg_version(input).as_deref(),
                *expected,
                "input: {input:?}"
            );
        }
    }

    #[test]
    fn resolves_dot_venv_in_project() {
        let tmp = tempfile::tempdir().unwrap();
        write_pyvenv(&tmp.path().join(".venv"), "3.11.9");
        assert_eq!(
            resolve_python_venv(tmp.path(), None),
            Some(tmp.path().join(".venv"))
        );
    }

    #[test]
    fn prefers_dot_venv_over_venv() {
        let tmp = tempfile::tempdir().unwrap();
        write_pyvenv(&tmp.path().join(".venv"), "3.11.9");
        write_pyvenv(&tmp.path().join("venv"), "3.9.0");
        assert_eq!(
            resolve_python_venv(tmp.path(), None),
            Some(tmp.path().join(".venv"))
        );
    }

    #[test]
    fn forwarded_virtual_env_overrides_project_venv() {
        let project = tempfile::tempdir().unwrap();
        let external = tempfile::tempdir().unwrap();
        write_pyvenv(&project.path().join(".venv"), "3.11.9");
        write_pyvenv(external.path(), "3.12.4");
        assert_eq!(
            resolve_python_venv(project.path(), Some(external.path())),
            Some(external.path().to_path_buf())
        );
    }

    #[test]
    fn nonexistent_forwarded_venv_is_ignored() {
        let project = tempfile::tempdir().unwrap();
        write_pyvenv(&project.path().join(".venv"), "3.11.9");
        let missing = project.path().join("does-not-exist");
        assert_eq!(
            resolve_python_venv(project.path(), Some(&missing)),
            Some(project.path().join(".venv"))
        );
    }

    #[test]
    fn no_venv_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(resolve_python_venv(tmp.path(), None), None);
    }

    #[test]
    fn read_venv_version_uses_pyvenv_cfg_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        write_pyvenv(tmp.path(), "3.11.9");
        assert_eq!(read_venv_version(tmp.path()).as_deref(), Some("3.11.9"));
    }

    #[test]
    fn stash_round_trips_by_repo_root() {
        let repo = tempfile::tempdir().unwrap();
        let venv = repo.path().join(".venv");
        assert_eq!(stashed_project_venv(repo.path()), None);
        stash_project_venv(repo.path(), &venv);
        // The stash canonicalizes keys; compare against the canonical repo path.
        let expected = repo.path().canonicalize().unwrap().join(".venv");
        // Stored venv path is returned verbatim (not canonicalized).
        assert_eq!(stashed_project_venv(repo.path()), Some(venv));
        assert!(expected.ends_with(".venv"));
    }

    /// Regression for #589: the venv stash previously had its own hardcoded
    /// 24h TTL, independent of `config.language.cache_ttl_hours`. It now
    /// shares that knob via `TtlMap`, so narrowing/widening the shared TTL
    /// changes stash freshness too.
    #[test]
    #[serial_test::serial(global_ttl)]
    fn stash_freshness_tracks_the_shared_cache_ttl_knob() {
        crate::language::version::set_version_cache_ttl(1); // narrow to 1h

        let repo = tempfile::tempdir().unwrap();
        let venv = repo.path().join(".venv");
        let key = stash_key(repo.path());
        let stamped_at = SystemTime::now() - Duration::from_secs(3600 + 5);

        VENV_STASH.insert_at(key.clone(), venv.clone(), stamped_at);
        assert_eq!(
            stashed_project_venv(repo.path()),
            None,
            "an entry just over an hour old should read as stale under a 1h TTL"
        );

        // Re-stamp the same entry at the same age, then widen the shared TTL.
        // If the stash still had its own independent 24h TTL this would make
        // no difference; because it now reads the shared knob, the identical
        // entry becomes fresh again.
        VENV_STASH.insert_at(key, venv.clone(), stamped_at);
        crate::language::version::set_version_cache_ttl(24);
        assert_eq!(
            stashed_project_venv(repo.path()),
            Some(venv),
            "the same entry should read as fresh once the shared TTL widens to 24h"
        );
    }
}

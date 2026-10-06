//! Python virtualenv resolution and version reading.
//!
//! The agent daemon never inherits an interactive shell's activated
//! `VIRTUAL_ENV`, so bare `python --version` resolves the daemon's global
//! interpreter. This module lets the Python segment prefer a project venv:
//! a forwarded `VIRTUAL_ENV`, else a project-local `.venv`/`venv` directory.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

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

/// Read the venv's Python version from `<venv_dir>/pyvenv.cfg`.
///
/// Returns `None` when the file is absent, unreadable, or has neither a
/// `version` nor a `version_info` key.
#[must_use]
pub fn read_pyvenv_cfg_version(venv_dir: &Path) -> Option<String> {
    let contents =
        crate::fs_util::read_small_file(&venv_dir.join("pyvenv.cfg"), PYVENV_CFG_READ_CAP).ok()?;
    parse_pyvenv_cfg_version(&contents)
}

/// Parse a `pyvenv.cfg` file's contents for the interpreter version.
///
/// stdlib `venv` writes `version = X.Y.Z`; uv and virtualenv write
/// `version_info = X.Y.Z[.releaselevel.serial]` instead. `version` wins when
/// both are present; `version_info` is normalized to at most three numeric
/// components (`3.11.4.final.0` becomes `3.11.4`).
#[must_use]
fn parse_pyvenv_cfg_version(contents: &str) -> Option<String> {
    let find_value = |wanted: &str| {
        contents.lines().find_map(|line| {
            let (key, value) = line.split_once('=')?;
            (key.trim() == wanted).then(|| value.trim())
        })
    };
    if let Some(version) = find_value("version") {
        return Some(version.to_owned());
    }
    let components: Vec<&str> = find_value("version_info")?
        .split('.')
        .take_while(|part| !part.is_empty() && part.bytes().all(|byte| byte.is_ascii_digit()))
        .take(3)
        .collect();
    (!components.is_empty()).then(|| components.join("."))
}

/// Memoized interpreter-fallback versions, keyed by canonical interpreter path
/// plus its mtime so recreating the venv busts the entry. Successes only;
/// freshness follows the shared TTL knob via [`TtlMap`].
static VENV_BINARY_VERSION_CACHE: TtlMap<(PathBuf, SystemTime), String> =
    TtlMap::new(VENV_STASH_CAPACITY);

/// Interpreters whose probe failed, with the failure time. A failure
/// suppresses re-probing only for [`VERSION_FAILURE_TTL`] (the #689 policy),
/// not the full shared TTL.
static VENV_BINARY_FAILURE_CACHE: TtlMap<(PathBuf, SystemTime), SystemTime> =
    TtlMap::new(VENV_STASH_CAPACITY);

/// Probe the venv interpreter for its version, memoizing the outcome.
fn memoized_binary_version(python: &Path) -> Option<String> {
    let canonical = python.canonicalize().ok()?;
    let modified = std::fs::metadata(&canonical).ok()?.modified().ok()?;
    let key = (canonical, modified);
    if let Some(version) = VENV_BINARY_VERSION_CACHE.get(&key) {
        return Some(version);
    }
    let recently_failed = VENV_BINARY_FAILURE_CACHE
        .get(&key)
        .is_some_and(|recorded_at| {
            SystemTime::now()
                .duration_since(recorded_at)
                .is_ok_and(|elapsed| elapsed < crate::language::version::VERSION_FAILURE_TTL)
        });
    if recently_failed {
        return None;
    }
    if let Some(version) = crate::language::version::detect_python_binary_version(python) {
        VENV_BINARY_FAILURE_CACHE.remove(&key);
        VENV_BINARY_VERSION_CACHE.insert(key, version.clone());
        Some(version)
    } else {
        VENV_BINARY_FAILURE_CACHE.insert(key, SystemTime::now());
        None
    }
}

/// Read the version reported by a venv: `pyvenv.cfg` first (no subprocess),
/// then invoking the venv's Python interpreter as a memoized fallback.
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
        .then(|| memoized_binary_version(&python))
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
            ("home = /x\nversion_info = 3.12.13\n", Some("3.12.13")),
            ("version_info = 3.11.4.final.0\n", Some("3.11.4")),
            (
                "version = 3.10.1\nversion_info = 3.10.2.final.0\n",
                Some("3.10.1"),
            ),
            ("version_info = final\n", None),
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

    /// Shared vectors (`tests/fixtures/venv_forwarding_vectors.tsv`, #729).
    ///
    /// The same file is read by the Bash/Zsh/Fish tests: a forwarded env (`VIRTUAL_ENV`, or a non-base conda env) beats
    /// the project `.venv`; when the shells forward nothing, the project `.venv` wins.
    #[test]
    fn venv_forwarding_shared_vectors() {
        let fixture = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../tests/fixtures/venv_forwarding_vectors.tsv");
        let text = std::fs::read_to_string(&fixture)
            .unwrap_or_else(|e| panic!("cannot read {}: {e}", fixture.display()));
        let mut vectors = 0_u32;
        for line in text.lines() {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let cols: Vec<&str> = line.split('\t').collect();
            assert_eq!(cols.len(), 4, "bad vector line: {line:?}");
            let expected = cols.get(3).copied().unwrap();
            vectors += 1;

            let tmp = tempfile::tempdir().unwrap();
            let project = tmp.path().join("project");
            write_pyvenv(&project.join(".venv"), "3.11.9");
            let (forwarded, want) = if expected == "\\e" {
                (None, project.join(".venv"))
            } else {
                // Re-root the fixture's absolute path inside the tempdir as a conda-style
                // env (no pyvenv.cfg, only bin/python).
                let env_dir = tmp.path().join(expected.trim_start_matches('/'));
                let python = venv_python_binary(&env_dir);
                std::fs::create_dir_all(python.parent().unwrap()).unwrap();
                std::fs::write(&python, "").unwrap();
                (Some(env_dir.clone()), env_dir)
            };
            assert_eq!(
                resolve_python_venv(&project, forwarded.as_deref()),
                Some(want),
                "vector {line:?}"
            );
        }
        assert!(vectors > 0, "no vectors read from {}", fixture.display());
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
    fn read_venv_version_uses_version_info_without_spawning() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("pyvenv.cfg"),
            "home = /x\nuv = 0.9.16\nversion_info = 3.12.13\n",
        )
        .unwrap();
        assert_eq!(read_venv_version(tmp.path()).as_deref(), Some("3.12.13"));
    }

    /// Write an executable `bin/python` that appends to `counter` per spawn
    /// and prints `stdout_line` (failing when `exit_code` is non-zero).
    #[cfg(unix)]
    fn write_counting_python(venv: &Path, counter: &Path, stdout_line: &str, exit_code: u8) {
        use std::os::unix::fs::PermissionsExt;
        let bin = venv.join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        let python = bin.join("python");
        std::fs::write(
            &python,
            format!(
                "#!/bin/sh\necho x >> '{}'\necho '{stdout_line}'\nexit {exit_code}\n",
                counter.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&python, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[cfg(unix)]
    fn spawn_count(counter: &Path) -> usize {
        std::fs::read_to_string(counter).map_or(0, |text| text.lines().count())
    }

    #[cfg(unix)]
    #[test]
    fn interpreter_fallback_is_memoized() {
        let tmp = tempfile::tempdir().unwrap();
        let counter = tmp.path().join("spawns.log");
        write_counting_python(tmp.path(), &counter, "Python 3.9.1", 0);
        assert_eq!(read_venv_version(tmp.path()).as_deref(), Some("3.9.1"));
        assert_eq!(read_venv_version(tmp.path()).as_deref(), Some("3.9.1"));
        assert_eq!(spawn_count(&counter), 1, "second call must hit the memo");
    }

    #[cfg(unix)]
    #[test]
    fn interpreter_fallback_failure_is_cached_only_briefly() {
        let tmp = tempfile::tempdir().unwrap();
        let counter = tmp.path().join("spawns.log");
        write_counting_python(tmp.path(), &counter, "broken", 1);
        assert_eq!(read_venv_version(tmp.path()), None);
        assert_eq!(read_venv_version(tmp.path()), None);
        assert_eq!(
            spawn_count(&counter),
            1,
            "a fresh failure suppresses re-probing"
        );

        // Age the recorded failure past VERSION_FAILURE_TTL: it must be retried.
        let python = venv_python_binary(tmp.path()).canonicalize().unwrap();
        let modified = std::fs::metadata(&python).unwrap().modified().unwrap();
        VENV_BINARY_FAILURE_CACHE.insert(
            (python, modified),
            SystemTime::now() - crate::language::version::VERSION_FAILURE_TTL * 2,
        );
        assert_eq!(read_venv_version(tmp.path()), None);
        assert_eq!(spawn_count(&counter), 2, "an aged failure is re-probed");
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

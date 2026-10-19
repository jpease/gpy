//! Python virtualenv resolution and version reading.
//!
//! The daemon never inherits a shell's activated `VIRTUAL_ENV`, so the Python
//! segment prefers a forwarded venv, else a project `.venv`/`venv`.
//!
//! Only a forwarded venv's interpreter may be executed: a project venv is
//! repository content, so its version comes from `pyvenv.cfg` alone.

use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::cache::ttl_map::TtlMap;
use crate::language::version::{ColdVersion, ProbePolicy};

/// Candidate venv directory names probed under a project root, in order.
const VENV_DIR_NAMES: [&str; 2] = [".venv", "venv"];

/// A resolved venv, tagged with where it came from because that decides
/// whether its interpreter may run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PythonVenv {
    /// The shell's activated `VIRTUAL_ENV`, forwarded over IPC.
    Forwarded(PathBuf),
    /// A `.venv`/`venv` found under the project; never executed.
    Discovered(PathBuf),
}

impl PythonVenv {
    /// The venv's Python version: `pyvenv.cfg`, then, for a forwarded venv
    /// only, a memoized interpreter probe.
    #[must_use]
    pub fn version(&self) -> Option<String> {
        self.lookup_version(ProbePolicy::Spawn).ok().flatten()
    }

    /// [`Self::version`] under `policy`.
    ///
    /// # Errors
    /// [`ColdVersion`] when only an interpreter probe could answer and
    /// `policy` forbids running it.
    pub fn lookup_version(&self, policy: ProbePolicy) -> Result<Option<String>, ColdVersion> {
        match self {
            Self::Forwarded(dir) => {
                if let Some(version) = read_pyvenv_cfg_version(dir) {
                    return Ok(Some(version));
                }
                let python = venv_python_binary(dir);
                if python.is_file() {
                    memoized_binary_version(&python, policy)
                } else {
                    Ok(None)
                }
            }
            Self::Discovered(dir) => Ok(read_pyvenv_cfg_version(dir)),
        }
    }
}

/// Resolve the venv for Python version detection: forwarded `virtual_env`,
/// then `<cwd>/.venv`, then `<cwd>/venv`.
#[must_use]
pub fn resolve_python_venv(cwd: &Path, virtual_env: Option<&Path>) -> Option<PythonVenv> {
    if let Some(forwarded) = virtual_env
        && is_venv_dir(forwarded)
    {
        return Some(PythonVenv::Forwarded(forwarded.to_path_buf()));
    }
    VENV_DIR_NAMES
        .iter()
        .map(|name| cwd.join(name))
        .find(|candidate| is_venv_dir(candidate))
        .map(PythonVenv::Discovered)
}

/// A venv holds a `pyvenv.cfg` or a bin-dir Python interpreter.
fn is_venv_dir(dir: &Path) -> bool {
    dir.join("pyvenv.cfg").is_file() || venv_python_binary(dir).is_file()
}

/// Path to the venv's Python interpreter.
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

/// Cap on `pyvenv.cfg` reads, bounding a pathologically huge file.
const PYVENV_CFG_READ_CAP: usize = 4_096;

/// Read the venv's Python version from `<venv_dir>/pyvenv.cfg`.
fn read_pyvenv_cfg_version(venv_dir: &Path) -> Option<String> {
    let contents =
        crate::fs_util::read_small_file(&venv_dir.join("pyvenv.cfg"), PYVENV_CFG_READ_CAP).ok()?;
    parse_pyvenv_cfg_version(&contents)
}

/// Parse `pyvenv.cfg` for the interpreter version. stdlib `venv` writes
/// `version`; uv/virtualenv write `version_info` (`3.11.4.final.0` → `3.11.4`).
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

/// Successful interpreter-probe versions, keyed by canonical path + mtime so
/// recreating the venv busts the entry.
static VENV_BINARY_VERSION_CACHE: TtlMap<(PathBuf, SystemTime), String> =
    TtlMap::new(VENV_STASH_CAPACITY);

/// Failed interpreter probes; suppress re-probing only for [`VERSION_FAILURE_TTL`].
static VENV_BINARY_FAILURE_CACHE: TtlMap<(PathBuf, SystemTime), SystemTime> =
    TtlMap::new(VENV_STASH_CAPACITY);

/// Probe the venv interpreter for its version, memoizing the outcome;
/// [`ColdVersion`] when nothing is memoized and `policy` forbids the probe.
fn memoized_binary_version(
    python: &Path,
    policy: ProbePolicy,
) -> Result<Option<String>, ColdVersion> {
    let Some(key) = python.canonicalize().ok().and_then(|canonical| {
        let modified = std::fs::metadata(&canonical).ok()?.modified().ok()?;
        Some((canonical, modified))
    }) else {
        return Ok(None);
    };
    if let Some(version) = VENV_BINARY_VERSION_CACHE.get(&key) {
        return Ok(Some(version));
    }
    let recently_failed = VENV_BINARY_FAILURE_CACHE
        .get(&key)
        .is_some_and(|recorded_at| {
            SystemTime::now()
                .duration_since(recorded_at)
                .is_ok_and(|elapsed| elapsed < crate::language::version::VERSION_FAILURE_TTL)
        });
    if recently_failed {
        return Ok(None);
    }
    if policy == ProbePolicy::CachedOnly {
        return Err(ColdVersion);
    }
    if let Some(version) = crate::language::version::detect_python_binary_version(python) {
        VENV_BINARY_FAILURE_CACHE.remove(&key);
        VENV_BINARY_VERSION_CACHE.insert(key, version.clone());
        Ok(Some(version))
    } else {
        VENV_BINARY_FAILURE_CACHE.insert(key, SystemTime::now());
        Ok(None)
    }
}

/// Max repos tracked by the forwarded-venv stash.
const VENV_STASH_CAPACITY: usize = 256;

/// Forwarded venv per repo; shares the version cache's TTL knob.
static VENV_STASH: TtlMap<PathBuf, PathBuf> = TtlMap::new(VENV_STASH_CAPACITY);

fn stash_key(repo_root: &Path) -> PathBuf {
    repo_root
        .canonicalize()
        .unwrap_or_else(|_| repo_root.to_path_buf())
}

/// Record a repo's forwarded venv so background detection (no IPC context)
/// renders the same interpreter as the synchronous handler.
pub fn stash_project_venv(repo_root: &Path, venv: &Path) {
    VENV_STASH.insert(stash_key(repo_root), venv.to_path_buf());
}

/// Drop a repo's stashed forwarded venv.
pub fn clear_project_venv(repo_root: &Path) {
    VENV_STASH.remove(&stash_key(repo_root));
}

/// Fetch a repo's stashed forwarded venv, dropping it if the TTL lapsed.
#[must_use]
pub fn stashed_project_venv(repo_root: &Path) -> Option<PythonVenv> {
    VENV_STASH
        .get(&stash_key(repo_root))
        .map(PythonVenv::Forwarded)
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
            Some(PythonVenv::Discovered(tmp.path().join(".venv")))
        );
    }

    #[test]
    fn prefers_dot_venv_over_venv() {
        let tmp = tempfile::tempdir().unwrap();
        write_pyvenv(&tmp.path().join(".venv"), "3.11.9");
        write_pyvenv(&tmp.path().join("venv"), "3.9.0");
        assert_eq!(
            resolve_python_venv(tmp.path(), None),
            Some(PythonVenv::Discovered(tmp.path().join(".venv")))
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
            Some(PythonVenv::Forwarded(external.path().to_path_buf()))
        );
    }

    /// Shared with the shell tests: a forwarded env beats the project `.venv`.
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
                (None, PythonVenv::Discovered(project.join(".venv")))
            } else {
                // Re-root the fixture path in the tempdir as a conda-style env.
                let env_dir = tmp.path().join(expected.trim_start_matches('/'));
                let python = venv_python_binary(&env_dir);
                std::fs::create_dir_all(python.parent().unwrap()).unwrap();
                std::fs::write(&python, "").unwrap();
                (Some(env_dir.clone()), PythonVenv::Forwarded(env_dir))
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
            Some(PythonVenv::Discovered(project.path().join(".venv")))
        );
    }

    #[test]
    fn no_venv_returns_none() {
        let tmp = tempfile::tempdir().unwrap();
        assert_eq!(resolve_python_venv(tmp.path(), None), None);
    }

    #[test]
    fn venv_version_uses_pyvenv_cfg_when_present() {
        let tmp = tempfile::tempdir().unwrap();
        write_pyvenv(tmp.path(), "3.11.9");
        let dir = tmp.path().to_path_buf();
        for venv in [
            PythonVenv::Forwarded(dir.clone()),
            PythonVenv::Discovered(dir),
        ] {
            assert_eq!(venv.version().as_deref(), Some("3.11.9"), "{venv:?}");
        }
    }

    #[test]
    fn venv_version_uses_version_info_without_spawning() {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(
            tmp.path().join("pyvenv.cfg"),
            "home = /x\nuv = 0.9.16\nversion_info = 3.12.13\n",
        )
        .unwrap();
        let venv = PythonVenv::Discovered(tmp.path().to_path_buf());
        assert_eq!(venv.version().as_deref(), Some("3.12.13"));
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
        let venv = PythonVenv::Forwarded(tmp.path().to_path_buf());
        assert_eq!(venv.version().as_deref(), Some("3.9.1"));
        assert_eq!(venv.version().as_deref(), Some("3.9.1"));
        assert_eq!(spawn_count(&counter), 1, "second call must hit the memo");
    }

    #[cfg(unix)]
    #[test]
    fn interpreter_fallback_failure_is_cached_only_briefly() {
        let tmp = tempfile::tempdir().unwrap();
        let counter = tmp.path().join("spawns.log");
        write_counting_python(tmp.path(), &counter, "broken", 1);
        let venv = PythonVenv::Forwarded(tmp.path().to_path_buf());
        assert_eq!(venv.version(), None);
        assert_eq!(venv.version(), None);
        assert_eq!(
            spawn_count(&counter),
            1,
            "a fresh failure suppresses re-probing"
        );

        let python = venv_python_binary(tmp.path()).canonicalize().unwrap();
        let modified = std::fs::metadata(&python).unwrap().modified().unwrap();
        VENV_BINARY_FAILURE_CACHE.insert(
            (python, modified),
            SystemTime::now() - crate::language::version::VERSION_FAILURE_TTL * 2,
        );
        assert_eq!(venv.version(), None);
        assert_eq!(spawn_count(&counter), 2, "an aged failure is re-probed");
    }

    /// A discovered venv's interpreter is repository content: whatever state
    /// `pyvenv.cfg` is in, it is never run.
    #[cfg(unix)]
    #[test]
    fn discovered_venv_interpreter_is_never_executed() {
        let cfgs: [Option<&str>; 3] = [
            None,
            Some("home = /x\nversion_info = final\n"),
            Some("home = /x\ninclude-system-site-packages = false\n"),
        ];
        for cfg in cfgs {
            let tmp = tempfile::tempdir().unwrap();
            let venv_dir = tmp.path().join(".venv");
            let counter = tmp.path().join("spawns.log");
            write_counting_python(&venv_dir, &counter, "Python 3.9.1", 0);
            if let Some(contents) = cfg {
                std::fs::write(venv_dir.join("pyvenv.cfg"), contents).unwrap();
            }

            let venv = resolve_python_venv(tmp.path(), None).unwrap();
            assert_eq!(venv, PythonVenv::Discovered(venv_dir), "cfg {cfg:?}");
            assert_eq!(venv.version(), None, "cfg {cfg:?}");
            assert_eq!(spawn_count(&counter), 0, "cfg {cfg:?}");
        }
    }

    #[test]
    fn stash_round_trips_by_repo_root() {
        let repo = tempfile::tempdir().unwrap();
        let venv = repo.path().join(".venv");
        assert_eq!(stashed_project_venv(repo.path()), None);
        stash_project_venv(repo.path(), &venv);
        let expected = repo.path().canonicalize().unwrap().join(".venv");
        assert_eq!(
            stashed_project_venv(repo.path()),
            Some(PythonVenv::Forwarded(venv))
        );
        assert!(expected.ends_with(".venv"));
    }

    #[test]
    fn clear_project_venv_removes_the_stash() {
        let repo = tempfile::tempdir().unwrap();
        let venv = repo.path().join(".venv");
        stash_project_venv(repo.path(), &venv);
        assert_eq!(
            stashed_project_venv(repo.path()),
            Some(PythonVenv::Forwarded(venv))
        );
        clear_project_venv(repo.path());
        assert_eq!(stashed_project_venv(repo.path()), None);
    }

    /// The venv stash follows the shared `cache_ttl_hours` knob.
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

        // Re-stamp at the same age, then widen the shared TTL: entry is fresh again.
        VENV_STASH.insert_at(key, venv.clone(), stamped_at);
        crate::language::version::set_version_cache_ttl(24);
        assert_eq!(
            stashed_project_venv(repo.path()),
            Some(PythonVenv::Forwarded(venv)),
            "the same entry should read as fresh once the shared TTL widens to 24h"
        );
    }
}

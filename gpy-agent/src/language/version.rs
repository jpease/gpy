//! Language version detection via tool subprocesses, with caching.

use crate::cache::bounded::VERSION_CACHE_CAPACITY;
use crate::cache::ttl_map::TtlMap;
use crate::{Error, Result};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

use super::cache;

/// Trait for detecting language versions
pub trait ReleaseSource: Send + Sync {
    /// Language name this detector handles
    fn language_name(&self) -> &'static str;

    /// Command to execute for version detection
    fn version_command(&self) -> &[&str];

    /// Parse version from command output (return X.Y.Z format)
    fn parse_version(&self, output: &str) -> Option<String>;

    /// Whether command outputs to stderr instead of stdout
    fn uses_stderr(&self) -> bool {
        false
    }

    /// Programs tried in order when the previous one was not found
    /// (`NotFound` only); each reuses the same arguments.
    fn fallback_programs(&self) -> &[&str] {
        &[]
    }
}

/// Look up `cache_key` in the shared version cache, evicting it if stale.
fn get_cached_version(cache_key: &str) -> cache::CacheLookup {
    VERSION_CACHE
        .get(cache_key)
        .map_or(cache::CacheLookup::Miss, cache::CacheLookup::Hit)
}

/// Execute version command and parse output, bypassing the cache.
/// `pub` for Criterion benches.
///
/// # Errors
/// If the version command cannot be executed successfully.
pub fn execute_version_command<P: ReleaseSource + ?Sized>(
    detector: &P,
    cwd: Option<&Path>,
) -> Result<Option<String>> {
    let command_args = detector.version_command();
    let Some((program, args)) = command_args.split_first() else {
        return Ok(None);
    };

    // Skip mise when it previously failed here. Timeouts propagate via `?`
    // before negative caching, so a slow mise is never poisoned.
    if !mise_known_negative(program, cwd)
        && let Some(output) = execute_mise_version_command(program, args, cwd)?
    {
        if output.status.success() {
            return Ok(parse_version_output(detector, &output));
        }
        record_mise_negative(program, cwd);
    }

    let candidates = std::iter::once(*program).chain(detector.fallback_programs().iter().copied());
    let mut last_not_found = None;
    for candidate in candidates {
        let child = match spawn_direct_version_command(candidate, args, cwd) {
            Ok(child) => child,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                last_not_found = Some((candidate, e));
                continue;
            }
            Err(e) => {
                return Err(Error::language(format!(
                    "Failed to execute {candidate}: {e}"
                )));
            }
        };

        let output = wait_with_timeout(child, candidate, VERSION_COMMAND_TIMEOUT)?;
        if !output.status.success() {
            return Ok(None);
        }
        return Ok(parse_version_output(detector, &output));
    }

    Err(last_not_found.map_or_else(
        || Error::language(format!("Failed to execute {program}")),
        |(candidate, e)| Error::language(format!("Failed to execute {candidate}: {e}")),
    ))
}

/// Spawn a version command directly; returns the raw error so callers can
/// detect `NotFound`.
///
/// # Errors
/// If the program cannot be started.
fn spawn_direct_version_command(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> std::io::Result<Child> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(path) = cwd {
        command.current_dir(path);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::process::spawn_in_own_process_group(&mut command);
    command.spawn()
}

/// Version of a forwarded venv's Python binary when its `pyvenv.cfg` has none.
pub(crate) fn detect_python_binary_version(python: &Path) -> Option<String> {
    let mut command = Command::new(python);
    command.arg("--version");
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::process::spawn_in_own_process_group(&mut command);
    let child = command.spawn().ok()?;
    let output = wait_with_timeout(child, "venv python", VERSION_COMMAND_TIMEOUT).ok()?;
    if !output.status.success() {
        return None;
    }
    // Python < 3.4 prints `--version` to stderr.
    let stdout = String::from_utf8_lossy(&output.stdout);
    PythonDetector
        .parse_version(&stdout)
        .or_else(|| PythonDetector.parse_version(&String::from_utf8_lossy(&output.stderr)))
}

/// Execute a version command through mise for project-local tool versions.
///
/// # Errors
/// If mise is installed but the command cannot be spawned.
fn execute_mise_version_command(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<Option<Output>> {
    let Some(path) = cwd else {
        return Ok(None);
    };
    if !has_tool_version_config(path) {
        return Ok(None);
    }

    let mut command = Command::new("mise");
    command.args(["exec", "--", program]);
    command.args(args);
    command.current_dir(path);
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::process::spawn_in_own_process_group(&mut command);

    let child = match command.spawn() {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::language(format!("Failed to execute mise: {e}"))),
    };

    wait_with_timeout(child, "mise", VERSION_COMMAND_TIMEOUT).map(Some)
}

/// Hard timeout for version subprocesses, so a hung shim (network mise, stale
/// NFS) can't wedge a `spawn_blocking` worker.
const VERSION_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Wait for a version command, labelling errors with `label`.
///
/// # Errors
/// If the child's I/O fails or the timeout expires.
fn wait_with_timeout(child: Child, label: &str, timeout: Duration) -> Result<Output> {
    crate::process::wait_with_timeout(child, timeout).map_err(|e| match e {
        crate::process::WaitError::Io(io_err) => {
            Error::language(format!("{label} I/O error: {io_err}"))
        }
        crate::process::WaitError::TimedOut(t) => {
            Error::language(format!("{label} version command timed out after {t:?}"))
        }
        crate::process::WaitError::WorkerDisconnected => Error::language(format!(
            "{label} version command worker exited without a result"
        )),
    })
}

/// Whether a `.tool-versions`/`mise.toml`/`.mise.toml` governs `path`.
/// Memoized per directory; the walk stops at `$HOME` or
/// [`MAX_TOOL_VERSION_ANCESTORS`] levels.
fn has_tool_version_config(path: &Path) -> bool {
    let cache_key = canonical_dir_key(path);
    if let Some(cached) = get_cached_tool_config(&cache_key) {
        return cached;
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let present = compute_has_tool_version_config(path, home.as_deref());
    cache_tool_config_result(cache_key, present);
    present
}

/// Ancestor-walk depth bound outside `$HOME` (guards pathological paths).
pub(super) const MAX_TOOL_VERSION_ANCESTORS: usize = 64;

/// Bounded ancestor walk backing [`has_tool_version_config`].
fn compute_has_tool_version_config(path: &Path, home: Option<&Path>) -> bool {
    let mut checked = 0_usize;
    for ancestor in path.ancestors() {
        if ancestor.join(".tool-versions").exists()
            || ancestor.join("mise.toml").exists()
            || ancestor.join(".mise.toml").exists()
        {
            return true;
        }

        if home.is_some_and(|home_dir| ancestor == home_dir) {
            break;
        }

        checked = checked.saturating_add(1);
        if checked >= MAX_TOOL_VERSION_ANCESTORS {
            break;
        }
    }
    false
}

/// Nearest `.ruby-version`, with [`compute_has_tool_version_config`]'s bounds.
fn find_ruby_version_file(path: &Path, home: Option<&Path>) -> Option<PathBuf> {
    let mut checked = 0_usize;
    for ancestor in path.ancestors() {
        let candidate = ancestor.join(".ruby-version");
        if candidate.is_file() {
            return Some(candidate);
        }

        if home.is_some_and(|home_dir| ancestor == home_dir) {
            break;
        }

        checked = checked.saturating_add(1);
        if checked >= MAX_TOOL_VERSION_ANCESTORS {
            break;
        }
    }
    None
}

/// Read the project's `.ruby-version` pin when no mise/tool-versions config
/// governs `cwd`. mise often ignores idiomatic version files for ruby, so
/// `mise exec -- ruby --version` would report the global Ruby.
pub(crate) fn resolve_ruby_version_file(cwd: &Path) -> Option<String> {
    if has_tool_version_config(cwd) {
        return None;
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = find_ruby_version_file(cwd, home.as_deref())?;
    let contents = crate::fs_util::read_small_file(&path, RUBY_VERSION_READ_CAP).ok()?;
    parse_ruby_version(&contents)
}

/// Cap on `.ruby-version` reads, bounding a pathologically huge file.
const RUBY_VERSION_READ_CAP: usize = 4_096;

/// Parse `.ruby-version`: first non-blank, non-`#` line, minus a `ruby-`
/// prefix and RVM `@gemset` (`ruby-3.2.2@app` → `3.2.2`).
#[must_use]
fn parse_ruby_version(contents: &str) -> Option<String> {
    let first = contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))?;
    let unprefixed = first.strip_prefix("ruby-").unwrap_or(first);
    let (version, _gemset) = unprefixed.split_once('@').unwrap_or((unprefixed, ""));
    (!version.trim().is_empty()).then(|| version.trim().to_owned())
}

fn parse_version_output<P: ReleaseSource + ?Sized>(
    detector: &P,
    output: &Output,
) -> Option<String> {
    let text = if detector.uses_stderr() {
        String::from_utf8_lossy(&output.stderr)
    } else {
        String::from_utf8_lossy(&output.stdout)
    };

    detector.parse_version(&text)
}

/// Record `version` for `cache_key` in the shared version cache.
fn cache_version_result(cache_key: &str, version: Option<String>) {
    VERSION_CACHE.insert(cache_key.to_owned(), version);
}

/// Cached version detection without a working directory.
#[must_use]
pub fn detect_language_release<P: ReleaseSource + ?Sized>(detector: &P) -> Option<String> {
    detect_language_release_at(detector, None)
}

/// Cached version detection in a project directory.
///
/// The cwd is part of the cache key since version managers resolve per
/// project. Failures are cached for [`VERSION_FAILURE_TTL`] so a broken
/// tool costs one probe round per window.
#[must_use]
pub fn detect_language_release_at<P: ReleaseSource + ?Sized>(
    detector: &P,
    cwd: Option<&Path>,
) -> Option<String> {
    let cache_key = version_cache_key(detector.language_name(), cwd);

    if let cache::CacheLookup::Hit(cached_result) = get_cached_version(&cache_key) {
        return cached_result;
    }
    if has_recent_failure(&cache_key) {
        return None;
    }

    if let Ok(Some(version)) = execute_version_command(detector, cwd) {
        VERSION_FAILURE_CACHE.remove(&cache_key);
        cache_version_result(&cache_key, Some(version.clone()));
        Some(version)
    } else {
        VERSION_FAILURE_CACHE.insert(cache_key, SystemTime::now());
        None
    }
}

/// Whether a probe for `cache_key` failed within [`VERSION_FAILURE_TTL`];
/// a backwards clock counts as not recent.
fn has_recent_failure(cache_key: &str) -> bool {
    VERSION_FAILURE_CACHE
        .get(cache_key)
        .is_some_and(|recorded_at| {
            SystemTime::now()
                .duration_since(recorded_at)
                .is_ok_and(|elapsed| elapsed < VERSION_FAILURE_TTL)
        })
}

/// Node.js version detector
pub struct NodeDetector;
impl ReleaseSource for NodeDetector {
    fn language_name(&self) -> &'static str {
        "node"
    }
    fn version_command(&self) -> &[&str] {
        &["node", "--version"]
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        Some(output.trim().trim_start_matches('v').to_owned())
    }
}

/// Python version detector
pub struct PythonDetector;
impl ReleaseSource for PythonDetector {
    fn language_name(&self) -> &'static str {
        "python"
    }
    fn version_command(&self) -> &[&str] {
        &["python", "--version"]
    }
    /// `python` first so shims and `.python-version` win; `python3` for hosts
    /// without an unversioned `python`.
    fn fallback_programs(&self) -> &[&str] {
        &["python3"]
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        output
            .split_whitespace()
            .nth(1)
            .map(std::borrow::ToOwned::to_owned)
    }
}

/// Rust version detector
pub struct RustDetector;
impl ReleaseSource for RustDetector {
    fn language_name(&self) -> &'static str {
        "rust"
    }
    fn version_command(&self) -> &[&str] {
        &["rustc", "--version"]
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        output
            .split_whitespace()
            .nth(1)
            .map(std::borrow::ToOwned::to_owned)
    }
}

/// Go version detector
pub struct GoDetector;
impl ReleaseSource for GoDetector {
    fn language_name(&self) -> &'static str {
        "go"
    }
    fn version_command(&self) -> &[&str] {
        &["go", "version"]
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        output
            .split_whitespace()
            .nth(2)
            .and_then(|v| v.strip_prefix("go"))
            .map(std::borrow::ToOwned::to_owned)
    }
}

/// Swift version detector
pub struct SwiftDetector;
impl ReleaseSource for SwiftDetector {
    fn language_name(&self) -> &'static str {
        "swift"
    }
    fn version_command(&self) -> &[&str] {
        &["swift", "--version"]
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        output
            .find("Swift version ")
            .and_then(|pos| output.get(pos..))
            .and_then(|tail| tail.strip_prefix("Swift version "))
            .and_then(|section| section.split_whitespace().next())
            .map(std::borrow::ToOwned::to_owned)
    }
}

/// Elixir version detector
pub struct ElixirDetector;
impl ReleaseSource for ElixirDetector {
    fn language_name(&self) -> &'static str {
        "elixir"
    }
    fn version_command(&self) -> &[&str] {
        &["elixir", "--version"]
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        output
            .find("Elixir ")
            .and_then(|pos| output.get(pos..))
            .and_then(|tail| tail.strip_prefix("Elixir "))
            .and_then(|section| section.split_whitespace().next())
            .map(std::borrow::ToOwned::to_owned)
    }
}

/// Erlang version detector
pub struct ErlangDetector;
impl ReleaseSource for ErlangDetector {
    fn language_name(&self) -> &'static str {
        "erlang"
    }
    fn version_command(&self) -> &[&str] {
        &[
            "erl",
            "-eval",
            "io:format(\"~s~n\", [erlang:system_info(otp_release)]), halt().",
            "-noshell",
        ]
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        Some(output.trim().to_owned())
    }
}

/// Ruby version detector
pub struct RubyDetector;
impl ReleaseSource for RubyDetector {
    fn language_name(&self) -> &'static str {
        "ruby"
    }
    fn version_command(&self) -> &[&str] {
        &["ruby", "--version"]
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        output
            .split_whitespace()
            .nth(1)
            .map(std::borrow::ToOwned::to_owned)
    }
}

/// Java version detector
pub struct JavaDetector;
impl ReleaseSource for JavaDetector {
    fn language_name(&self) -> &'static str {
        "java"
    }
    fn version_command(&self) -> &[&str] {
        &["java", "-version"]
    }
    fn uses_stderr(&self) -> bool {
        true
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        let start_index = output.find('"')?;
        let remainder = start_index
            .checked_add(1)
            .and_then(|idx| output.get(idx..))?;
        remainder
            .find('"')
            .and_then(|end_index| remainder.get(..end_index))
            .map(std::borrow::ToOwned::to_owned)
    }
}

/// Fish shell version detector
pub struct FishDetector;
impl ReleaseSource for FishDetector {
    fn language_name(&self) -> &'static str {
        "fish"
    }
    fn version_command(&self) -> &[&str] {
        &["fish", "--version"]
    }
    fn parse_version(&self, output: &str) -> Option<String> {
        output
            .split_whitespace()
            .nth(2)
            .map(std::borrow::ToOwned::to_owned)
    }
}

use std::sync::OnceLock;
use std::time::{Duration, SystemTime};

/// Registered version detectors, built once.
static VERSION_DETECTORS: OnceLock<Vec<Box<dyn ReleaseSource + Sync + Send>>> = OnceLock::new();

/// Successful probe results; shares [`TtlMap`]'s TTL knob.
static VERSION_CACHE: TtlMap<String, Option<String>> = TtlMap::new(VERSION_CACHE_CAPACITY);

/// How long a failed probe suppresses re-probing: avoids respawning a broken
/// tool every render while picking up a fixed one within a minute.
pub(crate) const VERSION_FAILURE_TTL: Duration = Duration::from_secs(60);

/// Failed probe timestamps, keyed like [`VERSION_CACHE`];
/// [`has_recent_failure`] applies the shorter [`VERSION_FAILURE_TTL`].
static VERSION_FAILURE_CACHE: TtlMap<String, SystemTime> = TtlMap::new(VERSION_CACHE_CAPACITY);

fn version_cache_key(language_name: &str, cwd: Option<&Path>) -> String {
    let Some(path) = cwd else {
        return language_name.to_owned();
    };

    let scoped_path = version_cache_path_key(path);
    format!("{language_name}:{scoped_path}")
}

fn version_cache_path_key(path: &Path) -> String {
    path.canonicalize()
        .unwrap_or_else(|_| path.to_path_buf())
        .to_string_lossy()
        .into_owned()
}

/// Canonicalized directory key shared by the mise-walk and negative-mise caches.
fn canonical_dir_key(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Maximum directories tracked by the mise `.tool-versions` presence cache.
const TOOL_CONFIG_CACHE_CAPACITY: usize = 512;

/// Maximum `(directory, program)` pairs tracked by the negative-mise cache.
const MISE_NEGATIVE_CACHE_CAPACITY: usize = 1024;

/// Memoizes the mise ancestor walk per canonical directory.
static TOOL_CONFIG_CACHE: TtlMap<PathBuf, bool> = TtlMap::new(TOOL_CONFIG_CACHE_CAPACITY);

/// `(directory, program)` pairs where a completed `mise exec` failed.
static MISE_NEGATIVE_CACHE: TtlMap<(PathBuf, String), ()> =
    TtlMap::new(MISE_NEGATIVE_CACHE_CAPACITY);

/// Fetch a memoized tool-config presence result, dropping it if the TTL lapsed.
fn get_cached_tool_config(cache_key: &Path) -> Option<bool> {
    TOOL_CONFIG_CACHE.get(cache_key)
}

/// Store a tool-config presence result, bounding the cache by capacity.
fn cache_tool_config_result(cache_key: PathBuf, present: bool) {
    TOOL_CONFIG_CACHE.insert(cache_key, present);
}

/// Whether `mise exec` recently failed for `program` in `cwd`.
fn mise_known_negative(program: &str, cwd: Option<&Path>) -> bool {
    let Some(path) = cwd else {
        return false;
    };
    let cache_key = (canonical_dir_key(path), program.to_owned());
    MISE_NEGATIVE_CACHE.get(&cache_key).is_some()
}

/// Record that `mise exec` definitively failed for `program` in `cwd`.
fn record_mise_negative(program: &str, cwd: Option<&Path>) {
    let Some(path) = cwd else {
        return;
    };
    let cache_key = (canonical_dir_key(path), program.to_owned());
    MISE_NEGATIVE_CACHE.insert(cache_key, ());
}

/// Clear cached versions (successes and failures) for a project directory.
pub(crate) fn invalidate_language_release_cache_at(cwd: &Path) {
    let scoped_path = version_cache_path_key(cwd);
    let keep_other_dirs = |cache_key: &String| {
        cache_key
            .split_once(':')
            .is_none_or(|(_language_name, cached_path)| cached_path != scoped_path)
    };
    VERSION_CACHE.retain(keep_other_dirs);
    VERSION_FAILURE_CACHE.retain(keep_other_dirs);

    // Also drop mise memoizations so a new `.tool-versions` is seen immediately.
    let canonical = canonical_dir_key(cwd);
    TOOL_CONFIG_CACHE.remove(&canonical);
    MISE_NEGATIVE_CACHE.retain(|(cached_dir, _program)| cached_dir != &canonical);
}

/// Set the shared cache TTL in hours (version, mise, and venv-stash caches).
pub(crate) fn set_version_cache_ttl(hours: u64) {
    crate::cache::ttl_map::set_shared_cache_ttl(hours);
}

#[cfg(test)]
#[must_use]
pub(crate) fn version_cache_ttl_seconds_for_tests() -> u64 {
    crate::cache::ttl_map::shared_cache_ttl_seconds()
}

/// Whether a version detector exists for `language` (case-insensitive).
#[must_use]
pub fn has_version_detector(language: &str) -> bool {
    get_version_detectors()
        .iter()
        .any(|probe| probe.language_name().eq_ignore_ascii_case(language))
}

/// Get all registered version detectors.
pub fn get_version_detectors() -> &'static Vec<Box<dyn ReleaseSource + Sync + Send>> {
    VERSION_DETECTORS.get_or_init(|| {
        vec![
            Box::new(NodeDetector),
            Box::new(PythonDetector),
            Box::new(RustDetector),
            Box::new(GoDetector),
            Box::new(SwiftDetector),
            Box::new(ElixirDetector),
            Box::new(ErlangDetector),
            Box::new(RubyDetector),
            Box::new(JavaDetector),
            Box::new(FishDetector),
        ]
    })
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::*;

    #[test]
    fn parse_ruby_version_table() {
        let cases: &[(&str, Option<&str>)] = &[
            ("3.2.4", Some("3.2.4")),
            ("3.2.4\n", Some("3.2.4")),
            ("  3.2.4  \n", Some("3.2.4")),
            ("# comment\n3.2.4\n", Some("3.2.4")),
            ("\n\n3.2.4\n", Some("3.2.4")),
            ("# only comments\n#more\n", None),
            ("\n\n\n", None),
            ("", None),
            ("ruby-3.2.2\n", Some("3.2.2")),
            ("ruby-3.2.2@app\n", Some("3.2.2")),
            ("3.3.0@app", Some("3.3.0")),
            ("jruby-9.4.5.0", Some("jruby-9.4.5.0")),
            ("ruby-", None),
            ("@app", None),
        ];
        for (input, expected) in cases {
            assert_eq!(
                parse_ruby_version(input).as_deref(),
                *expected,
                "input: {input:?}"
            );
        }
    }

    struct CwdVersionFileDetector;
    impl ReleaseSource for CwdVersionFileDetector {
        fn language_name(&self) -> &'static str {
            "python-test"
        }

        fn version_command(&self) -> &[&str] {
            #[cfg(unix)]
            {
                &["/bin/sh", "-c", "cat .python-version"]
            }
            #[cfg(not(unix))]
            {
                &["cmd", "/c", "type .python-version"]
            }
        }

        fn parse_version(&self, output: &str) -> Option<String> {
            Some(output.trim().to_owned())
        }
    }

    #[test]
    #[serial_test::serial(global_ttl)]
    fn cache_lookup_respects_ttl_setting() {
        set_version_cache_ttl(1);

        VERSION_CACHE.clear();
        VERSION_CACHE.insert("test-lang".to_owned(), Some("1.0.0".to_owned()));

        match get_cached_version("test-lang") {
            cache::CacheLookup::Hit(Some(value)) => assert_eq!(value, "1.0.0"),
            cache::CacheLookup::Hit(None) => panic!("expected cached value, got None"),
            cache::CacheLookup::Miss => panic!("cache should have contained entry"),
        }

        VERSION_CACHE.insert_at(
            "test-lang".to_owned(),
            Some("1.0.0".to_owned()),
            SystemTime::now() - Duration::from_secs(3600 + 5),
        );

        match get_cached_version("test-lang") {
            cache::CacheLookup::Miss => {}
            cache::CacheLookup::Hit(result) => {
                panic!("expected cache miss after expiration, got hit: {result:?}")
            }
        }

        set_version_cache_ttl(24);
    }

    #[test]
    #[serial_test::serial(global_ttl)]
    fn path_scoped_version_detection_respects_project_files() {
        let first = tempfile::tempdir().expect("create first temp dir");
        let second = tempfile::tempdir().expect("create second temp dir");
        std::fs::write(first.path().join(".python-version"), "3.9.24\n")
            .expect("write first version");
        std::fs::write(second.path().join(".python-version"), "3.14.4\n")
            .expect("write second version");

        VERSION_CACHE.clear();

        let detector = CwdVersionFileDetector;
        let first_version = detect_language_release_at(&detector, Some(first.path()));
        let second_version = detect_language_release_at(&detector, Some(second.path()));

        assert_eq!(first_version.as_deref(), Some("3.9.24"));
        assert_eq!(second_version.as_deref(), Some("3.14.4"));

        set_version_cache_ttl(24);
    }

    #[test]
    fn invalidating_project_version_cache_keeps_other_projects() {
        let first = tempfile::tempdir().expect("create first temp dir");
        let second = tempfile::tempdir().expect("create second temp dir");

        let first_key = version_cache_key("ruby", Some(first.path()));
        let second_key = version_cache_key("ruby", Some(second.path()));

        VERSION_CACHE.clear();
        VERSION_CACHE.insert(first_key.clone(), Some("3.3.11".to_owned()));
        VERSION_CACHE.insert(second_key.clone(), Some("4.0.5".to_owned()));

        invalidate_language_release_cache_at(first.path());

        assert_eq!(
            VERSION_CACHE.get(&first_key),
            None,
            "invalidated project's entry should be gone"
        );
        assert_eq!(
            VERSION_CACHE.get(&second_key).flatten().as_deref(),
            Some("4.0.5"),
            "other project's entry should survive invalidation"
        );
    }

    #[test]
    #[serial_test::serial(global_ttl)]
    fn version_cache_is_bounded_across_many_projects() {
        VERSION_CACHE.clear();

        let visits = VERSION_CACHE_CAPACITY.saturating_mul(2);
        for i in 0..visits {
            cache_version_result(&format!("ruby:/project/{i}"), Some("3.3.0".to_owned()));
        }

        let len = VERSION_CACHE.len();
        assert!(
            len <= VERSION_CACHE_CAPACITY,
            "version cache grew to {len} entries, exceeding cap {VERSION_CACHE_CAPACITY}"
        );

        VERSION_CACHE.clear();
    }

    #[test]
    fn tool_version_config_detects_ancestor_files() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let project = temp_dir.path().join("project");
        let nested = project.join("nested");
        std::fs::create_dir_all(&nested).expect("create nested dir");
        std::fs::write(project.join(".tool-versions"), "ruby 3.3\n").expect("write tool versions");

        assert!(has_tool_version_config(&nested));
        assert!(!has_tool_version_config(temp_dir.path()));
    }

    #[test]
    fn python_detector_uses_python_command() {
        let detector = PythonDetector;
        assert_eq!(detector.version_command(), ["python", "--version"]);
    }

    /// Stub detector with a custom command and fallbacks; parses the 2nd token.
    #[cfg(unix)]
    struct StubDetector<'a> {
        name: &'static str,
        command: Vec<&'a str>,
        fallbacks: Vec<&'a str>,
    }

    #[cfg(unix)]
    impl ReleaseSource for StubDetector<'_> {
        fn language_name(&self) -> &'static str {
            self.name
        }

        fn version_command(&self) -> &[&str] {
            &self.command
        }

        fn fallback_programs(&self) -> &[&str] {
            &self.fallbacks
        }

        fn parse_version(&self, output: &str) -> Option<String> {
            output
                .split_whitespace()
                .nth(1)
                .map(std::borrow::ToOwned::to_owned)
        }
    }

    /// Write an executable `/bin/sh` script at `path` with `body`.
    #[cfg(unix)]
    fn write_script(path: &Path, body: &str) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::write(path, format!("#!/bin/sh\n{body}\n")).expect("write script");
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))
            .expect("chmod script");
    }

    #[test]
    #[cfg(unix)]
    fn fallback_program_used_when_primary_not_found() {
        let stub = StubDetector {
            name: "fallback-test",
            command: vec!["gpy-test-no-such-binary-7f3a", "-c", "echo Python 3.11.9"],
            fallbacks: vec!["sh"],
        };

        let version = execute_version_command(&stub, None).expect("fallback should run");
        assert_eq!(version.as_deref(), Some("3.11.9"));
    }

    #[test]
    #[cfg(unix)]
    fn fallback_not_used_on_nonzero_exit() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let marker = temp_dir.path().join("fallback-ran");
        let fallback = temp_dir.path().join("fallback.sh");
        write_script(
            &fallback,
            &format!("echo ran >> '{}'\necho Python 9.9.9", marker.display()),
        );
        let fallback_str = fallback.to_str().expect("utf-8 temp path");
        let stub = StubDetector {
            name: "fallback-test",
            command: vec!["sh", "-c", "exit 1"],
            fallbacks: vec![fallback_str],
        };

        let version = execute_version_command(&stub, None).expect("non-zero exit is Ok(None)");
        assert_eq!(version, None);
        assert!(
            !marker.exists(),
            "fallback must not run after a non-zero exit"
        );
    }

    #[test]
    #[cfg(unix)]
    fn fallback_not_used_when_primary_succeeds() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let marker = temp_dir.path().join("fallback-ran");
        let fallback = temp_dir.path().join("fallback.sh");
        write_script(
            &fallback,
            &format!("echo ran >> '{}'\necho Python 9.9.9", marker.display()),
        );
        let fallback_str = fallback.to_str().expect("utf-8 temp path");
        let stub = StubDetector {
            name: "fallback-test",
            command: vec!["sh", "-c", "echo Python 3.12.1"],
            fallbacks: vec![fallback_str],
        };

        let version = execute_version_command(&stub, None).expect("primary should run");
        assert_eq!(version.as_deref(), Some("3.12.1"));
        assert!(
            !marker.exists(),
            "fallback must not run when primary exists"
        );
    }

    #[test]
    fn python_detector_falls_back_to_python3() {
        assert_eq!(PythonDetector.fallback_programs(), ["python3"]);
    }

    #[test]
    fn other_detectors_have_no_fallback_programs() {
        for detector in get_version_detectors() {
            if detector.language_name() != "python" {
                assert!(
                    detector.fallback_programs().is_empty(),
                    "{} unexpectedly declares fallbacks",
                    detector.language_name()
                );
            }
        }
    }

    /// Lines appended to `path` so far (0 when it does not exist).
    #[cfg(unix)]
    fn spawn_count(path: &Path) -> usize {
        std::fs::read_to_string(path).map_or(0, |contents| contents.lines().count())
    }

    #[test]
    #[cfg(unix)]
    #[serial_test::serial(global_ttl)]
    fn probe_error_is_negatively_cached() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let dir = temp_dir.path();
        let count = dir.join("count");
        let script = dir.join("probe.sh");
        // Non-executable: spawn fails like a timeout, without the 5s wait.
        std::fs::write(&script, "").expect("write non-executable probe");
        let script_str = script.to_str().expect("utf-8 temp path");
        let stub = StubDetector {
            name: "probe-error-test",
            command: vec![script_str],
            fallbacks: vec![],
        };

        assert_eq!(detect_language_release_at(&stub, Some(dir)), None);

        write_script(
            &script,
            &format!("echo spawn >> '{}'\necho v 1.2.3", count.display()),
        );
        assert_eq!(detect_language_release_at(&stub, Some(dir)), None);
        assert!(
            !count.exists(),
            "a recent probe failure must suppress re-spawning"
        );
    }

    #[test]
    #[cfg(unix)]
    #[serial_test::serial(global_ttl)]
    fn nonzero_exit_retries_after_failure_ttl() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let dir = temp_dir.path();
        let count = dir.join("count");
        let stub = StubDetector {
            name: "nonzero-exit-test",
            command: vec![
                "sh",
                "-c",
                "echo spawn >> count; test -f ok && echo v 1.2.3 || exit 1",
            ],
            fallbacks: vec![],
        };
        let key = version_cache_key(stub.name, Some(dir));

        assert_eq!(detect_language_release_at(&stub, Some(dir)), None);
        assert_eq!(spawn_count(&count), 1);

        std::fs::write(dir.join("ok"), "").expect("mark tool installed");
        assert_eq!(detect_language_release_at(&stub, Some(dir)), None);
        assert_eq!(spawn_count(&count), 1, "no re-probe within the failure TTL");

        VERSION_FAILURE_CACHE.insert(
            key.clone(),
            SystemTime::now() - VERSION_FAILURE_TTL - Duration::from_secs(1),
        );
        assert_eq!(
            detect_language_release_at(&stub, Some(dir)).as_deref(),
            Some("1.2.3")
        );
        assert_eq!(spawn_count(&count), 2);
        assert_eq!(VERSION_FAILURE_CACHE.get(&key), None);

        assert_eq!(
            detect_language_release_at(&stub, Some(dir)).as_deref(),
            Some("1.2.3")
        );
        assert_eq!(spawn_count(&count), 2, "successes stay cached");
    }

    #[test]
    #[cfg(unix)]
    #[serial_test::serial(global_ttl)]
    fn invalidate_clears_failure_entries() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let dir = temp_dir.path();
        let count = dir.join("count");
        let stub = StubDetector {
            name: "invalidate-failure-test",
            command: vec!["sh", "-c", "echo spawn >> count; exit 1"],
            fallbacks: vec![],
        };

        assert_eq!(detect_language_release_at(&stub, Some(dir)), None);
        assert_eq!(spawn_count(&count), 1);
        assert_eq!(detect_language_release_at(&stub, Some(dir)), None);
        assert_eq!(spawn_count(&count), 1, "failure recorded");

        invalidate_language_release_cache_at(dir);
        assert_eq!(detect_language_release_at(&stub, Some(dir)), None);
        assert_eq!(spawn_count(&count), 2, "invalidation must force a re-probe");
    }

    #[test]
    fn ruby_version_file_used_when_no_tool_version_config_present() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(temp_dir.path().join(".ruby-version"), "3.3.11\n")
            .expect("write .ruby-version");

        assert_eq!(
            resolve_ruby_version_file(temp_dir.path()).as_deref(),
            Some("3.3.11")
        );
    }

    #[test]
    fn ruby_version_file_found_in_ancestor_directory() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        let nested = temp_dir.path().join("nested");
        std::fs::create_dir_all(&nested).expect("create nested dir");
        std::fs::write(temp_dir.path().join(".ruby-version"), "3.2.4\n")
            .expect("write .ruby-version");

        assert_eq!(resolve_ruby_version_file(&nested).as_deref(), Some("3.2.4"));
    }

    #[test]
    fn ruby_version_file_defers_when_tool_version_config_present() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(temp_dir.path().join(".ruby-version"), "3.3.11\n")
            .expect("write .ruby-version");
        std::fs::write(
            temp_dir.path().join("mise.toml"),
            "[tools]\nruby = \"3.3\"\n",
        )
        .expect("write mise.toml");

        assert_eq!(resolve_ruby_version_file(temp_dir.path()), None);
    }

    #[test]
    fn missing_ruby_version_file_returns_none() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        assert_eq!(resolve_ruby_version_file(temp_dir.path()), None);
    }

    #[test]
    fn ruby_version_file_trims_whitespace_and_skips_comment_lines() {
        let temp_dir = tempfile::tempdir().expect("create temp dir");
        std::fs::write(
            temp_dir.path().join(".ruby-version"),
            "# managed by rbenv\n  3.3.11  \n",
        )
        .expect("write .ruby-version");

        assert_eq!(
            resolve_ruby_version_file(temp_dir.path()).as_deref(),
            Some("3.3.11")
        );
    }
}

//! Extensible version detection system
//!
//! Trait-based architecture for detecting language versions with easy
//! extensibility for adding new languages like PHP.

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
}

/// Look up `cache_key` in the shared version cache, evicting it if stale.
fn get_cached_version(cache_key: &str) -> cache::CacheLookup {
    VERSION_CACHE
        .get(cache_key)
        .map_or(cache::CacheLookup::Miss, cache::CacheLookup::Hit)
}

/// Execute version command and parse output (I/O operation)
///
/// `pub` so Criterion benches (which compile as separate crates and cannot
/// see `#[cfg(test)]` helpers) can measure our spawn/read/timeout/parse
/// overhead directly with a stub [`ReleaseSource`], bypassing the version
/// cache in [`detect_language_release_at`] entirely.
///
/// # Errors
///
/// Returns an error if the version command cannot be executed successfully.
pub fn execute_version_command<P: ReleaseSource + ?Sized>(
    detector: &P,
    cwd: Option<&Path>,
) -> Result<Option<String>> {
    let command_args = detector.version_command();
    let Some((program, args)) = command_args.split_first() else {
        return Ok(None);
    };

    // Skip the mise attempt entirely when a prior run proved mise does not
    // resolve this program in this directory: that spawn is guaranteed wasted.
    // A mise that timed out never lands here (the `?` below propagates its error
    // before any negative caching), so a working-but-slow mise is never poisoned.
    if !mise_known_negative(program, cwd)
        && let Some(output) = execute_mise_version_command(program, args, cwd)?
    {
        if output.status.success() {
            return Ok(parse_version_output(detector, &output));
        }
        // mise ran to completion and definitively failed for this program in this
        // directory (the same fall-through condition as before). Remember it so
        // later misses skip straight to direct execution.
        record_mise_negative(program, cwd);
    }

    let output = execute_direct_version_command(program, args, cwd)?;

    if !output.status.success() {
        return Ok(None);
    }

    Ok(parse_version_output(detector, &output))
}

/// Execute a version command directly in the requested working directory.
///
/// # Errors
///
/// Returns an error when the detector command cannot be spawned.
fn execute_direct_version_command(
    program: &str,
    args: &[&str],
    cwd: Option<&Path>,
) -> Result<Output> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(path) = cwd {
        command.current_dir(path);
    }
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    crate::process::spawn_in_own_process_group(&mut command);

    let child = command
        .spawn()
        .map_err(|e| Error::language(format!("Failed to execute {program}: {e}")))?;

    wait_with_timeout(child, program, VERSION_COMMAND_TIMEOUT)
}

/// Detect the version reported by a specific Python interpreter binary.
///
/// Used for venv interpreters that lack a readable `pyvenv.cfg`. Reuses the
/// same spawn/timeout/process-group machinery as the generic detectors, then
/// parses with [`PythonDetector`]'s rule. Returns `None` on spawn failure,
/// timeout, non-zero exit, or unparseable output.
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
    // Python < 3.4 prints `--version` to stderr; modern Python uses stdout.
    // Try stdout, fall back to stderr, mirroring PythonDetector's parse rule.
    let stdout = String::from_utf8_lossy(&output.stdout);
    PythonDetector
        .parse_version(&stdout)
        .or_else(|| PythonDetector.parse_version(&String::from_utf8_lossy(&output.stderr)))
}

/// Execute a version command through mise for project-local tool versions.
///
/// # Errors
///
/// Returns an error when mise is installed but the command cannot be spawned.
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
        // mise is not installed; fall back to direct detection rather than erroring.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::language(format!("Failed to execute mise: {e}"))),
    };

    wait_with_timeout(child, "mise", VERSION_COMMAND_TIMEOUT).map(Some)
}

/// Hard timeout for language version detection subprocesses.
///
/// Version commands (`node --version`, `mise exec`, etc.) normally return in
/// well under a second. A hung tool shim — mise resolving over the network, a
/// wrapper script, or an interpreter blocked on a stale NFS mount — must never
/// be allowed to wedge a `spawn_blocking` worker indefinitely.
const VERSION_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// Wait for a spawned version-detection command.
///
/// Maps the shared [`crate::process::WaitError`] onto this module's
/// `Error::language`, with `label` naming the command in the message (e.g.
/// "node", "mise", "venv python") -- the shared
/// [`crate::process::wait_with_timeout`] primitive is domain-agnostic, so
/// per-caller labeling happens here rather than there (#590). Preserves the
/// exact error wording each of this file's call sites depended on before the
/// consolidation.
///
/// # Errors
///
/// Returns an error if the child's I/O fails or the timeout expires.
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

/// Whether a `.tool-versions`/`mise.toml`/`.mise.toml` file governs `path`.
///
/// Memoized per canonical directory (24h TTL, shared with the version cache) so
/// the ancestor stat walk runs at most once per directory within the TTL. The
/// walk itself is bounded: it stops at `$HOME` (there is no point resolving tool
/// versions above the user's home), and otherwise after
/// [`MAX_TOOL_VERSION_ANCESTORS`] levels, so an unusual path outside `$HOME`
/// never fans the walk all the way to the filesystem root.
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

/// Upper bound on how many ancestor directories the mise walk inspects.
///
/// Applies when the path is not anchored under `$HOME`. Deep enough to cover
/// any realistic project nesting; a guard against pathological paths, not a
/// functional limit.
const MAX_TOOL_VERSION_ANCESTORS: usize = 64;

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

        // Stop once the current level is `$HOME` (already checked above): tool
        // versions are never resolved from above the user's home directory.
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

/// Bounded ancestor walk backing [`find_ruby_version_file`], mirroring
/// [`compute_has_tool_version_config`]'s stop conditions ($HOME, then
/// [`MAX_TOOL_VERSION_ANCESTORS`] levels).
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

/// Read a project's pinned Ruby version directly from `.ruby-version`.
///
/// mise only honors idiomatic version files (`.ruby-version`, `.node-version`,
/// etc.) for tools listed in the user's `idiomatic_version_file_enable_tools`
/// config, and `ruby` is commonly not opted in there. That leaves
/// `mise exec -- ruby --version` silently resolving the globally pinned Ruby
/// instead of the project's `.ruby-version` pin. When no
/// `.tool-versions`/`mise.toml`/`.mise.toml` already governs `cwd` (the same
/// condition [`execute_mise_version_command`] uses to skip mise entirely via
/// [`has_tool_version_config`]), read `.ruby-version` from disk directly —
/// mirroring how [`crate::language::venv`] reads `pyvenv.cfg` instead of
/// trusting the environment. Returns `None` when a tool-version config is
/// present (defer to the existing mise-based path) or no `.ruby-version` is
/// found.
pub(crate) fn resolve_ruby_version_file(cwd: &Path) -> Option<String> {
    if has_tool_version_config(cwd) {
        return None;
    }

    let home = std::env::var_os("HOME").map(PathBuf::from);
    let path = find_ruby_version_file(cwd, home.as_deref())?;
    let contents = crate::fs_util::read_small_file(&path, RUBY_VERSION_READ_CAP).ok()?;
    parse_ruby_version(&contents)
}

/// Cap on `.ruby-version` reads: a single-line-to-few-line file by
/// construction, so 4 `KiB` comfortably covers any legitimate file while
/// bounding a maliciously or accidentally huge one.
const RUBY_VERSION_READ_CAP: usize = 4_096;

/// Parse a `.ruby-version` file's contents: the first non-empty, non-comment
/// (`#`) line, trimmed. `None` if every line is blank or a comment.
#[must_use]
fn parse_ruby_version(contents: &str) -> Option<String> {
    contents
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty() && !line.starts_with('#'))
        .map(std::borrow::ToOwned::to_owned)
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

/// Record `version` for `cache_key` in the shared version cache, evicting
/// older entries past capacity.
fn cache_version_result(cache_key: &str, version: Option<String>) {
    VERSION_CACHE.insert(cache_key.to_owned(), version);
}

/// Execute version detection for any detector (with 24-hour cache)
///
/// # Errors
///
/// Returns an error if version detection fails or if caching fails.
pub fn detect_language_release<P: ReleaseSource + ?Sized>(detector: &P) -> Result<Option<String>> {
    detect_language_release_at(detector, None)
}

/// Execute version detection for any detector in a project directory.
///
/// The working directory is part of the cache key because version managers
/// such as pyenv, rbenv, mise, and asdf can resolve different tool versions
/// for different projects even when the executable path is identical.
///
/// # Errors
///
/// Returns an error if version detection fails or if caching fails.
pub fn detect_language_release_at<P: ReleaseSource + ?Sized>(
    detector: &P,
    cwd: Option<&Path>,
) -> Result<Option<String>> {
    let cache_key = version_cache_key(detector.language_name(), cwd);

    // Check cache first
    match get_cached_version(&cache_key) {
        cache::CacheLookup::Hit(cached_result) => Ok(cached_result),
        cache::CacheLookup::Miss => {
            // Execute version detection
            let version = execute_version_command(detector, cwd)?;

            // Cache the result (including failures)
            cache_version_result(&cache_key, version.clone());

            Ok(version)
        }
    }
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
        // Parse "Swift version X.Y.Z" or "Apple Swift version X.Y.Z"
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
        // Parse "Elixir X.Y.Z ..."
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
        // Parse "ruby X.Y.Z ..."
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
        // Parse "java version "X.Y.Z"" or similar
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
        // Parse "fish, version X.Y.Z"
        output
            .split_whitespace()
            .nth(2)
            .map(std::borrow::ToOwned::to_owned)
    }
}

use std::sync::OnceLock;
use std::time::Duration;

/// Lazy-initialized version detectors to avoid repeated allocations
static VERSION_DETECTORS: OnceLock<Vec<Box<dyn ReleaseSource + Sync + Send>>> = OnceLock::new();

/// Global version cache: 24h default TTL (configurable, shared with the
/// mise-walk and negative-mise caches below via [`TtlMap`]'s shared knob) to
/// avoid repeated external command execution.
static VERSION_CACHE: TtlMap<String, Option<String>> = TtlMap::new(VERSION_CACHE_CAPACITY);

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

/// Memoizes the (bounded) mise ancestor walk per canonical directory. Shares
/// [`VERSION_CACHE`]'s TTL knob via [`TtlMap`] — no separate freshness logic.
static TOOL_CONFIG_CACHE: TtlMap<PathBuf, bool> = TtlMap::new(TOOL_CONFIG_CACHE_CAPACITY);

/// Records `(directory, program)` pairs where mise is known not to resolve.
///
/// A completed `mise exec` proved mise does not resolve that program, so
/// future misses skip the wasted mise spawn. The value carries no payload
/// beyond "this key is a recorded negative" — `()` combined with
/// presence-and-freshness in [`TtlMap::get`] is all that's needed.
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

/// Whether a completed `mise exec` previously failed for `program` in `cwd`
/// (within the TTL), meaning the mise attempt can be skipped this time.
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

/// Clear cached language versions for a project directory.
pub(crate) fn invalidate_language_release_cache_at(cwd: &Path) {
    let scoped_path = version_cache_path_key(cwd);
    VERSION_CACHE.retain(|cache_key| {
        cache_key
            .split_once(':')
            .is_none_or(|(_language_name, cached_path)| cached_path != scoped_path)
    });

    // Drop the mise-walk and negative-mise memoizations for the same directory so
    // a project-file change (e.g. adding `.tool-versions`) is observed on the next
    // probe rather than after the shared TTL lapses.
    let canonical = canonical_dir_key(cwd);
    TOOL_CONFIG_CACHE.remove(&canonical);
    MISE_NEGATIVE_CACHE.retain(|(cached_dir, _program)| cached_dir != &canonical);
}

/// Update the shared cache TTL (in hours) used by the version cache and the
/// three caches sharing its TTL knob (tool-config, negative-mise, and the
/// venv stash in [`super::venv`]).
///
/// Kept under its original name at this original call path — 3 real
/// production call sites (`agent/oneshot.rs`, `agent/mod.rs`,
/// `agent/events.rs`) depend on `crate::language::version::set_version_cache_ttl`
/// continuing to work — even though the TTL it sets is now
/// [`crate::cache::ttl_map`]'s single shared knob rather than a field local
/// to this module (#589).
pub(crate) fn set_version_cache_ttl(hours: u64) {
    crate::cache::ttl_map::set_shared_cache_ttl(hours);
}

#[cfg(test)]
#[must_use]
pub(crate) fn version_cache_ttl_seconds_for_tests() -> u64 {
    crate::cache::ttl_map::shared_cache_ttl_seconds()
}

/// Get all available version detectors (lazy-initialized, cached)
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

// Design reference (see docs/ARCHITECTURE.md § Language Versions):
// - Detectors normalize outputs to X.Y.Z, cache for 24h, and fail soft when tools are missing
// - Extending to a new language requires only a new ReleaseSource implementation and registry entry

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::*;
    use std::time::SystemTime;

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
            // `/bin/sh` is Unix-only; Windows has no equivalent path.
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
        let first_version =
            detect_language_release_at(&detector, Some(first.path())).expect("detect first");
        let second_version =
            detect_language_release_at(&detector, Some(second.path())).expect("detect second");

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

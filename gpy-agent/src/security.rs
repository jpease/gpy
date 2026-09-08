//! Security validation and hardening utilities

#[cfg(unix)]
use crate::warn_log;
use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// A validated, security-checked filesystem path.
///
/// This type guarantees that the path has passed all security checks
/// defined in `PathValidator`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct SafePath(PathBuf);

impl SafePath {
    /// Create a new `SafePath` after validation.
    ///
    /// # Blocking
    /// Calls [`PathValidator::validate_path`], which does a blocking
    /// `canonicalize()` syscall. A hung filesystem (dead NFS/automount) blocks
    /// the calling thread indefinitely, so this — and any `SafePath`
    /// deserialization — must run on `tokio::task::spawn_blocking`, never
    /// directly on an async task (#314).
    ///
    /// # Errors
    /// Returns an error if the path fails security validation.
    pub fn new(path: &str) -> Result<Self> {
        let validated = PathValidator::validate_path(path)?;
        Ok(Self(validated))
    }

    /// Get the path as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.to_str().unwrap_or_default()
    }

    /// Get the inner `Path` reference.
    #[must_use]
    pub fn as_path(&self) -> &Path {
        self.0.as_path()
    }

    /// Convert into the inner `PathBuf`.
    #[must_use]
    pub fn into_inner(self) -> PathBuf {
        self.0
    }
}

impl AsRef<Path> for SafePath {
    fn as_ref(&self) -> &Path {
        self.0.as_path()
    }
}

impl std::ops::Deref for SafePath {
    type Target = Path;
    fn deref(&self) -> &Self::Target {
        self.0.as_path()
    }
}

impl<'de> Deserialize<'de> for SafePath {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Self::new(&s).map_err(serde::de::Error::custom)
    }
}

impl std::fmt::Display for SafePath {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0.display())
    }
}

impl PartialEq<&str> for SafePath {
    fn eq(&self, other: &&str) -> bool {
        self.0.to_string_lossy() == *other
    }
}

impl PartialEq<String> for SafePath {
    fn eq(&self, other: &String) -> bool {
        self.0.to_string_lossy() == *other
    }
}

impl PartialEq<SafePath> for &str {
    fn eq(&self, other: &SafePath) -> bool {
        other == self
    }
}

/// Maximum allowed path length in bytes
const MAX_PATH_LENGTH: usize = 4096;

/// Maximum message size in bytes (64KB)
///
/// The single definition of the wire-size cap (#578). `ipc::protocol`'s
/// `validate_message_size` used to declare its own local copy of this value, so
/// the two could drift and `GuardSettings::max_message_size` — which the
/// connection-level buffer guard does honour — could never be raised past the
/// protocol layer's hardcoded limit.
pub(crate) const MAX_MESSAGE_SIZE: usize = 64 * 1024;

/// Default request timeout in seconds
const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 5;

/// Default maximum connections per second for rate limiting
const DEFAULT_MAX_CONNECTIONS_PER_SEC: u32 = 1000;

/// Default maximum concurrent connections
const DEFAULT_MAX_CONCURRENT_CONNECTIONS: usize = 200;

/// Security configuration and limits
#[derive(Clone)]
pub struct GuardSettings {
    /// Maximum request timeout (5 seconds based on IPC latency budget)
    pub request_timeout: Duration,
    /// Maximum message size (64KB already implemented in protocol.rs)
    pub max_message_size: usize,
    /// Enable PID validation for register requests
    pub validate_pids: bool,
    /// Rate limiting: max connections per second
    pub max_connections_per_second: u32,
    /// Maximum concurrent connections to prevent task flood
    pub max_concurrent_connections: usize,
}

impl Default for GuardSettings {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
            max_message_size: MAX_MESSAGE_SIZE,
            validate_pids: true,
            max_connections_per_second: DEFAULT_MAX_CONNECTIONS_PER_SEC, // High limit for local IPC
            max_concurrent_connections: DEFAULT_MAX_CONCURRENT_CONNECTIONS, // Prevent unbounded task spawning
        }
    }
}

/// Reject a field whose value contains a NUL byte or any other control
/// character, rather than silently stripping it (#578).
///
/// A stripped value is a *different* value than the caller specified, with no
/// signal that anything changed — `/tmp/a\tb` silently becoming `/tmp/ab`
/// names another directory entirely. Control characters in a value that later
/// reaches the prompt can also line-break or otherwise corrupt the rendered
/// output (#323), which is why they must not survive validation.
///
/// This is the one control-character policy for inbound data. Path validation
/// ([`PathValidator::validate_path`]) and general IPC string-field validation
/// (`ipc::protocol::validate_string_field`) previously disagreed — the former
/// stripped every control character, the latter tolerated up to five
/// non-whitespace ones — and now share this single "reject any control
/// character, always" rule. The render-time stripping in `template::eval` is a
/// separate, deliberate defence: the prompt has to render something, so it
/// cannot reject.
///
/// # Errors
///
/// Returns an error if `value` contains a NUL byte or any other control
/// character (`char::is_control`: U+0000–U+001F, U+007F, and U+0080–U+009F).
pub(crate) fn reject_control_chars(field_name: &str, value: &str) -> Result<()> {
    if value.contains('\0') {
        return Err(Error::ipc(format!(
            "Field '{field_name}' contains a null byte"
        )));
    }

    if value.chars().any(char::is_control) {
        return Err(Error::ipc(format!(
            "Field '{field_name}' contains a control character"
        )));
    }

    Ok(())
}

/// Path security validation utilities
pub struct PathValidator;

impl PathValidator {
    /// Canonicalize and validate a path for security
    ///
    /// # Errors
    ///
    /// Returns an error if the path contains invalid characters, is too long,
    /// contains directory traversal attempts, or points to sensitive system files.
    pub fn validate_path(path: &str) -> Result<PathBuf> {
        // Reject NUL and other control characters rather than stripping them:
        // a stripped path is a different path than the caller asked for (#578).
        reject_control_chars("path", path)?;

        // Length cap (prevent extremely long paths)
        if path.len() > MAX_PATH_LENGTH {
            return Err(Error::ipc(format!(
                "Path too long (max {MAX_PATH_LENGTH} characters)"
            )));
        }

        let path_buf = PathBuf::from(path);

        // Canonicalize path (resolves symlinks, removes ..). Both branches run
        // the same component checks: a path that does not exist yet gets no
        // weaker treatment than one that does (#578).
        if let Ok(canonical) = path_buf.canonicalize() {
            Self::check_components(&canonical)?;
            Ok(canonical)
        } else {
            Self::check_components(&path_buf)?;
            Ok(path_buf)
        }
    }

    /// Denylisted system directories, in both their literal and canonicalized
    /// forms.
    ///
    /// The canonical form is required because `validate_path` canonicalizes an
    /// input that exists: comparing `/private/etc/hosts` against the raw `/etc`
    /// prefix would never match, silently making the denylist a no-op on a host
    /// where `/etc` is a symlink (macOS's `/etc` -> `/private/etc`) (#323). The
    /// literal form is required for the opposite case: a path that does *not*
    /// exist is never canonicalized, so `/etc/does-not-exist` only matches the
    /// unresolved `/etc` prefix (#578). The denied directories themselves are
    /// unchanged — `/etc`, `/proc`, `/sys`, `/dev` — this only keeps both
    /// spellings of each one.
    fn denylisted_system_prefixes() -> &'static [PathBuf] {
        static PREFIXES: OnceLock<Vec<PathBuf>> = OnceLock::new();
        PREFIXES.get_or_init(|| {
            let mut prefixes: Vec<PathBuf> = Vec::new();
            for raw in ["/etc", "/proc", "/sys", "/dev"] {
                let literal = PathBuf::from(raw);
                if let Ok(canonical) = std::fs::canonicalize(raw)
                    && canonical != literal
                {
                    prefixes.push(canonical);
                }
                prefixes.push(literal);
            }
            prefixes
        })
    }

    /// Check a path's components for parent-directory traversal and for
    /// denylisted system directories.
    ///
    /// Applied identically to both branches of [`Self::validate_path`] (#578).
    /// Previously the canonical branch ran the denylist check plus a `/../`
    /// *substring* check that can never fire on an already-canonical path
    /// (`canonicalize` resolves every `..` component by definition), while the
    /// non-canonical branch ran only the component-based traversal check and
    /// never consulted the denylist — so a nonexistent path under a denied
    /// prefix, such as `/etc/does-not-exist`, passed straight through.
    ///
    /// # Errors
    ///
    /// Returns an error when the path contains a parent-directory component or
    /// targets a protected system directory.
    fn check_components(path: &Path) -> Result<()> {
        // Check for an actual `..` path component rather than a `".."` substring:
        // the substring check rejected legitimate names like `my..project` that
        // merely contain two dots without being a parent-directory traversal (#323).
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(Error::ipc("Path traversal patterns not allowed".to_owned()));
        }

        // Component-wise comparison against the denied prefixes (see
        // `denylisted_system_prefixes`) rather than a string prefix check.
        if Self::denylisted_system_prefixes()
            .iter()
            .any(|prefix| path.starts_with(prefix))
        {
            return Err(Error::ipc("Access to system directories denied".to_owned()));
        }

        Ok(())
    }
}

/// PID validation utilities
pub struct PidValidator;

impl PidValidator {
    /// Probe whether a process with this PID is currently alive.
    ///
    /// Liveness only: the plausible-range invariant belongs to
    /// [`crate::config::types::ClientPid`], which cannot be constructed out of
    /// range. This runs exactly once per request, at IPC route time
    /// (`ipc::server::connection::route_request_secure`), and is the one place
    /// `GuardSettings::validate_pids` actually has an effect. It used to run at
    /// `ClientPid` construction as well — unconditionally, before any
    /// `GuardSettings` was in scope — so that toggle could not disable liveness
    /// probing and every registration paid for the same `kill(pid, 0)` twice
    /// (#578).
    ///
    /// A PID is not an identity: this proves a process exists, and nothing
    /// about who is on the other end of the socket.
    ///
    /// # Errors
    ///
    /// Returns an error if no process with this PID exists. A probe that cannot
    /// answer (an unexpected `errno`) is allowed through with a warning.
    #[cfg(unix)]
    pub fn validate_pid_liveness(pid: u32) -> Result<()> {
        match Self::check_process_exists(pid) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::ipc(format!("Process {pid} does not exist"))),
            Err(_) => {
                // If we can't check, allow it but log warning
                warn_log!("security", "Could not validate PID {pid}");
                Ok(())
            }
        }
    }

    /// Native Windows stub: there is no Unix-socket IPC peer to probe on this
    /// platform, so liveness always succeeds. The range invariant still holds —
    /// it is enforced by [`crate::config::types::ClientPid`] on every platform
    /// (#578).
    ///
    /// # Errors
    ///
    /// Never returns an error on this platform.
    #[cfg(not(unix))]
    pub fn validate_pid_liveness(_pid: u32) -> Result<()> {
        Ok(())
    }

    /// Check if a process exists (Unix only)
    ///
    /// # Errors
    ///
    /// Returns an error if probing the process state fails for reasons other than missing or
    /// permission-restricted processes.
    #[cfg(unix)]
    fn check_process_exists(pid: u32) -> Result<bool> {
        // Use nix for direct syscall - faster and more reliable than shell command
        use nix::sys::signal::kill;
        use nix::unistd::Pid;

        // kill(pid, 0) checks if process exists without actually sending a signal
        let pid_i32 = i32::try_from(pid)
            .map_err(|_| Error::ipc(format!("PID {pid} too large for platform")))?;

        match kill(Pid::from_raw(pid_i32), None) {
            Ok(()) => {
                // Process exists and we can signal it
                Ok(true)
            }
            Err(nix::errno::Errno::ESRCH) => {
                // No such process
                Ok(false)
            }
            Err(nix::errno::Errno::EPERM) => {
                // Process exists but we lack permission to signal it
                Ok(true)
            }
            Err(e) => {
                // Other error - treat as failure to check
                Err(Error::ipc(format!("Failed to check process: {e}")))
            }
        }
    }
}

/// Rate limiting for connection attempts.
///
/// This is a global sliding window (not a per-client token bucket): every
/// accepted connection shares one counter, capped at `max_per_second` per
/// rolling one-second window.
pub struct RateLimiter {
    connections: Vec<Instant>,
    max_per_second: u32,
}

impl RateLimiter {
    /// Create a new rate limiter
    #[must_use]
    pub const fn new(max_per_second: u32) -> Self {
        Self {
            connections: Vec::new(),
            max_per_second,
        }
    }

    /// Check if a new connection should be allowed
    pub fn allow_connection(&mut self) -> bool {
        self.allow_connection_at(Instant::now())
    }

    /// Core of `allow_connection`, parameterized on the current time so tests
    /// can exercise window-boundary behavior without sleeping.
    ///
    /// Uses `Instant` (monotonic) rather than `SystemTime` (wall clock): a
    /// backward NTP step can timestamp already-recorded connections as
    /// "in the future" relative to a stepped-back wall clock, so they never
    /// expire and the window fills permanently, locking out every client
    /// (#315). `Instant` cannot be affected by wall-clock adjustments.
    fn allow_connection_at(&mut self, now: Instant) -> bool {
        let one_second_ago = now.checked_sub(Duration::from_secs(1));

        // Remove old connections. `checked_sub` only returns `None` when
        // `now` is less than one second past the monotonic clock's epoch
        // (i.e. near process start), in which case nothing has aged out yet.
        self.connections
            .retain(|&time| one_second_ago.is_none_or(|cutoff| time > cutoff));

        // Check if we're under the limit
        if self.connections.len() < usize::try_from(self.max_per_second).unwrap_or(usize::MAX) {
            self.connections.push(now);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Ensure control characters are rejected, not rewritten.
    ///
    /// Was `test_path_sanitization`, which asserted the opposite for
    /// non-NUL control characters: `sanitize_string` returned a *cleaned* copy
    /// of the value, so the caller silently got a different path than it asked
    /// for. #578 replaced that with `reject_control_chars`.
    ///
    /// # Panics
    ///
    /// Panics if `reject_control_chars` does not behave as expected.
    #[test]
    fn test_reject_control_chars() {
        // Valid paths should pass through
        assert!(reject_control_chars("path", "/valid/path").is_ok());

        // NUL bytes should be rejected
        assert!(
            reject_control_chars("path", "/path\x00with\x01control").is_err(),
            "reject_control_chars should reject NUL bytes"
        );

        // Other control characters should be rejected too, not stripped
        assert!(
            reject_control_chars("path", "/path\x01with\x02control").is_err(),
            "reject_control_chars should reject control characters"
        );
    }

    /// Validate traversal detection rejects unsafe paths.
    ///
    /// # Panics
    ///
    /// Panics if `validate_path` does not enforce traversal rules.
    #[test]
    fn test_traversal_detection() {
        // These should be rejected (traversal patterns)
        assert!(PathValidator::validate_path("../../../etc/passwd").is_err());
        assert!(PathValidator::validate_path("path/with/../traversal").is_err());

        // System directories should be rejected
        // Note: /etc/shadow might canonicalize successfully, so we test the safety check
        let _result = PathValidator::validate_path("/etc/shadow");
        // Could be Ok if file doesn't exist, or Err if it does exist and we catch it

        // Valid relative paths should work
        assert!(PathValidator::validate_path("./src").is_ok());
        assert!(PathValidator::validate_path("relative/path").is_ok());
    }

    /// Regression test for #323: a `..` *substring* inside a single path
    /// segment (not a `..` path *component*) must not be treated as traversal.
    ///
    /// # Panics
    ///
    /// Panics if a legitimately dotted directory name is rejected.
    #[test]
    fn test_check_path_safety_allows_dotted_names_without_parent_dir_component() {
        assert!(
            PathValidator::validate_path("relative/my..project").is_ok(),
            "a directory literally named 'my..project' must not be rejected as traversal"
        );
    }

    /// Regression test for #323.
    ///
    /// `/etc/hosts` must be denied even when the host resolves `/etc` through
    /// a symlink (e.g. macOS's `/etc` -> `/private/etc`), which previously
    /// bypassed a literal `/etc/` prefix check once the path was canonicalized.
    ///
    /// # Panics
    ///
    /// Panics if a system path is not rejected.
    #[test]
    fn test_traversal_detection_rejects_etc_hosts_regardless_of_symlinks() {
        if std::path::Path::new("/etc/hosts").exists() {
            assert!(
                PathValidator::validate_path("/etc/hosts").is_err(),
                "/etc/hosts must be denied even after symlink resolution"
            );
        }
    }

    /// Regression test for #578: a path that does not exist on disk must still
    /// be checked against the denylist.
    ///
    /// `canonicalize` fails for a nonexistent path, and the denylist check used
    /// to live only in the branch where it succeeded, so any path under a
    /// denied prefix passed straight through as long as it had not been created
    /// yet.
    ///
    /// # Panics
    ///
    /// Panics if a nonexistent path under a denied prefix is accepted.
    #[test]
    fn test_nonexistent_path_under_denied_prefix_is_rejected() {
        for candidate in [
            "/etc/gpy-578-nonexistent-test-file",
            "/dev/gpy-578-nonexistent-test-file",
        ] {
            assert!(
                !std::path::Path::new(candidate).exists(),
                "test precondition: {candidate} must not exist"
            );
            assert!(
                PathValidator::validate_path(candidate).is_err(),
                "{candidate} must be denied even though it does not exist"
            );
        }
    }

    /// Newline/tab/CR in a path must be rejected (#578), superseding the #323
    /// fix that stripped them.
    ///
    /// #323 was right that these characters must not reach the prompt — a
    /// directory name containing a newline can line-break the rendered output —
    /// but stripping them handed the caller a path to a *different* directory
    /// with no indication anything had changed. Rejecting refuses the request
    /// instead, which is the only answer that cannot silently mislead.
    ///
    /// # Panics
    ///
    /// Panics if a path containing whitespace control characters is accepted.
    #[test]
    fn test_reject_control_chars_rejects_newline_tab_cr() {
        assert!(
            reject_control_chars("path", "/path\nwith\ttab\rand-cr").is_err(),
            "newline, tab and CR must be rejected, not stripped"
        );

        // And the same through the public entry point, which is what callers hit.
        assert!(
            PathValidator::validate_path("/path\nwith\ttab\rand-cr").is_err(),
            "validate_path must reject a path containing control characters"
        );
    }

    /// Check the PID range invariant and the liveness probe, which #578 split
    /// apart.
    ///
    /// Range belongs to `ClientPid` (enforced by construction, on every
    /// platform), liveness to `PidValidator` (probed once, at route time).
    ///
    /// # Panics
    ///
    /// Panics if either check returns unexpected results for test values.
    #[test]
    fn test_pid_validation() {
        use crate::config::types::ClientPid;

        // Range: rejected by construction, so no out-of-range `ClientPid` can exist.
        assert!(!ClientPid::pid_in_range(0));
        assert!(!ClientPid::pid_in_range(4_194_305));
        assert!(ClientPid::new(0).is_err());
        assert!(ClientPid::new(4_194_305).is_err());
        assert!(ClientPid::pid_in_range(1));

        // Liveness: the current process is alive by definition.
        let current_pid = std::process::id();
        assert!(PidValidator::validate_pid_liveness(current_pid).is_ok());
    }

    /// #578: constructing a `ClientPid` must not probe liveness.
    ///
    /// Liveness is probed exactly once per request, at route time
    /// (`route_request_secure`), which is the only call site of
    /// `PidValidator::validate_pid_liveness` in production code and the only
    /// place `GuardSettings::validate_pids` can gate. Construction previously
    /// ran its own `kill(pid, 0)`, so the toggle could not disable liveness
    /// checking and every registration probed twice.
    ///
    /// A dead-but-in-range PID is the observable difference: it constructs
    /// cleanly now, and is rejected later at route time.
    ///
    /// # Panics
    ///
    /// Panics if a range-valid PID fails to construct.
    #[test]
    fn test_client_pid_construction_does_not_probe_liveness() {
        use crate::config::types::ClientPid;

        // A PID that is in range but (almost certainly) not running. If
        // construction still probed liveness, this would fail.
        let dead_pid = 4_194_303_u32;
        assert!(
            ClientPid::new(dead_pid).is_ok(),
            "construction must range-check only, never probe liveness"
        );

        // Repeated construction stays a pure, syscall-free range check.
        for pid in 1_u32..=256 {
            assert!(ClientPid::new(pid).is_ok(), "PID {pid} is in range");
        }
    }

    /// Verify the rate limiter enforces the configured allowance.
    ///
    /// # Panics
    ///
    /// Panics if the limiter does not accept or reject connections as expected.
    #[test]
    fn test_rate_limiting() {
        let mut limiter = RateLimiter::new(2);

        // First two connections should be allowed
        assert!(limiter.allow_connection());
        assert!(limiter.allow_connection());

        // Third should be denied
        assert!(!limiter.allow_connection());
    }

    /// Regression test for #315.
    ///
    /// The limiter previously timestamped connections with `SystemTime::now()`.
    /// A backward wall-clock step made already-recorded connections look like
    /// they were made "in the future", so they never expired and the window
    /// filled permanently, rejecting every client. `Instant` is monotonic and
    /// immune to that class of failure; drive `allow_connection_at` directly
    /// to verify the window still expires and refills correctly across time.
    ///
    /// # Panics
    ///
    /// Panics if the sliding window does not expire and refill as expected.
    #[test]
    fn test_rate_limiter_immune_to_wall_clock_step() {
        let mut limiter = RateLimiter::new(1);
        let base = Instant::now();

        // Fill the one-connection-per-second window.
        assert!(limiter.allow_connection_at(base));
        assert!(!limiter.allow_connection_at(base));

        // A wall-clock backward step cannot move a real `Instant` backward,
        // so simulate the only thing that changes: forward progress. Once
        // more than a second has elapsed, the earlier entry expires.
        assert!(limiter.allow_connection_at(base + Duration::from_secs(2)));

        // The window is full again immediately after.
        assert!(!limiter.allow_connection_at(base + Duration::from_secs(2)));

        // And it keeps refilling on schedule rather than wedging shut.
        assert!(limiter.allow_connection_at(base + Duration::from_secs(4)));
    }
}

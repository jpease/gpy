//! Security validation and hardening utilities

#[cfg(unix)]
use crate::warn_log;
use crate::{Error, Result};
use std::path::{Component, Path, PathBuf};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

/// A path that has passed [`PathValidator`] checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafePath(PathBuf);

impl SafePath {
    /// Validate and wrap `path`. Blocks on `canonicalize()`; call from `spawn_blocking`.
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

/// Maximum allowed path length in bytes.
const MAX_PATH_LENGTH: usize = 4096;

/// Wire-size cap shared by the connection guard and `ipc::protocol`.
pub(crate) const MAX_MESSAGE_SIZE: usize = 64 * 1024;

const DEFAULT_REQUEST_TIMEOUT_SECS: u64 = 5;
const DEFAULT_MAX_CONNECTIONS_PER_SEC: u32 = 1000;
const DEFAULT_MAX_CONCURRENT_CONNECTIONS: usize = 200;

/// Security limits for the IPC server.
#[derive(Clone)]
pub struct GuardSettings {
    /// Per-request timeout.
    pub request_timeout: Duration,
    /// Maximum message size in bytes.
    pub max_message_size: usize,
    /// Probe client PID liveness at route time.
    pub validate_pids: bool,
    /// Global connection rate limit.
    pub max_connections_per_second: u32,
    /// Cap on concurrent connection tasks.
    pub max_concurrent_connections: usize,
}

impl Default for GuardSettings {
    fn default() -> Self {
        Self {
            request_timeout: Duration::from_secs(DEFAULT_REQUEST_TIMEOUT_SECS),
            max_message_size: MAX_MESSAGE_SIZE,
            validate_pids: true,
            max_connections_per_second: DEFAULT_MAX_CONNECTIONS_PER_SEC,
            max_concurrent_connections: DEFAULT_MAX_CONCURRENT_CONNECTIONS,
        }
    }
}

/// Reject (never strip) control characters: a stripped value names something
/// else. `template::eval` strips at render time because it cannot reject.
///
/// # Errors
/// Returns an error if `value` contains any `char::is_control` character.
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

/// Path security validation.
pub struct PathValidator;

impl PathValidator {
    /// Canonicalize and validate a path.
    ///
    /// # Errors
    /// Returns an error for control characters, excess length, `..` components, or system directories.
    pub fn validate_path(path: &str) -> Result<PathBuf> {
        reject_control_chars("path", path)?;

        if path.len() > MAX_PATH_LENGTH {
            return Err(Error::ipc(format!(
                "Path too long (max {MAX_PATH_LENGTH} characters)"
            )));
        }

        let path_buf = PathBuf::from(path);

        // Nonexistent paths get the same component checks.
        if let Ok(canonical) = path_buf.canonicalize() {
            Self::check_components(&canonical)?;
            Ok(canonical)
        } else {
            Self::check_components(&path_buf)?;
            Ok(path_buf)
        }
    }

    /// Denied system directories in literal and canonical form: existing
    /// inputs are canonicalized (macOS `/etc` -> `/private/etc`), missing ones are not.
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

    /// Reject `..` components and denylisted system directories.
    ///
    /// # Errors
    /// Returns an error on traversal or a protected directory.
    fn check_components(path: &Path) -> Result<()> {
        if path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
        {
            return Err(Error::ipc("Path traversal patterns not allowed".to_owned()));
        }

        if Self::denylisted_system_prefixes()
            .iter()
            .any(|prefix| path.starts_with(prefix))
        {
            return Err(Error::ipc("Access to system directories denied".to_owned()));
        }

        Ok(())
    }
}

/// PID validation.
pub struct PidValidator;

impl PidValidator {
    /// Probe whether `pid` is alive. Range checks live in `ClientPid`; this runs
    /// once per request at route time so `GuardSettings::validate_pids` can gate it.
    ///
    /// # Errors
    /// Returns an error if no such process exists; an inconclusive probe is allowed.
    #[cfg(unix)]
    pub fn validate_pid_liveness(pid: u32) -> Result<()> {
        match Self::check_process_exists(pid) {
            Ok(true) => Ok(()),
            Ok(false) => Err(Error::ipc(format!("Process {pid} does not exist"))),
            Err(_) => {
                warn_log!("security", "Could not validate PID {pid}");
                Ok(())
            }
        }
    }

    /// No Unix-socket peer to probe on native Windows; always succeeds.
    ///
    /// # Errors
    /// Never.
    #[cfg(not(unix))]
    pub fn validate_pid_liveness(_pid: u32) -> Result<()> {
        Ok(())
    }

    /// `kill(pid, 0)`: `EPERM` still means the process exists.
    ///
    /// # Errors
    /// Returns an error on unexpected `errno`.
    #[cfg(unix)]
    fn check_process_exists(pid: u32) -> Result<bool> {
        use nix::sys::signal::kill;
        use nix::unistd::Pid;

        let pid_i32 = i32::try_from(pid)
            .map_err(|_| Error::ipc(format!("PID {pid} too large for platform")))?;

        match kill(Pid::from_raw(pid_i32), None) {
            // EPERM: the process exists but belongs to someone else.
            Ok(()) | Err(nix::errno::Errno::EPERM) => Ok(true),
            Err(nix::errno::Errno::ESRCH) => Ok(false),
            Err(e) => Err(Error::ipc(format!("Failed to check process: {e}"))),
        }
    }
}

/// Global sliding-window connection rate limiter (one shared counter).
pub struct RateLimiter {
    connections: Vec<Instant>,
    max_per_second: u32,
}

impl RateLimiter {
    /// Create a new rate limiter.
    #[must_use]
    pub const fn new(max_per_second: u32) -> Self {
        Self {
            connections: Vec::new(),
            max_per_second,
        }
    }

    /// Check if a new connection should be allowed.
    pub fn allow_connection(&mut self) -> bool {
        self.allow_connection_at(Instant::now())
    }

    /// Uses monotonic `Instant`: a backward wall-clock step would keep entries
    /// from ever expiring and lock out every client.
    fn allow_connection_at(&mut self, now: Instant) -> bool {
        let one_second_ago = now.checked_sub(Duration::from_secs(1));

        // `None` only near the monotonic epoch, when nothing has aged out yet.
        self.connections
            .retain(|&time| one_second_ago.is_none_or(|cutoff| time > cutoff));

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

    /// Control characters are rejected, not rewritten.
    #[test]
    fn test_reject_control_chars() {
        assert!(reject_control_chars("path", "/valid/path").is_ok());

        assert!(
            reject_control_chars("path", "/path\x00with\x01control").is_err(),
            "reject_control_chars should reject NUL bytes"
        );

        assert!(
            reject_control_chars("path", "/path\x01with\x02control").is_err(),
            "reject_control_chars should reject control characters"
        );
    }

    #[test]
    fn test_traversal_detection() {
        assert!(PathValidator::validate_path("../../../etc/passwd").is_err());
        assert!(PathValidator::validate_path("path/with/../traversal").is_err());

        let _result = PathValidator::validate_path("/etc/shadow");

        assert!(PathValidator::validate_path("./src").is_ok());
        assert!(PathValidator::validate_path("relative/path").is_ok());
    }

    /// `my..project` is not a `..` component.
    #[test]
    fn test_check_path_safety_allows_dotted_names_without_parent_dir_component() {
        assert!(
            PathValidator::validate_path("relative/my..project").is_ok(),
            "a directory literally named 'my..project' must not be rejected as traversal"
        );
    }

    /// Denied even when `/etc` is a symlink (macOS).
    #[test]
    fn test_traversal_detection_rejects_etc_hosts_regardless_of_symlinks() {
        if std::path::Path::new("/etc/hosts").exists() {
            assert!(
                PathValidator::validate_path("/etc/hosts").is_err(),
                "/etc/hosts must be denied even after symlink resolution"
            );
        }
    }

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

    #[test]
    fn test_reject_control_chars_rejects_newline_tab_cr() {
        assert!(
            reject_control_chars("path", "/path\nwith\ttab\rand-cr").is_err(),
            "newline, tab and CR must be rejected, not stripped"
        );

        assert!(
            PathValidator::validate_path("/path\nwith\ttab\rand-cr").is_err(),
            "validate_path must reject a path containing control characters"
        );
    }

    #[test]
    fn test_pid_validation() {
        use crate::config::types::ClientPid;

        assert!(!ClientPid::pid_in_range(0));
        assert!(!ClientPid::pid_in_range(4_194_305));
        assert!(ClientPid::new(0).is_err());
        assert!(ClientPid::new(4_194_305).is_err());
        assert!(ClientPid::pid_in_range(1));

        let current_pid = std::process::id();
        assert!(PidValidator::validate_pid_liveness(current_pid).is_ok());
    }

    /// Constructing a `ClientPid` must not probe liveness (route time only).
    #[test]
    fn test_client_pid_construction_does_not_probe_liveness() {
        use crate::config::types::ClientPid;

        let dead_pid = 4_194_303_u32;
        assert!(
            ClientPid::new(dead_pid).is_ok(),
            "construction must range-check only, never probe liveness"
        );

        for pid in 1_u32..=256 {
            assert!(ClientPid::new(pid).is_ok(), "PID {pid} is in range");
        }
    }

    #[test]
    fn test_rate_limiting() {
        let mut limiter = RateLimiter::new(2);

        assert!(limiter.allow_connection());
        assert!(limiter.allow_connection());

        assert!(!limiter.allow_connection());
    }

    /// Window expires on monotonic time.
    #[test]
    fn test_rate_limiter_immune_to_wall_clock_step() {
        let mut limiter = RateLimiter::new(1);
        let base = Instant::now();

        assert!(limiter.allow_connection_at(base));
        assert!(!limiter.allow_connection_at(base));

        assert!(limiter.allow_connection_at(base + Duration::from_secs(2)));

        assert!(!limiter.allow_connection_at(base + Duration::from_secs(2)));

        assert!(limiter.allow_connection_at(base + Duration::from_secs(4)));
    }
}

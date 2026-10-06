//! Two-level logging surface, controlled by the `GPY_DEBUG_LOG` environment variable.
//!
//! - `debug_log!` / [`write_debug_log`] is the opt-in trace level: it writes to the
//!   `GPY_DEBUG_LOG` file only, and is silent when that variable is unset. Intended for
//!   development and troubleshooting.
//! - `warn_log!` / [`warn`] is the always-visible level: it unconditionally writes to
//!   stderr, and additionally appends the same line to the `GPY_DEBUG_LOG` file when it
//!   is set. This is what recovers a daemonized agent's diagnostics — once started in
//!   the background its stdio is redirected to `/dev/null`, so stderr alone is not
//!   enough to make an error recoverable.
//!
//! Both levels share one line formatter ([`format_line`]) and one file-append routine
//! ([`append_to`]), so the two destinations never drift out of sync.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

/// `GPY_DEBUG_LOG`, made absolute against the launch directory (#724).
static DEBUG_PATH: OnceLock<Option<String>> = OnceLock::new();

/// Check if debug logging is enabled
fn debug_log_path() -> Option<&'static str> {
    DEBUG_PATH
        .get_or_init(|| {
            std::env::var("GPY_DEBUG_LOG").ok().map(|value| {
                crate::paths::absolutize(Path::new(&value))
                    .to_string_lossy()
                    .into_owned()
            })
        })
        .as_deref()
}

/// Resolve `GPY_DEBUG_LOG` now, so a daemon forked afterwards inherits the
/// path as it resolved in the launch directory, not against `/` (#724).
pub fn init_path() {
    let _ = debug_log_path();
}

/// Format one log line as `[timestamp_ms] [category] message`.
///
/// Pure formatter shared by both [`write_debug_log`] and [`warn`], so the two levels
/// never diverge in on-disk shape and this can be unit-tested without touching the
/// process environment.
fn format_line(timestamp_ms: u128, category: &str, message: &str) -> String {
    format!("[{timestamp_ms}] [{category}] {message}")
}

/// Append an already-formatted line to the log file at `path`, creating it if needed.
///
/// Best-effort: a failure to open or write the file is silently ignored, matching the
/// pre-existing behavior of `write_debug_log` (a logging failure must never surface as
/// an application error).
fn append_to(path: &Path, line: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

/// Current time as milliseconds since the Unix epoch, for log line timestamps.
fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Log a debug message if `GPY_DEBUG_LOG` is set
///
/// Opt-in trace level: silent unless `GPY_DEBUG_LOG` is set. For a message that must
/// stay visible even when the process is daemonized (stdio redirected to
/// `/dev/null`), use [`warn_log`] instead.
///
/// Usage:
/// ```no_run
/// use gpy_agent::debug_log;
///
/// let repo_path = "/path/to/repo";
/// let pid = 12345;
/// debug_log!("cache", "Cache hit for {}", repo_path);
/// debug_log!("watcher", "Registered client {}", pid);
/// ```
#[macro_export]
macro_rules! debug_log {
    ($category:expr, $($arg:tt)*) => {
        $crate::debug::write_debug_log($category, &format!($($arg)*));
    };
}

/// Write a debug log entry (internal use only)
#[doc(hidden)]
pub fn write_debug_log(category: &str, message: &str) {
    if let Some(log_path) = debug_log_path() {
        append_to(
            Path::new(log_path),
            &format_line(now_millis(), category, message),
        );
    }
}

/// Check if debug logging is enabled (for conditional expensive operations)
#[must_use]
pub fn is_debug_enabled() -> bool {
    debug_log_path().is_some()
}

/// Log a warning message: unconditionally to stderr, and to `GPY_DEBUG_LOG` if set.
///
/// Usage:
/// ```no_run
/// use gpy_agent::warn_log;
///
/// let pid = 12345;
/// let error = "connection reset";
/// warn_log!("ipc", "Client {} disconnected: {}", pid, error);
/// ```
#[macro_export]
macro_rules! warn_log {
    ($category:expr, $($arg:tt)*) => {
        $crate::debug::warn($category, &format!($($arg)*))
    };
}

/// Write a warning: always visible on stderr, and also mirrored into the
/// `GPY_DEBUG_LOG` file when that variable is set.
///
/// This is the always-visible counterpart to [`write_debug_log`]/`debug_log!`: it
/// never depends on `GPY_DEBUG_LOG` to be seen, but still becomes recoverable from the
/// debug log file for a daemonized agent whose stderr is not being captured. Prefer
/// the [`warn_log!`] macro at call sites; this function exists so [`warn_fallback`]
/// (and any other formatting wrapper) can share the same destination pair.
pub fn warn(category: &str, message: &str) {
    // Deliberately not `eprintln!`: `logging_channel_guard` (tests/) asserts
    // no raw `eprintln!` remains anywhere under this file so every warning,
    // including this crate's own, is proven to funnel through this one
    // function rather than a call site reaching around it.
    let _ = writeln!(std::io::stderr(), "Warning: [{category}] {message}");

    if let Some(log_path) = debug_log_path() {
        append_to(
            Path::new(log_path),
            &format_line(now_millis(), category, message),
        );
    }
}

/// Print a warning about a fallback condition to stderr
///
/// Used to standardize warning messages when operations fail but have
/// reasonable defaults to fall back to. A thin formatting wrapper over [`warn`],
/// using `component` as the category.
///
/// # Example
///
/// ```no_run
/// use gpy_agent::debug::warn_fallback;
/// use std::io;
///
/// let error = io::Error::new(io::ErrorKind::NotFound, "config not found");
/// warn_fallback("Config loading", "using defaults", &error);
/// // Prints: "Warning: [Config loading] failed (config not found), using defaults"
/// ```
pub fn warn_fallback(component: &str, fallback_action: &str, error: &dyn std::error::Error) {
    warn(component, &format!("failed ({error}), {fallback_action}"));
}

#[cfg(test)]
mod tests {
    #![allow(clippy::missing_panics_doc)]

    use super::{append_to, format_line};
    use std::fs;

    #[test]
    fn format_line_matches_bracketed_shape() {
        let line = format_line(1_700_000_000_000, "cache", "Cache hit for /repo");
        assert_eq!(line, "[1700000000000] [cache] Cache hit for /repo");
    }

    #[test]
    fn format_line_preserves_message_content_verbatim() {
        let line = format_line(0, "server", "Transient accept() error: broken pipe");
        assert_eq!(line, "[0] [server] Transient accept() error: broken pipe");
    }

    #[test]
    fn append_to_creates_file_and_writes_line() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("debug.log");

        append_to(&path, "[1] [test] first line");

        let contents = fs::read_to_string(&path).expect("read log file");
        assert_eq!(contents, "[1] [test] first line\n");
    }

    #[test]
    fn append_to_appends_rather_than_overwrites() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("debug.log");

        append_to(&path, "[1] [test] first line");
        append_to(&path, "[2] [test] second line");

        let contents = fs::read_to_string(&path).expect("read log file");
        assert_eq!(contents, "[1] [test] first line\n[2] [test] second line\n");
    }

    #[test]
    fn append_to_silently_ignores_an_unwritable_path() {
        // A path under a nonexistent parent directory can't be opened for
        // append; this must not panic (best-effort, matching the pre-existing
        // `write_debug_log` behavior).
        let path = std::path::Path::new("/nonexistent-gpy-debug-dir/debug.log");
        append_to(path, "[1] [test] unreachable");
    }
}

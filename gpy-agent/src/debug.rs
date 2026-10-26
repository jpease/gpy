//! Logging controlled by `GPY_DEBUG_LOG`.
//!
//! `debug_log!` writes only to the `GPY_DEBUG_LOG` file. `warn_log!` always
//! writes stderr and mirrors to that file, since a daemon's stderr is `/dev/null`.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;
use std::sync::OnceLock;

/// `GPY_DEBUG_LOG`, made absolute against the launch directory.
static DEBUG_PATH: OnceLock<Option<String>> = OnceLock::new();

/// Return the path to the debug log file if enabled.
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
/// path as it resolved in the launch directory, not against `/`.
pub fn init_path() {
    let _ = debug_log_path();
}

/// Format one log line as `[timestamp_ms] [category] message`.
fn format_line(timestamp_ms: u128, category: &str, message: &str) -> String {
    format!("[{timestamp_ms}] [{category}] {message}")
}

/// Append `line` to `path`, creating it; failures are ignored.
fn append_to(path: &Path, line: &str) {
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(file, "{line}");
    }
}

/// Milliseconds since the Unix epoch.
fn now_millis() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

/// Log to the `GPY_DEBUG_LOG` file; silent when unset. Use [`warn_log`] for
/// messages that must stay visible.
#[macro_export]
macro_rules! debug_log {
    ($category:expr, $($arg:tt)*) => {
        if $crate::debug::is_debug_enabled() {
            $crate::debug::write_debug_log($category, &format!($($arg)*));
        }
    };
}

/// Write a debug log entry (internal use only).
pub(crate) fn write_debug_log(category: &str, message: &str) {
    if let Some(log_path) = debug_log_path() {
        append_to(
            Path::new(log_path),
            &format_line(now_millis(), category, message),
        );
    }
}

/// Check if debug logging is enabled.
#[must_use]
pub fn is_debug_enabled() -> bool {
    debug_log_path().is_some()
}

/// Log a warning to stderr, and to `GPY_DEBUG_LOG` if set.
#[macro_export]
macro_rules! warn_log {
    ($category:expr, $($arg:tt)*) => {
        $crate::debug::warn($category, &format!($($arg)*))
    };
}

/// Write a warning to stderr and mirror it to `GPY_DEBUG_LOG` when set.
pub fn warn(category: &str, message: &str) {
    // Not `eprintln!`: `clippy::print_stderr` is denied.
    let _ = writeln!(std::io::stderr(), "Warning: [{category}] {message}");

    write_debug_log(category, message);
}

/// Warn that `component` failed with `error` and is doing `fallback_action`.
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
        // Best-effort: must not panic.
        let path = std::path::Path::new("/nonexistent-gpy-debug-dir/debug.log");
        append_to(path, "[1] [test] unreachable");
    }

    #[test]
    fn debug_log_does_not_evaluate_arguments_when_logging_is_disabled() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        struct CountingDisplay<'a>(&'a AtomicUsize);
        impl std::fmt::Display for CountingDisplay<'_> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.0.fetch_add(1, Ordering::SeqCst);
                write!(f, "counted")
            }
        }

        assert!(
            !super::is_debug_enabled(),
            "test expects GPY_DEBUG_LOG to be unset"
        );
        let counter = AtomicUsize::new(0);
        crate::debug_log!("test", "side effect: {}", CountingDisplay(&counter));
        assert_eq!(counter.load(Ordering::SeqCst), 0);
    }
}

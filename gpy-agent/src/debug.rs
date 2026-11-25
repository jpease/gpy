//! Logging: `GPY_DEBUG_LOG` for traces, `agent.log` for warnings.
//!
//! `debug_log!` writes only to the `GPY_DEBUG_LOG` file. `warn_log!` always
//! writes stderr and appends to the warning log, since a daemon's stderr is
//! `/dev/null`. The warning log is the `GPY_DEBUG_LOG` file when set, else
//! `agent.log` in the cache root (`~/.cache/gpy/agent.log`), so warnings
//! survive without any opt-in. Nothing else writes `agent.log`. `gpy prompt`
//! holds the stderr line ([`StderrHold`]) and shows each warning once per
//! shell session.

use crate::sync_ext::MutexExt as _;
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::Instant;

/// File name of the default warning log inside the cache root.
const AGENT_LOG_NAME: &str = "agent.log";

/// Where `warn_log!` appends: `GPY_DEBUG_LOG`, else the cache root's `agent.log`;
/// `None` without a cache root.
static WARN_PATH: LazyLock<Option<PathBuf>> = LazyLock::new(|| {
    debug_log_path()
        .map(Path::to_path_buf)
        .or_else(agent_log_path)
});

/// `GPY_DEBUG_LOG`, as the environment spells it (a relative value is
/// ignored).
static DEBUG_PATH: LazyLock<Option<PathBuf>> =
    LazyLock::new(|| crate::env_config::DEBUG_LOG.path());

/// Return the path to the debug log file if enabled.
fn debug_log_path() -> Option<&'static Path> {
    DEBUG_PATH.as_deref()
}

/// `agent.log` in the cache root, the warning log without `GPY_DEBUG_LOG`.
fn agent_log_path() -> Option<PathBuf> {
    crate::paths::cache_root().map(|root| root.join(AGENT_LOG_NAME))
}

/// Read `GPY_DEBUG_LOG` now, so a value the agent ignores is reported on the
/// terminal that starts it, before the daemon's stderr is `/dev/null`.
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
        append_to(log_path, &format_line(now_millis(), category, message));
    }
}

/// Check if debug logging is enabled.
#[must_use]
pub fn is_debug_enabled() -> bool {
    debug_log_path().is_some()
}

/// Log a warning to stderr and the warning log (`GPY_DEBUG_LOG`, else
/// `agent.log` in the cache root), whether or not debug logging is on.
#[macro_export]
macro_rules! warn_log {
    ($category:expr, $($arg:tt)*) => {
        $crate::debug::warn($category, &format!($($arg)*))
    };
}

#[cfg(test)]
thread_local! {
    /// Warnings captured instead of written while [`capture_warnings`] runs.
    static CAPTURED: std::cell::RefCell<Option<Vec<String>>> =
        const { std::cell::RefCell::new(None) };
}

/// Run `f`, collecting the `[category] message` of every [`warn`] it makes on
/// this thread instead of writing stderr or the log file.
#[cfg(test)]
pub(crate) fn capture_warnings<R>(f: impl FnOnce() -> R) -> (R, Vec<String>) {
    CAPTURED.with(|slot| *slot.borrow_mut() = Some(Vec::new()));
    let result = f();
    let captured = CAPTURED.with(|slot| slot.borrow_mut().take().unwrap_or_default());
    (result, captured)
}

/// Write a warning to stderr and append it to the warning log.
///
/// An identical warning is dropped when this process emitted it within the
/// last `WARN_REPEAT_WINDOW` or when it already is the newest entry of the
/// warning log (a oneshot process per prompt would otherwise repeat it on
/// every prompt). The log is rotated to `<name>.1` past `WARN_LOG_MAX_BYTES`.
/// While a [`StderrHold`] lives, the stderr line is held for [`take_held`]
/// instead, the newest-entry repeat included.
pub fn warn(category: &str, message: &str) {
    warn_into(WARN_PATH.as_deref(), category, message);
}

/// [`warn`], into `agent.log` whatever `GPY_DEBUG_LOG` says: for a warning
/// raised while `GPY_DEBUG_LOG` itself resolves, where [`warn`] would
/// re-enter that resolution. Only an ignored `GPY_DEBUG_LOG` warns there, and
/// then `agent.log` is the warning log anyway.
pub(crate) fn warn_to_agent_log(category: &str, message: &str) {
    warn_into(agent_log_path().as_deref(), category, message);
}

fn warn_into(log_path: Option<&Path>, category: &str, message: &str) {
    #[cfg(test)]
    {
        let captured = CAPTURED.with(|slot| {
            slot.borrow_mut().as_mut().is_some_and(|lines| {
                lines.push(format!("[{category}] {message}"));
                true
            })
        });
        if captured {
            return;
        }
    }
    if seen_recently(category, message) {
        return;
    }
    let repeat = log_path.is_some_and(|log| is_logged_repeat(log, category, message));
    let warning = HeldWarning {
        line: format!("Warning: [{category}] {message}"),
        repeat,
    };
    if let Some(held) = HELD.lock_or_recover().as_mut() {
        held.push(warning);
    } else if !repeat {
        warning.show();
    }
    if !repeat && let Some(log) = log_path {
        rotate_if_oversized(log);
        append_to(log, &format_line(now_millis(), category, message));
    }
}

/// A warning's stderr line, held while a [`StderrHold`] lives.
#[derive(Debug)]
pub(crate) struct HeldWarning {
    /// `Warning: [category] message`.
    pub(crate) line: String,
    /// The warning log already ended with it, so the usual rule would not
    /// show it.
    pub(crate) repeat: bool,
}

impl HeldWarning {
    /// Write the line to stderr.
    pub(crate) fn show(&self) {
        // Not `eprintln!`: `clippy::print_stderr` is denied.
        let _ = writeln!(std::io::stderr(), "{}", self.line);
    }
}

/// The held warnings while a [`StderrHold`] lives, else `None`.
static HELD: Mutex<Option<Vec<HeldWarning>>> = Mutex::new(None);

/// Holds every warning's stderr line for [`take_held`], so a caller that
/// knows its user decides when it shows: `gpy prompt` shows each once per
/// shell session. Dropped, it shows what is still held by the usual rule.
#[must_use = "warnings are held only while the hold lives"]
pub(crate) struct StderrHold;

/// Start holding warnings' stderr lines; see [`StderrHold`].
pub(crate) fn hold_stderr() -> StderrHold {
    *HELD.lock_or_recover() = Some(Vec::new());
    StderrHold
}

/// The warnings held since the hold started or the last call.
pub(crate) fn take_held() -> Vec<HeldWarning> {
    HELD.lock_or_recover()
        .as_mut()
        .map_or_else(Vec::new, std::mem::take)
}

impl Drop for StderrHold {
    fn drop(&mut self) {
        let left = HELD.lock_or_recover().take().unwrap_or_default();
        for warning in left.iter().filter(|warning| !warning.repeat) {
            warning.show();
        }
    }
}

/// Largest the warning log grows before it is rotated to `<name>.1`.
const WARN_LOG_MAX_BYTES: u64 = 256_u64 * 1024_u64;

/// How long an identical warning is suppressed within one process.
const WARN_REPEAT_WINDOW: std::time::Duration = std::time::Duration::from_secs(600_u64);

/// Most distinct warnings remembered per process before the memo resets.
const WARN_MEMO_CAP: usize = 64;

/// Warnings this process emitted recently, keyed by `category` + message.
static RECENT_WARNINGS: LazyLock<Mutex<HashMap<String, Instant>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

/// Record `category`/`message` and report whether it was already emitted
/// within `WARN_REPEAT_WINDOW` by this process.
fn seen_recently(category: &str, message: &str) -> bool {
    let Ok(mut recent) = RECENT_WARNINGS.lock() else {
        return false;
    };
    let now = Instant::now();
    let key = format!("{category}\0{message}");
    if recent
        .get(&key)
        .is_some_and(|at| now.saturating_duration_since(*at) < WARN_REPEAT_WINDOW)
    {
        return true;
    }
    if recent.len() >= WARN_MEMO_CAP {
        recent.clear();
    }
    recent.insert(key, now);
    false
}

/// Whether the last bytes of `path` are exactly `suffix`, i.e. the newest log
/// entry is the same warning (a short-lived process cannot remember its
/// predecessors, so the log itself is the cross-process memo).
fn log_ends_with(path: &Path, suffix: &str) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let Ok(len) = file.metadata().map(|meta| meta.len()) else {
        return false;
    };
    let Ok(want) = u64::try_from(suffix.len()) else {
        return false;
    };
    if len < want
        || file
            .seek(SeekFrom::Start(len.saturating_sub(want)))
            .is_err()
    {
        return false;
    }
    let mut tail = Vec::with_capacity(suffix.len());
    file.read_to_end(&mut tail).is_ok() && tail == suffix.as_bytes()
}

/// Move an oversized warning log aside to `<name>.1`, replacing any older one.
fn rotate_if_oversized(path: &Path) {
    let oversized = std::fs::metadata(path).is_ok_and(|meta| meta.len() >= WARN_LOG_MAX_BYTES);
    if oversized {
        let mut rotated = path.as_os_str().to_owned();
        rotated.push(".1");
        let _ = std::fs::rename(path, PathBuf::from(rotated));
    }
}

/// Whether the newest entry of the warning log already is this warning.
fn is_logged_repeat(path: &Path, category: &str, message: &str) -> bool {
    log_ends_with(path, &format!("] [{category}] {message}\n"))
}

/// Warn that `component` failed with `error` and is doing `fallback_action`.
pub fn warn_fallback(component: &str, fallback_action: &str, error: &dyn std::error::Error) {
    warn(component, &format!("failed ({error}), {fallback_action}"));
}

#[cfg(test)]
mod tests {

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

    #[test]
    fn log_ends_with_matches_only_the_newest_entry() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("agent.log");
        append_to(&path, "[1] [cat] first\nsecond line");
        assert!(super::is_logged_repeat(&path, "cat", "first\nsecond line"));
        assert!(!super::is_logged_repeat(&path, "cat", "first"));
        assert!(!super::is_logged_repeat(
            &dir.path().join("missing"),
            "cat",
            "x"
        ));
    }

    #[test]
    fn oversized_log_rotates_aside() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("agent.log");
        let big = "x".repeat(usize::try_from(super::WARN_LOG_MAX_BYTES).expect("fits"));
        fs::write(&path, big).expect("write");
        super::rotate_if_oversized(&path);
        assert!(!path.exists());
        assert!(dir.path().join("agent.log.1").exists());
    }

    #[test]
    fn identical_warning_is_suppressed_within_the_window() {
        assert!(!super::seen_recently("test-dedupe", "same message"));
        assert!(super::seen_recently("test-dedupe", "same message"));
        assert!(!super::seen_recently("test-dedupe", "other message"));
    }
}

//! Rate-limited warning log for failed git commands.
//!
//! A repo git cannot read (revoked directory access after an upgrade, say)
//! fails every refresh. Each failure is otherwise invisible, so one warning
//! per `(repo, message)` goes to the warning log, then repeats are counted and
//! reported on the next line allowed through.

use crate::Error;
use crate::sync_ext::MutexExt;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

/// Minimum gap between log lines for the same `(repo, message)`.
const REPEAT_WINDOW: Duration = Duration::from_secs(60);

/// Distinct failures tracked before the table is pruned.
const MAX_TRACKED: usize = 256;

/// Characters of the failure detail kept in a log line.
const MAX_DETAIL_CHARS: usize = 200;

/// When a failure was last logged and how many repeats were dropped since.
struct Seen {
    /// Time of the last emitted line.
    last_logged: Instant,
    /// Repeats suppressed since then.
    suppressed: u64,
}

/// What to do with one failure.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Verdict {
    /// Log it, mentioning `suppressed` dropped repeats when non-zero.
    Emit {
        /// Repeats dropped since the previous line for this failure.
        suppressed: u64,
    },
    /// Within the window of a line already logged.
    Suppress,
}

/// Per-`(repo, message)` limiter; time is passed in so it is testable.
#[derive(Default)]
pub(super) struct FailureLimiter {
    /// Last-logged state per failure.
    seen: HashMap<(PathBuf, String), Seen>,
}

impl FailureLimiter {
    /// Decide whether the failure `message` in `repo` is logged at `now`.
    pub(super) fn check(&mut self, repo: &Path, message: &str, now: Instant) -> Verdict {
        let key = (repo.to_path_buf(), message.to_owned());
        if let Some(entry) = self.seen.get_mut(&key) {
            if now.saturating_duration_since(entry.last_logged) < REPEAT_WINDOW {
                entry.suppressed = entry.suppressed.saturating_add(1);
                return Verdict::Suppress;
            }
            let suppressed = entry.suppressed;
            entry.last_logged = now;
            entry.suppressed = 0;
            return Verdict::Emit { suppressed };
        }
        if self.seen.len() >= MAX_TRACKED {
            self.seen.retain(|_, entry| {
                now.saturating_duration_since(entry.last_logged) < REPEAT_WINDOW
            });
            if self.seen.len() >= MAX_TRACKED {
                self.seen.clear();
            }
        }
        self.seen.insert(
            key,
            Seen {
                last_logged: now,
                suppressed: 0,
            },
        );
        Verdict::Emit { suppressed: 0 }
    }
}

/// Process-wide limiter shared by every git invocation.
static LIMITER: LazyLock<Mutex<FailureLimiter>> =
    LazyLock::new(|| Mutex::new(FailureLimiter::default()));

/// First line of `message`, cut to [`MAX_DETAIL_CHARS`].
fn detail(message: &str) -> String {
    message
        .lines()
        .next()
        .unwrap_or_default()
        .chars()
        .take(MAX_DETAIL_CHARS)
        .collect()
}

/// The git subcommand in `args`: the first argument that is neither an option
/// nor the value of `-c`/`-C`. Only that is logged, never the rest of the
/// command line.
fn subcommand<S: AsRef<OsStr>>(args: &[S]) -> String {
    let mut words = args.iter().map(|arg| arg.as_ref().to_string_lossy());
    while let Some(arg) = words.next() {
        if arg == "-c" || arg == "-C" {
            words.next();
        } else if !arg.starts_with('-') {
            return arg.into_owned();
        }
    }
    "?".to_owned()
}

/// The warning text for one emitted failure.
fn line(repo: &Path, subcommand: &str, detail: &str, suppressed: u64) -> String {
    let base = format!("git {subcommand} failed in {}: {detail}", repo.display());
    if suppressed == 0 {
        base
    } else {
        format!(
            "{base} ({suppressed} identical failures suppressed in the last {}s)",
            REPEAT_WINDOW.as_secs()
        )
    }
}

/// Log a failed git command run in `repo`, at most once per
/// [`REPEAT_WINDOW`] for the same `(repo, message)`.
pub(super) fn log_failure<S: AsRef<OsStr>>(repo: &Path, args: &[S], error: &Error) {
    let Error::Git { message } = error else {
        return;
    };
    let detail = detail(message);
    let verdict = LIMITER
        .lock_or_recover()
        .check(repo, &detail, Instant::now());
    if let Verdict::Emit { suppressed } = verdict {
        crate::debug::warn("git", &line(repo, &subcommand(args), &detail, suppressed));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(base: Instant, secs: u64) -> Instant {
        base.checked_add(Duration::from_secs(secs))
            .expect("instant")
    }

    #[test]
    fn first_failure_emits_and_repeats_inside_the_window_are_suppressed() {
        let mut limiter = FailureLimiter::default();
        let base = Instant::now();
        let repo = Path::new("/repo");
        assert_eq!(
            limiter.check(repo, "boom", base),
            Verdict::Emit { suppressed: 0 }
        );
        assert_eq!(limiter.check(repo, "boom", at(base, 1)), Verdict::Suppress);
        assert_eq!(limiter.check(repo, "boom", at(base, 59)), Verdict::Suppress);
    }

    #[test]
    fn next_emit_after_the_window_reports_the_suppressed_count_once() {
        let mut limiter = FailureLimiter::default();
        let base = Instant::now();
        let repo = Path::new("/repo");
        let _ = limiter.check(repo, "boom", base);
        let _ = limiter.check(repo, "boom", at(base, 10));
        let _ = limiter.check(repo, "boom", at(base, 20));
        assert_eq!(
            limiter.check(repo, "boom", at(base, 60)),
            Verdict::Emit { suppressed: 2 }
        );
        // The window restarts at the emit, and the count with it.
        assert_eq!(limiter.check(repo, "boom", at(base, 61)), Verdict::Suppress);
        assert_eq!(
            limiter.check(repo, "boom", at(base, 120)),
            Verdict::Emit { suppressed: 1 }
        );
    }

    #[test]
    fn different_repos_and_messages_are_limited_independently() {
        let mut limiter = FailureLimiter::default();
        let base = Instant::now();
        let _ = limiter.check(Path::new("/a"), "boom", base);
        assert_eq!(
            limiter.check(Path::new("/b"), "boom", at(base, 1)),
            Verdict::Emit { suppressed: 0 }
        );
        assert_eq!(
            limiter.check(Path::new("/a"), "other", at(base, 1)),
            Verdict::Emit { suppressed: 0 }
        );
    }

    #[test]
    fn table_stays_bounded() {
        let mut limiter = FailureLimiter::default();
        let base = Instant::now();
        for n in 0..(MAX_TRACKED * 2) {
            let _ = limiter.check(Path::new(&format!("/repo{n}")), "boom", base);
        }
        assert!(limiter.seen.len() <= MAX_TRACKED);
    }

    #[test]
    fn subcommand_skips_leading_options_and_ignores_the_rest() {
        assert_eq!(
            subcommand(&["-c", "core.x=1", "status", "--porcelain=v2"]),
            "status"
        );
        assert_eq!(subcommand(&["--no-pager", "status", "-z"]), "status");
        assert_eq!(subcommand::<&str>(&[]), "?");
    }

    #[test]
    fn failing_git_invocation_reaches_the_log_once_per_window() {
        let dir = tempfile::tempdir().expect("tempdir");
        let missing = dir.path().join("not-there");
        let run = || {
            let deadline = Instant::now()
                .checked_add(Duration::from_secs(30))
                .expect("deadline");
            super::super::run_git_env_until(&missing, &["status", "--porcelain"], &[], deadline)
        };

        let (first, lines) = crate::debug::capture_warnings(run);
        assert!(first.is_err(), "git -C <missing> must fail");
        let [logged] = lines.as_slice() else {
            panic!("expected exactly one warning, got {lines:?}");
        };
        let prefix = format!("[git] git status failed in {}: ", missing.display());
        assert!(logged.starts_with(&prefix), "{logged}");
        assert!(logged.contains("Git command failed"), "{logged}");
        assert!(!logged.contains("--porcelain"), "args leaked: {logged}");

        let (second, repeat_lines) = crate::debug::capture_warnings(run);
        assert!(second.is_err());
        assert!(
            repeat_lines.is_empty(),
            "repeat within 60s must be silent: {repeat_lines:?}"
        );
    }

    #[test]
    fn detail_keeps_the_first_line_and_truncates() {
        assert_eq!(detail("first\nsecond"), "first");
        assert_eq!(detail(&"x".repeat(500)).chars().count(), MAX_DETAIL_CHARS);
    }

    #[test]
    fn line_names_repo_subcommand_and_suppressed_count() {
        let repo = Path::new("/repo");
        assert_eq!(
            line(
                repo,
                "status",
                "Git command failed (exit status: 128): fatal",
                0
            ),
            "git status failed in /repo: Git command failed (exit status: 128): fatal"
        );
        assert_eq!(
            line(repo, "status", "boom", 3),
            "git status failed in /repo: boom (3 identical failures suppressed in the last 60s)"
        );
    }
}

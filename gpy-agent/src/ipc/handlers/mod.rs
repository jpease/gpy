//! IPC Request Handler Infrastructure
//!
//! This module provides a trait-based handler system for different types of IPC requests.
//! By separating handlers into distinct modules, we achieve:
//!
//! - **Better testability** - Each handler can be tested in isolation
//! - **Clear responsibilities** - Each handler focuses on one domain
//! - **Easier maintenance** - Changes to one handler don't affect others
//! - **Dependency injection** - Handlers receive only what they need
//!
//! # Architecture
//!
//! Each handler module implements request processing for a specific domain:
//!
//! - **`git_handler.rs`** - Git repository status operations
//! - **`language_handler.rs`** - Programming language detection
//! - **`theme_handler.rs`** - Theme queries and configuration
//! - **`client_handler.rs`** - Client registration and workspace updates
//!
//! # Design Pattern
//!
//! All handlers implement the `RequestHandler` trait which provides a consistent
//! interface for processing IPC messages.

use crate::ipc::{ClientDirectory, Message, Response};
use crate::warn_log;
use std::collections::HashMap;
use std::hash::Hash;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

/// Why a handler refused or failed a request.
///
/// Replaces the bare `String` the handlers used to return (#600): the error a
/// handler produces is now inspectable by variant instead of only greppable by
/// substring. It stays *internal* — `route_request_secure` stringifies it into
/// the wire-visible `Response::Error { message }`, whose shape is unchanged —
/// so every `Display` arm below reproduces its message byte-for-byte.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// This handler was dispatched a message variant it does not serve.
    ///
    /// `qualifier` carries each handler's own wording ("non-git",
    /// "unsupported", …) so the message text is preserved exactly.
    #[error("{handler} received {qualifier} message: {message}")]
    UnexpectedMessage {
        /// The handler that received the message, e.g. `GitHandler`.
        handler: &'static str,
        /// How that handler describes a message it does not serve.
        qualifier: &'static str,
        /// The debug-formatted message.
        message: String,
    },

    /// A segment this request needs is turned off in config.
    #[error("{0} segment disabled via config")]
    FeatureDisabled(&'static str),

    /// The request's path is not inside a git repository.
    #[error("Not in a git repository")]
    NotInRepository,

    /// The named operation exceeded its configured timeout.
    #[error("{0} timed out")]
    TimedOut(&'static str),

    /// The request named a PID that never registered.
    #[error("PID {0} not registered - please register first")]
    PidNotRegistered(u32),

    /// A git status computation failed.
    ///
    /// Carries the already-stringified job result, which is also how a panicked
    /// background status job surfaces.
    #[error("Git error: {0}")]
    Git(String),

    /// A named sub-operation of the request failed, carrying the upstream error.
    ///
    /// `label` names the step ("Config reload", "Watcher unregister") so the
    /// message says which half of a multi-step handler gave way.
    #[error("{label} failed: {source}")]
    OperationFailed {
        /// The step that failed, in the wording the message has always used.
        label: &'static str,
        /// The upstream failure.
        source: crate::Error,
    },

    /// A theme query asked for a key the theme schema does not define.
    #[error("Unknown theme key: {key}. Valid keys: {valid_keys}")]
    UnknownThemeKey {
        /// The key the client asked for.
        key: String,
        /// Comma-separated list of keys that would have been accepted.
        valid_keys: String,
    },
}

/// Trait for handling specific types of IPC requests
pub trait RequestHandler: Send + Sync {
    /// Process an IPC message and return a response
    ///
    /// # Errors
    ///
    /// Returns an error if the message type is unsupported by this handler or
    /// if processing the message fails.
    fn handle(&self, message: &Message) -> Result<Response, HandlerError>;
    /// Get the handler name for debugging and logging
    fn name(&self) -> &'static str;
}

/// Shared dependencies needed to write the instant-prompt cache and repaint
/// live shells on a real content change (#587).
///
/// Every publish site runs the same sequence — read config, theme and palette,
/// write the domain-specific cache, then notify-or-log on the result — and each
/// used to thread these same five handles through its own parameter list. The
/// handlers now hold one `RenderDeps` instead of five separate fields, and the
/// background jobs they spawn capture a clone of it as a unit.
#[derive(Clone)]
pub struct RenderDeps {
    /// Live configuration. Re-read on every publish so a hot reload is picked up.
    pub config_manager: Arc<crate::config::manager::ConfigManager>,
    /// Active theme, shared with the agent event loop rather than reloaded from disk.
    pub theme_manager: Arc<crate::theme::ThemeManager>,
    /// Cached active palette, shared with the agent event loop so a `palette use`
    /// reload is reflected without re-parsing the palette TOML on every render
    /// (the render must use the same palette `active_palette(config)` would produce).
    pub palette_cache: Arc<crate::palette::PaletteCache>,
    /// On-disk cache the shell reads to render a prompt without any IPC round trip.
    pub instant_cache: Arc<crate::cache::InstantPromptCache>,
    /// Live shells to repaint (SIGUSR1) when a write produces user-visibly
    /// different output. Required so a background refresh does not strand
    /// updated status until the next keystroke (#160).
    pub client_registry: Arc<ClientDirectory>,
}

/// Repaint live shells with a forced SIGUSR1 when `write_result` reports the
/// rendered output actually changed; log a warning (never fail the request) on
/// error.
///
/// The write itself is domain-specific — git status and detected languages have
/// different shapes — so callers perform their own `instant_cache.write_*` call
/// and hand the `Result` here. This is the one place the "gate on change,
/// notify-or-log" wrapper around that result lives (#587). The change gate keeps
/// no-op refreshes silent, preserving the clock-gated broadcast behavior from
/// #145/#146.
pub(crate) fn notify_if_changed(
    client_registry: &ClientDirectory,
    root: &Path,
    write_result: crate::Result<bool>,
    context: &str,
) {
    match write_result {
        Ok(true) => client_registry.notify_sigusr1_force(Some(root)),
        Ok(false) => {}
        // Log but don't fail - the instant cache is an optimization, not critical.
        Err(e) => warn_log!(
            "cache",
            "Failed to write {context} instant-prompt cache: {e}"
        ),
    }
}

/// Run `job` off the calling thread: on a tokio runtime, as a blocking task;
/// off one, on a plain OS thread.
///
/// Both the async server path and any synchronous CLI/oneshot caller need "run
/// this without blocking the caller", and each background-job spawn site used
/// to carry its own copy of the try-current-runtime-or-fall-back-to-thread
/// check (#587).
pub(crate) fn spawn_detached<F>(job: F)
where
    F: FnOnce() + Send + 'static,
{
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn_blocking(job);
    } else {
        std::thread::spawn(job);
    }
}

/// Shared result slot for a single in-flight computation.
pub(super) struct SharedJob<V> {
    result: Mutex<Option<V>>,
    ready: Condvar,
}

impl<V> SharedJob<V> {
    const fn new() -> Self {
        Self {
            result: Mutex::new(None),
            ready: Condvar::new(),
        }
    }

    /// Complete the job and wake all waiters.
    pub(super) fn complete(&self, value: V) {
        if let Ok(mut result) = self.result.lock() {
            *result = Some(value);
            self.ready.notify_all();
        }
    }

    /// Wait until the job completes.
    ///
    /// Test-only: production callers always bound their waits with
    /// [`SharedJob::wait_timeout`] so a stuck job can never block indefinitely
    /// (#154).
    #[cfg(test)]
    pub(super) fn wait(&self) -> Option<V>
    where
        V: Clone,
    {
        let mut result = self.result.lock().ok()?;
        loop {
            if let Some(value) = result.as_ref() {
                return Some(value.clone());
            }
            result = self.ready.wait(result).ok()?;
        }
    }

    /// Wait until the job completes or the timeout expires.
    ///
    /// Uses `wait_timeout_while` rather than a single `wait_timeout` call: a
    /// condvar wait can wake spuriously before the job actually completes and
    /// before the full `timeout` has elapsed. A single call would then return
    /// `None` and be mistaken for a real timeout; `wait_timeout_while` re-checks
    /// the predicate on every wakeup and keeps waiting until either the job
    /// completes or the deadline genuinely passes (#323).
    #[expect(
        clippy::significant_drop_tightening,
        reason = "clippy's suggested fix (drop(result_guard) after the wait_timeout_while call) doesn't apply -- result_guard is moved by value into wait_timeout_while and no longer exists afterward"
    )]
    pub(super) fn wait_timeout(&self, timeout: Duration) -> Option<V>
    where
        V: Clone,
    {
        let result_guard = self.result.lock().ok()?;
        let (final_guard, _timed_out) = self
            .ready
            .wait_timeout_while(result_guard, timeout, |result| result.is_none())
            .ok()?;
        final_guard.clone()
    }
}

pub(super) type JobMap<K, V> = Arc<Mutex<HashMap<K, Arc<SharedJob<V>>>>>;

/// RAII handle for a single-flight job's completion.
///
/// `get_or_start` hands one of these to the `start` closure instead of the
/// raw `(job, jobs, key)` triple. The closure is expected to move the guard
/// into the spawned work and call [`JobGuard::finish`] with the real result
/// once it is available.
///
/// If the spawned work panics (or otherwise drops the guard) before calling
/// `finish`, `Drop` completes the job with the configured fallback value and
/// removes the map entry. Without this, a job that panics mid-computation
/// (e.g. `spawn_blocking`/`std::thread::spawn`, whose `JoinHandle` nobody
/// awaits) never calls `complete`/removes its map entry, so the slot for that
/// key stays permanently poisoned: every later request joins the same dead
/// job and waits out the full timeout, forever, until a daemon restart (#318).
pub(super) struct JobGuard<K, V>
where
    K: Eq + Hash,
{
    shared_job: Arc<SharedJob<V>>,
    jobs: JobMap<K, V>,
    key: Option<K>,
    fallback: Option<V>,
}

impl<K, V> JobGuard<K, V>
where
    K: Eq + Hash,
{
    /// Complete the job with `value` and remove the map entry.
    ///
    /// Consumes `self` so `Drop`'s fallback path can never also fire
    /// afterward.
    ///
    /// The map entry goes first, then the completion. See [`JobGuard`]'s
    /// `Drop` for why that order is load-bearing (#531).
    pub(super) fn finish(mut self, value: V) {
        self.fallback = None;
        if let Some(key) = self.key.take()
            && let Ok(mut jobs) = self.jobs.lock()
        {
            jobs.remove(&key);
        }
        self.shared_job.complete(value);
    }
}

impl<K, V> Drop for JobGuard<K, V>
where
    K: Eq + Hash,
{
    /// Free the slot first, complete the job second.
    ///
    /// The order is the whole guarantee. Completing first published the
    /// fallback while the map still held the dead entry, so a caller that
    /// woke from `wait_timeout` and immediately re-requested the same key
    /// could still find that entry and rejoin the job it had just watched
    /// die — the exact poisoning #318 added this guard to prevent, in a
    /// window a few instructions wide. Removing the entry before the
    /// completion is observable means anyone who has seen the result is
    /// guaranteed to find the slot already free (#531).
    ///
    /// A `get_or_start` landing between the two steps starts a fresh job,
    /// which is correct: waiters already holding the old `Arc` still receive
    /// this value.
    fn drop(&mut self) {
        // A no-op if `finish` already ran: it clears `fallback` and takes
        // `key`, so both branches below are skipped.
        if let Some(key) = self.key.take()
            && let Ok(mut jobs) = self.jobs.lock()
        {
            jobs.remove(&key);
        }
        if let Some(fallback) = self.fallback.take() {
            self.shared_job.complete(fallback);
        }
    }
}

/// Coalesces duplicate concurrent computations for the same key.
pub(super) struct SingleFlight<K, V> {
    jobs: JobMap<K, V>,
}

impl<K, V> SingleFlight<K, V>
where
    K: Clone + Eq + Hash,
{
    pub(super) fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Return an existing in-flight job, or start one for `key`.
    ///
    /// `fallback` is the value used to complete the job (and free the slot)
    /// if the `start` closure's spawned work panics before calling
    /// [`JobGuard::finish`] — see [`JobGuard`] for why this guarantee matters.
    pub(super) fn get_or_start<F>(&self, key: K, fallback: V, start: F) -> Arc<SharedJob<V>>
    where
        F: FnOnce(JobGuard<K, V>),
    {
        let job = match self.jobs.lock() {
            Ok(mut jobs) => {
                if let Some(job) = jobs.get(&key) {
                    return Arc::clone(job);
                }

                let job = Arc::new(SharedJob::new());
                jobs.insert(key.clone(), Arc::clone(&job));
                job
            }
            Err(_) => Arc::new(SharedJob::new()),
        };

        let guard = JobGuard {
            shared_job: Arc::clone(&job),
            jobs: Arc::clone(&self.jobs),
            key: Some(key),
            fallback: Some(fallback),
        };
        start(guard);
        job
    }
}

impl<K, V> Default for SingleFlight<K, V>
where
    K: Clone + Eq + Hash,
{
    fn default() -> Self {
        Self::new()
    }
}

/// Registry of all request handlers
pub struct HandlerRegistry {
    git: Arc<dyn RequestHandler>,
    language: Arc<dyn RequestHandler>,
    theme: Arc<dyn RequestHandler>,
    client: Arc<dyn RequestHandler>,
    misc: Arc<dyn RequestHandler>,
}

impl HandlerRegistry {
    /// Create a new handler registry with all required handlers
    pub fn new(
        git: Arc<dyn RequestHandler>,
        language: Arc<dyn RequestHandler>,
        theme: Arc<dyn RequestHandler>,
        client: Arc<dyn RequestHandler>,
        misc: Arc<dyn RequestHandler>,
    ) -> Self {
        Self {
            git,
            language,
            theme,
            client,
            misc,
        }
    }

    /// Route a message to the appropriate handler based on message type
    ///
    /// # Errors
    ///
    /// Returns an error if the handler for the message type fails to process the request.
    pub fn route(&self, message: &Message) -> Result<Response, HandlerError> {
        match message {
            Message::RepositoryStatus { .. } => self.git.handle(message),
            Message::LanguageDetect { .. } => self.language.handle(message),
            Message::DirectoryRequest { .. }
            | Message::DurationRequest { .. }
            | Message::CharacterRequest { .. }
            | Message::HostnameRequest { .. }
            | Message::UsernameRequest { .. } => self.misc.handle(message),
            Message::ThemeQuery { .. } => self.theme.handle(message),
            Message::Ping
            | Message::Status
            | Message::RegisterClient { .. }
            | Message::UnregisterClient { .. }
            | Message::WorkspaceUpdate { .. }
            | Message::Shutdown
            | Message::ConfigReload
            | Message::LatencyStats => self.client.handle(message),
        }
    }
}

/// Client lifecycle and management handler
pub mod client_handler;
/// Git repository status handler
pub mod git_handler;
/// Programming language detection handler
pub mod language_handler;
/// Handler for small, self-contained segment requests (directory/duration/character/hostname/username)
pub mod misc_handler;
/// Theme query handler
pub mod theme_handler;

pub use client_handler::ClientHandler;
pub use git_handler::GitHandler;
pub use language_handler::LanguageHandler;
pub use misc_handler::MiscHandler;
pub use theme_handler::ThemeHandler;

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::{HandlerError, SharedJob, SingleFlight};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier};
    use std::time::{Duration, Instant};

    // `HandlerError` is stringified straight into the wire-visible
    // `Response::Error { message }`, so each arm below pins the exact text a
    // shell sees. A change to any of these strings is a change to the IPC
    // payload, not a cosmetic edit (#600).

    #[test]
    fn unexpected_message_renders_each_handlers_own_wording() {
        let err = HandlerError::UnexpectedMessage {
            handler: "GitHandler",
            qualifier: "non-git",
            message: "Ping".to_owned(),
        };

        assert_eq!(err.to_string(), "GitHandler received non-git message: Ping");
    }

    #[test]
    fn feature_disabled_renders_the_config_wording() {
        assert_eq!(
            HandlerError::FeatureDisabled("Git").to_string(),
            "Git segment disabled via config"
        );
    }

    #[test]
    fn not_in_repository_renders_unchanged() {
        assert_eq!(
            HandlerError::NotInRepository.to_string(),
            "Not in a git repository"
        );
    }

    #[test]
    fn timed_out_renders_both_operations() {
        assert_eq!(
            HandlerError::TimedOut("Git status").to_string(),
            "Git status timed out"
        );
        assert_eq!(
            HandlerError::TimedOut("Language detection").to_string(),
            "Language detection timed out"
        );
    }

    #[test]
    fn pid_not_registered_renders_with_the_pid() {
        assert_eq!(
            HandlerError::PidNotRegistered(4321).to_string(),
            "PID 4321 not registered - please register first"
        );
    }

    #[test]
    fn git_renders_the_upstream_text_behind_a_prefix() {
        assert_eq!(
            HandlerError::Git("git status job panicked".to_owned()).to_string(),
            "Git error: git status job panicked"
        );
    }

    #[test]
    fn operation_failed_names_the_step_and_the_cause() {
        let err = HandlerError::OperationFailed {
            label: "Config reload",
            source: crate::Error::config("missing field"),
        };

        assert_eq!(
            err.to_string(),
            "Config reload failed: Configuration error: missing field"
        );
    }

    #[test]
    fn unknown_theme_key_lists_the_valid_keys() {
        let err = HandlerError::UnknownThemeKey {
            key: "nonsuch".to_owned(),
            valid_keys: "a, b".to_owned(),
        };

        assert_eq!(
            err.to_string(),
            "Unknown theme key: nonsuch. Valid keys: a, b"
        );
    }

    /// A cold cache miss must never wait forever.
    ///
    /// `wait_timeout` is the bounded primitive that backs git/language request
    /// timeouts (#154). When the job never completes, it returns `None` promptly
    /// instead of blocking.
    #[test]
    fn wait_timeout_returns_none_without_blocking_indefinitely() {
        let job = Arc::new(SharedJob::<usize>::new());
        let started = Instant::now();

        let result = job.wait_timeout(Duration::from_millis(50));

        assert!(
            result.is_none(),
            "wait_timeout must return None when the job never completes"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "wait_timeout must return shortly after the deadline, not hang"
        );
    }

    /// When the job completes before the deadline, `wait_timeout` returns the value.
    #[test]
    fn wait_timeout_returns_value_when_job_completes_before_deadline() {
        let job = Arc::new(SharedJob::<usize>::new());
        let background = Arc::clone(&job);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(10));
            background.complete(7);
        });

        let result = job.wait_timeout(Duration::from_secs(5));

        assert_eq!(result, Some(7));
    }

    #[test]
    fn single_flight_coalesces_concurrent_jobs_for_same_key() {
        let shared_single_flight = Arc::new(SingleFlight::<String, usize>::new());
        let start_count = Arc::new(AtomicUsize::new(0));
        let waiters = 8;
        let start_barrier = Arc::new(Barrier::new(waiters));

        let mut threads = Vec::new();
        for _ in 0..waiters {
            let worker_single_flight = Arc::clone(&shared_single_flight);
            let worker_start_count = Arc::clone(&start_count);
            let worker_barrier = Arc::clone(&start_barrier);

            threads.push(std::thread::spawn(move || {
                worker_barrier.wait();
                let job = worker_single_flight.get_or_start("repo".to_owned(), 0, move |guard| {
                    worker_start_count.fetch_add(1, Ordering::SeqCst);
                    std::thread::spawn(move || {
                        std::thread::sleep(Duration::from_millis(25));
                        guard.finish(42);
                    });
                });

                job.wait()
            }));
        }

        let results = threads
            .into_iter()
            .map(|thread| match thread.join() {
                Ok(result) => result,
                Err(error) => std::panic::resume_unwind(error),
            })
            .collect::<Vec<_>>();

        assert_eq!(start_count.load(Ordering::SeqCst), 1);
        assert!(results.iter().all(|result| *result == Some(42)));
    }

    /// Reproduces #318: a panic inside the spawned job body (before `finish`
    /// runs) must not permanently poison the single-flight slot.
    ///
    /// Proves both halves of the acceptance criterion:
    /// (a) the in-flight `SharedJob` degrades to the fallback value promptly
    ///     instead of hanging until the caller's own timeout, and
    /// (b) a second `get_or_start` for the *same key*, issued once the
    ///     fallback has been observed, starts a genuinely fresh job rather
    ///     than rejoining the dead one.
    ///
    /// (b) used to be a race, and flaked on `ubuntu-latest` and
    /// `windows-latest` alike (#531): `Drop` published the fallback *before*
    /// removing the map entry, so `wait_timeout` could return while the dead
    /// entry was still there and `start_count` stayed at 1. `Drop` now frees
    /// the slot first, which makes observing the fallback sufficient to
    /// guarantee the slot is free — the assertion below is ordered by the
    /// guard's own memory ordering rather than by how fast a panic unwinds.
    ///
    /// Against the pre-guard code, (a) hangs until `wait_timeout`'s deadline
    /// and returns `None` (never the fallback), and (b) fails because
    /// `get_or_start` returns the existing (dead) map entry early, so the
    /// second call's `start` closure never runs and `start_count` stays at 1.
    #[test]
    fn panic_in_spawned_job_does_not_poison_slot_for_same_key() {
        const FALLBACK: usize = 999;

        let single_flight = SingleFlight::<String, usize>::new();
        let start_count = Arc::new(AtomicUsize::new(0));

        let first_job = single_flight.get_or_start("repo".to_owned(), FALLBACK, {
            let first_start_count = Arc::clone(&start_count);
            move |guard| {
                first_start_count.fetch_add(1, Ordering::SeqCst);
                // Fire-and-forget, mirroring production's spawn_blocking/
                // std::thread::spawn pattern: nobody joins this handle, so a
                // panic here never propagates into the test process.
                std::thread::spawn(move || {
                    let _guard = guard;
                    panic!("injected panic");
                });
            }
        });

        // (a) The guard's Drop must degrade the job to the fallback value
        // promptly, well inside a generous bound, not hang until a
        // production-sized timeout.
        let started = Instant::now();
        assert_eq!(
            first_job.wait_timeout(Duration::from_secs(5)),
            Some(FALLBACK),
            "a panicked job must complete with the fallback value, not hang"
        );
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "the fallback must arrive from the panic unwind, not the timeout deadline"
        );

        // (b) A subsequent request for the same key must start a fresh job.
        let second_job = single_flight.get_or_start("repo".to_owned(), FALLBACK, {
            let second_start_count = Arc::clone(&start_count);
            move |guard| {
                second_start_count.fetch_add(1, Ordering::SeqCst);
                guard.finish(7);
            }
        });

        assert_eq!(
            start_count.load(Ordering::SeqCst),
            2,
            "a request after a panicked job must start a fresh job for the same key"
        );
        assert_eq!(second_job.wait_timeout(Duration::from_secs(5)), Some(7));
    }

    #[test]
    fn single_flight_runs_separate_jobs_for_separate_keys() {
        let single_flight = SingleFlight::<String, usize>::new();
        let start_count = Arc::new(AtomicUsize::new(0));

        let first = single_flight.get_or_start("first".to_owned(), 0, {
            let first_start_count = Arc::clone(&start_count);
            move |guard| {
                first_start_count.fetch_add(1, Ordering::SeqCst);
                guard.finish(1);
            }
        });
        let second = single_flight.get_or_start("second".to_owned(), 0, {
            let second_start_count = Arc::clone(&start_count);
            move |guard| {
                second_start_count.fetch_add(1, Ordering::SeqCst);
                guard.finish(2);
            }
        });

        assert_eq!(start_count.load(Ordering::SeqCst), 2);
        assert_eq!(first.wait(), Some(1));
        assert_eq!(second.wait(), Some(2));
    }
}

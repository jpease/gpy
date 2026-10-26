//! IPC request handlers, one per domain (git, language, client, misc), behind
//! the [`RequestHandler`] trait.

use crate::ipc::{ClientDirectory, Message, Response};
use crate::sync_ext::MutexExt;
use crate::warn_log;
use std::collections::HashMap;
use std::hash::Hash;
use std::path::Path;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

/// Why a handler refused or failed a request.
///
/// Stringified into the wire-visible `Response::Error { message }`, so each
/// `Display` arm's text is part of the IPC payload.
#[derive(Debug, thiserror::Error)]
pub enum HandlerError {
    /// This handler was dispatched a message variant it does not serve.
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

    /// A git status computation failed (including a panicked background job).
    #[error("Git error: {0}")]
    Git(String),

    /// A named sub-operation of the request failed.
    #[error("{label} failed: {source}")]
    OperationFailed {
        /// The step that failed, e.g. "Config reload".
        label: &'static str,
        /// The upstream failure.
        source: crate::Error,
    },
}

/// Handles one domain of IPC requests.
pub trait RequestHandler: Send + Sync {
    /// Process an IPC message and return a response.
    ///
    /// # Errors
    /// Fails if the message is unsupported by this handler or processing fails.
    fn handle(&self, message: &Message) -> Result<Response, HandlerError>;
}

/// Shared handles needed to write the instant-prompt cache and repaint live
/// shells on a real content change.
#[derive(Clone)]
pub struct RenderDeps {
    /// Live configuration, re-read on every publish to pick up hot reloads.
    pub config_manager: Arc<crate::config::manager::ConfigManager>,
    /// Active theme, shared with the agent event loop.
    pub theme_manager: Arc<crate::theme::ThemeManager>,
    /// Cached active palette, shared with the agent event loop.
    pub palette_cache: Arc<crate::palette::PaletteCache>,
    /// On-disk cache the shell reads without an IPC round trip.
    pub instant_cache: Arc<crate::cache::InstantPromptCache>,
    /// Live shells to repaint (SIGURG) when output changes.
    pub client_registry: Arc<ClientDirectory>,
}

/// Repaint live shells when `write_result` reports changed output; log (never
/// fail) on error. The change gate keeps no-op refreshes from waking terminals.
pub(crate) fn notify_if_changed(
    client_registry: &ClientDirectory,
    root: &Path,
    write_result: crate::Result<bool>,
    context: &str,
) {
    match write_result {
        Ok(true) => client_registry.notify_repaint_force(Some(root)),
        Ok(false) => {}
        // The instant cache is an optimization; don't fail the request.
        Err(e) => warn_log!(
            "cache",
            "Failed to write {context} instant-prompt cache: {e}"
        ),
    }
}

/// Run `job` off the calling thread: a blocking task on a tokio runtime, else
/// a plain OS thread.
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
        *self.result.lock_or_recover() = Some(value);
        self.ready.notify_all();
    }

    /// Wait until the job completes or the timeout expires.
    ///
    /// `wait_timeout_while` re-checks the predicate so spurious condvar wakeups
    /// aren't mistaken for a timeout.
    pub(super) fn wait_timeout(&self, timeout: Duration) -> Option<V>
    where
        V: Clone,
    {
        let result_guard = self.result.lock_or_recover();
        let (final_guard, _timed_out) = self
            .ready
            .wait_timeout_while(result_guard, timeout, |result| result.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        final_guard.clone()
    }
}

pub(super) type JobMap<K, V> = Arc<Mutex<HashMap<K, Arc<SharedJob<V>>>>>;

/// RAII handle for a single-flight job's completion.
///
/// If the spawned work drops this without calling [`JobGuard::finish`] (e.g.
/// panics), `Drop` completes the job with the fallback and frees the map slot;
/// otherwise the key would stay poisoned until daemon restart.
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
    /// Complete the job with `value` and remove the map entry (entry first; see `Drop`).
    pub(super) fn finish(mut self, value: V) {
        self.fallback = None;
        if let Some(key) = self.key.take() {
            self.jobs.lock_or_recover().remove(&key);
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
    /// Order matters: anyone who has observed the result must find the slot
    /// already free, or a re-request could rejoin the dead job.
    fn drop(&mut self) {
        // No-op if `finish` already ran.
        if let Some(key) = self.key.take() {
            self.jobs.lock_or_recover().remove(&key);
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
    /// `fallback` completes the job if the spawned work drops its [`JobGuard`].
    pub(super) fn get_or_start<F>(&self, key: K, fallback: V, start: F) -> Arc<SharedJob<V>>
    where
        F: FnOnce(JobGuard<K, V>),
    {
        let job = {
            let mut jobs = self.jobs.lock_or_recover();
            if let Some(job) = jobs.get(&key) {
                return Arc::clone(job);
            }

            let job = Arc::new(SharedJob::new());
            jobs.insert(key.clone(), Arc::clone(&job));
            job
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

/// Registry of all request handlers.
pub struct HandlerRegistry {
    git: Arc<dyn RequestHandler>,
    language: Arc<dyn RequestHandler>,
    client: Arc<dyn RequestHandler>,
    misc: Arc<dyn RequestHandler>,
}

impl HandlerRegistry {
    /// Create a registry with all handlers.
    pub fn new(
        git: Arc<dyn RequestHandler>,
        language: Arc<dyn RequestHandler>,
        client: Arc<dyn RequestHandler>,
        misc: Arc<dyn RequestHandler>,
    ) -> Self {
        Self {
            git,
            language,
            client,
            misc,
        }
    }

    /// Route a message to the handler for its type.
    ///
    /// # Errors
    /// Fails if the handler fails to process the request.
    pub fn route(&self, message: &Message) -> Result<Response, HandlerError> {
        match message {
            Message::RepositoryStatus { .. } => self.git.handle(message),
            Message::LanguageDetect { .. } => self.language.handle(message),
            Message::DirectoryRequest { .. }
            | Message::ClockRequest { .. }
            | Message::DurationRequest { .. }
            | Message::CharacterRequest { .. }
            | Message::HostnameRequest { .. }
            | Message::UsernameRequest { .. }
            | Message::ExitStatusRequest { .. } => self.misc.handle(message),
            Message::Ping
            | Message::Status
            | Message::RegisterClient { .. }
            | Message::UnregisterClient { .. }
            | Message::WorkspaceUpdate { .. }
            | Message::Shutdown
            | Message::ConfigReload => self.client.handle(message),
        }
    }
}

/// Client lifecycle handler.
pub mod client_handler;
/// Git status handler.
pub mod git_handler;
/// Language detection handler.
pub mod language_handler;
/// Small self-contained segments (directory, duration, character, hostname, username).
pub mod misc_handler;

pub use client_handler::ClientHandler;
pub use git_handler::GitHandler;
pub use language_handler::LanguageHandler;
pub use misc_handler::MiscHandler;

#[cfg(test)]
#[allow(clippy::missing_panics_doc)]
mod tests {
    use super::{HandlerError, SharedJob, SingleFlight};
    use crate::sync_ext::MutexExt;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Arc, Barrier, PoisonError};
    use std::time::{Duration, Instant};

    impl<V: Clone> SharedJob<V> {
        /// Wait until the job completes, however long it takes.
        fn wait(&self) -> V {
            let mut result = self.result.lock_or_recover();
            loop {
                if let Some(value) = result.as_ref() {
                    return value.clone();
                }
                result = self
                    .ready
                    .wait(result)
                    .unwrap_or_else(PoisonError::into_inner);
            }
        }
    }

    // These strings are the wire-visible `Response::Error` text.

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

    /// A cold cache miss must never wait forever.
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

    /// A job completing before the deadline returns its value.
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
        assert!(results.iter().all(|result| *result == 42));
    }

    /// A panic before `finish` degrades the job to the fallback promptly, and
    /// a later request for the same key starts a fresh job.
    #[test]
    fn panic_in_spawned_job_does_not_poison_slot_for_same_key() {
        const FALLBACK: usize = 999;

        let single_flight = SingleFlight::<String, usize>::new();
        let start_count = Arc::new(AtomicUsize::new(0));

        let first_job = single_flight.get_or_start("repo".to_owned(), FALLBACK, {
            let first_start_count = Arc::clone(&start_count);
            move |guard| {
                first_start_count.fetch_add(1, Ordering::SeqCst);
                // Fire-and-forget like production: nobody joins this handle.
                std::thread::spawn(move || {
                    let _guard = guard;
                    panic!("injected panic");
                });
            }
        });

        // (a) Degrades to the fallback promptly.
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
        assert_eq!(first.wait(), 1);
        assert_eq!(second.wait(), 2);
    }
}

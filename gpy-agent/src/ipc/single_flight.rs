//! Single-flight coalescing of duplicate concurrent computations.

use crate::sync_ext::MutexExt;
use std::collections::HashMap;
use std::hash::Hash;
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

/// Shared result slot for a single in-flight computation.
pub(crate) struct SharedJob<V> {
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
    pub(crate) fn complete(&self, value: V) {
        *self.result.lock_or_recover() = Some(value);
        self.ready.notify_all();
    }

    /// Wait until the job completes or the timeout expires.
    ///
    /// `wait_timeout_while` re-checks the predicate so spurious condvar wakeups
    /// aren't mistaken for a timeout.
    pub(crate) fn wait_timeout(&self, timeout: Duration) -> Option<V>
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

pub(crate) type JobMap<K, V> = Arc<Mutex<HashMap<K, Arc<SharedJob<V>>>>>;

/// RAII handle for a single-flight job's completion.
///
/// If the spawned work drops this without calling [`JobGuard::finish`] (e.g.
/// panics), `Drop` completes the job with the fallback and frees the map slot;
/// otherwise the key would stay poisoned until daemon restart.
pub(crate) struct JobGuard<K, V>
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
    pub(crate) fn finish(mut self, value: V) {
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
pub(crate) struct SingleFlight<K, V> {
    jobs: JobMap<K, V>,
}

impl<K, V> SingleFlight<K, V>
where
    K: Clone + Eq + Hash,
{
    pub(crate) fn new() -> Self {
        Self {
            jobs: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Whether no job is in flight.
    #[cfg(test)]
    pub(crate) fn is_idle(&self) -> bool {
        self.jobs.lock_or_recover().is_empty()
    }

    /// Return an existing in-flight job, or start one for `key`.
    ///
    /// `fallback` completes the job if the spawned work drops its [`JobGuard`].
    pub(crate) fn get_or_start<F>(&self, key: K, fallback: V, start: F) -> Arc<SharedJob<V>>
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

#[cfg(test)]
mod tests {
    use super::{SharedJob, SingleFlight};
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

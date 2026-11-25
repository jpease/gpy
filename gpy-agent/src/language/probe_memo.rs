//! Probe-result memo with a short negative cache.

use crate::cache::ttl_map::TtlMap;
use crate::language::version::{ColdVersion, ProbePolicy};
use crate::sync_ext::MutexExt;
use std::hash::Hash;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::{Duration, SystemTime};

/// How long a failed probe suppresses re-probing: avoids respawning a broken
/// tool every render while picking up a fixed one within a minute.
pub(crate) const FAILURE_TTL: Duration = Duration::from_mins(1);

/// Longest a lookup waits on another thread's probe of the same key. A probe
/// is bounded by its own subprocess timeouts (mise, then each direct
/// candidate); this only stops a lookup waiting forever on a wedged one.
const FOLLOWER_WAIT: Duration = Duration::from_secs(20);

/// A probe in progress for one key; concurrent lookups of that key wait on it
/// instead of spawning the same subprocess again.
struct Flight {
    done: Mutex<bool>,
    finished: Condvar,
    /// Set by [`ProbeMemo::retain`] when it drops this key mid-probe: the
    /// probe saw the state before the invalidation, so its result is not
    /// stored. Read and written under the memo's `flights` lock.
    invalidated: AtomicBool,
}

impl Flight {
    const fn new() -> Self {
        Self {
            done: Mutex::new(false),
            finished: Condvar::new(),
            invalidated: AtomicBool::new(false),
        }
    }

    /// Wait until the probe ends or `timeout` passes.
    fn wait(&self, timeout: Duration) {
        let guard = self.done.lock_or_recover();
        let (finished_guard, _timed_out) = self
            .finished
            .wait_timeout_while(guard, timeout, |finished| !*finished)
            .unwrap_or_else(PoisonError::into_inner);
        drop(finished_guard);
    }
}

/// Ends a [`Flight`] on drop, so a panicking probe still frees its key and
/// wakes its waiters.
struct FlightGuard<'a, K: Eq> {
    flights: &'a Mutex<Vec<(K, Arc<Flight>)>>,
    key: &'a K,
    flight: Arc<Flight>,
}

impl<K: Eq> Drop for FlightGuard<'_, K> {
    fn drop(&mut self) {
        self.flights
            .lock_or_recover()
            .retain(|(key, _)| key != self.key);
        *self.flight.done.lock_or_recover() = true;
        self.flight.finished.notify_all();
    }
}

/// Successful probe results by `K`, plus a negative cache of recent failures.
///
/// Successes live as long as the shared cache TTL; a failure only suppresses
/// the probe for [`FAILURE_TTL`].
pub(crate) struct ProbeMemo<K> {
    found: TtlMap<K, String>,
    failed: TtlMap<K, SystemTime>,
    flights: Mutex<Vec<(K, Arc<Flight>)>>,
}

impl<K: Eq + Hash + Clone> ProbeMemo<K> {
    /// A memo holding at most `capacity` successes and `capacity` failures.
    pub(crate) const fn new(capacity: usize) -> Self {
        Self {
            found: TtlMap::new(capacity),
            failed: TtlMap::new(capacity),
            flights: Mutex::new(Vec::new()),
        }
    }

    /// The memoized version for `key`, else the result of `probe`, which is
    /// memoized (success) or negatively cached (`None`). Concurrent lookups of
    /// one key run a single probe; the others wait for it and read its result.
    ///
    /// `Ok(None)` is no version or a recent failure.
    ///
    /// # Errors
    /// [`ColdVersion`] when nothing is memoized and `policy` forbids probing.
    pub(crate) fn lookup_or_probe(
        &self,
        key: &K,
        policy: ProbePolicy,
        probe: impl FnOnce() -> Option<String>,
    ) -> Result<Option<String>, ColdVersion> {
        if let Some(version) = self.found.get(key) {
            return Ok(Some(version));
        }
        if self.has_recent_failure(key) {
            return Ok(None);
        }
        if policy == ProbePolicy::CachedOnly {
            return Err(ColdVersion);
        }
        let flight = match self.claim(key) {
            Ok(flight) => flight,
            Err(running) => {
                running.wait(FOLLOWER_WAIT);
                return Ok(self.found.get(key));
            }
        };
        let version = probe();
        self.store(&flight, version.as_deref());
        Ok(version)
    }

    /// Memoize the probed version (a success) or record a failure (`None`) for
    /// the flight's key, unless [`Self::retain`] dropped the key while the
    /// probe ran. Under the `flights` lock, so a `retain` either is seen here
    /// or removes the entry.
    fn store(&self, flight: &FlightGuard<'_, K>, probed: Option<&str>) {
        let flights = self.flights.lock_or_recover();
        if flight.flight.invalidated.load(Ordering::Relaxed) {
            return;
        }
        let key = flight.key;
        if let Some(version) = probed {
            self.failed.remove(key);
            self.found.insert(key.clone(), version.to_owned());
        } else {
            self.failed.insert(key.clone(), SystemTime::now());
        }
        drop(flights);
    }

    /// Become the prober for `key`, or get the probe already running for it.
    fn claim<'a>(&'a self, key: &'a K) -> Result<FlightGuard<'a, K>, Arc<Flight>> {
        let mut flights = self.flights.lock_or_recover();
        if let Some((_, running)) = flights.iter().find(|(running_key, _)| running_key == key) {
            return Err(Arc::clone(running));
        }
        let flight = Arc::new(Flight::new());
        flights.push((key.clone(), Arc::clone(&flight)));
        drop(flights);
        Ok(FlightGuard {
            flights: &self.flights,
            key,
            flight,
        })
    }

    /// Whether a probe for `key` failed within [`FAILURE_TTL`]; a backwards
    /// clock counts as not recent.
    fn has_recent_failure(&self, key: &K) -> bool {
        self.failed.get(key).is_some_and(|recorded_at| {
            SystemTime::now()
                .duration_since(recorded_at)
                .is_ok_and(|elapsed| elapsed < FAILURE_TTL)
        })
    }

    /// Keep only the entries (successes and failures) whose key satisfies
    /// `keep`. A probe still running for a dropped key stores nothing: it saw
    /// the state before this invalidation.
    pub(crate) fn retain(&self, mut keep: impl FnMut(&K) -> bool) {
        let flights = self.flights.lock_or_recover();
        for (key, flight) in flights.iter() {
            if !keep(key) {
                flight.invalidated.store(true, Ordering::Relaxed);
            }
        }
        self.found.retain(&mut keep);
        self.failed.retain(keep);
        drop(flights);
    }
}

#[cfg(test)]
impl<K: Eq + Hash + Clone> ProbeMemo<K> {
    /// The success map.
    pub(crate) const fn found(&self) -> &TtlMap<K, String> {
        &self.found
    }

    /// The failure map.
    pub(crate) const fn failed(&self) -> &TtlMap<K, SystemTime> {
        &self.failed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    #[test]
    fn memoizes_success_and_skips_probe() {
        let memo = ProbeMemo::<u8>::new(8);
        let calls = Cell::new(0_u32);
        let probe = || {
            calls.set(calls.get().saturating_add(1));
            Some("1.0".to_owned())
        };
        assert_eq!(
            memo.lookup_or_probe(&1, ProbePolicy::Spawn, probe),
            Ok(Some("1.0".to_owned()))
        );
        assert_eq!(
            memo.lookup_or_probe(&1, ProbePolicy::Spawn, probe),
            Ok(Some("1.0".to_owned()))
        );
        assert_eq!(calls.get(), 1);
    }

    #[test]
    fn failure_suppresses_probe_until_it_ages_out() {
        let memo = ProbeMemo::<u8>::new(8);
        let calls = Cell::new(0_u32);
        let probe = || {
            calls.set(calls.get().saturating_add(1));
            None
        };
        assert_eq!(
            memo.lookup_or_probe(&1, ProbePolicy::Spawn, probe),
            Ok(None)
        );
        assert_eq!(
            memo.lookup_or_probe(&1, ProbePolicy::Spawn, probe),
            Ok(None)
        );
        assert_eq!(calls.get(), 1);

        memo.failed()
            .insert(1, SystemTime::now() - FAILURE_TTL - Duration::from_secs(1));
        assert_eq!(
            memo.lookup_or_probe(&1, ProbePolicy::Spawn, probe),
            Ok(None)
        );
        assert_eq!(calls.get(), 2);
    }

    /// An invalidation landing while a probe runs wins: the probe saw the old
    /// state, so neither its version nor its failure may be memoized.
    #[test]
    fn invalidation_during_a_probe_discards_its_result() {
        let memo = ProbeMemo::<u8>::new(8);
        let invalidate = |dropped: u8| memo.retain(|key| *key != dropped);

        let stale = memo.lookup_or_probe(&1, ProbePolicy::Spawn, || {
            invalidate(1);
            Some("1.0".to_owned())
        });
        let fresh = memo.lookup_or_probe(&1, ProbePolicy::Spawn, || Some("2.0".to_owned()));
        let failed = memo.lookup_or_probe(&2, ProbePolicy::Spawn, || {
            invalidate(2);
            None
        });
        let retried = memo.lookup_or_probe(&2, ProbePolicy::Spawn, || Some("3.0".to_owned()));

        assert_eq!(
            stale,
            Ok(Some("1.0".to_owned())),
            "its caller still gets it"
        );
        assert_eq!(
            fresh,
            Ok(Some("2.0".to_owned())),
            "a stale version was kept"
        );
        assert_eq!(failed, Ok(None));
        assert_eq!(
            retried,
            Ok(Some("3.0".to_owned())),
            "a stale failure was kept"
        );
    }

    #[test]
    fn cached_only_reports_cold_without_probing() {
        let memo = ProbeMemo::<u8>::new(8);
        let probed = Cell::new(false);
        let result = memo.lookup_or_probe(&1, ProbePolicy::CachedOnly, || {
            probed.set(true);
            None
        });
        assert_eq!(result, Err(ColdVersion));
        assert!(!probed.get(), "a cached-only lookup must not probe");
    }

    #[test]
    fn concurrent_lookups_of_one_key_probe_once() {
        use std::sync::atomic::{AtomicU32, Ordering};

        let memo = ProbeMemo::<u8>::new(8);
        let calls = AtomicU32::new(0);
        let probe = || {
            calls.fetch_add(1, Ordering::SeqCst);
            std::thread::sleep(Duration::from_millis(300));
            Some("1.0".to_owned())
        };
        let results = std::thread::scope(|scope| {
            let first = scope.spawn(|| memo.lookup_or_probe(&1, ProbePolicy::Spawn, probe));
            std::thread::sleep(Duration::from_millis(100));
            let second = scope.spawn(|| memo.lookup_or_probe(&1, ProbePolicy::Spawn, probe));
            (first.join(), second.join())
        });
        assert_eq!(results.0.expect("first"), Ok(Some("1.0".to_owned())));
        assert_eq!(results.1.expect("second"), Ok(Some("1.0".to_owned())));
        assert_eq!(calls.load(Ordering::SeqCst), 1);
    }
}

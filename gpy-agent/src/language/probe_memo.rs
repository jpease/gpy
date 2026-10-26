//! Probe-result memo with a short negative cache.

use crate::cache::ttl_map::TtlMap;
use crate::language::version::{ColdVersion, ProbePolicy};
use std::hash::Hash;
use std::time::{Duration, SystemTime};

/// How long a failed probe suppresses re-probing: avoids respawning a broken
/// tool every render while picking up a fixed one within a minute.
pub(crate) const FAILURE_TTL: Duration = Duration::from_secs(60);

/// Successful probe results by `K`, plus a negative cache of recent failures.
///
/// Successes live as long as the shared cache TTL; a failure only suppresses
/// the probe for [`FAILURE_TTL`].
pub(crate) struct ProbeMemo<K> {
    found: TtlMap<K, String>,
    failed: TtlMap<K, SystemTime>,
}

impl<K: Eq + Hash + Clone> ProbeMemo<K> {
    /// A memo holding at most `capacity` successes and `capacity` failures.
    pub(crate) const fn new(capacity: usize) -> Self {
        Self {
            found: TtlMap::new(capacity),
            failed: TtlMap::new(capacity),
        }
    }

    /// The memoized version for `key`, else the result of `probe`, which is
    /// memoized (success) or negatively cached (`None`).
    ///
    /// `Ok(None)` is no version or a recent failure.
    ///
    /// # Errors
    /// [`ColdVersion`] when nothing is memoized and `policy` forbids probing.
    pub(crate) fn lookup_or_probe(
        &self,
        key: K,
        policy: ProbePolicy,
        probe: impl FnOnce() -> Option<String>,
    ) -> Result<Option<String>, ColdVersion> {
        if let Some(version) = self.found.get(&key) {
            return Ok(Some(version));
        }
        if self.has_recent_failure(&key) {
            return Ok(None);
        }
        if policy == ProbePolicy::CachedOnly {
            return Err(ColdVersion);
        }
        if let Some(version) = probe() {
            self.failed.remove(&key);
            self.found.insert(key, version.clone());
            Ok(Some(version))
        } else {
            self.failed.insert(key, SystemTime::now());
            Ok(None)
        }
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

    /// Keep only the entries (successes and failures) whose key satisfies `keep`.
    pub(crate) fn retain(&self, mut keep: impl FnMut(&K) -> bool) {
        self.found.retain(&mut keep);
        self.failed.retain(keep);
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
            memo.lookup_or_probe(1, ProbePolicy::Spawn, probe),
            Ok(Some("1.0".to_owned()))
        );
        assert_eq!(
            memo.lookup_or_probe(1, ProbePolicy::Spawn, probe),
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
        assert_eq!(memo.lookup_or_probe(1, ProbePolicy::Spawn, probe), Ok(None));
        assert_eq!(memo.lookup_or_probe(1, ProbePolicy::Spawn, probe), Ok(None));
        assert_eq!(calls.get(), 1);

        memo.failed()
            .insert(1, SystemTime::now() - FAILURE_TTL - Duration::from_secs(1));
        assert_eq!(memo.lookup_or_probe(1, ProbePolicy::Spawn, probe), Ok(None));
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn cached_only_reports_cold_without_probing() {
        let memo = ProbeMemo::<u8>::new(8);
        let probed = Cell::new(false);
        let result = memo.lookup_or_probe(1, ProbePolicy::CachedOnly, || {
            probed.set(true);
            None
        });
        assert_eq!(result, Err(ColdVersion));
        assert!(!probed.get(), "a cached-only lookup must not probe");
    }
}

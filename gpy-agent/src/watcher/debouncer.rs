//! Event debouncing for file system changes
//!
//! Flow: notify → `FileSystemWatcher` → `DebounceEngine` → agent callback
//! 1. The `notify` crate emits raw filesystem events.
//! 2. `FileSystemWatcher` filters them via `should_trigger_update`, producing `PendingEvent` values.
//! 3. `DebounceEngine` groups pending events by `(event kind, repo)` over the configured window.
//! 4. Once the window elapses (trailing-edge quiet gap) or the max-delay cap is reached
//!    (whichever comes first), `drain_expired_events` removes and returns the events as
//!    `DebouncedEvent`. `DebounceEngine` never invokes a callback itself — see
//!    `watcher::WatchCoordinator` for why dispatch happens outside this engine's lock.

use super::{DebouncedEvent, FileEvent, PendingEvent};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Debouncer for file system events
pub struct DebounceEngine {
    /// Pending events by (kind, repo) key
    pending_events: HashMap<FileEventKey, PendingEntry>,
    /// Debounce duration: how long a key must be quiet before it flushes
    debounce_duration: Duration,
    /// Upper bound on how long a key may be held once first observed, even
    /// under continued churn. Prevents the trailing-edge quiet-gap check from
    /// postponing a flush indefinitely (#322).
    max_delay: Duration,
}

/// A pending event plus the timestamps needed to decide when to flush it.
struct PendingEntry {
    /// When this key was first observed (drives the max-delay cap).
    first_seen: Instant,
    /// When this key was last updated (drives the quiet-gap debounce).
    last_seen: Instant,
    /// Most recent event data for this key.
    pending: PendingEvent,
    /// Earliest instant this entry may be drained, or `None` for the ordinary
    /// quiet-gap/max-delay rule alone (#569). Set (and only ever raised, never
    /// lowered) by [`DebounceEngine::handle_event_due`] — used for the
    /// directory-create follow-up, so the flush thread `WatchCoordinator`
    /// already owns does the waiting instead of a dedicated timer thread per
    /// created directory. A plain [`DebounceEngine::handle_event`] call never
    /// touches this field, so an outstanding floor survives being merged with
    /// an ordinary event for the same key.
    not_before: Option<Instant>,
}

/// Key for grouping similar events for debouncing
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
enum FileEventKey {
    Git(PathBuf),
    Language(PathBuf),
    Config(PathBuf),
    Theme(PathBuf),
}

impl FileEventKey {
    fn from_pending(pending: &PendingEvent) -> Self {
        match pending.event {
            FileEvent::Git { .. } => Self::Git(pending.repo.clone()),
            FileEvent::Language { .. } => Self::Language(pending.repo.clone()),
            FileEvent::Config { .. } => Self::Config(pending.repo.clone()),
            FileEvent::Theme { .. } => Self::Theme(pending.repo.clone()),
        }
    }
}

/// Fold a newly observed event into the one already pending for its key.
///
/// Git events accumulate their paths: the agent turns them into a
/// pathspec-limited status scan, so dropping one path publishes a status short
/// by that file until the cache TTL or the reconcile backstop catches up
/// (#466). [`GitPaths::merge`] owns the dedup and the bound.
///
/// Every other kind keeps last-writer-wins. Their handlers refresh a whole
/// subsystem for the repository the key already pins — the language cache, the
/// theme cache — so the specific path adds nothing to retain.
fn merge_pending(existing: &mut PendingEvent, incoming: PendingEvent) {
    match (&mut existing.event, incoming.event) {
        (FileEvent::Git { paths: accumulated }, FileEvent::Git { paths: observed }) => {
            accumulated.merge(&observed);
        }
        (existing_event, incoming_event) => *existing_event = incoming_event,
    }
}

impl DebounceEngine {
    /// Create a new event debouncer
    #[must_use]
    pub fn new(debounce_duration: Duration) -> Self {
        Self {
            pending_events: HashMap::new(),
            debounce_duration,
            max_delay: Self::default_max_delay(),
        }
    }

    /// Override the max-delay cap (default: `default_max_delay()`). Mainly
    /// useful for tests that need a short cap to run quickly.
    #[must_use]
    pub const fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    /// Process a file system event (may be debounced). Keeps the first-seen
    /// timestamp for an existing key so the max-delay cap in
    /// `drain_expired_events` can be enforced regardless of continued churn,
    /// and folds the new event into the pending one via [`merge_pending`]
    /// rather than overwriting it (#466).
    pub fn handle_event(&mut self, pending: PendingEvent) {
        let key = FileEventKey::from_pending(&pending);
        let now = Instant::now();
        match self.pending_events.get_mut(&key) {
            Some(entry) => {
                entry.last_seen = now;
                merge_pending(&mut entry.pending, pending);
            }
            None => {
                self.pending_events.insert(
                    key,
                    PendingEntry {
                        first_seen: now,
                        last_seen: now,
                        pending,
                        not_before: None,
                    },
                );
            }
        }
    }

    /// Schedule `pending` for delivery no earlier than `due`, using the same
    /// per-key coalescing as [`DebounceEngine::handle_event`] (#569).
    ///
    /// If an entry already exists for this key — whether from a prior
    /// `handle_event` or `handle_event_due` call — this merges into it via the
    /// same [`merge_pending`] rule and keeps the LATER of any existing floor
    /// and `due`, so the last event in a burst still gets its full delay window
    /// from the moment it was observed.
    ///
    /// This is what replaced the per-directory timer thread the filesystem
    /// layer used to spawn for the directory-create follow-up: a burst of N
    /// created directories in one repository is N synchronous map updates
    /// collapsing into one pending entry, waited on by the flush thread the
    /// coordinator already owns, and cleared by `WatchCoordinator::stop`
    /// along with everything else pending (#569).
    pub fn handle_event_due(&mut self, pending: PendingEvent, due: Instant) {
        let key = FileEventKey::from_pending(&pending);
        let now = Instant::now();
        match self.pending_events.get_mut(&key) {
            Some(entry) => {
                entry.last_seen = now;
                entry.not_before = Some(entry.not_before.map_or(due, |existing| existing.max(due)));
                merge_pending(&mut entry.pending, pending);
            }
            None => {
                self.pending_events.insert(
                    key,
                    PendingEntry {
                        first_seen: now,
                        last_seen: now,
                        pending,
                        not_before: Some(due),
                    },
                );
            }
        }
    }

    /// Remove and return events that have crossed the debounce quiet-gap
    /// window or the max-delay cap. Does **not** invoke any callback — the
    /// caller is responsible for delivering the returned events, and must do
    /// so without holding this engine's lock, since delivery may run
    /// unbounded git/language work (#306).
    pub fn drain_expired_events(&mut self) -> Vec<DebouncedEvent> {
        let now = Instant::now();
        let debounce_duration = self.debounce_duration;
        let max_delay = self.max_delay;
        self.pending_events
            .iter()
            .filter(|&(_, entry)| {
                let debounce_elapsed = now.duration_since(entry.last_seen) >= debounce_duration
                    || now.duration_since(entry.first_seen) >= max_delay;
                // An AND-gate, and only ever for an entry that asked for one:
                // an entry with no floor keeps the pure quiet-gap/max-delay
                // rule, byte for byte (#569).
                let due_elapsed = entry.not_before.is_none_or(|due| now >= due);
                debounce_elapsed && due_elapsed
            })
            .map(|(key, _)| key.clone())
            .collect::<Vec<_>>()
            .into_iter()
            .filter_map(|key| self.pending_events.remove(&key))
            .map(|entry| DebouncedEvent {
                event: entry.pending.event,
                repo: entry.pending.repo,
            })
            .collect()
    }

    /// Clear all pending events
    pub fn clear(&mut self) {
        self.pending_events.clear();
    }

    /// Get recommended debounce duration for file watching
    #[must_use]
    pub const fn default_debounce_duration() -> Duration {
        Duration::from_millis(100) // 100ms debounce
    }

    /// Get the default max-delay cap: the longest a key may be held once
    /// first observed, even under continued churn (#322).
    #[must_use]
    pub const fn default_max_delay() -> Duration {
        Duration::from_secs(1)
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::missing_panics_doc)]
    use super::*;
    use crate::watcher::{FileEvent, GitPaths, PendingEvent};
    use std::path::{Path, PathBuf};
    use std::thread;

    fn make_pending(repo: &str, file: &str) -> PendingEvent {
        PendingEvent {
            event: FileEvent::Git {
                paths: GitPaths::single(PathBuf::from(file)),
            },
            repo: PathBuf::from(repo),
        }
    }

    /// The due-time floor recorded for a repository's Git key, or `None` when
    /// nothing is pending for it.
    ///
    /// Reaches into private state deliberately:
    /// `mod tests` is a child module, and the alternative is a production
    /// accessor no production caller wants.
    fn git_floor(engine: &DebounceEngine, repo: &str) -> Option<Instant> {
        engine
            .pending_events
            .get(&FileEventKey::Git(PathBuf::from(repo)))
            .and_then(|entry| entry.not_before)
    }

    /// Sleep until `deadline` has passed, plus a small margin so the drain that
    /// follows is unambiguously on the far side of it.
    fn sleep_past(deadline: Instant) {
        let now = Instant::now();
        if deadline > now {
            thread::sleep(deadline.duration_since(now));
        }
        thread::sleep(Duration::from_millis(5));
    }

    #[test]
    fn debouncer_keeps_events_per_repo() {
        let mut engine = DebounceEngine::new(Duration::from_millis(10));

        engine.handle_event(make_pending("/repo1", "/repo1/.git/HEAD"));
        engine.handle_event(make_pending("/repo2", "/repo2/.git/HEAD"));

        thread::sleep(Duration::from_millis(15));
        let flushed = engine.drain_expired_events();

        let repos: Vec<_> = flushed.into_iter().map(|event| event.repo).collect();
        assert_eq!(repos.len(), 2);
        assert!(repos.contains(&PathBuf::from("/repo1")));
        assert!(repos.contains(&PathBuf::from("/repo2")));
    }

    #[test]
    fn debouncer_collapses_events_same_repo() {
        let mut engine = DebounceEngine::new(Duration::from_millis(20));

        engine.handle_event(make_pending("/repo", "/repo/.git/HEAD"));
        thread::sleep(Duration::from_millis(5));
        engine.handle_event(make_pending("/repo", "/repo/.git/index"));

        thread::sleep(Duration::from_millis(25));
        let flushed = engine.drain_expired_events();

        assert_eq!(flushed.len(), 1);
    }

    #[test]
    fn drain_expired_events_never_invokes_a_callback() {
        // DebounceEngine has no callback storage at all — dispatch is the
        // caller's responsibility, off this engine's lock (#306).
        let mut engine = DebounceEngine::new(Duration::from_millis(1));
        engine.handle_event(make_pending("/repo", "/repo/.git/HEAD"));
        thread::sleep(Duration::from_millis(5));

        let flushed = engine.drain_expired_events();
        assert_eq!(flushed.len(), 1);
        // A second drain immediately after finds nothing left pending.
        assert!(engine.drain_expired_events().is_empty());
    }

    #[test]
    fn sustained_churn_still_forces_a_flush_within_max_delay() {
        // Regression test for #322: continuous events under the debounce
        // quiet-gap window used to reset the expiry forever. The max-delay
        // cap must force at least one flush despite unbroken churn.
        let mut engine = DebounceEngine::new(Duration::from_millis(20))
            .with_max_delay(Duration::from_millis(50));

        let start = Instant::now();
        let mut flushed_total = 0_usize;
        while start.elapsed() < Duration::from_millis(120) {
            engine.handle_event(make_pending("/repo", "/repo/.git/HEAD"));
            flushed_total += engine.drain_expired_events().len();
            thread::sleep(Duration::from_millis(5));
        }

        assert!(
            flushed_total >= 1,
            "max-delay cap should force a flush despite continued sub-debounce churn"
        );
    }

    #[test]
    fn quiet_gap_alone_still_flushes_when_under_max_delay() {
        // The max-delay cap must not change ordinary trailing-edge behavior
        // when a key simply goes quiet well before the cap is reached.
        let mut engine =
            DebounceEngine::new(Duration::from_millis(10)).with_max_delay(Duration::from_secs(5));

        engine.handle_event(make_pending("/repo", "/repo/.git/HEAD"));
        thread::sleep(Duration::from_millis(15));

        assert_eq!(engine.drain_expired_events().len(), 1);
    }

    #[test]
    /// Regression test for #466: two changes to different files in one debounce window must
    /// both survive.
    ///
    /// Used to keep only the last path, so the incremental scan that followed published a
    /// status short by one file until the cache TTL or the reconcile backstop caught up.
    ///
    fn same_repo_events_accumulate_every_path() {
        let mut engine = DebounceEngine::new(Duration::from_millis(10));

        engine.handle_event(make_pending("/repo", "/repo/a.txt"));
        engine.handle_event(make_pending("/repo", "/repo/b.txt"));

        thread::sleep(Duration::from_millis(15));
        let flushed = engine.drain_expired_events();

        assert_eq!(flushed.len(), 1);
        let first = flushed.first().expect("one flushed event");
        let FileEvent::Git { paths } = &first.event else {
            panic!("expected a git event");
        };
        assert_eq!(
            paths.as_slice(),
            Some([PathBuf::from("/repo/a.txt"), PathBuf::from("/repo/b.txt")].as_slice()),
            "both changed paths must survive coalescing, in the order observed"
        );
    }

    #[test]
    /// One editor save emits several events for the same file; accumulating
    /// them verbatim would spend the bound on duplicates.
    fn repeated_events_for_one_path_are_deduplicated() {
        let mut engine = DebounceEngine::new(Duration::from_millis(10));

        engine.handle_event(make_pending("/repo", "/repo/a.txt"));
        engine.handle_event(make_pending("/repo", "/repo/a.txt"));

        thread::sleep(Duration::from_millis(15));
        let flushed = engine.drain_expired_events();

        let first = flushed.first().expect("one flushed event");
        let FileEvent::Git { paths } = &first.event else {
            panic!("expected a git event");
        };
        assert_eq!(
            paths.as_slice(),
            Some([PathBuf::from("/repo/a.txt")].as_slice())
        );
    }

    #[test]
    /// Past the bound the pathspec stops being cheaper than a full scan, and unbounded
    /// accumulation would grow `PendingEntry` against the memory budget.
    ///
    /// Degrade explicitly rather than truncating silently (#466).
    fn accumulation_past_the_bound_degrades_to_a_whole_repo_scan() {
        let mut engine = DebounceEngine::new(Duration::from_millis(10));

        for index in 0..=GitPaths::MAX_PATHS {
            engine.handle_event(make_pending("/repo", &format!("/repo/file{index}.txt")));
        }

        thread::sleep(Duration::from_millis(15));
        let flushed = engine.drain_expired_events();

        let first = flushed.first().expect("one flushed event");
        let FileEvent::Git { paths } = &first.event else {
            panic!("expected a git event");
        };
        assert_eq!(paths, &GitPaths::WholeRepo);
        assert!(
            paths.as_slice().is_none(),
            "a whole-repo event has no pathspec"
        );
    }

    #[test]
    /// Only Git events accumulate. A language/config/theme handler refreshes a
    /// whole subsystem for the repo the key already pins, so the specific path
    /// carries no extra information worth retaining.
    fn non_git_events_keep_last_writer_wins() {
        let mut engine = DebounceEngine::new(Duration::from_millis(10));

        engine.handle_event(PendingEvent {
            event: FileEvent::Language {
                path: PathBuf::from("/repo/Cargo.toml"),
            },
            repo: PathBuf::from("/repo"),
        });
        engine.handle_event(PendingEvent {
            event: FileEvent::Language {
                path: PathBuf::from("/repo/package.json"),
            },
            repo: PathBuf::from("/repo"),
        });

        thread::sleep(Duration::from_millis(15));
        let flushed = engine.drain_expired_events();

        assert_eq!(flushed.len(), 1);
        let first = flushed.first().expect("one flushed event");
        assert!(matches!(
            &first.event,
            FileEvent::Language { path } if path == Path::new("/repo/package.json")
        ));
    }

    #[test]
    /// #569: an entry scheduled through `handle_event_due` must not drain
    /// merely because its quiet gap elapsed.
    ///
    /// The due-time floor is what the directory-create follow-up's delay now
    /// rides on, so the flush thread does the waiting a per-directory timer
    /// thread used to do.
    fn a_due_floor_holds_an_entry_past_its_quiet_gap() {
        let mut engine =
            DebounceEngine::new(Duration::from_millis(10)).with_max_delay(Duration::from_secs(5));
        let due = Instant::now() + Duration::from_millis(200);

        engine.handle_event_due(make_pending("/repo", "/repo/newdir"), due);

        thread::sleep(Duration::from_millis(25));
        assert!(
            engine.drain_expired_events().is_empty(),
            "the quiet gap alone must not release an entry still under its due floor"
        );

        sleep_past(due);
        assert_eq!(
            engine.drain_expired_events().len(),
            1,
            "the entry must drain once its due floor has passed"
        );
    }

    #[test]
    /// #569: the floor is a property of the entry, not of the call that set it.
    ///
    /// An ordinary event merging into a scheduled follow-up (and vice versa)
    /// keeps the floor, so the pair delivers once, after the delay — never as
    /// an early flush that drops the follow-up's whole reason to exist.
    fn a_due_floor_survives_merging_with_an_ordinary_event() {
        for plain_first in [true, false] {
            let mut engine = DebounceEngine::new(Duration::from_millis(10))
                .with_max_delay(Duration::from_secs(5));
            let due = Instant::now() + Duration::from_millis(200);

            if plain_first {
                engine.handle_event(make_pending("/repo", "/repo/a.txt"));
                engine.handle_event_due(make_pending("/repo", "/repo/newdir"), due);
            } else {
                engine.handle_event_due(make_pending("/repo", "/repo/newdir"), due);
                engine.handle_event(make_pending("/repo", "/repo/a.txt"));
            }

            thread::sleep(Duration::from_millis(25));
            assert!(
                engine.drain_expired_events().is_empty(),
                "merged entry must stay held by the floor (plain event first: {plain_first})"
            );

            sleep_past(due);
            let flushed = engine.drain_expired_events();
            assert_eq!(flushed.len(), 1, "the two events must coalesce into one");
            let first = flushed.first().expect("one flushed event");
            let FileEvent::Git { paths } = &first.event else {
                panic!("expected a git event");
            };
            let observed = paths.as_slice().expect("a pathspec, not a whole-repo scan");
            assert!(
                observed.contains(&PathBuf::from("/repo/a.txt"))
                    && observed.contains(&PathBuf::from("/repo/newdir")),
                "both the ordinary path and the follow-up path must survive: {observed:?}"
            );
        }
    }

    #[test]
    /// #569: two follow-ups for one repository keep the later floor, in either
    /// arrival order.
    ///
    /// The last directory created in a burst still gets its full delay window
    /// rather than inheriting an already-expiring one.
    fn a_second_due_floor_keeps_whichever_is_later() {
        let early = Instant::now() + Duration::from_millis(30);
        let late = Instant::now() + Duration::from_millis(250);

        let mut later_last = DebounceEngine::new(Duration::from_millis(10));
        later_last.handle_event_due(make_pending("/repo", "/repo/a"), early);
        later_last.handle_event_due(make_pending("/repo", "/repo/b"), late);
        assert_eq!(git_floor(&later_last, "/repo"), Some(late));

        let mut later_first = DebounceEngine::new(Duration::from_millis(10));
        later_first.handle_event_due(make_pending("/repo", "/repo/a"), late);
        later_first.handle_event_due(make_pending("/repo", "/repo/b"), early);
        assert_eq!(git_floor(&later_first, "/repo"), Some(late));
    }

    #[test]
    /// An ordinary event carries no floor, so every pre-#569 caller keeps the
    /// pure quiet-gap/max-delay rule.
    fn an_ordinary_event_records_no_due_floor() {
        let mut engine = DebounceEngine::new(Duration::from_millis(10));
        engine.handle_event(make_pending("/repo", "/repo/a.txt"));
        assert_eq!(git_floor(&engine, "/repo"), None);
    }

    #[test]
    /// Acceptance criterion for #569: a burst of 500 created directories in one
    /// repository spawns zero additional threads.
    ///
    /// The no-thread half is true by construction — `handle_event_due` is a
    /// synchronous map update with no `thread::spawn` anywhere beneath it — so
    /// what needs proving is the other half: 500 scheduling calls collapse into
    /// a single pending entry, and so into a single follow-up scan, instead of
    /// 500 of anything.
    fn five_hundred_directory_followups_collapse_to_one_entry() {
        let mut engine = DebounceEngine::new(Duration::from_millis(10));
        let due = Instant::now() + Duration::from_millis(50);

        for index in 0..500_usize {
            engine.handle_event_due(make_pending("/repo", &format!("/repo/dir{index}")), due);
        }

        assert_eq!(
            engine.pending_events.len(),
            1,
            "500 directory follow-ups for one repo must coalesce into one pending entry"
        );

        sleep_past(due);
        let flushed = engine.drain_expired_events();
        assert_eq!(flushed.len(), 1, "and into one delivered event");
        let first = flushed.first().expect("one flushed event");
        let FileEvent::Git { paths } = &first.event else {
            panic!("expected a git event");
        };
        assert_eq!(
            paths,
            &GitPaths::WholeRepo,
            "past MAX_PATHS the coalesced burst degrades to a whole-repo scan, as for any burst"
        );
    }
}

//! Event debouncing for file system changes.
//!
//! Groups pending events by `(kind, repo)` and releases them after a quiet gap
//! or a max-delay cap. It never invokes callbacks: callers dispatch outside its lock.

use super::{DebouncedEvent, FileEvent, PendingEvent};
use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Longest an entry may wait under sustained churn before it flushes anyway.
const DEFAULT_MAX_DELAY: Duration = Duration::from_secs(1);

/// Debouncer for file system events
pub struct DebounceEngine {
    /// Pending events by (kind, repo) key
    pending_events: HashMap<FileEventKey, PendingEntry>,
    /// How long a key must be quiet before it flushes
    debounce_duration: Duration,
    /// Longest a key may be held under continued churn.
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
    /// Earliest drain instant, only ever raised; set by
    /// [`DebounceEngine::handle_event_due`] and kept across ordinary merges.
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
/// Git events accumulate paths (they drive a pathspec-limited scan); other
/// kinds refresh a whole subsystem, so last-writer-wins.
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

    /// Override the max-delay cap.
    #[must_use]
    pub const fn with_max_delay(mut self, max_delay: Duration) -> Self {
        self.max_delay = max_delay;
        self
    }

    /// Process a file system event, keeping the first-seen time for the
    /// max-delay cap and merging via [`merge_pending`].
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

    /// Like [`DebounceEngine::handle_event`], but not drained before `due`;
    /// merges keep the later floor. Used for directory-create follow-ups so
    /// the existing flush thread does the waiting.
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

    /// Remove and return events past the quiet gap or max-delay cap. Callers
    /// must deliver them without holding this engine's lock.
    pub fn drain_expired_events(&mut self) -> Vec<DebouncedEvent> {
        let now = Instant::now();
        let debounce_duration = self.debounce_duration;
        let max_delay = self.max_delay;
        self.pending_events
            .iter()
            .filter(|&(_, entry)| {
                let debounce_elapsed = now.duration_since(entry.last_seen) >= debounce_duration
                    || now.duration_since(entry.first_seen) >= max_delay;
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

    /// The earliest instant [`DebounceEngine::drain_expired_events`] would
    /// release a pending entry, or `None` when nothing can fall due. Per entry
    /// that is the quiet gap or the max-delay cap, whichever is sooner, but no
    /// earlier than its `not_before` floor; an overflowing term never falls due.
    #[must_use]
    pub fn next_deadline(&self) -> Option<Instant> {
        self.pending_events
            .values()
            .filter_map(|entry| {
                let quiet = entry.last_seen.checked_add(self.debounce_duration);
                let cap = entry.first_seen.checked_add(self.max_delay);
                let due = match (quiet, cap) {
                    (Some(quiet_at), Some(cap_at)) => quiet_at.min(cap_at),
                    (Some(due), None) | (None, Some(due)) => due,
                    (None, None) => return None,
                };
                Some(due.max(entry.not_before.unwrap_or(due)))
            })
            .min()
    }

    /// Clear all pending events
    pub fn clear(&mut self) {
        self.pending_events.clear();
    }

    /// Get recommended debounce duration for file watching
    #[must_use]
    pub const fn default_debounce_duration() -> Duration {
        Duration::from_millis(super::DEFAULT_DEBOUNCE_MS)
    }

    /// Get the default max-delay cap.
    #[must_use]
    pub const fn default_max_delay() -> Duration {
        DEFAULT_MAX_DELAY
    }
}

#[cfg(test)]
mod tests {
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

    /// The due-time floor recorded for a repository's Git key.
    fn git_floor(engine: &DebounceEngine, repo: &str) -> Option<Instant> {
        engine
            .pending_events
            .get(&FileEventKey::Git(PathBuf::from(repo)))
            .and_then(|entry| entry.not_before)
    }

    /// Sleep until just past `deadline`.
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
        let mut engine = DebounceEngine::new(Duration::from_millis(1));
        engine.handle_event(make_pending("/repo", "/repo/.git/HEAD"));
        thread::sleep(Duration::from_millis(5));

        let flushed = engine.drain_expired_events();
        assert_eq!(flushed.len(), 1);
        assert!(engine.drain_expired_events().is_empty());
    }

    #[test]
    fn sustained_churn_still_forces_a_flush_within_max_delay() {
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
        let mut engine =
            DebounceEngine::new(Duration::from_millis(10)).with_max_delay(Duration::from_secs(5));

        engine.handle_event(make_pending("/repo", "/repo/.git/HEAD"));
        thread::sleep(Duration::from_millis(15));

        assert_eq!(engine.drain_expired_events().len(), 1);
    }

    #[test]
    /// Different files in one window must all survive coalescing.
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
    /// Past the bound, degrade explicitly to a whole-repo scan.
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
    /// The quiet gap alone must not release an entry under its due floor.
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
    /// A floor survives merging with an ordinary event, in either order.
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
    /// Two follow-ups for one repository keep the later floor, in either order.
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
    fn an_ordinary_event_records_no_due_floor() {
        let mut engine = DebounceEngine::new(Duration::from_millis(10));
        engine.handle_event(make_pending("/repo", "/repo/a.txt"));
        assert_eq!(git_floor(&engine, "/repo"), None);
    }

    #[test]
    /// A burst of 500 directory follow-ups collapses to one pending entry.
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

    #[test]
    fn next_deadline_is_none_when_empty() {
        let engine = DebounceEngine::new(Duration::from_millis(10));
        assert_eq!(engine.next_deadline(), None);
    }

    #[test]
    fn next_deadline_is_last_seen_plus_debounce() {
        let debounce = Duration::from_millis(10);
        let mut engine = DebounceEngine::new(debounce);
        let before = Instant::now();
        engine.handle_event(make_pending("/repo", "/repo/.git/HEAD"));
        let after = Instant::now();

        let deadline = engine.next_deadline().expect("one pending entry");
        assert!(
            deadline >= before + debounce,
            "not before last_seen + debounce"
        );
        assert!(
            deadline <= after + debounce,
            "not after last_seen + debounce"
        );
    }

    #[test]
    fn next_deadline_is_capped_by_max_delay() {
        let mut engine =
            DebounceEngine::new(Duration::from_secs(60)).with_max_delay(Duration::from_millis(20));
        engine.handle_event(make_pending("/repo", "/repo/.git/HEAD"));
        let after = Instant::now();

        let deadline = engine.next_deadline().expect("one pending entry");
        assert!(
            deadline <= after + Duration::from_millis(20),
            "the max-delay cap bounds the deadline"
        );
    }

    #[test]
    fn next_deadline_respects_not_before() {
        let mut engine = DebounceEngine::new(Duration::from_millis(10));
        let due = Instant::now() + Duration::from_millis(500);
        engine.handle_event_due(make_pending("/repo", "/repo/.git/HEAD"), due);

        assert_eq!(engine.next_deadline(), Some(due));
    }

    #[test]
    fn next_deadline_skips_overflowing_entries() {
        let mut engine = DebounceEngine::new(Duration::MAX).with_max_delay(Duration::MAX);
        engine.handle_event(make_pending("/repo", "/repo/.git/HEAD"));

        assert_eq!(engine.next_deadline(), None);
    }
}

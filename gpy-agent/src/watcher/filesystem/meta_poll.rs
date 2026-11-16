//! Polling half for git metadata: compares each entry's size, mtime and (on
//! Unix) inode and ctime instead of hashing its contents.
//!
//! `notify::PollWatcher` compares whole-second mtimes, so the content-hashing
//! mode is its only way to see a same-second rewrite; that re-reads every file
//! of every target on each tick, including a `.git/index` of hundreds of MB.
//! Git rewrites metadata by lock file and rename (a new inode), and any write
//! moves the ctime, so this stat-only comparison sees the same changes.

use super::FileSystemWatcher;
use crate::sync_ext::MutexExt;
use notify::Event;
use notify::event::{CreateKind, DataChange, EventKind, ModifyKind, RemoveKind};
use std::collections::HashMap;
use std::fs::Metadata;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime};

/// What a scan compares for one entry.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
struct EntryStamp {
    is_dir: bool,
    len: u64,
    modified: Option<SystemTime>,
    /// `(inode, ctime seconds, ctime nanoseconds)`; ctime cannot be set from
    /// user space, so it catches a rewrite whose mtime was pinned.
    #[cfg(unix)]
    identity: (u64, i64, i64),
}

impl EntryStamp {
    fn of(metadata: &Metadata) -> Self {
        Self {
            is_dir: metadata.is_dir(),
            len: metadata.len(),
            modified: metadata.modified().ok(),
            #[cfg(unix)]
            identity: {
                use std::os::unix::fs::MetadataExt as _;
                (metadata.ino(), metadata.ctime(), metadata.ctime_nsec())
            },
        }
    }
}

/// One watched target: the target itself plus, for a directory, its direct
/// children (targets are armed shallow, like the poll expansion).
type TargetSnapshot = HashMap<PathBuf, EntryStamp>;

/// Stamp `target` and its direct children; empty when `target` is missing.
fn snapshot(target: &Path) -> TargetSnapshot {
    let mut entries = HashMap::new();
    let Ok(metadata) = std::fs::metadata(target) else {
        return entries;
    };
    let is_dir = metadata.is_dir();
    entries.insert(target.to_path_buf(), EntryStamp::of(&metadata));
    if !is_dir {
        return entries;
    }
    let Ok(children) = std::fs::read_dir(target) else {
        return entries;
    };
    for child in children.flatten() {
        // `DirEntry::metadata` does not follow symlinks.
        if let Ok(child_metadata) = child.metadata() {
            entries.insert(child.path(), EntryStamp::of(&child_metadata));
        }
    }
    entries
}

/// Events turning `old` into `new`. A directory's own stamp changes whenever a
/// child does, and the child's event carries that change, so it is skipped.
fn diff(old: &TargetSnapshot, new: &TargetSnapshot) -> Vec<Event> {
    let mut events = Vec::new();
    for (path, stamp) in new {
        let kind = match old.get(path) {
            None => EventKind::Create(CreateKind::Any),
            Some(previous) if previous != stamp && !stamp.is_dir => {
                EventKind::Modify(ModifyKind::Data(DataChange::Any))
            }
            Some(_) => continue,
        };
        events.push(Event::new(kind).add_path(path.clone()));
    }
    for path in old.keys() {
        if !new.contains_key(path) {
            events.push(Event::new(EventKind::Remove(RemoveKind::Any)).add_path(path.clone()));
        }
    }
    events
}

/// Watched targets and their last snapshots.
type Targets = Arc<Mutex<HashMap<PathBuf, TargetSnapshot>>>;

/// Rescan every target once, forwarding the differences. Filesystem reads run
/// without the lock, so `watch`/`unwatch` never wait on a slow mount.
fn scan_all(targets: &Mutex<HashMap<PathBuf, TargetSnapshot>>, event_tx: &Sender<Event>) {
    let paths: Vec<PathBuf> = targets.lock_or_recover().keys().cloned().collect();
    for target in paths {
        let fresh = snapshot(&target);
        let events = {
            let mut guard = targets.lock_or_recover();
            let Some(previous) = guard.get_mut(&target) else {
                continue;
            };
            let events = diff(previous, &fresh);
            *previous = fresh;
            events
        };
        for event in events {
            FileSystemWatcher::forward_event(event_tx, Ok(event));
        }
    }
}

/// Stat-comparing poller for git metadata targets.
pub(super) struct MetadataPoller {
    targets: Targets,
    /// Dropping it disconnects the scan thread's stop channel, ending it.
    _stop: Sender<()>,
}

impl MetadataPoller {
    /// Start the scan thread, which forwards to `event_tx` every `poll_interval`.
    ///
    /// # Errors
    /// Returns an error if the scan thread cannot be spawned.
    pub(super) fn new(event_tx: &Sender<Event>, poll_interval: Duration) -> notify::Result<Self> {
        let targets: Targets = Arc::default();
        let (stop, stop_rx) = mpsc::channel::<()>();
        let targets_for_scan = Arc::clone(&targets);
        let tx_for_scan = event_tx.clone();
        std::thread::Builder::new()
            .name("gpy-gitdir-poll".to_owned())
            .spawn(move || {
                while matches!(
                    stop_rx.recv_timeout(poll_interval),
                    Err(mpsc::RecvTimeoutError::Timeout)
                ) {
                    scan_all(&targets_for_scan, &tx_for_scan);
                }
            })
            .map_err(notify::Error::io)?;
        Ok(Self {
            targets,
            _stop: stop,
        })
    }

    /// Baseline `path` now; a missing path is armed empty and reports its
    /// creation later.
    pub(super) fn watch(&self, path: &Path) {
        let baseline = snapshot(path);
        self.targets
            .lock_or_recover()
            .insert(path.to_path_buf(), baseline);
    }

    /// Stop scanning `path`.
    ///
    /// # Errors
    /// Returns an error if `path` was not watched.
    pub(super) fn unwatch(&self, path: &Path) -> notify::Result<()> {
        if self.targets.lock_or_recover().remove(path).is_some() {
            Ok(())
        } else {
            Err(notify::Error::watch_not_found())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{diff, snapshot};
    use notify::Event;
    use notify::event::{CreateKind, DataChange, EventKind, ModifyKind, RemoveKind};
    use std::path::Path;
    use std::time::{Duration, SystemTime};

    /// Whether `events` holds a `kind` event for exactly `path`.
    fn has_event(events: &[Event], path: &Path, kind: EventKind) -> bool {
        events
            .iter()
            .any(|event| event.kind == kind && event.paths.as_slice() == [path.to_path_buf()])
    }

    #[test]
    fn same_size_rewrite_with_pinned_mtime_is_a_modify() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let file = tmp.path().join("HEAD");
        let pinned = SystemTime::UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        let pin = |contents: &[u8]| {
            std::fs::write(&file, contents).expect("write");
            std::fs::File::options()
                .write(true)
                .open(&file)
                .expect("open")
                .set_modified(pinned)
                .expect("pin mtime");
        };

        pin(b"ref: refs/heads/a\n");
        let before = snapshot(tmp.path());
        // Let the ctime clock move on filesystems with coarse timestamps.
        std::thread::sleep(Duration::from_millis(20));
        pin(b"ref: refs/heads/b\n");
        let after = snapshot(tmp.path());

        let events = diff(&before, &after);
        assert!(
            has_event(
                &events,
                &file,
                EventKind::Modify(ModifyKind::Data(DataChange::Any))
            ),
            "a same-size rewrite with a pinned mtime must be reported: {events:?}"
        );
    }

    #[test]
    fn creates_and_removes_are_reported_and_directory_stamps_are_not() {
        let tmp = tempfile::TempDir::new().expect("temp dir");
        let gone = tmp.path().join("ORIG_HEAD");
        std::fs::write(&gone, b"x\n").expect("write");
        let before = snapshot(tmp.path());

        std::fs::remove_file(&gone).expect("remove");
        let added = tmp.path().join("heads");
        std::fs::create_dir_all(&added).expect("mkdir");
        let after = snapshot(tmp.path());

        let events = diff(&before, &after);
        assert!(has_event(
            &events,
            &added,
            EventKind::Create(CreateKind::Any)
        ));
        assert!(has_event(
            &events,
            &gone,
            EventKind::Remove(RemoveKind::Any)
        ));
        assert_eq!(
            events.len(),
            2,
            "the target directory's own stamp change is noise: {events:?}"
        );
    }
}

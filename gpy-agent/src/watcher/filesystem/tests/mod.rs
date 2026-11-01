#![allow(clippy::missing_panics_doc)]

use super::backend::poll_watch_targets;
use super::ignore::{force_added_set, is_force_added};
use super::probe::{engage_poll_fallback, native_backend_delivers_events};
use super::*;
use crate::env_config::{DEFAULT_WATCH_POLL_MS, DEFAULT_WATCH_PROBE_MS};
use crate::sync_ext::MutexExt;
use crate::watcher::{FileEvent, GitPaths, PendingCallback, PendingEvent};
use notify::event::{
    AccessKind, AccessMode, CreateKind, DataChange, EventAttributes, EventKind, Flag, MetadataKind,
    ModifyKind, RenameMode,
};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

impl FileSystemWatcher {
    /// The backend currently armed.
    fn backend_kind(&self) -> BackendKind {
        self.backend.lock_or_recover().kind
    }

    /// Swap to the polling backend on demand via the production swap path.
    fn force_poll_fallback(&self, poll_interval: Duration) {
        if let Some(event_tx) = self.event_tx.as_ref() {
            engage_poll_fallback(
                &self.backend,
                event_tx,
                &self.stop_flag,
                poll_interval,
                &self.registry,
            );
        }
    }

    /// Registrations with owner counts, and the armed OS watches.
    pub(crate) fn held_watches(
        &self,
    ) -> (
        BTreeMap<(PathBuf, WatchMode), u32>,
        BTreeMap<PathBuf, WatchMode>,
    ) {
        let guard = self.backend.lock_or_recover();
        let owners = guard
            .registrations
            .iter()
            .map(|(key, registration)| (key.clone(), registration.owners))
            .collect();
        (owners, guard.os.armed_paths())
    }
}

/// Follow-up delay used by `process_event` in tests: short, but long enough to
/// tell a non-directory event scheduled none.
const TEST_FOLLOWUP_DELAY: Duration = Duration::from_millis(50);

/// A private [`WatchRegistry`]; empty `roots` makes attribution fall through to
/// the ancestor `.git` walk.
fn test_registry(worktree_enabled: bool, roots: HashSet<PathBuf>) -> Arc<WatchRegistry> {
    configure_registry(WatchRegistry::new(), worktree_enabled, roots)
}

/// [`test_registry`] whose force-added sets come from `ls_files` instead of git.
fn test_registry_with_ls_files(
    worktree_enabled: bool,
    roots: HashSet<PathBuf>,
    ls_files: impl Fn(&Path) -> Vec<PathBuf> + Send + Sync + 'static,
) -> Arc<WatchRegistry> {
    let registry = WatchRegistry::with_ignore(IgnoreCaches::new(Arc::new(ls_files)));
    configure_registry(registry, worktree_enabled, roots)
}

fn configure_registry(
    registry: WatchRegistry,
    worktree_enabled: bool,
    roots: HashSet<PathBuf>,
) -> Arc<WatchRegistry> {
    registry.set_worktree_enabled(worktree_enabled);
    for root in roots {
        registry.insert_root(root);
    }
    Arc::new(registry)
}

/// Shared buffer of events captured by a recording callback.
type CapturedEvents = Arc<Mutex<Vec<PendingEvent>>>;

/// A callback that appends every event to a shared buffer, which outlives the
/// callback so a reader never depends on the watcher releasing its own handle.
fn recording_callback() -> (PendingCallback, CapturedEvents) {
    let captured = CapturedEvents::default();
    let sink = Arc::clone(&captured);
    let callback: PendingCallback = Box::new(move |event| {
        sink.lock().unwrap().push(event);
    });
    (callback, captured)
}

/// What `process_event` emits for one `kind` event on `paths`; no follow-up is scheduled.
fn events_for(
    registry: &Arc<WatchRegistry>,
    kind: EventKind,
    paths: Vec<PathBuf>,
) -> Vec<PendingEvent> {
    let (callback, captured) = recording_callback();
    let event = Event {
        kind,
        paths,
        attrs: EventAttributes::default(),
    };
    FileSystemWatcher::process_event(
        &event,
        &Arc::new(callback),
        registry,
        TEST_FOLLOWUP_DELAY,
        &no_op_scheduler(),
    );
    std::mem::take(&mut *captured.lock().unwrap())
}

/// A scheduler that does nothing, for paths that never schedule a follow-up.
fn no_op_scheduler() -> DelayedEventScheduler {
    Arc::new(|_, _| {})
}

/// Every `(PendingEvent, Instant)` a scheduler was invoked with.
type CapturedSchedules = Arc<Mutex<Vec<(PendingEvent, Instant)>>>;

/// A scheduler that records requests, so tests assert on the scheduling
/// decision; delivery is `DebounceEngine`'s contract.
fn recording_scheduler() -> (DelayedEventScheduler, CapturedSchedules) {
    let captured: CapturedSchedules = Arc::new(Mutex::new(Vec::new()));
    let captured_for_hook = Arc::clone(&captured);
    let scheduler: DelayedEventScheduler = Arc::new(move |pending, due| {
        captured_for_hook.lock().unwrap().push((pending, due));
    });
    (scheduler, captured)
}

/// A real repo in a temp dir with a worktree-enabled watcher on it. The watcher
/// field precedes the directory, so the watcher stops before the tree is removed.
struct PollRepo {
    watcher: FileSystemWatcher,
    root: PathBuf,
    events: CapturedEvents,
    _dir: tempfile::TempDir,
}

impl PollRepo {
    /// A repo watched by the `PollWatcher` fallback, forced on from the start.
    fn forced_poll(poll_interval: Duration) -> Self {
        Self::with_policy(FallbackPolicy::ForcePoll { poll_interval })
    }

    fn with_policy(policy: FallbackPolicy) -> Self {
        let dir = tempfile::TempDir::new().expect("temp dir");
        let root = std::fs::canonicalize(dir.path()).expect("canonicalize repo");
        std::fs::create_dir_all(root.join(".git")).expect("git dir");

        let registry = test_registry(true, HashSet::from([root.clone()]));
        let (callback, events) = recording_callback();
        let watcher = FileSystemWatcher::with_fallback_policy(
            callback,
            Some(registry),
            Duration::from_millis(50),
            no_op_scheduler(),
            policy,
        )
        .expect("create watcher");
        Self {
            watcher,
            root,
            events,
            _dir: dir,
        }
    }

    /// Append to `file` every 500 ms until the watcher delivers an event or
    /// `budget` elapses. A single append can land before the poll baseline is taken.
    fn append_until_delivered(&self, file: &Path, budget: Duration) -> bool {
        wait_until(budget, || {
            let mut handle = std::fs::OpenOptions::new()
                .append(true)
                .open(file)
                .expect("open file for append");
            std::io::Write::write_all(&mut handle, b"modified\n").expect("append");
            drop(handle);
            wait_for_any_event(&self.events, Duration::from_millis(500))
        })
    }

    /// Create a new file `<stem>-<n>.txt` every 500 ms until the watcher delivers
    /// an event or `budget` elapses.
    fn create_until_delivered(&self, stem: &str, budget: Duration) -> bool {
        let mut attempt = 0_usize;
        wait_until(budget, || {
            attempt = attempt.saturating_add(1);
            let file = self.root.join(format!("{stem}-{attempt}.txt"));
            std::fs::write(&file, b"content").expect("write new file");
            wait_for_any_event(&self.events, Duration::from_millis(500))
        })
    }
}

/// Poll until `done` holds, or `budget` elapses.
fn wait_until(budget: Duration, mut done: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now()
        .checked_add(budget)
        .unwrap_or_else(Instant::now);
    while Instant::now() < deadline {
        if done() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
    false
}

/// Poll `events` until non-empty or `budget` elapses.
fn wait_for_any_event(events: &CapturedEvents, budget: Duration) -> bool {
    wait_until(budget, || !events.lock().unwrap().is_empty())
}

/// Whether `events` holds a Git event naming `path`.
fn has_git_event_for(events: &CapturedEvents, path: &Path) -> bool {
    events
        .lock()
        .unwrap()
        .iter()
        .any(|pending| match &pending.event {
            FileEvent::Git { paths } => paths
                .as_slice()
                .is_some_and(|named| named.iter().any(|named_path| named_path == path)),
            _ => false,
        })
}

/// Poll `events` until it holds a Git event naming `path`, or `budget` elapses.
fn wait_for_git_event_for(events: &CapturedEvents, path: &Path, budget: Duration) -> bool {
    wait_until(budget, || has_git_event_for(events, path))
}

/// The OS watches `watcher` holds armed, by path.
fn armed_dirs(watcher: &FileSystemWatcher) -> BTreeSet<PathBuf> {
    watcher.held_watches().1.into_keys().collect()
}

/// The poll targets the recursive registration of `root` holds.
fn poll_targets_of(watcher: &FileSystemWatcher, root: &Path) -> BTreeSet<PathBuf> {
    let guard = watcher.backend.lock().expect("backend lock");
    guard
        .registrations
        .get(&(root.to_path_buf(), WatchMode::Recursive))
        .map(|registration| {
            registration
                .targets
                .iter()
                .map(|(target, _)| target.clone())
                .collect()
        })
        .unwrap_or_default()
}

/// The union of every registration's targets: what must be armed.
fn all_targets(watcher: &FileSystemWatcher) -> BTreeSet<PathBuf> {
    let guard = watcher.backend.lock().expect("backend lock");
    guard
        .registrations
        .values()
        .flat_map(|registration| registration.targets.iter())
        .map(|(target, _)| target.clone())
        .collect()
}

/// Run a git command in `dir`, panicking on failure.
fn git_in(dir: &Path, args: &[&str]) {
    let output = std::process::Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap_or_else(|e| panic!("failed to run git {args:?}: {e}"));
    assert!(
        output.status.success(),
        "git {args:?} failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// `git init` in `dir` with an identity and signing off, ready to commit.
fn git_init(dir: &Path) {
    git_in(dir, &["init", "-q"]);
    git_in(dir, &["config", "user.email", "test@example.com"]);
    git_in(dir, &["config", "user.name", "Test User"]);
    git_in(dir, &["config", "commit.gpgsign", "false"]);
}

mod attribution;
mod gitignore;
mod locking;
mod poll_backend;
mod probe_policy;
mod rescan;

//! File system watching built on the notify crate.

mod backend;
mod ignore;
mod probe;

use super::watch_set::WatchMode;
use super::{
    DelayedEventScheduler, PendingCallback, PendingEvent, WatchRegistry, should_trigger_update,
};
use crate::Result;
use crate::sync_ext::MutexExt;
use backend::{
    BackendKind, WatcherBackend, build_native_watcher, build_poll_watcher, poll_watch_targets,
    track_poll_expansion,
};
pub(crate) use ignore::IgnoreCaches;
use ignore::{
    IgnoreTokenHit, language_marker_is_ignored, path_is_gitignored, path_matches_ignored_patterns,
};
use notify::Event;
use notify::event::{CreateKind, EventKind, MetadataKind, ModifyKind};
use probe::{FallbackPolicy, ProbeTiming, spawn_health_probe};

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

/// Shared read-only context for one `process_event` call.
struct WorktreeEventContext<'a> {
    event: &'a Event,
    registry: &'a Arc<WatchRegistry>,
    worktree_enabled: bool,
}

/// One raw event path already attributed to a repository root.
struct AttributedWorktreeEvent<'a> {
    event: &'a Event,
    path: &'a Path,
    repo: PathBuf,
    registry: &'a Arc<WatchRegistry>,
}

/// Where the directory-create follow-up refresh goes, and its delay.
struct FollowupHandoff {
    delay: Duration,
    schedule: DelayedEventScheduler,
}

/// Watcher state the event thread shares beyond its channel and stop flag.
struct EventThreadState {
    registry: Arc<WatchRegistry>,
    backend: Arc<Mutex<WatcherBackend>>,
}

/// File system watcher using notify crate.
pub struct FileSystemWatcher {
    backend: Arc<Mutex<WatcherBackend>>,
    event_tx: Option<Sender<Event>>,
    handle: Option<thread::JoinHandle<()>>,
    probe_handle: Option<thread::JoinHandle<()>>,
    stop_flag: Arc<AtomicBool>,
    /// Shared with the owning coordinator and the event thread.
    registry: Arc<WatchRegistry>,
}

impl FileSystemWatcher {
    /// Create a watcher. `registry: None` allocates a private registry (standalone watchers).
    ///
    /// # Errors
    /// Returns an error if the file watcher cannot be created.
    pub fn new(
        callback: PendingCallback,
        registry: Option<Arc<WatchRegistry>>,
        followup_delay: Duration,
        schedule_delayed: DelayedEventScheduler,
    ) -> Result<Self> {
        let policy = FallbackPolicy::from_env();
        Self::with_fallback_policy(callback, registry, followup_delay, schedule_delayed, policy)
    }

    /// Construction with an explicit [`FallbackPolicy`], so tests avoid env mutation.
    ///
    /// # Errors
    /// Returns an error if the selected notify backend cannot be created.
    fn with_fallback_policy(
        callback: PendingCallback,
        registry: Option<Arc<WatchRegistry>>,
        followup_delay: Duration,
        schedule_delayed: DelayedEventScheduler,
        policy: FallbackPolicy,
    ) -> Result<Self> {
        let shared_registry = registry.unwrap_or_else(|| Arc::new(WatchRegistry::new()));
        let (event_tx, event_rx): (Sender<Event>, Receiver<Event>) = mpsc::channel();

        let (watcher, kind) = match policy {
            FallbackPolicy::ForcePoll { poll_interval } => (
                build_poll_watcher(&event_tx, poll_interval)?,
                BackendKind::Poll,
            ),
            FallbackPolicy::NativeOnly | FallbackPolicy::ProbeThenFallback { .. } => {
                (build_native_watcher(&event_tx)?, BackendKind::Native)
            }
        };
        crate::debug_log!(
            "watcher",
            "backend armed: {} (policy: {policy:?})",
            kind.label()
        );

        let backend = Arc::new(Mutex::new(WatcherBackend::armed(watcher, kind)));

        let stop_flag = Arc::new(AtomicBool::new(false));
        let handle = Self::spawn_event_thread(
            event_rx,
            callback,
            &stop_flag,
            EventThreadState {
                registry: Arc::clone(&shared_registry),
                backend: Arc::clone(&backend),
            },
            FollowupHandoff {
                delay: followup_delay,
                schedule: schedule_delayed,
            },
        );

        let probe_handle = match policy {
            FallbackPolicy::ProbeThenFallback {
                probe_timeout,
                poll_interval,
            } => Some(spawn_health_probe(
                &backend,
                &event_tx,
                &stop_flag,
                ProbeTiming {
                    probe_timeout,
                    poll_interval,
                },
                &shared_registry,
            )),
            FallbackPolicy::NativeOnly | FallbackPolicy::ForcePoll { .. } => None,
        };

        Ok(Self {
            backend,
            event_tx: Some(event_tx),
            handle: Some(handle),
            probe_handle,
            stop_flag,
            registry: shared_registry,
        })
    }

    /// Spawn the thread that drains the event channel into `process_event`.
    fn spawn_event_thread(
        event_rx: Receiver<Event>,
        callback: PendingCallback,
        stop_flag: &Arc<AtomicBool>,
        state: EventThreadState,
        followup: FollowupHandoff,
    ) -> thread::JoinHandle<()> {
        let callback_arc = Arc::new(callback);
        let stop_flag_thread = Arc::clone(stop_flag);
        thread::spawn(move || {
            while let Ok(event) = event_rx.recv() {
                if stop_flag_thread.load(Ordering::Relaxed) {
                    break;
                }
                // Arm new directories before any refresh scans them.
                track_poll_expansion(&event, &state.backend, &state.registry);
                Self::process_event(
                    &event,
                    &callback_arc,
                    &state.registry,
                    followup.delay,
                    &followup.schedule,
                );
            }
        })
    }

    /// Forward one raw notify result into the shared event channel.
    fn forward_event(event_tx: &Sender<Event>, res: notify::Result<Event>) {
        match res {
            Ok(event) => {
                let _ = event_tx.send(event);
            }
            Err(error) => {
                crate::debug_log!("watcher", "notify backend error: {error}");
            }
        }
    }

    /// Start watching a directory recursively.
    ///
    /// # Errors
    /// Returns an error if the path cannot be watched.
    pub fn watch<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path_buf = path.as_ref().to_path_buf();
        let is_dir = path_buf.is_dir();

        loop {
            // Release the lock before `poll_watch_targets`: it can walk the tree and
            // run `git ls-files`, and this mutex is shared by every repo.
            let kind = self.backend.lock_or_recover().kind;

            let poll_targets = (kind == BackendKind::Poll && is_dir)
                .then(|| poll_watch_targets(&path_buf, &self.registry));

            let mut backend_guard = self.backend.lock_or_recover();
            // The probe may have swapped native -> poll meanwhile (at most once); retry.
            if backend_guard.kind != kind {
                drop(backend_guard);
                continue;
            }
            return backend_guard.acquire(&path_buf, WatchMode::Recursive, poll_targets);
        }
    }

    /// Watch a directory non-recursively, e.g. a common git dir without `objects/**`.
    /// Watching the directory (not files) survives git's lock-then-rename.
    ///
    /// # Errors
    /// Returns an error if the path cannot be watched.
    pub fn watch_shallow<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let mut backend_guard = self.backend.lock_or_recover();
        backend_guard.acquire(path.as_ref(), WatchMode::Shallow, None)
    }

    /// Whether an already-armed watch delivers events for `path`.
    /// Lets callers skip redundant watches: on macOS adding a path rebuilds the
    /// `FSEventStream`, which can drop its first event.
    #[must_use]
    pub fn covers<P: AsRef<Path>>(&self, path: P) -> bool {
        self.backend.lock_or_recover().covers(path.as_ref())
    }

    /// Release one recursive watch of `path`; watches others still need stay armed.
    ///
    /// # Errors
    /// Returns an error if `path` is not watched recursively.
    pub fn unwatch<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        self.backend
            .lock_or_recover()
            .release(path.as_ref(), WatchMode::Recursive)
    }

    /// Release one shallow watch of `path`.
    ///
    /// # Errors
    /// Returns an error if `path` is not watched shallowly.
    pub fn unwatch_shallow<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        self.backend
            .lock_or_recover()
            .release(path.as_ref(), WatchMode::Shallow)
    }

    /// Process a notify event and trigger callback if relevant.
    fn process_event(
        event: &Event,
        callback_arc: &Arc<PendingCallback>,
        registry: &Arc<WatchRegistry>,
        followup_delay: Duration,
        schedule_delayed: &DelayedEventScheduler,
    ) {
        if event.need_rescan() {
            // Backend dropped events (queue overflow / MustScanSubDirs): rescan
            // affected roots through the normal debounced path.
            Self::handle_rescan(event, callback_arc, registry);
            return;
        }

        let worktree_enabled = registry.worktree_enabled();
        let worktree_ctx = WorktreeEventContext {
            event,
            registry,
            worktree_enabled,
        };

        // `Access` (inotify only) is never a change; inotify arms OPEN, so our own
        // reads (watch-arming walks, reloads) would otherwise loop as fake changes.
        // Real writes arrive as separate `Modify`/`Create`/`Remove` events.
        if matches!(event.kind, EventKind::Access(_)) {
            return;
        }

        for path in &event.paths {
            // Additive, not a fallback: the in-tree common dir path classifies
            // (and breaks) below, so a fallback arm would be unreachable.
            Self::emit_common_gitdir_fanout(path, registry, callback_arc);
            // Additive for the same reason; cannot double-emit with the above.
            Self::emit_submodule_parent_fanout(path, registry, callback_arc);

            if let Some(pending) = should_trigger_update(path, registry) {
                if Self::is_unregistered_common_gitdir_wake(&pending.repo, path, registry) {
                    // Drop the delivery but keep the once-per-event break.
                    break;
                }
                Self::emit_classified_event(&worktree_ctx, path, pending, callback_arc);
                // Once per event: no backend's multi-path event carries unique paths.
                break;
            }

            // Submodule/linked-worktree metadata lives in an external gitdir that
            // `should_trigger_update` cannot attribute; use the registration map.
            if let Some(repo) = registry.attribute_external_gitdir(path) {
                callback_arc(PendingEvent::git(repo, path.clone()));
                break;
            }

            // Permissive parent-guess fallback is pinned behavior here.
            if let Some(repo) = worktree_git_repo(
                event,
                path,
                registry,
                worktree_enabled,
                RepoEvidence::AnyAttribution,
            ) {
                Self::emit_attributed_worktree_event(
                    AttributedWorktreeEvent {
                        event,
                        path,
                        repo,
                        registry,
                    },
                    callback_arc,
                    followup_delay,
                    schedule_delayed,
                );
                break;
            }
        }
    }

    /// Deliver an event attributed by the watched-root/ancestor-walk arm.
    fn emit_attributed_worktree_event(
        attributed: AttributedWorktreeEvent<'_>,
        callback_arc: &Arc<PendingCallback>,
        followup_delay: Duration,
        schedule_delayed: &DelayedEventScheduler,
    ) {
        let AttributedWorktreeEvent {
            event,
            path,
            repo,
            registry,
        } = attributed;

        if Self::is_unregistered_common_gitdir_wake(&repo, path, registry) {
            return;
        }

        // A new directory's watch may arm after files land in it, dropping those
        // creates; schedule one coalesced follow-up refresh to catch them.
        if is_directory_create(event.kind, path) {
            Self::schedule_directory_followup(
                schedule_delayed,
                repo.clone(),
                path.to_path_buf(),
                followup_delay,
            );
        }
        callback_arc(PendingEvent::git(repo, path.to_path_buf()));
    }

    /// Whether this emission is a cross-repo wake from a common-gitdir watch on an
    /// unregistered main checkout, which must be dropped.
    fn is_unregistered_common_gitdir_wake(
        repo: &Path,
        path: &Path,
        registry: &WatchRegistry,
    ) -> bool {
        // Cheap check first.
        !registry.root_is_registered(repo) && registry.is_under_registered_common_gitdir(path)
    }

    /// Emit one Git event per linked-worktree root reading the shared metadata `path`.
    fn emit_common_gitdir_fanout(
        path: &Path,
        registry: &WatchRegistry,
        callback_arc: &Arc<PendingCallback>,
    ) {
        for repo in registry.attribute_common_gitdir(path) {
            callback_arc(PendingEvent::git(repo, path.to_path_buf()));
        }
    }

    /// Emit one Git event per registered superproject whose gitlink may change via
    /// submodule metadata at `path`; no other arm reaches these.
    fn emit_submodule_parent_fanout(
        path: &Path,
        registry: &WatchRegistry,
        callback_arc: &Arc<PendingCallback>,
    ) {
        let parents = registry.attribute_submodule_parents(path);
        if parents.is_empty() {
            return;
        }
        // Callbacks must not run under the registry lock.
        let registered = registry.filter_registered_roots(parents);

        for repo in registered {
            callback_arc(PendingEvent::git(repo, path.to_path_buf()));
        }
    }

    /// Deliver the classified event, plus a Git refresh for `Language`/`Config`/`Theme`
    /// when the path is in a genuine repo (e.g. `Cargo.toml` also changes status).
    fn emit_classified_event(
        worktree_ctx: &WorktreeEventContext<'_>,
        path: &Path,
        pending: PendingEvent,
        callback_arc: &Arc<PendingCallback>,
    ) {
        if matches!(
            pending.event,
            super::FileEvent::Language { .. }
                | super::FileEvent::Config { .. }
                | super::FileEvent::Theme { .. }
        ) && let Some(repo) = worktree_git_repo(
            worktree_ctx.event,
            path,
            worktree_ctx.registry,
            worktree_ctx.worktree_enabled,
            RepoEvidence::GenuineRepoOnly,
        ) {
            callback_arc(PendingEvent::git(repo, path.to_path_buf()));
        }

        if matches!(pending.event, super::FileEvent::Language { .. })
            && language_marker_is_ignored(path, worktree_ctx.registry)
        {
            return;
        }

        callback_arc(pending);
    }

    /// Emit one whole-repo Git event per watched root affected by an overflow.
    fn handle_rescan(event: &Event, callback_arc: &Arc<PendingCallback>, registry: &WatchRegistry) {
        for root in Self::affected_rescan_roots(event, registry) {
            callback_arc(PendingEvent::git_whole_repo(root));
        }
    }

    /// Watched roots a rescan event affects; a pathless event (inotify
    /// `IN_Q_OVERFLOW`) affects all of them.
    fn affected_rescan_roots(event: &Event, registry: &WatchRegistry) -> Vec<PathBuf> {
        if event.paths.is_empty() {
            return registry.root_snapshot();
        }

        let mut roots: Vec<PathBuf> = event
            .paths
            .iter()
            .filter_map(|path| attribute_repo(path, registry))
            .collect();
        roots.sort();
        roots.dedup();
        roots
    }

    /// Schedule one coalesced follow-up Git refresh for `repo`, due after `delay`.
    /// Debouncer-owned so bursts collapse and nothing outlives `stop`.
    fn schedule_directory_followup(
        schedule_delayed: &DelayedEventScheduler,
        repo: PathBuf,
        dir_path: PathBuf,
        delay: Duration,
    ) {
        // `Instant + Duration` panics on overflow.
        let due = Instant::now()
            .checked_add(delay)
            .unwrap_or_else(Instant::now);
        schedule_delayed(PendingEvent::git(repo, dir_path), due);
    }

    /// Stop the file watcher and clean up.
    pub fn stop(&mut self) {
        self.stop_flag.store(true, Ordering::Relaxed);

        // Scoped: the probe may block on this mutex mid-swap, so release before joining.
        self.backend.lock_or_recover().watcher = None;

        if let Some(tx) = self.event_tx.take() {
            let _ = tx.send(Event::new(EventKind::Other));
        }

        // The probe checks `stop_flag` each iteration, so this returns promptly.
        if let Some(handle) = self.probe_handle.take()
            && let Err(e) = handle.join()
        {
            crate::debug::write_debug_log(
                "watcher",
                &format!("probe worker thread panicked: {e:?}"),
            );
        }
        if let Some(handle) = self.handle.take()
            && let Err(e) = handle.join()
        {
            crate::debug::write_debug_log(
                "watcher",
                &format!("filesystem watcher worker thread panicked: {e:?}"),
            );
        }
    }
}

impl Drop for FileSystemWatcher {
    fn drop(&mut self) {
        self.stop();
    }
}

fn is_in_git_dir(path: &Path) -> bool {
    path.components()
        .any(|component| component.as_os_str() == OsStr::new(".git"))
}

/// Whether `worktree_git_repo` may accept an evidence-free `ParentGuess`.
#[derive(Clone, Copy)]
enum RepoEvidence {
    /// Accept any attribution, including `ParentGuess`.
    AnyAttribution,
    /// Reject `ParentGuess`; used for the extra Git emission on Language/Config
    /// paths (e.g. `~/.config/gpy/config.toml` is not a repo).
    GenuineRepoOnly,
}

/// Repo root if `path` passes every worktree Git-refresh filter, shared so both
/// emission arms apply identical rules.
fn worktree_git_repo(
    event: &Event,
    path: &Path,
    registry: &Arc<WatchRegistry>,
    worktree_enabled: bool,
    repo_evidence: RepoEvidence,
) -> Option<PathBuf> {
    if !worktree_enabled {
        return None;
    }
    // `PollWatcher` reports file edits only as `Modify(Metadata(WriteTime))`, so
    // metadata must be accepted. `AccessTime` can't change `git status`; exclude it.
    if !matches!(
        event.kind,
        EventKind::Create(_)
            | EventKind::Remove(_)
            | EventKind::Modify(
                ModifyKind::Name(_)
                    | ModifyKind::Data(_)
                    | ModifyKind::Any
                    | ModifyKind::Metadata(
                        MetadataKind::Any
                            | MetadataKind::WriteTime
                            | MetadataKind::Permissions
                            | MetadataKind::Ownership
                            | MetadataKind::Extended
                            | MetadataKind::Other
                    )
            )
    ) {
        return None;
    }
    if is_in_git_dir(path) {
        return None;
    }
    // Attribute first: ignore tokens are scoped relative to the repo root.
    let attribution = attribute_repo_detailed(path, registry)?;
    if matches!(repo_evidence, RepoEvidence::GenuineRepoOnly) && !attribution.is_genuine_repo() {
        return None;
    }
    // In a trusted root a directory token (`vendor/`...) defers to gitignore,
    // since committed files live there.
    match path_matches_ignored_patterns(path, attribution.trusted_root()) {
        Some(IgnoreTokenHit::Noise) => return None,
        Some(IgnoreTokenHit::Directory) if attribution.trusted_root().is_none() => return None,
        Some(IgnoreTokenHit::Directory) | None => {}
    }
    let repo = attribution.into_repo();
    // Ignored files never show in `git status`.
    if path_is_gitignored(&repo, path, registry) {
        return None;
    }
    Some(repo)
}

/// Whether a `Create` event is a directory. `FSEvents` emits `Any`/`Other`, so stat
/// best-effort; a missed follow-up after a race is caught by reconcile.
fn is_directory_create(kind: EventKind, path: &Path) -> bool {
    match kind {
        EventKind::Create(CreateKind::Folder) => true,
        EventKind::Create(CreateKind::Any | CreateKind::Other) => path.is_dir(),
        _ => false,
    }
}

/// Whether `event` is the poll backend's directory-mtime echo of a child change;
/// the child event carries the real change, so this one is dropped.
fn is_poll_directory_mtime_noise(event: &Event) -> bool {
    matches!(
        event.kind,
        EventKind::Modify(ModifyKind::Metadata(MetadataKind::WriteTime))
    ) && !event.paths.is_empty()
        && event.paths.iter().all(|path| path.is_dir())
}

/// How an event path was attributed to a repo.
enum RepoAttribution {
    /// Under a registered watched root: a trustworthy repo boundary.
    WatchedRoot(PathBuf),
    /// Found via an ancestor `.git` walk: genuinely in a repo, but not registry-backed.
    AncestorGitDir(PathBuf),
    /// Bare parent-directory guess with no `.git` evidence.
    ParentGuess(PathBuf),
}

impl RepoAttribution {
    /// The attributed repo root.
    fn into_repo(self) -> PathBuf {
        match self {
            Self::WatchedRoot(root) | Self::AncestorGitDir(root) | Self::ParentGuess(root) => root,
        }
    }

    /// Root to scope ignore tokens against; only `WatchedRoot` is trusted.
    fn trusted_root(&self) -> Option<&Path> {
        match self {
            Self::WatchedRoot(root) => Some(root.as_path()),
            Self::AncestorGitDir(_) | Self::ParentGuess(_) => None,
        }
    }

    /// Whether `path` is genuinely inside a repository.
    const fn is_genuine_repo(&self) -> bool {
        !matches!(self, Self::ParentGuess(_))
    }
}

/// Attribute an event path to the watched repo that produced it.
///
/// Longest watched-root prefix wins (zero stats): a watched nested repo wins, while
/// an unwatched one never had its own watch, so the outer root is correct. Falls
/// back to an ancestor `.git` walk, then the parent directory.
fn attribute_repo_detailed(path: &Path, registry: &WatchRegistry) -> Option<RepoAttribution> {
    if let Some(root) = registry.longest_root_prefix(path) {
        return Some(RepoAttribution::WatchedRoot(root));
    }

    if let Some(root) = crate::git::find_git_dir_only(path) {
        return Some(RepoAttribution::AncestorGitDir(root));
    }

    path.parent()
        .map(Path::to_path_buf)
        .map(RepoAttribution::ParentGuess)
}

/// Attribute `path` to its repo root, discarding provenance.
fn attribute_repo(path: &Path, registry: &WatchRegistry) -> Option<PathBuf> {
    attribute_repo_detailed(path, registry).map(RepoAttribution::into_repo)
}

#[cfg(test)]
mod tests;

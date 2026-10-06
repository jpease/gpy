//! Main agent process implementation
//!
//! Orchestrates the background agent process including IPC server startup,
//! client management, and graceful shutdown handling.
//!
//! # Architecture Overview
//!
//! The agent is the central daemon process that coordinates four concurrent subsystems:
//!
//! 1. **IPC Server** ([`crate::ipc::server::EndpointHandle`]) - Handles Fish shell requests
//! 2. **File Watcher** ([`crate::watcher::multi_repo::MultiRepoWatcher`]) - Detects git/config changes
//! 3. **Clock Timer** - Rings the periodic repaint doorbell (SIGURG) for live clock updates
//! 4. **Pruning Timer** - Removes dead client PIDs every 60 seconds
//!
//! These run concurrently via `tokio::select!` in the main event loop (see `start_background`).
//!
//! # Coordination Points (Critical for AI Agents)
//!
//! **RULE**: Cross-module coordination MUST happen in this agent module. Independent subsystems
//! (git, language, watcher, config, theme) should NOT directly coordinate with each other.
//! All integration logic flows through this agent module to maintain architectural clarity.
//!
//! ## 1. Git Status → Cache Invalidation
//!
//! **What**: Git file changes trigger cache invalidation and refresh
//!
//! **Where**: `events::handle_file_event()` function
//!
//! **Flow**:
//! ```text
//! File watcher detects .git change
//!   → DebouncedEvent::Git
//!   → handle_file_event()
//!   → GitStatusCache::invalidate()
//!   → load_repository_state()
//!   → GitStatusCache::set()
//!   → ClientDirectory::notify_repaint_force()
//! ```
//!
//! **Why here**: Git module (`git::status`) owns status detection, cache module (`git::cache`)
//! owns caching policy, but this agent module coordinates the invalidation → refresh → notify
//! sequence. This prevents circular dependencies between git and cache modules.
//!
//! **AI Agent Note**: When adding new cache invalidation triggers, add them to `handle_file_event()`.
//! Do NOT add invalidation logic directly to git or cache modules.
//!
//! ## 2. File Watcher → Client Notifications (repaint doorbell)
//!
//! **What**: Filesystem changes trigger prompt re-rendering in Fish shells
//!
//! **Where**: `events::handle_file_event()` function
//!
//! **Flow**:
//! ```text
//! File change detected
//!   → Watcher debouncer
//!   → handle_file_event()
//!   → [Update relevant cache/state]
//!   → ClientDirectory::notify_repaint_force()
//!   → shells receive SIGURG (the doorbell; default disposition is ignore)
//!   → shell re-renders prompt
//! ```
//!
//! **Why here**: Watcher module (`watcher::multi_repo`) owns file watching, client registry
//! (`ipc::registry`) owns client management, but this agent module decides WHEN to notify
//! clients based on event types and configuration state.
//!
//! **AI Agent Note**: All client notifications must flow through `ClientDirectory` methods.
//! Never send signals directly from watcher or other modules. Use `notify_repaint_force()`
//! for immediate updates or `notify_repaint()` for throttled updates.
//!
//! ## 3. Config Changes → Subsystem Reload (reload flag + doorbell)
//!
//! **What**: Configuration file changes trigger theme export reload in Fish shells
//!
//! **Where**: `events::handle_file_event()` function, FileEvent::Config case
//!
//! **Flow**:
//! ```text
//! Config file changed (.toml or theme files)
//!   → Watcher detects FileEvent::Config
//!   → handle_file_event() logs the event
//!   → ThemeManager calls `ClientDirectory::notify_reload()` (writes `<pid>.reload`, rings SIGURG)
//!   → Fish shells re-source theme export
//! ```
//!
//! **Why here**: Config module (`config::loader`) owns file parsing, theme module (`theme::manager`)
//! owns theme generation, but this agent module coordinates and ensures reload signals
//! propagate correctly.
//!
//! **AI Agent Note**: When adding new config-driven features, ensure the affected state
//! is exported via `theme export`. The reload doorbell is rung by the `ThemeManager` callback,
//! not directly in this function. There is no config/theme cache to invalidate any more
//! (#608) -- `ConfigManager`/`ThemeManager` each hold one live `Arc<RwLock<Arc<_>>>` state
//! that is always current.
//!
//! ## 4. Clock Timer → Client Updates (repaint doorbell)
//!
//! **What**: Periodic timer sends signals to update clock segment in prompts
//!
//! **Where**: `start_background()` method, clock timer branch
//!
//! **Flow**:
//! ```text
//! tokio::interval tick (1s or 60s)
//!   → start_background() event loop
//!   → Check if clients registered
//!   → ClientDirectory::notify_repaint()
//!   → Fish shells re-render clock segment
//! ```
//!
//! **Why here**: Clock logic could live in a separate module, but centralizing timer
//! coordination in the agent's event loop makes shutdown and signal throttling simpler.
//!
//! **AI Agent Note**: Only rings the doorbell if registered clients exist (see `clients.len() > 0` check).
//! This prevents unnecessary timer processing when no Fish shells are connected. To add new
//! timer-based updates, add another `tokio::interval` to the `tokio::select!` block.
//!
//! ## 5. IPC Requests → Watcher Registration
//!
//! **What**: IPC requests for git status trigger automatic watcher registration
//!
//! **Where**: `EndpointHandle::serve_connection()` via `Agent::new()` (server initialization)
//!
//! **Flow**:
//! ```text
//! Fish sends RepositoryStatus request
//!   → EndpointHandle::serve_connection()
//!   → [Loads git status]
//!   → Watcher auto-registers repository (via shared watcher_slot)
//!   → Future file changes trigger notifications
//! ```
//!
//! **Why here**: IPC server (`ipc::server`) owns request handling, watcher (`watcher::multi_repo`)
//! owns file monitoring, but this agent module provides the shared `watcher_slot` that allows
//! lazy registration. The `Agent` struct holds both components and coordinates their interaction.
//!
//! **AI Agent Note**: Watcher registration is automatic when enabled via config. The `watcher_slot`
//! is an `Arc<Mutex<Option<MultiRepoWatcher>>>` pattern allowing the server to register repos
//! without holding permanent watcher references. This prevents reference cycles.
//!
//! ## 6. Client Registration → Watcher Workspace Tracking
//!
//! **What**: Fish shell registration links working directory to watcher for targeted updates
//!
//! **Where**: `EndpointHandle::serve_connection()` handling `Message::RegisterClient`
//!
//! **Flow**:
//! ```text
//! Fish sends RegisterClient { pid, cwd }
//!   → Server stores in ClientDirectory
//!   → Watcher uses workspace info for targeted repaint doorbells
//!   → Only affected clients get notification
//! ```
//!
//! **Why here**: Client registry and watcher share state through the agent's initialization.
//! This coordination prevents all clients from being notified on every file change.
//!
//! **AI Agent Note**: Workspace tracking is optional but improves performance. The registry
//! tracks client PIDs and CWDs. When a file changes, only clients in affected workspace
//! receive signals. See `ClientDirectory::notify_repaint()`'s `target`.
//!
//! ## Coordination Architecture Diagram
//!
//! ```text
//!                    ┌─────────────────────┐
//!                    │   Agent (This)      │
//!                    │  Coordination Hub   │
//!                    └──────────┬──────────┘
//!                               │
//!           ┌───────────────────┼───────────────────┐
//!           │                   │                   │
//!      ┌────▼────┐         ┌────▼────┐        ┌────▼────┐
//!      │   IPC   │         │ Watcher │        │  Cache  │
//!      │ Server  │         │  Multi  │        │  Git    │
//!      └─────────┘         │  Repo   │        │ Status  │
//!           │              └─────────┘        └─────────┘
//!           │                   │                   │
//!      ┌────▼────┐         ┌────▼────┐        ┌────▼────┐
//!      │ Client  │         │   Git   │        │  Theme  │
//!      │Registry │         │ Status  │        │ Manager │
//!      └─────────┘         └─────────┘        └─────────┘
//! ```
//!
//! **Key Principle**: Arrows represent data flow. All arrows MUST pass through the Agent
//! coordination hub. Direct communication between subsystems is prohibited to maintain
//! architectural clarity and prevent circular dependencies.
//!
//! ## Adding New Coordination Points
//!
//! When implementing features that require coordination between subsystems:
//!
//! 1. **Identify subsystems**: Which modules need to interact? (e.g., config + cache)
//! 2. **Choose coordination point**: Add logic to agent module, never to individual subsystems
//! 3. **Document flow**: Add flow diagram to this coordination section
//! 4. **Update tests**: Add integration test validating the coordination
//! 5. **Respect boundaries**: Keep subsystems independent, agent module orchestrates
//!
//! **Example**: If adding Docker container detection that affects both language detection
//! and git status, add coordination logic to `handle_file_event()` or create a new handler
//! method in this agent module. Do NOT add cross-module calls in language or git modules.
//!
//! ## How to Add Shared State (For AI Agents)
//!
//! When you need to share state across IPC handlers (e.g., metrics, caches, trackers):
//!
//! **1. Create the state struct with interior mutability:**
//! ```rust
//! use std::sync::Mutex;
//!
//! pub struct MyTracker {
//!     data: Mutex<Vec<u64>>,  // Use Mutex for mutable state
//! }
//!
//! impl MyTracker {
//!     pub fn new() -> Self {
//!         Self { data: Mutex::new(Vec::new()) }
//!     }
//!
//!     pub fn record(&self, value: u64) {
//!         self.data.lock().unwrap().push(value);
//!     }
//! }
//! ```
//!
//! **2. Initialize in `Agent::new()` (this file):**
//! ```text
//! let my_tracker = Arc::new(MyTracker::new());
//! ```
//!
//! **3. Add field to `EndpointHandle` struct (`ipc/server/handle.rs`):**
//! ```text
//! pub struct EndpointHandle {
//!     // ... existing fields ...
//!     my_tracker: Arc<MyTracker>,
//! }
//! ```
//!
//! **4. Add the field to `EndpointHandleBuilder` (`ipc/server/builder.rs`):**
//! - Add an `Option<Arc<MyTracker>>` field to the builder struct
//! - Add a `.my_tracker(tracker: Arc<MyTracker>)` setter, matching the pattern
//!   already used by `.instant_cache(...)` / `.language_cache(...)`
//! - In `build()`, extract it (`.ok_or_else(...)` for a required field, or
//!   `.unwrap_or_else(...)`/`.unwrap_or_default()` for an optional one) and
//!   pass it through to `EndpointHandle::with_path_and_state()`
//! - All construction goes through `EndpointHandle::builder()...build()` — see
//!   the Builder Pattern Requirements in `.claude/rules/gpy-rust.md`; there is
//!   no positional constructor to update instead
//!
//! **5. Clone and pass in `spawn_client_handler()` (`ipc/server/handle.rs`):**
//! ```text
//! let my_tracker = Arc::clone(&self.my_tracker);
//! tokio::spawn(async move {
//!     // ... client handler uses my_tracker ...
//! });
//! ```
//!
//! **6. Update test files (search for `EndpointHandle::builder()`):**
//! ```text
//! let my_tracker = Arc::new(MyTracker::new());
//! let server = EndpointHandle::builder()
//!     // ... other required fields ...
//!     .my_tracker(my_tracker)
//!     .build()?;
//! ```
//! Tests that only need a minimal handle can instead use
//! `EndpointHandle::new_test_handle()`.
//!
//! **Common Patterns:**
//! - **Mutable state**: `Arc<Mutex<T>>` - Use for metrics, counters, buffers
//! - **Immutable/config state**: `Arc<T>` - Use for readonly config, static data
//! - **Read-heavy state**: `Arc<RwLock<T>>` - Use when reads >> writes
//!
//! **Test file locations to update:**
//! - `gpy-agent/tests/integration_tests.rs`
//! - `gpy-agent/tests/e2e_ipc_tests.rs`
//! - `gpy-agent/tests/fork_daemon_tests.rs`
//! - Any test using `EndpointHandle::builder()` (search project-wide)
//!
//! **Example**: See `LatencyTracker` implementation in `ipc/latency.rs` for a complete
//! example of adding shared metrics state.
//!
//! ## Async Task Orchestration
//!
//! **Event loop structure** (tokio::select! in start_background()):
//! ```text
//! tokio::select! {
//!     server_result = server_future => {...}      // IPC server (long-lived)
//!     shutdown = shutdown_signal => {...}         // SIGTERM/SIGINT handler
//!     _ = clock_timer.tick() => {...}             // Clock timer (1s poll; see "Clock Timer Design")
//!     _ = pruning_timer.tick() => {...}           // Client pruning (60s interval)
//!     _ = reconcile_timer.tick() => {...}         // Reconcile scan (defense-in-depth catch-up
//!                                                  // for filesystem events the watcher may
//!                                                  // have dropped; re-scans watched repos and
//!                                                  // only notifies on a real diff)
//! }
//! ```
//!
//! **Why tokio::select?** Allows the agent to respond to shutdown signals immediately
//! while the IPC server and clock timer run in parallel. Without select, we'd need
//! complex channel-based coordination between tasks.
//!
//! **Thread safety**: All shared state uses `Arc<T>` for safe sharing across async tasks:
//! - `ClientDirectory` - Registry of connected Fish processes
//! - `GitStatusCache` - Cached git status to avoid redundant queries
//! - `MultiRepoWatcher` - Shared reference for registration coordination
//!
//! ## Clock Timer Design
//!
//! **Purpose**: Update the clock segment in the prompt without user interaction.
//!
//! **Implementation**: `startup::create_clock_timer()` always creates a fixed
//! `tokio::time::interval(Duration::from_secs(1))` — there is no minute-aligned
//! or variable-length interval. Every tick of this 1-second poll runs the
//! `clock_timer` arm of the event loop's `tokio::select!`.
//!
//! **Whether a tick actually notifies clients is a separate decision**, made by
//! `startup::clock_signal_decision(show_seconds, current_minute, last_notified_minute)`,
//! where the caller (this event loop) computes `current_minute` from the wall
//! clock and passes it in:
//! - `show_seconds=true`: returns "send" on every tick (stopwatch-style display).
//! - `show_seconds=false`: returns "send" only when the current wall-clock
//!   minute differs from `last_notified_minute` — i.e. once per minute
//!   rollover, even though the underlying timer is still polling every second.
//!
//! So the 1-second poll is constant; what varies is purely the gate deciding
//! whether that poll results in a repaint doorbell broadcast.
//!
//! **Why not OS signals?** Could use a cron-like approach, but `tokio::time::interval`
//! is simpler, testable, and doesn't require external dependencies or system config.
//!
//! **Signal throttling**: Only rings the doorbell if registered clients exist. When no Fish
//! processes are connected, timer ticks are no-ops.
//!
//! ## File Event Handling
//!
//! **Flow**: File change → Watcher → Debouncer → `handle_file_event` → repaint doorbell (SIGURG)
//!
//! **Coalescing, not a freshness cooldown** (see `events::handle_file_event`,
//! `events::refresh_and_notify_coalesced`, and the per-repo
//! `events::RepoRefreshCoordinator`): concurrent file events for the same repo
//! are coalesced into a single in-flight scan rather than skipped based on a
//! time-based cache-freshness check. The first caller for a repo becomes the
//! sole owner of that repo's scan; any change that arrives while a scan is
//! already in flight is recorded as pending instead of starting a second,
//! concurrent scan. Once the in-flight scan finishes, the owner drains any
//! pending change with exactly one follow-up scan (repeating until nothing is
//! pending, bounded by a defensive follow-up cap), so no change is lost and no
//! two scans for the same repo ever run at once. Scans for *different* repos
//! still run fully concurrently — only same-repo scans are serialized.
//!
//! **Why coalesce?** A burst of filesystem events for one repo (e.g. `git
//! commit` touching several files in `.git`) would otherwise trigger a
//! redundant git-status subprocess per event. Coalescing collapses the burst
//! into one scan (plus, if needed, one catch-up scan for anything that arrived
//! mid-scan) instead of skipping events outright, so the final notified state
//! always reflects the latest change.
//!
//! ## Graceful Shutdown
//!
//! **Signal handling**: SIGTERM and SIGINT both trigger graceful shutdown
//! (see `startup::create_shutdown_signal`).
//!
//! **Shutdown sequence**:
//! 1. Stop file watcher (prevents new events)
//! 2. Stop theme/config file watchers
//! 3. Drop the in-flight server future (releases the borrow on `self.server`)
//!    and explicitly call `self.server.stop()`, which unlinks the socket file
//! 4. Print "GPY Agent stopped" message and return from `start_background`
//!
//! **Why explicit `stop()`?** The accept loop only unlinks the socket at the end
//! of its own loop body, which normally only runs after an IPC `Shutdown`
//! message. A signal short-circuits the event loop instead, dropping the
//! server future mid-`accept()` — without an explicit `stop()` call here, the
//! socket file would leak (#304).
//!
//! **Why watcher before server?** Stopping the watcher first prevents events
//! from queuing up during shutdown.

// Module declarations
pub mod events;
/// Agent lifecycle management (start/stop/status)
pub mod lifecycle;
pub mod oneshot;
pub mod startup;

use crate::debug_log;
use crate::git::cache::GitStatusCache;
use crate::ipc::{ClientDirectory, server::EndpointHandle};
use crate::theme::ThemeManager;
use crate::watcher::{WatcherConfig, multi_repo::MultiRepoWatcher};
use crate::{Error, Result};
use events::SharedWatcherSlot;
use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

fn matches_ignore_case(value: &str, truthy: &[&str]) -> bool {
    truthy
        .iter()
        .any(|candidate| value.trim().eq_ignore_ascii_case(candidate))
}

/// Whether the `GPY_DISABLE_WATCHER` env var requests the file watcher be disabled.
///
/// Checked at both `setup_hot_reload` (hot-reload's own re-init-on-config-change
/// path) and `init_watcher` (initial startup), which must agree so a reload
/// never re-enables what startup deliberately left off.
fn watcher_disabled_by_env() -> bool {
    env::var("GPY_DISABLE_WATCHER")
        .is_ok_and(|value| matches_ignore_case(&value, &["1", "true", "yes", "on"]))
}

/// Default interval for the periodic reconcile that catches dropped
/// filesystem events. Chosen above the git-status cache TTL so reconcile
/// scans see fresh state.
const RECONCILE_INTERVAL_SECS: u64 = 45;

/// Parse a raw `GPY_RECONCILE_INTERVAL_SECS` value into an interval in
/// seconds.
///
/// `None` when unset, non-numeric, or `0` — the caller falls back
/// to `RECONCILE_INTERVAL_SECS` in that case. Pure function, directly
/// testable without touching the real process environment (this crate
/// forbids `unsafe_code`, and `std::env::set_var` requires it).
fn parse_reconcile_interval_secs(raw: Option<&str>) -> Option<u64> {
    raw?.parse::<u64>().ok().filter(|value| *value > 0)
}

/// Resolve the reconcile interval: `GPY_RECONCILE_INTERVAL_SECS` overrides
/// the default when set to a positive integer (seconds); otherwise falls
/// back to `RECONCILE_INTERVAL_SECS`.
///
/// This is a defense-in-depth knob for environments where the file
/// watcher's OS-level notifications (`FSEvents` on macOS) never arrive — see
/// #354. Tightening this interval does not add new watching machinery; it
/// just runs the existing reconcile scan (`refresh_and_notify_if_changed`
/// per watched repo) more often.
fn reconcile_interval_secs() -> u64 {
    let raw = std::env::var("GPY_RECONCILE_INTERVAL_SECS").ok();
    parse_reconcile_interval_secs(raw.as_deref()).unwrap_or(RECONCILE_INTERVAL_SECS)
}

/// Runs `scan_fn` over `repos` on a blocking thread (git status is a subprocess call and must
/// never block the event loop).
///
/// No-op when `repos` is empty. A panic inside `scan_fn` is caught by `spawn_blocking`'s
/// `JoinHandle` and logged, matching the pre-#417 behavior.
async fn run_reconcile_pass<F>(repos: Vec<PathBuf>, scan_fn: F)
where
    F: Fn(Vec<PathBuf>) + Send + 'static,
{
    if repos.is_empty() {
        return;
    }
    let result = tokio::task::spawn_blocking(move || scan_fn(repos)).await;
    if let Err(join_err) = result {
        debug_log!("agent", "Reconcile scan task failed: {}", join_err);
    }
}

/// Runs one reconcile scan pass over `initial_repos`, then catches up a skipped tick if one
/// landed while that pass was running.
///
/// A tick skipped (due to the in-flight guard) while the first pass was running gets exactly
/// one catch-up pass before `in_flight` is released.
///
/// See #417: without this catch-up, a tick skipped during a slow scan was only caught up at
/// the *next* interval, doubling worst-case staleness.
///
/// Generic over `roots_fn` (how to fetch the currently-watched repo roots
/// for the catch-up pass) and `scan_fn` (the actual per-repo scan) so this
/// function is unit-testable with a mocked slow scan, without spinning up a
/// real `MultiRepoWatcher` or running real git subprocesses. Production code
/// (the reconcile-tick arm in [`Agent::start_background`]) passes a
/// `roots_fn` that reads `SharedWatcherSlot::watched_roots()` and a
/// `scan_fn` that calls [`events::refresh_and_notify_if_changed`] per repo.
///
/// # Single-flight contract
///
/// Callers must set `in_flight` to `true` (via `swap`) *before* spawning
/// this function, and must never spawn a second concurrent call while
/// `in_flight` is `true`. This function is the sole owner of clearing
/// `in_flight`, and only does so once both the initial scan and any
/// catch-up pass have completed — so at no point can two scans run
/// concurrently.
///
/// # Bounding the catch-up
///
/// At most one catch-up pass runs per invocation: `pending` is checked
/// (and cleared) exactly once, after the initial pass. If a *new* tick sets
/// `pending` again while the catch-up pass itself is running, that flag is
/// left set for the *next* invocation to pick up — this function never
/// loops.
async fn run_reconcile_scan<R, F>(
    initial_repos: Vec<PathBuf>,
    in_flight: Arc<AtomicBool>,
    pending: Arc<AtomicBool>,
    roots_fn: R,
    scan_fn: F,
) where
    R: Fn() -> Vec<PathBuf> + Send + 'static,
    F: Fn(Vec<PathBuf>) + Send + Clone + 'static,
{
    run_reconcile_pass(initial_repos, scan_fn.clone()).await;

    // A tick that fired while the pass above was in flight was skipped by
    // the tick arm's guard and recorded as `pending` instead of starting a
    // second concurrent scan. Run exactly one catch-up pass for it here,
    // re-fetching watched roots since the skipped tick may reflect repos
    // added/removed after `initial_repos` was snapshotted.
    if pending.swap(false, Ordering::AcqRel) {
        debug_log!(
            "agent",
            "Reconcile catch-up: running scan for tick skipped while previous scan was in flight"
        );
        let catch_up_repos = roots_fn();
        run_reconcile_pass(catch_up_repos, scan_fn).await;
    }

    in_flight.store(false, Ordering::Release);
}

/// Prune dead clients from `registry` and unregister them from the watcher.
///
/// Returns the pruned PIDs so a caller can do its own follow-up work with them.
///
/// A free function rather than a method on [`Agent`] because the `select!` loop
/// in [`Agent::start_background`] cannot call one: the in-flight
/// `self.server.start()` future holds an exclusive borrow of `self.server` for
/// the whole loop, and any `&self` call would have to re-borrow the entire
/// `Agent`. Taking exactly the two pieces this needs sidesteps that, so the
/// pruning arm and [`Agent::prune_dead_clients_and_cleanup`] share one
/// implementation instead of two copies.
fn prune_dead_clients(registry: &ClientDirectory, watcher_slot: &SharedWatcherSlot) -> Vec<u32> {
    let mut pruned_pids = registry.prune_dead_clients();

    // Also unregister from watcher to clean up filesystem watches
    if let Ok(guard) = watcher_slot.lock()
        && let Some(watcher) = guard.clone()
    {
        for pid in &pruned_pids {
            let _ = watcher.unregister_client(*pid);
        }

        // A doorbell broadcast drops clients from the registry alone (#782);
        // drop whatever the watcher still tracks for them. Snapshot first so
        // the watcher is never called with registry state held.
        for pid in watcher.client_pids() {
            if !registry.is_registered(pid) {
                let _ = watcher.unregister_client(pid);
                if !pruned_pids.contains(&pid) {
                    pruned_pids.push(pid);
                }
            }
        }
    }

    if !pruned_pids.is_empty() {
        debug_log!(
            "agent",
            "Pruned {} dead clients from registry",
            pruned_pids.len()
        );
    }

    pruned_pids
}

/// Decide whether this clock tick warrants a repaint doorbell, and ring it if so.
///
/// Returns the `last_notified_minute` the caller should carry into the next
/// tick — unchanged when the clock segment is disabled, so a later tick that
/// finds it re-enabled still compares against the last minute actually
/// broadcast.
fn handle_clock_tick(
    ctx: &events::AgentContext,
    registry: &ClientDirectory,
    last_notified_minute: u64,
) -> u64 {
    let config = ctx.config_manager.get();

    // Skip broadcast when the clock segment is not enabled
    if !config.ui.enabled_segments.iter().any(|s| s == "clock") {
        return last_notified_minute;
    }

    let clock_show_seconds = ctx
        .theme_manager
        .get()
        .segments
        .clock
        .show_seconds
        .unwrap_or(false);
    debug_log!("agent", "Clock tick: show_seconds={}", clock_show_seconds);

    let current_minute = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
        / 60;
    let (should_send, new_minute) =
        startup::clock_signal_decision(clock_show_seconds, current_minute, last_notified_minute);

    if should_send {
        let client_count = registry.len();
        debug_log!(
            "agent",
            "Clock: ringing repaint doorbell for {} clients",
            client_count
        );
        if client_count > 0 {
            registry.notify_repaint(None);
        }
    } else {
        debug_log!("agent", "Clock: should_send=false, skipping this tick");
    }

    new_minute
}

/// Start a reconcile scan for this tick unless one is already running.
///
/// When a scan is in flight this only records the tick as pending, so the
/// in-flight scan's catch-up pass picks it up as soon as it finishes instead of
/// waiting a full interval (#417). Otherwise it snapshots the watched repos and,
/// if there is nothing to scan, clears `in_flight` itself right here; if there
/// is, it spawns [`run_reconcile_scan`], which then owns clearing `in_flight`
/// once the scan (and any catch-up pass) completes. Exactly one of the two
/// functions ends up clearing the flag for a given call, depending on whether
/// `repos` was empty.
fn spawn_reconcile_scan(
    ctx: &events::AgentContext,
    watcher_slot: &SharedWatcherSlot,
    config: Arc<crate::config::Config>,
    in_flight: &Arc<AtomicBool>,
    pending: &Arc<AtomicBool>,
) {
    if in_flight.swap(true, Ordering::AcqRel) {
        // A scan is already running: record that this tick's work still
        // needs to happen, and let the in-flight scan's catch-up pass (see
        // run_reconcile_scan) pick it up as soon as it finishes, instead of
        // waiting for the next full interval (#417).
        pending.store(true, Ordering::Release);
        debug_log!(
            "agent",
            "Reconcile tick skipped: previous reconcile scan still in flight; marked pending for catch-up"
        );
        return;
    }

    let repos = watcher_slot
        .lock()
        .ok()
        .and_then(|guard| guard.as_ref().map(|w| w.watched_roots()))
        .unwrap_or_default();
    if repos.is_empty() {
        in_flight.store(false, Ordering::Release);
        return;
    }

    // Git status runs a subprocess; offload so the event loop never blocks on
    // the reconcile scan. The blocking scan is wrapped in an outer task so its
    // JoinHandle can be awaited: a bare spawn_blocking whose handle nobody
    // awaits would let a panic inside the scan vanish silently and would never
    // clear the in-flight guard (#323).
    let reconcile_ctx = ctx.clone();
    let watcher_for_reconcile = Arc::clone(watcher_slot);
    let in_flight_for_scan = Arc::clone(in_flight);
    let pending_for_scan = Arc::clone(pending);
    let scan_fn = move |scan_repos: Vec<PathBuf>| {
        for git_root in scan_repos {
            // Route through the coalescing wrapper so a reconcile scan for a
            // repo can never run concurrently with a watcher-dispatched scan
            // for that same repo (#418).
            events::refresh_and_notify_coalesced(&reconcile_ctx, &git_root, &config, None);
        }
    };
    let roots_fn = move || {
        watcher_for_reconcile
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|w| w.watched_roots()))
            .unwrap_or_default()
    };
    tokio::spawn(run_reconcile_scan(
        repos,
        in_flight_for_scan,
        pending_for_scan,
        roots_fn,
        scan_fn,
    ));
}

/// Main agent process that coordinates all subsystems
pub struct Agent {
    server: EndpointHandle,
    watcher: SharedWatcherSlot,
    /// The one place the agent's shared subsystem handles live (#587): the
    /// config/theme managers, the client registry, the caches, and the refresh
    /// coordinator. Previously the theme manager, config manager and client
    /// registry were *also* held as three separate `Agent` fields cloned from
    /// this very context, so each of those `Arc`s was stored twice.
    ctx: events::AgentContext,
}

impl Agent {
    fn with_watcher<F>(&self, f: F)
    where
        F: FnOnce(&MultiRepoWatcher),
    {
        if let Ok(guard) = self.watcher.lock()
            && let Some(watcher) = guard.as_ref()
        {
            f(watcher);
        }
    }

    /// Create a new agent instance
    ///
    /// # Errors
    ///
    /// Returns an error if the IPC server cannot be created or if the file watcher
    /// cannot be initialized.
    ///
    /// # Panics
    ///
    /// Panics if the default theme cannot be loaded (this should never happen as the
    /// default theme is bundled with the application).
    pub fn new() -> Result<Self> {
        let watcher_config = WatcherConfig::from_env();
        let client_registry = ClientDirectory::new().shared();
        client_registry.set_throttle_ms(watcher_config.throttle_ms());
        let git_cache = Arc::new(GitStatusCache::new());

        let config_manager = Self::init_config_manager()?;
        let initial_config = config_manager.get();
        crate::language::version::set_version_cache_ttl(
            initial_config.language.cache_ttl_hours.get(),
        );
        // One registry for the whole agent, shared by the repo, config and
        // theme coordinators: `classify_event` consults the registered config
        // paths on every coordinator, so they must all see the same set (#617).
        let watch_registry = Arc::new(crate::watcher::WatchRegistry::new());
        watch_registry.set_worktree_enabled(initial_config.git.watch_worktree);

        let theme_manager = Self::init_theme_manager(&initial_config)?;

        // Start theme watching with 5-second debounce and notify clients on changes
        if let Err(e) = theme_manager.start_watching_with(
            Duration::from_secs(5),
            Some(Arc::clone(&client_registry)),
            &watch_registry,
        ) {
            // eprintln! vanishes once daemonized (stdio -> /dev/null); log instead (#323).
            debug_log!("agent", "Warning: Failed to start theme watching ({e})");
        }

        let instant_cache_inner = crate::cache::InstantPromptCache::new()?;
        instant_cache_inner.clear_language_files();
        let instant_cache = Arc::new(instant_cache_inner);

        // The initial theme-export cache is written by a background task in
        // `start_background` *after* the socket begins accepting, so the accept
        // path is no longer gated on the theme render + atomic write. This is
        // safe because the export file's only reader is the shell's reload
        // handler, which falls back to spawning `gpy-agent theme export` on a
        // cache miss; first-prompt renders never read it.
        let watcher_slot: SharedWatcherSlot = Arc::new(Mutex::new(None));
        let language_cache = crate::language::DetectionCache::new();
        let palette_cache = Arc::new(crate::palette::PaletteCache::from_config(&initial_config));

        let ctx = events::AgentContext {
            registry: Arc::clone(&client_registry),
            cache: Arc::clone(&git_cache),
            config_manager: Arc::clone(&config_manager),
            theme_manager: Arc::clone(&theme_manager),
            instant_cache,
            language_cache: language_cache.clone(),
            palette_cache: Arc::clone(&palette_cache),
            dispatch_guard: Arc::new(events::RepoRefreshCoordinator::default()),
            watch_registry,
        };

        Self::setup_hot_reload(&ctx, &watcher_slot, watcher_config);

        Self::init_watcher(&initial_config, watcher_config, &ctx, &watcher_slot);

        let latency_tracker = Arc::new(crate::ipc::LatencyTracker::new(100_usize));

        let socket_path = crate::agent::lifecycle::get_socket_path()?;
        let server = Self::init_server(
            &ctx,
            &watcher_slot,
            &latency_tracker,
            language_cache,
            socket_path,
        )?;

        Ok(Self {
            server,
            watcher: watcher_slot,
            ctx,
        })
    }

    /// # Errors
    ///
    /// Returns an error if the config manager fails to initialize.
    fn init_config_manager() -> Result<Arc<crate::config::manager::ConfigManager>> {
        let config_manager_inner = crate::config::manager::ConfigManager::new()
            .or_else(|e| {
                crate::debug::warn_fallback("Config loading", "using defaults", &e);
                crate::config::manager::ConfigManager::with_defaults()
            })
            .map_err(|err| {
                Error::agent(format!(
                    "Failed to initialize config manager: {err}. \
                     This is a fatal error - neither user config nor defaults could be loaded."
                ))
            })?;
        Ok(Arc::new(config_manager_inner))
    }

    /// # Errors
    ///
    /// Returns an error if the theme manager fails to initialize.
    fn init_theme_manager(config: &crate::config::Config) -> Result<Arc<ThemeManager>> {
        let theme_name = &config.ui.theme;
        let theme_manager_inner = ThemeManager::new(theme_name.as_str())
            .or_else(|e| {
                crate::debug::warn_fallback(
                    &format!("Theme '{theme_name}' loading"),
                    "using default",
                    &e,
                );
                ThemeManager::new("default")
            })
            .map_err(|err| {
                Error::agent(format!(
                    "Failed to initialize theme manager: {err}. \
                     This is a fatal error - neither user theme nor default theme could be loaded."
                ))
            })?;
        Ok(Arc::new(theme_manager_inner))
    }

    fn setup_hot_reload(
        ctx: &events::AgentContext,
        watcher_slot: &SharedWatcherSlot,
        watcher_config: WatcherConfig,
    ) {
        let disable_watcher = watcher_disabled_by_env();

        // Prepare callback dependencies
        let watcher_config_for_callback = watcher_config;
        let ctx_for_callback = ctx.clone();
        let watcher_slot_for_callback = Arc::clone(watcher_slot);

        // Set up hot-reload callback
        ctx.config_manager
            .set_reload_callback(Arc::new(move |old_config, new_config| {
                events::handle_config_reload(
                    &ctx_for_callback,
                    &watcher_slot_for_callback,
                    watcher_config_for_callback,
                    disable_watcher,
                    old_config,
                    new_config,
                )
            }));

        // Theme-file hot reloads must refresh the export and instant caches
        // before the doorbell rings; the hook replaces the manager's direct
        // `notify_reload` (#710).
        let ctx_for_theme_callback = ctx.clone();
        ctx.theme_manager.set_reload_callback(Arc::new(move || {
            let config = ctx_for_theme_callback.config_manager.get();
            if let Err(e) = crate::cache::write_theme_export_cache(
                &ctx_for_theme_callback.theme_manager,
                &config,
            ) {
                debug_log!("agent", "Failed to write theme export cache: {}", e);
            }
            events::regenerate_instant_caches_for_theme_change(&ctx_for_theme_callback, &config);
            ctx_for_theme_callback.registry.notify_reload();
        }));

        // Config changes should feel responsive, so debounce at 1 second.
        if let Err(e) = ctx
            .config_manager
            .start_watching_with(Duration::from_secs(1), &ctx.watch_registry)
        {
            debug_log!(
                "config",
                "Warning: Failed to start config watching ({e}), config changes won't reload automatically"
            );
        } else {
            debug_log!("config", "Config hot-reload enabled with 1-second debounce");
        }
    }

    fn init_watcher(
        initial_config: &crate::config::Config,
        watcher_config: WatcherConfig,
        ctx: &events::AgentContext,
        watcher_slot: &SharedWatcherSlot,
    ) {
        let disable_watcher = watcher_disabled_by_env();

        if !disable_watcher
            && initial_config.agent.live_updates
            && (initial_config.git.enabled || initial_config.language.enabled)
        {
            match events::create_watcher(watcher_config, ctx) {
                Ok(initial_watcher) => {
                    if let Ok(mut guard) = watcher_slot.lock() {
                        *guard = Some(initial_watcher);
                    }
                }
                Err(err) => {
                    debug_log!(
                        "agent",
                        "Warning: File watcher creation failed - running without live updates ({err})"
                    );
                }
            }
        } else if disable_watcher {
            debug_log!("agent", "Watcher disabled via GPY_DISABLE_WATCHER");
        } else if !initial_config.agent.live_updates {
            debug_log!(
                "agent",
                "Live updates disabled via config; skipping watcher startup"
            );
        } else {
            debug_log!("agent", "Git segment disabled; skipping watcher startup");
        }
    }

    /// # Errors
    ///
    /// Returns an error if the endpoint handle builder fails.
    fn init_server(
        ctx: &events::AgentContext,
        watcher_slot: &SharedWatcherSlot,
        latency_tracker: &Arc<crate::ipc::LatencyTracker>,
        language_cache: crate::language::DetectionCache,
        socket_path: std::path::PathBuf,
    ) -> Result<EndpointHandle> {
        EndpointHandle::builder()
            .socket_path(socket_path)
            .client_registry(Arc::clone(&ctx.registry))
            .git_cache(Arc::clone(&ctx.cache))
            .config_manager(Arc::clone(&ctx.config_manager))
            .watcher_slot(Arc::clone(watcher_slot))
            .theme_manager(Arc::clone(&ctx.theme_manager))
            .instant_cache(Arc::clone(&ctx.instant_cache))
            .latency_tracker(Arc::clone(latency_tracker))
            .language_cache(language_cache)
            .palette_cache(Arc::clone(&ctx.palette_cache))
            .build()
    }

    /// Prune dead clients from registry and clean up watcher
    ///
    /// This method is called periodically by the pruning timer and can also
    /// be called directly for testing purposes.
    pub fn prune_dead_clients_and_cleanup(&self) {
        // Shares one implementation with the pruning-timer arm in
        // `start_background`, which cannot call this method at all (see
        // `prune_dead_clients`'s doc comment).
        let _pruned_pids = prune_dead_clients(&self.ctx.registry, &self.watcher);
    }

    /// Start the agent in background mode
    ///
    /// # Errors
    ///
    /// Returns an error if signal handling cannot be set up, if the IPC server
    /// fails to start, or if the server fails to stop gracefully.
    #[expect(
        clippy::too_many_lines,
        reason = "top-level background-mode startup sequence: recovery nudge, signal handling setup, IPC server accept loop, and graceful shutdown all have to be wired together in order; splitting would scatter this single orchestration path across helpers with no independent reuse"
    )]
    pub async fn start_background(&mut self) -> Result<()> {
        startup::startup_checks(self.server.socket_path());

        // Recovery nudge. The daemon is the only place it runs (the
        // `gpy-agent start` process just waits for readiness, #741), so each
        // tracked shell is nudged once: the daemon writes its `.reregister`
        // flag and rings its doorbell once our socket is accepting connections.
        // Those shells re-register and resume live updates without the user
        // having to press enter.
        #[cfg(unix)]
        {
            let socket_path = self.server.socket_path().to_owned();
            tokio::spawn(async move {
                for _ in 0..100_u32 {
                    if std::os::unix::net::UnixStream::connect(&socket_path).is_ok() {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                crate::agent::lifecycle::start::notify_existing_shells_of_restart();
            });
        }

        // Warm the theme-export cache off the accept path. This renders the theme
        // and atomically writes the export file (blocking FS work), so it runs on
        // a blocking thread and only after the server has begun accepting below.
        // A first request or reload doorbell racing this warmup gets a cache miss and the
        // shell falls back to spawning `gpy-agent theme export`, so the prompt is
        // always correct, never gated on this write.
        {
            let theme_manager_for_export = Arc::clone(&self.ctx.theme_manager);
            let config_for_export = self.ctx.config_manager.get();
            tokio::task::spawn_blocking(move || {
                if let Err(e) = crate::cache::write_theme_export_cache(
                    &theme_manager_for_export,
                    &config_for_export,
                ) {
                    debug_log!(
                        "agent",
                        "Warning: Failed to write initial theme export cache ({e})"
                    );
                }
            });
        }

        let registry_for_clock = Arc::clone(&self.ctx.registry);
        let registry_for_pruning = Arc::clone(&self.ctx.registry);
        let watcher_for_pruning = Arc::clone(&self.watcher);
        let mut clock_timer = startup::create_clock_timer();
        let mut pruning_timer = tokio::time::interval(Duration::from_secs(60));
        pruning_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        // Bounded per-PID retry budget for re-nudging stranded shells; see
        // `lifecycle::start::ShellRenudger`.
        #[cfg(unix)]
        let mut shell_renudger = lifecycle::start::ShellRenudger::default();
        // Reconcile catches working-tree/.git mutations the OS watcher dropped. It
        // recomputes status and only signals on a real diff, so it never produces a
        // no-op broadcast. The first tick fires immediately; skip it so we don't scan
        // before any client has registered.
        let mut reconcile_timer =
            tokio::time::interval(Duration::from_secs(reconcile_interval_secs()));
        reconcile_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        reconcile_timer.reset();
        // Guards against overlapping reconcile scans: a slow scan (many watched
        // repos, slow subprocess) could otherwise still be running when the next
        // reconcile tick fires, piling up concurrent scans (#323).
        let reconcile_in_flight = Arc::new(AtomicBool::new(false));
        // Set when a reconcile tick is skipped because a scan is already in
        // flight. `run_reconcile_scan` consumes this to run exactly one
        // catch-up pass right after the in-flight scan finishes, so a
        // watcher-dropped event skipped mid-scan is caught up in
        // ~(interval + scan time) instead of doubling to ~2x interval (#417).
        let reconcile_pending = Arc::new(AtomicBool::new(false));
        let mut server_future = Box::pin(self.server.start());
        let mut shutdown_signal = startup::create_shutdown_signal();
        let mut last_notified_minute = 0_u64;

        // Main event loop
        loop {
            tokio::select! {
                result = &mut server_future => {
                    match &result {
                        Ok(()) => {
                            debug_log!("agent", "Server completed successfully - this shouldn't happen");
                        }
                        Err(e) => {
                            debug_log!("agent", "Server failed with error: {e}");
                        }
                    }
                    return result;
                }
                shutdown_result = &mut shutdown_signal => {
                    shutdown_result?;

                    // Graceful shutdown
                    debug_log!("agent", "Received shutdown signal");
                    if let Ok(mut guard) = self.watcher.lock()
                        && let Some(watcher) = guard.take()
                    {
                        watcher.stop();
                    }
                    self.ctx.theme_manager.stop_watching();
                    self.ctx.config_manager.stop_watching();

                    // Drop the in-flight server future first: it holds the only
                    // mutable borrow of `self.server`, and stop() needs one to
                    // unlink the socket. Without this, SIGINT/SIGTERM shutdown
                    // just drops the accept loop mid-await, skipping the cleanup
                    // that normally only runs at the end of `accept_loop` (which
                    // requires an IPC `Shutdown` message, not a signal).
                    drop(server_future);
                    if let Err(e) = self.server.stop() {
                        debug_log!("agent", "Failed to stop IPC server cleanly: {e}");
                    }

                    println!("GPY Agent stopped.");

                    return Ok(());
                }
                _ = clock_timer.tick(), if self.ctx.config_manager.get().agent.live_updates => {
                    last_notified_minute =
                        handle_clock_tick(&self.ctx, &registry_for_clock, last_notified_minute);
                }
                _ = pruning_timer.tick() => {
                    let _pruned_pids =
                        prune_dead_clients(&registry_for_pruning, &watcher_for_pruning);

                    // Re-nudge shells that are alive and still tracked but have
                    // fallen out of the registry. The one-shot nudge sent at
                    // restart is otherwise their only chance to re-register, and
                    // a shell whose circuit breaker was still backing off when
                    // that nudge landed would stay stranded — receiving no
                    // live updates at all — until the user happened to
                    // render another prompt.
                    #[cfg(unix)]
                    lifecycle::start::renudge_unregistered_shells(
                        &mut shell_renudger,
                        &registry_for_pruning,
                    );
                }
                _ = reconcile_timer.tick(), if self.ctx.config_manager.get().agent.live_updates => {
                    let reconcile_config = self.ctx.config_manager.get();
                    spawn_reconcile_scan(
                        &self.ctx,
                        &self.watcher,
                        reconcile_config,
                        &reconcile_in_flight,
                        &reconcile_pending,
                    );
                }
            }
        }
    }

    /// Handle a single oneshot request (no background agent)
    ///
    /// # Errors
    ///
    /// Returns an error if the request format is invalid, if git status detection fails,
    /// or if language detection fails.
    pub fn handle_oneshot(request: &str) -> Result<String> {
        oneshot::handle_oneshot(request)
    }

    /// Handle a single, already-typed oneshot request (no background agent, no
    /// JSON round-trip) — the in-process CLI entry point.
    ///
    /// # Errors
    ///
    /// Returns an error if the request format is invalid, if git status detection fails,
    /// or if language detection fails.
    pub fn handle_oneshot_request(request: &oneshot::OneshotRequest) -> Result<String> {
        oneshot::handle_oneshot_request(request)
    }

    // Test helper methods
    /// Create an Agent instance for integration testing with watcher enabled
    ///
    /// # Errors
    ///
    /// Returns an error if watcher creation fails.
    #[doc(hidden)]
    pub fn new_for_testing_with_watcher(_repo_path: &std::path::Path) -> Result<Self> {
        use crate::git::cache::GitStatusCache;
        use crate::watcher::{DebouncedEvent, WatcherConfig, multi_repo::MultiRepoWatcher};
        use std::sync::Mutex;

        // Create default theme manager for testing first
        let theme_manager = Arc::new(ThemeManager::new("default")?);
        // Create default config manager for testing
        let config_manager = Arc::new(crate::config::manager::ConfigManager::new()?);
        // Create instant cache for testing
        let instant_cache = Arc::new(crate::cache::InstantPromptCache::new()?);

        // Create watcher
        let config = WatcherConfig::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = Box::new(move |event| {
            if let Ok(mut guard) = events_clone.lock() {
                guard.push(event);
            }
        });
        let watcher = Arc::new(
            MultiRepoWatcher::builder()
                .config(config)
                .callback(callback)
                .build()?,
        );
        let watcher_slot = Arc::new(Mutex::new(Some(Arc::clone(&watcher))));

        // Create minimal agent for testing
        let client_registry = Arc::new(ClientDirectory::new());
        let git_cache = Arc::new(GitStatusCache::new());
        let server = EndpointHandle::new_test_handle(
            Arc::clone(&client_registry),
            &git_cache,
            Arc::clone(&watcher_slot),
        );

        let initial_config_for_testing = config_manager.get();
        let ctx = events::AgentContext {
            registry: Arc::clone(&client_registry),
            cache: Arc::clone(&git_cache),
            config_manager: Arc::clone(&config_manager),
            theme_manager: Arc::clone(&theme_manager),
            instant_cache,
            language_cache: crate::language::DetectionCache::new(),
            palette_cache: Arc::new(crate::palette::PaletteCache::from_config(
                &initial_config_for_testing,
            )),
            dispatch_guard: Arc::new(events::RepoRefreshCoordinator::default()),
            watch_registry: Arc::clone(watcher.registry()),
        };

        Ok(Self {
            server,
            watcher: watcher_slot,
            ctx,
        })
    }

    /// The palette the agent currently renders with, for integration testing.
    #[doc(hidden)]
    #[must_use]
    pub fn active_palette_for_testing(&self) -> crate::template::Palette {
        self.ctx.palette_cache.get()
    }

    /// Register a test client for integration testing
    #[doc(hidden)]
    pub fn register_test_client(&self, pid: u32, repo_path: &std::path::Path) {
        self.ctx
            .registry
            .register(pid, Some(repo_path.to_path_buf()));
        self.with_watcher(|watcher| {
            let _ = watcher.register_client(pid, repo_path);
        });
    }

    /// Get client count for integration testing
    #[doc(hidden)]
    pub fn client_count(&self) -> usize {
        self.ctx.registry.len()
    }

    /// Get watcher repo count for integration testing
    #[doc(hidden)]
    pub fn watcher_repo_count(&self) -> usize {
        self.watcher
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|w| w.watched_repo_count()))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod language_tests {
    #![allow(clippy::missing_panics_doc)]

    use crate::config::LanguageSettings;
    use crate::language::detector::DetectedLanguage;
    use crate::language::display::build_language_display_info;
    use crate::theme::ThemeConfig;

    fn sample_detected_languages() -> Vec<DetectedLanguage> {
        vec![
            DetectedLanguage {
                name: "alpha".to_owned(),
                confidence: 1.0,
                file_count: 1,
                total_bytes: 42,
            },
            DetectedLanguage {
                name: "beta".to_owned(),
                confidence: 0.8,
                file_count: 2,
                total_bytes: 256,
            },
        ]
    }

    #[test]
    fn build_language_display_retains_languages_when_versions_hidden() {
        let detected = sample_detected_languages();
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            show_versions: false,
            ..LanguageSettings::default()
        };

        let info = build_language_display_info(&detected, &theme, &language_cfg);

        assert_eq!(info.len(), detected.len());
    }

    #[test]
    fn build_language_display_filters_enabled_languages() {
        let detected = sample_detected_languages();
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            show_versions: true,
            enabled_languages: vec!["alpha".to_owned()],
            ..LanguageSettings::default()
        };

        let info = build_language_display_info(&detected, &theme, &language_cfg);
        let names: Vec<&str> = info.iter().map(|lang| lang.name.as_str()).collect();
        assert_eq!(names, vec!["alpha"]);
    }

    #[test]
    fn build_language_display_disabled_returns_empty() {
        let detected = sample_detected_languages();
        let theme = ThemeConfig::default();
        let language_cfg = LanguageSettings {
            enabled: false,
            ..LanguageSettings::default()
        };

        let info = build_language_display_info(&detected, &theme, &language_cfg);
        assert!(info.is_empty());
    }
}

#[cfg(test)]
mod reconcile_interval_tests {
    #![allow(clippy::missing_panics_doc)]

    use super::parse_reconcile_interval_secs;

    #[test]
    fn parse_reconcile_interval_secs_rejects_unset_zero_and_invalid() {
        assert_eq!(parse_reconcile_interval_secs(None), None);
        assert_eq!(parse_reconcile_interval_secs(Some("0")), None);
        assert_eq!(parse_reconcile_interval_secs(Some("not-a-number")), None);
    }

    #[test]
    fn parse_reconcile_interval_secs_accepts_positive_integer() {
        assert_eq!(parse_reconcile_interval_secs(Some("5")), Some(5_u64));
    }
}

/// Tests for the reconcile-tick skip/catch-up logic (#417).
///
/// A reconcile tick that arrives while a scan is already in flight must not be silently
/// dropped until the next full interval — it must be caught up as soon as the in-flight scan
/// completes, without ever running two scans concurrently.
///
#[cfg(test)]
mod reconcile_scan_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::{PathBuf, run_reconcile_scan};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    fn fake_repo() -> Vec<PathBuf> {
        vec![PathBuf::from("/fake/repo")]
    }

    /// `mpsc::Receiver` is `Send` but not `Sync`, so it must be wrapped in a `Mutex` to share.
    ///
    /// Shared (via `Arc`) into closures that may be cloned and moved into `spawn_blocking` more
    /// than once, or awaited repeatedly from the test body without moving ownership back and
    /// forth.
    ///
    type SharedRx<T> = Arc<Mutex<mpsc::Receiver<T>>>;

    /// Blocks the calling (blocking-pool) thread until `scan_fn` runs, announcing each call via
    /// `start_tx` and then waiting on `release_rx` before returning.
    ///
    /// A stand-in for a slow git-status scan that the test can pause and resume at precise
    /// points, without real sleeps.
    ///
    fn gated_scan_fn(
        call_count: Arc<AtomicUsize>,
        start_tx: mpsc::Sender<usize>,
        release_rx: SharedRx<()>,
    ) -> impl Fn(Vec<PathBuf>) + Send + Clone + 'static {
        move |_repos: Vec<PathBuf>| {
            let call_number = call_count.fetch_add(1, Ordering::AcqRel).saturating_add(1);
            let _ = start_tx.send(call_number);
            let _ = release_rx.lock().expect("release_rx mutex poisoned").recv();
        }
    }

    /// Waits (off the async runtime thread) for the next `start_tx` signal
    /// from `gated_scan_fn`, returning the 1-based call number.
    async fn await_scan_started(start_rx: &SharedRx<usize>) -> usize {
        let owned_start_rx = Arc::clone(start_rx);
        tokio::task::spawn_blocking(move || {
            owned_start_rx
                .lock()
                .expect("start_rx mutex poisoned")
                .recv()
                .expect("scan_fn never started")
        })
        .await
        .expect("blocking wait task panicked")
    }

    /// Baseline / steady-state case: no tick is ever skipped, so exactly one scan pass runs
    /// and `pending` never triggers a catch-up.
    ///
    /// Guards against a regression that would make the extraction always run a second pass.
    #[tokio::test]
    async fn no_pending_tick_runs_exactly_one_pass() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let (start_tx, start_rx_raw) = mpsc::channel::<usize>();
        let shared_start_rx = Arc::new(Mutex::new(start_rx_raw));
        let (release_tx, release_rx_raw) = mpsc::channel::<()>();
        let shared_release_rx = Arc::new(Mutex::new(release_rx_raw));

        let in_flight = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(AtomicBool::new(false));
        let roots_fn_calls = Arc::new(AtomicUsize::new(0));
        let roots_fn_calls_for_closure = Arc::clone(&roots_fn_calls);
        let roots_fn = move || {
            roots_fn_calls_for_closure.fetch_add(1, Ordering::AcqRel);
            fake_repo()
        };
        let scan_fn = gated_scan_fn(Arc::clone(&call_count), start_tx, shared_release_rx);

        let handle = tokio::spawn(run_reconcile_scan(
            fake_repo(),
            Arc::clone(&in_flight),
            Arc::clone(&pending),
            roots_fn,
            scan_fn,
        ));

        let first_call_number = await_scan_started(&shared_start_rx).await;
        assert_eq!(first_call_number, 1);
        release_tx.send(()).expect("failed to release scan pass 1");

        handle.await.expect("run_reconcile_scan task panicked");

        assert_eq!(
            call_count.load(Ordering::Acquire),
            1,
            "no tick was skipped, so exactly one scan pass should run"
        );
        assert_eq!(
            roots_fn_calls.load(Ordering::Acquire),
            0,
            "roots_fn (only used for the catch-up pass) must not run when nothing is pending"
        );
        assert!(!pending.load(Ordering::Acquire));
        assert!(
            !in_flight.load(Ordering::Acquire),
            "in_flight must be released"
        );
    }

    /// Core #417 regression: a tick that arrives while the first scan is
    /// still in flight sets `pending` instead of starting a second concurrent
    /// scan (mirrors the tick arm's guard in `start_background`).
    ///
    /// `run_reconcile_scan` must then run exactly one catch-up pass as soon
    /// as the in-flight scan finishes, instead of waiting for the next full
    /// interval. This also exercises the "bounded catch-up" requirement: a
    /// *second* tick that arrives while the catch-up pass itself is running
    /// must not trigger a third pass in this invocation — `pending` is left
    /// set for the next scan-completion to handle (verified below via
    /// `run_reconcile_scan` never looping internally: exactly 2 calls happen
    /// here, not 3).
    #[tokio::test]
    async fn skipped_tick_triggers_exactly_one_bounded_catch_up_pass() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let (start_tx, start_rx_raw) = mpsc::channel::<usize>();
        let shared_start_rx = Arc::new(Mutex::new(start_rx_raw));
        let (release_tx, release_rx_raw) = mpsc::channel::<()>();
        let shared_release_rx = Arc::new(Mutex::new(release_rx_raw));

        let in_flight = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(AtomicBool::new(false));
        let roots_fn = fake_repo;
        let scan_fn = gated_scan_fn(Arc::clone(&call_count), start_tx, shared_release_rx);

        let handle = tokio::spawn(run_reconcile_scan(
            fake_repo(),
            Arc::clone(&in_flight),
            Arc::clone(&pending),
            roots_fn,
            scan_fn,
        ));

        // Pass 1 (the original scan) starts.
        let first_call_number = await_scan_started(&shared_start_rx).await;
        assert_eq!(first_call_number, 1);

        // Simulate a reconcile tick arriving while pass 1 is still in
        // flight: the tick arm would see `in_flight == true` and mark
        // `pending` rather than spawning a second scan.
        pending.store(true, Ordering::Release);
        release_tx.send(()).expect("failed to release scan pass 1");

        // `pending` was set, so exactly one catch-up pass (pass 2) must
        // start next.
        let second_call_number = await_scan_started(&shared_start_rx).await;
        assert_eq!(second_call_number, 2, "expected exactly one catch-up pass");

        // Simulate a second tick arriving mid-catch-up: it must be recorded
        // as pending again but must NOT cause a third pass inside this
        // invocation of run_reconcile_scan (bounded catch-up).
        pending.store(true, Ordering::Release);
        release_tx.send(()).expect("failed to release scan pass 2");

        handle.await.expect("run_reconcile_scan task panicked");

        assert_eq!(
            call_count.load(Ordering::Acquire),
            2,
            "catch-up must run exactly once per invocation, never loop"
        );
        assert!(
            pending.load(Ordering::Acquire),
            "the tick that arrived during the catch-up pass must remain \
             pending for the next scan completion to handle"
        );
        assert!(
            !in_flight.load(Ordering::Acquire),
            "in_flight must be released once the bounded catch-up completes"
        );
    }

    /// The catch-up pass must re-fetch watched roots, not reuse the stale
    /// `initial_repos` snapshot.
    ///
    /// Repos may have been added or removed between the skipped tick and
    /// the catch-up pass actually running, so `run_reconcile_scan` must call
    /// `roots_fn` again rather than replaying the original snapshot.
    #[tokio::test]
    async fn catch_up_pass_uses_freshly_fetched_roots() {
        let call_count = Arc::new(AtomicUsize::new(0));
        let (start_tx, start_rx_raw) = mpsc::channel::<usize>();
        let shared_start_rx = Arc::new(Mutex::new(start_rx_raw));
        let (release_tx, release_rx_raw) = mpsc::channel::<()>();
        let shared_release_rx = Arc::new(Mutex::new(release_rx_raw));

        let in_flight = Arc::new(AtomicBool::new(true));
        let pending = Arc::new(AtomicBool::new(false));

        let seen_repos: Arc<Mutex<Vec<Vec<PathBuf>>>> = Arc::new(Mutex::new(Vec::new()));
        let seen_repos_for_scan = Arc::clone(&seen_repos);
        let call_count_for_scan = Arc::clone(&call_count);
        let scan_fn = move |repos: Vec<PathBuf>| {
            let call_number = call_count_for_scan.fetch_add(1, Ordering::AcqRel) + 1;
            seen_repos_for_scan
                .lock()
                .expect("seen_repos mutex poisoned")
                .push(repos);
            let _ = start_tx.send(call_number);
            let _ = shared_release_rx
                .lock()
                .expect("release_rx mutex poisoned")
                .recv();
        };

        let roots_fn = || vec![PathBuf::from("/fresh/repo-added-after-snapshot")];

        let handle = tokio::spawn(run_reconcile_scan(
            fake_repo(),
            Arc::clone(&in_flight),
            Arc::clone(&pending),
            roots_fn,
            scan_fn,
        ));

        let first_call_number = await_scan_started(&shared_start_rx).await;
        assert_eq!(first_call_number, 1);
        pending.store(true, Ordering::Release);
        release_tx.send(()).expect("failed to release scan pass 1");

        let second_call_number = await_scan_started(&shared_start_rx).await;
        assert_eq!(second_call_number, 2);
        release_tx.send(()).expect("failed to release scan pass 2");

        handle.await.expect("run_reconcile_scan task panicked");

        // Clone the recorded repos out and drop the guard immediately so the
        // mutex isn't held across the assertions below.
        let seen = seen_repos
            .lock()
            .expect("seen_repos mutex poisoned")
            .clone();

        assert_eq!(seen.len(), 2);
        let first_pass = seen.first().expect("pass 1 recorded no repos");
        let catch_up_pass = seen.get(1).expect("catch-up pass recorded no repos");
        assert_eq!(*first_pass, fake_repo(), "pass 1 uses the initial snapshot");
        assert_eq!(
            *catch_up_pass,
            vec![PathBuf::from("/fresh/repo-added-after-snapshot")],
            "catch-up pass must use freshly-fetched roots, not the stale initial snapshot"
        );
    }
}

/// The registry and the watcher must agree on who is registered after every
/// pruning pass, even when a doorbell broadcast removed clients from the
/// registry alone (#782).
#[cfg(test)]
mod prune_reconcile_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::{Arc, Mutex, PathBuf, prune_dead_clients};
    use super::{ClientDirectory, MultiRepoWatcher, SharedWatcherSlot, WatcherConfig};
    use std::collections::{BTreeMap, BTreeSet};
    use std::path::Path;
    use std::process::Command;

    fn init_repo(root: &Path) {
        let status = Command::new("git")
            .args(["-c", "commit.gpgsign=false", "init", "--quiet"])
            .current_dir(root)
            .status()
            .expect("run git init");
        assert!(status.success(), "git init must succeed");
    }

    fn watcher_slot() -> (SharedWatcherSlot, Arc<MultiRepoWatcher>) {
        let watcher = Arc::new(
            MultiRepoWatcher::builder()
                .config(WatcherConfig::default())
                .callback(Box::new(|_event| {}))
                .build()
                .expect("build watcher"),
        );
        (Arc::new(Mutex::new(Some(Arc::clone(&watcher)))), watcher)
    }

    /// Two canonical temp repos (the second with a subdirectory) plus the
    /// registry and watcher under test.
    struct Fixture {
        _tmp: tempfile::TempDir,
        repos: Vec<PathBuf>,
        /// `(cwd, index into repos)`
        cwds: Vec<(PathBuf, usize)>,
        registry: ClientDirectory,
        slot: SharedWatcherSlot,
        watcher: Arc<MultiRepoWatcher>,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().expect("temp dir");
        let base = std::fs::canonicalize(tmp.path()).expect("canonicalize temp dir");
        let mut repos = Vec::new();
        let mut cwds = Vec::new();
        for (index, name) in ["repo-a", "repo-b"].into_iter().enumerate() {
            let root = base.join(name);
            std::fs::create_dir_all(root.join("sub")).expect("create repo dir");
            init_repo(&root);
            cwds.push((root.clone(), index));
            cwds.push((root.join("sub"), index));
            repos.push(root);
        }
        let shell_dir = base.join("shell");
        std::fs::create_dir_all(&shell_dir).expect("create shell dir");
        let (slot, watcher) = watcher_slot();
        Fixture {
            _tmp: tmp,
            repos,
            cwds,
            registry: ClientDirectory::with_shell_dir(shell_dir),
            slot,
            watcher,
        }
    }

    // Relies on the broadcast dropping a recycled PID, which native Windows
    // has no start-time lookup to detect.
    #[cfg(unix)]
    #[test]
    fn prune_reconciles_watcher_with_registry_after_broadcast_drop() {
        let fx = fixture();
        let pid = std::process::id();
        let repo = fx.repos.first().expect("repo").clone();

        fx.registry
            .register_with_started_at_for_test(pid, Some(repo.clone()), Some(1));
        fx.watcher.register_client(pid, &repo).unwrap();
        assert_eq!(fx.watcher.watched_repo_count(), 1);

        // The broadcast path drops a confirmed recycle from the registry alone.
        fx.registry.notify_repaint(None);
        assert!(!fx.registry.is_registered(pid), "broadcast must drop it");
        assert_eq!(
            fx.watcher.client_pids(),
            vec![pid],
            "watcher still holds it"
        );

        let pruned = prune_dead_clients(&fx.registry, &fx.slot);

        assert_eq!(pruned, vec![pid], "reconciled pid is reported once");
        assert_eq!(fx.watcher.client_pids(), Vec::<u32>::new());
        assert_eq!(fx.watcher.watched_repo_count(), 0);
        assert_eq!(fx.watcher.watched_roots(), Vec::<std::path::PathBuf>::new());
    }

    proptest::proptest! {
        #![proptest_config(proptest::prelude::ProptestConfig {
            cases: 24,
            failure_persistence: None,
            ..proptest::prelude::ProptestConfig::default()
        })]

        /// Random register / workspace / unregister / registry-only drop
        /// sequences, each followed (at random) by a prune pass, leave the
        /// watcher tracking exactly the registered pids and watching exactly
        /// the repos they are in.
        #[test]
        fn prune_keeps_watcher_matching_registry(
            steps in proptest::collection::vec(
                (0_u8..4_u8, 0_usize..2_usize, 0_usize..4_usize, proptest::bool::ANY),
                1..=24,
            ),
        ) {
            let fx = fixture();
            // Two live pids, so registration with no recorded start time
            // survives the liveness check in the prune pass.
            #[cfg(unix)]
            let second_pid = std::os::unix::process::parent_id();
            // Native Windows reports every client alive, so any distinct id works.
            #[cfg(not(unix))]
            let second_pid = std::process::id().wrapping_add(1);
            let live = [std::process::id(), second_pid];
            let mut model: BTreeMap<u32, usize> = BTreeMap::new();
            for (done, (kind, pid_index, cwd_index, prune)) in steps.iter().copied().enumerate() {
                let trace = steps.get(..=done).expect("in range");
                let pid = *live.get(pid_index).expect("pid index");
                let (cwd, repo_index) = fx.cwds.get(cwd_index).expect("cwd index");
                match kind {
                    0 => {
                        fx.registry
                            .register_with_started_at_for_test(pid, Some(cwd.clone()), None);
                        fx.watcher.register_client(pid, cwd).unwrap();
                        model.insert(pid, *repo_index);
                    }
                    1 => {
                        if fx.registry.update_workspace(pid, cwd) {
                            fx.watcher.update_client(pid, cwd).unwrap();
                            model.insert(pid, *repo_index);
                        }
                    }
                    2 => {
                        fx.registry.unregister(pid);
                        fx.watcher.unregister_client(pid).unwrap();
                        model.remove(&pid);
                    }
                    _ => {
                        // Registry-only drop, as the broadcast path does.
                        fx.registry.unregister(pid);
                        model.remove(&pid);
                    }
                }
                if prune || trace.len() == steps.len() {
                    let _ = prune_dead_clients(&fx.registry, &fx.slot);
                    let mut registered = fx.registry.clients();
                    registered.sort_unstable();
                    assert_eq!(
                        fx.watcher.client_pids(),
                        registered,
                        "watcher pids != registry pids after {trace:?}"
                    );
                    let wanted: BTreeSet<PathBuf> = model
                        .values()
                        .filter_map(|index| fx.repos.get(*index).cloned())
                        .collect();
                    let watched: BTreeSet<PathBuf> = fx.watcher.watched_roots().into_iter().collect();
                    assert_eq!(watched, wanted, "watched repos != repos with a client after {trace:?}");
                }
            }
        }
    }
}

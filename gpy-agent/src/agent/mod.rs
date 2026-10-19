//! Background agent process: IPC server, file watcher, clock and pruning
//! timers, and graceful shutdown, driven by one `tokio::select!` loop in
//! [`Agent::start_background`].
//!
//! Cross-subsystem coordination (git cache invalidation, repaint/reload
//! doorbells, watcher registration) lives here so git, watcher, config, and
//! theme modules never call each other directly.

pub mod events;
/// Agent lifecycle management (start/stop/status).
pub mod lifecycle;
pub mod oneshot;
pub mod startup;

use crate::debug_log;
use crate::git::cache::GitStatusCache;
use crate::ipc::{
    ClientDirectory,
    server::{EndpointDeps, EndpointHandle},
};
use crate::theme::ThemeManager;
use crate::watcher::hot_reload::HOT_RELOAD_DEBOUNCE;
use crate::watcher::{WatcherConfig, multi_repo::MultiRepoWatcher};
use crate::{Error, Result};
use events::SharedWatcherSlot;
use std::env;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

fn matches_ignore_case(value: &str, truthy: &[&str]) -> bool {
    truthy
        .iter()
        .any(|candidate| value.trim().eq_ignore_ascii_case(candidate))
}

/// Whether `GPY_DISABLE_WATCHER` disables the file watcher. Startup and
/// hot-reload both check it so a reload never re-enables the watcher.
fn watcher_disabled_by_env() -> bool {
    env::var("GPY_DISABLE_WATCHER")
        .is_ok_and(|value| matches_ignore_case(&value, &["1", "true", "yes", "on"]))
}

/// Default reconcile interval; above the git-status cache TTL so scans see fresh state.
const RECONCILE_INTERVAL_SECS: u64 = 45;

/// Parse `GPY_RECONCILE_INTERVAL_SECS`; `None` when unset, non-numeric, or `0`.
/// Pure so tests avoid `std::env::set_var` (needs `unsafe`, which is forbidden).
fn parse_reconcile_interval_secs(raw: Option<&str>) -> Option<u64> {
    raw?.parse::<u64>().ok().filter(|value| *value > 0)
}

/// Resolve the reconcile interval from `GPY_RECONCILE_INTERVAL_SECS`, else
/// [`RECONCILE_INTERVAL_SECS`]. For setups where `FSEvents` never arrive.
fn reconcile_interval_secs() -> u64 {
    let raw = std::env::var("GPY_RECONCILE_INTERVAL_SECS").ok();
    parse_reconcile_interval_secs(raw.as_deref()).unwrap_or(RECONCILE_INTERVAL_SECS)
}

/// Runs `scan_fn` over `repos` on a blocking thread (git status is a subprocess).
/// A panic in `scan_fn` is logged, not propagated.
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

/// Runs one reconcile pass, then one catch-up pass if a tick was skipped meanwhile.
///
/// Callers must set `in_flight` before spawning this and never spawn a second
/// call while it is set; this function alone clears it. A tick arriving during
/// the catch-up pass stays `pending` for the next invocation.
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

    // Re-fetch roots: repos may have changed since `initial_repos` was taken.
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

/// Prune dead clients from `registry` and unregister them from the watcher;
/// returns the pruned PIDs. A free function because the `select!` loop holds
/// a borrow of `self.server` and cannot call `&self` methods.
fn prune_dead_clients(registry: &ClientDirectory, watcher_slot: &SharedWatcherSlot) -> Vec<u32> {
    let mut pruned_pids = registry.prune_dead_clients();

    {
        let guard = watcher_slot.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(watcher) = guard.as_ref() {
            for pid in &pruned_pids {
                watcher.unregister_client(*pid);
            }

            // Snapshot first so the watcher is never called with registry state held.
            for pid in watcher.client_pids() {
                if !registry.is_registered(pid) {
                    watcher.unregister_client(pid);
                    if !pruned_pids.contains(&pid) {
                        pruned_pids.push(pid);
                    }
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

/// Ring the clock repaint doorbell if this tick warrants it. Returns the
/// `last_notified_minute` to carry forward (unchanged while the clock is disabled).
fn handle_clock_tick(
    ctx: &events::AgentContext,
    registry: &ClientDirectory,
    last_notified_minute: u64,
) -> u64 {
    let config = ctx.config_manager.get();

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

/// Start a reconcile scan unless one is running (then just mark `pending`).
/// Clears `in_flight` here if there is nothing to scan; otherwise
/// [`run_reconcile_scan`] owns clearing it.
fn spawn_reconcile_scan(
    ctx: &events::AgentContext,
    watcher_slot: &SharedWatcherSlot,
    config: Arc<crate::config::Config>,
    in_flight: &Arc<AtomicBool>,
    pending: &Arc<AtomicBool>,
) {
    if in_flight.swap(true, Ordering::AcqRel) {
        pending.store(true, Ordering::Release);
        debug_log!(
            "agent",
            "Reconcile tick skipped: previous reconcile scan still in flight; marked pending for catch-up"
        );
        return;
    }

    let repos = watcher_slot
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref()
        .map_or_else(Vec::new, |w| w.watched_roots());
    if repos.is_empty() {
        in_flight.store(false, Ordering::Release);
        return;
    }

    // Await the blocking scan in an outer task so a panic is logged and the
    // in-flight guard is always cleared.
    let reconcile_ctx = ctx.clone();
    let watcher_for_reconcile = Arc::clone(watcher_slot);
    let in_flight_for_scan = Arc::clone(in_flight);
    let pending_for_scan = Arc::clone(pending);
    let scan_fn = move |scan_repos: Vec<PathBuf>| {
        for git_root in scan_repos {
            // Coalesce with watcher-dispatched scans of the same repo.
            events::refresh_and_notify_coalesced(&reconcile_ctx, &git_root, &config, None);
        }
    };
    let roots_fn = move || {
        watcher_for_reconcile
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or_else(Vec::new, |w| w.watched_roots())
    };
    tokio::spawn(run_reconcile_scan(
        repos,
        in_flight_for_scan,
        pending_for_scan,
        roots_fn,
        scan_fn,
    ));
}

/// Main agent process that coordinates all subsystems.
pub struct Agent {
    server: EndpointHandle,
    watcher: SharedWatcherSlot,
    /// Shared subsystem handles (managers, registry, caches, refresh coordinator).
    ctx: events::AgentContext,
}

impl Agent {
    fn with_watcher<F>(&self, f: F)
    where
        F: FnOnce(&MultiRepoWatcher),
    {
        if let Some(watcher) = self
            .watcher
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            f(watcher);
        }
    }

    /// Create a new agent instance.
    ///
    /// # Errors
    /// Returns an error if the IPC server or file watcher cannot be initialized.
    ///
    /// # Panics
    /// Panics if the bundled default theme cannot be loaded.
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
        // One registry shared by all coordinators: `classify_event` consults it
        // on every coordinator, so they must see the same config paths.
        let watch_registry = Arc::new(crate::watcher::WatchRegistry::new());
        watch_registry.set_worktree_enabled(initial_config.git.watch_worktree);

        let theme_manager = Self::init_theme_manager(&initial_config)?;

        if let Err(e) = theme_manager.start_watching_with(HOT_RELOAD_DEBOUNCE, &watch_registry) {
            // eprintln! vanishes once daemonized (stdio -> /dev/null); log instead.
            debug_log!("agent", "Warning: Failed to start theme watching ({e})");
        }

        let instant_cache_inner = crate::cache::InstantPromptCache::new()?;
        instant_cache_inner.clear_unrefreshable_files();
        let instant_cache = Arc::new(instant_cache_inner);

        // The theme-export cache is warmed in `start_background` after accept
        // begins; its only reader falls back to `gpy-agent theme export` on miss.
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
            theme_export_dir: crate::cache::theme_export::theme_export_dir()?,
            announce_gate: Arc::new(Mutex::new(())),
        };

        Self::setup_hot_reload(&ctx, &watcher_slot, watcher_config);

        Self::init_watcher(&initial_config, watcher_config, &ctx, &watcher_slot);

        let socket_path = crate::agent::lifecycle::get_socket_path()?;
        let server = Self::init_server(&ctx, &watcher_slot, language_cache, socket_path)?;

        Ok(Self {
            server,
            watcher: watcher_slot,
            ctx,
        })
    }

    /// # Errors
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

        let watcher_config_for_callback = watcher_config;
        let ctx_for_callback = ctx.clone();
        let watcher_slot_for_callback = Arc::clone(watcher_slot);

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

        // Theme reloads must refresh export and instant caches before the doorbell.
        let ctx_for_theme_callback = ctx.clone();
        ctx.theme_manager
            .set_reload_callback(Arc::new(move |reload| {
                events::handle_theme_reload(&ctx_for_theme_callback, reload);
            }));

        if let Err(e) = ctx
            .config_manager
            .start_watching_with(HOT_RELOAD_DEBOUNCE, &ctx.watch_registry)
        {
            debug_log!(
                "config",
                "Warning: Failed to start config watching ({e}), config changes won't reload automatically"
            );
        } else {
            debug_log!("config", "Config hot-reload enabled");
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
                    *watcher_slot.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(initial_watcher);
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
    /// Returns an error if the endpoint handle builder fails.
    fn init_server(
        ctx: &events::AgentContext,
        watcher_slot: &SharedWatcherSlot,
        language_cache: crate::language::DetectionCache,
        socket_path: std::path::PathBuf,
    ) -> Result<EndpointHandle> {
        let builder = EndpointHandle::builder(EndpointDeps {
            client_registry: Arc::clone(&ctx.registry),
            git_cache: Arc::clone(&ctx.cache),
            config_manager: Arc::clone(&ctx.config_manager),
            theme_manager: Arc::clone(&ctx.theme_manager),
            instant_cache: Arc::clone(&ctx.instant_cache),
            language_cache,
        })
        .socket_path(socket_path)
        .watcher_slot(Arc::clone(watcher_slot))
        .palette_cache(Arc::clone(&ctx.palette_cache));
        #[cfg(unix)]
        let builder = builder.socket_dir_repair(crate::agent::lifecycle::socket_dir_repair());
        builder.build()
    }

    /// Prune dead clients from the registry and clean up the watcher.
    pub fn prune_dead_clients_and_cleanup(&self) {
        let _pruned_pids = prune_dead_clients(&self.ctx.registry, &self.watcher);
    }

    /// Stop the repository, theme and config watchers.
    fn stop_watchers(&self) {
        {
            let mut guard = self.watcher.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(watcher) = guard.take() {
                watcher.stop();
            }
        }
        self.ctx.theme_manager.stop_watching();
        self.ctx.config_manager.stop_watching();
    }

    /// Start the agent in background mode.
    ///
    /// # Errors
    /// Returns an error if signal setup fails or the IPC server fails to start or stop.
    #[expect(
        clippy::too_many_lines,
        reason = "single ordered startup/shutdown orchestration"
    )]
    pub async fn start_background(&mut self) -> Result<()> {
        startup::startup_checks(self.server.socket_path());

        // Recovery nudge: only the daemon runs it, so each tracked shell is
        // nudged once to re-register after the socket accepts.
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

        // Warm the theme-export cache on a blocking thread after accept begins;
        // racing requests fall back to `gpy-agent theme export`. If the export
        // changed, ring tracked shells with `.reload`.
        {
            let theme_manager_for_export = Arc::clone(&self.ctx.theme_manager);
            let config_for_export = self.ctx.config_manager.get();
            let palette_for_export = self.ctx.palette_cache.get();
            tokio::task::spawn_blocking(move || {
                #[cfg(unix)]
                lifecycle::start::warm_theme_export_cache(
                    &theme_manager_for_export,
                    &config_for_export,
                    &palette_for_export,
                );
                #[cfg(not(unix))]
                if let Err(e) = crate::cache::write_theme_export_cache(
                    &theme_manager_for_export,
                    &config_for_export,
                    &palette_for_export,
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
        let mut pruning_timer = startup::create_pruning_timer();
        // Bounded per-PID re-nudge budget; see `lifecycle::start::ShellRenudger`.
        #[cfg(unix)]
        let mut shell_renudger = lifecycle::start::ShellRenudger::default();
        // Reconcile catches mutations the OS watcher dropped and signals only on
        // a real diff. Skip the immediate first tick: no clients yet.
        let mut reconcile_timer =
            tokio::time::interval(Duration::from_secs(reconcile_interval_secs()));
        reconcile_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        reconcile_timer.reset();
        // Prevents overlapping reconcile scans.
        let reconcile_in_flight = Arc::new(AtomicBool::new(false));
        // Set when a tick is skipped mid-scan; consumed by the catch-up pass.
        let reconcile_pending = Arc::new(AtomicBool::new(false));
        let mut server_future = Box::pin(self.server.start());
        let mut shutdown_signal = startup::create_shutdown_signal();
        let mut last_notified_minute = 0_u64;

        loop {
            tokio::select! {
                result = &mut server_future => {
                    // IPC `Shutdown` or a removed/rebound socket: expected exits.
                    match &result {
                        Ok(()) => {
                            debug_log!("agent", "Server stopped; shutting down");
                        }
                        Err(e) => {
                            debug_log!("agent", "Server failed with error: {e}");
                        }
                    }
                    drop(server_future);
                    self.stop_watchers();
                    return result;
                }
                shutdown_result = &mut shutdown_signal => {
                    shutdown_result?;

                    debug_log!("agent", "Received shutdown signal");

                    // Drop the server future first: `stop()` needs its mutable
                    // borrow to unlink the socket, which a signal would otherwise leak.
                    drop(server_future);
                    self.stop_watchers();
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

                    // Re-nudge live tracked shells missing from the registry
                    // (e.g. their circuit breaker was backing off at restart).
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

    /// Handle a single JSON oneshot request (no background agent).
    ///
    /// # Errors
    /// Returns an error if the request is invalid or detection fails.
    pub fn handle_oneshot(request: &str) -> Result<String> {
        oneshot::handle_oneshot(request)
    }

    /// Create an Agent for integration testing with the watcher enabled.
    ///
    /// # Errors
    /// Returns an error if watcher creation fails.
    #[doc(hidden)]
    pub fn new_for_testing_with_watcher(_repo_path: &std::path::Path) -> Result<Self> {
        use crate::git::cache::GitStatusCache;
        use crate::watcher::{DebouncedEvent, WatcherConfig, multi_repo::MultiRepoWatcher};
        use std::sync::Mutex;

        // Embedded theme, default config, temp cache: never the developer's ~/.config.
        let theme_manager = Arc::new(ThemeManager::builtin("default")?);
        let config_manager = Arc::new(crate::config::manager::ConfigManager::with_defaults()?);
        let instant_cache = Arc::new(crate::cache::InstantPromptCache::new_for_test());

        let config = WatcherConfig::default();
        let events = Arc::new(Mutex::new(Vec::new()));
        let events_clone = Arc::clone(&events);
        let callback: Box<dyn Fn(DebouncedEvent) + Send + Sync> = Box::new(move |event| {
            events_clone
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(event);
        });
        let watcher = Arc::new(
            MultiRepoWatcher::builder()
                .config(config)
                .callback(callback)
                .build()?,
        );
        let watcher_slot = Arc::new(Mutex::new(Some(Arc::clone(&watcher))));

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
            theme_export_dir: std::env::temp_dir()
                .join(format!("gpy-test-theme-export-{}", std::process::id())),
            announce_gate: Arc::new(Mutex::new(())),
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
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map_or(0, |w| w.watched_repo_count())
    }
}

#[cfg(test)]
mod tests;

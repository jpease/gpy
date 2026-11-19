//! Theme manager with hot-reload support.

use crate::config::defaults::{DEFAULT_THEME_CONTENT, STARSHIP_THEME_CONTENT, TEXT_THEME_CONTENT};
use crate::config::discovery::{self, Discovered, Source};
use crate::debug_log;
use crate::plugin::discover_plugins;
use crate::sync_ext::{MutexExt, RwLockExt};
use crate::theme::model::ThemeConfig;
use crate::theme::overlay::{self, ChainLink, LoadedTheme, ThemeChain};
use crate::theme::poll::{PollPaths, PollSubject};
use crate::watcher::hot_reload::{HOT_RELOAD_DEBOUNCE, HotReloadSlot, PollTarget};
use crate::watcher::{WatchCoordinator, WatchRegistry};
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// Builtin themes: name → embedded TOML content. Single source for all builtin lookups.
pub(crate) const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("default", DEFAULT_THEME_CONTENT),
    ("text", TEXT_THEME_CONTENT),
    ("starship", STARSHIP_THEME_CONTENT),
];

/// The link the theme `name` resolves to as the requested theme (user, then
/// plugin, then builtin). Unresolvable names get the embedded link; the
/// loader reports the error.
pub(super) fn resolve_top(name: &str) -> ChainLink {
    overlay::resolve_top(name).unwrap_or_else(|| ChainLink::embedded(name))
}

/// Watches `chain` needs beyond the always-watched user themes dir (none
/// without a config root): the directory of each file outside it (a plugin's
/// `themes/`), and each file on disk, since some notify backends report
/// truncate/write saves only on a file watch.
fn watch_paths(chain: &ThemeChain, user_themes_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    for link in chain.iter() {
        if let Some(dir) = link.path.parent()
            && Some(dir) != user_themes_dir
        {
            push_unique(&mut paths, dir);
        }
        if link.on_disk {
            push_unique(&mut paths, &link.path);
        }
    }
    paths
}

/// Append `path` unless `paths` already holds it.
pub(super) fn push_unique(paths: &mut Vec<PathBuf>, path: &Path) {
    if !paths.iter().any(|known| known == path) {
        paths.push(path.to_path_buf());
    }
}

/// Active theme name, chain, and content behind one lock so readers never see a torn pairing.
#[derive(Debug, Clone)]
struct ThemeState {
    name: String,
    /// The name the config asks for. Equal to `name` unless it failed to load
    /// and `name` stands in for it; every reload then retries it first.
    requested: String,
    /// The files the theme was merged from, one link per file (path, source,
    /// on disk): the requested file first, then each base it extends.
    chain: ThemeChain,
    /// Fingerprint of the bytes of every file of the chain on disk; `None`
    /// when every link is an embedded builtin.
    fingerprint: Option<u64>,
    theme: Arc<ThemeConfig>,
}

impl ThemeState {
    fn new(name: String, loaded: LoadedTheme) -> Self {
        Self {
            requested: name.clone(),
            name,
            chain: loaded.chain,
            fingerprint: loaded.fingerprint,
            theme: Arc::new(loaded.theme),
        }
    }
}

/// What a reload of the active name found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reloaded {
    /// Same chain, same content: nothing was swapped.
    Unchanged,
    /// Same chain with new content in one of its files (the overlay or a base).
    Content,
    /// The name now resolves to a different chain: an override or overlay
    /// appeared or went away, or a base moved to another source.
    Source,
}

/// Runs every watcher- or poll-triggered reload (see
/// [`ThemeManager::set_reload_callback`]). It gets the reload step, which swaps
/// in the active theme and reports whether it changed; ringing shells is the
/// callback's job.
pub type ThemeReloadCallback = Arc<dyn Fn(&dyn Fn() -> bool) + Send + Sync>;
/// A check a loaded theme must pass before a reload or switch applies it
/// (see [`ThemeManager::set_reload_check`]); it gets the loaded theme, then
/// the active one.
pub type ThemeCheck = Arc<dyn Fn(&LoadedTheme, &ThemeConfig) -> Result<()> + Send + Sync>;
/// [`ThemeCheck`] as a borrowed function: loaded theme, then active theme.
pub type ThemeCheckFn<'check> = dyn Fn(&LoadedTheme, &ThemeConfig) -> Result<()> + 'check;
type ReloadHookSlot = Arc<Mutex<Option<ThemeReloadCallback>>>;
type CheckSlot = Arc<Mutex<Option<ThemeCheck>>>;
type WatcherSlot = Arc<Mutex<Option<WatchCoordinator>>>;

/// Run the check registered in `slot` on `loaded`, replacing `active`;
/// passes without one.
///
/// # Errors
/// Returns the registered check's error.
fn run_check(slot: &CheckSlot, loaded: &LoadedTheme, active: &ThemeConfig) -> Result<()> {
    let registered = slot.lock_or_recover().clone();
    registered.map_or(Ok(()), |check| check(loaded, active))
}

/// Manages theme loading, caching, and hot-reloading.
pub struct ThemeManager {
    state: Arc<RwLock<Arc<ThemeState>>>,
    hot_reload: HotReloadSlot,
    /// When set, runs each hot reload so the owner can serialize it, refresh
    /// caches and ring the shell doorbell.
    on_reload: ReloadHookSlot,
    /// Checked on every reload and switch before a theme is applied.
    reload_check: CheckSlot,
    watch_debounce: Arc<Mutex<Duration>>,
    /// Registry last armed with, so `switch_theme` re-arms on the shared one.
    watch_registry: Arc<Mutex<Option<Arc<WatchRegistry>>>>,
    /// Watches armed for the active chain (see [`watch_paths`]).
    /// Lock order: the watcher slot first, then this.
    source_watches: Arc<Mutex<Vec<PathBuf>>>,
    /// Serializes resolve-load-swap, so a slow reload never installs a source
    /// resolved before a faster one's.
    reload_gate: Arc<Mutex<()>>,
}

/// What a watcher or poll-thread reload needs, detached from `&ThemeManager`.
#[derive(Clone)]
struct ReloadTarget {
    state: Arc<RwLock<Arc<ThemeState>>>,
    reload_gate: Arc<Mutex<()>>,
    on_reload: ReloadHookSlot,
    reload_check: CheckSlot,
    watcher: WatcherSlot,
    source_watches: Arc<Mutex<Vec<PathBuf>>>,
}

impl ReloadTarget {
    /// Reload via the registered hook, else just swap in the reloaded theme.
    fn reload_and_apply(&self, context: &str) {
        let registered = self.on_reload.lock_or_recover().clone();
        let reload = || self.reload_step(context);
        match registered {
            // Called outside every lock: the hook reads `get()` and may re-register.
            Some(hook) => hook(&reload),
            None => {
                reload();
            }
        }
    }

    /// Reload the active name from its current chain, logging failures, and
    /// re-arm the source watches if the theme changed. Returns whether it did.
    fn reload_step(&self, context: &str) -> bool {
        let check = |loaded: &LoadedTheme, active: &ThemeConfig| {
            run_check(&self.reload_check, loaded, active)
        };
        match ThemeManager::reload_state(&self.state, &self.reload_gate, &check) {
            Ok(Reloaded::Unchanged) => return false,
            Ok(Reloaded::Content | Reloaded::Source) => {}
            Err(e) => {
                crate::debug::warn_fallback(
                    &format!("Theme reload ({context})"),
                    "keeping the current theme",
                    &e,
                );
                return false;
            }
        }
        // Only `try_lock`: `stop` holds the slot while joining the thread we run on.
        if let Ok(mut slot) = self.watcher.try_lock() {
            self.rearm(&mut slot);
        }
        true
    }

    /// Point the source watches at the active chain, leaving the user themes
    /// dir alone. Best-effort: a watch that fails is retried on the next re-arm.
    fn rearm(&self, slot: &mut Option<WatchCoordinator>) {
        let Some(coordinator) = slot.as_mut() else {
            return;
        };
        let user_themes_dir = ThemeManager::user_themes_dir().ok();
        let wanted = watch_paths(
            &self.state.read_or_recover().chain,
            user_themes_dir.as_deref(),
        );
        let mut armed = self.source_watches.lock_or_recover();
        ThemeManager::rearm_source_watches(coordinator, &mut armed, wanted);
    }
}

impl ThemeManager {
    /// Create a new theme manager with the specified theme name.
    ///
    /// # Errors
    /// Returns an error if the theme file cannot be loaded or parsed.
    pub fn new(theme_name: &str) -> Result<Self> {
        discovery::validate_config_name("theme", theme_name)?;
        let loaded = Self::load_name(theme_name)?;
        Ok(Self::with_state(theme_name, loaded))
    }

    /// Load `fallback` in place of `requested`, which failed to load. Every
    /// reload (theme watcher, poll fallback, [`reload`](Self::reload)) then
    /// retries `requested` first and switches to it once it loads and passes
    /// the reload check, so fixing the theme needs no restart. A `requested`
    /// that is not a valid theme name is never retried.
    ///
    /// # Errors
    /// Returns an error if `fallback` can't be loaded.
    pub fn standing_in_for(requested: &str, fallback: &str) -> Result<Self> {
        discovery::validate_config_name("theme", fallback)?;
        let mut state = ThemeState::new(fallback.to_owned(), Self::load_name(fallback)?);
        if discovery::validate_config_name("theme", requested).is_ok() {
            requested.clone_into(&mut state.requested);
        }
        Ok(Self::from_state(state))
    }

    fn with_state(name: &str, loaded: LoadedTheme) -> Self {
        Self::from_state(ThemeState::new(name.to_owned(), loaded))
    }

    fn from_state(state: ThemeState) -> Self {
        Self {
            state: Arc::new(RwLock::new(Arc::new(state))),
            // A removed override falls back to a lower-priority source, so a
            // vanished file is a change for the poller too.
            hot_reload: HotReloadSlot::new().reloading_on_missing(),
            on_reload: Arc::new(Mutex::new(None)),
            reload_check: Arc::new(Mutex::new(None)),
            watch_debounce: Arc::new(Mutex::new(HOT_RELOAD_DEBOUNCE)),
            watch_registry: Arc::new(Mutex::new(None)),
            source_watches: Arc::new(Mutex::new(Vec::new())),
            reload_gate: Arc::new(Mutex::new(())),
        }
    }

    /// Construct a manager from embedded builtin content, ignoring on-disk overrides.
    /// Builtins are full themes, never overlays. The recorded path is still the
    /// user location so reload/watch match [`new`](Self::new).
    ///
    /// # Errors
    /// Returns an error if `theme_name` is not a builtin or fails to parse.
    pub fn builtin(theme_name: &str) -> Result<Self> {
        if !BUILTIN_THEMES.iter().any(|(name, _)| *name == theme_name) {
            let expected = BUILTIN_THEMES
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::config(format!(
                "Unknown builtin theme: {theme_name} (expected one of: {expected})"
            )));
        }

        let loaded = overlay::load_link(ChainLink::embedded(theme_name))?;
        Ok(Self::with_state(theme_name, loaded))
    }

    /// Consistent snapshot of name, chain, and content.
    fn state(&self) -> Arc<ThemeState> {
        let guard = self.state.read_or_recover();
        Arc::clone(&guard)
    }

    /// Get the current theme configuration (cheap `Arc` clone).
    #[must_use]
    pub fn get(&self) -> Arc<ThemeConfig> {
        Arc::clone(&self.state().theme)
    }

    /// Register the hook that runs every watcher or poll reload. It is the only
    /// path from a theme reload to the shell doorbell; without it a reload just
    /// swaps the theme. Not run on [`switch_theme`](Self::switch_theme).
    pub fn set_reload_callback(&self, callback: ThemeReloadCallback) {
        *self.on_reload.lock_or_recover() = Some(callback);
    }

    /// Register the check every reload and switch runs on the theme it
    /// loaded before applying it (the agent renders the segment templates
    /// against the active palette). A theme that fails it is not applied:
    /// the current one is kept and the reload or switch reports the error.
    /// Without a check, every theme that loads is applied.
    pub fn set_reload_check(&self, check: ThemeCheck) {
        *self.reload_check.lock_or_recover() = Some(check);
    }

    /// Reload the active theme from wherever its name now resolves (user, then
    /// plugin, then builtin), re-arming the watcher if its chain changed. A
    /// fallback standing in for a requested theme that failed to load is
    /// replaced by that theme once it loads.
    ///
    /// Returns whether the loaded theme changed: a different chain, or new
    /// content in one of its files. Identical content swaps nothing, so a
    /// reload after the watcher already applied an edit reports `false`. The
    /// current theme is kept on error.
    ///
    /// # Errors
    /// Returns an error if the resolved theme can't be read or parsed, or
    /// fails the reload check.
    pub fn reload(&self) -> Result<bool> {
        let check = |loaded: &LoadedTheme, active: &ThemeConfig| {
            run_check(&self.reload_check, loaded, active)
        };
        self.reload_with(&check)
    }

    /// [`reload`](Self::reload) with `check` in place of the registered
    /// check, for a config reload whose palette is not applied yet.
    ///
    /// # Errors
    /// As [`reload`](Self::reload), with `check` as the reload check.
    pub fn reload_with(&self, check: &ThemeCheckFn<'_>) -> Result<bool> {
        let reloaded = Self::reload_state(&self.state, &self.reload_gate, check)?;
        if reloaded == Reloaded::Source {
            self.rearm_source_watches_now();
        }
        Ok(reloaded != Reloaded::Unchanged)
    }

    /// Re-resolve and load the active name's chain, swapping it in unless
    /// unchanged; first, while a fallback stands in for the requested name,
    /// try that name and switch to it if it now loads and passes `check`. A
    /// no-op if a concurrent `switch_theme` replaced the state meanwhile.
    ///
    /// # Errors
    /// Returns an error if the resolved theme can't be read or parsed, or
    /// fails `check`.
    fn reload_state(
        state_lock: &RwLock<Arc<ThemeState>>,
        reload_gate: &Mutex<()>,
        check: &ThemeCheckFn<'_>,
    ) -> Result<Reloaded> {
        let _serialized = reload_gate.lock_or_recover();
        let (name, requested, current) = {
            let active = state_lock.read_or_recover();
            (
                active.name.clone(),
                active.requested.clone(),
                Arc::clone(&active.theme),
            )
        };
        let replaced = |active: &ThemeState| active.name != name || active.requested != requested;

        if requested != name {
            match Self::load_checked(&requested, check, &current) {
                Ok(loaded) => {
                    let mut guard = state_lock.write_or_recover();
                    if replaced(&guard) {
                        return Ok(Reloaded::Unchanged);
                    }
                    *guard = Arc::new(ThemeState::new(requested, loaded));
                    drop(guard);
                    return Ok(Reloaded::Source);
                }
                Err(e) => {
                    debug_log!(
                        "theme",
                        "Theme '{requested}' still fails to load; keeping '{name}' ({e})"
                    );
                }
            }
        }

        let loaded = Self::load_checked(&name, check, &current)?;
        let mut guard = state_lock.write_or_recover();
        let reloaded = if replaced(&guard) {
            Reloaded::Unchanged
        } else if guard.chain != loaded.chain {
            Reloaded::Source
        } else if guard.fingerprint != loaded.fingerprint {
            Reloaded::Content
        } else {
            Reloaded::Unchanged
        };
        if reloaded != Reloaded::Unchanged {
            let mut state = ThemeState::new(name, loaded);
            state.requested = requested;
            *guard = Arc::new(state);
        }
        drop(guard);
        Ok(reloaded)
    }

    /// Load the theme `name` resolves to now (user, then plugin, then
    /// builtin), merging an overlay's chain.
    ///
    /// # Errors
    /// Returns an error if a file of the chain can't be read or the theme
    /// can't be parsed.
    fn load_name(name: &str) -> Result<LoadedTheme> {
        crate::theme::load::load_top_annotated(resolve_top(name))
    }

    /// [`load_name`](Self::load_name), then `check` on the loaded theme
    /// against `active`, the theme it would replace.
    ///
    /// # Errors
    /// As [`load_name`](Self::load_name), or `check`'s error.
    fn load_checked(
        name: &str,
        check: &ThemeCheckFn<'_>,
        active: &ThemeConfig,
    ) -> Result<LoadedTheme> {
        let loaded = Self::load_name(name)?;
        check(&loaded, active)?;
        Ok(loaded)
    }

    /// Re-arm the source watches from a non-watcher thread.
    fn rearm_source_watches_now(&self) {
        let target = self.reload_target();
        // Not a watcher thread, so blocking on the slot cannot self-deadlock.
        let mut slot = target.watcher.lock_or_recover();
        target.rearm(&mut slot);
    }

    /// Switch to a different theme, installed in a single write.
    ///
    /// # Errors
    /// Returns an error if the theme can't be found/parsed, fails the reload
    /// check, or a lock is poisoned.
    pub fn switch_theme(&self, new_theme_name: &str) -> Result<()> {
        let check = |loaded: &LoadedTheme, active: &ThemeConfig| {
            run_check(&self.reload_check, loaded, active)
        };
        self.switch_theme_with(new_theme_name, &check)
    }

    /// [`switch_theme`](Self::switch_theme) with `check` in place of the
    /// registered check, for a config reload whose palette is not applied yet.
    ///
    /// # Errors
    /// As [`switch_theme`](Self::switch_theme), with `check` as the reload check.
    pub fn switch_theme_with(&self, new_theme_name: &str, check: &ThemeCheckFn<'_>) -> Result<()> {
        let loaded = Self::load_checked(new_theme_name, check, &self.get())?;
        let new_state = Arc::new(ThemeState::new(new_theme_name.to_owned(), loaded));

        self.stop_watching();

        *self
            .state
            .write()
            .map_err(|e| Error::config(e.to_string()))? = new_state;

        let debounce_duration = *self
            .watch_debounce
            .lock()
            .map_err(|e| Error::config(e.to_string()))?;
        // Re-arm on the shared agent registry, not a private default.
        let watch_registry = self
            .watch_registry
            .lock()
            .map_err(|e| Error::config(e.to_string()))?
            .clone()
            .unwrap_or_else(|| Arc::new(WatchRegistry::new()));
        if let Err(error) = self.start_watching_with(debounce_duration, &watch_registry) {
            // Non-fatal: theme switched, but hot-reload is now off.
            debug_log!(
                "theme",
                "Failed to restart theme watching after switching to '{new_theme_name}': {error}"
            );
        }
        Ok(())
    }

    /// Start watching the theme's sources for changes.
    ///
    /// # Errors
    /// Returns an error if the watcher can't start or a lock is poisoned.
    pub fn start_watching(&self, debounce_duration: Duration) -> Result<()> {
        self.start_watching_with(debounce_duration, &Arc::new(WatchRegistry::new()))
    }

    /// Start watching the theme's sources, sharing `watch_registry` with the caller.
    ///
    /// The user themes dir is always watched, since an override created there
    /// outranks any plugin or builtin source; the watches of the active chain's
    /// files (an overlay's plugin base included) follow it as it changes.
    ///
    /// # Errors
    /// Returns an error if the watcher can't start or a lock is poisoned.
    pub fn start_watching_with(
        &self,
        debounce_duration: Duration,
        watch_registry: &Arc<WatchRegistry>,
    ) -> Result<()> {
        *self
            .watch_debounce
            .lock()
            .map_err(|e| Error::config(e.to_string()))? = debounce_duration;
        *self
            .watch_registry
            .lock()
            .map_err(|e| Error::config(e.to_string()))? = Some(Arc::clone(watch_registry));

        let started = self.hot_reload.start_watcher(|| {
            let target = self.reload_target();
            let callback = Box::new(move |_event: crate::watcher::DebouncedEvent| {
                target.reload_and_apply("watch callback");
            });

            let mut watcher = WatchCoordinator::new(
                debounce_duration,
                callback,
                Some(Arc::clone(watch_registry)),
            )?;
            let user_themes_dir = Self::user_themes_dir().ok();
            // Best-effort: a read-only config tree (or none) only loses
            // override detection; the active chain is still watched below.
            if let Some(dir) = &user_themes_dir
                && let Err(e) = std::fs::create_dir_all(dir)
                    .map_err(Error::from)
                    .and_then(|()| watcher.watch_directory(dir))
            {
                crate::debug::warn_fallback(
                    &format!("Watching {}", dir.display()),
                    "watching only the active theme source",
                    &e,
                );
            }
            let mut armed = self.source_watches.lock_or_recover();
            armed.clear();
            let wanted = watch_paths(&self.state().chain, user_themes_dir.as_deref());
            Self::rearm_source_watches(&mut watcher, &mut armed, wanted);
            drop(armed);
            Ok(watcher)
        })?;

        if !started {
            return Ok(());
        }

        self.start_poll_fallback(debounce_duration);
        Ok(())
    }

    /// Stop watching the theme's sources.
    pub fn stop_watching(&self) {
        self.hot_reload.stop();
        self.source_watches.lock_or_recover().clear();
    }

    fn reload_target(&self) -> ReloadTarget {
        ReloadTarget {
            state: Arc::clone(&self.state),
            reload_gate: Arc::clone(&self.reload_gate),
            on_reload: Arc::clone(&self.on_reload),
            reload_check: Arc::clone(&self.reload_check),
            watcher: self.hot_reload.watcher_handle(),
            source_watches: Arc::clone(&self.source_watches),
        }
    }

    /// Make `armed` match `wanted` on `coordinator`, unwatching stale paths
    /// first. A watch that fails is logged and left out of `armed`.
    fn rearm_source_watches(
        coordinator: &mut WatchCoordinator,
        armed: &mut Vec<PathBuf>,
        wanted: Vec<PathBuf>,
    ) {
        for stale in armed.extract_if(.., |path| !wanted.contains(path)) {
            if let Err(e) = coordinator.unwatch_directory(&stale) {
                debug_log!("theme", "Failed to unwatch {}: {e}", stale.display());
            }
        }
        for path in wanted {
            if armed.contains(&path) {
                continue;
            }
            match coordinator.watch_directory(&path) {
                Ok(()) => armed.push(path),
                Err(e) => {
                    debug_log!("theme", "Failed to watch {}: {e}", path.display());
                }
            }
        }
    }

    /// Test-only: the `WatchRegistry` currently backing the watcher.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn watch_registry_for_test(&self) -> Option<Arc<WatchRegistry>> {
        self.watch_registry.lock_or_recover().clone()
    }

    /// Test-only: the chain the active theme was merged from.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn chain_for_test(&self) -> ThemeChain {
        self.state().chain.clone()
    }

    /// List available themes with source metadata and resolved path (if file-backed).
    #[must_use]
    pub fn discover_available_themes() -> Vec<Discovered> {
        let mut merged: BTreeMap<String, Discovered> = BTreeMap::new();

        discovery::insert_builtins(&mut merged, BUILTIN_THEMES);
        Self::insert_plugin_themes(&mut merged);
        Self::insert_user_themes(&mut merged);

        merged.into_values().collect()
    }

    /// Get the directory where user themes are stored.
    ///
    /// # Errors
    /// Returns an error when there is no config root ([`crate::paths::config_root`]).
    pub fn user_themes_dir() -> Result<PathBuf> {
        Ok(crate::paths::config_root()?.join("themes"))
    }

    fn insert_plugin_themes(merged: &mut BTreeMap<String, Discovered>) {
        let discovery = discover_plugins();
        for plugin in discovery.plugins {
            let source = Source::Plugin {
                plugin_id: plugin.manifest.id.as_str().to_owned(),
            };
            if let Some(root) = &plugin.root {
                discovery::insert_toml_dir(merged, &root.join("themes"), &source);
            }
        }
    }

    fn insert_user_themes(merged: &mut BTreeMap<String, Discovered>) {
        if let Ok(dir) = Self::user_themes_dir() {
            discovery::insert_toml_dir(merged, &dir, &Source::User);
        }
    }

    /// Poll target. The chain is re-resolved whenever theme resolution's
    /// inputs change (see [`PollPaths`]): `switch_theme` can repoint it, an
    /// override appearing or vanishing moves its top, and a base can move to
    /// another source.
    fn poll_target(&self) -> PollTarget {
        let path_state = Arc::clone(&self.state);
        let target = self.reload_target();
        let poll_paths = PollPaths::default();

        PollTarget {
            path_of: Box::new(move || {
                let active = Arc::clone(&path_state.read_or_recover());
                poll_paths.get(PollSubject {
                    name: active.name.clone(),
                    requested: active.requested.clone(),
                    chain: active.chain.clone(),
                })
            }),
            reload: Box::new(move || target.reload_and_apply("poll fallback")),
        }
    }

    fn start_poll_fallback(&self, debounce_duration: Duration) {
        self.hot_reload.start_poll_fallback(
            &crate::env_config::THEME_WATCH_POLL_MS,
            debounce_duration,
            self.poll_target(),
        );
    }
}

impl Drop for ThemeManager {
    fn drop(&mut self) {
        self.stop_watching();
    }
}

#[cfg(test)]
mod tests {

    use super::{BUILTIN_THEMES, ThemeManager};
    use crate::config::Config;
    use crate::config::discovery::{Discovered, Source};
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// `new` must error naming the theme, not fall back to a blank default.
    #[test]
    fn new_errors_on_unresolvable_theme_name() {
        let Err(err) = ThemeManager::new("this-theme-definitely-does-not-exist-anywhere") else {
            panic!("an unresolvable theme name must error, not silently succeed");
        };

        assert!(
            err.to_string()
                .contains("this-theme-definitely-does-not-exist-anywhere"),
            "error message must name the theme, got: {err}"
        );
    }

    /// Plugin-sourced discovered paths survive the merge `overlay::resolve_top` relies on.
    /// (Real plugin discovery needs env mutation, which `forbid(unsafe_code)` prevents.)
    #[test]
    fn plugin_sourced_theme_candidate_resolves_via_discovered_path() {
        let mut merged: BTreeMap<String, Discovered> = BTreeMap::new();
        let plugin_theme_path = PathBuf::from("/fake/plugin/themes/acme.toml");

        crate::config::discovery::merge_candidate(
            &mut merged,
            Discovered {
                name: "acme".to_owned(),
                source: Source::Plugin {
                    plugin_id: "acme-plugin".to_owned(),
                },
                path: Some(plugin_theme_path.clone()),
            },
        );

        // Same expression `overlay::resolve_top` uses.
        let resolved = merged
            .into_values()
            .find(|theme| theme.name == "acme")
            .and_then(|theme| theme.path);

        assert_eq!(
            resolved,
            Some(plugin_theme_path),
            "a plugin-sourced theme candidate must resolve to its discovered path"
        );
    }

    /// The prompt values of `name`, a builtin theme, under the default config.
    fn builtin_values(name: &str) -> std::collections::HashMap<String, String> {
        let manager = ThemeManager::builtin(name).expect("builtin theme should load");
        crate::theme::export::prompt_shell_values(
            &manager.get(),
            &Config::default(),
            &crate::template::Palette::default(),
        )
    }

    #[test]
    fn default_theme_values_include_segment_bg_colors() {
        let values = builtin_values("default");
        for name in [
            "__color_directory_bg",
            "__color_git_clean_bg",
            "__color_language_bg",
        ] {
            assert!(
                values.get(name).is_some_and(|value| !value.is_empty()),
                "{name} must be set and non-empty: {values:?}"
            );
        }
    }

    /// `username` and `hostname` are script segments: the default theme styles
    /// them as plugin pills (the `root` variant for root), with no template flag.
    #[test]
    fn default_theme_values_include_identity_pill_styles() {
        let values = builtin_values("default");
        for (name, expected) in [
            ("__color_hostname_bg", "black"),
            ("__color_hostname_fg", "white"),
            ("__color_username_bg", "yellow"),
            ("__color_username_fg", "black"),
            ("__color_username_root_bg", "red"),
            ("__color_username_root_fg", "white"),
        ] {
            assert_eq!(
                values.get(name).map(String::as_str),
                Some(expected),
                "{name}"
            );
        }
        for gone in [
            "__hostname_format",
            "__username_format",
            "__hostname_trim_at",
        ] {
            assert!(!values.contains_key(gone), "{gone} is not a prompt value");
        }
    }

    /// The default theme is fully agent-rendered; a theme without a status
    /// format keeps the presence flag empty.
    #[test]
    fn format_presence_flags_follow_the_theme() {
        let flag = "__status_format";
        assert_eq!(
            builtin_values("default").get(flag).map(String::as_str),
            Some("1")
        );
        assert_eq!(
            builtin_values("text").get(flag).map(String::as_str),
            Some("")
        );
    }

    /// Every `BUILTIN_THEMES` entry loads via `builtin`, the loader fallback, and discovery.
    /// Discovery uses `insert_builtins` directly: real user themes could shadow builtins.
    #[test]
    fn all_builtins_load_from_every_site() {
        assert!(
            !BUILTIN_THEMES.is_empty(),
            "BUILTIN_THEMES must list at least one builtin theme"
        );

        let mut discovered = std::collections::BTreeMap::new();
        crate::config::discovery::insert_builtins(&mut discovered, BUILTIN_THEMES);

        for (name, _) in BUILTIN_THEMES {
            assert!(
                ThemeManager::builtin(name).is_ok(),
                "ThemeManager::builtin(\"{name}\") should load a theme listed in BUILTIN_THEMES"
            );

            // Site 2: `theme::load::load_theme_from_path`'s embedded-content
            // fallback, exercised via a path that does not exist on disk.
            let synthetic_path = format!("/nonexistent/path/{name}.toml");
            assert!(
                crate::theme::load::load_theme_from_path(std::path::Path::new(&synthetic_path))
                    .is_ok(),
                "load_theme_from_path should resolve embedded content for \"{name}\" via its BUILTIN_THEMES entry"
            );

            assert!(
                discovered
                    .get(*name)
                    .is_some_and(|theme| matches!(theme.source, Source::Builtin)),
                "discovery::insert_builtins should list \"{name}\" as a Builtin theme"
            );
        }
    }

    #[test]
    fn unknown_builtin_theme_error_lists_all_known_names() {
        let message = match ThemeManager::builtin("no-such-theme-xyz") {
            Ok(_) => panic!("unknown theme name must be rejected"),
            Err(e) => e.to_string(),
        };

        assert!(
            message.contains("Unknown builtin theme: no-such-theme-xyz"),
            "error should name the offending theme. Got: {message}"
        );
        for (name, _) in BUILTIN_THEMES {
            assert!(
                message.contains(name),
                "error should mention builtin \"{name}\" as an expected option. Got: {message}"
            );
        }
    }
}

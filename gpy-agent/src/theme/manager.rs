//! Theme manager with hot-reload support.

use crate::config::Config;
use crate::config::defaults::{DEFAULT_THEME_CONTENT, STARSHIP_THEME_CONTENT, TEXT_THEME_CONTENT};
use crate::config::discovery::{self, Discovered, Source};
use crate::config::types::is_safe_config_name;
use crate::debug_log;
use crate::plugin::discover_plugins;
use crate::shell::Shell;
use crate::sync_ext::{MutexExt, RwLockExt};
use crate::template::Palette;
use crate::theme::export;
use crate::theme::model::ThemeConfig;
use crate::watcher::hot_reload::{
    HOT_RELOAD_DEBOUNCE, HotReloadSlot, PollTarget, bytes_fingerprint,
};
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

/// Where a theme name resolves (user, then plugin, then builtin): its file path,
/// and whether a file exists there (otherwise the embedded builtin is loaded).
#[derive(Debug, Clone, PartialEq, Eq)]
struct ThemeLocation {
    path: PathBuf,
    on_disk: bool,
}

impl ThemeLocation {
    fn resolve(theme_name: &str) -> Self {
        let path = ThemeManager::theme_path(theme_name);
        let on_disk = path.exists();
        Self { path, on_disk }
    }

    /// Watches this source needs beyond the always-watched user themes dir: a
    /// source directory outside it (a plugin's), and the file itself, since some
    /// notify backends report truncate/write saves only on a file watch.
    fn watch_paths(&self, user_themes_dir: &Path) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        if let Some(parent) = self.path.parent()
            && parent != user_themes_dir
        {
            paths.push(parent.to_path_buf());
        }
        if self.on_disk {
            paths.push(self.path.clone());
        }
        paths
    }
}

/// Active theme name, source, and content behind one lock so readers never see a torn pairing.
#[derive(Debug, Clone)]
struct ThemeState {
    name: String,
    location: ThemeLocation,
    /// Fingerprint of the file bytes the theme was parsed from; `None` when embedded.
    fingerprint: Option<u64>,
    theme: Arc<ThemeConfig>,
}

/// What a reload of the active name found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Reloaded {
    /// Same source, same content: nothing was swapped.
    Unchanged,
    /// Same source with new content.
    Content,
    /// The name now resolves to a different source (an override appeared or went away).
    Source,
}

/// Runs every watcher- or poll-triggered reload (see
/// [`ThemeManager::set_reload_callback`]). It gets the reload step, which swaps
/// in the active theme and reports whether it changed; ringing shells is the
/// callback's job.
pub type ThemeReloadCallback = Arc<dyn Fn(&dyn Fn() -> bool) + Send + Sync>;
type ReloadHookSlot = Arc<Mutex<Option<ThemeReloadCallback>>>;
type WatcherSlot = Arc<Mutex<Option<WatchCoordinator>>>;

/// Manages theme loading, caching, and hot-reloading.
pub struct ThemeManager {
    state: Arc<RwLock<Arc<ThemeState>>>,
    hot_reload: HotReloadSlot,
    /// When set, runs each hot reload so the owner can serialize it, refresh
    /// caches and ring the shell doorbell.
    on_reload: ReloadHookSlot,
    watch_debounce: Arc<Mutex<Duration>>,
    /// Registry last armed with, so `switch_theme` re-arms on the shared one.
    watch_registry: Arc<Mutex<Option<Arc<WatchRegistry>>>>,
    /// Watches armed for the active source (see [`ThemeLocation::watch_paths`]).
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

    /// Reload the active name from its current source, logging failures, and
    /// re-arm the source watches if the theme changed. Returns whether it did.
    fn reload_step(&self, context: &str) -> bool {
        match ThemeManager::reload_state(&self.state, &self.reload_gate) {
            Ok(Reloaded::Unchanged) => return false,
            Ok(Reloaded::Content | Reloaded::Source) => {}
            Err(e) => {
                debug_log!("theme", "Failed to reload theme ({context}): {e}");
                return false;
            }
        }
        // Only `try_lock`: `stop` holds the slot while joining the thread we run on.
        if let Ok(mut slot) = self.watcher.try_lock() {
            self.rearm(&mut slot);
        }
        true
    }

    /// Point the source watches at the active location, leaving the user themes
    /// dir alone. Best-effort: a watch that fails is retried on the next re-arm.
    fn rearm(&self, slot: &mut Option<WatchCoordinator>) {
        let Some(coordinator) = slot.as_mut() else {
            return;
        };
        let wanted = self
            .state
            .read_or_recover()
            .location
            .watch_paths(&ThemeManager::user_themes_dir());
        let mut armed = self.source_watches.lock_or_recover();
        ThemeManager::rearm_source_watches(coordinator, &mut armed, wanted);
    }
}

/// Source metadata for a discovered theme (shared with palettes).
pub type ThemeSource = Source;

/// A theme entry exposed to CLI/runtime discovery.
pub type DiscoveredTheme = Discovered;

impl ThemeManager {
    /// Create a new theme manager with the specified theme name.
    ///
    /// # Errors
    /// Returns an error if the theme file cannot be loaded or parsed.
    pub fn new(theme_name: &str) -> Result<Self> {
        if !is_safe_config_name(theme_name) {
            return Err(Error::config(format!(
                "invalid theme name '{theme_name}': must not contain path separators, '..', or control characters"
            )));
        }
        let location = ThemeLocation::resolve(theme_name);
        let (theme, fingerprint) = Self::load_source(&location)?;
        Ok(Self::with_state(theme_name, location, fingerprint, theme))
    }

    fn with_state(
        name: &str,
        location: ThemeLocation,
        fingerprint: Option<u64>,
        theme: ThemeConfig,
    ) -> Self {
        Self {
            state: Arc::new(RwLock::new(Arc::new(ThemeState {
                name: name.to_owned(),
                location,
                fingerprint,
                theme: Arc::new(theme),
            }))),
            // A removed override falls back to a lower-priority source, so a
            // vanished file is a change for the poller too.
            hot_reload: HotReloadSlot::new().reloading_on_missing(),
            on_reload: Arc::new(Mutex::new(None)),
            watch_debounce: Arc::new(Mutex::new(HOT_RELOAD_DEBOUNCE)),
            watch_registry: Arc::new(Mutex::new(None)),
            source_watches: Arc::new(Mutex::new(Vec::new())),
            reload_gate: Arc::new(Mutex::new(())),
        }
    }

    /// Construct a manager from embedded builtin content, ignoring on-disk overrides.
    /// The recorded path is still the user location so reload/watch match [`new`](Self::new).
    ///
    /// # Errors
    /// Returns an error if `theme_name` is not a builtin or fails to parse.
    pub fn builtin(theme_name: &str) -> Result<Self> {
        let Some((_, content)) = BUILTIN_THEMES.iter().find(|(name, _)| *name == theme_name) else {
            let expected = BUILTIN_THEMES
                .iter()
                .map(|(name, _)| *name)
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Error::config(format!(
                "Unknown builtin theme: {theme_name} (expected one of: {expected})"
            )));
        };

        let theme = crate::theme::parse(content, theme_name)?;
        let location = ThemeLocation {
            path: Self::user_theme_path(theme_name),
            on_disk: false,
        };
        Ok(Self::with_state(theme_name, location, None, theme))
    }

    /// Consistent snapshot of name, location, and content.
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

    /// Reload the active theme from wherever its name now resolves (user, then
    /// plugin, then builtin), re-arming the watcher if that source moved.
    ///
    /// Returns whether the loaded theme changed: a different source, or new
    /// content in the same file. Identical content swaps nothing, so a reload
    /// after the watcher already applied an edit reports `false`. The current
    /// theme is kept on error.
    ///
    /// # Errors
    /// Returns an error if the resolved theme can't be read or parsed.
    pub fn reload(&self) -> Result<bool> {
        let reloaded = Self::reload_state(&self.state, &self.reload_gate)?;
        if reloaded == Reloaded::Source {
            self.rearm_source_watches_now();
        }
        Ok(reloaded != Reloaded::Unchanged)
    }

    /// Re-resolve and load the active name, swapping it in unless unchanged.
    /// A no-op if a concurrent `switch_theme` replaced the name meanwhile.
    ///
    /// # Errors
    /// Returns an error if the resolved theme can't be read or parsed.
    fn reload_state(
        state_lock: &RwLock<Arc<ThemeState>>,
        reload_gate: &Mutex<()>,
    ) -> Result<Reloaded> {
        let _serialized = reload_gate.lock_or_recover();
        let name = state_lock.read_or_recover().name.clone();
        let location = ThemeLocation::resolve(&name);
        let (theme, fingerprint) = Self::load_source(&location)?;

        let mut guard = state_lock.write_or_recover();
        let reloaded = if guard.name != name {
            Reloaded::Unchanged
        } else if guard.location != location {
            Reloaded::Source
        } else if guard.fingerprint != fingerprint {
            Reloaded::Content
        } else {
            Reloaded::Unchanged
        };
        if reloaded != Reloaded::Unchanged {
            *guard = Arc::new(ThemeState {
                name,
                location,
                fingerprint,
                theme: Arc::new(theme),
            });
        }
        drop(guard);
        Ok(reloaded)
    }

    /// Load the theme at `location`, with the fingerprint of the file bytes
    /// parsed (`None` for an embedded builtin).
    ///
    /// # Errors
    /// Returns an error if the file can't be read or the theme can't be parsed.
    fn load_source(location: &ThemeLocation) -> Result<(ThemeConfig, Option<u64>)> {
        if !location.on_disk {
            return Ok((Self::load_from_path(&location.path)?, None));
        }
        let source = location.path.display().to_string();
        let content = std::fs::read_to_string(&location.path)
            .map_err(|e| Error::config(format!("Failed to read theme file {source}: {e}")))?;
        let fingerprint = bytes_fingerprint(content.as_bytes());
        Ok((crate::theme::parse(&content, &source)?, Some(fingerprint)))
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
    /// Returns an error if the theme can't be found/parsed or a lock is poisoned.
    pub fn switch_theme(&self, new_theme_name: &str) -> Result<()> {
        let location = ThemeLocation::resolve(new_theme_name);
        let (new_theme, fingerprint) = Self::load_source(&location)?;
        let new_state = Arc::new(ThemeState {
            name: new_theme_name.to_owned(),
            location,
            fingerprint,
            theme: Arc::new(new_theme),
        });

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
    /// outranks any plugin or builtin source; the active source's own watches
    /// follow it as it moves.
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
            let user_themes_dir = Self::user_themes_dir();
            // Best-effort: a read-only config tree only loses override detection;
            // the active source is still watched below.
            if let Err(e) = std::fs::create_dir_all(&user_themes_dir)
                .map_err(Error::from)
                .and_then(|()| watcher.watch_directory(&user_themes_dir))
            {
                crate::debug::warn_fallback(
                    &format!("Watching {}", user_themes_dir.display()),
                    "watching only the active theme source",
                    &e,
                );
            }
            let mut armed = self.source_watches.lock_or_recover();
            armed.clear();
            let wanted = self.state().location.watch_paths(&user_themes_dir);
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

    /// Export the theme for `shell` from one consistent state snapshot, with
    /// palette roles resolved through `palette`.
    #[must_use]
    pub fn export(&self, shell: Shell, config: &Config, palette: &Palette) -> String {
        let state = self.state();
        let plugin_files = if matches!(shell, Shell::Fish) {
            Self::collect_plugin_segment_files()
        } else {
            BTreeMap::new()
        };
        export::theme_to_shell(
            &state.theme,
            &state.name,
            config,
            shell,
            palette,
            &plugin_files,
        )
    }

    /// Discover third-party `.fish` plugin segment file paths, keyed by segment name.
    fn collect_plugin_segment_files() -> BTreeMap<String, String> {
        let discovery = discover_plugins();
        let mut files = BTreeMap::new();
        for plugin in discovery.plugins {
            if matches!(plugin.source, crate::plugin::PluginSource::FirstParty)
                || !matches!(plugin.manifest.entry_type, crate::plugin::EntryType::File)
            {
                continue;
            }
            for provided_segment in plugin.manifest.provided_segments {
                let segment_name = provided_segment.to_string();
                let segment_file = plugin
                    .root
                    .join("segments")
                    .join(format!("{segment_name}.fish"));
                files
                    .entry(segment_name)
                    .or_insert_with(|| segment_file.to_string_lossy().into_owned());
            }
        }
        files
    }

    /// List all available themes.
    #[must_use]
    pub fn list_available_themes() -> Vec<String> {
        Self::discover_available_themes()
            .into_iter()
            .map(|theme| theme.name)
            .collect()
    }

    /// List available themes with source metadata and resolved path (if file-backed).
    #[must_use]
    pub fn discover_available_themes() -> Vec<DiscoveredTheme> {
        let mut merged: BTreeMap<String, DiscoveredTheme> = BTreeMap::new();

        discovery::insert_builtins(&mut merged, BUILTIN_THEMES);
        Self::insert_plugin_themes(&mut merged);
        Self::insert_user_themes(&mut merged);

        merged.into_values().collect()
    }

    /// Get the default theme template.
    #[must_use]
    pub const fn default_theme_template() -> &'static str {
        DEFAULT_THEME_CONTENT
    }

    /// Raw TOML source of a discovered theme, verbatim, for use as a clone base.
    ///
    /// # Errors
    /// Returns an error if the theme isn't discoverable or its file can't be read.
    pub fn theme_source_content(theme_name: &str) -> Result<String> {
        if !is_safe_config_name(theme_name) {
            return Err(Error::config(format!(
                "invalid theme name '{theme_name}': must not contain path separators, '..', or control characters"
            )));
        }

        let Some(discovered) = Self::discover_available_themes()
            .into_iter()
            .find(|theme| theme.name == theme_name)
        else {
            return Err(Error::config(format!(
                "No discovered theme named '{theme_name}'"
            )));
        };

        if discovered.source == ThemeSource::Builtin {
            let Some((_, content)) = BUILTIN_THEMES.iter().find(|(name, _)| *name == theme_name)
            else {
                return Err(Error::config(format!(
                    "Unknown builtin theme: {theme_name}"
                )));
            };
            return Ok((*content).to_owned());
        }

        let Some(path) = discovered.path else {
            return Err(Error::config(format!(
                "Theme '{theme_name}' has no backing file"
            )));
        };

        std::fs::read_to_string(&path).map_err(|e| {
            Error::config(format!("Failed to read theme file {}: {e}", path.display()))
        })
    }

    /// Get the directory where user themes are stored.
    #[must_use]
    pub fn user_themes_dir() -> PathBuf {
        crate::paths::config_root_for(
            crate::paths::root_var("XDG_CONFIG_HOME").as_deref(),
            crate::paths::home_dir().as_deref(),
            crate::paths::Os::host(),
        )
        .join("themes")
    }

    fn user_theme_path(theme_name: &str) -> PathBuf {
        Self::user_themes_dir().join(format!("{theme_name}.toml"))
    }

    /// Write `theme` as a user override under `user_themes_dir()`; never touches builtin/plugin files.
    /// The file is replaced atomically, so a failed save keeps the previous theme, and a symlinked
    /// theme file is written through.
    ///
    /// # Errors
    /// Returns an error on an unsafe name, serialization failure, or I/O failure.
    pub fn save_user_theme(theme_name: &str, theme: &ThemeConfig) -> Result<PathBuf> {
        if !is_safe_config_name(theme_name) {
            return Err(Error::config(format!(
                "invalid theme name '{theme_name}': must not contain path separators, '..', or control characters"
            )));
        }

        let toml_content = toml::to_string_pretty(theme)
            .map_err(|e| Error::config(format!("Failed to serialize theme to TOML: {e}")))?;

        let themes_dir = Self::user_themes_dir();
        std::fs::create_dir_all(&themes_dir)
            .map_err(|e| Error::config(format!("Failed to create themes directory: {e}")))?;

        let target_path = themes_dir.join(format!("{theme_name}.toml"));
        let content_with_header = format!(
            "# GPY Theme: {theme_name}\n# Written by `gpy config wizard` — edit carefully\n\n{toml_content}"
        );
        crate::fs_util::write_atomically(&target_path, content_with_header.as_bytes())
            .map_err(|e| Error::config(format!("Failed to write theme file: {e}")))?;

        Ok(target_path)
    }

    /// # Errors
    /// Returns an error if the file cannot be read or the TOML cannot be parsed.
    fn load_from_path(path: &Path) -> Result<ThemeConfig> {
        crate::config::loader::load_theme_from_path(&path.display().to_string())
    }

    /// Resolve `theme_name` (user, plugin, then builtin), shared by daemon and oneshot.
    /// Builtins resolve to their user path; the loader falls back to embedded content by stem.
    pub(crate) fn resolve_path(theme_name: &str) -> Option<PathBuf> {
        let user_path = Self::user_theme_path(theme_name);
        if user_path.exists() {
            return Some(user_path);
        }

        if let Some(path) = Self::discover_available_themes()
            .into_iter()
            .find(|theme| theme.name == theme_name)
            .and_then(|theme| theme.path)
        {
            return Some(path);
        }

        if BUILTIN_THEMES.iter().any(|(name, _)| *name == theme_name) {
            return Some(user_path);
        }

        None
    }

    fn theme_path(theme_name: &str) -> PathBuf {
        // Unresolvable names get the user path; the loader reports the error.
        Self::resolve_path(theme_name).unwrap_or_else(|| Self::user_theme_path(theme_name))
    }

    fn insert_plugin_themes(merged: &mut BTreeMap<String, DiscoveredTheme>) {
        let discovery = discover_plugins();
        for plugin in discovery.plugins {
            let source = ThemeSource::Plugin {
                plugin_id: plugin.manifest.id.as_str().to_owned(),
            };
            discovery::insert_toml_dir(merged, &plugin.root.join("themes"), &source);
        }
    }

    fn insert_user_themes(merged: &mut BTreeMap<String, DiscoveredTheme>) {
        discovery::insert_toml_dir(merged, &Self::user_themes_dir(), &ThemeSource::User);
    }

    /// Poll target. The path is re-resolved each tick: `switch_theme` can
    /// repoint it, and an override appearing or vanishing moves it.
    fn poll_target(&self) -> PollTarget {
        let path_state = Arc::clone(&self.state);
        let target = self.reload_target();

        PollTarget {
            path_of: Box::new(move || {
                let name = path_state.read_or_recover().name.clone();
                Some(ThemeLocation::resolve(&name).path)
            }),
            reload: Box::new(move |_path: &Path| target.reload_and_apply("poll fallback")),
        }
    }

    fn start_poll_fallback(&self, debounce_duration: Duration) {
        self.hot_reload.start_poll_fallback(
            "GPY_THEME_WATCH_POLL_MS",
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
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{BUILTIN_THEMES, DiscoveredTheme, ThemeLocation, ThemeManager, ThemeSource};
    use crate::config::Config;
    use crate::shell::Shell;
    use crate::theme::ThemeConfig;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    /// A manager holding `theme` under `name`, with no file behind it.
    fn manager_with(name: &str, theme: ThemeConfig) -> ThemeManager {
        let location = ThemeLocation {
            path: PathBuf::from("/tmp/gpy-test-theme.toml"),
            on_disk: false,
        };
        ThemeManager::with_state(name, location, None, theme)
    }

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

    /// Plugin-sourced discovered paths survive the merge `resolve_path` relies on.
    /// (Real plugin discovery needs env mutation, which `forbid(unsafe_code)` prevents.)
    #[test]
    fn plugin_sourced_theme_candidate_resolves_via_discovered_path() {
        let mut merged: BTreeMap<String, DiscoveredTheme> = BTreeMap::new();
        let plugin_theme_path = PathBuf::from("/fake/plugin/themes/acme.toml");

        crate::config::discovery::merge_candidate(
            &mut merged,
            DiscoveredTheme {
                name: "acme".to_owned(),
                source: ThemeSource::Plugin {
                    plugin_id: "acme-plugin".to_owned(),
                },
                path: Some(plugin_theme_path.clone()),
            },
        );

        // Same expression `resolve_path` uses.
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

    #[test]
    fn export_escapes_injected_theme_value() {
        // End-to-end through the real `export` path: an unvalidated `trim_at`
        // value must be emitted with an escaped `$`, not raw.
        let base = ThemeManager::builtin("default").expect("default theme should load");
        let mut theme = (*base.get()).clone();
        theme.segments.hostname.trim_at = "$(touch /tmp/gpy_pwned)".to_owned();
        let manager = manager_with("default", theme);

        let output = manager.export(
            Shell::Bash,
            &Config::default(),
            &crate::template::Palette::default(),
        );

        assert!(
            output.contains("__hostname_trim_at=\"\\$(touch /tmp/gpy_pwned)\""),
            "trim_at must be exported with an escaped dollar. Got:\n{output}"
        );
        assert!(
            !output.contains("__hostname_trim_at=\"$(touch"),
            "raw unescaped command substitution must not appear. Got:\n{output}"
        );
    }

    #[test]
    fn export_escapes_theme_name() {
        // `theme_name` is only validated by `is_safe_config_name`, which allows
        // `$`, backtick, and `"`; it must be escaped like every other value.
        let base = ThemeManager::builtin("default").expect("default theme should load");
        let theme = (*base.get()).clone();
        let manager = manager_with("weird$(touch x)", theme);

        let output = manager.export(
            Shell::Fish,
            &Config::default(),
            &crate::template::Palette::default(),
        );

        assert!(
            output.contains("set -g __gpy_theme_name \"weird\\$(touch x)\""),
            "theme name must be exported with an escaped dollar. Got:\n{output}"
        );
    }

    #[test]
    fn theme_export_includes_segment_bg_color_vars() {
        let manager = ThemeManager::builtin("default").expect("default theme should load");
        let output = manager.export(
            Shell::Fish,
            &Config::default(),
            &crate::template::Palette::default(),
        );

        assert!(
            output.contains("__color_directory_bg"),
            "export should include __color_directory_bg. Got:\n{output}"
        );
        assert!(
            !output.contains("__color_directory_bg \"\""),
            "export should not emit empty __color_directory_bg. Got:\n{output}"
        );
        assert!(
            output.contains("__color_git_clean_bg"),
            "export should include __color_git_clean_bg. Got:\n{output}"
        );
        assert!(
            !output.contains("__color_git_clean_bg \"\""),
            "export should not emit empty __color_git_clean_bg. Got:\n{output}"
        );
        assert!(
            output.contains("__color_language_bg"),
            "export should include __color_language_bg. Got:\n{output}"
        );
        assert!(
            !output.contains("__color_language_bg \"\""),
            "export should not emit empty __color_language_bg. Got:\n{output}"
        );
    }

    #[test]
    fn theme_export_includes_hostname_vars_for_default_theme() {
        let manager = ThemeManager::builtin("default").expect("default theme should load");
        let output = manager.export(
            Shell::Fish,
            &Config::default(),
            &crate::template::Palette::default(),
        );

        assert!(
            output.contains("__color_hostname_bg"),
            "export should include __color_hostname_bg. Got:\n{output}"
        );
        assert!(
            !output.contains("__color_hostname_bg \"\""),
            "export should not emit empty __color_hostname_bg. Got:\n{output}"
        );
        assert!(
            output.contains("__color_hostname_fg"),
            "export should include __color_hostname_fg. Got:\n{output}"
        );
        assert!(
            !output.contains("__color_hostname_fg \"\""),
            "export should not emit empty __color_hostname_fg. Got:\n{output}"
        );
        assert!(
            output.contains("set -g __hostname_trim_at \".\""),
            "export should include default __hostname_trim_at of \".\". Got:\n{output}"
        );
        assert!(
            output.contains("set -g __hostname_show_always \"0\""),
            "default theme should export __hostname_show_always as 0. Got:\n{output}"
        );
        assert!(
            output.contains("set -g __hostname_format \"1\""),
            "__hostname_format must be the presence flag \"1\" for the default theme. Got:\n{output}"
        );
        assert!(
            output.contains("set -g __icon_hostname \"\""),
            "__icon_hostname must be present but empty when icon is None. Got:\n{output}"
        );
    }

    #[test]
    fn theme_export_includes_username_vars_for_default_theme() {
        let manager = ThemeManager::builtin("default").expect("default theme should load");
        let output = manager.export(
            Shell::Fish,
            &Config::default(),
            &crate::template::Palette::default(),
        );

        assert!(
            output.contains("set -g __color_username_bg \"red\""),
            "default theme should export __color_username_bg of \"red\". Got:\n{output}"
        );
        assert!(
            output.contains("set -g __color_username_fg \"white\""),
            "default theme should export __color_username_fg of \"white\". Got:\n{output}"
        );
        assert!(
            output.contains("set -g __username_show_always \"0\""),
            "default theme should export __username_show_always as 0. Got:\n{output}"
        );
        assert!(
            output.contains("set -g __username_format \"1\""),
            "__username_format must be the presence flag \"1\" for the default theme. Got:\n{output}"
        );
        assert!(
            output.contains("set -g __icon_username \"\""),
            "__icon_username must be present but empty when icon is None. Got:\n{output}"
        );
    }

    #[test]
    fn theme_export_username_format_presence_flag_for_starship_theme() {
        let manager = ThemeManager::builtin("starship").expect("starship theme should load");
        let output = manager.export(
            Shell::Fish,
            &Config::default(),
            &crate::template::Palette::default(),
        );

        // Presence flag only; the raw format string is never exported.
        assert!(
            output.contains("set -g __username_format \"1\""),
            "starship preset sets a username format -> presence flag \"1\". Got:\n{output}"
        );
        assert!(
            !output.contains("$username"),
            "the raw username format string must never be exported. Got:\n{output}"
        );
    }

    /// The default theme is fully agent-rendered; a theme without formats keeps the
    /// presence flags empty so the shells draw the pills locally.
    #[test]
    fn theme_export_format_presence_flags_follow_the_theme() {
        let export = |name: &str| {
            ThemeManager::builtin(name)
                .expect("builtin theme should load")
                .export(
                    Shell::Fish,
                    &Config::default(),
                    &crate::template::Palette::default(),
                )
        };
        let default = export("default");
        let text = export("text");
        for flag in ["__hostname_format", "__username_format", "__status_format"] {
            assert!(
                default.contains(&format!("set -g {flag} \"1\"")),
                "default theme has a {flag} format -> presence flag \"1\". Got:\n{default}"
            );
            assert!(
                text.contains(&format!("set -g {flag} \"\"")),
                "text theme has no {flag} format -> empty flag. Got:\n{text}"
            );
        }
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

            // Site 2: `config::loader::load_theme_from_path`'s embedded-content
            // fallback, exercised via a path that does not exist on disk.
            let synthetic_path = format!("/nonexistent/path/{name}.toml");
            assert!(
                crate::config::loader::load_theme_from_path(&synthetic_path).is_ok(),
                "load_theme_from_path should resolve embedded content for \"{name}\" via its BUILTIN_THEMES entry"
            );

            assert!(
                discovered
                    .get(*name)
                    .is_some_and(|theme| matches!(theme.source, ThemeSource::Builtin)),
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

    #[test]
    fn theme_source_content_rejects_unknown_theme_name() {
        let message = match ThemeManager::theme_source_content("no-such-theme-xyz") {
            Ok(_) => panic!("unknown theme name must be rejected"),
            Err(e) => e.to_string(),
        };
        assert!(
            message.contains("no-such-theme-xyz"),
            "error should name the offending theme. Got: {message}"
        );
    }
}

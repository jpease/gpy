//! Theme manager with hot-reload support.

use crate::config::Config;
use crate::config::defaults::{DEFAULT_THEME_CONTENT, STARSHIP_THEME_CONTENT, TEXT_THEME_CONTENT};
use crate::config::discovery::{self, Discovered, Source};
use crate::config::types::is_safe_config_name;
use crate::debug_log;
use crate::plugin::discover_plugins;
use crate::shell::Shell;
use crate::theme::export;
use crate::theme::model::ThemeConfig;
use crate::watcher::hot_reload::{HotReloadSlot, PollTarget};
use crate::watcher::{WatchCoordinator, WatchRegistry};
use crate::{Error, Result};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

/// Builtin themes: name → embedded TOML content. Single source for all builtin lookups.
pub(crate) const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("default", DEFAULT_THEME_CONTENT),
    ("text", TEXT_THEME_CONTENT),
    ("starship", STARSHIP_THEME_CONTENT),
];

/// Active theme name, path, and content behind one lock so readers never see a torn pairing.
#[derive(Debug, Clone)]
struct ThemeState {
    name: String,
    path: PathBuf,
    theme: Arc<ThemeConfig>,
}

/// Callback run after a successful hot-reload swap (see [`ThemeManager::set_reload_callback`]).
pub type ThemeReloadCallback = Arc<dyn Fn() + Send + Sync>;
type ReloadHookSlot = Arc<Mutex<Option<ThemeReloadCallback>>>;

/// Manages theme loading, caching, and hot-reloading.
pub struct ThemeManager {
    state: Arc<RwLock<Arc<ThemeState>>>,
    hot_reload: HotReloadSlot,
    /// When set, replaces `notify_reload` so the owner can refresh caches first.
    on_reload: ReloadHookSlot,
    client_registry: Arc<Mutex<Option<Arc<crate::ipc::ClientDirectory>>>>,
    watch_debounce: Arc<Mutex<Duration>>,
    /// Registry last armed with, so `switch_theme` re-arms on the shared one.
    watch_registry: Arc<Mutex<Option<Arc<WatchRegistry>>>>,
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
        let theme_path = Self::theme_path(theme_name);
        let theme = Self::load_from_path(&theme_path)?;
        Ok(Self::with_theme_and_path(theme_name, theme, theme_path))
    }

    fn with_theme_and_path(name: &str, theme: ThemeConfig, path: std::path::PathBuf) -> Self {
        Self {
            state: Arc::new(RwLock::new(Arc::new(ThemeState {
                name: name.to_owned(),
                path,
                theme: Arc::new(theme),
            }))),
            hot_reload: HotReloadSlot::new(),
            on_reload: Arc::new(Mutex::new(None)),
            client_registry: Arc::new(Mutex::new(None)),
            watch_debounce: Arc::new(Mutex::new(Duration::from_secs(5_u64))),
            watch_registry: Arc::new(Mutex::new(None)),
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
        let theme_path = Self::user_theme_path(theme_name);
        Ok(Self::with_theme_and_path(theme_name, theme, theme_path))
    }

    /// Consistent snapshot of name, path, and content.
    fn state(&self) -> Arc<ThemeState> {
        let guard = self.state.read().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(&guard)
    }

    /// Get the current theme configuration (cheap `Arc` clone).
    #[must_use]
    pub fn get(&self) -> Arc<ThemeConfig> {
        Arc::clone(&self.state().theme)
    }

    /// Register the post-reload hook; it replaces the `notify_reload` doorbell.
    /// Not run on [`switch_theme`](Self::switch_theme).
    pub fn set_reload_callback(&self, callback: ThemeReloadCallback) {
        *self
            .on_reload
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(callback);
    }

    /// Reload the current theme from disk.
    ///
    /// # Errors
    /// Returns an error if the theme can't be read/parsed or the lock is poisoned.
    pub fn reload(&self) -> Result<()> {
        let current = self.state();
        let new_theme = Self::load_from_path(&current.path)?;

        let mut guard = self
            .state
            .write()
            .map_err(|e| Error::config(e.to_string()))?;
        *guard = Arc::new(ThemeState {
            name: current.name.clone(),
            path: current.path.clone(),
            theme: Arc::new(new_theme),
        });
        drop(guard);
        Ok(())
    }

    /// Switch to a different theme, installed in a single write.
    ///
    /// # Errors
    /// Returns an error if the theme can't be found/parsed or a lock is poisoned.
    pub fn switch_theme(&self, new_theme_name: &str) -> Result<()> {
        let new_theme_path = Self::theme_path(new_theme_name);
        let new_theme = Self::load_from_path(&new_theme_path)?;
        let new_state = Arc::new(ThemeState {
            name: new_theme_name.to_owned(),
            path: new_theme_path,
            theme: Arc::new(new_theme),
        });

        self.stop_watching();

        *self
            .state
            .write()
            .map_err(|e| Error::config(e.to_string()))? = new_state;

        let client_registry = self
            .client_registry
            .lock()
            .map_err(|e| Error::config(e.to_string()))?
            .clone();
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
        if let Err(error) =
            self.start_watching_with(debounce_duration, client_registry, &watch_registry)
        {
            // Non-fatal: theme switched, but hot-reload is now off.
            debug_log!(
                "theme",
                "Failed to restart theme watching after switching to '{new_theme_name}': {error}"
            );
        }
        Ok(())
    }

    /// Start watching the theme file for changes.
    ///
    /// # Errors
    /// Returns an error if the watcher can't start or a lock is poisoned.
    pub fn start_watching(
        &self,
        debounce_duration: Duration,
        client_registry: Option<Arc<crate::ipc::ClientDirectory>>,
    ) -> Result<()> {
        self.start_watching_with(
            debounce_duration,
            client_registry,
            &Arc::new(WatchRegistry::new()),
        )
    }

    /// Start watching the theme file, sharing `watch_registry` with the caller.
    ///
    /// # Errors
    /// Returns an error if the watcher can't start or a lock is poisoned.
    pub fn start_watching_with(
        &self,
        debounce_duration: Duration,
        client_registry: Option<Arc<crate::ipc::ClientDirectory>>,
        watch_registry: &Arc<WatchRegistry>,
    ) -> Result<()> {
        self.client_registry
            .lock()
            .map_err(|e| Error::config(e.to_string()))?
            .clone_from(&client_registry);
        *self
            .watch_debounce
            .lock()
            .map_err(|e| Error::config(e.to_string()))? = debounce_duration;
        *self
            .watch_registry
            .lock()
            .map_err(|e| Error::config(e.to_string()))? = Some(Arc::clone(watch_registry));

        let started = self.hot_reload.start_watcher(|| {
            let state_arc = Arc::clone(&self.state);
            let callback_registry = client_registry.clone();
            let on_reload = Arc::clone(&self.on_reload);

            let callback = Box::new(move |_event: crate::watcher::DebouncedEvent| {
                let theme_path = state_arc
                    .read()
                    .unwrap_or_else(PoisonError::into_inner)
                    .path
                    .clone();
                Self::reload_and_apply(
                    &state_arc,
                    callback_registry.as_deref(),
                    &on_reload,
                    &theme_path,
                    "watch callback",
                );
            });

            let mut watcher = WatchCoordinator::new(
                debounce_duration,
                callback,
                Some(Arc::clone(watch_registry)),
            )?;
            let theme_path = self.state().path.clone();
            if let Some(parent) = theme_path.parent() {
                if !parent.exists() {
                    std::fs::create_dir_all(parent).map_err(|e| Error::config(e.to_string()))?;
                }
                watcher.watch_directory(parent)?;
            }
            if theme_path.exists() {
                // Some notify backends need a file watch too for truncate/write saves.
                watcher.watch_directory(&theme_path)?;
            }
            Ok(watcher)
        })?;

        if !started {
            return Ok(());
        }

        self.start_poll_fallback(client_registry, debounce_duration);
        Ok(())
    }

    /// Stop watching the theme file.
    pub fn stop_watching(&self) {
        self.hot_reload.stop();
    }

    /// Test-only: the `WatchRegistry` currently backing the watcher.
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn watch_registry_for_test(&self) -> Option<Arc<WatchRegistry>> {
        self.watch_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Export the theme for `shell` from one consistent state snapshot.
    #[must_use]
    pub fn export(&self, shell: Shell, config: &Config) -> String {
        let state = self.state();
        let plugin_files = Self::collect_plugin_segment_files();
        export::theme_to_shell(&state.theme, &state.name, config, shell, &plugin_files)
    }

    /// Discover third-party plugin segment file paths, keyed by segment name.
    fn collect_plugin_segment_files() -> BTreeMap<String, String> {
        let discovery = discover_plugins();
        let mut files = BTreeMap::new();
        for plugin in discovery.plugins {
            if matches!(plugin.source, crate::plugin::PluginSource::FirstParty) {
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
        std::fs::write(&target_path, content_with_header)
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

    /// Reload from `theme_path` and swap state in one write, logging failures.
    /// Runs the reload hook if registered, else `notify_reload`.
    fn reload_and_apply(
        state_lock: &Arc<RwLock<Arc<ThemeState>>>,
        client_registry: Option<&crate::ipc::ClientDirectory>,
        on_reload: &ReloadHookSlot,
        theme_path: &Path,
        context: &str,
    ) {
        let path = theme_path.display();
        match Self::load_from_path(theme_path) {
            Ok(new_theme) => {
                let mut guard = state_lock.write().unwrap_or_else(PoisonError::into_inner);
                let current = Arc::clone(&guard);
                *guard = Arc::new(ThemeState {
                    name: current.name.clone(),
                    path: current.path.clone(),
                    theme: Arc::new(new_theme),
                });
                drop(guard);

                // Called outside both locks: the hook reads `get()` and may re-register.
                let registered = on_reload
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone();
                if let Some(hook) = registered {
                    hook();
                } else if let Some(registry) = client_registry {
                    registry.notify_reload();
                }
            }
            Err(e) => {
                debug_log!(
                    "theme",
                    "Failed to reload theme from {path} ({context}): {e}"
                );
            }
        }
    }

    /// Poll target; the path is re-read each tick because `switch_theme` can repoint it.
    fn poll_target(&self, client_registry: Option<Arc<crate::ipc::ClientDirectory>>) -> PollTarget {
        let path_state = Arc::clone(&self.state);
        let reload_state = Arc::clone(&self.state);
        let on_reload = Arc::clone(&self.on_reload);

        PollTarget {
            path_of: Box::new(move || {
                Some(
                    path_state
                        .read()
                        .unwrap_or_else(PoisonError::into_inner)
                        .path
                        .clone(),
                )
            }),
            reload: Box::new(move |path: &Path| {
                Self::reload_and_apply(
                    &reload_state,
                    client_registry.as_deref(),
                    &on_reload,
                    path,
                    "poll fallback",
                );
            }),
        }
    }

    fn start_poll_fallback(
        &self,
        client_registry: Option<Arc<crate::ipc::ClientDirectory>>,
        debounce_duration: Duration,
    ) {
        self.hot_reload.start_poll_fallback(
            "GPY_THEME_WATCH_POLL_MS",
            debounce_duration,
            self.poll_target(client_registry),
        );
    }

    /// Spawn the poll thread with a given interval; lets tests avoid `env::set_var` (unsafe).
    #[cfg(test)]
    fn spawn_poll_thread(
        &self,
        client_registry: Option<Arc<crate::ipc::ClientDirectory>>,
        poll_interval: Duration,
        debounce_duration: Duration,
    ) {
        self.hot_reload.spawn_poll_thread(
            poll_interval,
            debounce_duration,
            self.poll_target(client_registry),
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

    use super::{BUILTIN_THEMES, DiscoveredTheme, ThemeManager, ThemeSource};
    use crate::config::Config;
    use crate::config::defaults::DEFAULT_THEME_CONTENT;
    use crate::shell::Shell;
    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use std::sync::{Arc, Mutex, PoisonError};
    use std::time::Duration;

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
        let manager = ThemeManager::with_theme_and_path(
            "default",
            theme,
            std::path::PathBuf::from("/tmp/gpy-test-theme.toml"),
        );

        let output = manager.export(Shell::Bash, &Config::default());

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
        let manager = ThemeManager::with_theme_and_path(
            "weird$(touch x)",
            theme,
            std::path::PathBuf::from("/tmp/gpy-test-theme.toml"),
        );

        let output = manager.export(Shell::Fish, &Config::default());

        assert!(
            output.contains("set -g __gpy_theme_name \"weird\\$(touch x)\""),
            "theme name must be exported with an escaped dollar. Got:\n{output}"
        );
    }

    #[test]
    fn theme_export_includes_segment_bg_color_vars() {
        let manager = ThemeManager::builtin("default").expect("default theme should load");
        let output = manager.export(Shell::Fish, &Config::default());

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
        let output = manager.export(Shell::Fish, &Config::default());

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
            output.contains("set -g __hostname_format \"\""),
            "__hostname_format must be present but empty when format is None. Got:\n{output}"
        );
        assert!(
            output.contains("set -g __icon_hostname \"\""),
            "__icon_hostname must be present but empty when icon is None. Got:\n{output}"
        );
    }

    #[test]
    fn theme_export_includes_username_vars_for_default_theme() {
        let manager = ThemeManager::builtin("default").expect("default theme should load");
        let output = manager.export(Shell::Fish, &Config::default());

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
            output.contains("set -g __username_format \"\""),
            "__username_format must be present but empty when format is None. Got:\n{output}"
        );
        assert!(
            output.contains("set -g __icon_username \"\""),
            "__icon_username must be present but empty when icon is None. Got:\n{output}"
        );
    }

    #[test]
    fn theme_export_username_format_presence_flag_for_starship_theme() {
        let manager = ThemeManager::builtin("starship").expect("starship theme should load");
        let output = manager.export(Shell::Fish, &Config::default());

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

    #[test]
    fn poll_fallback_detects_a_toctou_same_length_rewrite() {
        // Immediate post-spawn, same-length rewrite: models the TOCTOU race and
        // mtime-granularity blind spot. `time_format` keeps the anchor clock-specific.
        let variant_a = DEFAULT_THEME_CONTENT;
        let variant_b = variant_a.replacen(
            "text_color = \"white\"\nbg_color = \"black\"\ntime_format",
            "text_color = \"black\"\nbg_color = \"black\"\ntime_format",
            1,
        );
        assert_ne!(
            variant_a, variant_b,
            "the swap anchor must have matched something"
        );
        assert_eq!(
            variant_a.len(),
            variant_b.len(),
            "the two variants must be the same byte length for this test to mean anything"
        );

        let parsed_a = crate::theme::parse(variant_a, "default")
            .expect("variant A (unmodified default theme) must parse");
        assert_eq!(
            parsed_a.segments.clock.text_color, "white",
            "variant A must carry the original text_color"
        );
        let parsed_b = crate::theme::parse(&variant_b, "default")
            .expect("variant B (swapped clock text_color) must parse");
        assert_eq!(
            parsed_b.segments.clock.text_color, "black",
            "variant B must carry the swapped text_color"
        );

        let temp = tempfile::TempDir::new().expect("tempdir");
        let theme_path = temp.path().join("theme.toml");
        std::fs::write(&theme_path, variant_a).expect("write initial theme");

        let manager = ThemeManager::with_theme_and_path("default", parsed_a, theme_path.clone());
        assert_eq!(
            manager.get().segments.clock.text_color,
            "white",
            "clock text_color should be white initially"
        );

        manager.spawn_poll_thread(None, Duration::from_millis(10), Duration::from_millis(10));
        // Written right after spawn to model the TOCTOU race.
        std::fs::write(&theme_path, &variant_b).expect("write updated theme");

        let mut reloaded = false;
        for _ in 0_i32..300_i32 {
            std::thread::sleep(Duration::from_millis(20));
            if manager.get().segments.clock.text_color == "black" {
                reloaded = true;
                break;
            }
        }

        assert!(
            reloaded,
            "poll fallback must detect a same-length rewrite written immediately after spawn"
        );

        manager.stop_watching();
    }

    #[test]
    fn theme_reload_runs_registered_callback_after_swap() {
        let variant_a = DEFAULT_THEME_CONTENT;
        let variant_b = variant_a.replacen(
            "text_color = \"white\"\nbg_color = \"black\"\ntime_format",
            "text_color = \"black\"\nbg_color = \"black\"\ntime_format",
            1,
        );
        assert_ne!(variant_a, variant_b, "the swap anchor must have matched");
        let parsed_a = crate::theme::parse(variant_a, "default").expect("variant A must parse");

        let temp = tempfile::TempDir::new().expect("tempdir");
        let theme_path = temp.path().join("theme.toml");
        std::fs::write(&theme_path, variant_a).expect("write initial theme");
        let manager = Arc::new(ThemeManager::with_theme_and_path(
            "default",
            parsed_a,
            theme_path.clone(),
        ));

        let observed: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let observed_in_cb = Arc::clone(&observed);
        let manager_in_cb = Arc::clone(&manager);
        manager.set_reload_callback(Arc::new(move || {
            // Reads the manager: would deadlock if called under the state lock.
            let text_color = manager_in_cb.get().segments.clock.text_color.clone();
            observed_in_cb
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(text_color.to_string());
        }));

        manager.spawn_poll_thread(None, Duration::from_millis(10), Duration::from_millis(10));
        std::fs::write(&theme_path, &variant_b).expect("write updated theme");

        let mut seen_black = false;
        for _ in 0_i32..300_i32 {
            std::thread::sleep(Duration::from_millis(20));
            if observed
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .iter()
                .any(|color| color == "black")
            {
                seen_black = true;
                break;
            }
        }
        manager.stop_watching();
        assert!(
            seen_black,
            "reload callback must run after the swap and observe the new theme"
        );
    }
}

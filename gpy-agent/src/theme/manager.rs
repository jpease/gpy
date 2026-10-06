//! Theme manager with hot-reload support

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
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// Builtin themes: name → embedded TOML content.
///
/// Adding a new builtin theme requires one entry here; `builtin`,
/// `config::discovery::insert_builtins`, and `config::loader::load_theme_from_path`'s
/// embedded-content fallback all iterate this table automatically.
pub(crate) const BUILTIN_THEMES: &[(&str, &str)] = &[
    ("default", DEFAULT_THEME_CONTENT),
    ("text", TEXT_THEME_CONTENT),
    ("starship", STARSHIP_THEME_CONTENT),
];

/// A theme's name, the on-disk path it was loaded from, and its parsed
/// content, updated together as one atomic unit (#588).
///
/// Previously these were three independent `RwLock`s: `switch_theme` wrote
/// them in three separate critical sections, so a reader between any two of
/// those writes (`export` is the concrete case this matters for) could
/// observe a torn pairing — new theme content under the old name, or vice
/// versa. Bundling them means every read sees a state that was true at some
/// single instant, and `switch_theme` becomes one pointer swap instead of
/// three sequential writes.
///
/// `theme` is itself an `Arc` so [`ThemeManager::get`] hands out a
/// reference-count bump rather than deep-cloning `ThemeConfig` on every
/// call, matching `ConfigManager`'s `Arc<RwLock<Arc<Config>>>`.
#[derive(Debug, Clone)]
struct ThemeState {
    name: String,
    path: PathBuf,
    theme: Arc<ThemeConfig>,
}

/// Callback run after a successful hot-reload swap (see [`ThemeManager::set_reload_callback`]).
pub type ThemeReloadCallback = Arc<dyn Fn() + Send + Sync>;
type ReloadHookSlot = Arc<Mutex<Option<ThemeReloadCallback>>>;

/// Manages theme loading, caching, and hot-reloading
pub struct ThemeManager {
    /// Name, path, and content of the active theme, swapped as one unit.
    state: Arc<RwLock<Arc<ThemeState>>>,
    /// File watcher plus poll fallback for hot-reload, shared with the other
    /// file-watching managers (#588).
    hot_reload: HotReloadSlot,
    /// Post-reload hook. When set it replaces the direct `notify_reload` so the
    /// owner can refresh derived caches before ringing the doorbell (#710).
    on_reload: ReloadHookSlot,
    client_registry: Arc<Mutex<Option<Arc<crate::ipc::ClientDirectory>>>>,
    watch_debounce: Arc<Mutex<Duration>>,
    /// The `WatchRegistry` the watcher was last armed with, so
    /// [`switch_theme`](Self::switch_theme) can re-arm on the same registry
    /// instead of falling back to a private one (#663).
    watch_registry: Arc<Mutex<Option<Arc<WatchRegistry>>>>,
}

/// Source metadata for a discovered theme.
///
/// Themes and palettes shared one enum shape and one precedence ordering, so
/// both names now refer to the single [`crate::config::discovery::Source`]
/// (#588).
pub type ThemeSource = Source;

/// A theme entry exposed to CLI/runtime discovery.
///
/// See [`ThemeSource`] on why this is shared with palette discovery.
pub type DiscoveredTheme = Discovered;

impl ThemeManager {
    /// Create a new theme manager with the specified theme name
    ///
    /// # Errors
    ///
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

    /// Construct a manager for a builtin theme from embedded content, bypassing
    /// any user or plugin theme overrides on disk.
    ///
    /// Unlike [`new`](Self::new), this never consults `~/.config` or discovered
    /// plugin themes: it always parses the embedded builtin content for
    /// `default`, `text`, or `starship`. Use it for tests and tooling that need
    /// a deterministic theme regardless of the host's configuration.
    ///
    /// The recorded theme path points at the conventional user location for the
    /// theme so subsequent [`reload`](Self::reload)/watch behavior matches
    /// [`new`](Self::new); the initially loaded content, however, is always the
    /// embedded builtin.
    ///
    /// # Errors
    ///
    /// Returns an error if `theme_name` is not a known builtin theme, or if the
    /// embedded content fails to parse or validate.
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

    /// A consistent snapshot of name, path, and theme content.
    ///
    /// Every reader goes through here so it observes one state that was true
    /// at a single instant, rather than reading fields under separate locks
    /// that a concurrent [`switch_theme`](Self::switch_theme) could interleave
    /// between (#588).
    ///
    /// Recovers from a poisoned lock by using the last-written value rather
    /// than panicking the render path (a panicked writer should not also take
    /// down every subsequent read), matching `ConfigManager::get`.
    fn state(&self) -> Arc<ThemeState> {
        let guard = self.state.read().unwrap_or_else(|poisoned| {
            debug_log!(
                "theme",
                "CRITICAL: Theme lock poisoned, recovering with poisoned data"
            );
            poisoned.into_inner()
        });
        Arc::clone(&guard)
    }

    /// Get the current theme configuration
    ///
    /// Returns an `Arc<ThemeConfig>` which is a cheap clone (just a reference
    /// count bump) rather than a deep clone of the whole theme.
    ///
    /// Recovers from a poisoned lock by using the last-written value rather
    /// than panicking the render path (a panicked writer should not also take
    /// down every subsequent read).
    #[must_use]
    pub fn get(&self) -> Arc<ThemeConfig> {
        Arc::clone(&self.state().theme)
    }

    /// Register the post-reload hook for hot-reloads of the theme file.
    ///
    /// Mirrors `ConfigManager::set_reload_callback`. When set, the hook runs
    /// after the new theme is swapped in and **replaces** the direct
    /// `notify_reload` doorbell: the hook owns ringing it, so it can write the
    /// theme-export and instant caches first (#710). Name switches via
    /// [`switch_theme`](Self::switch_theme) do not run it.
    pub fn set_reload_callback(&self, callback: ThemeReloadCallback) {
        if let Ok(mut slot) = self.on_reload.lock() {
            *slot = Some(callback);
        }
    }

    /// Reload the current theme from disk
    ///
    /// The name and path are unchanged by a reload; only the content is
    /// replaced, but the whole state is still swapped as one unit so no reader
    /// can observe a half-applied update.
    ///
    /// # Errors
    ///
    /// Returns an error if the theme file cannot be read or parsed, or if the
    /// theme state lock is poisoned.
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

    /// Switch to a different theme
    ///
    /// The new name, path, and content are assembled off-lock and installed in
    /// a single write, so a concurrent [`export`](Self::export) sees either the
    /// whole old theme or the whole new one, never a mix (#588).
    ///
    /// # Errors
    ///
    /// Returns an error if the new theme file cannot be found or parsed, or if
    /// one of the manager's locks is poisoned.
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
        // Re-arm on the registry the manager was started with, if any,
        // instead of `start_watching`'s private default — otherwise every
        // switch after the first drops the agent-wide registry the other
        // coordinators share (#617, #663).
        let watch_registry = self
            .watch_registry
            .lock()
            .map_err(|e| Error::config(e.to_string()))?
            .clone()
            .unwrap_or_else(|| Arc::new(WatchRegistry::new()));
        if let Err(error) =
            self.start_watching_with(debounce_duration, client_registry, &watch_registry)
        {
            // Non-fatal: the theme itself switched successfully, but hot-reload
            // for the new theme file is now off. Previously discarded with
            // `let _ =`, leaving no signal at all (#588).
            debug_log!(
                "theme",
                "Failed to restart theme watching after switching to '{new_theme_name}': {error}"
            );
        }
        Ok(())
    }

    /// Start watching the theme file for changes
    ///
    /// # Errors
    ///
    /// Returns an error if the file watcher cannot be initialized, or if the
    /// client-registry or debounce lock is poisoned.
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
    /// The agent passes the one registry it also gives its repo and config
    /// coordinators, so every coordinator classifies events against the same
    /// registered config paths (#617).
    ///
    /// # Errors
    ///
    /// Returns an error if the file watcher cannot be initialized, or if the
    /// client-registry or debounce lock is poisoned.
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
                let Ok(theme_path) = state_arc.read().map(|guard| guard.path.clone()) else {
                    return;
                };
                Self::reload_and_apply(
                    &state_arc,
                    callback_registry.as_deref(),
                    &on_reload,
                    &theme_path,
                    "watch callback",
                );
            });

            // This coordinator never registers a repository itself; with a
            // private registry its root set is empty and attribution uses the
            // ancestor `.git` stat-walk fallback, and with the agent's shared
            // registry it attributes against the repo watcher's roots (see
            // `WatchCoordinator::new` docs, #344, #617).
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
                // Some notify backends are more reliable when both directory and concrete
                // file watches are active, especially for truncate/write/save workflows.
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

    /// Stop watching the theme file
    pub fn stop_watching(&self) {
        self.hot_reload.stop();
    }

    /// Test-only: the `WatchRegistry` currently backing the watcher, if any.
    ///
    /// Lets a test confirm [`switch_theme`](Self::switch_theme) re-armed on
    /// the registry the manager was started with instead of a private
    /// default (#663).
    #[cfg(feature = "test-support")]
    #[must_use]
    pub fn watch_registry_for_test(&self) -> Option<Arc<WatchRegistry>> {
        self.watch_registry
            .lock()
            .ok()
            .and_then(|slot| slot.clone())
    }

    /// Export theme for specified shell
    ///
    /// This is the unified method for theme export. Use this instead of
    /// shell-specific methods.
    ///
    /// Recovers from a poisoned lock instead of panicking, matching [`get`](Self::get).
    ///
    /// Name and content come from one [`state`](Self::state) snapshot, so a
    /// concurrent [`switch_theme`](Self::switch_theme) can never pair the new
    /// theme's content with the old theme's name (#588).
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

    /// List all available themes
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

    /// Get the default theme template
    #[must_use]
    pub const fn default_theme_template() -> &'static str {
        DEFAULT_THEME_CONTENT
    }

    /// Resolve the raw TOML source of a discovered theme by name, for use as a
    /// clone base (e.g. `gpy theme new <name> --from <base>`).
    ///
    /// Returns the embedded content for builtin themes and the on-disk file
    /// contents for user- or plugin-provided themes, verbatim (comments and
    /// formatting are preserved rather than round-tripped through the parsed
    /// model).
    ///
    /// # Errors
    ///
    /// Returns an error if `theme_name` is not a discoverable theme, or if a
    /// file-backed theme's contents cannot be read.
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

    /// Get the directory where user themes are stored
    #[must_use]
    pub fn user_themes_dir() -> PathBuf {
        crate::paths::config_root_for(
            crate::paths::root_var("XDG_CONFIG_HOME").as_deref(),
            crate::paths::home_dir().as_deref(),
        )
        .join("themes")
    }

    fn user_theme_path(theme_name: &str) -> PathBuf {
        Self::user_themes_dir().join(format!("{theme_name}.toml"))
    }

    /// Persist `theme` as a user-writable override at
    /// `user_themes_dir()/{theme_name}.toml`, materializing the themes
    /// directory if needed, and return the path written.
    ///
    /// This is the config wizard's per-field theme-edit persistence primitive
    /// (#407): the caller loads the currently-active theme, mutates only the
    /// field(s) a picker edited, and hands the whole `ThemeConfig` here to be
    /// written back. Every unrelated field is carried through by value, so on
    /// reload nothing but the edited field(s) differs — the same guarantee
    /// (that only what was edited changes) `config::loader::save_config`
    /// gives for `config.toml`.
    ///
    /// It only ever writes under the user themes directory: a builtin
    /// (embedded `config/themes/*.toml`) or plugin theme file on disk is never
    /// touched. The user copy simply shadows the builtin/plugin source at
    /// higher [`ThemeSource`] precedence, and deleting it restores the
    /// original.
    ///
    /// # Errors
    ///
    /// Returns an error if `theme_name` is not a safe config name, if the theme
    /// cannot be serialized to TOML, if the themes directory cannot be created,
    /// or if the file cannot be written.
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
    ///
    /// Returns an error if the file cannot be read or the TOML cannot be parsed.
    fn load_from_path(path: &Path) -> Result<ThemeConfig> {
        crate::config::loader::load_theme_from_path(&path.display().to_string())
    }

    /// Resolve `theme_name` to the theme file this crate would load for it, or
    /// `None` when the name matches no user theme, no plugin theme, and no
    /// builtin (#572).
    ///
    /// The single source of truth for theme-name resolution, used by both the
    /// daemon (`Self::theme_path`, below) and CLI oneshot rendering
    /// (`config::loader::load_theme_uncached`). Before this, oneshot only ever
    /// checked the user themes directory, so it could not resolve a
    /// plugin-provided theme the daemon renders fine.
    ///
    /// A builtin name resolves to its user path even though nothing exists
    /// there on disk: `load_theme_from_path`'s embedded-content fallback
    /// matches builtins by file stem regardless of whether the path is real, so
    /// this only needs to confirm the NAME is a known builtin, not that a file
    /// backs it.
    pub(crate) fn resolve_path(theme_name: &str) -> Option<PathBuf> {
        let user_path = Self::user_theme_path(theme_name);
        if user_path.exists() {
            return Some(user_path);
        }

        // Fall back to discovered themes (e.g., plugin-provided themes).
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
        // Preserve historical behavior for non-existent custom themes: always
        // return a path (falling back to the nonexistent user path), leaving
        // the actual "unresolvable" error to `load_theme_from_path` one layer
        // down (#572).
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

    /// Reload the theme at `theme_path` and apply it, or log why the reload
    /// failed. When a reload hook is registered ([`Self::set_reload_callback`]) it runs
    /// in place of the direct doorbell; otherwise asks clients to reload (`notify_reload`). Shared by
    /// the file-watcher callback (`start_watching`) and the polling fallback
    /// (`start_poll_fallback`) so a load failure is never silently discarded.
    /// The name and path are carried over from the state being replaced (a
    /// hot-reload only ever re-reads the theme's own current file), but the
    /// whole state is still rebuilt and swapped in one write so no reader can
    /// observe a partially-updated theme (#588).
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
                let Ok(mut guard) = state_lock.write() else {
                    // Previously an `if let Ok(..)` with no `else`: a poisoned
                    // lock silently discarded a freshly-loaded theme with no
                    // diagnostic at all (#588).
                    debug_log!(
                        "theme",
                        "Theme state lock poisoned; discarding theme reloaded from {path} ({context})"
                    );
                    return;
                };
                let current = Arc::clone(&guard);
                *guard = Arc::new(ThemeState {
                    name: current.name.clone(),
                    path: current.path.clone(),
                    theme: Arc::new(new_theme),
                });
                drop(guard);

                // Never invoked under the state lock (dropped above): the hook
                // reads `theme_manager.get()`. The slot lock is released before
                // the call as well so a hook may re-register itself.
                let registered = on_reload.lock().ok().and_then(|slot| slot.clone());
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

    /// What the shared [`HotReloadSlot`] needs from this manager: which file to
    /// poll, and what to do when its contents change.
    ///
    /// The path is read from the state lock on every tick rather than captured
    /// once, because `switch_theme` can repoint it while the poll thread runs;
    /// a poisoned lock yields `None`, which retires the thread.
    fn poll_target(&self, client_registry: Option<Arc<crate::ipc::ClientDirectory>>) -> PollTarget {
        let path_state = Arc::clone(&self.state);
        let reload_state = Arc::clone(&self.state);
        let on_reload = Arc::clone(&self.on_reload);

        PollTarget {
            path_of: Box::new(move || path_state.read().ok().map(|state| state.path.clone())),
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

    /// Spawn the poll-fallback thread with an already-resolved poll
    /// interval. No-op if a poll thread is already running. Split out from
    /// `start_poll_fallback` so tests can exercise the polling behavior
    /// directly without mutating `GPY_THEME_WATCH_POLL_MS` (this crate
    /// forbids `unsafe_code`, and `std::env::set_var` requires it).
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
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// #572: an unresolvable theme name must error, naming the theme, instead
    /// of silently constructing a blank `ThemeConfig::default()`.
    ///
    /// This is the daemon-side half of the fix: `ThemeManager::new` already
    /// propagates `load_from_path`'s (-> `config::loader::load_theme_from_path`)
    /// result via `?`, so this test exercises that whole chain, not just the
    /// resolver.
    ///
    /// # Panics
    ///
    /// Panics if an unresolvable theme name is silently accepted, or if the
    /// error doesn't name the theme.
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

    /// #572: `resolve_path` is the single resolver both the daemon
    /// (`theme_path`) and CLI oneshot (`config::loader::load_theme_uncached`)
    /// now share.
    ///
    /// It replaces oneshot's old user-directory-only, non-plugin-aware
    /// resolver.
    ///
    /// A real end-to-end proof would need a plugin discovered on disk via
    /// `discover_plugins()`, which is driven entirely by environment
    /// variables (`GPY_BUNDLED_PLUGIN_DIR`, `XDG_CONFIG_HOME`, `HOME`) this
    /// crate's `#![forbid(unsafe_code)]` makes impossible to fabricate
    /// in-process (`std::env::set_var` is `unsafe` under edition 2024; no
    /// other test in this crate mutates process env for exactly that reason
    /// -- see `paths.rs`'s pure, parameterized resolvers for the established
    /// pattern). So this instead proves the wiring `resolve_path` and
    /// `theme_path` both depend on: merging a plugin-sourced `DiscoveredTheme`
    /// into the discovery map preserves its `path`, which is exactly the
    /// `.find(|theme| theme.name == theme_name).and_then(|theme| theme.path)`
    /// expression both functions run over `discover_available_themes()`'s
    /// output.
    ///
    /// # Panics
    ///
    /// Panics if a plugin-sourced theme candidate's path isn't retained by
    /// the same merge/lookup expression `resolve_path` uses.
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

        // The exact expression `resolve_path`/`theme_path` run over
        // `discover_available_themes()`'s output.
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

        // The starship preset sets a `format`, so the presence flag is "1" — and
        // the raw format string must NEVER be exported (injection/`set -u` safety).
        assert!(
            output.contains("set -g __username_format \"1\""),
            "starship preset sets a username format -> presence flag \"1\". Got:\n{output}"
        );
        assert!(
            !output.contains("$username"),
            "the raw username format string must never be exported. Got:\n{output}"
        );
    }

    /// Proves adding a builtin theme requires editing exactly one place:
    /// `BUILTIN_THEMES`. Every name listed there must be independently:
    /// - loadable via `ThemeManager::builtin`
    /// - accepted by `load_theme_from_path`'s embedded-content fallback
    ///   (config/loader.rs's builtin path-suffix matching)
    /// - discoverable via `config::discovery::insert_builtins`
    ///
    /// `discovery::insert_builtins` is exercised directly on a fresh map rather than
    /// through `discover_available_themes()`, which also merges real
    /// `~/.config/gpy/themes` user files; a developer machine with its own
    /// theme files installed there would make an end-to-end discovery
    /// assertion environment-dependent (a real user "default.toml" shadows
    /// the builtin with higher precedence).
    ///
    /// Against the pre-refactor code (three independent hardcoded lists), this
    /// test could not even be written against a single source of truth; it only
    /// becomes meaningful once `BUILTIN_THEMES` exists.
    #[test]
    fn all_builtins_load_from_every_site() {
        assert!(
            !BUILTIN_THEMES.is_empty(),
            "BUILTIN_THEMES must list at least one builtin theme"
        );

        let mut discovered = std::collections::BTreeMap::new();
        crate::config::discovery::insert_builtins(&mut discovered, BUILTIN_THEMES);

        for (name, _) in BUILTIN_THEMES {
            // Site 1: `ThemeManager::builtin`.
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

            // Site 3: `config::discovery::insert_builtins`.
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
    #[allow(clippy::missing_panics_doc)]
    fn poll_fallback_detects_a_toctou_same_length_rewrite() {
        // Covers both acceptance-criteria bullets for #567 in one test: the
        // immediate post-spawn write models the TOCTOU race (#385's fix
        // ported here), and using same-byte-length content models #529's
        // timestamp-granularity blind spot.
        //
        // `[segments.clock]` in the default theme's `text_color` ("white",
        // 5 bytes) is swapped for "black" (also 5 bytes): total file length
        // is unchanged, but the reloaded `ThemeConfig` differs in a field
        // reachable off `manager.get()`.
        //
        // The anchor runs through `time_format` rather than starting at the
        // `[segments.clock]` header: the header and its `text_color` are no
        // longer adjacent lines now that the section carries a comment
        // explaining its `format` template. `time_format` keeps the anchor
        // clock-specific — `[segments.directory]` has the same
        // text_color/bg_color pair but no time settings.
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

        // Both variants must actually be loadable themes with the expected
        // (differing) field value, or the test proves nothing.
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
        // Written immediately after the spawn call returns, modeling the
        // TOCTOU race: a real scheduler gap between this write and the poll
        // thread's first tick would, under the old in-thread baseline
        // capture, be silently absorbed into the "initial" state.
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
    #[allow(clippy::missing_panics_doc)]
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
            if let Ok(mut seen) = observed_in_cb.lock() {
                seen.push(text_color.to_string());
            }
        }));

        manager.spawn_poll_thread(None, Duration::from_millis(10), Duration::from_millis(10));
        std::fs::write(&theme_path, &variant_b).expect("write updated theme");

        let mut seen_black = false;
        for _ in 0_i32..300_i32 {
            std::thread::sleep(Duration::from_millis(20));
            if observed
                .lock()
                .is_ok_and(|seen| seen.iter().any(|color| color == "black"))
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

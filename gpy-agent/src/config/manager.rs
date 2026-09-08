//! Configuration management with hot-reload support
//!
//! This module provides `ConfigManager` which centralizes configuration loading
//! and provides automatic file watching with hot-reload capabilities.
//!
//! # Architecture
//!
//! Similar to `ThemeManager`, `ConfigManager` uses:
//! - `Arc<RwLock<Arc<Config>>>` for thread-safe shared access to configuration:
//!   the inner `Arc<Config>` makes `get()` a cheap reference-count bump instead
//!   of a full deep clone of `Config` on every call
//! - `watcher::hot_reload::HotReloadSlot` for file watching plus the polling
//!   fallback — the same mechanism `ThemeManager` uses (#588)
//! - 5-second debounce to handle editor save patterns
//!
//! # Hot-Reload Behavior
//!
//! When the config file changes:
//! - The file is reparsed and validated
//! - If valid, the new config atomically replaces the old one
//! - If invalid, the old config is kept and a warning is logged
//! - All code using `config_manager.get()` immediately sees the new config
//!
//! # Cross-Dependencies
//!
//! **Live Theme Switching**: When `ui.theme` changes in the config file, the change is
//! automatically detected and the theme switches without requiring a restart. The callback
//! registered in `Agent::new()` calls `ThemeManager::switch_theme()`, which atomically
//! updates the theme state and restarts the file watcher for the new theme.
//!
//! This works because `ThemeManager` keeps its name, path, and parsed theme in one
//! `Arc<RwLock<Arc<ThemeState>>>`, so a switch installs all three in a single write and
//! a concurrent reader never sees a half-applied one (#588).

use crate::config::schema::get_config_paths;
use crate::config::{Config, loader::load_config_from_file};
use crate::watcher::hot_reload::{HotReloadSlot, PollTarget};
use crate::watcher::{WatchCoordinator, WatchRegistry};
use crate::{Error, Result, debug_log};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// Callback invoked before a config reload is committed.
///
/// Returning an error aborts the reload and keeps the previous config active.
pub type ConfigReloadCallback = Arc<dyn Fn(&Config, &Config) -> Result<()> + Send + Sync>;

/// Manages configuration with hot-reload support
///
/// Provides thread-safe access to configuration and optional file watching
/// for automatic reloading when the config file changes.
pub struct ConfigManager {
    /// Path to the config file being watched
    config_path: PathBuf,
    /// The loaded configuration (thread-safe shared access). The inner `Arc`
    /// lets `get()` hand out a cheap clone instead of deep-cloning `Config`.
    config: Arc<RwLock<Arc<Config>>>,
    /// File watcher plus poll fallback for hot-reload, shared with the other
    /// file-watching managers (#588).
    hot_reload: HotReloadSlot,
    /// Optional callback invoked after successful config reload (`old_config`, `new_config`)
    /// Wrapped in Mutex to allow setting callback through shared reference
    on_reload: Arc<Mutex<Option<ConfigReloadCallback>>>,
    /// The registry the running watcher registered `config_path` on, so
    /// [`ConfigManager::stop_watching`] unregisters from the same one it
    /// registered against (#617). `None` until a watcher is started.
    watch_registry: Mutex<Option<Arc<WatchRegistry>>>,
    /// Serializes every reload -- the explicit [`reload_now`](Self::reload_now)
    /// and the watcher- and poll-triggered [`reload_and_apply`](Self::reload_and_apply)
    /// -- across its read-old, run-callback, store-new sequence.
    ///
    /// Without it, the config watcher's own reload (the CLI writes the file
    /// and then asks for a reload over IPC, so both fire) could read the
    /// pre-request config while the requested reload was still inside its
    /// callback, and run the callback a second time against the same stale
    /// old config: two theme switches, two cache rewrites, two repaints for
    /// one change (#661). Held as `()` so a callback panic can be recovered
    /// from rather than wedging every later reload.
    reload_gate: Arc<Mutex<()>>,
}

impl ConfigManager {
    /// Create a new `ConfigManager` with default configuration
    ///
    /// This uses `Config::default()` without attempting to read any files.
    /// Useful as a fallback when config file parsing fails.
    ///
    /// # Errors
    ///
    /// Returns an error if the default config path cannot be determined.
    pub fn with_defaults() -> Result<Self> {
        let config_paths = get_config_paths();
        let default_config = Config::default();
        let default_path = config_paths
            .first()
            .ok_or_else(|| Error::config("No config paths available".to_owned()))?;

        Ok(Self::from_parts(
            PathBuf::from(default_path),
            default_config,
        ))
    }

    /// Create a new `ConfigManager` for a specific config file path.
    ///
    /// # Errors
    ///
    /// Returns an error if the provided path cannot be converted to UTF-8 or the
    /// configuration file exists but fails to parse.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let config = if path_buf.exists() {
            let path_str = path_buf
                .to_str()
                .ok_or_else(|| Error::config("Config path contains invalid UTF-8".to_owned()))?;
            load_config_from_file(path_str)?
        } else {
            Config::default()
        };

        Ok(Self::from_parts(path_buf, config))
    }

    /// Create a new `ConfigManager` by loading the default config file
    ///
    /// This searches for config files in the standard locations:
    /// - `$XDG_CONFIG_HOME/gpy/config.toml` (or `~/.config/gpy/config.toml`)
    ///
    /// If no config file exists, uses `Config::default()`.
    ///
    /// # Errors
    ///
    /// Returns an error if a config file exists but cannot be parsed or is invalid.
    pub fn new() -> Result<Self> {
        let config_paths = get_config_paths();

        // Find first existing config file
        let existing_path = config_paths
            .iter()
            .find(|path| Path::new(path).exists())
            .map(PathBuf::from);

        let (config, resolved_path) = if let Some(path) = existing_path {
            let loaded_config =
                load_config_from_file(path.to_str().ok_or_else(|| {
                    Error::config("Config path contains invalid UTF-8".to_owned())
                })?)?;
            (loaded_config, path)
        } else {
            // No config file found, use defaults
            let default_config = Config::default();
            // Use the first path as the "default" location (even though it doesn't exist yet)
            let default_path = config_paths
                .first()
                .ok_or_else(|| Error::config("No config paths available".to_owned()))?;
            (default_config, PathBuf::from(default_path))
        };

        Ok(Self::from_parts(resolved_path, config))
    }

    /// Build a `ConfigManager` from an already-resolved path and config,
    /// wrapping `config` in the shared `Arc<RwLock<Arc<Config>>>` slot and
    /// initializing hot-reload state fresh (no watcher started, no reload
    /// callback registered).
    fn from_parts(config_path: PathBuf, config: Config) -> Self {
        Self {
            config_path,
            config: Arc::new(RwLock::new(Arc::new(config))),
            hot_reload: HotReloadSlot::new(),
            on_reload: Arc::new(Mutex::new(None)),
            watch_registry: Mutex::new(None),
            reload_gate: Arc::new(Mutex::new(())),
        }
    }

    /// Set a callback to be invoked after successful config reload
    ///
    /// The callback receives references to the old and new configs,
    /// allowing detection of specific changes (e.g., theme name changes).
    ///
    /// This method uses interior mutability and can be called on a shared reference.
    pub fn set_reload_callback(&self, callback: ConfigReloadCallback) {
        if let Ok(mut guard) = self.on_reload.lock() {
            *guard = Some(callback);
        }
    }

    #[cfg(test)]
    ///
    /// # Panics
    ///
    /// Panics when the registered reload callback returns an error.
    /// Test helper: invoke the reload callback with synthetic config values.
    pub fn trigger_reload_for_tests(&self, old_config: &Config, new_config: &Config) {
        if let Ok(guard) = self.on_reload.lock()
            && let Some(callback) = guard.as_ref()
        {
            callback(old_config, new_config).expect("reload callback should succeed");
        }
    }

    #[cfg(test)]
    /// Test helper: overwrite the in-memory configuration without hitting the filesystem.
    pub fn overwrite_for_tests(&self, new_config: Config) {
        if let Ok(mut guard) = self.config.write() {
            *guard = Arc::new(new_config);
        }
    }

    /// Get a clone of the current configuration
    ///
    /// Returns an `Arc<Config>` which is a cheap clone (just increments a reference count).
    /// The config is immutable, so this is thread-safe.
    ///
    /// Recovers from a poisoned lock by using the last-written value rather than
    /// panicking every subsequent caller on the event loop (a panicked writer
    /// should not also take down every reader).
    #[must_use]
    pub fn get(&self) -> Arc<Config> {
        let guard = self.config.read().unwrap_or_else(|poisoned| {
            debug_log!(
                "config",
                "CRITICAL: Config lock poisoned, recovering with poisoned data"
            );
            poisoned.into_inner()
        });
        Arc::clone(&guard)
    }

    /// Get the path to the config file being managed
    #[must_use]
    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    /// Start watching the config file for changes with automatic reload
    ///
    /// When the file changes, it will be automatically reloaded after the debounce period.
    /// If the reload fails (invalid config), the old config is kept and a warning is logged.
    ///
    /// # Errors
    ///
    /// Returns an error if the file watcher cannot be started.
    ///
    /// # Panics
    ///
    /// May panic if:
    /// - The watcher lock is poisoned (should never happen)
    /// - The config path contains invalid UTF-8
    /// - The config directory cannot be created
    pub fn start_watching(&self, debounce_duration: Duration) -> Result<()> {
        self.start_watching_with(debounce_duration, &Arc::new(WatchRegistry::new()))
    }

    /// Start watching the config file, sharing `registry` with the caller.
    ///
    /// `classify_event` consults the registry's config-path set for EVERY
    /// event on EVERY coordinator that shares it, so the agent hands its one
    /// registry to this watcher and to its repo watcher alike: that is what
    /// keeps a custom-named config file classified as a config change on the
    /// repo watcher too (#617).
    ///
    /// # Errors
    ///
    /// Returns an error if the file watcher cannot be started.
    ///
    /// # Panics
    ///
    /// May panic if:
    /// - The watcher lock is poisoned (should never happen)
    /// - The config path contains invalid UTF-8
    /// - The config directory cannot be created
    pub fn start_watching_with(
        &self,
        debounce_duration: Duration,
        registry: &Arc<WatchRegistry>,
    ) -> Result<()> {
        let started = self.hot_reload.start_watcher(|| {
            // Create callback that reloads config when file changes
            // Extract callback from Mutex for the watcher
            #[expect(
                clippy::option_if_let_else,
                reason = "matching on the lock Result, not an Option; map_or_else would need a closure repeating the same guard.clone() vs None branches with no clarity gain"
            )]
            let on_reload_option = if let Ok(guard) = self.on_reload.lock() {
                guard.clone()
            } else {
                None
            };

            let reload_callback = Self::create_reload_callback(
                Arc::clone(&self.config),
                Arc::clone(&self.reload_gate),
                self.config_path.clone(),
                on_reload_option,
            );

            // Start watching with debounce. This coordinator never registers a
            // repository itself; with a private registry its root set is empty
            // and attribution uses the ancestor `.git` stat-walk fallback, and
            // with the agent's shared registry it attributes against the repo
            // watcher's roots (see `WatchCoordinator::new` docs, #344, #617).
            let mut coordinator = WatchCoordinator::new(
                debounce_duration,
                reload_callback,
                Some(Arc::clone(registry)),
            )?;

            // Watch the parent directory (not just the file, since editors may delete/rename)
            if let Some(parent) = self.config_path.parent() {
                Self::ensure_config_directory_exists(parent)?;
                coordinator.watch_directory(parent)?;
            }
            registry.register_config_path(&self.config_path);
            if let Ok(mut slot) = self.watch_registry.lock() {
                *slot = Some(Arc::clone(registry));
            }
            Ok(coordinator)
        })?;

        if !started {
            debug_log!(
                "config",
                "ConfigManager already watching, ignoring start_watching call"
            );
            return Ok(());
        }

        debug_log!(
            "config",
            "Started watching config file: {:?}",
            self.config_path
        );

        self.start_poll_fallback(debounce_duration);

        Ok(())
    }

    /// Load, validate (via `load_config_from_file`'s own validation — no
    /// second `validate_config` call here), and run `on_reload` against `old`.
    /// Pure of any shared state: the caller decides what to do with the
    /// resulting `Config` (write it into the shared slot) and how to report
    /// failure (log-and-keep-old vs. propagate).
    ///
    /// # Errors
    ///
    /// Returns an error if the file cannot be read, parsed, or fails
    /// validation, or if `on_reload` rejects the change.
    fn try_reload(
        path: &Path,
        on_reload: Option<&ConfigReloadCallback>,
        old: &Config,
    ) -> Result<Config> {
        let path_str = path
            .to_str()
            .ok_or_else(|| Error::config("Config path contains invalid UTF-8".to_owned()))?;

        let new_config = load_config_from_file(path_str)?;

        if let Some(callback) = on_reload {
            callback(old, &new_config)?;
        }

        Ok(new_config)
    }

    /// Reload the config at `config_path` and apply it if valid, or log why
    /// the reload was skipped. Shared by the file-watcher callback
    /// (`create_reload_callback`) and the polling fallback
    /// (`spawn_poll_thread`) so both triggers apply the identical reload
    /// logic.
    fn reload_and_apply(
        config: &Arc<RwLock<Arc<Config>>>,
        reload_gate: &Mutex<()>,
        config_path: &Path,
        on_reload: Option<&ConfigReloadCallback>,
    ) {
        debug_log!("config", "Config file changed, reloading...");

        let _serialized = Self::enter_reload(reload_gate);
        let old_config = if let Ok(guard) = config.read() {
            guard.clone()
        } else {
            debug_log!("config", "Failed to acquire config read lock");
            return;
        };

        match Self::try_reload(config_path, on_reload, &old_config) {
            Ok(new_config) => {
                if let Ok(mut config_state) = config.write() {
                    *config_state = Arc::new(new_config);
                    debug_log!("config", "Config reloaded successfully");
                } else {
                    debug_log!("config", "Failed to acquire config write lock");
                }
            }
            Err(e) => {
                debug_log!(
                    "config",
                    "Failed to reload config, keeping old config: {}",
                    e
                );
            }
        }
    }

    /// Create the callback closure for config reloading, invoked by the
    /// file watcher. Delegates to `reload_and_apply`.
    fn create_reload_callback(
        config: Arc<RwLock<Arc<Config>>>,
        reload_gate: Arc<Mutex<()>>,
        config_path: PathBuf,
        on_reload: Option<ConfigReloadCallback>,
    ) -> Box<dyn Fn(crate::watcher::DebouncedEvent) + Send + Sync + 'static> {
        Box::new(move |_event| {
            Self::reload_and_apply(&config, &reload_gate, &config_path, on_reload.as_ref());
        })
    }

    /// Take the reload gate, recovering from a poisoned lock: a callback that
    /// panicked mid-reload must not turn every later reload into a no-op.
    fn enter_reload(reload_gate: &Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
        reload_gate
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    /// What the shared [`HotReloadSlot`] needs from this manager: which file to
    /// poll, and what to do when its contents change. Both halves capture only
    /// owned clones, so the poll thread outlives this borrow.
    fn poll_target(&self) -> PollTarget {
        let path_for_poll = self.config_path.clone();
        let config_arc = Arc::clone(&self.config);
        let reload_gate = Arc::clone(&self.reload_gate);
        let on_reload_option = self.on_reload.lock().ok().and_then(|guard| guard.clone());

        PollTarget {
            // The config path is fixed for a manager's lifetime, unlike the
            // theme manager's, which `switch_theme` can repoint.
            path_of: Box::new(move || Some(path_for_poll.clone())),
            reload: Box::new(move |path: &Path| {
                Self::reload_and_apply(&config_arc, &reload_gate, path, on_reload_option.as_ref());
            }),
        }
    }

    /// Spawn the poll-fallback thread with an already-resolved poll
    /// interval. No-op if a poll thread is already running. Split out from
    /// `start_poll_fallback` so tests can exercise the polling behavior
    /// directly without mutating `GPY_CONFIG_WATCH_POLL_MS` (this crate
    /// forbids `unsafe_code`, and `std::env::set_var` requires it).
    #[cfg(test)]
    fn spawn_poll_thread(&self, poll_interval: Duration, debounce_duration: Duration) {
        self.hot_reload
            .spawn_poll_thread(poll_interval, debounce_duration, self.poll_target());
    }

    /// Start the poll-fallback thread when `GPY_CONFIG_WATCH_POLL_MS` is set
    /// to a positive integer (milliseconds). No-op otherwise.
    ///
    /// This is a defense-in-depth path for environments where the file
    /// watcher's OS-level notifications (`FSEvents` on macOS) never arrive —
    /// see #354.
    fn start_poll_fallback(&self, debounce_duration: Duration) {
        self.hot_reload.start_poll_fallback(
            "GPY_CONFIG_WATCH_POLL_MS",
            debounce_duration,
            self.poll_target(),
        );
    }

    /// Ensure the config directory exists, creating it if necessary
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created
    fn ensure_config_directory_exists(parent: &Path) -> Result<()> {
        if !parent.exists() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Error::config(format!(
                    "Failed to create config directory {}: {e}",
                    parent.display()
                ))
            })?;
        }
        Ok(())
    }

    /// Stop watching the config file
    ///
    /// This is idempotent - calling it multiple times is safe.
    pub fn stop_watching(&self) {
        if self.hot_reload.stop() {
            debug_log!("config", "Stopped watching config file");
        }
        // Unregister from the registry the running watcher registered against,
        // which may be the agent's shared one rather than a private default.
        let started_with = self
            .watch_registry
            .lock()
            .ok()
            .and_then(|mut slot| slot.take());
        if let Some(registry) = started_with {
            registry.unregister_config_path(&self.config_path);
        }
    }

    /// Reload configuration from disk immediately.
    ///
    /// Serialized with the watcher- and poll-triggered reloads through the
    /// reload gate, so a reload the file watcher fires for the same write
    /// waits for this one and then sees the already-applied config (#661).
    ///
    /// # Errors
    ///
    /// Returns an error if the configuration file cannot be read, parsed,
    /// or validated, or if the in-memory config lock is poisoned.
    pub fn reload_now(&self) -> Result<()> {
        let _serialized = Self::enter_reload(&self.reload_gate);
        let old_config = self
            .config
            .read()
            .map_err(|_| Error::config("Config lock poisoned".to_owned()))?
            .clone();

        let on_reload_option = self.on_reload.lock().ok().and_then(|guard| guard.clone());
        let new_config =
            Self::try_reload(&self.config_path, on_reload_option.as_ref(), &old_config)?;

        let mut guard = self
            .config
            .write()
            .map_err(|_| Error::config("Config lock poisoned".to_owned()))?;
        *guard = Arc::new(new_config);
        drop(guard);

        Ok(())
    }
}

impl Drop for ConfigManager {
    fn drop(&mut self) {
        self.stop_watching();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    #[allow(clippy::missing_panics_doc)]
    fn test_config_manager_new_with_default() {
        // ConfigManager should fall back to built-in defaults when no config file
        // exists. Use `from_path` against a path inside a fresh temp dir so the test
        // is hermetic: `ConfigManager::new()` reads the developer's real
        // ~/.config/gpy/config.toml, which makes the assertions depend on local state
        // (e.g. a `theme = "starship"` override would fail this test).
        let temp = tempfile::TempDir::new().expect("tempdir");
        let missing_config = temp.path().join("config.toml");
        let manager =
            ConfigManager::from_path(&missing_config).expect("manager from missing config");
        let config = manager.get();

        // Default config should have reasonable values
        assert_eq!(config.agent.timeout_seconds.get(), 5); // Updated to match actual default
        assert_eq!(config.ui.theme.as_str(), "default");
    }

    #[test]
    #[allow(clippy::missing_panics_doc)]
    fn test_config_manager_get_returns_config() {
        let manager = ConfigManager::new().unwrap();
        let config = manager.get();

        // Should be able to access config fields
        assert!(!config.ui.theme.as_str().is_empty());
        assert!(config.agent.timeout_seconds.get() > 0);
    }

    #[test]
    #[allow(clippy::missing_panics_doc)]
    fn test_config_manager_start_stop_watching() {
        // Watch a `TempDir` rather than whatever `ConfigManager::new()`
        // resolves from the ambient environment.
        //
        // This test used to gate its assertion on
        // `XDG_CONFIG_HOME || HOME`, a #474-era workaround for a Windows
        // runner that could not resolve a config location at all. Two things
        // then went wrong with it. The predicate never named
        // `%LOCALAPPDATA%`, so once #473 taught `paths.rs` to resolve Windows
        // roots the test began asserting the absence of a feature that had
        // shipped. And `get_config_paths` always yields at least the
        // `.gpy.toml` fallback, so "the environment names a config location"
        // was not the thing that decided success in the first place (#527).
        //
        // A temp directory decides it locally on every platform, so the test
        // exercises the start/stop contract instead of the host's
        // environment.
        let temp = tempfile::TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(&config_path, "[git]\nenabled = true\n").expect("write config");
        let manager = ConfigManager::from_path(&config_path).expect("manager");

        manager
            .start_watching(Duration::from_millis(100))
            .expect("watching a real config path must start");

        // Starting again should be a no-op
        manager
            .start_watching(Duration::from_millis(100))
            .expect("start_watching must be idempotent");

        // Should be able to stop watching
        manager.stop_watching();

        // Stopping again should be safe
        manager.stop_watching();
    }

    #[test]
    #[allow(clippy::missing_panics_doc)]
    fn poll_fallback_detects_a_same_length_rewrite() {
        // The #529 blind spot end to end: same byte length, and written so
        // soon after the baseline that Windows gives both writes one
        // timestamp. Under the old mtime-equality check this rewrite was
        // missed permanently, because the loop had already adopted the
        // shared timestamp as its new baseline.
        let temp = tempfile::TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(&config_path, "[git]\nenabled = true \n").expect("write config");

        let manager = ConfigManager::from_path(&config_path).expect("manager");
        assert!(manager.get().git.enabled, "git should be enabled initially");

        manager.spawn_poll_thread(Duration::from_millis(10), Duration::from_millis(10));
        std::fs::write(&config_path, "[git]\nenabled = false\n").expect("write updated config");

        let mut reloaded = false;
        for _ in 0_i32..300_i32 {
            std::thread::sleep(Duration::from_millis(20));
            if !manager.get().git.enabled {
                reloaded = true;
                break;
            }
        }

        assert!(
            reloaded,
            "poll fallback must detect a rewrite that changes content but not length"
        );

        manager.stop_watching();
    }

    #[test]
    #[allow(clippy::missing_panics_doc)]
    fn test_reload_now_keeps_old_config_when_callback_rejects() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(
            &config_path,
            r#"
[ui]
theme = "default"
"#,
        )
        .expect("write config");

        let manager = ConfigManager::from_path(&config_path).expect("manager");
        manager.set_reload_callback(Arc::new(|_old, _new| {
            Err(Error::config("reject reload".to_owned()))
        }));

        std::fs::write(
            &config_path,
            r#"
[ui]
theme = "other"
"#,
        )
        .expect("write updated config");

        let err = manager.reload_now().expect_err("reload should fail");
        assert!(err.to_string().contains("reject reload"));
        assert_eq!(manager.get().ui.theme.as_str(), "default");
    }

    #[test]
    #[allow(clippy::missing_panics_doc)]
    fn test_config_manager_with_defaults_fallback() {
        // Test that with_defaults() always works (doesn't try to read files)
        let manager_res = ConfigManager::with_defaults();
        assert!(manager_res.is_ok());

        let manager = manager_res.unwrap();
        let config = manager.get();

        // Should have default values
        assert_eq!(config.agent.timeout_seconds.get(), 5);
        assert_eq!(config.ui.theme.as_str(), "default");
        assert!(config.git.skip_paths.is_empty());
    }

    #[test]
    #[allow(clippy::missing_panics_doc)]
    fn poll_fallback_reloads_without_any_watcher_running() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(&config_path, "[git]\nenabled = true\n").expect("write config");

        let manager = ConfigManager::from_path(&config_path).expect("manager");
        assert!(manager.get().git.enabled, "git should be enabled initially");

        // Exercise the poll thread directly (bypassing env-var parsing and
        // the FSEvents-backed WatchCoordinator entirely): deterministic
        // regardless of this machine's FSEvents behavior, and needs no
        // `unsafe` env mutation (this crate forbids unsafe_code).
        manager.spawn_poll_thread(Duration::from_millis(10), Duration::from_millis(10));

        std::fs::write(&config_path, "[git]\nenabled = false\n").expect("write updated config");

        // Generous ceiling (6s) so this doesn't flake under a heavily
        // parallel full test-suite run (many hundreds of concurrent tests
        // can delay this thread's scheduling well past the 10ms poll
        // interval); the loop still exits immediately once reloaded.
        let mut reloaded = false;
        for _ in 0_i32..300_i32 {
            std::thread::sleep(Duration::from_millis(20));
            if !manager.get().git.enabled {
                reloaded = true;
                break;
            }
        }

        assert!(
            reloaded,
            "poll fallback should reload the config with no watcher running"
        );

        manager.stop_watching();
    }

    /// Concurrent reloads run the callback exactly once per change (#661).
    ///
    /// The CLI writes the config file and then requests a reload over IPC,
    /// so the explicit `reload_now` and the watcher's own reload both fire
    /// for one change. The second must wait for the first and then see the
    /// already-applied config, not read the pre-request config while the
    /// first is still inside its callback and re-run the callback (and so
    /// the theme switch) against the same stale `old`.
    #[test]
    #[allow(clippy::missing_panics_doc)]
    fn concurrent_reloads_run_the_callback_once_per_change() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(&config_path, "[ui]\ntheme = \"default\"\n").expect("write config");
        let manager = Arc::new(ConfigManager::from_path(&config_path).expect("manager"));

        // Every (old, new) theme pair the callback is asked to apply, in order.
        let transitions = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let recorded = Arc::clone(&transitions);
        manager.set_reload_callback(Arc::new(move |old: &Config, new: &Config| {
            recorded
                .lock()
                .expect("transitions lock")
                .push((old.ui.theme.to_string(), new.ui.theme.to_string()));
            // Stand in for the theme switch: long enough that the second
            // reload below starts while this one is still applying.
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        }));

        std::fs::write(&config_path, "[ui]\ntheme = \"text\"\n").expect("write updated config");

        let explicit_manager = Arc::clone(&manager);
        let explicit = std::thread::spawn(move || explicit_manager.reload_now());
        // Let the explicit reload reach its callback before the watcher-shaped
        // reload starts; the gate, not this delay, is what the test relies on
        // for ordering -- the delay only makes the race window certain.
        std::thread::sleep(Duration::from_millis(100));
        // The same entry point the poll fallback (and, via
        // `create_reload_callback`, the file watcher) uses.
        (manager.poll_target().reload)(&config_path);
        explicit
            .join()
            .expect("reload thread")
            .expect("explicit reload succeeds");

        let seen = transitions.lock().expect("transitions lock").clone();
        assert_eq!(
            seen,
            vec![
                ("default".to_owned(), "text".to_owned()),
                ("text".to_owned(), "text".to_owned()),
            ],
            "the second reload must observe the config the first one applied, \
             so the change is applied exactly once"
        );
        assert_eq!(manager.get().ui.theme.as_str(), "text");
    }
}

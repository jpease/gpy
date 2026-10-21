//! Configuration management with hot-reload: `ConfigManager` holds the live
//! `Arc<Config>` and swaps it atomically when a valid edit lands; invalid
//! edits keep the old config.

use crate::config::schema::{get_config_paths, resolve_active_config};
use crate::config::{Config, loader::load_config_from_file};
use crate::watcher::hot_reload::{HotReloadSlot, PollTarget};
use crate::watcher::{WatchCoordinator, WatchRegistry};
use crate::{Error, Result, debug_log};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, PoisonError, RwLock};
use std::time::Duration;

/// Runs before a reload commits; an error aborts it and keeps the old config.
pub type ConfigReloadCallback = Arc<dyn Fn(&Config, &Config) -> Result<()> + Send + Sync>;

/// Thread-safe configuration with optional hot-reload.
pub struct ConfigManager {
    /// Candidates, highest priority first; re-resolved on every reload.
    candidates: Arc<[PathBuf]>,
    config: Arc<RwLock<Arc<Config>>>,
    hot_reload: HotReloadSlot,
    /// Invoked before committing a reload as (old, new).
    on_reload: Arc<Mutex<Option<ConfigReloadCallback>>>,
    /// Registry the watcher registered on, so `stop_watching` unregisters from it.
    watch_registry: Mutex<Option<Arc<WatchRegistry>>>,
    /// Symlink-target directory watched when the active candidate is a symlink.
    canonical_watch: Arc<Mutex<Option<PathBuf>>>,
    /// Serializes reloads across read-old/callback/store-new. The CLI write
    /// and its IPC reload both fire; without this both run the callback against
    /// the same stale old config. `()` so a callback panic is recoverable.
    reload_gate: Arc<Mutex<()>>,
}

impl ConfigManager {
    /// Start on built-in defaults (no file read) but reload from the standard
    /// candidates; fallback when the startup config is invalid.
    ///
    /// # Errors
    /// Returns an error if no config path can be determined.
    pub fn with_defaults() -> Result<Self> {
        Self::from_candidates_with_defaults(Self::ambient_candidates())
    }

    /// Create a new `ConfigManager` for a specific config file path.
    ///
    /// # Errors
    /// Returns an error if the path is not UTF-8 or the file fails to parse.
    pub fn from_path<P: AsRef<Path>>(path: P) -> Result<Self> {
        Self::from_candidates(vec![path.as_ref().to_path_buf()])
    }

    /// Load the highest-priority existing candidate (or defaults).
    ///
    /// # Errors
    /// Returns an error if `candidates` is empty or the active file fails to load.
    pub fn from_candidates(candidates: Vec<PathBuf>) -> Result<Self> {
        Self::ensure_candidates(&candidates)?;
        let config = Self::load_resolved(&candidates)?;
        Ok(Self::from_parts(candidates, config))
    }

    /// Like [`from_candidates`](Self::from_candidates) but starts on defaults.
    ///
    /// # Errors
    /// Returns an error if `candidates` is empty.
    pub fn from_candidates_with_defaults(candidates: Vec<PathBuf>) -> Result<Self> {
        Self::ensure_candidates(&candidates)?;
        Ok(Self::from_parts(candidates, Config::default()))
    }

    /// Load from the standard locations (`GPY_CONFIG_PATH`, XDG, `~/.config`).
    ///
    /// # Errors
    /// Returns an error if a config file exists but is invalid.
    pub fn new() -> Result<Self> {
        Self::from_candidates(Self::ambient_candidates())
    }

    fn ambient_candidates() -> Vec<PathBuf> {
        get_config_paths().into_iter().map(PathBuf::from).collect()
    }

    fn ensure_candidates(candidates: &[PathBuf]) -> Result<()> {
        if candidates.is_empty() {
            return Err(Error::config("No config paths available".to_owned()));
        }
        Ok(())
    }

    fn from_parts(candidates: Vec<PathBuf>, config: Config) -> Self {
        Self {
            candidates: Arc::from(candidates),
            config: Arc::new(RwLock::new(Arc::new(config))),
            // No candidate left means defaults, so the poller must see deletions.
            hot_reload: HotReloadSlot::new().reloading_on_missing(),
            on_reload: Arc::new(Mutex::new(None)),
            watch_registry: Mutex::new(None),
            canonical_watch: Arc::new(Mutex::new(None)),
            reload_gate: Arc::new(Mutex::new(())),
        }
    }

    /// Set the callback run before each reload commits.
    pub fn set_reload_callback(&self, callback: ConfigReloadCallback) {
        *self
            .on_reload
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(callback);
    }

    #[cfg(test)]
    /// Invoke the reload callback with synthetic configs.
    ///
    /// # Panics
    /// Panics when the callback returns an error.
    pub fn trigger_reload_for_tests(&self, old_config: &Config, new_config: &Config) {
        if let Some(callback) = self
            .on_reload
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
        {
            callback(old_config, new_config).expect("reload callback should succeed");
        }
    }

    #[cfg(test)]
    /// Overwrite the in-memory config.
    pub fn overwrite_for_tests(&self, new_config: Config) {
        *self.config.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(new_config);
    }

    /// Current config (cheap `Arc` clone); recovers from a poisoned lock.
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

    /// Watch the config candidates with a private registry.
    ///
    /// # Errors
    /// Returns an error if the file watcher cannot be started.
    pub fn start_watching(&self, debounce_duration: Duration) -> Result<()> {
        self.start_watching_with(debounce_duration, &Arc::new(WatchRegistry::new()))
    }

    /// Watch the config candidates on a shared registry, so the repo watcher
    /// also classifies custom-named config files as config changes.
    ///
    /// # Errors
    /// Returns an error if the file watcher cannot be started.
    pub fn start_watching_with(
        &self,
        debounce_duration: Duration,
        registry: &Arc<WatchRegistry>,
    ) -> Result<()> {
        let started = self.hot_reload.start_watcher(|| {
            let on_reload_option = self
                .on_reload
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clone();
            let rearm: Box<dyn Fn() + Send + Sync + 'static> = {
                let candidates = Arc::clone(&self.candidates);
                let watcher = self.hot_reload.watcher_handle();
                let canonical_dir = Arc::clone(&self.canonical_watch);
                Box::new(move || {
                    Self::rearm_canonical_watch(&candidates, &watcher, &canonical_dir);
                })
            };

            let reload_callback = Self::create_reload_callback(
                Arc::clone(&self.config),
                Arc::clone(&self.reload_gate),
                Arc::clone(&self.candidates),
                on_reload_option,
                rearm,
            );

            // Never registers a repo itself; attribution uses the shared
            // registry's roots or the `.git` stat-walk fallback.
            let mut coordinator = WatchCoordinator::new(
                debounce_duration,
                reload_callback,
                Some(Arc::clone(registry)),
            )?;

            self.watch_candidate_dirs(&mut coordinator)?;
            self.arm_canonical_watch(&mut coordinator);
            for candidate in self.candidates.iter() {
                Self::register_candidate(registry, candidate);
            }
            *self
                .watch_registry
                .lock()
                .unwrap_or_else(PoisonError::into_inner) = Some(Arc::clone(registry));
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
            "Started watching config candidates: {:?}",
            self.candidates
        );

        self.start_poll_fallback(debounce_duration);

        Ok(())
    }

    /// Highest-priority existing file, else highest-priority candidate.
    fn active_path(candidates: &[PathBuf]) -> Option<PathBuf> {
        resolve_active_config(candidates).map(|(path, _existing)| path)
    }

    /// Load the active file, or defaults only when no file exists; a broken
    /// file errors so a reload keeps the last good config.
    ///
    /// # Errors
    /// Returns an error if the active file cannot be read, parsed, or validated.
    fn load_resolved(candidates: &[PathBuf]) -> Result<Config> {
        let Some((_, Some(existing))) = resolve_active_config(candidates) else {
            return Ok(Config::default());
        };
        let path_str = existing
            .to_str()
            .ok_or_else(|| Error::config("Config path contains invalid UTF-8".to_owned()))?;
        load_config_from_file(path_str)
    }

    /// Load the active config and run `on_reload` against `old`; no shared state.
    ///
    /// # Errors
    /// Returns an error if loading fails or `on_reload` rejects the change.
    fn try_reload(
        candidates: &[PathBuf],
        on_reload: Option<&ConfigReloadCallback>,
        old: &Config,
    ) -> Result<Config> {
        let new_config = Self::load_resolved(candidates)?;

        if let Some(callback) = on_reload {
            callback(old, &new_config)?;
        }

        Ok(new_config)
    }

    /// Reload and apply if valid, else log; shared by watcher and poller.
    fn reload_and_apply(
        config: &Arc<RwLock<Arc<Config>>>,
        reload_gate: &Mutex<()>,
        candidates: &[PathBuf],
        on_reload: Option<&ConfigReloadCallback>,
    ) {
        debug_log!("config", "Config file changed, reloading...");

        let _serialized = Self::enter_reload(reload_gate);
        let old_config = config
            .read()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();

        match Self::try_reload(candidates, on_reload, &old_config) {
            Ok(new_config) => {
                *config.write().unwrap_or_else(PoisonError::into_inner) = Arc::new(new_config);
                debug_log!("config", "Config reloaded successfully");
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

    fn create_reload_callback(
        config: Arc<RwLock<Arc<Config>>>,
        reload_gate: Arc<Mutex<()>>,
        candidates: Arc<[PathBuf]>,
        on_reload: Option<ConfigReloadCallback>,
        rearm: Box<dyn Fn() + Send + Sync + 'static>,
    ) -> Box<dyn Fn(crate::watcher::DebouncedEvent) + Send + Sync + 'static> {
        Box::new(move |event| {
            // The config dir is watched recursively, but `gpy/themes` belongs to
            // the theme watcher; reloading here too races it into a second doorbell.
            if event.is_theme_only() {
                return;
            }
            Self::reload_and_apply(&config, &reload_gate, &candidates, on_reload.as_ref());
            // A retarget (`ln -sf other config.toml`) lands as an event in the
            // lexical parent; follow the link to its new directory.
            rearm();
        })
    }

    /// Also watch the symlink target's directory (shallow): writes through the
    /// link land there, which the lexical parent never reports. Best-effort.
    fn arm_canonical_watch(&self, coordinator: &mut WatchCoordinator) {
        let Some(dir) = Self::active_path(&self.candidates)
            .as_deref()
            .and_then(Self::canonical_watch_dir)
        else {
            return;
        };
        match coordinator.watch_directory_shallow(&dir) {
            Ok(()) => {
                *self
                    .canonical_watch
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner) = Some(dir);
            }
            Err(e) => {
                debug_log!("config", "Failed to watch {}: {}", dir.display(), e);
            }
        }
    }

    /// Symlink target's directory when it differs from the link's; else `None`.
    fn canonical_watch_dir(config_path: &Path) -> Option<PathBuf> {
        let target_dir = std::fs::canonicalize(config_path)
            .ok()?
            .parent()?
            .to_path_buf();
        let link_dir = std::fs::canonicalize(config_path.parent()?).ok()?;
        (target_dir != link_dir).then_some(target_dir)
    }

    /// Retarget the shallow symlink watch only when the target moved: each
    /// extra watch rebuilds the `FSEvents` stream on macOS. Best-effort.
    fn rearm_canonical_watch(
        candidates: &[PathBuf],
        watcher: &Mutex<Option<WatchCoordinator>>,
        canonical_dir: &Mutex<Option<PathBuf>>,
    ) {
        let next = Self::active_path(candidates)
            .as_deref()
            .and_then(Self::canonical_watch_dir);
        let mut current = canonical_dir.lock().unwrap_or_else(PoisonError::into_inner);
        if *current == next {
            return;
        }
        // `stop` holds this lock while joining the thread we run on.
        let Ok(mut slot) = watcher.try_lock() else {
            return;
        };
        let Some(coordinator) = slot.as_mut() else {
            return;
        };
        if let Some(old) = current.take()
            && let Err(e) = coordinator.unwatch_directory_shallow(&old)
        {
            debug_log!("config", "Failed to unwatch {}: {}", old.display(), e);
        }
        if let Some(dir) = next {
            match coordinator.watch_directory_shallow(&dir) {
                Ok(()) => *current = Some(dir),
                Err(e) => {
                    debug_log!("config", "Failed to watch {}: {}", dir.display(), e);
                }
            }
        }
    }

    /// Recover a poisoned gate so a panicking callback doesn't wedge later reloads.
    fn enter_reload(reload_gate: &Mutex<()>) -> std::sync::MutexGuard<'_, ()> {
        reload_gate.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn poll_target(&self) -> PollTarget {
        let candidates_for_poll = Arc::clone(&self.candidates);
        let candidates_for_reload = Arc::clone(&self.candidates);
        let config_arc = Arc::clone(&self.config);
        let reload_gate = Arc::clone(&self.reload_gate);
        let on_reload_option = self
            .on_reload
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();

        PollTarget {
            path_of: Box::new(move || Self::active_path(&candidates_for_poll)),
            reload: Box::new(move |_path: &Path| {
                Self::reload_and_apply(
                    &config_arc,
                    &reload_gate,
                    &candidates_for_reload,
                    on_reload_option.as_ref(),
                );
            }),
        }
    }

    /// Tests call this directly since `set_var` needs `unsafe` (forbidden here).
    #[cfg(test)]
    fn spawn_poll_thread(&self, poll_interval: Duration, debounce_duration: Duration) {
        self.hot_reload
            .spawn_poll_thread(poll_interval, debounce_duration, self.poll_target());
    }

    /// Poll fallback when `GPY_CONFIG_WATCH_POLL_MS` > 0, for when `FSEvents`
    /// never arrive.
    fn start_poll_fallback(&self, debounce_duration: Duration) {
        self.hot_reload.start_poll_fallback(
            "GPY_CONFIG_WATCH_POLL_MS",
            debounce_duration,
            self.poll_target(),
        );
    }

    /// Watch every candidate's parent; the first is created if missing
    /// (`config set` writes there), others only if absolute and existing.
    ///
    /// # Errors
    /// Returns an error if the first candidate's directory cannot be created or watched.
    fn watch_candidate_dirs(&self, coordinator: &mut WatchCoordinator) -> Result<()> {
        let mut watched: Vec<&Path> = Vec::new();
        for (index, candidate) in self.candidates.iter().enumerate() {
            let Some(parent) = candidate.parent() else {
                continue;
            };
            if watched.contains(&parent) {
                continue;
            }
            if index == 0 {
                Self::ensure_config_directory_exists(parent)?;
                coordinator.watch_directory(parent)?;
            } else if candidate.is_absolute() && parent.is_dir() {
                if let Err(e) = coordinator.watch_directory(parent) {
                    debug_log!("config", "Failed to watch {}: {}", parent.display(), e);
                    continue;
                }
            } else {
                continue;
            }
            watched.push(parent);
        }
        Ok(())
    }

    /// Register under the literal and canonical-parent spellings: a not-yet-created
    /// file has no canonical form, but the OS reports it via its parent's.
    fn register_candidate(registry: &WatchRegistry, candidate: &Path) {
        registry.register_config_path(candidate);
        if let Some(canonical) = Self::dir_canonical_spelling(candidate) {
            registry.register_config_path(&canonical);
        }
    }

    /// Undo [`register_candidate`](Self::register_candidate).
    fn unregister_candidate(registry: &WatchRegistry, candidate: &Path) {
        registry.unregister_config_path(candidate);
        if let Some(canonical) = Self::dir_canonical_spelling(candidate) {
            registry.unregister_config_path(&canonical);
        }
    }

    fn dir_canonical_spelling(candidate: &Path) -> Option<PathBuf> {
        let name = candidate.file_name()?;
        Some(std::fs::canonicalize(candidate.parent()?).ok()?.join(name))
    }

    /// # Errors
    /// Returns an error if the directory cannot be created.
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

    /// Stop watching; idempotent.
    pub fn stop_watching(&self) {
        if self.hot_reload.stop() {
            debug_log!("config", "Stopped watching config file");
        }
        // May be the agent's shared registry, not a private one.
        let started_with = self
            .watch_registry
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(registry) = started_with {
            for candidate in self.candidates.iter() {
                Self::unregister_candidate(&registry, candidate);
            }
        }
        *self
            .canonical_watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Re-resolve and reload now, serialized with watcher/poll reloads.
    ///
    /// # Errors
    /// Returns an error if the active file fails to load or the lock is poisoned.
    pub fn reload_now(&self) -> Result<()> {
        let _serialized = Self::enter_reload(&self.reload_gate);
        let old_config = self
            .config
            .read()
            .map_err(|_| Error::config("Config lock poisoned".to_owned()))?
            .clone();

        let on_reload_option = self
            .on_reload
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone();
        let new_config =
            Self::try_reload(&self.candidates, on_reload_option.as_ref(), &old_config)?;

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
    fn test_config_manager_new_with_default() {
        // Hermetic: `new()` would read the developer's real config.
        let temp = tempfile::TempDir::new().expect("tempdir");
        let missing_config = temp.path().join("config.toml");
        let manager =
            ConfigManager::from_path(&missing_config).expect("manager from missing config");
        let config = manager.get();

        assert_eq!(config.agent.timeout_seconds.get(), 5);
        assert_eq!(config.ui.theme.as_str(), "default");
    }

    #[test]
    fn test_config_manager_get_returns_config() {
        // `new` would parse the developer's real config.toml.
        let manager = ConfigManager::with_defaults().unwrap();
        let config = manager.get();

        assert_ne!(config.ui.theme.as_str(), "");
        assert!(config.agent.timeout_seconds.get() > 0);
    }

    #[test]
    fn test_config_manager_start_stop_watching() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(&config_path, "[git]\nenabled = true\n").expect("write config");
        let manager = ConfigManager::from_path(&config_path).expect("manager");

        manager
            .start_watching(Duration::from_millis(100))
            .expect("watching a real config path must start");

        manager
            .start_watching(Duration::from_millis(100))
            .expect("start_watching must be idempotent");

        manager.stop_watching();

        manager.stop_watching();
    }

    /// The config dir is watched recursively, yet an edit under `gpy/themes`
    /// belongs to the theme watcher and must not reload config; a config edit
    /// still does, which also proves the watcher was live for the theme edit.
    #[test]
    fn config_watcher_leaves_theme_edits_to_the_theme_watcher() {
        use std::sync::atomic::{AtomicUsize, Ordering};

        let temp = tempfile::TempDir::new().expect("tempdir");
        let themes = temp.path().join("gpy").join("themes");
        std::fs::create_dir_all(&themes).expect("create themes dir");
        let theme_path = themes.join("mine.toml");
        std::fs::write(&theme_path, "[ui]\n").expect("write theme");
        let config_path = temp.path().join("gpy").join("config.toml");
        std::fs::write(&config_path, "[git]\nenabled = true\n").expect("write config");

        let manager = ConfigManager::from_path(&config_path).expect("manager");
        let reloads = Arc::new(AtomicUsize::new(0));
        let counted = Arc::clone(&reloads);
        // FSEvents may replay the fixture writes made just before the stream
        // started, firing reloads of unchanged content. Count only reloads that
        // change `git.enabled`: the config edit does, a theme edit never could.
        manager.set_reload_callback(Arc::new(move |old: &Config, new: &Config| {
            if old.git.enabled != new.git.enabled {
                counted.fetch_add(1, Ordering::SeqCst);
            }
            Ok(())
        }));
        manager
            .start_watching(Duration::from_millis(100))
            .expect("start watching");
        std::thread::sleep(Duration::from_millis(500));

        std::fs::write(&theme_path, "[ui]\n# edited\n").expect("edit theme");
        // Several flushes of the 100ms debounce, so a theme reload would land.
        std::thread::sleep(Duration::from_millis(1500));
        std::fs::write(&config_path, "[git]\nenabled = false\n").expect("edit config");
        let mut applied = false;
        for _ in 0_i32..500_i32 {
            if !manager.get().git.enabled {
                applied = true;
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        std::thread::sleep(Duration::from_millis(500));
        manager.stop_watching();

        assert!(applied, "the config edit must reload");
        assert_eq!(
            reloads.load(Ordering::SeqCst),
            1,
            "only the config edit may reload config"
        );
    }

    #[test]
    fn poll_fallback_detects_a_same_length_rewrite() {
        // Same length, written within one mtime tick (Windows), so only a
        // content check catches it.
        fn write_file_atomically(path: &std::path::Path, contents: &str) {
            let tmp = path.with_extension("toml.tmp");
            std::fs::write(&tmp, contents).expect("write temp config");
            std::fs::rename(&tmp, path).expect("rename temp config");
        }

        let temp = tempfile::TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(&config_path, "[git]\nenabled = true \n").expect("write config");

        let manager = ConfigManager::from_path(&config_path).expect("manager");
        assert!(manager.get().git.enabled, "git should be enabled initially");

        let (tx, rx) = std::sync::mpsc::channel();
        manager.set_reload_callback(Arc::new(move |_old: &Config, new: &Config| {
            let _ = tx.send(new.git.enabled);
            Ok(())
        }));
        manager.spawn_poll_thread(Duration::from_millis(10), Duration::from_millis(10));
        write_file_atomically(&config_path, "[git]\nenabled = false\n");

        let mut reloaded = false;
        while let Ok(enabled) = rx.recv_timeout(Duration::from_secs(30)) {
            if !enabled {
                reloaded = true;
                break;
            }
        }
        manager.stop_watching();

        assert!(
            reloaded,
            "poll fallback must detect a rewrite that changes content but not length"
        );
        assert!(!manager.get().git.enabled);
    }

    #[test]
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
    fn test_config_manager_with_defaults_fallback() {
        let manager_res = ConfigManager::with_defaults();
        assert!(manager_res.is_ok());

        let manager = manager_res.unwrap();
        let config = manager.get();

        assert_eq!(config.agent.timeout_seconds.get(), 5);
        assert_eq!(config.ui.theme.as_str(), "default");
        assert_eq!(config.git.skip_paths, Vec::<String>::new());
    }

    #[test]
    fn poll_fallback_reloads_without_any_watcher_running() {
        fn write_file_atomically(path: &std::path::Path, contents: &str) {
            let tmp = path.with_extension("toml.tmp");
            std::fs::write(&tmp, contents).expect("write temp config");
            std::fs::rename(&tmp, path).expect("rename temp config");
        }

        let temp = tempfile::TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(&config_path, "[git]\nenabled = true\n").expect("write config");

        let manager = ConfigManager::from_path(&config_path).expect("manager");
        assert!(manager.get().git.enabled, "git should be enabled initially");

        let (tx, rx) = std::sync::mpsc::channel();
        manager.set_reload_callback(Arc::new(move |_old: &Config, new: &Config| {
            let _ = tx.send(new.git.enabled);
            Ok(())
        }));
        // Poll thread only: deterministic regardless of FSEvents.
        manager.spawn_poll_thread(Duration::from_millis(10), Duration::from_millis(10));

        write_file_atomically(&config_path, "[git]\nenabled = false\n");

        let mut reloaded = false;
        while let Ok(enabled) = rx.recv_timeout(Duration::from_secs(30)) {
            if !enabled {
                reloaded = true;
                break;
            }
        }
        manager.stop_watching();

        assert!(
            reloaded,
            "poll fallback should reload the config with no watcher running"
        );
        assert!(!manager.get().git.enabled);
    }

    /// The CLI's `reload_now` and the watcher's reload both fire per change;
    /// the second must see the applied config, not re-run against stale `old`.
    #[test]
    fn concurrent_reloads_run_the_callback_once_per_change() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let config_path = temp.path().join("config.toml");
        std::fs::write(&config_path, "[ui]\ntheme = \"default\"\n").expect("write config");
        let manager = Arc::new(ConfigManager::from_path(&config_path).expect("manager"));

        let transitions = Arc::new(Mutex::new(Vec::<(String, String)>::new()));
        let recorded = Arc::clone(&transitions);
        manager.set_reload_callback(Arc::new(move |old: &Config, new: &Config| {
            recorded
                .lock()
                .expect("transitions lock")
                .push((old.ui.theme.to_string(), new.ui.theme.to_string()));
            // Long enough that the second reload starts mid-callback.
            std::thread::sleep(Duration::from_millis(300));
            Ok(())
        }));

        std::fs::write(&config_path, "[ui]\ntheme = \"text\"\n").expect("write updated config");

        let explicit_manager = Arc::clone(&manager);
        let explicit = std::thread::spawn(move || explicit_manager.reload_now());
        // The delay only makes the race window certain; the gate orders them.
        std::thread::sleep(Duration::from_millis(100));
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

    /// Deleting the active config reverts to defaults.
    #[test]
    fn reload_after_delete_reverts_to_defaults() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "[ui]\ntheme = \"text\"\n").expect("write config");
        let manager = ConfigManager::from_candidates(vec![path.clone()]).expect("manager");
        assert_eq!(manager.get().ui.theme.as_str(), "text");

        std::fs::remove_file(&path).expect("delete config");

        manager
            .reload_now()
            .expect("reload with no file is defaults");
        assert_eq!(manager.get().ui.theme.as_str(), "default");
    }

    #[test]
    fn reload_picks_highest_priority_existing_candidate() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let higher = temp.path().join("a.toml");
        let lower = temp.path().join("b.toml");
        std::fs::write(&lower, "[ui]\ntheme = \"text\"\n").expect("write lower");
        let manager = ConfigManager::from_candidates(vec![higher.clone(), lower]).expect("manager");
        assert_eq!(manager.get().ui.theme.as_str(), "text");

        std::fs::write(&higher, "[ui]\ntheme = \"starship\"\n").expect("write higher");
        manager.reload_now().expect("reload");
        assert_eq!(manager.get().ui.theme.as_str(), "starship");

        std::fs::remove_file(&higher).expect("delete higher");
        manager.reload_now().expect("reload");
        assert_eq!(manager.get().ui.theme.as_str(), "text");
    }

    #[test]
    fn fallback_manager_reloads_the_file_that_failed() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let first = temp.path().join("a.toml");
        let failing = temp.path().join("b.toml");
        std::fs::write(
            &failing,
            "[ui]\ntheme = \"text\"\n[git]\ntimeout_seconds = 0\n",
        )
        .expect("write invalid config");
        assert!(
            ConfigManager::from_candidates(vec![first.clone(), failing.clone()]).is_err(),
            "startup on an invalid active file fails"
        );
        let manager = ConfigManager::from_candidates_with_defaults(vec![first, failing.clone()])
            .expect("fallback manager");
        assert_eq!(manager.get().ui.theme.as_str(), "default");

        std::fs::write(&failing, "[ui]\ntheme = \"text\"\n").expect("fix config");
        manager.reload_now().expect("reload");
        assert_eq!(manager.get().ui.theme.as_str(), "text");
    }

    /// Only "no file at all" means defaults; an invalid file keeps the last good config.
    #[test]
    fn reload_keeps_last_good_config_when_the_active_file_turns_invalid() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "[ui]\ntheme = \"text\"\n").expect("write config");
        let manager = ConfigManager::from_candidates(vec![path.clone()]).expect("manager");

        std::fs::write(&path, "[git]\ntimeout_seconds = 0\n").expect("break config");

        manager
            .reload_now()
            .expect_err("invalid file fails the reload");
        assert_eq!(manager.get().ui.theme.as_str(), "text");
    }

    #[test]
    fn poll_fallback_reverts_to_defaults_after_delete() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let path = temp.path().join("config.toml");
        std::fs::write(&path, "[ui]\ntheme = \"text\"\n").expect("write config");
        let manager = ConfigManager::from_candidates(vec![path.clone()]).expect("manager");
        assert_eq!(manager.get().ui.theme.as_str(), "text");

        manager.spawn_poll_thread(Duration::from_millis(10), Duration::from_millis(10));
        std::fs::remove_file(&path).expect("delete config");

        let mut reverted = false;
        for _ in 0_i32..300_i32 {
            std::thread::sleep(Duration::from_millis(20));
            if manager.get().ui.theme.as_str() == "default" {
                reverted = true;
                break;
            }
        }
        manager.stop_watching();

        assert!(reverted, "poll fallback must revert to defaults on delete");
    }

    /// Includes a not-yet-existing candidate; unregistered on stop.
    #[test]
    fn every_candidate_is_registered_while_watching() {
        let temp = tempfile::TempDir::new().expect("tempdir");
        let missing = temp.path().join("a.toml");
        let existing = temp.path().join("b.toml");
        std::fs::write(&existing, "[ui]\ntheme = \"text\"\n").expect("write config");
        // Spelled the way the OS reports it once created (macOS: /private/var).
        let reported = std::fs::canonicalize(temp.path())
            .expect("canonical dir")
            .join("a.toml");
        let manager = ConfigManager::from_candidates(vec![missing.clone(), existing.clone()])
            .expect("manager");
        let registry = Arc::new(WatchRegistry::new());

        manager
            .start_watching_with(Duration::from_millis(100), &registry)
            .expect("start watching");
        assert!(registry.is_registered_config_path(&missing));
        assert!(registry.is_registered_config_path(&reported));
        assert!(registry.is_registered_config_path(&existing));

        manager.stop_watching();
        assert!(!registry.is_registered_config_path(&missing));
        assert!(!registry.is_registered_config_path(&reported));
        assert!(!registry.is_registered_config_path(&existing));
    }
}

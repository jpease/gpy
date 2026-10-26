//! IPC handlers for client registration, workspace updates, and agent control.

use super::git_handler::publish_and_repaint;
use super::{HandlerError, RenderDeps, RequestHandler, spawn_detached};
use crate::git::CompleteStatus;
use crate::git::cache::GitStatusCache;
use crate::ipc::{Message, Response};
use crate::sync_ext::MutexExt;
use crate::warn_log;
use crate::watcher::multi_repo::MultiRepoWatcher;
use std::sync::{Arc, Mutex};

/// Shared, lazily-set watcher handle.
type WatcherRef = Arc<Mutex<Option<Arc<MultiRepoWatcher>>>>;

/// Handler for client lifecycle, status, config reload, ping and shutdown.
pub struct ClientHandler {
    /// Publishing handles; see [`ClientHandler::trigger_initial_scan`].
    render: RenderDeps,
    git_cache: Arc<GitStatusCache>,
    watcher: WatcherRef,
}

impl ClientHandler {
    /// Create a new `ClientHandler`.
    #[must_use]
    pub const fn new(
        render: RenderDeps,
        git_cache: Arc<GitStatusCache>,
        watcher: WatcherRef,
    ) -> Self {
        Self {
            render,
            git_cache,
            watcher,
        }
    }

    /// Handle client registration (PID already validated by the server).
    ///
    /// # Errors
    /// Fails if workspace path validation fails.
    #[expect(clippy::unnecessary_wraps, reason = "dispatch signature")]
    fn handle_register_client(
        &self,
        pid: crate::config::types::ClientPid,
        cwd: Option<crate::security::SafePath>,
        shell: Option<&crate::config::types::ShellVariant>,
        shell_version: Option<&String>,
    ) -> Result<Response, HandlerError> {
        let shell_info = match (&shell, &shell_version) {
            (Some(s), Some(v)) => format!("{s} {v}"),
            (Some(s), None) => format!("{s}"),
            _ => "unknown".to_owned(),
        };

        crate::debug::write_debug_log(
            "client",
            &format!("Registering client: PID={pid}, shell={shell_info}"),
        );

        let sanitized = cwd.map(crate::security::SafePath::into_inner);

        self.render
            .client_registry
            .register(pid.get(), sanitized.clone());

        if let Some(ref path) = sanitized {
            // Best-effort: watcher failures must not break registration; the
            // initial scan still fills the cache and TTL reconciles missed events.
            let current_watcher = self.watcher.lock_or_recover().clone();
            if let Some(watcher) = current_watcher
                && let Err(e) = watcher.register_client(pid.get(), path)
            {
                crate::debug::write_debug_log(
                    "client",
                    &format!("Watcher registration failed for PID={pid} (continuing): {e}"),
                );
            }

            // Populate the instant-prompt cache immediately, watcher or not.
            self.trigger_initial_scan(path);
        }

        Ok(Response::Ack)
    }

    /// Handle workspace update for a registered client.
    ///
    /// # Errors
    /// Fails if path validation fails or the PID is not registered.
    fn handle_workspace_update(
        &self,
        pid: crate::config::types::ClientPid,
        cwd: &crate::security::SafePath,
    ) -> Result<Response, HandlerError> {
        let path_buf = cwd.as_path();

        if !self
            .render
            .client_registry
            .update_workspace(pid.get(), path_buf)
        {
            return Err(HandlerError::PidNotRegistered(pid.get()));
        }

        // Clone out first: in an `if let` chain the guard would live through
        // the block, holding the slot across the watcher call and the scan.
        let current_watcher = self.watcher.lock_or_recover().clone();
        // Best-effort: on watcher failure, scan immediately so the cache is fresh.
        if let Some(watcher) = current_watcher
            && let Err(e) = watcher.update_client(pid.get(), path_buf)
        {
            crate::debug::write_debug_log(
                "client",
                &format!("Watcher update failed for PID={pid} (continuing): {e}"),
            );
            self.trigger_initial_scan(path_buf);
        }

        Ok(Response::Ack)
    }

    /// Handle client unregistration.
    ///
    /// # Errors
    /// Never fails; matches the dispatch signature.
    #[expect(clippy::unnecessary_wraps, reason = "dispatch signature")]
    fn handle_unregister_client(
        &self,
        pid: crate::config::types::ClientPid,
    ) -> Result<Response, HandlerError> {
        self.render.client_registry.unregister(pid.get());
        let current_watcher = self.watcher.lock_or_recover().clone();
        if let Some(watcher) = current_watcher {
            watcher.unregister_client(pid.get());
        }

        Ok(Response::Ack)
    }

    /// Handle agent status query.
    ///
    /// # Errors
    /// Never fails; matches the dispatch signature.
    #[expect(clippy::unnecessary_wraps, reason = "dispatch signature")]
    fn handle_status(&self) -> Result<Response, HandlerError> {
        let watched_repos = self
            .watcher
            .lock_or_recover()
            .as_ref()
            .map_or(0, |w| w.watched_repo_count());
        let registered_clients = self.render.client_registry.len();
        let cache_entries = self.git_cache.len();

        Ok(Response::AgentStatus {
            version: crate::VERSION.to_owned(),
            protocol_version: crate::ipc::protocol::PROTOCOL_VERSION,
            watched_repos,
            registered_clients,
            cache_entries,
        })
    }

    /// Handle configuration reload request.
    ///
    /// # Errors
    /// Fails if configuration reload fails.
    fn handle_config_reload(&self) -> Result<Response, HandlerError> {
        self.render
            .config_manager
            .reload_now()
            .map_err(|e| HandlerError::OperationFailed {
                label: "Config reload",
                source: e,
            })?;
        Ok(Response::Ack)
    }

    /// Handle ping request.
    ///
    /// # Errors
    /// Never fails; matches the dispatch signature.
    #[expect(clippy::unnecessary_wraps, reason = "dispatch signature")]
    #[expect(clippy::unused_self, reason = "dispatch signature")]
    const fn handle_ping(&self) -> Result<Response, HandlerError> {
        Ok(Response::Ack)
    }

    /// Handle shutdown request.
    ///
    /// # Errors
    /// Never fails; matches the dispatch signature.
    #[expect(clippy::unnecessary_wraps, reason = "dispatch signature")]
    #[expect(clippy::unused_self, reason = "dispatch signature")]
    const fn handle_shutdown(&self) -> Result<Response, HandlerError> {
        Ok(Response::Ack)
    }

    /// Populate the instant-prompt cache for a newly registered client's repo.
    fn trigger_initial_scan(&self, path: &std::path::Path) {
        use crate::git::status::load_repository_state;
        use crate::watcher::multi_repo::MultiRepoWatcher;

        let Some(git_root) = MultiRepoWatcher::find_git_root(path) else {
            return;
        };

        if !self
            .render
            .config_manager
            .get()
            .git
            .is_enabled_for(&git_root)
        {
            return;
        }

        let git_cache = Arc::clone(&self.git_cache);
        let render = self.render.clone();
        let scan_path = git_root.clone();

        let job = move || {
            let config = render.config_manager.get();
            let started = std::time::Instant::now();
            match load_repository_state(
                &scan_path,
                None,
                config.git.max_ahead_behind,
                config.git.stash_enabled,
            ) {
                Ok(Some(CompleteStatus { status, files })) => {
                    // `git_root` is already canonical. A newer entry was
                    // already published, so skip republishing older content.
                    if git_cache.set_canonical_if_not_newer(
                        &git_root,
                        started,
                        status.clone(),
                        files,
                    ) {
                        // No request color context, so prev_bg is None.
                        publish_and_repaint(&render, &git_root, &status, None);
                    }
                }
                Ok(None) => {}
                Err(e) => {
                    warn_log!("client", "Initial git scan failed on registration: {e}");
                }
            }
        };

        spawn_detached(job);
    }
}

impl RequestHandler for ClientHandler {
    fn handle(&self, message: &Message) -> Result<Response, HandlerError> {
        match message {
            Message::RegisterClient {
                pid,
                cwd,
                shell,
                shell_version,
            } => self.handle_register_client(
                *pid,
                cwd.clone(),
                shell.as_ref(),
                shell_version.as_ref(),
            ),
            Message::UnregisterClient { pid } => self.handle_unregister_client(*pid),
            Message::WorkspaceUpdate { pid, cwd } => self.handle_workspace_update(*pid, cwd),
            Message::Status => self.handle_status(),
            Message::ConfigReload => self.handle_config_reload(),
            Message::Ping => self.handle_ping(),
            Message::Shutdown => self.handle_shutdown(),
            _ => Err(HandlerError::UnexpectedMessage {
                handler: "ClientHandler",
                qualifier: "unexpected",
                message: format!("{message:?}"),
            }),
        }
    }

    fn name(&self) -> &'static str {
        "ClientHandler"
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]
    use super::*;
    use crate::cache::InstantPromptCache;
    use crate::config::manager::ConfigManager;
    use crate::config::{Config, GitSettings};
    use crate::ipc::ClientDirectory;
    use crate::palette::PaletteCache;
    use crate::theme::ThemeManager;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    fn init_repo(repo: &Path) -> PathBuf {
        std::fs::create_dir_all(repo).expect("repo dir");
        let output = std::process::Command::new("git")
            .args(["init", "--initial-branch=main"])
            .arg(repo)
            .output()
            .expect("run git init");
        assert!(output.status.success(), "git init failed");
        std::fs::write(repo.join("untracked.txt"), "x").expect("write file");
        std::fs::canonicalize(repo).expect("canonical repo")
    }

    fn handler_with(skip_paths: Vec<String>, cache_dir: &Path) -> ClientHandler {
        let config_manager = Arc::new(
            ConfigManager::from_config(Config {
                git: GitSettings {
                    skip_paths,
                    ..GitSettings::default()
                },
                ..Config::default()
            })
            .expect("config"),
        );
        let render = RenderDeps {
            palette_cache: Arc::new(PaletteCache::from_config(&config_manager.get())),
            config_manager,
            theme_manager: Arc::new(ThemeManager::builtin("default").expect("theme")),
            instant_cache: Arc::new(
                InstantPromptCache::new_in_dir(cache_dir.to_path_buf()).expect("cache"),
            ),
            client_registry: ClientDirectory::new().shared(),
        };
        ClientHandler::new(
            render,
            Arc::new(GitStatusCache::new()),
            Arc::new(Mutex::new(None)),
        )
    }

    fn git_cache_files(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir)
            .expect("read cache dir")
            .filter_map(Result::ok)
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains(".git"))
            .collect()
    }

    fn register(handler: &ClientHandler, cwd: &Path) {
        let pid = crate::config::types::ClientPid::new(std::process::id()).expect("pid");
        let safe = crate::security::SafePath::new(cwd.to_str().expect("utf8")).expect("safe");
        handler
            .handle_register_client(pid, Some(safe), None, None)
            .expect("register");
    }

    /// Registering in a repo skipped via a symlinked entry writes no git cache files.
    #[cfg(unix)]
    #[test]
    fn register_in_skipped_repo_writes_no_git_instant_cache() {
        let tmp = tempfile::TempDir::new().expect("tmp");
        let real = tmp.path().join("real");
        let repo = init_repo(&real.join("repo"));
        let link = tmp.path().join("link");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");
        let cache_dir = tmp.path().join("cache");

        let handler = handler_with(vec![link.to_string_lossy().into_owned()], &cache_dir);
        register(&handler, &repo);

        // No job to wait on; allow a real scan's latency to elapse.
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(git_cache_files(&cache_dir), Vec::<String>::new());
    }

    /// Control: without a skip entry the cache is populated.
    #[test]
    fn register_in_unskipped_repo_writes_git_instant_cache() {
        let tmp = tempfile::TempDir::new().expect("tmp");
        let repo = init_repo(&tmp.path().join("repo"));
        let cache_dir = tmp.path().join("cache");

        let handler = handler_with(Vec::new(), &cache_dir);
        register(&handler, &repo);

        let deadline = Instant::now() + Duration::from_secs(5);
        while git_cache_files(&cache_dir).is_empty() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        assert_ne!(git_cache_files(&cache_dir), Vec::<String>::new());
    }
}

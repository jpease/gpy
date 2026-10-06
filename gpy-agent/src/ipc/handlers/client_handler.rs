//! IPC handlers for client registration and workspace updates.
//!
//! Fish shells identify themselves to the daemon so they can receive live prompt
//! repaint signals. This handler updates the shared [`crate::ipc::ClientDirectory`]
//! and coordinates watcher registration for the client workspace without
//! embedding that bookkeeping in the socket server.

use super::git_handler::publish_and_repaint;
use super::{HandlerError, RenderDeps, RequestHandler, spawn_detached};
use crate::git::CompleteStatus;
use crate::git::cache::GitStatusCache;
use crate::ipc::{LatencyTracker, Message, Response};
use crate::warn_log;
use crate::watcher::multi_repo::MultiRepoWatcher;
use std::sync::{Arc, Mutex};

/// Type alias for the watcher reference used in client handler
type WatcherRef = Arc<Mutex<Option<Arc<MultiRepoWatcher>>>>;

/// Handler for client management operations
///
/// Processes messages related to client lifecycle, status, and configuration:
/// - Client registration/unregistration
/// - Workspace updates
/// - Agent status queries
/// - Configuration reload
/// - Latency statistics
/// - Ping/shutdown commands
pub struct ClientHandler {
    /// Everything needed to publish the initial scan's status to the
    /// instant-prompt cache and repaint live shells; see
    /// [`ClientHandler::trigger_initial_scan`].
    render: RenderDeps,
    git_cache: Arc<GitStatusCache>,
    watcher: WatcherRef,
    latency_tracker: Arc<LatencyTracker>,
}

impl ClientHandler {
    /// Create a new `ClientHandler` with required dependencies
    #[must_use]
    pub const fn new(
        render: RenderDeps,
        git_cache: Arc<GitStatusCache>,
        watcher: WatcherRef,
        latency_tracker: Arc<LatencyTracker>,
    ) -> Self {
        Self {
            render,
            git_cache,
            watcher,
            latency_tracker,
        }
    }

    /// Handle client registration with PID and optional workspace path validation
    ///
    /// Note: PID validation is performed by server.rs before routing to this handler
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace path validation fails.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "signature must match the RequestHandler dispatch shape (Result<Response, HandlerError>) even though this arm currently never returns Err"
    )]
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

        // Workspace path is already validated via SafePath
        let sanitized = cwd.map(crate::security::SafePath::into_inner);

        // Register client in registry
        self.render
            .client_registry
            .register(pid.get(), sanitized.clone());

        // Register with watcher if we have a workspace path
        if let Some(ref path) = sanitized {
            // Register with watcher for ongoing updates. This is best-effort:
            // watcher failures (resource exhaustion, unsupported paths) MUST NOT
            // break client registration or prompt initialization. The initial
            // scan below still populates the instant-prompt cache, and the cache
            // TTL drives reconciliation when filesystem events are missed.
            if let Ok(guard) = self.watcher.lock()
                && let Some(watcher) = guard.as_ref()
                && let Err(e) = watcher.register_client(pid.get(), path)
            {
                crate::debug::write_debug_log(
                    "client",
                    &format!("Watcher registration failed for PID={pid} (continuing): {e}"),
                );
            }

            // Trigger initial git scan and cache write for new client
            // This ensures instant-prompt cache exists immediately after registration,
            // regardless of whether we have a watcher for ongoing updates
            self.trigger_initial_scan(path);
        }

        Ok(Response::Ack)
    }

    /// Handle workspace update for a registered client
    ///
    /// # Errors
    ///
    /// Returns an error if the workspace path validation fails or the PID is not registered.
    fn handle_workspace_update(
        &self,
        pid: crate::config::types::ClientPid,
        cwd: &crate::security::SafePath,
    ) -> Result<Response, HandlerError> {
        let path_buf = cwd.as_path();

        // Check if PID is registered and update workspace
        if !self
            .render
            .client_registry
            .update_workspace(pid.get(), path_buf)
        {
            return Err(HandlerError::PidNotRegistered(pid.get()));
        }

        // Update watcher for new directory. Best-effort: a watcher failure must
        // not fail the update or leave the registry (already updated above) and
        // the watcher disagreeing in a way that breaks the prompt. On failure we
        // fall back to an immediate scan so the new workspace's cache is fresh,
        // and the cache TTL drives ongoing reconciliation.
        if let Ok(guard) = self.watcher.lock()
            && let Some(watcher) = guard.clone()
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

    /// Handle client unregistration
    ///
    /// # Errors
    ///
    /// Returns an error if watcher unregistration fails.
    fn handle_unregister_client(
        &self,
        pid: crate::config::types::ClientPid,
    ) -> Result<Response, HandlerError> {
        // Unregister from client registry
        self.render.client_registry.unregister(pid.get());

        // Unregister from watcher
        if let Ok(guard) = self.watcher.lock()
            && let Some(watcher) = guard.as_ref()
        {
            watcher
                .unregister_client(pid.get())
                .map_err(|e| HandlerError::OperationFailed {
                    label: "Watcher unregister",
                    source: e,
                })?;
        }

        Ok(Response::Ack)
    }

    /// Handle agent status query
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but maintains Result for API consistency.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "signature must match the RequestHandler dispatch shape (Result<Response, HandlerError>) even though this arm never returns Err"
    )]
    fn handle_status(&self) -> Result<Response, HandlerError> {
        let watched_repos = self
            .watcher
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|w| w.watched_repo_count()))
            .unwrap_or(0);
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

    /// Handle configuration reload request
    ///
    /// # Errors
    ///
    /// Returns an error if configuration reload fails.
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

    /// Handle latency statistics query
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but maintains Result for API consistency.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "signature must match the RequestHandler dispatch shape (Result<Response, HandlerError>) even though this arm never returns Err"
    )]
    fn handle_latency_stats(&self) -> Result<Response, HandlerError> {
        let (min_ms, max_ms, avg_ms, sample_count) = self.latency_tracker.stats();
        Ok(Response::LatencyStatsResult {
            min_ms,
            max_ms,
            avg_ms,
            sample_count,
        })
    }

    /// Handle ping request
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but maintains Result for API consistency.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "signature must match the RequestHandler dispatch shape (Result<Response, HandlerError>) even though this arm never returns Err"
    )]
    #[expect(
        clippy::unused_self,
        reason = "&self is required by the RequestHandler dispatch shape even though this arm needs no handler state"
    )]
    const fn handle_ping(&self) -> Result<Response, HandlerError> {
        Ok(Response::Ack)
    }

    /// Handle shutdown request
    ///
    /// # Errors
    ///
    /// This function currently never returns an error but maintains Result for API consistency.
    #[expect(
        clippy::unnecessary_wraps,
        reason = "signature must match the RequestHandler dispatch shape (Result<Response, HandlerError>) even though this arm never returns Err"
    )]
    #[expect(
        clippy::unused_self,
        reason = "&self is required by the RequestHandler dispatch shape even though this arm needs no handler state"
    )]
    const fn handle_shutdown(&self) -> Result<Response, HandlerError> {
        Ok(Response::Ack)
    }

    /// Trigger initial git scan for a newly registered client
    ///
    /// This ensures the instant-prompt cache is populated immediately after registration,
    /// rather than waiting for the first file change or IPC request.
    fn trigger_initial_scan(&self, path: &std::path::Path) {
        use crate::git::status::load_repository_state;
        use crate::watcher::multi_repo::MultiRepoWatcher;

        // Find git root for this path
        let Some(git_root) = MultiRepoWatcher::find_git_root(path) else {
            return; // Not a git repository, nothing to scan
        };

        // Honor `git.enabled` / `git.skip_paths` exactly like the IPC git
        // request: a skipped repo must not get a status computed or an
        // instant-prompt cache written for it (#696).
        let skipped = {
            let current = self.render.config_manager.get();
            !current.git.enabled || current.git.is_path_skipped(&git_root)
        };
        if skipped {
            return;
        }

        // Spawn background task to avoid blocking registration
        let git_cache = Arc::clone(&self.git_cache);
        let render = self.render.clone();
        let scan_path = git_root.clone();

        let job = move || {
            let config = render.config_manager.get();

            // Load git status
            match load_repository_state(
                &scan_path,
                None,
                config.git.max_ahead_behind,
                config.git.stash_enabled,
            ) {
                Ok(Some(CompleteStatus { status, files })) => {
                    // Update git cache. `git_root` is already canonical (from
                    // find_git_root), so skip the redundant canonicalize.
                    git_cache.set_canonical(&git_root, status.clone(), files);

                    // Write the instant-prompt cache and repaint live shells if
                    // the render changed, exactly as the git handler's request
                    // and background paths do (#587) -- this path used to write
                    // the cache and then silently skip the repaint, stranding a
                    // freshly-scanned status in shells already showing an older
                    // prompt for this repo.
                    //
                    // Registration path: no per-request color context, so prev_bg is None.
                    publish_and_repaint(&render, &git_root, &status, None);
                }
                Ok(None) => {
                    // Not a git repository (shouldn't happen since we found git_root above)
                }
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
            Message::LatencyStats => self.handle_latency_stats(),
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
        let config_manager = Arc::new(ConfigManager::with_defaults().expect("config"));
        config_manager.overwrite_for_tests(Config {
            git: GitSettings {
                skip_paths,
                ..GitSettings::default()
            },
            ..Config::default()
        });
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
            Arc::new(LatencyTracker::new(16)),
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

    /// #696: registering inside a skipped repo (entry written through a
    /// symlink) must write no git instant-prompt cache files.
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

        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(git_cache_files(&cache_dir), Vec::<String>::new());
    }

    /// Control: the same registration without a skip entry does populate the
    /// cache, so the assertion above is not vacuous.
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

//! IPC handlers for client registration, workspace updates, and agent control.

use super::{GitHandler, HandlerError, RenderDeps};
use crate::ipc::Response;
use crate::sync_ext::MutexExt;
use crate::watcher::multi_repo::MultiRepoWatcher;
use std::sync::{Arc, Mutex};

/// Shared, lazily-set watcher handle.
type WatcherRef = Arc<Mutex<Option<Arc<MultiRepoWatcher>>>>;

/// Handler for client lifecycle, status and config reload.
pub struct ClientHandler {
    render: RenderDeps,
    /// Runs the registration scan; see [`GitHandler::prewarm`].
    git: Arc<GitHandler>,
    watcher: WatcherRef,
}

impl ClientHandler {
    /// Create a new `ClientHandler`.
    #[must_use]
    pub const fn new(render: RenderDeps, git: Arc<GitHandler>, watcher: WatcherRef) -> Self {
        Self {
            render,
            git,
            watcher,
        }
    }

    /// Register a client (PID already validated by the server).
    #[must_use]
    pub fn register(
        &self,
        pid: crate::ipc::ClientPid,
        cwd: Option<crate::security::SafePath>,
        shell: Option<crate::shell::Shell>,
        shell_version: Option<&String>,
    ) -> Response {
        let shell_info = match (&shell, &shell_version) {
            (Some(s), Some(v)) => format!("{s} {v}"),
            (Some(s), None) => format!("{s}"),
            _ => "unknown".to_owned(),
        };

        crate::debug_log!(
            "client",
            "Registering client: PID={pid}, shell={shell_info}"
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
                && let Err(e) = watcher.attach_client(pid.get(), path)
            {
                crate::debug_log!(
                    "client",
                    "Watcher registration failed for PID={pid} (continuing): {e}"
                );
            }

            // Populate the instant-prompt cache immediately, watcher or not.
            self.git.prewarm(path);
        }

        Response::Ack
    }

    /// Update workspace information for a registered client.
    ///
    /// # Errors
    /// Fails if path validation fails or the PID is not registered.
    pub(crate) fn workspace_update(
        &self,
        pid: crate::ipc::ClientPid,
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
            && let Err(e) = watcher.attach_client(pid.get(), path_buf)
        {
            crate::debug_log!(
                "client",
                "Watcher update failed for PID={pid} (continuing): {e}"
            );
            self.git.prewarm(path_buf);
        }

        Ok(Response::Ack)
    }

    /// Unregister a client.
    pub(crate) fn unregister(&self, pid: crate::ipc::ClientPid) -> Response {
        self.render.client_registry.unregister(pid.get());
        let current_watcher = self.watcher.lock_or_recover().clone();
        if let Some(watcher) = current_watcher {
            watcher.unregister_client(pid.get());
        }

        Response::Ack
    }

    /// Answer the agent status query.
    pub(crate) fn status(&self) -> Response {
        let watched_repos = self
            .watcher
            .lock_or_recover()
            .as_ref()
            .map_or_default(|w| w.watched_repo_count());
        let registered_clients = self.render.client_registry.len();
        let cache_entries = self.git.cached_repo_count();

        Response::AgentStatus(crate::ipc::AgentStatus {
            version: crate::VERSION.to_owned(),
            protocol_version: crate::ipc::protocol::PROTOCOL_VERSION,
            watched_repos,
            registered_clients,
            cache_entries,
            pid: Some(std::process::id()),
        })
    }

    /// Reload configuration from disk.
    ///
    /// # Errors
    /// Fails if configuration reload fails.
    pub(crate) fn config_reload(&self) -> Result<Response, HandlerError> {
        self.render
            .config_manager
            .reload_now()
            .map_err(|e| HandlerError::OperationFailed {
                label: "Config reload",
                source: e,
            })?;
        Ok(Response::Ack)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cache::{CachePolicy, InstantPromptCache};
    use crate::config::manager::ConfigManager;
    use crate::config::{Config, GitSettings};
    use crate::git::cache::GitStatusCache;
    use crate::git::{RepositoryState, RepositoryStatus};
    use crate::ipc::fixtures::render_deps;
    use std::collections::HashMap;
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
        handler_with_cache(skip_paths, cache_dir, Arc::new(GitStatusCache::new()))
    }

    fn handler_with_cache(
        skip_paths: Vec<String>,
        cache_dir: &Path,
        git_cache: Arc<GitStatusCache>,
    ) -> ClientHandler {
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
            instant_cache: Arc::new(
                InstantPromptCache::new_in_dir(cache_dir.to_path_buf()).expect("cache"),
            ),
            ..render_deps(config_manager)
        };
        let git = Arc::new(GitHandler::new(render.clone(), git_cache));
        ClientHandler::new(render, git, Arc::new(Mutex::new(None)))
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
        let pid = crate::ipc::ClientPid::new(std::process::id()).expect("pid");
        let safe = crate::security::SafePath::new(cwd.to_str().expect("utf8")).expect("safe");
        let _ack = handler.register(pid, Some(safe), None, None);
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

        // Absence window: a skipped repo starts no job to wait on, so allow a
        // real scan's latency (1.5 s) to elapse before checking nothing was written.
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

    /// A status still within its cooldown is not rescanned on registration,
    /// so shells re-registering together after an agent restart scan once.
    #[test]
    fn register_skips_scan_while_cached_status_is_fresh() {
        let tmp = tempfile::TempDir::new().expect("tmp");
        let repo = init_repo(&tmp.path().join("repo"));
        let cache_dir = tmp.path().join("cache");
        let git_cache = Arc::new(GitStatusCache::with_policy(CachePolicy {
            ttl: Duration::from_secs(30),
            cooldown: Duration::from_hours(1),
        }));
        let cached = RepositoryStatus {
            branch: "cached-branch".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked: 0,
            conflicts: 0,
            state: RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            stale_error: false,
            rebase_progress: None,
        };
        git_cache.set_canonical(&repo, cached, HashMap::new());

        let handler = handler_with_cache(Vec::new(), &cache_dir, Arc::clone(&git_cache));
        register(&handler, &repo);

        // Absence window, as in the skipped-repo test: a real scan would have
        // replaced the branch and written instant-cache files by now.
        std::thread::sleep(Duration::from_millis(1500));
        assert_eq!(
            git_cache
                .get_any_canonical(&repo)
                .map(|status| status.branch),
            Some("cached-branch".to_owned())
        );
        assert_eq!(git_cache_files(&cache_dir), Vec::<String>::new());
    }
}

//! Runtime handle for the IPC server: binds the Unix socket, accepts
//! connections, enforces guard settings, and dispatches messages.

use super::builder::{EndpointDeps, EndpointHandleBuilder};
#[cfg(unix)]
use super::connection::ConnectionHandler;
#[cfg(unix)]
use crate::debug_log;
#[cfg(unix)]
use crate::ipc::Response;
use crate::ipc::{
    ClientDirectory,
    handlers::{
        ClientHandler, GitHandler, HandlerRegistry, LanguageHandler, MiscHandler, RenderDeps,
        ThemeHandler,
    },
};
use crate::palette::PaletteCache;
use crate::security::{GuardSettings, RateLimiter};
use crate::theme::ThemeManager;
#[cfg(unix)]
use crate::warn_log;
use crate::{Error, Result};
#[cfg(unix)]
use std::path::Path;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, broadcast};
#[cfg(unix)]
use tokio::time::timeout;

/// Whole-connection deadline = this × `request_timeout()`. Clients send one
/// line per connection; the cap stops a trickling client holding a permit forever.
#[cfg(unix)]
const CONNECTION_DEADLINE_MULTIPLIER: u32 = 4;

/// Shared slot for the multi-repo watcher; the `Option` lets it be taken out or replaced.
pub type WatcherRef = Arc<Mutex<Option<Arc<crate::watcher::multi_repo::MultiRepoWatcher>>>>;

/// Shared subsystem handles the IPC endpoint needs for its whole lifetime.
pub(crate) struct ServerDeps {
    /// Registered shell clients, used to target repaint signals.
    pub client_registry: Arc<ClientDirectory>,
    /// Shared git status cache backing both the request path and the watcher.
    pub git_cache: Arc<crate::git::cache::GitStatusCache>,
    /// Live configuration, re-read per request so a hot reload takes effect.
    pub config_manager: Arc<crate::config::manager::ConfigManager>,
    /// Active theme, shared rather than reloaded from disk per render.
    pub theme_manager: Arc<ThemeManager>,
    /// On-disk cache the shell reads to render a prompt without any IPC.
    pub instant_cache: Arc<crate::cache::InstantPromptCache>,
    /// Rolling request-latency samples reported by `Message::LatencyStats`.
    pub latency_tracker: Arc<crate::ipc::LatencyTracker>,
    /// Per-repo language detection results shared with the agent event loop.
    pub language_cache: crate::language::DetectionCache,
    /// Cached active palette, so a reload is picked up without re-parsing per render.
    pub palette_cache: Arc<PaletteCache>,
}

impl ServerDeps {
    /// The subset of these handles every publish-and-repaint site needs.
    fn render(&self) -> RenderDeps {
        RenderDeps {
            config_manager: Arc::clone(&self.config_manager),
            theme_manager: Arc::clone(&self.theme_manager),
            palette_cache: Arc::clone(&self.palette_cache),
            instant_cache: Arc::clone(&self.instant_cache),
            client_registry: Arc::clone(&self.client_registry),
        }
    }
}

/// Whether an `accept()` error is transient (`EMFILE`/`ENFILE`/`EINTR`/`ECONNABORTED`);
/// all others are fatal.
#[cfg(unix)]
fn accept_error_is_transient(err: &std::io::Error) -> bool {
    use std::io::ErrorKind;
    if matches!(
        err.kind(),
        ErrorKind::Interrupted | ErrorKind::ConnectionAborted
    ) {
        return true;
    }
    // EMFILE/ENFILE have no stable `ErrorKind`, so match on the raw errno.
    accept_error_is_fd_exhaustion(err)
}

/// Whether `err` is fd exhaustion (`EMFILE`/`ENFILE`), which warrants a backoff.
#[cfg(unix)]
fn accept_error_is_fd_exhaustion(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(libc::EMFILE | libc::ENFILE))
}

/// What [`secure_socket_dir`] does with a directory we own that is group- or
/// world-writable. Decided by where the socket path came from.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DirRepair {
    /// GPY's own runtime directory: set it to `0700` and use it.
    Tighten,
    /// A directory the user chose (`GPY_AGENT_SOCKET_PATH`, `--socket`): refuse
    /// it rather than change the mode of a directory GPY does not own.
    Refuse,
}

/// Make the directory `socket_path` is published in safe for it. Socket mode
/// `0600` does not stop anyone who can write the directory from replacing the
/// socket, so refuse a symlink, a non-directory, or a directory owned by
/// another user; one we own that is group/world-writable is handled by `repair`.
///
/// # Errors
/// Fails if the directory cannot be opened or fails a trust check.
#[cfg(unix)]
pub(crate) fn secure_socket_dir(socket_path: &Path, repair: DirRepair) -> Result<()> {
    let dir = socket_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    secure_dir_for_uid(dir, nix::unistd::getuid().as_raw(), repair)
}

/// [`secure_socket_dir`] against an explicit owner `uid`.
///
/// # Errors
/// Fails if `dir` cannot be opened or fails a trust check.
#[cfg(unix)]
fn secure_dir_for_uid(dir: &Path, uid: u32, repair: DirRepair) -> Result<()> {
    use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};

    let refuse = |problem: &str| {
        Error::ipc(format!(
            "refusing socket directory {}: {problem}. Use a directory you own that \
             only you can write (via XDG_RUNTIME_DIR or GPY_AGENT_SOCKET_PATH)",
            dir.display()
        ))
    };
    let unreadable = |e: std::io::Error| {
        Error::ipc(format!(
            "cannot inspect socket directory {}: {e}",
            dir.display()
        ))
    };

    // O_NOFOLLOW fails on a final-component symlink instead of following it,
    // and every check below acts on this one inode (fstat/fchmod), so the
    // path cannot be swapped between check and repair.
    let handle = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_DIRECTORY | libc::O_NOFOLLOW)
        .open(dir)
        .map_err(|e| match e.raw_os_error() {
            Some(libc::ELOOP | libc::ENOTDIR) if dir.is_symlink() => refuse("it is a symlink"),
            Some(libc::ENOTDIR) => refuse("it is not a directory"),
            _ => unreadable(e),
        })?;
    let meta = handle.metadata().map_err(unreadable)?;
    if meta.uid() != uid {
        return Err(refuse(&format!(
            "it is owned by uid {}, not the current user (uid {uid})",
            meta.uid()
        )));
    }
    if meta.mode() & 0o022 == 0 {
        return Ok(());
    }
    if repair == DirRepair::Refuse {
        return Err(refuse(&format!(
            "it is group/world-writable (mode {:o}); run `chmod 700 {}`",
            meta.mode() & 0o7777,
            dir.display()
        )));
    }

    handle
        .set_permissions(std::fs::Permissions::from_mode(0o700))
        .map_err(|e| {
            refuse(&format!(
                "it is group/world-writable and chmod 0700 failed: {e}"
            ))
        })?;
    let mode = handle.metadata().map_err(unreadable)?.mode();
    if mode & 0o022 != 0 {
        return Err(refuse(&format!(
            "it is still group/world-writable (mode {:o}) after chmod 0700",
            mode & 0o7777
        )));
    }
    warn_log!(
        "server",
        "Socket directory {} was group/world-writable; set it to 0700",
        dir.display()
    );
    Ok(())
}

#[cfg(unix)]
use crate::ipc::socket_identity::SocketIdentity;

/// Accept-loop socket-ownership check period; long enough that a normal
/// restart's briefly missing socket never makes a healthy agent exit.
#[cfg(unix)]
pub(crate) const SOCKET_OWNERSHIP_CHECK: Duration = Duration::from_secs(30);

/// Outcome of one ownership check. Only `Lost` acts.
#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum SocketOwnership {
    /// The path holds the socket this handle bound (or it never bound one).
    Owned,
    /// The path is gone or holds a different socket.
    Lost,
    /// The path could not be inspected (EACCES, EIO, ...): not evidence.
    Unknown,
}

/// IPC server handle used by the agent to serve client requests
#[cfg_attr(
    not(unix),
    expect(
        dead_code,
        reason = "the accept/serve path that reads these fields is Unix-only"
    )
)]
pub struct EndpointHandle {
    pub(crate) socket_path: PathBuf,
    #[cfg(unix)]
    listener: Option<UnixListener>,
    /// Identity of the socket this handle bound; `None` means it never unlinks anything.
    #[cfg(unix)]
    bound_socket: Option<SocketIdentity>,
    /// Inherited per-socket start lock, held until [`Self::bind_socket`] returns so a
    /// concurrent start cannot evict or fork between our fork and bind.
    #[cfg(unix)]
    start_lock: Option<nix::fcntl::Flock<std::fs::File>>,
    /// Ownership check period ([`SOCKET_OWNERSHIP_CHECK`] outside tests).
    #[cfg(unix)]
    pub(super) socket_ownership_check: Duration,
    security_config: GuardSettings,
    rate_limiter: RateLimiter,
    client_registry: Arc<ClientDirectory>,
    config_manager: Arc<crate::config::manager::ConfigManager>,
    /// How [`Self::bind_socket`] treats a socket directory we own that others
    /// can write; the builder's caller knows whether GPY owns the directory.
    #[cfg(unix)]
    pub(super) socket_dir_repair: DirRepair,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only a test reads it back, to assert the builder keeps the shared slot"
        )
    )]
    pub(crate) watcher: WatcherRef,
    theme_manager: Arc<ThemeManager>,
    latency_tracker: Arc<crate::ipc::LatencyTracker>,
    palette_cache: Arc<PaletteCache>,
    connection_semaphore: Arc<Semaphore>,
    shutdown_tx: broadcast::Sender<()>, // Used to signal shutdown to all listeners
    // Keeps `shutdown_tx.send(())` from erroring for lack of receivers.
    _shutdown_rx: broadcast::Receiver<()>,
    pub(crate) handler_registry: Arc<HandlerRegistry>,
}

/// Content hash of a live-update response for the repaint throttle, or `None`
/// if serialization fails. Payloads carry no timestamps, so equal state coalesces.
#[cfg(unix)]
fn content_token(response: &Response) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    let bytes = serde_json::to_vec(response).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(hasher.finish())
}

/// Signal live-update subscribers when a response affects git/language state
/// and live updates are enabled.
#[cfg(unix)]
pub(super) fn notify_clients_for_response(
    config_manager: &crate::config::manager::ConfigManager,
    client_registry: &Arc<ClientDirectory>,
    response: &Response,
    notify_path: Option<&str>,
) {
    let config = config_manager.get();
    if !config.agent.live_updates {
        return;
    }

    if matches!(
        response,
        Response::RepositoryStatus(_) | Response::Language { .. }
    ) {
        match notify_path {
            // Content-derived token: duplicates coalesce, real changes get through.
            Some(path) => {
                let target = std::path::Path::new(path);
                match content_token(response) {
                    Some(token) => client_registry.notify_repaint_coalesced(Some(target), token),
                    None => client_registry.notify_repaint(Some(target)),
                }
            }
            None => client_registry.notify_repaint(None),
        }
    }
}

impl EndpointHandle {
    #[cfg(unix)]
    fn request_timeout(&self) -> Duration {
        let config = self.config_manager.get();
        Duration::from_secs(config.agent.timeout_seconds.get())
    }

    /// Hard cap on total connection lifetime (see [`CONNECTION_DEADLINE_MULTIPLIER`]).
    #[cfg(unix)]
    fn connection_deadline(&self) -> Duration {
        self.request_timeout()
            .saturating_mul(CONNECTION_DEADLINE_MULTIPLIER)
    }

    pub(crate) fn with_path_and_state(
        socket_path: PathBuf,
        deps: ServerDeps,
        watcher: WatcherRef,
        security_config: GuardSettings,
    ) -> Self {
        let rate_limiter = RateLimiter::new(security_config.max_connections_per_second);
        let connection_semaphore =
            Arc::new(Semaphore::new(security_config.max_concurrent_connections));

        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let render = deps.render();
        let git_handler = Arc::new(GitHandler::new(render.clone(), Arc::clone(&deps.git_cache)));
        let language_handler = Arc::new(LanguageHandler::new(
            render.clone(),
            deps.language_cache.clone(),
        ));
        let theme_handler = Arc::new(ThemeHandler::new(Arc::clone(&deps.theme_manager)));
        let client_handler = Arc::new(ClientHandler::new(
            render,
            Arc::clone(&deps.git_cache),
            Arc::clone(&watcher),
            Arc::clone(&deps.latency_tracker),
        ));
        let misc_handler = Arc::new(MiscHandler::new());
        let handler_registry = Arc::new(HandlerRegistry::new(
            git_handler,
            language_handler,
            theme_handler,
            client_handler,
            misc_handler,
        ));

        Self {
            socket_path,
            #[cfg(unix)]
            listener: None,
            #[cfg(unix)]
            bound_socket: None,
            #[cfg(unix)]
            start_lock: None,
            #[cfg(unix)]
            socket_ownership_check: SOCKET_OWNERSHIP_CHECK,
            security_config,
            rate_limiter,
            client_registry: deps.client_registry,
            config_manager: deps.config_manager,
            #[cfg(unix)]
            socket_dir_repair: DirRepair::Refuse,
            watcher,
            theme_manager: deps.theme_manager,
            latency_tracker: deps.latency_tracker,
            palette_cache: deps.palette_cache,
            connection_semaphore,
            shutdown_tx,
            _shutdown_rx: shutdown_rx,
            handler_registry,
        }
    }

    /// Create a builder for an `EndpointHandle` from its required [`EndpointDeps`].
    #[must_use]
    pub fn builder(deps: EndpointDeps) -> EndpointHandleBuilder {
        EndpointHandleBuilder::new(deps)
    }

    /// Hold the per-socket start lock until this handle's socket is bound
    /// (see the `start_lock` field).
    #[cfg(unix)]
    pub(crate) fn hold_start_lock_until_bound(&mut self, lock: nix::fcntl::Flock<std::fs::File>) {
        self.start_lock = Some(lock);
    }

    /// Start accepting client connections.
    ///
    /// # Errors
    /// Fails if the socket cannot be bound or accepting fails.
    #[cfg(unix)]
    pub async fn start(&mut self) -> Result<()> {
        use crate::debug_log;

        let bound = self.bind_socket();
        // Release the start lock regardless, so the next start sees the real outcome.
        self.start_lock = None;
        bound?;
        debug_log!("server", "Listener stored, starting accept loop...");

        debug_log!("server", "About to enter accept_loop");
        let result = self.accept_loop().await;
        debug_log!("server", "accept_loop returned: {:?}", result.is_ok());
        result
    }

    /// Bind the socket in a trusted directory (see [`secure_socket_dir`]), set
    /// 0600 permissions, and record its identity so cleanup never removes a
    /// socket another agent rebound.
    ///
    /// # Errors
    /// Fails if the directory is untrusted, or binding, reading metadata, or
    /// setting permissions fails.
    #[cfg(unix)]
    fn bind_socket(&mut self) -> Result<()> {
        use crate::debug_log;
        use std::os::unix::fs::PermissionsExt;

        // Explain over-long paths instead of the kernel's bare SUN_LEN error.
        crate::paths::check_socket_path_len(&self.socket_path)?;
        secure_socket_dir(&self.socket_path, self.socket_dir_repair)?;

        debug_log!("server", "Attempting to bind socket...");
        let listener = UnixListener::bind(&self.socket_path).map_err(|e| {
            let socket_path = self.socket_path.display();
            let error_msg = format!("Failed to bind socket '{socket_path}': {e}");
            debug_log!("server", "{}", error_msg);
            Error::ipc(error_msg)
        })?;
        debug_log!("server", "Socket bound successfully");

        debug_log!("server", "Setting socket permissions...");
        let metadata = std::fs::metadata(&self.socket_path).map_err(|e| {
            let error_msg = format!("Failed to read socket metadata: {e}");
            debug_log!("server", "{}", error_msg);
            Error::ipc(error_msg)
        })?;

        let mut perms = metadata.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&self.socket_path, perms).map_err(|e| {
            let error_msg = format!("Failed to set socket permissions: {e}");
            debug_log!("server", "{}", error_msg);
            Error::ipc(error_msg)
        })?;
        debug_log!("server", "Socket permissions set successfully");

        // Record after chmod: it changes ctime, so a pre-chmod identity would never match.
        let bound_metadata = std::fs::metadata(&self.socket_path).map_err(|e| {
            let error_msg = format!("Failed to re-read socket metadata: {e}");
            debug_log!("server", "{}", error_msg);
            Error::ipc(error_msg)
        })?;
        self.bound_socket = Some(SocketIdentity::from_metadata(&bound_metadata));

        self.listener = Some(listener);
        Ok(())
    }

    /// Main connection acceptance loop.
    ///
    /// # Errors
    /// Fails if there is no listener or accepting fails fatally.
    #[cfg(unix)]
    async fn accept_loop(&mut self) -> Result<()> {
        use crate::debug_log;

        debug_log!("server", "Starting accept loop");
        let listener = self
            .listener
            .as_ref()
            .ok_or_else(|| Error::ipc("Server not started - no listener available".to_owned()))?;

        let mut shutdown_rx = self.shutdown_tx.subscribe();

        // A socket removed or rebound by another process makes us unreachable: exit like `Shutdown`.
        let mut ownership_timer = tokio::time::interval(self.socket_ownership_check);
        ownership_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ownership_timer.reset();

        debug_log!("server", "Got listener reference, entering main loop");
        loop {
            debug_log!("server", "Waiting for connection...");

            tokio::select! {
                result = listener.accept() => {
                    let stream = match result {
                        Ok((stream, _)) => stream,
                        Err(e) if accept_error_is_transient(&e) => {
                            // Transient errors must not tear down the agent.
                            warn_log!("server", "Transient accept() error, continuing to serve: {e}");
                            debug_log!("server", "Transient accept() error: {}", e);
                            // Back off on fd exhaustion to avoid a hot spin.
                            if accept_error_is_fd_exhaustion(&e) {
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                            continue;
                        }
                        Err(e) => {
                            let error_msg = format!("Failed to accept connection: {e}");
                            debug_log!("server", "{}", error_msg);
                            // Fatal errors skip the loop exit below, so clean up here.
                            self.cleanup_socket_file();
                            return Err(Error::ipc(error_msg));
                        }
                    };
                    debug_log!("server", "Connection accepted");

                    if !self.rate_limiter.allow_connection() {
                        warn_log!("server", "Rate limit exceeded, rejecting connection");
                        continue; // Implicit drop
                    }

                    self.spawn_client_handler(stream);
                }
                _ = shutdown_rx.recv() => {
                    debug_log!("server", "Received shutdown signal, exiting accept loop");
                    break;
                }
                _ = ownership_timer.tick() => {
                    if self.socket_ownership() == SocketOwnership::Lost {
                        warn_log!(
                            "server",
                            "Socket {} no longer belongs to this agent (removed or rebound by another process); shutting down",
                            self.socket_path.display()
                        );
                        break;
                    }
                }
            }
        }

        debug_log!("server", "Accept loop finished, cleaning up socket");
        self.cleanup_socket_file();

        Ok(())
    }

    /// Best-effort, ownership-checked removal of the socket file on accept-loop exit.
    #[cfg(unix)]
    fn cleanup_socket_file(&self) {
        self.unlink_socket_if_owned();
    }

    /// Whether the path still holds the socket this handle bound (dev+ino+ctime).
    #[cfg(unix)]
    fn socket_ownership(&self) -> SocketOwnership {
        let Some(bound_socket) = self.bound_socket else {
            return SocketOwnership::Owned;
        };
        match SocketIdentity::at(&self.socket_path) {
            Ok(current) if current == bound_socket => SocketOwnership::Owned,
            Ok(_) => SocketOwnership::Lost,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => SocketOwnership::Lost,
            Err(_) => SocketOwnership::Unknown,
        }
    }

    /// Remove the socket file only if it is still the one this handle bound; after a
    /// restart race it may belong to a new agent.
    #[cfg(unix)]
    fn unlink_socket_if_owned(&self) {
        let Some(bound_socket) = self.bound_socket else {
            return; // Never bound a socket -> nothing of ours to remove.
        };

        let current_socket = match std::fs::metadata(&self.socket_path) {
            Ok(metadata) => SocketIdentity::from_metadata(&metadata),
            Err(_) => return,
        };

        if current_socket != bound_socket {
            debug_log!(
                "server",
                "Socket at path was rebound by another process; skipping unlink"
            );
            return;
        }

        if let Err(e) = std::fs::remove_file(&self.socket_path) {
            debug_log!("server", "Failed to remove owned socket file: {}", e);
        } else {
            debug_log!("server", "Socket file removed");
        }
    }

    /// Spawn a background task to handle a client connection.
    #[cfg(unix)]
    fn spawn_client_handler(&self, stream: UnixStream) {
        // Computed before fields are moved into the task.
        let connection_deadline = self.connection_deadline();
        let client_registry = Arc::clone(&self.client_registry);
        let semaphore = Arc::clone(&self.connection_semaphore);

        // Only per-connection dependencies travel into the task.
        let connection_handler = ConnectionHandler {
            security_config: self.security_config.clone(),
            config_manager: Arc::clone(&self.config_manager),
            theme_manager: Arc::clone(&self.theme_manager),
            palette_cache: Arc::clone(&self.palette_cache),
            latency_tracker: Arc::clone(&self.latency_tracker),
            shutdown_tx: self.shutdown_tx.clone(),
            handler_registry: Arc::clone(&self.handler_registry),
        };

        tokio::spawn(async move {
            // Fail fast at the connection limit: queueing would let slow-loris
            // clients starve legitimate requests.
            let Ok(_permit) = semaphore.try_acquire() else {
                Self::reject_connection_busy(stream);
                return;
            };

            // Per-read timeouts don't bound a trickling client; this caps total
            // lifetime. Elapse drops `stream` and `_permit`, closing and releasing.
            match timeout(
                connection_deadline,
                connection_handler.handle_client(stream, client_registry),
            )
            .await
            {
                Ok(Ok(())) => {}
                Ok(Err(e)) => warn_log!("server", "Client handling error: {e}"),
                Err(_) => warn_log!(
                    "server",
                    "Connection exceeded whole-connection deadline of {}ms, closing",
                    connection_deadline.as_millis()
                ),
            }
        });
    }

    /// Close without replying when the connection semaphore is saturated. The
    /// request format is unknown, and shells treat no reply as an unreachable agent.
    #[cfg(unix)]
    fn reject_connection_busy(stream: UnixStream) {
        debug_log!(
            "server",
            "Rejecting connection: too many concurrent connections"
        );
        drop(stream);
    }

    /// Notify live-update subscribers about a response (test-only; production
    /// uses `ConnectionHandler::notify_clients`).
    #[cfg(all(test, unix))]
    pub(crate) fn notify_clients(
        &self,
        client_registry: &Arc<ClientDirectory>,
        response: &Response,
        notify_path: Option<&str>,
    ) {
        notify_clients_for_response(&self.config_manager, client_registry, response, notify_path);
    }

    /// Stop the server. On Unix the socket is removed only if still owned.
    ///
    /// # Errors
    /// Fails if the socket file cannot be removed (non-Unix only).
    #[cfg_attr(
        unix,
        expect(
            clippy::unnecessary_wraps,
            reason = "non-Unix path can fail; Unix body is always Ok"
        )
    )]
    pub fn stop(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            // Stop accepting, then remove the socket only if still ours.
            self.listener.take();
            self.unlink_socket_if_owned();
        }

        #[cfg(not(unix))]
        {
            if self.socket_path.exists() {
                std::fs::remove_file(&self.socket_path)
                    .map_err(|e| Error::ipc(format!("Failed to remove socket file: {e}")))?;
            }
        }

        Ok(())
    }

    /// Get the socket path being used.
    pub fn socket_path(&self) -> &str {
        self.socket_path.to_str().unwrap_or("")
    }

    /// Create an unbound `EndpointHandle` for tests.
    #[doc(hidden)]
    #[expect(
        clippy::expect_used,
        reason = "helper for tests, panic is acceptable on setup failure"
    )]
    pub fn new_test_handle(
        client_registry: Arc<ClientDirectory>,
        git_cache: &Arc<crate::git::cache::GitStatusCache>,
        watcher: WatcherRef,
    ) -> Self {
        // Built-in defaults, never the developer's ~/.config/gpy.
        let theme_manager =
            Arc::new(ThemeManager::builtin("default").expect("default theme should always load"));
        let config_manager = Arc::new(
            crate::config::manager::ConfigManager::with_defaults()
                .expect("default config should always load"),
        );

        let instant_cache = Arc::new(crate::cache::InstantPromptCache::new_for_test());

        let latency_tracker = Arc::new(crate::ipc::LatencyTracker::new(100_usize));

        let language_cache = crate::language::DetectionCache::new();

        let initial_config = config_manager.get();
        let palette_cache = Arc::new(PaletteCache::from_config(&initial_config));

        Self::with_path_and_state(
            PathBuf::from("/tmp/test.sock"),
            ServerDeps {
                client_registry,
                git_cache: Arc::clone(git_cache),
                config_manager,
                theme_manager,
                instant_cache,
                latency_tracker,
                language_cache,
                palette_cache,
            },
            watcher,
            GuardSettings::default(),
        )
    }
}

/// Native Windows stub: the IPC server is Unix-socket based; use WSL.
#[cfg(not(unix))]
impl EndpointHandle {
    /// Start accepting client connections (unsupported on native Windows).
    ///
    /// # Errors
    /// Always fails on this platform.
    pub async fn start(&mut self) -> Result<()> {
        Err(crate::ipc::native_windows_unsupported())
    }
}

#[cfg(all(test, unix))]
mod content_token_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::content_token;
    use crate::git::{RepositoryState, RepositoryStatus};
    use crate::ipc::Response;

    fn status_response(untracked: u32) -> Response {
        Response::RepositoryStatus(RepositoryStatus {
            branch: "main".to_owned(),
            ahead: 0,
            behind: 0,
            ahead_capped: false,
            behind_capped: false,
            staged: 0,
            unstaged: 0,
            untracked,
            conflicts: 0,
            state: RepositoryState::Clean,
            stash_count: 0,
            detached: false,
            rebase_progress: None,
        })
    }

    #[test]
    fn identical_responses_hash_to_the_same_token() {
        let token_a = content_token(&status_response(0));
        let token_b = content_token(&status_response(0));
        assert!(token_a.is_some(), "a hashable response must yield a token");
        assert_eq!(
            token_a, token_b,
            "identical repository state must coalesce (same token)"
        );
    }

    #[test]
    fn distinct_responses_hash_to_distinct_tokens() {
        let clean = content_token(&status_response(0));
        let dirty = content_token(&status_response(3));
        assert!(clean.is_some() && dirty.is_some());
        assert_ne!(
            clean, dirty,
            "a different repository state must produce a distinct token (latest-wins)"
        );
    }
}

#[cfg(all(test, unix))]
mod route_prev_bg_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::*;
    use crate::formatter::Format;
    use crate::ipc::Message;
    use std::sync::{Arc, Mutex};

    fn make_handler_registry() -> Arc<HandlerRegistry> {
        let client_registry = Arc::new(crate::ipc::ClientDirectory::new());
        let git_cache = Arc::new(crate::git::cache::GitStatusCache::new());
        let watcher: WatcherRef = Arc::new(Mutex::new(None));
        let handle = EndpointHandle::new_test_handle(client_registry, &git_cache, watcher);
        Arc::clone(&handle.handler_registry)
    }

    #[test]
    fn route_request_secure_threads_prev_bg_through_tuple() {
        let registry = make_handler_registry();
        let security_config = GuardSettings::default();
        let safe_path = crate::security::SafePath::new("/tmp").expect("valid path");
        let msg = Message::RepositoryStatus {
            path: safe_path,
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: Some("blue".to_owned()),
        };
        let result = ConnectionHandler::route_request_secure(&registry, &security_config, &msg);
        assert_eq!(
            result.prev_bg,
            Some("blue".to_owned()),
            "prev_bg must be threaded through the routing result unchanged"
        );
    }

    #[test]
    fn route_request_secure_handles_absent_prev_bg() {
        let registry = make_handler_registry();
        let security_config = GuardSettings::default();
        let safe_path = crate::security::SafePath::new("/tmp").expect("valid path");
        let msg = Message::RepositoryStatus {
            path: safe_path,
            format: Format::Json,
            is_last: false,
            is_first: false,
            prev_bg: None,
        };
        let result = ConnectionHandler::route_request_secure(&registry, &security_config, &msg);
        assert_eq!(
            result.prev_bg, None,
            "absent prev_bg must remain None in the routing result"
        );
    }

    #[test]
    fn handler_registry_routes_hostname_request_to_hostname_response() {
        let registry = make_handler_registry();
        let msg = Message::HostnameRequest {
            hostname: "h".to_owned(),
            format: Format::Json,
            is_last: false,
            is_ssh: true,
            prev_bg: None,
        };
        let response = registry.route(&msg).expect("routing must succeed");
        match response {
            crate::ipc::Response::Hostname { hostname, is_ssh } => {
                assert_eq!(hostname, "h");
                assert!(is_ssh, "the handler must echo the request's is_ssh");
            }
            other => panic!("expected Response::Hostname, got {other:?}"),
        }
    }

    #[test]
    fn route_request_secure_threads_hostname_request_prev_bg_and_is_last() {
        let registry = make_handler_registry();
        let security_config = GuardSettings::default();
        let msg = Message::HostnameRequest {
            hostname: "my-host".to_owned(),
            format: Format::Ansi,
            is_last: true,
            is_ssh: false,
            prev_bg: Some("blue".to_owned()),
        };
        let result = ConnectionHandler::route_request_secure(&registry, &security_config, &msg);

        match result.response {
            crate::ipc::Response::Hostname { hostname, is_ssh } => {
                assert_eq!(hostname, "my-host");
                assert!(!is_ssh);
            }
            other => panic!("expected Response::Hostname, got {other:?}"),
        }
        assert_eq!(result.format, Format::Ansi);
        assert_eq!(result.notify_path, None);
        assert!(result.is_last);
        assert_eq!(result.prev_bg, Some("blue".to_owned()));
    }

    #[test]
    fn handler_registry_routes_username_request_to_username_response() {
        let registry = make_handler_registry();
        let msg = Message::UsernameRequest {
            username: "u".to_owned(),
            format: Format::Json,
            is_last: false,
            prev_bg: None,
        };
        let response = registry.route(&msg).expect("routing must succeed");
        match response {
            crate::ipc::Response::Username { username } => {
                assert_eq!(username, "u");
            }
            other => panic!("expected Response::Username, got {other:?}"),
        }
    }

    #[test]
    fn route_request_secure_threads_username_request_prev_bg_and_is_last() {
        let registry = make_handler_registry();
        let security_config = GuardSettings::default();
        let msg = Message::UsernameRequest {
            username: "my-user".to_owned(),
            format: Format::Ansi,
            is_last: true,
            prev_bg: Some("blue".to_owned()),
        };
        let result = ConnectionHandler::route_request_secure(&registry, &security_config, &msg);

        match result.response {
            crate::ipc::Response::Username { username } => {
                assert_eq!(username, "my-user");
            }
            other => panic!("expected Response::Username, got {other:?}"),
        }
        assert_eq!(result.format, Format::Ansi);
        assert_eq!(result.notify_path, None);
        assert!(result.is_last);
        assert_eq!(result.prev_bg, Some("blue".to_owned()));
    }
}

#[cfg(all(test, unix))]
mod socket_ownership_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{EndpointDeps, EndpointHandle};
    use crate::config::manager::ConfigManager;
    use crate::git::cache::GitStatusCache;
    use crate::ipc::ClientDirectory;
    use crate::ipc::LatencyTracker;
    use crate::theme::ThemeManager;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// Fresh default collaborators for an unbound handle.
    fn test_deps() -> EndpointDeps {
        EndpointDeps {
            client_registry: Arc::new(ClientDirectory::new()),
            git_cache: Arc::new(GitStatusCache::new()),
            config_manager: Arc::new(ConfigManager::with_defaults().expect("default config")),
            theme_manager: Arc::new(ThemeManager::builtin("default").expect("default theme")),
            instant_cache: Arc::new(crate::cache::InstantPromptCache::new_for_test()),
            latency_tracker: Arc::new(LatencyTracker::new(100_usize)),
            language_cache: crate::language::DetectionCache::new(),
        }
    }

    /// Build an unbound `EndpointHandle` targeting `socket_path` (no accept loop).
    pub(super) fn build_handle(socket_path: &Path) -> EndpointHandle {
        EndpointHandle::builder(test_deps())
            .socket_path(socket_path.to_path_buf())
            .watcher_slot(Arc::new(Mutex::new(None)))
            .build()
            .expect("handle build")
    }

    fn inode_of(path: &Path) -> u64 {
        std::fs::metadata(path).expect("metadata").ino()
    }

    // stop() must not unlink a socket rebound by another process.
    #[tokio::test]
    async fn stop_does_not_unlink_foreign_socket() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let socket_path = tmp.path().join("gpy.sock");

        let mut handle = build_handle(&socket_path);
        handle.bind_socket().expect("bind");

        // Simulate another agent rebinding the path (new inode).
        handle.listener.take();
        std::fs::remove_file(&socket_path).expect("remove ours");
        let foreign = UnixListener::bind(&socket_path).expect("foreign bind");
        let foreign_inode = inode_of(&socket_path);

        handle
            .stop()
            .expect("stop is a successful no-op when not owner");

        assert!(
            socket_path.exists(),
            "must not unlink a socket bound by another process"
        );
        assert_eq!(
            inode_of(&socket_path),
            foreign_inode,
            "the foreign agent's socket inode must remain in place"
        );
        drop(foreign);
    }

    #[tokio::test]
    async fn stop_unlinks_own_socket() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let socket_path = tmp.path().join("gpy.sock");

        let mut handle = build_handle(&socket_path);
        handle.bind_socket().expect("bind");
        assert!(socket_path.exists(), "socket should exist after bind");

        handle.stop().expect("stop");

        assert!(
            !socket_path.exists(),
            "a handle must remove its own still-current socket"
        );
    }

    // Accept-loop cleanup must also honour ownership.
    #[tokio::test]
    async fn cleanup_socket_file_does_not_unlink_foreign_socket() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let socket_path = tmp.path().join("gpy.sock");

        let mut handle = build_handle(&socket_path);
        handle.bind_socket().expect("bind");

        handle.listener.take();
        std::fs::remove_file(&socket_path).expect("remove ours");
        let foreign = UnixListener::bind(&socket_path).expect("foreign bind");
        let foreign_inode = inode_of(&socket_path);

        handle.cleanup_socket_file();

        assert!(
            socket_path.exists(),
            "accept-loop cleanup must not unlink another process's socket"
        );
        assert_eq!(inode_of(&socket_path), foreign_inode);
        drop(foreign);
    }

    /// A handle with a 50 ms ownership check, started on its own task.
    async fn spawn_started_handle(
        socket_path: &Path,
    ) -> (
        tokio::task::JoinHandle<crate::Result<()>>,
        tokio::sync::broadcast::Sender<()>,
    ) {
        let mut handle = EndpointHandle::builder(test_deps())
            .socket_path(socket_path.to_path_buf())
            .socket_ownership_check(std::time::Duration::from_millis(50))
            .build()
            .expect("handle build");
        let shutdown = handle.shutdown_tx.clone();
        let task = tokio::spawn(async move { handle.start().await });
        for _ in 0_u32..200 {
            if socket_path.exists() {
                break;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        assert!(socket_path.exists(), "the handle must bind its socket");
        (task, shutdown)
    }

    // Socket rebound by someone else: agent exits and leaves it alone.
    #[tokio::test]
    async fn accept_loop_exits_when_socket_is_rebound() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let socket_path = tmp.path().join("gpy.sock");
        let (task, _shutdown) = spawn_started_handle(&socket_path).await;

        std::fs::remove_file(&socket_path).expect("remove ours");
        let foreign = UnixListener::bind(&socket_path).expect("foreign bind");
        let foreign_inode = inode_of(&socket_path);

        let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .expect("the accept loop must exit once its socket is lost");
        assert!(matches!(outcome, Ok(Ok(()))), "{outcome:?}");
        assert_eq!(inode_of(&socket_path), foreign_inode);
        drop(foreign);
    }

    #[tokio::test]
    async fn accept_loop_keeps_running_while_socket_is_owned() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let socket_path = tmp.path().join("gpy.sock");
        let (task, shutdown) = spawn_started_handle(&socket_path).await;

        // Let several ownership checks elapse before asserting the loop survived.
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        assert!(
            !task.is_finished(),
            "an owned socket must keep the loop running"
        );

        shutdown.send(()).expect("send shutdown");
        let outcome = tokio::time::timeout(std::time::Duration::from_secs(2), task)
            .await
            .expect("shutdown must end the loop");
        assert!(matches!(outcome, Ok(Ok(()))), "{outcome:?}");
    }

    // A never-bound handle must never unlink a file at its socket path.
    #[test]
    fn never_bound_handle_does_not_unlink() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let socket_path = tmp.path().join("gpy.sock");
        std::fs::write(&socket_path, b"not ours").expect("write file");

        let handle = build_handle(&socket_path); // bind_socket never called

        handle.cleanup_socket_file();

        assert!(
            socket_path.exists(),
            "a handle that never bound must not remove anything"
        );
    }
}

#[cfg(all(test, unix))]
mod accept_error_tests {
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]

    use super::{accept_error_is_fd_exhaustion, accept_error_is_transient};
    use std::io::{Error, ErrorKind};

    #[test]
    fn fd_exhaustion_is_transient_and_backs_off() {
        for errno in [libc::EMFILE, libc::ENFILE] {
            let err = Error::from_raw_os_error(errno);
            assert!(
                accept_error_is_transient(&err),
                "errno {errno} should be transient"
            );
            assert!(
                accept_error_is_fd_exhaustion(&err),
                "errno {errno} should be classified as fd exhaustion"
            );
        }
    }

    #[test]
    fn interrupted_and_aborted_are_transient_without_backoff() {
        for kind in [ErrorKind::Interrupted, ErrorKind::ConnectionAborted] {
            let err = Error::new(kind, "transient");
            assert!(
                accept_error_is_transient(&err),
                "{kind:?} should be transient"
            );
            assert!(
                !accept_error_is_fd_exhaustion(&err),
                "{kind:?} is not fd exhaustion"
            );
        }
    }

    #[test]
    fn unrelated_errors_are_fatal() {
        let err = Error::new(ErrorKind::PermissionDenied, "fatal");
        assert!(!accept_error_is_transient(&err));
        assert!(!accept_error_is_fd_exhaustion(&err));
    }
}

#[cfg(all(test, unix))]
mod socket_dir_trust_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{DirRepair, secure_dir_for_uid};
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    const BOTH: [DirRepair; 2] = [DirRepair::Tighten, DirRepair::Refuse];

    #[test]
    fn accepts_owned_private_directory() {
        for repair in BOTH {
            let tmp = tempfile::tempdir().expect("temp dir");
            let dir = tmp.path().join("gpy-rt");
            fs::create_dir_all(&dir).expect("mkdir");
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o755)).expect("chmod");

            let uid = fs::metadata(&dir).expect("meta").uid();
            secure_dir_for_uid(&dir, uid, repair)
                .expect("an owned, not-writable-by-others dir must pass");
            assert_eq!(mode_of(&dir), 0o755, "a trusted dir is left as it was");
        }
    }

    #[test]
    fn rejects_symlinked_directory() {
        for repair in BOTH {
            let tmp = tempfile::tempdir().expect("temp dir");
            let real = tmp.path().join("real");
            fs::create_dir_all(&real).expect("mkdir real");
            let link = tmp.path().join("gpy-rt");
            symlink(&real, &link).expect("symlink");

            let uid = fs::metadata(&real).expect("meta").uid();
            let err = secure_dir_for_uid(&link, uid, repair)
                .expect_err("a symlink must be rejected even when it points to a dir we own");
            assert!(
                err.to_string().contains(&link.display().to_string()),
                "{err}"
            );
        }
    }

    #[test]
    fn rejects_a_regular_file() {
        for repair in BOTH {
            let tmp = tempfile::tempdir().expect("temp dir");
            let file = tmp.path().join("gpy-rt");
            fs::write(&file, "").expect("write file");

            let uid = fs::metadata(&file).expect("meta").uid();
            secure_dir_for_uid(&file, uid, repair).expect_err("a non-directory must be rejected");
        }
    }

    #[test]
    fn tightens_owned_group_or_world_writable_directory_to_0700() {
        for mode in [0o777, 0o775, 0o1777] {
            let tmp = tempfile::tempdir().expect("temp dir");
            let dir = tmp.path().join("gpy-rt");
            fs::create_dir_all(&dir).expect("mkdir");
            fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).expect("chmod");

            let uid = fs::metadata(&dir).expect("meta").uid();
            secure_dir_for_uid(&dir, uid, DirRepair::Tighten)
                .expect("an owned writable dir is repaired, not refused");
            assert_eq!(mode_of(&dir), 0o700, "mode {mode:o} must be tightened");
        }
    }

    #[test]
    fn refuses_owned_group_or_world_writable_directory_without_touching_it() {
        for mode in [0o777, 0o775, 0o1777] {
            let tmp = tempfile::tempdir().expect("temp dir");
            let dir = tmp.path().join("gpy-rt");
            fs::create_dir_all(&dir).expect("mkdir");
            fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).expect("chmod");

            let uid = fs::metadata(&dir).expect("meta").uid();
            let err = secure_dir_for_uid(&dir, uid, DirRepair::Refuse)
                .expect_err("a user-chosen writable dir is refused, not repaired");
            assert!(
                err.to_string().contains(&dir.display().to_string()),
                "{err}"
            );
            assert_eq!(mode_of(&dir), mode, "mode {mode:o} must be left alone");
        }
    }

    #[test]
    fn rejects_directory_owned_by_another_uid_without_touching_it() {
        for repair in BOTH {
            // Can't chown without privilege; claim a non-owner uid instead.
            let tmp = tempfile::tempdir().expect("temp dir");
            let dir = tmp.path().join("gpy-rt");
            fs::create_dir_all(&dir).expect("mkdir");
            fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).expect("chmod");

            let foreign_uid = fs::metadata(&dir).expect("meta").uid().wrapping_add(1);
            secure_dir_for_uid(&dir, foreign_uid, repair)
                .expect_err("a dir not owned by the expected uid must be rejected");
            assert_eq!(mode_of(&dir), 0o777, "a foreign dir is never chmod'ed");
        }
    }

    #[test]
    fn socket_in_an_untrusted_directory_is_never_bound() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let real = tmp.path().join("real");
        fs::create_dir_all(&real).expect("mkdir real");
        let link = tmp.path().join("rt");
        symlink(&real, &link).expect("symlink");

        let mut handle = super::socket_ownership_tests::build_handle(&link.join("gpy.sock"));
        handle
            .bind_socket()
            .expect_err("bind must refuse a symlinked directory");
        assert!(
            !real.join("gpy.sock").exists(),
            "nothing may be bound through the symlink"
        );
    }

    fn mode_of(path: &std::path::Path) -> u32 {
        fs::metadata(path).expect("meta").mode() & 0o7777
    }
}

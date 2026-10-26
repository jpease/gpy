//! Runtime handle for the IPC server: binds the Unix socket, accepts
//! connections, enforces guard settings, and dispatches messages.

#[cfg(unix)]
use super::connection::ConnectionHandler;
#[cfg(unix)]
use super::socket_dir::{DirRepair, secure_socket_dir};
#[cfg(unix)]
use crate::Error;
use crate::Result;
#[cfg(unix)]
use crate::debug_log;
#[cfg(unix)]
use crate::ipc::Response;
use crate::ipc::{
    ClientDirectory,
    handlers::{
        ClientHandler, GitHandler, HandlerRegistry, LanguageHandler, MiscHandler, RenderDeps,
    },
};
use crate::palette::PaletteCache;
use crate::security::{GuardSettings, RateLimiter};
use crate::theme::ThemeManager;
#[cfg(unix)]
use crate::warn_log;
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

/// Everything an [`EndpointHandle`] is constructed from.
pub struct EndpointDeps {
    /// Where the Unix socket is bound.
    pub socket_path: PathBuf,
    /// How a socket directory we own but others can write is treated: tighten
    /// it only when the caller knows GPY owns the directory.
    #[cfg(unix)]
    pub socket_dir_repair: DirRepair,
    /// Connection limits and rate limiting.
    pub security_config: GuardSettings,
    /// The agent's shared watcher slot; live updates break if it is not shared.
    pub watcher_slot: WatcherRef,
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
    /// Per-repo language detection results shared with the agent event loop.
    pub language_cache: crate::language::DetectionCache,
    /// Cached active palette, so a reload is picked up without re-parsing per render.
    pub palette_cache: Arc<PaletteCache>,
}

impl EndpointDeps {
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

/// An IPC error for a failed `action`, logged as it is raised.
#[cfg(unix)]
fn ipc_io_err(action: &str, err: &std::io::Error) -> Error {
    let message = format!("Failed to {action}: {err}");
    debug_log!("server", "{}", message);
    Error::ipc(message)
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
    /// Startup handshake with the waiting `gpy-agent start`, held until
    /// [`Self::bind_socket`] returns: its start lock stops a concurrent start
    /// evicting or forking between our fork and bind, and a bind failure is
    /// reported through it.
    #[cfg(unix)]
    startup: Option<crate::agent::lifecycle::start::StartupHandoff>,
    /// Ownership check period ([`SOCKET_OWNERSHIP_CHECK`]; tests shorten it).
    #[cfg(unix)]
    socket_ownership_check: Duration,
    security_config: GuardSettings,
    rate_limiter: RateLimiter,
    client_registry: Arc<ClientDirectory>,
    config_manager: Arc<crate::config::manager::ConfigManager>,
    /// How [`Self::bind_socket`] treats a socket directory we own that others
    /// can write; the caller knows whether GPY owns the directory.
    #[cfg(unix)]
    socket_dir_repair: DirRepair,
    theme_manager: Arc<ThemeManager>,
    palette_cache: Arc<PaletteCache>,
    connection_semaphore: Arc<Semaphore>,
    shutdown_tx: broadcast::Sender<()>, // Used to signal shutdown to all listeners
    /// Created with the channel and taken by `accept_loop` on entry, so a
    /// shutdown sent before the loop starts is still received.
    shutdown_rx: Option<broadcast::Receiver<()>>,
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

    /// Create a handle from its dependencies; the socket is bound by [`Self::start`].
    #[must_use]
    pub fn new(deps: EndpointDeps) -> Self {
        let rate_limiter = RateLimiter::new(deps.security_config.max_connections_per_second);
        let connection_semaphore = Arc::new(Semaphore::new(
            deps.security_config.max_concurrent_connections,
        ));

        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        let render = deps.render();
        let git_handler = Arc::new(GitHandler::new(render.clone(), Arc::clone(&deps.git_cache)));
        let language_handler = Arc::new(LanguageHandler::new(
            render.clone(),
            deps.language_cache.clone(),
        ));
        let client_handler = Arc::new(ClientHandler::new(
            render,
            Arc::clone(&deps.git_cache),
            deps.watcher_slot,
        ));
        let misc_handler = Arc::new(MiscHandler::new());
        let handler_registry = Arc::new(HandlerRegistry::new(
            git_handler,
            language_handler,
            client_handler,
            misc_handler,
        ));

        Self {
            socket_path: deps.socket_path,
            #[cfg(unix)]
            listener: None,
            #[cfg(unix)]
            bound_socket: None,
            #[cfg(unix)]
            startup: None,
            #[cfg(unix)]
            socket_ownership_check: SOCKET_OWNERSHIP_CHECK,
            security_config: deps.security_config,
            rate_limiter,
            client_registry: deps.client_registry,
            config_manager: deps.config_manager,
            #[cfg(unix)]
            socket_dir_repair: deps.socket_dir_repair,
            theme_manager: deps.theme_manager,
            palette_cache: deps.palette_cache,
            connection_semaphore,
            shutdown_tx,
            shutdown_rx: Some(shutdown_rx),
            handler_registry,
        }
    }

    /// Hold the startup handshake until this handle's socket is bound (see the
    /// `startup` field).
    #[cfg(unix)]
    pub(crate) fn hold_startup_until_bound(
        &mut self,
        startup: crate::agent::lifecycle::start::StartupHandoff,
    ) {
        self.startup = Some(startup);
    }

    /// Start accepting client connections.
    ///
    /// # Errors
    /// Fails if the socket cannot be bound or accepting fails.
    #[cfg(unix)]
    pub async fn start(&mut self) -> Result<()> {
        use crate::debug_log;

        let bound = self.bind_socket();
        // End the handshake either way, so the next start sees the real outcome.
        if let (Err(e), Some(startup)) = (&bound, self.startup.take()) {
            startup.fail(e);
        }
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
            ipc_io_err(&format!("bind socket '{}'", self.socket_path.display()), &e)
        })?;
        debug_log!("server", "Socket bound successfully");

        debug_log!("server", "Setting socket permissions...");
        let metadata = std::fs::metadata(&self.socket_path)
            .map_err(|e| ipc_io_err("read socket metadata", &e))?;

        let mut perms = metadata.permissions();
        perms.set_mode(0o600);
        std::fs::set_permissions(&self.socket_path, perms)
            .map_err(|e| ipc_io_err("set socket permissions", &e))?;
        debug_log!("server", "Socket permissions set successfully");

        // Record after chmod: it changes ctime, so a pre-chmod identity would never match.
        let bound_metadata = std::fs::metadata(&self.socket_path)
            .map_err(|e| ipc_io_err("re-read socket metadata", &e))?;
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
        let mut shutdown_rx = self.shutdown_rx.take().ok_or_else(|| {
            Error::ipc("accept loop already ran: no shutdown receiver".to_owned())
        })?;
        let listener = self
            .listener
            .as_ref()
            .ok_or_else(|| Error::ipc("Server not started - no listener available".to_owned()))?;

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
                            // Fatal errors skip the loop exit below, so clean up here.
                            self.unlink_socket_if_owned();
                            return Err(ipc_io_err("accept connection", &e));
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
        self.unlink_socket_if_owned();

        Ok(())
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

    /// Get the socket path being used.
    pub fn socket_path(&self) -> &str {
        self.socket_path.to_str().unwrap_or("")
    }

    /// Sender that stops the accept loop, for a caller that moved this handle into a task.
    #[must_use]
    pub fn shutdown_sender(&self) -> broadcast::Sender<()> {
        self.shutdown_tx.clone()
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
    use crate::ipc::{Message, Segment};
    use std::sync::Arc;

    fn make_handler_registry() -> Arc<HandlerRegistry> {
        let handle =
            super::socket_ownership_tests::build_handle(std::path::Path::new("/tmp/test.sock"));
        Arc::clone(&handle.handler_registry)
    }

    /// An Ansi segment at the head of the prompt, following a blue pill.
    fn first_ansi_after_blue() -> Segment {
        Segment {
            format: Format::Ansi,
            is_last: false,
            is_first: true,
            prev_bg: Some("blue".to_owned()),
        }
    }

    #[test]
    fn route_request_secure_threads_prev_bg_through_tuple() {
        let registry = make_handler_registry();
        let security_config = GuardSettings::default();
        let safe_path = crate::security::SafePath::new("/tmp").expect("valid path");
        let segment = Segment {
            prev_bg: Some("blue".to_owned()),
            ..Segment::default()
        };
        let msg = Message::RepositoryStatus {
            path: safe_path,
            segment: segment.clone(),
        };
        let result = ConnectionHandler::route_request_secure(&registry, &security_config, &msg);
        assert_eq!(
            result.segment, segment,
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
            segment: Segment::default(),
        };
        let result = ConnectionHandler::route_request_secure(&registry, &security_config, &msg);
        assert_eq!(
            result.segment.prev_bg, None,
            "absent prev_bg must remain None in the routing result"
        );
    }

    #[test]
    fn handler_registry_routes_hostname_request_to_hostname_response() {
        let registry = make_handler_registry();
        let msg = Message::HostnameRequest {
            hostname: "h".to_owned(),
            is_ssh: true,
            segment: Segment::default(),
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
    fn route_request_secure_threads_hostname_request_segment() {
        let registry = make_handler_registry();
        let security_config = GuardSettings::default();
        let msg = Message::HostnameRequest {
            hostname: "my-host".to_owned(),
            is_ssh: false,
            segment: first_ansi_after_blue(),
        };
        let result = ConnectionHandler::route_request_secure(&registry, &security_config, &msg);

        match result.response {
            crate::ipc::Response::Hostname { hostname, is_ssh } => {
                assert_eq!(hostname, "my-host");
                assert!(!is_ssh);
            }
            other => panic!("expected Response::Hostname, got {other:?}"),
        }
        assert_eq!(result.notify_path, None);
        assert_eq!(result.segment, first_ansi_after_blue());
    }

    #[test]
    fn handler_registry_routes_username_request_to_username_response() {
        let registry = make_handler_registry();
        let msg = Message::UsernameRequest {
            username: "u".to_owned(),
            is_root: true,
            segment: Segment::default(),
        };
        let response = registry.route(&msg).expect("routing must succeed");
        match response {
            crate::ipc::Response::Username { username, is_root } => {
                assert_eq!(username, "u");
                assert!(is_root, "the handler must echo the request's is_root");
            }
            other => panic!("expected Response::Username, got {other:?}"),
        }
    }

    #[test]
    fn route_request_secure_threads_username_request_segment() {
        let registry = make_handler_registry();
        let security_config = GuardSettings::default();
        let msg = Message::UsernameRequest {
            username: "my-user".to_owned(),
            is_root: false,
            segment: first_ansi_after_blue(),
        };
        let result = ConnectionHandler::route_request_secure(&registry, &security_config, &msg);

        match result.response {
            crate::ipc::Response::Username { username, is_root } => {
                assert_eq!(username, "my-user");
                assert!(!is_root);
            }
            other => panic!("expected Response::Username, got {other:?}"),
        }
        assert_eq!(result.notify_path, None);
        assert_eq!(result.segment, first_ansi_after_blue());
    }

    #[test]
    fn handler_registry_routes_exit_status_request_to_exit_status_response() {
        let registry = make_handler_registry();
        let msg = Message::ExitStatusRequest {
            success: false,
            segment: Segment::default(),
        };
        let response = registry.route(&msg).expect("routing must succeed");
        match response {
            crate::ipc::Response::ExitStatus { success } => assert!(!success),
            other => panic!("expected Response::ExitStatus, got {other:?}"),
        }
    }

    #[test]
    fn route_request_secure_threads_exit_status_request_segment() {
        let registry = make_handler_registry();
        let security_config = GuardSettings::default();
        let msg = Message::ExitStatusRequest {
            success: true,
            segment: first_ansi_after_blue(),
        };
        let result = ConnectionHandler::route_request_secure(&registry, &security_config, &msg);

        match result.response {
            crate::ipc::Response::ExitStatus { success } => assert!(success),
            other => panic!("expected Response::ExitStatus, got {other:?}"),
        }
        assert_eq!(result.segment, first_ansi_after_blue());
    }
}

#[cfg(all(test, unix))]
mod socket_ownership_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::{DirRepair, EndpointDeps, EndpointHandle};
    use crate::config::manager::ConfigManager;
    use crate::git::cache::GitStatusCache;
    use crate::ipc::ClientDirectory;
    use crate::palette::PaletteCache;
    use crate::security::GuardSettings;
    use crate::theme::ThemeManager;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// Build an unbound `EndpointHandle` targeting `socket_path` (no accept loop).
    pub(super) fn build_handle(socket_path: &Path) -> EndpointHandle {
        let config_manager = Arc::new(ConfigManager::with_defaults().expect("default config"));
        let palette_cache = Arc::new(PaletteCache::from_config(&config_manager.get()));
        EndpointHandle::new(EndpointDeps {
            socket_path: socket_path.to_path_buf(),
            socket_dir_repair: DirRepair::Refuse,
            security_config: GuardSettings::default(),
            watcher_slot: Arc::new(Mutex::new(None)),
            client_registry: Arc::new(ClientDirectory::new()),
            git_cache: Arc::new(GitStatusCache::new()),
            config_manager,
            theme_manager: Arc::new(ThemeManager::builtin("default").expect("default theme")),
            instant_cache: Arc::new(crate::cache::InstantPromptCache::new_for_test()),
            language_cache: crate::language::DetectionCache::new(),
            palette_cache,
        })
    }

    fn inode_of(path: &Path) -> u64 {
        std::fs::metadata(path).expect("metadata").ino()
    }

    #[tokio::test]
    async fn unlink_socket_if_owned_unlinks_own_socket() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let socket_path = tmp.path().join("gpy.sock");

        let mut handle = build_handle(&socket_path);
        handle.bind_socket().expect("bind");
        assert!(socket_path.exists(), "socket should exist after bind");

        handle.unlink_socket_if_owned();

        assert!(
            !socket_path.exists(),
            "a handle must remove its own still-current socket"
        );
    }

    // Accept-loop cleanup must also honour ownership.
    #[tokio::test]
    async fn unlink_socket_if_owned_does_not_unlink_foreign_socket() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let socket_path = tmp.path().join("gpy.sock");

        let mut handle = build_handle(&socket_path);
        handle.bind_socket().expect("bind");

        handle.listener.take();
        std::fs::remove_file(&socket_path).expect("remove ours");
        let foreign = UnixListener::bind(&socket_path).expect("foreign bind");
        let foreign_inode = inode_of(&socket_path);

        handle.unlink_socket_if_owned();

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
        let mut handle = build_handle(socket_path);
        handle.socket_ownership_check = std::time::Duration::from_millis(50);
        let shutdown = handle.shutdown_sender();
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

        // Absence window: several 50 ms ownership checks run, and none may end the loop.
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

        handle.unlink_socket_if_owned();

        assert!(
            socket_path.exists(),
            "a handle that never bound must not remove anything"
        );
    }

    #[test]
    fn socket_in_an_untrusted_directory_is_never_bound() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let real = tmp.path().join("real");
        std::fs::create_dir_all(&real).expect("mkdir real");
        let link = tmp.path().join("rt");
        std::os::unix::fs::symlink(&real, &link).expect("symlink");

        let mut handle = super::socket_ownership_tests::build_handle(&link.join("gpy.sock"));
        handle
            .bind_socket()
            .expect_err("bind must refuse a symlinked directory");
        assert!(
            !real.join("gpy.sock").exists(),
            "nothing may be bound through the symlink"
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

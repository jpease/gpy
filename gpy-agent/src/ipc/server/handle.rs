//! Runtime handle for the IPC server.
//!
//! [`EndpointHandle`] owns the shared state required to bind the Unix socket,
//! accept client connections, enforce guard settings, and dispatch parsed
//! messages to domain handlers. Construction is kept in the endpoint builder so
//! required dependencies stay explicit.

use super::builder::EndpointHandleBuilder;
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
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
#[cfg(unix)]
use std::time::Duration;
#[cfg(unix)]
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::{Semaphore, broadcast};
#[cfg(unix)]
use tokio::time::timeout;

/// Multiplier applied to the live per-read timeout (`request_timeout()`) to
/// derive the whole-connection deadline enforced in `spawn_client_handler`
/// (#316).
///
/// Every legitimate client in this codebase (Fish shell requests,
/// `RegisterClient`, etc.) opens a fresh connection, sends exactly one line,
/// reads exactly one response, and disconnects — a single read/process/write
/// cycle. 4x the per-read timeout gives that cycle generous headroom (room
/// for a couple of slow-but-legitimate read syscalls) while still bounding
/// total connection lifetime, so a client that never sends a complete line
/// (each individual read arriving just under the per-read timeout) cannot
/// hold its connection semaphore permit indefinitely. Deriving the deadline
/// from the live, user-configurable `request_timeout()` (rather than a fixed
/// duration) keeps it consistent if that timeout is ever raised.
#[cfg(unix)]
const CONNECTION_DEADLINE_MULTIPLIER: u32 = 4;

/// Type alias for the watcher reference used in the IPC server
///
/// This complex nested type represents a thread-safe, mutable reference to an optional
/// multi-repo watcher. The inner Option allows the watcher to be temporarily taken
/// out for operations (e.g., when shutting down or replacing the watcher).
pub type WatcherRef = Arc<Mutex<Option<Arc<crate::watcher::multi_repo::MultiRepoWatcher>>>>;

/// Shared subsystem handles the IPC endpoint needs for the whole of its
/// lifetime (#587).
///
/// Threaded as one value everywhere the same bundle used to be re-derived from
/// an eight-to-eleven-name parameter list: [`EndpointHandle::with_path_and_state`],
/// the handler wiring it used to delegate to, and
/// [`EndpointHandleBuilder::build`]. Five of these are also exactly what every
/// publish-and-repaint site needs, which [`ServerDeps::render`] hands out as a
/// [`RenderDeps`].
///
/// The watcher slot and the guard settings deliberately stay outside: the
/// watcher is a shared *mutable slot* with different semantics from these
/// handles, and the guard settings are a plain value-type settings struct, not
/// a shared handle.
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
    /// Cached active palette, shared so a `palette use` reload is picked up
    /// without re-parsing the palette TOML on every render.
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

/// Type alias for an optional watcher slot used in builders and optional fields
///
/// Same as `WatcherRef` but wrapped in an Option to indicate the slot may not be set yet.
pub type WatcherSlot = Option<WatcherRef>;

/// Returns `true` for `accept()` errors that are transient and recoverable.
///
/// A momentary file-descriptor spike (`EMFILE`/`ENFILE`), an interrupted syscall
/// (`EINTR`), or a connection aborted before it was accepted (`ECONNABORTED`) must
/// not terminate the agent: the accept loop logs and continues, resuming service
/// once the condition clears. All other errors are treated as fatal.
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

/// Returns `true` if `err` is a file-descriptor exhaustion error (`EMFILE`/`ENFILE`).
///
/// These warrant a brief backoff so the accept loop does not spin hot while no
/// descriptors are available.
#[cfg(unix)]
fn accept_error_is_fd_exhaustion(err: &std::io::Error) -> bool {
    matches!(err.raw_os_error(), Some(libc::EMFILE | libc::ENFILE))
}

#[cfg(unix)]
use crate::ipc::socket_identity::SocketIdentity;

/// How often the accept loop checks that its socket path still holds the
/// socket it bound (#779).
///
/// Long enough that a briefly missing socket during a normal restart never
/// races a healthy agent into exiting.
#[cfg(unix)]
pub(crate) const SOCKET_OWNERSHIP_CHECK: Duration = Duration::from_secs(30);

/// Outcome of one ownership check (#779). Only `Lost` acts.
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
//
// The collaborators below are all read by the accept/serve path, which is
#[cfg_attr(
    not(unix),
    expect(
        dead_code,
        reason = "Unix-only (the transport is a Unix domain socket); native Windows can still build and hold a handle -- the builder is cross-platform, and the lib tests construct one -- it simply never serves a connection, so every field is written and never read there (#540); cfg-gating ten fields would fork the struct's shape, its builder and its tests per platform for no behavioural gain, so the disuse is stated instead of engineered away"
    )
)]
pub struct EndpointHandle {
    pub(crate) socket_path: PathBuf,
    #[cfg(unix)]
    listener: Option<UnixListener>,
    /// Identity of the socket file this handle bound in [`Self::bind_socket`],
    /// used to verify ownership before unlinking (#317). `None` for handles that
    /// never bind (per-client handles), which therefore never unlink anything.
    #[cfg(unix)]
    bound_socket: Option<SocketIdentity>,
    /// Per-socket start lock (`<socket>.lock`, #723) inherited from the
    /// `gpy-agent start` process that forked this daemon. Held until
    /// [`Self::bind_socket`] returns, so a concurrent start cannot check,
    /// evict or fork between our fork and our bind; released on bind success
    /// and on bind failure alike.
    #[cfg(unix)]
    start_lock: Option<nix::fcntl::Flock<std::fs::File>>,
    /// Period of the accept loop's ownership check ([`SOCKET_OWNERSHIP_CHECK`]
    /// outside tests).
    #[cfg(unix)]
    pub(super) socket_ownership_check: Duration,
    security_config: GuardSettings,
    rate_limiter: RateLimiter,
    client_registry: Arc<ClientDirectory>,
    config_manager: Arc<crate::config::manager::ConfigManager>,
    #[cfg_attr(
        not(test),
        expect(
            dead_code,
            reason = "only read by a test (ipc::server::tests, cfg(test)-gated) asserting the builder threads through the exact shared watcher slot rather than creating a new one; a non-test build never reads it back off `self`"
        )
    )]
    pub(crate) watcher: WatcherRef,
    theme_manager: Arc<ThemeManager>,
    latency_tracker: Arc<crate::ipc::LatencyTracker>,
    palette_cache: Arc<PaletteCache>,
    connection_semaphore: Arc<Semaphore>,
    shutdown_tx: broadcast::Sender<()>, // Used to signal shutdown to all listeners
    // Kept alive so shutdown_tx.send(()) always has at least one receiver and never errors; each connection subscribes its own receiver via shutdown_tx.subscribe() instead of using this one.
    _shutdown_rx: broadcast::Receiver<()>,
    pub(crate) handler_registry: Arc<HandlerRegistry>,
}

/// Content fingerprint of a live-update response, used by the per-repo signal
/// throttle to distinguish a genuinely different push from a duplicate (#438).
///
/// Returns a stable-within-process hash of the serialized response, or `None`
/// if serialization fails (in which case the throttle falls back to its
/// purely time-based behavior). `RepositoryStatus`/`Language` payloads carry no
/// timestamps, so identical repository state hashes identically and continues
/// to coalesce; only a real change produces a distinct token.
#[cfg(unix)]
fn content_token(response: &Response) -> Option<u64> {
    use std::hash::{Hash, Hasher};
    #[expect(
        clippy::collection_is_never_read,
        reason = "false positive: collection_is_never_read doesn't recognize Hash::hash below as reading `bytes` -- it does hash every byte"
    )]
    let bytes = serde_json::to_vec(response).ok()?;
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    Some(hasher.finish())
}

/// Shared notification logic used by both [`EndpointHandle::notify_clients`]
/// and `ConnectionHandler`'s per-connection copy.
///
/// (Kept for existing test call sites.) Signal live-update subscribers when
/// a response affects git status or language detection and live updates are
/// enabled.
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
            // Carry a content token so the per-repo throttle can coalesce a
            // duplicate push yet still deliver a genuinely distinct one inside
            // the window (#438). A token derives only from the response
            // content, never a timestamp, so unchanged state keeps coalescing.
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

    /// Hard cap on total connection lifetime (#316).
    ///
    /// See [`CONNECTION_DEADLINE_MULTIPLIER`] for the rationale behind the
    /// multiplier. Derived from the live `request_timeout()` so it can never
    /// end up smaller than the per-read timeout it wraps.
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

        // Create shutdown channel for graceful shutdown coordination
        let (shutdown_tx, shutdown_rx) = broadcast::channel(1);

        // One domain handler per message family (git/language/theme/client/misc),
        // wrapped in a single `Arc` for cheap cloning into each connection's
        // `ConnectionHandler`. Every handler takes its dependencies off `deps`,
        // so wiring them is a handful of clones rather than a nine-parameter
        // hand-off (#587).
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

    /// Create a builder for constructing an `EndpointHandle` with clearer parameter semantics.
    ///
    /// # Example
    ///
    /// ```rust
    /// # use gpy_agent::ipc::server::EndpointHandle;
    /// # use std::sync::Arc;
    /// # use gpy_agent::ipc::registry::ClientDirectory;
    /// # use gpy_agent::git::cache::GitStatusCache;
    /// # use gpy_agent::config::manager::ConfigManager;
    /// # use gpy_agent::theme::manager::ThemeManager;
    /// # use gpy_agent::ipc::LatencyTracker;
    /// # use gpy_agent::cache::InstantPromptCache;
    /// # use gpy_agent::language::DetectionCache;
    /// # fn main() -> Result<(), Box<dyn std::error::Error>> {
    /// let registry = Arc::new(ClientDirectory::new());
    /// let git_cache = Arc::new(GitStatusCache::new());
    /// let config_manager = Arc::new(ConfigManager::with_defaults()?);
    /// let theme_manager = Arc::new(ThemeManager::new("default")?);
    /// let instant_cache = Arc::new(InstantPromptCache::new()?);
    /// let latency_tracker = Arc::new(LatencyTracker::new(100));
    /// let language_cache = DetectionCache::new();
    ///
    /// let server = EndpointHandle::builder()
    ///     .client_registry(registry)
    ///     .git_cache(git_cache)
    ///     .config_manager(config_manager)
    ///     .theme_manager(theme_manager)
    ///     .instant_cache(instant_cache)
    ///     .latency_tracker(latency_tracker)
    ///     .language_cache(language_cache)
    ///     .build()?;
    /// # Ok(())
    /// # }
    /// ```
    #[must_use]
    pub fn builder() -> EndpointHandleBuilder {
        EndpointHandleBuilder::new()
    }

    /// Get the default socket path for this system.
    ///
    /// # Errors
    ///
    /// Returns an error if determining or preparing the runtime directory fails.
    pub(crate) fn default_socket_path() -> Result<PathBuf> {
        // Check for custom socket path in environment (matches Fish shell logic).
        // Empty means unset, exactly as every shell reads it (#626).
        if let Some(custom_path) = crate::agent::lifecycle::socket_path_override() {
            return Ok(custom_path);
        }

        // Default: Unix socket only - Fish runs on Linux, macOS, and WSL (all Unix environments)
        let runtime_root = Self::get_runtime_root()?;
        Ok(runtime_root.join("gpy.sock"))
    }

    /// Get GPY runtime root directory (matches Fish shell logic)
    ///
    /// # Errors
    ///
    /// Returns an error if any of the candidate runtime directories cannot be created or
    /// accessed.
    fn get_runtime_root() -> Result<PathBuf> {
        // Follow same precedence as Fish shell
        // Normalised the same way `paths::runtime_root_for` normalises them
        // (#626): an empty or relative `XDG_*` value is "unset" per the XDG
        // spec, so the branch selection below and the resolver below it must
        // agree about which variables actually count.
        let xdg_runtime_raw = crate::paths::root_var("XDG_RUNTIME_DIR");
        let xdg_cache_raw = crate::paths::root_var("XDG_CACHE_HOME");
        let xdg_runtime =
            crate::paths::xdg_value(xdg_runtime_raw.as_deref(), crate::paths::Os::Unix);
        let xdg_cache = crate::paths::xdg_value(xdg_cache_raw.as_deref(), crate::paths::Os::Unix);
        let home = crate::paths::home_dir();

        // Final fallback: /tmp is world-writable, so — unlike the XDG/HOME
        // branches, which land in a per-user tree — another local user can
        // pre-create or symlink the runtime directory. Validate before trusting
        // it. Reached only when XDG_RUNTIME_DIR, XDG_CACHE_HOME, and HOME are
        // all unset (some daemon/CI contexts). See #426.
        if xdg_runtime.is_none() && xdg_cache.is_none() && home.is_none() {
            return Self::prepare_tmp_runtime_root();
        }

        let path = crate::paths::runtime_root_for(xdg_runtime, xdg_cache, home.as_deref());
        // Branch-specific error text, matching the pre-#477 per-branch
        // resolver this now replaces.
        let context = if xdg_runtime.is_some() {
            "XDG runtime dir"
        } else if xdg_cache.is_some() {
            "XDG cache dir"
        } else {
            "cache dir"
        };
        std::fs::create_dir_all(&path)
            .map_err(|e| Error::ipc(format!("Failed to create {context}: {e}")))?;
        Ok(path)
    }

    /// Prepare and validate the last-resort `/tmp` runtime directory.
    ///
    /// `/tmp` is world-writable, so another local user could pre-create the
    /// runtime directory and own it, or replace it with a symlink pointing
    /// elsewhere, leaving the agent to bind its socket inside an
    /// attacker-controlled directory. We use a uid-scoped name to avoid
    /// cross-user contention and then fail closed unless the directory is a
    /// real directory (not a symlink), owned by the current uid, and not
    /// group/other-writable — mirroring the trust guarantees the XDG/HOME
    /// branches inherit from their per-user parents. See #426.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created or fails any trust
    /// check (not a directory, foreign owner, or group/other-writable).
    #[cfg(unix)]
    fn prepare_tmp_runtime_root() -> Result<PathBuf> {
        let uid = nix::unistd::getuid().as_raw();
        // uid-scoped name: another user pre-creating `/tmp/gpy-<their-uid>`
        // does not collide with ours, and the validation below still rejects
        // one they pre-created under our uid's name.
        let path = PathBuf::from(format!("/tmp/gpy-{uid}"));
        std::fs::create_dir_all(&path)
            .map_err(|e| Error::ipc(format!("Failed to create tmp dir: {e}")))?;
        Self::validate_runtime_dir(&path, uid)?;
        Ok(path)
    }

    /// Fail closed unless `path` is a real directory (not a symlink), owned by
    /// `uid`, and not group/other-writable.
    ///
    /// Split out from [`Self::prepare_tmp_runtime_root`] so the trust checks can
    /// be exercised against a temp directory without touching the shared
    /// `/tmp/gpy-<uid>` a live agent may be using. See #426.
    ///
    /// # Errors
    ///
    /// Returns an error if `path` cannot be stat-ed or fails any trust check.
    #[cfg(unix)]
    fn validate_runtime_dir(path: &std::path::Path, uid: u32) -> Result<()> {
        use std::os::unix::fs::MetadataExt;

        // `symlink_metadata` does not follow a final symlink, so a symlinked
        // directory is caught here instead of silently trusting its target.
        let meta = std::fs::symlink_metadata(path)
            .map_err(|e| Error::ipc(format!("Failed to stat runtime dir: {e}")))?;

        if !meta.file_type().is_dir() {
            return Err(Error::ipc(format!(
                "Runtime dir {} is not a directory (possible symlink attack)",
                path.display()
            )));
        }
        if meta.uid() != uid {
            return Err(Error::ipc(format!(
                "Runtime dir {} is not owned by the current user (uid {uid})",
                path.display()
            )));
        }
        // Group- or world-writable (0o022): another user could swap the socket
        // out from under us even though we own the directory.
        if meta.mode() & 0o022 != 0 {
            return Err(Error::ipc(format!(
                "Runtime dir {} is group/other-writable",
                path.display()
            )));
        }

        Ok(())
    }

    /// Non-Unix fallback: `/tmp` has no Unix ownership semantics to validate,
    /// and this branch is effectively unreachable there (the socket path is
    /// Unix-only), so preserve the historical behavior.
    ///
    /// # Errors
    ///
    /// Returns an error if the directory cannot be created.
    #[cfg(not(unix))]
    fn prepare_tmp_runtime_root() -> Result<PathBuf> {
        let path = PathBuf::from("/tmp/gpy");
        std::fs::create_dir_all(&path)
            .map_err(|e| Error::ipc(format!("Failed to create tmp dir: {e}")))?;
        Ok(path)
    }

    /// Hold the per-socket start lock until this handle's socket is bound
    /// (see the `start_lock` field).
    #[cfg(unix)]
    pub(crate) fn hold_start_lock_until_bound(&mut self, lock: nix::fcntl::Flock<std::fs::File>) {
        self.start_lock = Some(lock);
    }

    /// Start accepting client connections
    ///
    /// # Errors
    ///
    /// Returns an error if the socket cannot be bound or if accepting connections fails.
    #[cfg(unix)]
    pub async fn start(&mut self) -> Result<()> {
        use crate::debug_log;

        let bound = self.bind_socket();
        // Drop the inherited start lock whether or not the bind worked: either
        // way the next `gpy-agent start` can now see the real outcome.
        self.start_lock = None;
        bound?;
        debug_log!("server", "Listener stored, starting accept loop...");

        // Start accepting connections in a loop
        debug_log!("server", "About to enter accept_loop");
        let result = self.accept_loop().await;
        debug_log!("server", "accept_loop returned: {:?}", result.is_ok());
        result
    }

    /// Bind the Unix socket, record its inode for ownership checks, and set
    /// user-only (0600) permissions.
    ///
    /// The inode captured here (#317) lets [`Self::unlink_socket_if_owned`]
    /// refuse to remove a socket a *different* agent process has since bound at
    /// the same path. Split out of [`Self::start`] so tests can exercise the
    /// bind/cleanup lifecycle without running the accept loop.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket cannot be bound, or its metadata read or
    /// permissions set.
    #[cfg(unix)]
    fn bind_socket(&mut self) -> Result<()> {
        use crate::debug_log;
        use std::os::unix::fs::PermissionsExt;

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

        // Record ownership *after* the chmod: `set_permissions` updates the
        // file's ctime, so an identity captured from the pre-chmod stat would
        // never match again and the handle would refuse to clean up its own
        // socket.
        let bound_metadata = std::fs::metadata(&self.socket_path).map_err(|e| {
            let error_msg = format!("Failed to re-read socket metadata: {e}");
            debug_log!("server", "{}", error_msg);
            Error::ipc(error_msg)
        })?;
        self.bound_socket = Some(SocketIdentity::from_metadata(&bound_metadata));

        self.listener = Some(listener);
        Ok(())
    }

    /// Main connection acceptance loop
    ///
    /// # Errors
    ///
    /// Returns an error if the server has not been started with a listener or if accepting a
    /// connection fails.
    #[cfg(unix)]
    async fn accept_loop(&mut self) -> Result<()> {
        use crate::debug_log;

        debug_log!("server", "Starting accept loop");
        let listener = self
            .listener
            .as_ref()
            .ok_or_else(|| Error::ipc("Server not started - no listener available".to_owned()))?;

        // Subscribe to shutdown signals
        let mut shutdown_rx = self.shutdown_tx.subscribe();

        // Ownership tick (#779): an agent whose socket was removed or
        // rebound by another process is unreachable, so it exits the same way
        // an IPC `Shutdown` does. No tick at t=0.
        let mut ownership_timer = tokio::time::interval(self.socket_ownership_check);
        ownership_timer.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        ownership_timer.reset();

        debug_log!("server", "Got listener reference, entering main loop");
        loop {
            debug_log!("server", "Waiting for connection...");

            // Wait for either a new connection or shutdown signal
            tokio::select! {
                result = listener.accept() => {
                    let stream = match result {
                        Ok((stream, _)) => stream,
                        Err(e) if accept_error_is_transient(&e) => {
                            // Transient errors (fd exhaustion, interrupted/aborted
                            // syscalls) must not tear down the agent: log and keep
                            // serving so prompts recover once the condition clears.
                            warn_log!("server", "Transient accept() error, continuing to serve: {e}");
                            debug_log!("server", "Transient accept() error: {}", e);
                            // Back off briefly on fd exhaustion to avoid a hot spin
                            // loop while descriptors are unavailable.
                            if accept_error_is_fd_exhaustion(&e) {
                                tokio::time::sleep(Duration::from_millis(20)).await;
                            }
                            continue;
                        }
                        Err(e) => {
                            let error_msg = format!("Failed to accept connection: {e}");
                            debug_log!("server", "{}", error_msg);
                            // Fatal accept() errors bypass the normal loop exit below,
                            // so the socket must be cleaned up here too or it leaks.
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

    /// Best-effort removal of the socket file.
    ///
    /// Called from every `accept_loop` exit path (graceful shutdown and fatal
    /// accept errors) so a stale socket never blocks the next `gpy-agent start`.
    /// Routes through [`Self::unlink_socket_if_owned`] so it can never remove a
    /// socket a different agent has since bound at the same path (#317).
    #[cfg(unix)]
    fn cleanup_socket_file(&self) {
        self.unlink_socket_if_owned();
    }

    /// Whether the path still holds the socket this handle bound (#779).
    ///
    /// Uses the same strict dev+ino+ctime comparison as
    /// [`Self::unlink_socket_if_owned`]. A handle that never bound owns
    /// nothing and so can never lose it.
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

    /// Remove the socket file only if it is still the inode this handle bound.
    ///
    /// After a restart race (#317) the file at `socket_path` may belong to a
    /// *new* agent process that rebound it. Comparing the current
    /// [`SocketIdentity`] against the one captured in [`Self::bind_socket`]
    /// ensures a handle only ever unlinks its own socket, never the
    /// replacement's. A handle that never bound (`bound_socket == None`) owns
    /// nothing and skips removal entirely - the safe default, since
    /// unconditional removal is precisely the bug being fixed.
    #[cfg(unix)]
    fn unlink_socket_if_owned(&self) {
        let Some(bound_socket) = self.bound_socket else {
            return; // Never bound a socket -> nothing of ours to remove.
        };

        let current_socket = match std::fs::metadata(&self.socket_path) {
            Ok(metadata) => SocketIdentity::from_metadata(&metadata),
            // Nothing at the path (already gone) or unreadable: nothing to do.
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

    /// Spawn a background task to handle client connection
    ///
    /// # Panics
    ///
    /// Panics if the instant-prompt cache cannot be created (e.g., if cache directory
    /// creation fails due to filesystem permissions).
    #[cfg(unix)]
    fn spawn_client_handler(&self, stream: UnixStream) {
        // Computed from `self` before the fields below are cloned into the
        // spawned task; see `connection_deadline()` for the rationale (#316).
        let connection_deadline = self.connection_deadline();
        let client_registry = Arc::clone(&self.client_registry);
        let semaphore = Arc::clone(&self.connection_semaphore);

        // Only the dependencies a connection actually needs to serve a
        // request travel into the spawned task; transport-only state
        // (listener, bound inode, rate limiter, the semaphore itself,
        // git_cache/watcher/language_cache which are only needed to
        // *construct* the handlers already captured in `handler_registry`)
        // stays on the server and is never cloned per connection (#359).
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
            // Fail fast rather than queueing when already at the concurrent
            // connection limit (#316): a blocking `acquire().await` here
            // would let a saturated semaphore queue legitimate requests
            // forever behind slow-loris connections that never release
            // their permit. `try_acquire()` rejects immediately instead.
            let Ok(_permit) = semaphore.try_acquire() else {
                Self::reject_connection_busy(stream);
                return;
            };

            // Whole-connection deadline (#316): each individual read()
            // already has a per-request timeout (`request_timeout()`), but
            // nothing capped total connection lifetime, so a client that
            // trickles bytes just under that per-read timeout (never
            // tripping it) could hold its semaphore permit forever. Wrapping
            // the whole handler in `timeout` bounds worst-case connection
            // lifetime regardless of read cadence; dropping the wrapped
            // future on elapse also drops `stream` (closing the socket) and
            // `_permit` (releasing it), so no extra cleanup is needed here.
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

    /// Close the connection without replying because the concurrent-connection
    /// semaphore is saturated (#316).
    ///
    /// The request is never read, so its response format is unknown, and any
    /// reply line would be printed verbatim into a prompt that asked for a
    /// rendered segment (#680). Closing instead is the fast-fail signal: the
    /// shells treat a missing reply exactly like an unreachable agent and
    /// fall back, and `gpy-agent` CLI clients report that the agent closed
    /// the connection without replying.
    #[cfg(unix)]
    fn reject_connection_busy(stream: UnixStream) {
        debug_log!(
            "server",
            "Rejecting connection: too many concurrent connections"
        );
        drop(stream);
    }

    /// Notify live-update subscribers about a response, if live updates are
    /// enabled and the response type warrants it.
    ///
    /// Kept on `EndpointHandle` (rather than moved to `ConnectionHandler`
    /// alongside the rest of per-connection serving) because it depends only
    /// on `config_manager`, which `EndpointHandle` already owns directly, and
    /// existing tests call it on an `EndpointHandle` instance. Delegates to
    /// [`notify_clients_for_response`], the same logic `ConnectionHandler`
    /// uses for its own per-connection copy.
    ///
    /// Production code no longer calls this directly (that path now runs
    /// through `ConnectionHandler::notify_clients`, see `spawn_client_handler`);
    /// it is kept `#[cfg(test)]` because `ipc::server::tests` still exercises
    /// it directly on an `EndpointHandle`.
    #[cfg(all(test, unix))]
    pub(crate) fn notify_clients(
        &self,
        client_registry: &Arc<ClientDirectory>,
        response: &Response,
        notify_path: Option<&str>,
    ) {
        notify_clients_for_response(&self.config_manager, client_registry, response, notify_path);
    }

    /// Stop the server and close all connections
    ///
    /// On Unix the socket file is only removed when this handle still owns the
    /// bound inode (#317): declining to unlink a socket a different agent has
    /// since rebound is a successful no-op, not a failure of `stop`.
    ///
    /// # Errors
    ///
    /// Returns an error if the socket file cannot be removed (non-Unix only).
    // The `Result` is load-bearing: the non-Unix path can fail, callers
    // (`agent::mod`) match on `Err`, and this is a stable public signature.
    // On Unix the ownership-checked unlink is best-effort, so that build sees
    // an always-`Ok` body -- hence the platform-specific allow.
    #[cfg_attr(
        unix,
        expect(
            clippy::unnecessary_wraps,
            reason = "the Result is load-bearing: the non-Unix path can fail, callers (agent::mod) match on Err, and this is a stable public signature; on Unix the ownership-checked unlink is best-effort, so that build sees an always-Ok body"
        )
    )]
    pub fn stop(&mut self) -> Result<()> {
        #[cfg(unix)]
        {
            // Drop the listener to stop accepting new connections, then remove
            // our socket file only if it is still ours.
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

    /// Get the socket path being used
    pub fn socket_path(&self) -> &str {
        self.socket_path.to_str().unwrap_or("")
    }

    /// Create a minimal test handle for integration testing
    ///
    /// This creates an `EndpointHandle` without binding a socket, suitable for
    /// testing code that needs an `EndpointHandle` but doesn't actually start the server.
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
        // The embedded theme and built-in config defaults, never the
        // developer's ~/.config/gpy (#664).
        let theme_manager =
            Arc::new(ThemeManager::builtin("default").expect("default theme should always load"));
        let config_manager = Arc::new(
            crate::config::manager::ConfigManager::with_defaults()
                .expect("default config should always load"),
        );

        // Create instant cache for testing
        let instant_cache = Arc::new(crate::cache::InstantPromptCache::new_for_test());

        // Create latency tracker for testing
        let latency_tracker = Arc::new(crate::ipc::LatencyTracker::new(100_usize));

        // Create language cache for testing
        let language_cache = crate::language::DetectionCache::new();

        // Create palette cache for testing
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

/// Native Windows stub: the IPC server is Unix-socket based and not yet
/// implemented on this platform (#284). Run under WSL instead.
#[cfg(not(unix))]
impl EndpointHandle {
    /// Start accepting client connections -- native Windows stub (#284).
    ///
    /// # Errors
    ///
    /// Always returns an error on this platform.
    pub async fn start(&mut self) -> Result<()> {
        Err(crate::ipc::native_windows_unsupported())
    }
}

// `content_token` feeds the repaint-doorbell coalescing throttle, which is Unix-only.
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

// Exercises `ConnectionHandler`, which only exists on Unix (#540).
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
                assert!(is_ssh, "the handler must echo the request's is_ssh (#826)");
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

    use super::EndpointHandle;
    use crate::config::manager::ConfigManager;
    use crate::git::cache::GitStatusCache;
    use crate::ipc::ClientDirectory;
    use crate::ipc::LatencyTracker;
    use crate::theme::ThemeManager;
    use std::os::unix::fs::MetadataExt;
    use std::os::unix::net::UnixListener;
    use std::path::Path;
    use std::sync::{Arc, Mutex};

    /// Build an unbound `EndpointHandle` targeting `socket_path` (no accept loop).
    fn build_handle(socket_path: &Path) -> EndpointHandle {
        EndpointHandle::builder()
            .socket_path(socket_path.to_path_buf())
            .client_registry(Arc::new(ClientDirectory::new()))
            .git_cache(Arc::new(GitStatusCache::new()))
            .config_manager(Arc::new(
                ConfigManager::with_defaults().expect("default config"),
            ))
            .watcher_slot(Arc::new(Mutex::new(None)))
            .theme_manager(Arc::new(
                ThemeManager::builtin("default").expect("default theme"),
            ))
            .instant_cache(Arc::new(crate::cache::InstantPromptCache::new_for_test()))
            .latency_tracker(Arc::new(LatencyTracker::new(100_usize)))
            .language_cache(crate::language::DetectionCache::new())
            .build()
            .expect("handle build")
    }

    fn inode_of(path: &Path) -> u64 {
        std::fs::metadata(path).expect("metadata").ino()
    }

    // Acceptance criterion 2: stop() must not unlink a socket bound by a
    // different process (a new agent that rebound the same path after a race).
    #[tokio::test]
    async fn stop_does_not_unlink_foreign_socket() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let socket_path = tmp.path().join("gpy.sock");

        let mut handle = build_handle(&socket_path);
        handle.bind_socket().expect("bind");

        // Simulate a different agent process rebinding the same path: drop our
        // listener, remove our socket, and bind a fresh listener (new inode).
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

    // No regression: stop() still removes the handle's own current socket.
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

    // The accept-loop cleanup path (cleanup_socket_file) must also honour
    // ownership and not unlink a socket rebound by another process.
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
        let mut handle = EndpointHandle::builder()
            .socket_path(socket_path.to_path_buf())
            .client_registry(Arc::new(ClientDirectory::new()))
            .git_cache(Arc::new(GitStatusCache::new()))
            .config_manager(Arc::new(
                ConfigManager::with_defaults().expect("default config"),
            ))
            .theme_manager(Arc::new(
                ThemeManager::builtin("default").expect("default theme"),
            ))
            .instant_cache(Arc::new(crate::cache::InstantPromptCache::new_for_test()))
            .latency_tracker(Arc::new(LatencyTracker::new(100_usize)))
            .language_cache(crate::language::DetectionCache::new())
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

    // #779: an agent whose socket was rebound by someone else exits, and
    // leaves the newcomer's socket alone.
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

    // #779: an untouched socket never makes the agent exit.
    #[tokio::test]
    async fn accept_loop_keeps_running_while_socket_is_owned() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let socket_path = tmp.path().join("gpy.sock");
        let (task, shutdown) = spawn_started_handle(&socket_path).await;

        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
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

    // A handle that never bound (bound_socket == None) owns nothing and must
    // never unlink a file sitting at its socket path.
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
mod runtime_dir_validation_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    use super::EndpointHandle;
    use std::fs;
    use std::os::unix::fs::{MetadataExt, PermissionsExt, symlink};

    #[test]
    fn accepts_owned_non_writable_directory() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let dir = tmp.path().join("gpy-rt");
        fs::create_dir_all(&dir).expect("mkdir");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).expect("chmod");

        let uid = fs::metadata(&dir).expect("meta").uid();
        EndpointHandle::validate_runtime_dir(&dir, uid).expect("owned 0700 dir must pass");
    }

    #[test]
    fn rejects_symlinked_directory() {
        // Attacker symlinks the runtime path to a directory they control.
        let tmp = tempfile::tempdir().expect("temp dir");
        let real = tmp.path().join("real");
        fs::create_dir_all(&real).expect("mkdir real");
        let link = tmp.path().join("gpy-rt");
        symlink(&real, &link).expect("symlink");

        let uid = fs::symlink_metadata(&link).expect("meta").uid();
        let err = EndpointHandle::validate_runtime_dir(&link, uid)
            .expect_err("a symlink must be rejected even when it points to a dir we own");
        assert!(err.to_string().contains("not a directory"), "{err}");
    }

    #[test]
    fn rejects_group_or_world_writable_directory() {
        let tmp = tempfile::tempdir().expect("temp dir");
        let dir = tmp.path().join("gpy-rt");
        fs::create_dir_all(&dir).expect("mkdir");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o777)).expect("chmod");

        let uid = fs::metadata(&dir).expect("meta").uid();
        let err = EndpointHandle::validate_runtime_dir(&dir, uid)
            .expect_err("a world-writable dir must be rejected");
        assert!(err.to_string().contains("writable"), "{err}");
    }

    #[test]
    fn rejects_directory_owned_by_another_uid() {
        // We cannot chown to a foreign uid without privilege, so assert the
        // ownership gate fires by claiming a uid that is not the owner.
        let tmp = tempfile::tempdir().expect("temp dir");
        let dir = tmp.path().join("gpy-rt");
        fs::create_dir_all(&dir).expect("mkdir");
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o700)).expect("chmod");

        let real_uid = fs::metadata(&dir).expect("meta").uid();
        let foreign_uid = real_uid.wrapping_add(1);
        let err = EndpointHandle::validate_runtime_dir(&dir, foreign_uid)
            .expect_err("a dir not owned by the expected uid must be rejected");
        assert!(err.to_string().contains("not owned"), "{err}");
    }
}

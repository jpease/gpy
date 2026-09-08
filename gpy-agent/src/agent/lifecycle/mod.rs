//! Agent daemon lifecycle utilities.
//!
//! This module owns the process-level mechanics around the background agent:
//! runtime path selection, socket discovery, version markers, compatibility
//! checks, and the Unix daemon startup path. The main [`crate::agent::Agent`]
//! owns runtime coordination once the daemon is alive; this module keeps the
//! preflight and launch concerns isolated from that event loop.

/// Agent startup and background process management.
pub mod start;
pub use start::start_background_agent;

#[cfg(unix)]
use crate::Error;
use crate::{Result, VERSION};
use std::env;
use std::path::PathBuf;
use std::sync::OnceLock;

#[cfg(unix)]
use std::os::unix::net::UnixListener;

/// Overrides the default Unix domain socket path for agent communication.
///
/// This is primarily used for testing or specific deployment scenarios
/// where the socket path needs to be fixed.
pub static SOCKET_OVERRIDE: OnceLock<PathBuf> = OnceLock::new();

/// Get the runtime directory for agent files.
///
/// Precedence (matches Fish `__gpy_runtime_root` and server default):
/// 1. `$XDG_RUNTIME_DIR/gpy`
/// 2. `$XDG_CACHE_HOME/gpy`
/// 3. `$HOME/.cache/gpy`
/// 4. `/tmp/gpy`
///
/// The precedence itself is computed by the pure, table-tested
/// [`crate::paths::runtime_root_for`] (#477); this wrapper owns only the
/// ambient env reads and the directory-creation side effect.
///
/// # Errors
///
/// Returns an error if the runtime directory cannot be created.
pub fn get_runtime_dir() -> Result<PathBuf> {
    let path = crate::paths::runtime_root_for(
        env::var("XDG_RUNTIME_DIR").ok().as_deref(),
        env::var("XDG_CACHE_HOME").ok().as_deref(),
        crate::paths::home_dir().as_deref(),
    );
    std::fs::create_dir_all(&path)?;
    Ok(path)
}

/// Get the socket path for the agent
///
/// # Errors
///
/// Returns an error if the runtime directory cannot be created.
pub fn get_socket_path() -> Result<PathBuf> {
    if let Some(override_path) = SOCKET_OVERRIDE.get() {
        return Ok(override_path.clone());
    }

    // Check for custom socket path in environment (matches Fish shell logic).
    // An empty value is "unset", not "the empty path": honouring `GPY_AGENT_SOCKET_PATH=`
    // used to make the agent resolve nothing while Bash and Zsh fell through
    // to the runtime root and talked to a socket nobody was bound to (#626).
    if let Some(custom_path) = socket_path_override() {
        return Ok(custom_path);
    }

    // Default: compute runtime directory
    Ok(get_runtime_dir()?.join("gpy.sock"))
}

/// The `GPY_AGENT_SOCKET_PATH` override, or `None` when it is unset or empty.
///
/// Shared by every socket resolver in the crate (`ipc::client`,
/// `ipc::server::handle`, and [`get_socket_path`]) so they cannot disagree
/// about what an empty override means -- all four implementations (agent,
/// Fish, Bash, Zsh) now read it as "no override" (#626).
#[must_use]
pub fn socket_path_override() -> Option<PathBuf> {
    std::env::var_os("GPY_AGENT_SOCKET_PATH")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

/// Get the version file path
///
/// # Errors
///
/// Returns an error if the runtime directory cannot be created.
pub fn get_version_file_path() -> Result<PathBuf> {
    Ok(get_runtime_dir()?.join("agent.version"))
}

/// Read the version of the currently running agent
#[must_use]
pub fn read_agent_version() -> Option<String> {
    let version_path = get_version_file_path().ok()?;
    std::fs::read_to_string(version_path)
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Write the current agent version to file
///
/// # Errors
///
/// Returns an error if the version file cannot be written.
pub fn write_agent_version() -> Result<()> {
    let version_path = get_version_file_path()?;
    std::fs::write(version_path, VERSION)?;
    Ok(())
}

/// Test socket binding to prevent race conditions (Unix only)
///
/// # Errors
///
/// Returns an error if socket binding fails.
#[cfg(unix)]
pub fn test_socket_bind(socket_path: &PathBuf) -> std::io::Result<()> {
    // Try to bind to the socket path - this is atomic and race-free
    // If successful, we have exclusive access to this path
    let listener = UnixListener::bind(socket_path)?;
    // Immediately drop the listener to release the binding
    // The actual agent will bind again when it starts
    drop(listener);
    // Clean up the socket file created by bind
    let _ = std::fs::remove_file(socket_path);
    Ok(())
}

/// Which step of a ping decided its answer (#539).
///
/// `ping_agent_blocking` collapses five distinct outcomes into `Ok(false)`, and
/// they carry very different evidence: `ECONNREFUSED` means nothing is
/// listening, while a read timeout means the agent *accepted* the connection
/// and is therefore alive. Shutdown detection has been treating them alike.
///
/// Counting them is `#[cfg(test)]` because it exists to let a test say why a
/// ping failed. It is relaxed atomics and no I/O on purpose: instrumenting this
/// path with stderr writes made the ubuntu failure stop reproducing across four
/// CI runs, so the measurement must not touch the timing it measures.
///
/// The counters are process-global. Under nextest, which runs one test per
/// process, that is exact. Under a threaded `cargo test` a concurrent ping from
/// another test can inflate them, so they are reported, never asserted on.
#[cfg(all(test, unix))]
pub(crate) mod ping_probe {
    use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};

    pub static ANSWERED: AtomicU32 = AtomicU32::new(0);
    pub static CONNECT_ERROR: AtomicU32 = AtomicU32::new(0);
    pub static CONNECT_TIMEOUT: AtomicU32 = AtomicU32::new(0);
    pub static WRITE_ERROR: AtomicU32 = AtomicU32::new(0);
    pub static READ_TIMEOUT: AtomicU32 = AtomicU32::new(0);
    pub static READ_CLOSED: AtomicU32 = AtomicU32::new(0);
    pub static UNRECOGNIZED: AtomicU32 = AtomicU32::new(0);
    /// `raw_os_error` of the most recent failed connect, or 0 if there was none.
    pub static LAST_CONNECT_ERRNO: AtomicI32 = AtomicI32::new(0);

    pub fn bump(counter: &AtomicU32) {
        counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn record_connect_errno(errno: i32) {
        LAST_CONNECT_ERRNO.store(errno, Ordering::Relaxed);
    }

    /// A one-line reading of every counter, for a failure message.
    pub fn summary() -> String {
        let read = |c: &AtomicU32| c.load(Ordering::Relaxed);
        format!(
            "answered={} connect_error={} (last errno {}) connect_timeout={} \
             write_error={} read_timeout={} read_closed={} unrecognized={}",
            read(&ANSWERED),
            read(&CONNECT_ERROR),
            LAST_CONNECT_ERRNO.load(Ordering::Relaxed),
            read(&CONNECT_TIMEOUT),
            read(&WRITE_ERROR),
            read(&READ_TIMEOUT),
            read(&READ_CLOSED),
            read(&UNRECOGNIZED),
        )
    }
}

/// Record a ping outcome when the probe is compiled in; a no-op otherwise.
#[cfg(unix)]
macro_rules! ping_outcome {
    ($counter:ident) => {{
        #[cfg(test)]
        crate::agent::lifecycle::ping_probe::bump(&crate::agent::lifecycle::ping_probe::$counter);
    }};
}

/// Record which way a connect failed, when the probe is compiled in (#539).
///
/// Split into two `#[cfg]`-gated definitions rather than one function with an
/// internal `#[cfg(test)]` branch: only the `#[cfg(not(test))]` body is
/// const-able (the `#[cfg(test)]` body calls non-const atomic bookkeeping), so
/// a single definition can never be `const fn` in both configurations at once.
#[cfg(all(unix, test))]
fn record_connect_failure(e: &std::io::Error) {
    ping_probe::record_connect_errno(e.raw_os_error().unwrap_or(-1_i32));
    ping_probe::bump(&ping_probe::CONNECT_ERROR);
}

/// Record which way a connect failed, when the probe is compiled in (#539).
///
/// No-op: the probe counters only exist under `#[cfg(test)]`. See the
/// `#[cfg(test)]` sibling definition above for the real implementation.
#[cfg(all(unix, not(test)))]
const fn record_connect_failure(e: &std::io::Error) {
    let _ = e;
}

/// Connect to the agent socket within the ping's connect budget.
///
/// `None` either way: the caller reports "no answer" for both. Which of the two
/// happened is recorded for the probe, because they are not equivalent
/// evidence -- a refused connect means nothing is listening, while a timeout
/// means the connect never resolved (#539).
#[cfg(unix)]
async fn ping_connect(socket_path: &PathBuf) -> Option<tokio::net::UnixStream> {
    match tokio::time::timeout(
        std::time::Duration::from_millis(500),
        tokio::net::UnixStream::connect(socket_path),
    )
    .await
    {
        Ok(Ok(stream)) => Some(stream),
        Ok(Err(e)) => {
            record_connect_failure(&e);
            None
        }
        Err(_) => {
            ping_outcome!(CONNECT_TIMEOUT);
            None
        }
    }
}

/// What a single ping attempt established, before a round of retries folds
/// them into an [`AgentLiveness`] verdict (#539, #548).
///
/// The distinction that matters: `NotListening` means nothing accepted the
/// connection at all (refused, or the connect never resolved in budget) --
/// weak evidence, since a busy local runtime can produce the same shape.
/// `AcceptedNoReply` means the agent *did* accept the connection and either
/// timed out, closed, or sent something unrecognized on the read -- strong
/// evidence something is there and alive, just not answering. Both used to
/// collapse into the same `Ok(false)`, and `ping_agent_liveness_with` read
/// them as identical evidence toward "dead."
#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum PingOutcome {
    /// The agent answered with a well-formed [`crate::ipc::Response::Ack`].
    Answered,
    /// Nothing accepted the connection.
    NotListening,
    /// The connection was accepted but never produced a recognized reply.
    AcceptedNoReply,
}

/// Ping the agent once and report which of [`PingOutcome`]'s three shapes the
/// attempt took.
///
/// Owns the actual connect/write/read logic; [`ping_agent_blocking`] is a
/// thin `Result<bool>` projection of it kept for its existing callers (#548).
///
/// # Errors
///
/// Returns an error if runtime creation fails, or if the ping message could
/// not be written to an accepted connection -- the connect succeeded, so
/// something is listening, but the exchange itself did not complete, which is
/// a local failure, not evidence about the agent (#546).
#[cfg(unix)]
fn ping_agent_outcome(socket_path: &PathBuf) -> Result<PingOutcome> {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    // A current-thread runtime, not the multi-threaded default (#539). One
    // connect, one write, one read needs no worker pool, and the default
    // spawned one per ping -- roughly thirty runtimes and a hundred-odd
    // threads over a single `wait_for_agent_shutdown`. That churn is the
    // likeliest source of a ping that fails locally without ever reaching the
    // agent, which is the failure this call site must not mistake for death.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::process("ping_runtime".to_owned(), e.to_string()))?;

    rt.block_on(async {
        let Some(stream) = ping_connect(socket_path).await else {
            return Ok(PingOutcome::NotListening);
        };

        let (mut reader, mut writer) = stream.into_split();

        // One write, message and newline together (#539). Sending them
        // separately let a server that answers the first write and closes --
        // which is the normal shape of a request/response server, and exactly
        // what this module's own fake agent does -- deliver EPIPE on the
        // second. On ubuntu CI that raced: the agent answered 17 pings, then
        // three in a row failed on the trailing newline, and shutdown detection
        // called the agent stopped. A ping must not depend on the peer keeping
        // the connection open between two halves of one message.
        let ping_msg = b"{\"op\":\"ping\"}\n";

        if let Err(e) = writer.write_all(ping_msg).await {
            ping_outcome!(WRITE_ERROR);
            // The connect succeeded, so something is listening on this socket.
            // Failing to finish the exchange says the ping did not complete,
            // never that the agent is gone -- report it as the local failure it
            // is so the caller classifies it as indeterminate (#546).
            return Err(Error::ipc(format!("ping write failed: {e}")));
        }

        let mut buffer = [0; 1024];
        match tokio::time::timeout(Duration::from_millis(500), reader.read(&mut buffer)).await {
            Ok(Ok(n)) if n > 0 => {
                let Some(chunk) = buffer.get(..n) else {
                    ping_outcome!(UNRECOGNIZED);
                    return Ok(PingOutcome::AcceptedNoReply);
                };
                let response = String::from_utf8_lossy(chunk);
                // A real reply is a `Response`, so decide on the decoded
                // value rather than on substrings of the raw bytes (#573):
                // `contains("ok")` matched any error message with "ok" in
                // it. The daemon renders `Ack` through the JSON formatter as
                // `{"status":"ok"}` (`ipc::ACK_WIRE_JSON`), not as serde's
                // `"Ack"`, so the decoder has to accept the formatter shape
                // or a live agent is never seen answering.
                if crate::ipc::decode_wire_reply(&response).is_ok_and(|reply| is_ack(&reply)) {
                    ping_outcome!(ANSWERED);
                    Ok(PingOutcome::Answered)
                } else {
                    ping_outcome!(UNRECOGNIZED);
                    Ok(PingOutcome::AcceptedNoReply)
                }
            }
            Err(_) => {
                ping_outcome!(READ_TIMEOUT);
                Ok(PingOutcome::AcceptedNoReply)
            }
            // `Ok(Ok(0))` is a clean close with no reply; `Ok(Err(_))` is a
            // read error on an accepted connection. Both mean the agent took
            // the connection and did not answer on it.
            Ok(_) => {
                ping_outcome!(READ_CLOSED);
                Ok(PingOutcome::AcceptedNoReply)
            }
        }
    })
}

/// Blocking version of `ping_agent` for startup coordination
///
/// A thin `Result<bool>` projection of [`ping_agent_outcome`] (#548): its
/// contract is unchanged for its callers, which only ever needed "did it
/// answer," never which of the non-answering shapes happened.
///
/// # Errors
///
/// Returns an error if runtime creation or ping operations fail.
#[cfg(unix)]
pub fn ping_agent_blocking(socket_path: &PathBuf) -> Result<bool> {
    Ok(matches!(
        ping_agent_outcome(socket_path)?,
        PingOutcome::Answered
    ))
}

/// Blocking version of `ping_agent` for startup coordination (native Windows stub).
///
/// Native Windows has no daemon to ping yet (#284); always reports "not running".
///
/// # Errors
///
/// Never returns an error on this platform.
#[cfg(not(unix))]
pub fn ping_agent_blocking(_socket_path: &PathBuf) -> Result<bool> {
    Ok(false)
}

/// Whether `response` is the agent's acknowledgement of a command (#573).
///
/// The one place that decides what "acknowledged" means, so the shutdown,
/// reload, and ping paths cannot drift apart. Deciding on the decoded value
/// rather than on substrings of the raw reply is the point: the old
/// `contains("ok")` check also matched an [`crate::ipc::Response::Error`]
/// whose message happened to contain "ok".
#[cfg(unix)]
#[must_use]
const fn is_ack(response: &crate::ipc::Response) -> bool {
    matches!(response, crate::ipc::Response::Ack)
}

/// Send `message` (a JSON op body, without a trailing newline) to the agent
/// listening at `socket_path` and decode its reply as a
/// [`crate::ipc::Response`] (#573).
///
/// The whole exchange -- connect, write, read -- is bounded by `budget`, so no
/// caller can hang on a wedged agent that accepts connections and never
/// answers. The message and its newline go out in a single write (#539): a
/// request/response server that answers and closes can deliver EPIPE on a
/// separate second write, which is how a ping to a live agent used to be read
/// as no answer.
///
/// [`ping_agent_outcome`] deliberately keeps its own connect/write/read rather
/// than calling this. Its callers need the difference between a write error, a
/// read timeout, a clean close, and an unrecognized body, and collapsing those
/// into one `Err` would throw away the evidence `ping_agent_liveness_with`
/// classifies on (#546, #548).
///
/// # Errors
///
/// Returns an error if the connect, write, or read fails, if the budget
/// elapses, if the agent closes without replying, or if the reply is not a
/// decodable `Response`.
#[cfg(unix)]
async fn request(
    socket_path: &std::path::Path,
    message: &str,
    budget: std::time::Duration,
) -> Result<crate::ipc::Response> {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    tokio::time::timeout(budget, async {
        let mut stream = UnixStream::connect(socket_path)
            .await
            .map_err(|e| Error::ipc(format!("Failed to connect to agent: {e}")))?;

        let mut payload = Vec::with_capacity(message.len().saturating_add(1));
        payload.extend_from_slice(message.as_bytes());
        payload.push(b'\n');
        stream
            .write_all(&payload)
            .await
            .map_err(|e| Error::ipc(format!("Failed to send command: {e}")))?;

        let mut buffer = [0u8; 1024];
        let n = stream
            .read(&mut buffer)
            .await
            .map_err(|e| Error::ipc(format!("Failed to read response: {e}")))?;
        if n == 0 {
            return Err(Error::ipc(
                "Agent closed the connection without replying".to_owned(),
            ));
        }
        let chunk = buffer
            .get(..n)
            .ok_or_else(|| Error::ipc("Invalid buffer slice".to_owned()))?;
        let text = String::from_utf8_lossy(chunk);
        crate::ipc::decode_wire_reply(&text)
    })
    .await
    .map_err(|_| Error::ipc("Timed out communicating with agent".to_owned()))?
}

/// Send shutdown command to agent
///
/// # Errors
///
/// Returns an error if the socket path cannot be computed, if the exchange
/// fails or times out, or if the agent answered with anything other than an
/// acknowledgement.
#[cfg(unix)]
pub async fn send_shutdown_command() -> Result<()> {
    // Bound the whole exchange like the ping/status paths: a wedged agent (one
    // not accepting, with a full socket backlog) must never hang
    // `gpy-agent stop` indefinitely — the caller falls back to a
    // signal/`pkill` instead.
    let socket_path = get_socket_path()?;
    let response = request(
        &socket_path,
        r#"{"op":"shutdown"}"#,
        std::time::Duration::from_secs(2),
    )
    .await?;

    if is_ack(&response) {
        Ok(())
    } else {
        Err(Error::ipc(format!(
            "Unexpected shutdown response: {response:?}"
        )))
    }
}

/// Send shutdown command to agent (native Windows stub).
///
/// Native Windows has no daemon to shut down yet (#284).
///
/// # Errors
///
/// Always returns an error on this platform.
#[cfg(not(unix))]
pub async fn send_shutdown_command() -> Result<()> {
    Err(crate::ipc::native_windows_unsupported())
}

/// Send config reload command to agent
///
/// Returns `Ok(true)` when an acknowledgement is received, `Ok(false)` if the
/// command could not be delivered or was not acknowledged (e.g., agent not
/// running, connection refused, read timeout, or an unexpected reply).
///
/// # Errors
///
/// Returns an error only when the socket path itself cannot be computed. Every
/// failure of the exchange is a `Ok(false)`, not an error: the caller's
/// question is "was the reload confirmed", and an unconfirmed reload must
/// never be reported as a success (#323).
#[cfg(unix)]
pub async fn send_config_reload_command() -> Result<bool> {
    let socket_path = get_socket_path()?;
    if !socket_path.exists() {
        return Ok(false);
    }

    let outcome = request(
        &socket_path,
        crate::ipc::protocol::CONFIG_RELOAD_JSON,
        std::time::Duration::from_secs(2),
    )
    .await;

    // A failed exchange, a read timeout, or an unrecognized reply all mean we
    // never confirmed the agent acknowledged the reload; reporting `Ok(true)`
    // here would tell the caller the config was reloaded when it may not have
    // been (#323).
    Ok(outcome.is_ok_and(|response| is_ack(&response)))
}

/// Send config reload command to agent (native Windows stub).
///
/// Native Windows has no daemon to reload yet (#284); always reports "not delivered".
///
/// # Errors
///
/// Never returns an error on this platform.
#[cfg(not(unix))]
pub async fn send_config_reload_command() -> Result<bool> {
    Ok(false)
}

/// Agent status information from IPC query
#[derive(Debug)]
pub struct AgentStatusInfo {
    /// The version of the running agent.
    pub version: String,
    /// The protocol version the agent is using.
    pub protocol_version: u8,
    /// The number of repositories currently being watched by the agent.
    pub watched_repos: usize,
    /// The number of clients currently registered with the agent.
    pub registered_clients: usize,
    /// The number of entries in the agent's cache.
    pub cache_entries: usize,
}

/// Query the running agent for its status via IPC
///
/// # Errors
///
/// Returns an error if the socket connection fails or the response cannot be parsed.
#[cfg(unix)]
pub async fn query_agent_status() -> Result<AgentStatusInfo> {
    use std::time::Duration;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let socket_path = get_socket_path()?;
    let stream = tokio::time::timeout(Duration::from_secs(2), UnixStream::connect(socket_path))
        .await
        .map_err(|_| Error::ipc("Timeout connecting to agent".to_owned()))?
        .map_err(|e| Error::ipc(format!("Failed to connect to agent: {e}")))?;

    let (mut reader, mut writer) = stream.into_split();

    // Send status query
    let status_msg = r#"{"op":"status"}"#;
    writer.write_all(status_msg.as_bytes()).await?;
    writer.write_all(b"\n").await?;

    // Read response
    let mut buffer = vec![0u8; 4096];
    let n = tokio::time::timeout(Duration::from_secs(2), reader.read(&mut buffer))
        .await
        .map_err(|_| Error::ipc("Timeout reading agent response".to_owned()))?
        .map_err(|e| Error::ipc(format!("Failed to read response: {e}")))?;

    if n == 0 {
        return Err(Error::ipc("Empty response from agent".to_owned()));
    }

    let chunk = buffer
        .get(..n)
        .ok_or_else(|| Error::ipc("Invalid buffer slice".to_owned()))?;
    let response = String::from_utf8_lossy(chunk);

    // Parse the JSON response
    // Expected format: {"AgentStatus":{"version":"0.1.0","protocol_version":1,"watched_repos":3,...}}
    parse_agent_status_response(&response)
}

/// Query the running agent for its status via IPC (native Windows stub).
///
/// Native Windows has no daemon to query yet (#284).
///
/// # Errors
///
/// Always returns an error on this platform.
#[cfg(not(unix))]
pub async fn query_agent_status() -> Result<AgentStatusInfo> {
    Err(crate::ipc::native_windows_unsupported())
}

/// Parse the agent status JSON response
///
/// # Errors
///
/// Returns an error if the JSON cannot be parsed or required fields are missing.
#[cfg(unix)]
fn parse_agent_status_response(json: &str) -> Result<AgentStatusInfo> {
    // Decode the reply as the `Response` the daemon actually serialized rather
    // than hand-walking `serde_json::Value` (#573). The hand-walk defaulted
    // every missing or mistyped field ("unknown", 0), so a malformed payload
    // was reported to the user as a plausible-looking status; a payload that
    // does not deserialize is now an error, which is what it is.
    match serde_json::from_str::<crate::ipc::Response>(json) {
        Ok(crate::ipc::Response::AgentStatus {
            version,
            protocol_version,
            watched_repos,
            registered_clients,
            cache_entries,
        }) => Ok(AgentStatusInfo {
            version,
            protocol_version,
            watched_repos,
            registered_clients,
            cache_entries,
        }),
        Ok(other) => Err(Error::ipc(format!(
            "Expected an AgentStatus response, got: {other:?}"
        ))),
        Err(e) => Err(Error::ipc(format!("Failed to parse JSON response: {e}"))),
    }
}

/// Check protocol version compatibility and warn if mismatched
///
/// Compares the running agent's protocol version against the expected version
/// and displays warnings if there's a MAJOR version mismatch.
pub fn check_protocol_version_compatibility(agent_protocol_version: u8) {
    use crate::ipc::protocol::PROTOCOL_VERSION;

    if agent_protocol_version != PROTOCOL_VERSION {
        println!();
        println!("⚠️  WARNING: Protocol Version Mismatch");
        println!("=================================================");
        println!("Running agent protocol version: {agent_protocol_version}");
        println!("Expected protocol version: {PROTOCOL_VERSION}");
        println!();
        println!("This may cause compatibility issues. Recommended actions:");
        println!("  1. Restart the agent: gpy-agent stop && gpy-agent start");
        println!("  2. If issues persist, reinstall: ./install-dev.fish");
        println!();
        println!("Fallback: Use oneshot mode if IPC fails:");
        println!("  gpy-agent oneshot git --cwd . --format ansi");
        println!("=================================================");
    }
}

/// Number of ping attempts before an existing socket is declared stale (#317).
///
/// A single missed 500ms ping is not proof the agent is dead: it may be
/// momentarily busy (blocked on a slow `canonicalize`, or under load) and miss
/// one round-trip while still alive. Retrying avoids orphaning a live daemon by
/// unlinking its socket and forking a duplicate. Worst-case wall time for a
/// socket that is bound-but-unresponsive is
/// `PING_STALE_ATTEMPTS * (~1s per ping) + (PING_STALE_ATTEMPTS - 1) * backoff`
/// (~2.4s here), acceptable for a one-shot CLI startup check. A genuinely dead
/// socket (connection refused) fails each attempt in microseconds, so the retry
/// loop stays cheap in the common "leftover socket" case.
#[cfg(unix)]
const PING_STALE_ATTEMPTS: u32 = 3;

/// Backoff slept between failed ping attempts (#317).
#[cfg(unix)]
const PING_STALE_BACKOFF: std::time::Duration = std::time::Duration::from_millis(200);

/// Maximum time to wait for a shutdown-signalled agent to stop responding
/// before proceeding to unlink its socket (#317).
///
/// Bounds the version-mismatch restart so a wedged old agent cannot hang
/// startup forever; if it elapses we proceed anyway. The IPC server's own
/// ownership-verified cleanup (`EndpointHandle::unlink_socket_if_owned`) is what
/// ultimately prevents the new agent's socket from being clobbered if the old
/// one exits after we rebind.
#[cfg(unix)]
const SHUTDOWN_WAIT_MAX: std::time::Duration = std::time::Duration::from_secs(3);

/// Poll interval while waiting for the old agent to stop responding (#317).
#[cfg(unix)]
const SHUTDOWN_WAIT_POLL: std::time::Duration = std::time::Duration::from_millis(100);

/// What a round of pings could establish about the agent (#539, #548).
///
/// The distinction that matters is between "the agent did not answer", "we
/// never managed to ask", and "the agent is definitely there but wedged". The
/// first two used to be reported as a dead agent, and acting on the second
/// one is destructive: it unlinks a live agent's socket and forks a duplicate
/// over it. The third one used to be folded into the first, which is also
/// wrong: an accepted-but-unanswered connection is strong evidence of life,
/// not death -- but it still cannot be left alone the way an unverifiable
/// round can, so it gets its own verdict and its own eviction path.
#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AgentLiveness {
    /// An attempt was answered. The agent is alive.
    Responding,
    /// Every attempt reached the socket and none was answered. This is the only
    /// verdict that justifies treating the socket as stale.
    NotResponding,
    /// Every attempt in the round connected successfully but got no valid
    /// reply (read timeout, read closed, or an unrecognized body). Strong
    /// evidence the agent is alive but wedged -- unlike `Indeterminate`, this
    /// justifies eviction, just not the silent unlink-as-dead treatment
    /// `NotResponding` gets (#548). A round mixing this with a `NotListening`
    /// attempt resolves to `NotResponding` instead: a genuinely dead agent
    /// that got replaced mid-round could plausibly produce a mixed round, so
    /// only a round of consistent accepted-no-reply evidence earns this
    /// verdict.
    AcceptedNoReply,
    /// At least one attempt could not be made at all -- the ping failed locally
    /// rather than being refused or ignored by the agent. Says nothing about
    /// the agent, and must never be read as death.
    Indeterminate,
}

/// Ping the agent, retrying before concluding anything (#317, #539).
///
/// See [`PING_STALE_ATTEMPTS`] for the retry count and timing rationale.
#[cfg(unix)]
fn ping_agent_liveness(socket_path: &PathBuf) -> AgentLiveness {
    ping_agent_liveness_with(|| ping_agent_outcome(socket_path), PING_STALE_BACKOFF)
}

/// [`ping_agent_liveness`] over an arbitrary ping, so the classification can be
/// tested without a socket and without waiting on real backoff.
///
/// A local failure anywhere in the round makes the whole round indeterminate,
/// even if other attempts came back cleanly unanswered: once one attempt never
/// reached the agent, "every attempt failed" is no longer evidence about the
/// agent. Erring toward `Indeterminate` costs a retry later; erring the other
/// way costs a running daemon.
#[cfg(unix)]
fn ping_agent_liveness_with<F>(mut ping: F, backoff: std::time::Duration) -> AgentLiveness
where
    F: FnMut() -> Result<PingOutcome>,
{
    let mut local_failure = false;
    let mut saw_not_listening = false;
    let mut saw_accepted_no_reply = false;

    for attempt in 0..PING_STALE_ATTEMPTS {
        match ping() {
            Ok(PingOutcome::Answered) => return AgentLiveness::Responding,
            Ok(PingOutcome::NotListening) => saw_not_listening = true,
            Ok(PingOutcome::AcceptedNoReply) => saw_accepted_no_reply = true,
            Err(_) => local_failure = true,
        }
        if attempt
            .checked_add(1)
            .is_some_and(|next| next < PING_STALE_ATTEMPTS)
        {
            std::thread::sleep(backoff);
        }
    }

    if local_failure {
        AgentLiveness::Indeterminate
    } else if saw_accepted_no_reply && !saw_not_listening {
        // Only a round of *consistent* accepted-no-reply evidence earns the
        // stronger verdict (#548): mixing in even one `NotListening` could be
        // a genuinely dead agent that a fresh one raced to replace mid-round,
        // so that mix resolves toward `NotResponding` below instead.
        AgentLiveness::AcceptedNoReply
    } else {
        AgentLiveness::NotResponding
    }
}

/// Poll the agent until it stops responding to pings, or `SHUTDOWN_WAIT_MAX`
/// elapses (#317).
///
/// Replaces a fixed 2s sleep that could unlink the socket while the old agent
/// was still alive. Returns early the moment the agent stops answering, and is
/// bounded so a stuck old agent cannot hang startup indefinitely.
///
/// Returns `true` once the agent is confirmed to have stopped responding, or
/// `false` if `SHUTDOWN_WAIT_MAX` elapses while it is still answering pings
/// (e.g. stuck draining an in-flight blocking task, #390) — callers that need
/// to report accurate shutdown status (as opposed to `check_and_cleanup_socket`'s
/// bounded-fallback-then-proceed-anyway use) should check this.
///
/// Each poll uses [`ping_agent_liveness`], not a single bare ping: a
/// live-but-busy agent can miss one 500ms ping under load (#317, and
/// reproduced for this function's own tests under a fully parallel test run,
/// #390), so a single miss must not be read as "stopped."
///
/// A round that could not be completed locally ([`AgentLiveness::Indeterminate`])
/// is not a shutdown. On ubuntu CI this function reported a confirmed shutdown
/// against a fake agent that had answered 29 pings and was still accepting
/// connections (#539); the pings had failed before ever reaching it, and every
/// such failure counted as evidence of death. An indeterminate round now keeps
/// polling, and if the budget runs out the answer is "not confirmed stopped" --
/// which is what this function's `false` already means.
#[cfg(unix)]
#[must_use]
pub fn wait_for_agent_shutdown(socket_path: &PathBuf) -> bool {
    let deadline = std::time::Instant::now()
        .checked_add(SHUTDOWN_WAIT_MAX)
        .unwrap_or_else(std::time::Instant::now);
    loop {
        if ping_agent_liveness(socket_path) == AgentLiveness::NotResponding {
            return true; // Reached the socket every time, never answered: confirmed stopped.
        }
        if std::time::Instant::now() >= deadline {
            return false; // Bounded fallback: still responding or unverifiable.
        }
        std::thread::sleep(SHUTDOWN_WAIT_POLL);
    }
}

/// Poll the agent until it stops responding to pings.
///
/// Native Windows has no daemon to wait on yet (#284); always reports "not
/// confirmed stopped" so `stop_agent` prints its indeterminate message rather
/// than claiming a shutdown it cannot observe.
#[cfg(not(unix))]
#[must_use]
pub fn wait_for_agent_shutdown(_socket_path: &PathBuf) -> bool {
    false
}

/// Evict a wedged agent.
///
/// Attempt a graceful shutdown, wait (bounded) for it to stop responding, then
/// unlink its socket and version marker regardless of whether that shutdown was
/// ever acknowledged, so a fresh agent can bind (#317).
///
/// Shared by the version-mismatch branch (a live but incompatible agent) and
/// the `AcceptedNoReply` branch (a live but unresponsive one, #548) -- both
/// are "give the old agent one real chance to leave, then replace it
/// regardless," and #548 reuses this rather than inventing a second path.
#[cfg(unix)]
fn evict_wedged_agent(socket_path: &PathBuf) {
    // Attempt graceful shutdown (best-effort; ignored on failure). A
    // current-thread runtime, matching ping_agent_outcome's fix (#539): one
    // connect-write-read behind a single `block_on` needs no worker pool, and
    // the multi-threaded default spawned one thread per core to do nothing.
    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        let _ = runtime.block_on(send_shutdown_command());
    }

    // Wait for the old agent to actually stop responding before unlinking,
    // rather than a fixed sleep that may unlink while it is still alive
    // (#317). Bounded so a stuck old agent cannot hang us; proceed to clean
    // up either way (bounded-fallback semantics).
    let _ = wait_for_agent_shutdown(socket_path);

    // Clean up socket and version file if they still exist.
    let _ = std::fs::remove_file(socket_path);
    if let Ok(version_path) = get_version_file_path() {
        let _ = std::fs::remove_file(version_path);
    }
}

/// Check if agent is already running and clean up stale socket if needed
///
/// Returns true if a compatible agent is already running, false if we should start a new one.
#[cfg(unix)]
#[must_use]
pub fn check_and_cleanup_socket(socket_path: &PathBuf) -> bool {
    if !socket_path.exists() {
        return false;
    }

    // Retry before declaring the socket stale (#317): a single missed 500ms
    // ping can happen to a live-but-busy agent. Only unlink its socket (and
    // fork a replacement) once every attempt has reached it and gone
    // unanswered.
    let liveness = ping_agent_liveness(socket_path);

    // A round we could not complete locally says nothing about the agent, and
    // the two ways of being wrong here are not symmetric (#539). Treating a
    // live agent as stale unlinks the socket out from under a running daemon
    // and forks a second one over it. Treating a dead agent as live costs this
    // one invocation an agent, and the next prompt tries again. So an
    // unverifiable answer leaves the socket alone.
    if liveness == AgentLiveness::Indeterminate {
        eprintln!("Could not verify whether the GPY agent is running; leaving its socket alone");
        return true;
    }

    // Every attempt reached the agent and none was answered -- strong
    // evidence it is alive but wedged, unlike `Indeterminate` (#548). Unlike
    // `NotResponding`, this cannot be silently cleaned up as if the agent
    // were dead: give it the same defined eviction path a version-mismatched
    // agent already gets, independent of version matching.
    if liveness == AgentLiveness::AcceptedNoReply {
        eprintln!(
            "GPY agent accepted a connection but did not answer across every retry; evicting the wedged agent..."
        );
        evict_wedged_agent(socket_path);
        return false; // Allow new agent to start
    }

    if liveness == AgentLiveness::Responding {
        // Agent is running - check version compatibility
        if let Some(running_version) = read_agent_version()
            && running_version != VERSION
        {
            eprintln!("Detected version mismatch (running: {running_version}, binary: {VERSION})");
            eprintln!("Restarting agent with new version...");
            evict_wedged_agent(socket_path);
            return false; // Allow new agent to start
        }

        eprintln!("GPY Agent is already running and responsive");
        true
    } else {
        // Socket exists but the agent is unresponsive across every retry -
        // clean up the stale socket so a fresh agent can bind.
        eprintln!("Cleaning up stale socket...");
        let _ = std::fs::remove_file(socket_path);
        if let Ok(version_path) = get_version_file_path() {
            let _ = std::fs::remove_file(version_path);
        }
        false
    }
}

#[cfg(all(test, unix))]
mod stale_socket_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    // NOTE: the ping-retry behaviour of `check_and_cleanup_socket` (acceptance
    // criterion 1) is exercised in `tests/stale_socket_tests.rs`, which can
    // redirect the runtime dir via `std::env::set_var`. Env mutation is
    // `unsafe` under edition 2024 and the lib crate denies `unsafe_code`, so
    // those isolated tests live in an integration binary instead. This unit
    // test covers only the env-free polling helper used by Fix C.

    use super::{SHUTDOWN_WAIT_MAX, wait_for_agent_shutdown};
    use crate::ipc::Response;
    use std::io::{Read, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    /// Exactly what the daemon puts on the wire for a `ping` or a
    /// `config_reload` it accepted: the JSON formatter's rendering of
    /// `Response::Ack`, `{"status":"ok"}` (`crate::ipc::ACK_WIRE_JSON`).
    ///
    /// `ack_wire_form_is_what_the_json_formatter_emits` pins this against
    /// the real formatter, so a fixture built on it cannot drift from the
    /// daemon the way the post-#573 `"Ack"` fixture did.
    const ACK_WIRE_FORM: &[u8] = b"{\"status\":\"ok\"}\n";

    // Fix C: the version-mismatch restart must not sleep a fixed 2s; it polls
    // and returns promptly once the old agent stops responding.
    #[test]
    fn wait_for_agent_shutdown_returns_promptly_when_dead() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("missing.sock"); // nothing listening

        let started = Instant::now();
        let stopped = wait_for_agent_shutdown(&socket_path);
        let elapsed = started.elapsed();

        assert!(
            stopped,
            "must report the agent as confirmed stopped when nothing is listening"
        );
        assert!(
            elapsed < Duration::from_secs(1),
            "must not wait the full fallback budget when the agent is already gone, took {elapsed:?}"
        );
    }

    use super::{AgentLiveness, PING_STALE_ATTEMPTS, PingOutcome, ping_agent_liveness_with};
    use crate::Error;
    use std::cell::RefCell;

    /// Drive the classifier over a scripted sequence of ping outcomes, with no
    /// backoff so the retry timing does not slow the suite down.
    fn classify(outcomes: Vec<crate::Result<PingOutcome>>) -> (AgentLiveness, usize) {
        let call_counter = RefCell::new(0_usize);
        let script = RefCell::new(outcomes.into_iter());
        let verdict = ping_agent_liveness_with(
            || {
                let mut call_count = call_counter.borrow_mut();
                *call_count = call_count.saturating_add(1);
                drop(call_count);
                script
                    .borrow_mut()
                    .next()
                    .unwrap_or_else(|| panic!("classifier asked for more pings than scripted"))
            },
            Duration::ZERO,
        );
        let calls = *call_counter.borrow();
        (verdict, calls)
    }

    /// A ping that never reached the agent, the way a runtime that cannot be
    /// built reports itself.
    ///
    /// # Errors
    ///
    /// Always; that is the point.
    fn local_failure() -> crate::Result<PingOutcome> {
        Err(Error::process(
            "ping_runtime".to_owned(),
            "simulated".to_owned(),
        ))
    }

    #[test]
    fn an_answered_ping_stops_the_round_immediately() {
        let (verdict, calls) = classify(vec![Ok(PingOutcome::Answered)]);
        assert_eq!(verdict, AgentLiveness::Responding);
        assert_eq!(calls, 1, "a responding agent must not be pinged again");
    }

    /// The real "confirmed stop" scenario (#548).
    ///
    /// Every attempt never even reached the agent (refused / connect-timeout),
    /// which is the only shape that justifies unlinking the socket and forking
    /// a replacement.
    #[test]
    fn pings_that_never_reach_the_agent_are_a_confirmed_stop() {
        let unanswered = (0..PING_STALE_ATTEMPTS)
            .map(|_| Ok(PingOutcome::NotListening))
            .collect();
        let (verdict, calls) = classify(unanswered);
        assert_eq!(verdict, AgentLiveness::NotResponding);
        assert_eq!(
            calls,
            usize::try_from(PING_STALE_ATTEMPTS).expect("PING_STALE_ATTEMPTS fits in usize")
        );
    }

    /// #548: a round that reached the agent on every attempt (connection
    /// accepted) but never got answered is strong evidence of a wedged-but-alive
    /// agent, not a dead one.
    ///
    /// It must resolve to `AcceptedNoReply`, not the `NotResponding` verdict
    /// that used to fold every unanswered shape together (renamed from the
    /// pre-#548
    /// `unanswered_pings_that_all_reached_the_agent_are_a_confirmed_stop`,
    /// whose name is no longer accurate under the tri-state split: reaching
    /// the agent every time is exactly the case that must NOT be read as a
    /// confirmed stop any more).
    #[test]
    fn unanswered_pings_that_all_reached_the_agent_are_accepted_no_reply() {
        let unanswered = (0..PING_STALE_ATTEMPTS)
            .map(|_| Ok(PingOutcome::AcceptedNoReply))
            .collect();
        let (verdict, calls) = classify(unanswered);
        assert_eq!(verdict, AgentLiveness::AcceptedNoReply);
        assert_eq!(
            calls,
            usize::try_from(PING_STALE_ATTEMPTS).expect("PING_STALE_ATTEMPTS fits in usize")
        );
    }

    /// #548: a single ping in the round that never reached the agent at all is
    /// enough to deny the `AcceptedNoReply` verdict, mirroring how a single
    /// local failure denies a confirmed-stop verdict below.
    ///
    /// A round with mixed evidence (some connects refused, some
    /// accepted-but-unanswered) could plausibly be a dead agent that a new one
    /// raced to replace mid-round, so it resolves toward `NotResponding` rather
    /// than claiming the stronger "definitely alive" verdict.
    #[test]
    fn a_mix_of_not_listening_and_accepted_no_reply_resolves_toward_not_responding() {
        let (verdict, calls) = classify(vec![
            Ok(PingOutcome::AcceptedNoReply),
            Ok(PingOutcome::NotListening),
            Ok(PingOutcome::AcceptedNoReply),
        ]);
        assert_eq!(verdict, AgentLiveness::NotResponding);
        assert_eq!(
            calls,
            usize::try_from(PING_STALE_ATTEMPTS).expect("PING_STALE_ATTEMPTS fits in usize")
        );
    }

    /// The #539 regression.
    ///
    /// A ping that never reached the agent is not evidence about the agent, and mixing one
    /// into an otherwise unanswered round must not produce a verdict that unlinks a live
    /// agent's socket.
    #[test]
    fn one_local_failure_makes_the_whole_round_indeterminate() {
        let (verdict, _) = classify(vec![
            Ok(PingOutcome::NotListening),
            local_failure(),
            Ok(PingOutcome::NotListening),
        ]);
        assert_eq!(
            verdict,
            AgentLiveness::Indeterminate,
            "a round containing a ping we never managed to send cannot confirm a shutdown"
        );
    }

    /// #548: the same guard, but against the new `AcceptedNoReply` evidence.
    ///
    /// Even overwhelming evidence that the agent is alive-but-wedged must not
    /// be trusted if part of the round never made it out of this process.
    #[test]
    fn a_local_failure_mixed_with_accepted_no_reply_is_still_indeterminate() {
        let (verdict, _) = classify(vec![
            Ok(PingOutcome::AcceptedNoReply),
            local_failure(),
            Ok(PingOutcome::AcceptedNoReply),
        ]);
        assert_eq!(
            verdict,
            AgentLiveness::Indeterminate,
            "a round containing a ping we never managed to send cannot confirm anything, \
             even when every other attempt reached the agent"
        );
    }

    #[test]
    fn a_round_of_only_local_failures_is_indeterminate() {
        let (verdict, _) = classify(vec![local_failure(), local_failure(), local_failure()]);
        assert_eq!(verdict, AgentLiveness::Indeterminate);
    }

    /// A local failure early must not stop the round: the agent may well answer
    /// on a later attempt, and that answer outranks everything before it.
    #[test]
    fn a_later_answer_outranks_an_earlier_local_failure() {
        let (verdict, calls) = classify(vec![local_failure(), Ok(PingOutcome::Answered)]);
        assert_eq!(verdict, AgentLiveness::Responding);
        assert_eq!(calls, 2);
    }

    /// A single aborted connection must not retire the fake agent (#539).
    ///
    /// `ECONNABORTED` is the normal way a queued connection reports that its
    /// client went away before `accept()` reached it -- exactly what a ping
    /// that hit its 500ms budget leaves behind -- and `EINTR` is a signal, not
    /// a failure. A real server retries both; this fixture used to treat every
    /// accept error as fatal, drop its listener, and leave the test measuring
    /// an agent that had genuinely departed. The cap stops a genuinely broken
    /// listener from spinning forever.
    const MAX_TRANSIENT_ACCEPT_ERRORS: u32 = 64;

    /// What the fake agent actually did, recorded with relaxed atomic stores so
    /// the server's timing is not perturbed -- stderr writes in this path made
    /// the #539 failure stop reproducing.
    struct FakeAgentCounters {
        answered: Arc<AtomicU32>,
        transient_errors: Arc<AtomicU32>,
        fatal_errno: Arc<AtomicI32>,
    }

    impl FakeAgentCounters {
        fn new() -> Self {
            Self {
                answered: Arc::new(AtomicU32::new(0)),
                transient_errors: Arc::new(AtomicU32::new(0)),
                fatal_errno: Arc::new(AtomicI32::new(0)),
            }
        }

        fn snapshot(&self) -> FakeAgentReport {
            FakeAgentReport {
                answered: self.answered.load(Ordering::Relaxed),
                transient_errors: self.transient_errors.load(Ordering::Relaxed),
                fatal_errno: self.fatal_errno.load(Ordering::Relaxed),
            }
        }
    }

    /// A reading of [`FakeAgentCounters`] taken the moment the verdict landed.
    struct FakeAgentReport {
        answered: u32,
        transient_errors: u32,
        fatal_errno: i32,
    }

    impl std::fmt::Display for FakeAgentReport {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(
                f,
                "agent answered {} pings, {} transient accept errors, fatal errno {}",
                self.answered, self.transient_errors, self.fatal_errno
            )
        }
    }

    impl FakeAgentReport {
        /// The premise of a "still responding" test: the agent was still there.
        fn assert_agent_stayed_up(&self) {
            assert_eq!(
                self.fatal_errno, 0_i32,
                "the fake agent stopped accepting; this run measured a departed \
                 fixture, not shutdown detection ({self})"
            );
            assert!(
                self.answered > 0,
                "the fake agent answered no pings, so nothing here exercised \
                 shutdown detection ({self})"
            );
        }
    }

    /// Serve pings on a blocking `accept()` loop until `stop` is set.
    fn spawn_fake_agent(
        listener: std::os::unix::net::UnixListener,
        stop: &Arc<AtomicBool>,
        counters: &FakeAgentCounters,
    ) -> std::thread::JoinHandle<()> {
        let stop_flag = Arc::clone(stop);
        let answered = Arc::clone(&counters.answered);
        let transient_errors = Arc::clone(&counters.transient_errors);
        let fatal_errno = Arc::clone(&counters.fatal_errno);

        std::thread::spawn(move || {
            loop {
                let mut stream = match listener.accept() {
                    Ok((stream, _)) => stream,
                    Err(e)
                        if matches!(
                            e.kind(),
                            std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::Interrupted
                        ) =>
                    {
                        if transient_errors.fetch_add(1, Ordering::Relaxed)
                            >= MAX_TRANSIENT_ACCEPT_ERRORS
                        {
                            fatal_errno
                                .store(e.raw_os_error().unwrap_or(-1_i32), Ordering::Relaxed);
                            break;
                        }
                        continue;
                    }
                    Err(e) => {
                        fatal_errno.store(e.raw_os_error().unwrap_or(-1_i32), Ordering::Relaxed);
                        break;
                    }
                };
                if stop_flag.load(Ordering::Relaxed) {
                    break;
                }
                let mut buf = [0_u8; 1024];
                let _ = stream.read(&mut buf);
                // The real wire form of `Response::Ack`: what the JSON
                // formatter emits for it, pinned by
                // `ack_wire_form_is_what_the_json_formatter_emits`.
                let _ = stream.write_all(ACK_WIRE_FORM);
                answered.fetch_add(1, Ordering::Relaxed);
            }
        })
    }

    /// #539: an agent that answers and immediately closes the connection is
    /// alive, and the ping must say so.
    ///
    /// This is the ordinary shape of a request/response server, and it is what
    /// the fake agent above does. While the ping was sent as two writes, the
    /// second one could land after the server had already replied and dropped
    /// the stream, and EPIPE there was reported as "no answer" -- for an agent
    /// that had just answered. Sending the whole message in one write removes
    /// the window rather than narrowing it.
    #[test]
    fn a_reply_then_close_is_still_a_live_agent() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("terse.sock");

        let listener =
            std::os::unix::net::UnixListener::bind(&socket_path).expect("bind fake socket");
        let server = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(ACK_WIRE_FORM);
            // Closes right here, before the caller could send anything more.
            drop(stream);
        });

        let alive = super::ping_agent_blocking(&socket_path);
        server.join().expect("join fake server thread");

        assert!(
            matches!(alive, Ok(true)),
            "an agent that replied and closed is alive; got {alive:?} (pings: {})",
            super::ping_probe::summary()
        );
    }

    // #390 follow-up: `gpy stop` must not report success while the agent is
    // still answering pings (e.g. stuck draining an in-flight blocking task).
    // A fake socket server that keeps answering "ok" forever simulates that
    // stuck-but-still-responding state.
    #[test]
    fn wait_for_agent_shutdown_reports_still_responding_on_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("alive.sock");

        // A real blocking `accept()` loop (no busy-polling) so responding to
        // a ping only ever waits on the kernel's own connection wakeup, not
        // on this thread getting rescheduled soon enough to poll — a
        // nonblocking poll loop was flaky under a fully parallel test run
        // (#390 follow-up) because scheduler jitter could occasionally miss
        // `ping_agent_blocking`'s 500ms connect/read budget.
        let listener =
            std::os::unix::net::UnixListener::bind(&socket_path).expect("bind fake socket");
        let stop = Arc::new(AtomicBool::new(false));
        let counters = FakeAgentCounters::new();
        let server = spawn_fake_agent(listener, &stop, &counters);

        let started = Instant::now();
        let stopped = wait_for_agent_shutdown(&socket_path);
        let elapsed = started.elapsed();
        let report = counters.snapshot();

        stop.store(true, Ordering::Relaxed);
        // Unblock the server's final pending `accept()` so it observes the
        // stop flag and exits instead of hanging the join.
        let _ = std::os::unix::net::UnixStream::connect(&socket_path);
        server.join().expect("join fake server thread");

        // Assert the premise before the conclusion, so a departed fixture
        // reports itself instead of masquerading as a shutdown-detection bug.
        report.assert_agent_stayed_up();

        assert!(
            !stopped,
            "must report the agent as still responding, not confirmed stopped \
             ({report}; pings: {})",
            super::ping_probe::summary()
        );
        assert!(
            elapsed >= SHUTDOWN_WAIT_MAX,
            "must wait the full bounded budget before giving up, took {elapsed:?}"
        );
    }

    // ---- #573: acknowledgements are decided by decoding, not by substring ----

    /// Pins the fixtures above to the daemon's real reply.
    ///
    /// `handle_ping` and the config-reload handler both answer `Response::Ack`,
    /// and the connection renders every non-status reply through the
    /// requested formatter (JSON by default), which writes `Ack` as
    /// `{"status":"ok"}`. #573 pinned the fixtures to serde's `"Ack"`
    /// instead, a shape no live daemon sends, so every CLI acknowledgement
    /// went unrecognised; both encodings decode now, and the fixture sends
    /// the one the daemon does.
    #[test]
    fn ack_wire_form_is_what_the_json_formatter_emits() {
        use crate::formatter::{Format, RenderContext, SegmentPosition, create_formatter};

        let config = crate::config::Config::default();
        let theme = crate::theme::ThemeConfig::default();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let rendered = create_formatter(Format::Json)
            .expect("JSON formatter exists")
            .render(&Response::Ack, &ctx)
            .expect("Ack renders");

        assert_eq!(rendered, crate::ipc::ACK_WIRE_JSON);
        assert_eq!(
            ACK_WIRE_FORM,
            format!("{rendered}\n").as_bytes(),
            "the test fixtures must send exactly what the daemon sends"
        );
    }

    /// Serde's own encoding of `Ack` (what a `Response` round-trips to) keeps
    /// decoding as well, so nothing that speaks the native form regresses.
    #[test]
    fn a_native_ack_over_the_wire_is_an_acknowledgement() {
        let response = request_against(b"\"Ack\"\n").expect("native Ack decodes");
        assert!(super::is_ack(&response), "got {response:?}");
    }

    /// The JSON formatter's error shape decodes as a failure, never as an
    /// acknowledgement -- even when its message contains "ok".
    #[test]
    fn a_formatter_error_reply_over_the_wire_is_not_an_acknowledgement() {
        let response = request_against(b"{\"error\":\"config not ok: parse failed\"}\n")
            .expect("a formatter Error reply decodes as a Response");
        assert!(
            matches!(&response, Response::Error { message } if message == "config not ok: parse failed"),
            "got {response:?}"
        );
        assert!(!super::is_ack(&response));
    }

    /// The #573 regression, stated directly.
    ///
    /// `Response::Error { message: "something not ok here" }` is a *failure*
    /// reply, and the old `response.contains("ok")` check matched it -- the
    /// assertion below proves the substring really is present, so this is not
    /// a hypothetical. Only `Response::Ack` is an acknowledgement.
    #[test]
    fn an_error_reply_containing_ok_is_not_an_acknowledgement() {
        let failure = Response::Error {
            message: "something not ok here".to_owned(),
        };
        let wire = serde_json::to_string(&failure).expect("Error response serializes");

        assert!(
            wire.contains("ok"),
            "premise: the old substring check would have matched this reply ({wire})"
        );
        assert!(
            !super::is_ack(&failure),
            "an Error reply is never an acknowledgement, whatever its message says"
        );
        assert!(super::is_ack(&Response::Ack));
    }

    /// Run one `request()` exchange against a fake agent that replies with
    /// `reply`, returning what the caller decoded.
    ///
    /// # Errors
    ///
    /// Whatever `request()` reported -- which is part of what these tests
    /// assert on, so it is returned rather than unwrapped here.
    // Test-fixture setup (tempdir, fake socket bind, test runtime, thread join)
    // panics on unexpected failure, matching this crate's test convention; the
    // function's own Result return type is for the real thing under test.
    #[allow(clippy::unwrap_in_result)]
    fn request_against(reply: &'static [u8]) -> crate::Result<Response> {
        let tmp = tempfile::tempdir().expect("tempdir");
        let socket_path = tmp.path().join("reply.sock");

        let listener =
            std::os::unix::net::UnixListener::bind(&socket_path).expect("bind fake socket");
        let server = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(reply);
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime");
        let outcome = runtime.block_on(super::request(
            &socket_path,
            r#"{"op":"config_reload"}"#,
            Duration::from_secs(2),
        ));
        server.join().expect("join fake server thread");

        outcome
    }

    /// The same regression over a real socket round-trip: an agent that
    /// answers a `config_reload` with an error mentioning "ok" must not be
    /// read as having reloaded.
    #[test]
    fn a_failure_reply_over_the_wire_is_not_an_acknowledgement() {
        let outcome =
            request_against(b"{\"Error\":{\"message\":\"config not ok: parse failed\"}}\n");

        let response = outcome.expect("a well-formed Error reply still decodes as a Response");
        assert!(
            !super::is_ack(&response),
            "a decoded Error reply must not be treated as an acknowledgement, got {response:?}"
        );
    }

    /// The positive half: the daemon's real `Ack` is recognized.
    #[test]
    fn an_ack_over_the_wire_is_an_acknowledgement() {
        let response = request_against(ACK_WIRE_FORM).expect("Ack decodes as a Response");
        assert!(super::is_ack(&response), "got {response:?}");
    }

    /// A reply that is not a `Response` at all is an error, not a silent
    /// "unacknowledged" -- the caller decides what to do with it.
    #[test]
    fn an_undecodable_reply_is_an_error() {
        let outcome = request_against(b"{\"op\":\"pong\"}\n");
        assert!(
            outcome.is_err(),
            "a reply that is not a Response must not decode, got {outcome:?}"
        );
    }

    #[test]
    fn agent_status_is_deserialized_from_the_response_enum() {
        let json = serde_json::to_string(&Response::AgentStatus {
            version: "9.9.9".to_owned(),
            protocol_version: 3_u8,
            watched_repos: 5_usize,
            registered_clients: 2_usize,
            cache_entries: 12_usize,
        })
        .expect("AgentStatus serializes");

        let info = super::parse_agent_status_response(&json).expect("round-trips");
        assert_eq!(info.version, "9.9.9");
        assert_eq!(info.protocol_version, 3_u8);
        assert_eq!(info.watched_repos, 5_usize);
        assert_eq!(info.registered_clients, 2_usize);
        assert_eq!(info.cache_entries, 12_usize);
    }

    /// #573: the hand-walked parser defaulted every field it could not read,
    /// so a payload missing `protocol_version` was reported to the user as a
    /// real status with protocol 0. It is a parse failure.
    #[test]
    fn a_malformed_agent_status_is_an_error_not_a_zeroed_status() {
        let partial = r#"{"AgentStatus":{"version":"9.9.9"}}"#;
        assert!(
            super::parse_agent_status_response(partial).is_err(),
            "a status payload missing required fields must not be reported as a status"
        );
        assert!(
            super::parse_agent_status_response("\"Ack\"").is_err(),
            "a non-status Response must not be reported as a status"
        );
        assert!(super::parse_agent_status_response("not json").is_err());
    }
}

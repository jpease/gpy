//! Agent daemon lifecycle: runtime paths, socket discovery, version markers,
//! compatibility checks, and Unix daemon startup.

/// Agent startup and background process management.
pub mod start;
pub use start::start_background_agent;

#[cfg(unix)]
use crate::Error;
#[cfg(unix)]
use crate::deadline::Deadline;
use crate::{Result, VERSION};
#[cfg(unix)]
use std::cmp::Ordering;
use std::path::{Path, PathBuf};
#[cfg(unix)]
use std::time::Duration;

#[cfg(unix)]
use crate::ipc::socket_identity::SocketIdentity;

/// Create the default runtime directory when the socket lives there (no
/// `--socket`/`GPY_AGENT_SOCKET_PATH` override). Only the start path calls this.
///
/// # Errors
/// Returns an error naming the runtime directory if it cannot be created.
pub fn ensure_runtime_dir() -> Result<()> {
    if crate::paths::socket_is_overridden() {
        return Ok(());
    }
    let path = crate::paths::runtime_dir();
    std::fs::create_dir_all(&path).map_err(|e| crate::Error::Agent {
        message: format!("cannot create runtime directory {}", path.display()),
        source: Some(Box::new(e)),
    })
}

/// How the socket's directory may be repaired, by where [`crate::paths::resolve_socket_path`]
/// takes the path from: GPY's own runtime directory is tightened, while a
/// user-chosen `--socket` or `GPY_AGENT_SOCKET_PATH` directory is never chmod'ed.
#[cfg(unix)]
#[must_use]
pub fn socket_dir_repair() -> crate::ipc::server::DirRepair {
    if crate::paths::socket_is_overridden() {
        crate::ipc::server::DirRepair::Refuse
    } else {
        crate::ipc::server::DirRepair::Tighten
    }
}

/// Version marker path: `<runtime>/agent.version` for the default socket,
/// `<socket>.version` for an overridden one so agents never share a marker.
#[must_use]
pub fn get_version_file_path() -> PathBuf {
    crate::paths::socket_override().map_or_else(
        || version_file_for(None, &crate::paths::runtime_dir()),
        |socket| version_file_for(Some(&socket), Path::new("")),
    )
}

/// Version marker path for `socket_override`, or the default under `runtime_dir`.
fn version_file_for(socket_override: Option<&Path>, runtime_dir: &Path) -> PathBuf {
    socket_override.map_or_else(
        || runtime_dir.join("agent.version"),
        |socket| {
            let mut marker = socket.as_os_str().to_owned();
            marker.push(".version");
            PathBuf::from(marker)
        },
    )
}

/// A parsed `MAJOR.MINOR.PATCH[-prerelease]` version.
#[cfg(unix)]
type AgentVersion<'a> = ((u64, u64, u64), Option<&'a str>);

/// Parse an agent version, ignoring surrounding whitespace.
#[cfg(unix)]
fn parse_agent_version(raw: &str) -> Option<AgentVersion<'_>> {
    let trimmed = raw.trim();
    let (core, pre) = match trimmed.split_once('-') {
        Some((core, pre)) if !pre.is_empty() => (core, Some(pre)),
        Some(_) => return None,
        None => (trimmed, None),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.parse().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some(((major, minor, patch), pre))
}

/// Order two parsed versions; a pre-release sorts before its release.
#[cfg(unix)]
fn compare_agent_versions(left: &AgentVersion<'_>, right: &AgentVersion<'_>) -> Ordering {
    left.0.cmp(&right.0).then_with(|| match (left.1, right.1) {
        (None, None) => Ordering::Equal,
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (Some(left_pre), Some(right_pre)) => left_pre.cmp(right_pre),
    })
}

/// Whether a running agent should be replaced: only if older or unparsable,
/// so a corrupt marker self-heals.
#[cfg(unix)]
fn should_replace(running: &str, binary: &str) -> bool {
    let Some(running_version) = parse_agent_version(running) else {
        return true;
    };
    parse_agent_version(binary).map_or_else(
        || running.trim() != binary.trim(),
        |binary_version| {
            compare_agent_versions(&running_version, &binary_version) == Ordering::Less
        },
    )
}

/// Read the version of the currently running agent.
#[must_use]
pub fn read_agent_version() -> Option<String> {
    let version_path = get_version_file_path();
    std::fs::read_to_string(version_path)
        .ok()
        .map(|s| s.trim().to_owned())
}

/// Write the current agent version to file.
///
/// # Errors
/// Returns an error if the version file cannot be written.
pub fn write_agent_version() -> Result<()> {
    std::fs::write(get_version_file_path(), VERSION)?;
    Ok(())
}

/// How long a ping waits for the agent to accept the connection.
#[cfg(unix)]
const PING_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

/// How long a ping waits for the reply once the connection is accepted.
#[cfg(unix)]
const PING_REPLY_TIMEOUT: Duration = Duration::from_millis(500);

/// Connect within the ping's connect budget; `None` when nothing accepted.
#[cfg(unix)]
async fn ping_connect(socket_path: &Path) -> Option<tokio::net::UnixStream> {
    tokio::time::timeout(
        PING_CONNECT_TIMEOUT,
        tokio::net::UnixStream::connect(socket_path),
    )
    .await
    .ok()?
    .ok()
}

/// Outcome of one ping attempt. `NotListening` is weak evidence (a busy
/// runtime looks the same); `AcceptedNoReply` means something is alive.
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

/// Ping the agent once and classify the attempt.
///
/// # Errors
/// Returns an error if runtime creation fails or the write to an accepted
/// connection fails (a local failure, not evidence about the agent).
#[cfg(unix)]
fn ping_agent_outcome(socket_path: &Path) -> Result<PingOutcome> {
    // Current-thread: a pool per ping churns threads enough to make pings fail
    // locally, which callers would mistake for agent death.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::process("ping_runtime", e.to_string()))?;

    rt.block_on(async {
        let Some(stream) = ping_connect(socket_path).await else {
            return Ok(PingOutcome::NotListening);
        };
        // The reply is a `Response` decoded as such: a substring check would
        // match any error message containing "ok". The daemon renders `Ack`
        // through the JSON formatter (`ipc::ACK_WIRE_JSON`), which the decoder
        // accepts alongside serde's own encoding.
        match tokio::time::timeout(PING_REPLY_TIMEOUT, exchange(stream, r#"{"op":"ping"}"#)).await {
            Ok(Ok(reply)) if is_ack(&reply) => Ok(PingOutcome::Answered),
            // Something is listening; report as indeterminate, not stopped.
            Ok(Err(ExchangeError::Send(e))) => Err(Error::ipc(format!("ping write failed: {e}"))),
            // Timeout, clean close, read error, or a reply that is not an ack:
            // accepted but unanswered.
            Ok(Ok(_) | Err(_)) | Err(_) => Ok(PingOutcome::AcceptedNoReply),
        }
    })
}

/// Blocking ping: whether the agent answered.
///
/// # Errors
/// Returns an error if runtime creation or ping operations fail.
#[cfg(unix)]
pub fn ping_agent_blocking(socket_path: &Path) -> Result<bool> {
    Ok(matches!(
        ping_agent_outcome(socket_path)?,
        PingOutcome::Answered
    ))
}

/// Blocking ping (native Windows stub: no daemon, always `false`).
///
/// # Errors
/// Never returns an error on this platform.
#[cfg(not(unix))]
pub fn ping_agent_blocking(_socket_path: &Path) -> Result<bool> {
    Ok(false)
}

/// Whether `response` is an acknowledgement. Decides on the decoded value, since
/// a substring check would match an `Error` whose message contains "ok".
#[cfg(unix)]
#[must_use]
const fn is_ack(response: &crate::ipc::Response) -> bool {
    matches!(response, crate::ipc::Response::Ack)
}

/// Why an exchange on an open connection failed.
#[cfg(unix)]
enum ExchangeError {
    /// The message could not be written.
    Send(std::io::Error),
    /// The reply line could not be read.
    Read(std::io::Error),
    /// The agent closed the connection without replying.
    Closed,
    /// The reply did not decode.
    Reply(Error),
}

#[cfg(unix)]
impl From<ExchangeError> for Error {
    fn from(failure: ExchangeError) -> Self {
        match failure {
            ExchangeError::Send(e) => Self::ipc(format!("Failed to send command: {e}")),
            ExchangeError::Read(e) => Self::ipc(format!("Failed to read response: {e}")),
            ExchangeError::Closed => {
                Self::ipc("Agent closed the connection without replying".to_owned())
            }
            ExchangeError::Reply(e) => e,
        }
    }
}

/// Send `message` (JSON op, no trailing newline) over `stream` and decode the
/// one-line reply. Message and newline go in one write to avoid EPIPE from a
/// server that answers and closes. Unbounded in time: callers wrap it in a
/// timeout.
#[cfg(unix)]
async fn exchange(
    mut stream: tokio::net::UnixStream,
    message: &str,
) -> std::result::Result<crate::ipc::Response, ExchangeError> {
    use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt};

    let mut payload = Vec::with_capacity(message.len().saturating_add(1));
    payload.extend_from_slice(message.as_bytes());
    payload.push(b'\n');
    stream
        .write_all(&payload)
        .await
        .map_err(ExchangeError::Send)?;

    // A whole line: an error reply quoting a config line can be long.
    let limit = u64::try_from(crate::security::MAX_MESSAGE_SIZE).unwrap_or(u64::MAX);
    let mut reply = Vec::new();
    tokio::io::BufReader::new(stream.take(limit))
        .read_until(b'\n', &mut reply)
        .await
        .map_err(ExchangeError::Read)?;
    if reply.is_empty() {
        return Err(ExchangeError::Closed);
    }
    crate::ipc::decode_wire_reply(&String::from_utf8_lossy(&reply)).map_err(ExchangeError::Reply)
}

/// Connect to the agent, send `message` and decode its one-line reply, all
/// within `budget`.
///
/// # Errors
/// Returns an error if the exchange fails, the budget elapses, the agent
/// closes without replying, or the reply does not decode.
#[cfg(unix)]
#[allow(
    clippy::map_err_ignore,
    reason = "the budget's Elapsed carries no detail and attaching it would append to the CLI error text"
)]
async fn request(
    socket_path: &Path,
    message: &str,
    budget: Duration,
) -> Result<crate::ipc::Response> {
    tokio::time::timeout(budget, async {
        let stream = tokio::net::UnixStream::connect(socket_path)
            .await
            .map_err(|e| Error::ipc(format!("Failed to connect to agent: {e}")))?;
        Ok(exchange(stream, message).await?)
    })
    .await
    .map_err(|_| Error::ipc("Timed out communicating with agent".to_owned()))?
}

/// Send shutdown command to agent.
///
/// # Errors
/// Returns an error if the exchange fails or times out, or the reply is not an ack.
#[cfg(unix)]
pub async fn send_shutdown_command() -> Result<()> {
    // Bounded so a wedged agent cannot hang `gpy-agent stop`.
    let socket_path = crate::paths::get_socket_path()?;
    let response = request(&socket_path, r#"{"op":"shutdown"}"#, REQUEST_BUDGET).await?;

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
/// # Errors
/// Always returns an error on this platform.
#[cfg(not(unix))]
pub async fn send_shutdown_command() -> Result<()> {
    Err(crate::ipc::native_windows_unsupported())
}

/// What became of a config reload request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigReload {
    /// The agent loaded and applied the configuration.
    Applied,
    /// No agent is listening, so the configuration applies at the next start.
    NotRunning,
    /// The agent refused the configuration and kept its previous one; holds
    /// the agent's reason.
    Rejected(String),
}

/// Ask the running agent to reload its configuration.
///
/// # Errors
/// Returns an error if the socket path cannot be computed, or a listening
/// agent does not answer the request (timeout, closed connection, or a reply
/// that is neither an ack nor an error).
#[cfg(unix)]
pub async fn send_config_reload_command() -> Result<ConfigReload> {
    let socket_path = crate::paths::get_socket_path()?;
    if !socket_path.exists() {
        return Ok(ConfigReload::NotRunning);
    }

    match request(
        &socket_path,
        crate::ipc::protocol::CONFIG_RELOAD_JSON,
        REQUEST_BUDGET,
    )
    .await
    {
        Ok(crate::ipc::Response::Ack) => Ok(ConfigReload::Applied),
        Ok(crate::ipc::Response::Error { message }) => {
            Ok(ConfigReload::Rejected(message.trim_end().to_owned()))
        }
        Ok(other) => Err(Error::ipc(format!(
            "Unexpected config reload response: {other:?}"
        ))),
        Err(_) if socket_is_stale(&socket_path) => Ok(ConfigReload::NotRunning),
        Err(e) => Err(e),
    }
}

/// Ask the running agent to reload its configuration (native Windows stub:
/// there is never a running agent).
///
/// # Errors
/// Never returns an error on this platform.
#[cfg(not(unix))]
pub async fn send_config_reload_command() -> Result<ConfigReload> {
    Ok(ConfigReload::NotRunning)
}

/// Whether nothing is listening on `socket_path` (connection refused or file
/// gone); an accepted connection means a live agent.
#[cfg(unix)]
#[must_use]
pub fn socket_is_stale(socket_path: &Path) -> bool {
    match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(_) => false,
        Err(e) => matches!(
            e.kind(),
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
        ),
    }
}

/// Native Windows has no daemon to probe; never claim a stale socket.
#[cfg(not(unix))]
#[must_use]
pub fn socket_is_stale(_socket_path: &Path) -> bool {
    false
}

/// Agent status information from IPC query.
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

/// Query the running agent for its status via IPC.
///
/// # Errors
/// Returns an error if the socket connection fails or the response is not an
/// agent status.
#[cfg(unix)]
pub async fn query_agent_status() -> Result<AgentStatusInfo> {
    query_status_at(&crate::paths::get_socket_path()?).await
}

/// [`query_agent_status`] against an explicit socket.
#[cfg(unix)]
async fn query_status_at(socket_path: &Path) -> Result<AgentStatusInfo> {
    agent_status_from(request(socket_path, r#"{"op":"status"}"#, REQUEST_BUDGET).await?)
}

/// Query agent status (native Windows stub).
///
/// # Errors
/// Always returns an error on this platform.
#[cfg(not(unix))]
pub async fn query_agent_status() -> Result<AgentStatusInfo> {
    Err(crate::ipc::native_windows_unsupported())
}

/// The status carried by an agent's reply.
///
/// # Errors
/// Returns an error if `response` is anything but an `AgentStatus`.
#[cfg(unix)]
fn agent_status_from(response: crate::ipc::Response) -> Result<AgentStatusInfo> {
    match response {
        crate::ipc::Response::AgentStatus {
            version,
            protocol_version,
            watched_repos,
            registered_clients,
            cache_entries,
        } => Ok(AgentStatusInfo {
            version,
            protocol_version,
            watched_repos,
            registered_clients,
            cache_entries,
        }),
        other => Err(Error::ipc(format!(
            "Expected an AgentStatus response, got: {other:?}"
        ))),
    }
}

/// Warn if the running agent's protocol version differs from ours.
pub fn check_protocol_version_compatibility(agent_protocol_version: u8) {
    use crate::ipc::protocol::PROTOCOL_VERSION;

    if agent_protocol_version != PROTOCOL_VERSION {
        println!();
        crate::commands::output::warn("Protocol version mismatch");
        println!("=================================================");
        println!("Running agent protocol version: {agent_protocol_version}");
        println!("Expected protocol version: {PROTOCOL_VERSION}");
        println!();
        println!("This may cause compatibility issues. Recommended actions:");
        println!("  1. Restart the agent: gpy-agent stop && gpy-agent start");
        println!("  2. If issues persist, update gpy and re-run: gpy setup");
        println!();
        println!("Fallback: Use oneshot mode if IPC fails:");
        println!("  gpy-agent oneshot git --cwd . --format ansi");
        println!("=================================================");
    }
}

/// Budget for one request/reply exchange with the agent (shutdown, status,
/// config reload): the agent answers in microseconds unless wedged.
#[cfg(unix)]
const REQUEST_BUDGET: Duration = Duration::from_secs(2);

/// Ping attempts before an existing socket is declared stale. A busy live agent
/// can miss one ping; unlinking its socket would orphan it. Refused
/// connects fail in microseconds, so retries are cheap for leftover sockets.
#[cfg(unix)]
const PING_STALE_ATTEMPTS: u32 = 3;

/// Backoff slept between failed ping attempts.
#[cfg(unix)]
const PING_STALE_BACKOFF: Duration = Duration::from_millis(200);

/// Max wait for a shutdown-signalled agent to stop responding before unlinking
/// its socket. Ownership-verified cleanup in the server protects the new socket
/// if the old agent exits late.
#[cfg(unix)]
const SHUTDOWN_WAIT_MAX: Duration = Duration::from_secs(3);

/// Poll interval while waiting for the old agent to stop responding.
#[cfg(unix)]
const SHUTDOWN_WAIT_POLL: Duration = Duration::from_millis(100);

/// What a round of pings established. Reading "never managed to ask" as death
/// would unlink a live agent's socket and fork a duplicate.
#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum AgentLiveness {
    /// An attempt was answered. The agent is alive.
    Responding,
    /// Every attempt reached the socket unanswered; the only stale verdict.
    NotResponding,
    /// Every attempt connected but got no valid reply: alive but wedged.
    /// Justifies eviction, not silent unlink.
    AcceptedNoReply,
    /// A ping failed locally; says nothing about the agent.
    Indeterminate,
}

/// Ping the agent, retrying before concluding anything.
#[cfg(unix)]
fn ping_agent_liveness(socket_path: &Path) -> AgentLiveness {
    ping_agent_liveness_with(|| ping_agent_outcome(socket_path), PING_STALE_BACKOFF)
}

/// [`ping_agent_liveness`] over an arbitrary ping, for tests. Any local
/// failure makes the round indeterminate: erring that way costs a retry,
/// the other way a running daemon.
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
        // A mixed round may be a dead agent replaced mid-round.
        AgentLiveness::AcceptedNoReply
    } else {
        AgentLiveness::NotResponding
    }
}

/// Poll until the agent stops responding to pings, bounded by `SHUTDOWN_WAIT_MAX`.
/// Returns `false` if still answering (or only indeterminate) when the budget ends.
#[cfg(unix)]
#[must_use]
pub fn wait_for_agent_shutdown(socket_path: &Path) -> bool {
    let deadline = Deadline::after(SHUTDOWN_WAIT_MAX);
    loop {
        if ping_agent_liveness(socket_path) == AgentLiveness::NotResponding {
            return true; // Reached the socket every time, never answered: confirmed stopped.
        }
        if deadline.expired() {
            return false; // Bounded fallback: still responding or unverifiable.
        }
        std::thread::sleep(SHUTDOWN_WAIT_POLL);
    }
}

/// Native Windows stub: always "not confirmed stopped".
#[cfg(not(unix))]
#[must_use]
pub fn wait_for_agent_shutdown(_socket_path: &Path) -> bool {
    false
}

/// What the identity-aware wait could establish about an evicted agent.
#[cfg(unix)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum ShutdownWait {
    /// The path is gone or holds a different socket, which is not ours to remove.
    Released,
    /// The captured socket is still at the path and no longer answers.
    Stopped,
    /// The captured socket still answers (or cannot be verified) at the deadline.
    TimedOut,
}

/// Whether the socket captured as `identity` has left `socket_path`: the path
/// is missing or holds a different inode. Any other `stat` failure is not
/// evidence of release.
#[cfg(unix)]
fn socket_released(socket_path: &Path, identity: SocketIdentity) -> bool {
    match SocketIdentity::at(socket_path) {
        Ok(current) => current != identity,
        Err(e) => e.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Wait until the socket captured as `identity` stops answering or leaves the
/// path, bounded by `SHUTDOWN_WAIT_MAX`. A replacement's answers end the wait.
#[cfg(unix)]
pub(crate) fn wait_for_socket_release(
    socket_path: &Path,
    identity: SocketIdentity,
) -> ShutdownWait {
    let deadline = Deadline::after(SHUTDOWN_WAIT_MAX);
    loop {
        if socket_released(socket_path, identity) {
            return ShutdownWait::Released;
        }
        if ping_agent_liveness(socket_path) == AgentLiveness::NotResponding {
            // The round may have reached a replacement bound mid-round.
            return if socket_released(socket_path, identity) {
                ShutdownWait::Released
            } else {
                ShutdownWait::Stopped
            };
        }
        if deadline.expired() {
            return ShutdownWait::TimedOut;
        }
        std::thread::sleep(SHUTDOWN_WAIT_POLL);
    }
}

/// Remove the socket and version marker only while the path still holds the
/// captured socket (the check-then-unlink window is inherent).
#[cfg(unix)]
fn remove_socket_if_unchanged(socket_path: &Path, identity: SocketIdentity) {
    if socket_released(socket_path, identity) {
        return;
    }
    let _ = std::fs::remove_file(socket_path);
    let _ = std::fs::remove_file(get_version_file_path());
}

/// Evict a wedged or incompatible agent: request shutdown, wait for release,
/// then unlink only the captured socket. Returns `true` when a responsive
/// replacement already holds the path, so the caller must not fork another.
#[cfg(unix)]
fn evict_wedged_agent(socket_path: &Path, identity: SocketIdentity) -> bool {
    // Best-effort graceful shutdown.
    if let Ok(runtime) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        let _ = runtime.block_on(send_shutdown_command());
    }

    // Proceed either way, but only ever unlink the captured socket.
    if wait_for_socket_release(socket_path, identity) != ShutdownWait::Released {
        remove_socket_if_unchanged(socket_path, identity);
        return false;
    }

    // A different, answering agent is the replacement: do not fork another.
    if SocketIdentity::at(socket_path).is_ok()
        && matches!(ping_agent_outcome(socket_path), Ok(PingOutcome::Answered))
    {
        eprintln!("GPY Agent is already running and responsive");
        return true;
    }
    false
}

/// Check for a running agent and clean up a stale socket. Returns `true` if a
/// compatible agent is running.
#[cfg(unix)]
#[must_use]
pub fn check_and_cleanup_socket(socket_path: &Path) -> bool {
    // Every later wait and unlink concerns this socket, never a replacement.
    let Ok(identity) = SocketIdentity::at(socket_path) else {
        return false;
    };

    let liveness = ping_agent_liveness(socket_path);

    // Wrongly treating a live agent as stale forks a duplicate; wrongly treating
    // a dead one as live costs one prompt. So leave an unverifiable socket alone.
    if liveness == AgentLiveness::Indeterminate {
        eprintln!("Could not verify whether the GPY agent is running; leaving its socket alone");
        return true;
    }

    // Alive but wedged: evict rather than silently unlink.
    if liveness == AgentLiveness::AcceptedNoReply {
        eprintln!(
            "GPY agent accepted a connection but did not answer across every retry; evicting the wedged agent..."
        );
        return evict_wedged_agent(socket_path, identity);
    }

    if liveness == AgentLiveness::Responding {
        if let Some(running_version) = read_agent_version()
            && running_version != VERSION
        {
            if should_replace(&running_version, VERSION) {
                eprintln!(
                    "Detected version mismatch (running: {running_version}, binary: {VERSION})"
                );
                eprintln!("Restarting agent with this binary's version ({VERSION})...");
                return evict_wedged_agent(socket_path, identity);
            }
            eprintln!(
                "A newer GPY agent ({running_version}) is already running; not replacing it with {VERSION}"
            );
            return true;
        }

        eprintln!("GPY Agent is already running and responsive");
        true
    } else {
        // Unresponsive across every retry; unlink unless already replaced.
        eprintln!("Cleaning up stale socket...");
        remove_socket_if_unchanged(socket_path, identity);
        false
    }
}

#[cfg(all(test, unix))]
mod version_order_tests {
    use super::{should_replace, version_file_for};
    use std::path::{Path, PathBuf};

    /// # Panics
    /// Panics when a case's verdict differs from the expected one.
    #[test]
    fn only_older_or_unparsable_running_versions_are_replaced() {
        let cases = [
            ("0.0.9", "0.1.0", true),
            ("9.9.9", "0.1.0", false),
            ("0.1.0", "0.1.0", false),
            ("0.1.0-rc1", "0.1.0", true),
            ("0.1.0", "0.1.0-rc1", false),
            ("0.0.0-old", "0.1.0", true),
            ("garbage", "0.1.0", true),
            ("0.1", "0.1.0", true),
            ("0.1.0-", "0.1.0", true),
            (" 0.1.0\n", "0.1.0", false),
            ("0.10.0", "0.9.0", false),
        ];
        for (running, binary, expected) in cases {
            assert_eq!(
                should_replace(running, binary),
                expected,
                "should_replace({running:?}, {binary:?})"
            );
        }
    }

    /// # Panics
    /// Panics when a marker path differs from the expected one.
    #[test]
    fn version_marker_follows_the_socket_override() {
        let runtime = Path::new("/run/gpy");
        assert_eq!(
            version_file_for(None, runtime),
            PathBuf::from("/run/gpy/agent.version")
        );
        assert_eq!(
            version_file_for(Some(Path::new("/x/dev.sock")), runtime),
            PathBuf::from("/x/dev.sock.version")
        );
    }
}

#[cfg(all(test, unix))]
mod stale_socket_tests {
    #![allow(clippy::unwrap_used)]
    #![allow(clippy::expect_used)]
    #![allow(clippy::panic)]
    #![allow(clippy::missing_panics_doc)]
    #![allow(missing_docs)]

    // `check_and_cleanup_socket` retries are tested in `tests/stale_socket_tests.rs`,
    // which needs `unsafe` env mutation the lib denies.
    use super::{SHUTDOWN_WAIT_MAX, wait_for_agent_shutdown};
    use crate::ipc::Response;
    use std::io::{Read, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, AtomicI32, AtomicU32, Ordering};
    use std::time::{Duration, Instant};

    /// The daemon's wire `Response::Ack`, pinned by
    /// `ack_wire_form_is_what_the_json_formatter_emits`.
    const ACK_WIRE_FORM: &[u8] = b"{\"status\":\"ok\"}\n";

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

    /// A rebound path releases the captured socket even though something answers.
    #[test]
    fn wait_for_socket_release_sees_a_rebound_path_as_released() {
        use super::{ShutdownWait, wait_for_socket_release};
        use crate::ipc::socket_identity::SocketIdentity;

        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("agent.sock");
        let old = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        let identity = SocketIdentity::at(&socket_path).unwrap();
        std::fs::remove_file(&socket_path).unwrap();
        let _replacement = std::os::unix::net::UnixListener::bind(&socket_path).unwrap();
        drop(old);

        let started = Instant::now();
        let outcome = wait_for_socket_release(&socket_path, identity);
        let elapsed = started.elapsed();

        assert_eq!(outcome, ShutdownWait::Released);
        assert!(
            elapsed < Duration::from_secs(1),
            "a rebound path must end the wait at once, took {elapsed:?}"
        );
    }

    use super::{AgentLiveness, PING_STALE_ATTEMPTS, PingOutcome, ping_agent_liveness_with};
    use crate::Error;
    use std::cell::RefCell;

    /// Drive the classifier over scripted ping outcomes with no backoff.
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

    /// A ping that failed locally.
    ///
    /// # Errors
    /// Always.
    fn local_failure() -> crate::Result<PingOutcome> {
        Err(Error::process("ping_runtime", "simulated".to_owned()))
    }

    #[test]
    fn an_answered_ping_stops_the_round_immediately() {
        let (verdict, calls) = classify(vec![Ok(PingOutcome::Answered)]);
        assert_eq!(verdict, AgentLiveness::Responding);
        assert_eq!(calls, 1, "a responding agent must not be pinged again");
    }

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

    /// All pings accepted but unanswered: wedged, not stopped.
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

    /// One `NotListening` denies `AcceptedNoReply`: it may be a replaced dead agent.
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

    /// A ping that never left this process is not evidence about the agent.
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

    /// A local failure also overrides `AcceptedNoReply` evidence.
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

    /// A later answer outranks an earlier local failure.
    #[test]
    fn a_later_answer_outranks_an_earlier_local_failure() {
        let (verdict, calls) = classify(vec![local_failure(), Ok(PingOutcome::Answered)]);
        assert_eq!(verdict, AgentLiveness::Responding);
        assert_eq!(calls, 2);
    }

    /// Cap on retried `ECONNABORTED` (a ping that timed out in the backlog) and
    /// `EINTR` accept errors before the fake agent gives up.
    const MAX_TRANSIENT_ACCEPT_ERRORS: u32 = 64;

    /// What the fake agent did; relaxed atomics, since stderr writes hide the races.
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
                let _ = stream.write_all(ACK_WIRE_FORM);
                answered.fetch_add(1, Ordering::Relaxed);
            }
        })
    }

    /// An agent that answers and immediately closes is alive.
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
            drop(stream);
        });

        let alive = super::ping_agent_blocking(&socket_path);
        server.join().expect("join fake server thread");

        assert!(
            matches!(alive, Ok(true)),
            "an agent that replied and closed is alive; got {alive:?}"
        );
    }

    // `gpy stop` must not report success while the agent still answers pings.
    #[test]
    fn wait_for_agent_shutdown_reports_still_responding_on_timeout() {
        let tmp = tempfile::tempdir().unwrap();
        let socket_path = tmp.path().join("alive.sock");

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
        let _ = std::os::unix::net::UnixStream::connect(&socket_path);
        server.join().expect("join fake server thread");

        // Premise first, so a departed fixture is not mistaken for a bug.
        report.assert_agent_stayed_up();

        assert!(
            !stopped,
            "must report the agent as still responding, not confirmed stopped ({report})"
        );
        assert!(
            elapsed >= SHUTDOWN_WAIT_MAX,
            "must wait the full bounded budget before giving up, took {elapsed:?}"
        );
    }

    /// Pins the fixtures to the daemon's real JSON-formatted `Ack` reply.
    #[test]
    fn ack_wire_form_is_what_the_json_formatter_emits() {
        use crate::formatter::{Format, RenderContext, SegmentPosition, create_formatter};

        let config = crate::config::Config::default();
        let theme = crate::theme::ThemeConfig::default();
        let ctx = RenderContext::new(&config, &theme, SegmentPosition::MIDDLE);
        let rendered = create_formatter(Format::Json)
            .render(&Response::Ack, &ctx)
            .expect("Ack renders");

        assert_eq!(rendered, crate::ipc::ACK_WIRE_JSON);
        assert_eq!(
            ACK_WIRE_FORM,
            format!("{rendered}\n").as_bytes(),
            "the test fixtures must send exactly what the daemon sends"
        );
    }

    /// Serde's own `"Ack"` encoding also decodes as an acknowledgement.
    #[test]
    fn a_native_ack_over_the_wire_is_an_acknowledgement() {
        let response = request_against(b"\"Ack\"\n").expect("native Ack decodes");
        assert!(super::is_ack(&response), "got {response:?}");
    }

    /// A formatter error whose message contains "ok" is not an acknowledgement.
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

    /// A serde `Error` reply containing "ok" is not an acknowledgement.
    #[test]
    fn an_error_reply_containing_ok_is_not_an_acknowledgement() {
        let failure = Response::Error {
            message: "something not ok here".to_owned(),
        };
        let wire = serde_json::to_string(&failure).expect("Error response serializes");

        assert!(
            wire.contains("ok"),
            "premise: a substring check would match this reply ({wire})"
        );
        assert!(
            !super::is_ack(&failure),
            "an Error reply is never an acknowledgement, whatever its message says"
        );
        assert!(super::is_ack(&Response::Ack));
    }

    /// Run one `request()` exchange against a fake agent replying with `reply`.
    ///
    /// # Errors
    /// Whatever `request()` reported.
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

    /// A wire `Error` reply to `config_reload` mentioning "ok" is not a reload.
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

    /// The daemon's real `Ack` is recognized.
    #[test]
    fn an_ack_over_the_wire_is_an_acknowledgement() {
        let response = request_against(ACK_WIRE_FORM).expect("Ack decodes as a Response");
        assert!(super::is_ack(&response), "got {response:?}");
    }

    /// A reply that is not a `Response` is an error.
    #[test]
    fn an_undecodable_reply_is_an_error() {
        let outcome = request_against(b"{\"op\":\"pong\"}\n");
        assert!(
            outcome.is_err(),
            "a reply that is not a Response must not decode, got {outcome:?}"
        );
    }

    /// Run `query_status_at` against a fake agent that sends `reply`.
    ///
    /// # Errors
    /// Whatever the query reported.
    fn query_status_against(reply: Vec<u8>) -> crate::Result<super::AgentStatusInfo> {
        let tmp = tempfile::tempdir().expect("tempdir");
        let socket_path = tmp.path().join("status.sock");

        let listener =
            std::os::unix::net::UnixListener::bind(&socket_path).expect("bind fake socket");
        let server = std::thread::spawn(move || {
            let Ok((mut stream, _)) = listener.accept() else {
                return;
            };
            let mut buf = [0_u8; 1024];
            let _ = stream.read(&mut buf);
            let _ = stream.write_all(&reply);
        });

        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("build test runtime");
        let outcome = runtime.block_on(super::query_status_at(&socket_path));
        server.join().expect("join fake server thread");

        outcome
    }

    /// A status line is parsed whole, however many reads it arrives in.
    #[test]
    fn a_status_reply_larger_than_one_read_buffer_parses() {
        let status = Response::AgentStatus {
            version: "9".repeat(16 * 1024),
            protocol_version: 3_u8,
            watched_repos: 5_usize,
            registered_clients: 2_usize,
            cache_entries: 12_usize,
        };
        let mut reply = serde_json::to_vec(&status).expect("AgentStatus serializes");
        reply.push(b'\n');
        assert!(reply.len() > 4096, "premise: the reply exceeds 4 KiB");

        let info = query_status_against(reply).expect("a long status line parses");
        assert_eq!(info.version.len(), 16 * 1024);
        assert_eq!(info.protocol_version, 3_u8);
        assert_eq!(info.watched_repos, 5_usize);
        assert_eq!(info.registered_clients, 2_usize);
        assert_eq!(info.cache_entries, 12_usize);
    }

    /// A partial status payload is an error, not a defaulted status.
    #[test]
    fn a_malformed_agent_status_is_an_error_not_a_zeroed_status() {
        assert!(
            query_status_against(b"{\"AgentStatus\":{\"version\":\"9.9.9\"}}\n".to_vec()).is_err(),
            "a status payload missing required fields must not be reported as a status"
        );
        assert!(
            query_status_against(b"\"Ack\"\n".to_vec()).is_err(),
            "a non-status Response must not be reported as a status"
        );
        assert!(query_status_against(b"not json\n".to_vec()).is_err());
    }
}

//! Talking to a running agent from the CLI: one request/reply exchange, the
//! liveness ping, and the typed shutdown, config reload and status requests.

use super::outcome::StatusProbe;
use super::{AgentStatusInfo, ConfigReload};
use crate::ipc::Response;
use crate::{Error, Result};
use std::io::ErrorKind;
use std::path::Path;
use std::time::Duration;

/// How long a ping waits for the agent to accept the connection.
const PING_CONNECT_TIMEOUT: Duration = Duration::from_millis(500);

/// How long a ping waits for the reply once the connection is accepted.
const PING_REPLY_TIMEOUT: Duration = Duration::from_millis(500);

/// Budget for one request/reply exchange with the agent (shutdown, status,
/// config reload): the agent answers in microseconds unless wedged.
const REQUEST_BUDGET: Duration = Duration::from_secs(2);

/// Connect within the ping's connect budget; `None` when nothing accepted.
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
/// runtime looks the same); the other non-answers mean something is alive.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(super) enum PingOutcome {
    /// The agent answered with a well-formed [`Response::Ack`].
    Answered,
    /// Nothing accepted the connection.
    NotListening,
    /// The connection was accepted but never produced a recognized reply
    /// (read timeout, other read error, or an undecodable reply).
    AcceptedNoReply,
    /// The agent closed or reset the connection within the reply window
    /// without answering: its accept loop is running but it is shedding load.
    ClosedUnanswered,
}

/// Ping the agent once and classify the attempt.
///
/// # Errors
/// Returns an error if runtime creation fails or the write to an accepted
/// connection fails for a reason other than the agent closing it (a local
/// failure, not evidence about the agent).
pub(super) fn ping_agent_outcome(socket_path: &Path) -> Result<PingOutcome> {
    // Current-thread: a pool per ping churns threads enough to make pings fail
    // locally, which callers would mistake for agent death.
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::agent(format!("cannot create the async runtime: {e}")))?;

    rt.block_on(async {
        let Some(stream) = ping_connect(socket_path).await else {
            return Ok(PingOutcome::NotListening);
        };
        // The reply is a `Response` decoded as such: a substring check would
        // match any error message containing "ok". The daemon renders `Ack`
        // through the JSON formatter (`ipc::ACK_WIRE_JSON`), which the decoder
        // accepts alongside serde's own encoding.
        // A read timeout is the wedged signature: the runtime accepted the
        // connection into its backlog but never served it.
        tokio::time::timeout(PING_REPLY_TIMEOUT, exchange(stream, r#"{"op":"ping"}"#))
            .await
            .map_or_else(|_| Ok(PingOutcome::AcceptedNoReply), classify_exchange)
    })
}

/// Whether `failure` is the agent closing or resetting the connection without
/// a reply. An agent over its connection limits does this without reading the
/// request, so the platform decides whether the client sees EOF (macOS),
/// `ECONNRESET` (Linux read) or `EPIPE` (a write after the close).
pub(super) fn closed_unanswered(failure: &ExchangeError) -> bool {
    match failure {
        ExchangeError::Closed => true,
        ExchangeError::Read(e) | ExchangeError::Send(e) => {
            matches!(e.kind(), ErrorKind::ConnectionReset | ErrorKind::BrokenPipe)
        }
        ExchangeError::Reply(_) => false,
    }
}

/// Classify a finished ping exchange.
///
/// # Errors
/// Returns an error for a write failure that is not the agent closing the
/// connection: a local failure, not evidence about the agent.
pub(super) fn classify_exchange(
    result: std::result::Result<Response, ExchangeError>,
) -> Result<PingOutcome> {
    match result {
        Ok(response) if is_ack(&response) => Ok(PingOutcome::Answered),
        Err(failure) if closed_unanswered(&failure) => Ok(PingOutcome::ClosedUnanswered),
        Err(ExchangeError::Send(e)) => Err(Error::ipc(format!("ping write failed: {e}"))),
        Ok(_) | Err(_) => Ok(PingOutcome::AcceptedNoReply),
    }
}

/// Why an exchange on an open connection failed.
#[derive(Debug)]
pub(super) enum ExchangeError {
    /// The message could not be written.
    Send(std::io::Error),
    /// The reply line could not be read.
    Read(std::io::Error),
    /// The agent closed the connection without replying.
    Closed,
    /// The reply did not decode.
    Reply(Error),
}

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
async fn exchange(
    mut stream: tokio::net::UnixStream,
    message: &str,
) -> std::result::Result<Response, ExchangeError> {
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

/// Why [`request`] failed, keeping the stage so callers can tell a refused
/// connection from a silent agent.
#[derive(Debug)]
pub(super) enum RequestFailure {
    /// The connection could not be made.
    Connect(std::io::Error),
    /// The budget elapsed.
    Timeout,
    /// The exchange on the open connection failed.
    Exchange(ExchangeError),
}

impl From<RequestFailure> for Error {
    fn from(failure: RequestFailure) -> Self {
        match failure {
            RequestFailure::Connect(e) => Self::ipc(format!("Failed to connect to agent: {e}")),
            RequestFailure::Timeout => Self::ipc("Timed out communicating with agent".to_owned()),
            RequestFailure::Exchange(inner) => inner.into(),
        }
    }
}

/// Connect to the agent, send `message` and decode its one-line reply, all
/// within `budget`.
///
/// # Errors
/// Returns the stage that failed: connect, the budget elapsing, or the
/// exchange (closed without a reply, I/O error, undecodable reply).
pub(super) async fn request(
    socket_path: &Path,
    message: &str,
    budget: Duration,
) -> std::result::Result<Response, RequestFailure> {
    tokio::time::timeout(budget, async {
        let stream = tokio::net::UnixStream::connect(socket_path)
            .await
            .map_err(RequestFailure::Connect)?;
        exchange(stream, message)
            .await
            .map_err(RequestFailure::Exchange)
    })
    .await
    .unwrap_or_else(|_| Err(RequestFailure::Timeout))
}

/// How a failed status request reads to `probe_status`.
enum ProbeClass {
    /// Nothing is listening.
    NotListening,
    /// The agent shed the connection without a reply.
    Busy,
    /// The agent accepted but did not answer.
    NoReply,
    /// The probe could not be made.
    Failed,
}

/// Classify one failed status request.
fn probe_class(failure: &RequestFailure) -> ProbeClass {
    match failure {
        RequestFailure::Connect(e)
            if matches!(e.kind(), ErrorKind::ConnectionRefused | ErrorKind::NotFound) =>
        {
            ProbeClass::NotListening
        }
        RequestFailure::Exchange(x) if closed_unanswered(x) => ProbeClass::Busy,
        RequestFailure::Connect(_) | RequestFailure::Exchange(ExchangeError::Send(_)) => {
            ProbeClass::Failed
        }
        RequestFailure::Timeout | RequestFailure::Exchange(_) => ProbeClass::NoReply,
    }
}

/// Map the result of one `status` request onto a [`StatusProbe`].
pub(super) fn status_probe_from(
    result: std::result::Result<Response, RequestFailure>,
) -> StatusProbe {
    match result {
        Ok(response) => match agent_status_from(response) {
            Ok(status) => StatusProbe::Answered(status),
            Err(e) => StatusProbe::NoReply(e),
        },
        Err(failure) => {
            let class = probe_class(&failure);
            let error = Error::from(failure);
            match class {
                ProbeClass::NotListening => StatusProbe::NotListening(error),
                ProbeClass::Busy => StatusProbe::Busy,
                ProbeClass::NoReply => StatusProbe::NoReply(error),
                ProbeClass::Failed => StatusProbe::Failed(error),
            }
        }
    }
}

/// Ask the agent at `socket_path` for its status once and say what became of
/// the attempt. This is the one place the CLI learns that an agent is busy.
#[must_use]
pub fn probe_status(socket_path: &Path) -> StatusProbe {
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(e) => {
            return StatusProbe::Failed(Error::agent(format!(
                "cannot create the async runtime: {e}"
            )));
        }
    };
    status_probe_from(runtime.block_on(request(socket_path, r#"{"op":"status"}"#, REQUEST_BUDGET)))
}

/// Blocking ping: whether the agent answered.
///
/// # Errors
/// Returns an error if runtime creation or ping operations fail.
pub fn ping_agent_blocking(socket_path: &Path) -> Result<bool> {
    Ok(matches!(
        ping_agent_outcome(socket_path)?,
        PingOutcome::Answered
    ))
}

/// Whether `response` is an acknowledgement. Decides on the decoded value, since
/// a substring check would match an `Error` whose message contains "ok".
#[must_use]
const fn is_ack(response: &Response) -> bool {
    matches!(response, Response::Ack)
}

/// Send shutdown command to agent.
///
/// # Errors
/// Returns an error if the exchange fails or times out, or the reply is not an ack.
pub async fn send_shutdown_command() -> Result<()> {
    // Bounded so a wedged agent cannot hang `gpy-agent stop`.
    let socket_path = crate::paths::get_socket_path()?;
    let response = request(&socket_path, r#"{"op":"shutdown"}"#, REQUEST_BUDGET).await?;

    if is_ack(&response) {
        Ok(())
    } else {
        crate::debug_log!("agent", "Unexpected shutdown response: {response:?}");
        Err(Error::ipc(format!(
            "agent at {} sent an unexpected reply to the shutdown request",
            socket_path.display()
        )))
    }
}

/// Ask the running agent to reload its configuration.
///
/// # Errors
/// Returns an error if the socket path cannot be computed, or a listening
/// agent does not answer the request (timeout, closed connection, or a reply
/// that is neither an ack nor an error).
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
        Ok(Response::Ack) => Ok(ConfigReload::Applied),
        Ok(Response::Error { message }) => {
            Ok(ConfigReload::Rejected(message.trim_end().to_owned()))
        }
        Ok(other) => {
            crate::debug_log!("agent", "Unexpected config reload response: {other:?}");
            Err(Error::ipc(format!(
                "agent at {} sent an unexpected reply to the config reload request",
                socket_path.display()
            )))
        }
        Err(_) if socket_is_stale(&socket_path) => Ok(ConfigReload::NotRunning),
        Err(e) => Err(e.into()),
    }
}

/// Whether nothing is listening on `socket_path` (connection refused or file
/// gone); an accepted connection means a live agent.
#[must_use]
pub fn socket_is_stale(socket_path: &Path) -> bool {
    match std::os::unix::net::UnixStream::connect(socket_path) {
        Ok(_) => false,
        Err(e) => matches!(e.kind(), ErrorKind::ConnectionRefused | ErrorKind::NotFound),
    }
}

/// Query the running agent for its status via IPC.
///
/// # Errors
/// Returns an error if the socket connection fails or the response is not an
/// agent status.
pub async fn query_agent_status() -> Result<AgentStatusInfo> {
    query_status_at(&crate::paths::get_socket_path()?).await
}

/// [`query_agent_status`] against an explicit socket.
async fn query_status_at(socket_path: &Path) -> Result<AgentStatusInfo> {
    agent_status_from(request(socket_path, r#"{"op":"status"}"#, REQUEST_BUDGET).await?)
}

/// Query the agent's status on a private runtime.
///
/// # Errors
/// Returns an error if the runtime cannot be built or the query fails.
pub(super) fn status_blocking(socket_path: &Path) -> Result<AgentStatusInfo> {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| Error::agent(format!("cannot create the async runtime: {e}")))?;
    runtime.block_on(query_status_at(socket_path))
}

/// The status carried by an agent's reply.
///
/// # Errors
/// Returns an error if `response` is anything but an `AgentStatus`.
fn agent_status_from(response: Response) -> Result<AgentStatusInfo> {
    match response {
        Response::AgentStatus {
            version,
            protocol_version,
            watched_repos,
            registered_clients,
            cache_entries,
            pid,
        } => Ok(AgentStatusInfo {
            version,
            protocol_version,
            watched_repos,
            registered_clients,
            cache_entries,
            pid,
        }),
        other => {
            crate::debug_log!("agent", "Expected an AgentStatus response, got: {other:?}");
            Err(Error::ipc(
                "the agent sent an unexpected reply to the status request",
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        ExchangeError, PingOutcome, RequestFailure, classify_exchange, closed_unanswered, is_ack,
        ping_agent_blocking, query_status_at, request, status_probe_from,
    };
    use crate::agent::lifecycle::outcome::StatusProbe;
    use crate::ipc::Response;
    use std::io::{Error as IoError, ErrorKind, Read, Write};
    use std::time::Duration;

    /// The daemon's wire `Response::Ack`, pinned by
    /// `ack_wire_form_is_what_the_json_formatter_emits`.
    const ACK_WIRE_FORM: &[u8] = b"{\"status\":\"ok\"}\n";

    #[test]
    fn a_connection_reset_read_is_closed_unanswered() {
        let outcome = classify_exchange(Err(ExchangeError::Read(IoError::from(
            ErrorKind::ConnectionReset,
        ))));
        assert!(
            matches!(outcome, Ok(PingOutcome::ClosedUnanswered)),
            "got {outcome:?}"
        );
    }

    #[test]
    fn a_clean_close_and_a_broken_pipe_write_are_closed_unanswered() {
        assert!(matches!(
            classify_exchange(Err(ExchangeError::Closed)),
            Ok(PingOutcome::ClosedUnanswered)
        ));
        assert!(matches!(
            classify_exchange(Err(ExchangeError::Send(IoError::from(
                ErrorKind::BrokenPipe
            )))),
            Ok(PingOutcome::ClosedUnanswered)
        ));
    }

    #[test]
    fn other_failures_are_not_closed_unanswered() {
        assert!(!closed_unanswered(&ExchangeError::Read(IoError::from(
            ErrorKind::TimedOut
        ))));
        assert!(matches!(
            classify_exchange(Err(ExchangeError::Read(IoError::from(ErrorKind::TimedOut)))),
            Ok(PingOutcome::AcceptedNoReply)
        ));
        assert!(
            classify_exchange(Err(ExchangeError::Send(IoError::from(
                ErrorKind::PermissionDenied
            ))))
            .is_err()
        );
        assert!(matches!(
            classify_exchange(Ok(crate::ipc::Response::Error {
                message: "no".to_owned()
            })),
            Ok(PingOutcome::AcceptedNoReply)
        ));
        assert!(matches!(
            classify_exchange(Ok(crate::ipc::Response::Ack)),
            Ok(PingOutcome::Answered)
        ));
    }

    #[test]
    fn a_refused_connect_is_not_listening_and_eacces_is_a_failure() {
        let refused = status_probe_from(Err(RequestFailure::Connect(IoError::from(
            ErrorKind::ConnectionRefused,
        ))));
        assert!(
            matches!(refused, StatusProbe::NotListening(_)),
            "got {refused:?}"
        );
        let denied = status_probe_from(Err(RequestFailure::Connect(IoError::from(
            ErrorKind::PermissionDenied,
        ))));
        assert!(matches!(denied, StatusProbe::Failed(_)), "got {denied:?}");
    }

    #[test]
    fn status_probe_maps_silence_and_shedding() {
        assert!(matches!(
            status_probe_from(Err(RequestFailure::Timeout)),
            StatusProbe::NoReply(_)
        ));
        assert!(matches!(
            status_probe_from(Err(RequestFailure::Exchange(ExchangeError::Closed))),
            StatusProbe::Busy
        ));
        assert!(matches!(
            status_probe_from(Err(RequestFailure::Exchange(ExchangeError::Send(
                IoError::from(ErrorKind::PermissionDenied)
            )))),
            StatusProbe::Failed(_)
        ));
        assert!(matches!(
            status_probe_from(Err(RequestFailure::Exchange(ExchangeError::Read(
                IoError::from(ErrorKind::UnexpectedEof)
            )))),
            StatusProbe::NoReply(_)
        ));
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

        let alive = ping_agent_blocking(&socket_path);
        server.join().expect("join fake server thread");

        assert!(
            matches!(alive, Ok(true)),
            "an agent that replied and closed is alive; got {alive:?}"
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
        assert!(is_ack(&response), "got {response:?}");
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
        assert!(!is_ack(&response));
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
            !is_ack(&failure),
            "an Error reply is never an acknowledgement, whatever its message says"
        );
        assert!(is_ack(&Response::Ack));
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
        let outcome = runtime
            .block_on(request(
                &socket_path,
                r#"{"op":"config_reload"}"#,
                Duration::from_secs(2),
            ))
            .map_err(crate::Error::from);
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
            !is_ack(&response),
            "a decoded Error reply must not be treated as an acknowledgement, got {response:?}"
        );
    }

    /// The daemon's real `Ack` is recognized.
    #[test]
    fn an_ack_over_the_wire_is_an_acknowledgement() {
        let response = request_against(ACK_WIRE_FORM).expect("Ack decodes as a Response");
        assert!(is_ack(&response), "got {response:?}");
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
        let outcome = runtime.block_on(query_status_at(&socket_path));
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
            pid: Some(4321_u32),
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
        assert_eq!(info.pid, Some(4321_u32));
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

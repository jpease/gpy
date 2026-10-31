//! Blocking-free building blocks for talking to a running agent from the CLI:
//! one request/reply exchange, the liveness ping, and the typed status probe.

use super::outcome::StatusProbe;
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
pub(super) const REQUEST_BUDGET: Duration = Duration::from_secs(2);

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
        .map_err(|e| Error::process("ping_runtime", e.to_string()))?;

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
        Ok(response) if super::is_ack(&response) => Ok(PingOutcome::Answered),
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
        Ok(response) => match super::agent_status_from(response) {
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
        Err(e) => return StatusProbe::Failed(Error::process("status_runtime", e.to_string())),
    };
    status_probe_from(runtime.block_on(request(socket_path, r#"{"op":"status"}"#, REQUEST_BUDGET)))
}

#[cfg(test)]
mod tests {
    use super::{
        ExchangeError, PingOutcome, RequestFailure, classify_exchange, closed_unanswered,
        status_probe_from,
    };
    use crate::agent::lifecycle::outcome::StatusProbe;
    use std::io::{Error as IoError, ErrorKind};

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
}

//! A minimal Unix-socket responder that models the agent's behaviour on each
//! accepted connection (alive, wedged, shedding load, exiting on shutdown).
//! Unix-only: it uses `UnixListener`.

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::time::Duration;

/// The daemon's exact ping ack (`ipc::wire::ACK_WIRE_JSON`).
pub const ACK_WIRE_FORM: &[u8] = b"{\"status\":\"ok\"}\n";

/// How long the fake waits for a client's request bytes before moving on.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_millis(200);

/// How a fake agent behaves on each accepted connection.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Behaviour {
    /// Answers pings with the wire ack and `status` with its status.
    Alive,
    /// Accepts and reads the request, then holds the connection open without
    /// replying until the agent stops (a runtime that never serves it).
    Wedged,
    /// Answers until a shutdown request, then stops listening.
    AliveUntilShutdown,
    /// Accepts and drops every connection at once without reading it, as the
    /// agent does over its connection limits.
    DropsUnread,
    /// Acks pings but drops `status` requests without a reply.
    AliveStatusFails,
}

/// Minimal Unix-socket responder modelling the agent.
pub struct FakeAgent {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    /// Accepted connections whose request contained the `"shutdown"` op.
    shutdown_requests: Arc<AtomicUsize>,
}

/// What the fake reports in its `status` reply.
struct Reported {
    version: String,
    protocol_version: u8,
}

impl FakeAgent {
    /// Spawns a responder at `socket_path`. Alive: acks pings, stalling the first
    /// connection by `first_delay`. Not alive: wedged, holding connections open
    /// without replying.
    pub fn spawn(socket_path: &Path, alive: bool, first_delay: Duration) -> Self {
        let behaviour = if alive {
            Behaviour::Alive
        } else {
            Behaviour::Wedged
        };
        Self::spawn_with(socket_path, behaviour, first_delay)
    }

    /// Live responder that stops listening after a shutdown request.
    pub fn spawn_exiting_on_shutdown(socket_path: &Path, first_delay: Duration) -> Self {
        Self::spawn_with(socket_path, Behaviour::AliveUntilShutdown, first_delay)
    }

    pub fn spawn_with(socket_path: &Path, behaviour: Behaviour, first_delay: Duration) -> Self {
        Self::spawn_inner(
            socket_path,
            behaviour,
            first_delay,
            Reported {
                version: gpy_agent::VERSION.to_owned(),
                protocol_version: gpy_agent::ipc::protocol::PROTOCOL_VERSION,
            },
            || {},
        )
    }

    /// A responder of `behaviour` reporting `version` and `protocol_version` in
    /// its status reply.
    pub fn spawn_reporting(
        socket_path: &Path,
        behaviour: Behaviour,
        version: &str,
        protocol_version: u8,
    ) -> Self {
        Self::spawn_inner(
            socket_path,
            behaviour,
            Duration::ZERO,
            Reported {
                version: version.to_owned(),
                protocol_version,
            },
            || {},
        )
    }

    /// An old-version agent that runs `before_shutdown_ack` before it acks a
    /// shutdown request and stops listening.
    pub fn spawn_old_exiting_on_shutdown_after(
        socket_path: &Path,
        before_shutdown_ack: impl FnOnce() + Send + 'static,
    ) -> Self {
        Self::spawn_inner(
            socket_path,
            Behaviour::AliveUntilShutdown,
            Duration::ZERO,
            Reported {
                version: "0.0.0-old".to_owned(),
                protocol_version: gpy_agent::ipc::protocol::PROTOCOL_VERSION,
            },
            before_shutdown_ack,
        )
    }

    fn spawn_inner(
        socket_path: &Path,
        behaviour: Behaviour,
        first_delay: Duration,
        reported: Reported,
        before_shutdown_ack: impl FnOnce() + Send + 'static,
    ) -> Self {
        let listener = std::os::unix::net::UnixListener::bind(socket_path)
            .expect("fake agent should bind test socket");
        listener
            .set_nonblocking(true)
            .expect("fake agent listener should go non-blocking");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let shutdown_requests = Arc::new(AtomicUsize::new(0));
        let shutdown_requests_clone = Arc::clone(&shutdown_requests);

        let handle = std::thread::spawn(move || {
            let mut conn_index = 0_usize;
            let mut shutdown_publication = Some(before_shutdown_ack);
            // Wedged connections stay open until the fake stops.
            let mut held: Vec<UnixStream> = Vec::new();
            while !stop_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        if behaviour == Behaviour::DropsUnread {
                            drop(stream);
                            continue;
                        }
                        stream.set_nonblocking(false).ok();
                        stream.set_read_timeout(Some(REQUEST_READ_TIMEOUT)).ok();
                        let mut buf = [0_u8; 64];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let request = buf
                            .get(..n)
                            .map(|body| String::from_utf8_lossy(body).into_owned())
                            .unwrap_or_default();
                        let is_shutdown = request.contains("shutdown");
                        if is_shutdown {
                            shutdown_requests_clone.fetch_add(1, Ordering::Relaxed);
                        }
                        if is_shutdown && behaviour == Behaviour::AliveUntilShutdown {
                            if let Some(publish) = shutdown_publication.take() {
                                publish();
                            }
                            let _ = stream.write_all(ACK_WIRE_FORM);
                            break;
                        }
                        if behaviour == Behaviour::Wedged {
                            held.push(stream);
                        } else {
                            let delay = if conn_index == 0 {
                                first_delay
                            } else {
                                Duration::ZERO
                            };
                            // Reply delay is the behavior under test.
                            std::thread::sleep(delay);
                            if request.contains("\"status\"") {
                                if behaviour != Behaviour::AliveStatusFails {
                                    let _ = stream.write_all(status_reply(&reported).as_bytes());
                                }
                            } else {
                                let _ = stream.write_all(ACK_WIRE_FORM);
                            }
                        }
                        conn_index = conn_index.saturating_add(1);
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // Idle accept loop on a non-blocking listener, not a wait for state.
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
            drop(held);
        });

        Self {
            stop,
            handle: Some(handle),
            shutdown_requests,
        }
    }

    /// How many accepted connections sent a `"shutdown"` op.
    pub fn shutdown_requests(&self) -> usize {
        self.shutdown_requests.load(Ordering::Relaxed)
    }
}

/// The wire form of an `AgentStatus` reply.
fn status_reply(reported: &Reported) -> String {
    format!(
        "{{\"AgentStatus\":{{\"version\":\"{}\",\"protocol_version\":{},\"watched_repos\":0,\"registered_clients\":0,\"cache_entries\":0}}}}\n",
        reported.version, reported.protocol_version
    )
}

impl Drop for FakeAgent {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

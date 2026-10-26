//! `check_and_cleanup_socket` ping-retry: one missed ping must not evict a live
//! agent; a dead socket must still be cleaned up. Integration binary because
//! redirecting the runtime dir via env is `unsafe` under edition 2024.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

#[path = "common/env_guard.rs"]
mod env_guard;

use env_guard::EnvGuard;
use gpy_agent::VERSION;
use gpy_agent::agent::lifecycle::{check_and_cleanup_socket, get_version_file_path};
use gpy_agent::paths::get_socket_path;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// The daemon's exact ping ack (`ipc::ACK_WIRE_JSON`).
const ACK_WIRE_FORM: &[u8] = b"{\"status\":\"ok\"}\n";

#[test]
fn fixture_ack_matches_the_daemon_wire_form() {
    assert_eq!(
        ACK_WIRE_FORM,
        format!("{}\n", gpy_agent::ipc::ACK_WIRE_JSON).as_bytes()
    );
}

// Serializes env-mutating tests so the redirected runtime dir cannot race.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Redirects the runtime dir to an isolated temp dir until the guard drops.
fn redirect_runtime_dir(dir: &Path) -> EnvGuard {
    let mut env = EnvGuard::cleared(&["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "HOME"]);
    // XDG_RUNTIME_DIR wins in paths::runtime_dir(), isolating the version file.
    env.set("XDG_RUNTIME_DIR", dir);
    // Resolving paths no longer creates it; a started agent would have.
    std::fs::create_dir_all(dir.join("gpy")).expect("create redirected runtime dir");
    env
}

/// How a fake agent behaves on each accepted connection.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Behaviour {
    /// Answers every request with the wire ack.
    Alive,
    /// Accepts and closes without answering anything.
    Wedged,
    /// Answers until a shutdown request, then stops listening.
    AliveUntilShutdown,
}

/// Minimal Unix-socket responder modelling the agent.
struct Responder {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    /// Accepted connections whose request contained the `"shutdown"` op.
    shutdown_requests: Arc<AtomicUsize>,
}

impl Responder {
    /// Spawns a responder at `socket_path`. Alive: acks pings, stalling the first
    /// connection by `first_delay`. Dead: accepts and closes without replying.
    fn spawn(socket_path: &Path, alive: bool, first_delay: Duration) -> Self {
        let behaviour = if alive {
            Behaviour::Alive
        } else {
            Behaviour::Wedged
        };
        Self::spawn_with(socket_path, behaviour, first_delay)
    }

    /// Live responder that stops listening after a shutdown request.
    fn spawn_exiting_on_shutdown(socket_path: &Path, first_delay: Duration) -> Self {
        Self::spawn_with(socket_path, Behaviour::AliveUntilShutdown, first_delay)
    }

    fn spawn_with(socket_path: &Path, behaviour: Behaviour, first_delay: Duration) -> Self {
        Self::spawn_with_before_shutdown_ack(socket_path, behaviour, first_delay, || {})
    }

    /// Runs a publication step before acknowledging shutdown.
    fn spawn_with_before_shutdown_ack(
        socket_path: &Path,
        behaviour: Behaviour,
        first_delay: Duration,
        before_shutdown_ack: impl FnOnce() + Send + 'static,
    ) -> Self {
        let alive = behaviour != Behaviour::Wedged;
        let exit_on_shutdown = behaviour == Behaviour::AliveUntilShutdown;
        let listener = std::os::unix::net::UnixListener::bind(socket_path)
            .expect("responder should bind test socket");
        listener
            .set_nonblocking(true)
            .expect("responder listener should go non-blocking");
        let stop = Arc::new(AtomicBool::new(false));
        let stop_clone = Arc::clone(&stop);
        let shutdown_requests = Arc::new(AtomicUsize::new(0));
        let shutdown_requests_clone = Arc::clone(&shutdown_requests);

        let handle = std::thread::spawn(move || {
            let mut conn_index = 0_usize;
            let mut shutdown_publication = Some(before_shutdown_ack);
            while !stop_clone.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut stream, _)) => {
                        stream.set_nonblocking(false).ok();
                        let mut buf = [0_u8; 64];
                        let n = stream.read(&mut buf).unwrap_or(0);
                        let is_shutdown = buf
                            .get(..n)
                            .is_some_and(|body| String::from_utf8_lossy(body).contains("shutdown"));
                        if is_shutdown {
                            shutdown_requests_clone.fetch_add(1, Ordering::Relaxed);
                        }
                        if is_shutdown && exit_on_shutdown {
                            if let Some(publish) = shutdown_publication.take() {
                                publish();
                            }
                            let _ = stream.write_all(ACK_WIRE_FORM);
                            break;
                        }
                        if alive {
                            let delay = if conn_index == 0 {
                                first_delay
                            } else {
                                Duration::ZERO
                            };
                            // Reply delay is the behavior under test.
                            std::thread::sleep(delay);
                            let _ = stream.write_all(ACK_WIRE_FORM);
                        }
                        // When !alive the stream drops here without a response.
                        conn_index = conn_index.saturating_add(1);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                        // Idle accept loop on a non-blocking listener, not a wait for state.
                        std::thread::sleep(Duration::from_millis(10));
                    }
                    Err(_) => break,
                }
            }
        });

        Self {
            stop,
            handle: Some(handle),
            shutdown_requests,
        }
    }

    /// How many accepted connections sent a `"shutdown"` op.
    fn shutdown_requests(&self) -> usize {
        self.shutdown_requests.load(Ordering::Relaxed)
    }
}

impl Drop for Responder {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}

// A slow-but-alive agent that misses one ping must not be declared stale.
// NOT be declared stale (its live socket must not be unlinked).
#[test]
fn slow_but_alive_agent_is_not_declared_stale() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket_path = tmp.path().join("agent.sock");
    // 650ms exceeds the 500ms ping read timeout, so only the retry succeeds.
    // the retry (after backoff) then succeeds once the responder has freed the
    let _responder = Responder::spawn(&socket_path, true, Duration::from_millis(650));

    let started = Instant::now();
    let is_running = check_and_cleanup_socket(&socket_path);
    let elapsed = started.elapsed();

    assert!(
        is_running,
        "a live agent that misses one ping must be treated as running, not stale"
    );
    assert!(
        socket_path.exists(),
        "the live agent's socket must not be unlinked"
    );
    // Proves the retry path ran.
    assert!(
        elapsed >= Duration::from_millis(500),
        "expected at least one missed ping before success, took {elapsed:?}"
    );
}

// Accepts every connection but never replies: stale, via the AcceptedNoReply
// eviction path rather than immediate cleanup.
// stale and must be reported as not running.
#[test]
fn unresponsive_listener_is_cleaned_up() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket_path = tmp.path().join("agent.sock");
    let _responder = Responder::spawn(&socket_path, false, Duration::ZERO);

    let is_running = check_and_cleanup_socket(&socket_path);

    assert!(
        !is_running,
        "a socket that never answers pings must be reported as not running"
    );
}

// A wedged-but-listening agent gets one eviction attempt (shutdown, bounded
// wait, unlink), not the hands-off `AgentLiveness::Indeterminate` verdict.
#[test]
fn wedged_agent_that_never_replies_is_evicted() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    // `send_shutdown_command()` targets `get_socket_path()`, so the fixture must
    // listen there to be reachable.
    let socket_path = get_socket_path().expect("compute redirected socket path");
    let responder = Responder::spawn(&socket_path, false, Duration::ZERO);

    let is_running = check_and_cleanup_socket(&socket_path);

    assert!(
        !is_running,
        "a wedged agent that never replies must be reported as not running once evicted"
    );
    assert!(
        responder.shutdown_requests() > 0,
        "check_and_cleanup_socket must attempt send_shutdown_command against a wedged \
         agent before evicting it, not go straight to unlinking its socket the way a \
         genuinely dead (connection-refused) agent does"
    );
    assert!(
        !socket_path.exists(),
        "the wedged agent's socket must be unlinked once eviction's bounded wait elapses"
    );
}

// A leftover socket file with nothing listening must be removed.
#[test]
fn dead_socket_file_is_removed() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket_path = tmp.path().join("agent.sock");
    std::fs::write(&socket_path, b"stale").unwrap();

    let is_running = check_and_cleanup_socket(&socket_path);

    assert!(
        !is_running,
        "a dead socket file must be reported not running"
    );
    assert!(!socket_path.exists(), "a dead socket file must be removed");
}

// `gpy-agent start` must evict a running agent with a different recorded
// version and leave a matching one alone; installers rely on this.
#[test]
fn version_mismatched_agent_is_evicted() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    // Eviction targets `get_socket_path()`, so the fixture must listen there.
    let socket_path = get_socket_path().expect("compute redirected socket path");
    let version_path = get_version_file_path();
    std::fs::write(&version_path, "0.0.0-old\n").expect("write the old agent's version");
    // Acks pings and goes away once told to shut down.
    let responder = Responder::spawn_exiting_on_shutdown(&socket_path, Duration::ZERO);

    let is_running = check_and_cleanup_socket(&socket_path);

    assert!(
        !is_running,
        "a responding agent of another version must be reported as not running so the new binary starts"
    );
    assert_eq!(
        responder.shutdown_requests(),
        1,
        "the old agent must be asked to shut down exactly once"
    );
    assert!(
        !socket_path.exists(),
        "the evicted agent's socket must be unlinked"
    );
    assert!(
        !version_path.exists(),
        "the evicted agent's version file must be removed"
    );
}

// A replacement that binds during eviction must be recognised, not unlinked.
// recognised as the replacement, never read as the old agent still alive, and
// never unlinked.
#[test]
fn eviction_does_not_unlink_a_replacement_agents_socket() {
    use std::os::unix::fs::MetadataExt;

    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket_path = get_socket_path().expect("compute redirected socket path");
    let version_path = get_version_file_path();
    std::fs::write(&version_path, "0.0.0-old\n").expect("write the old agent's version");
    // Bind the replacement early but publish via rename at shutdown, so there is
    // no gap eviction could read as "no agent".
    let replacement_path = socket_path.with_file_name("next.sock");
    let _replacement = Responder::spawn(&replacement_path, true, Duration::ZERO);
    let (published_tx, published_rx) = std::sync::mpsc::channel();
    let old = Responder::spawn_with_before_shutdown_ack(
        &socket_path,
        Behaviour::AliveUntilShutdown,
        Duration::ZERO,
        {
            let public_path = socket_path.clone();
            move || {
                // Eviction cannot recheck the path until the shutdown ack.
                std::fs::rename(&replacement_path, &public_path)
                    .expect("atomically publish the replacement socket");
                let meta = std::fs::metadata(&public_path).expect("stat published replacement");
                published_tx
                    .send((meta.ino(), meta.ctime(), meta.ctime_nsec()))
                    .expect("record replacement publication before shutdown ack");
            }
        },
    );
    let old_ino = std::fs::metadata(&socket_path)
        .expect("stat old agent socket before eviction")
        .ino();

    let is_running = check_and_cleanup_socket(&socket_path);
    let (ino, ctime, ctime_nsec) = published_rx
        .try_recv()
        .expect("replacement fixture must publish before eviction returns");
    assert_eq!(
        old.shutdown_requests(),
        1,
        "the old agent must receive exactly one shutdown before publication"
    );
    assert_ne!(
        ino, old_ino,
        "the fixture must publish a different socket before the eviction verdict"
    );

    assert!(
        is_running,
        "a responsive replacement must be reported as running so no second agent is forked"
    );
    let meta = std::fs::metadata(&socket_path).expect("replacement socket must still exist");
    assert_eq!(
        (meta.ino(), meta.ctime(), meta.ctime_nsec()),
        (ino, ctime, ctime_nsec),
        "the path must still hold the replacement's socket"
    );
    assert!(
        std::os::unix::net::UnixStream::connect(&socket_path).is_ok(),
        "the replacement must still accept connections"
    );
}

#[test]
fn version_matched_agent_is_left_running() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket_path = get_socket_path().expect("compute redirected socket path");
    let version_path = get_version_file_path();
    std::fs::write(&version_path, format!("{VERSION}\n")).expect("write the matching version");
    let responder = Responder::spawn(&socket_path, true, Duration::ZERO);

    let is_running = check_and_cleanup_socket(&socket_path);

    assert!(
        is_running,
        "a responding agent of the same version must be treated as already running"
    );
    assert_eq!(
        responder.shutdown_requests(),
        0,
        "a matching agent must not be asked to shut down"
    );
    assert!(socket_path.exists(), "the live agent's socket must stay");
    assert!(
        version_path.exists(),
        "the live agent's version file must stay"
    );
}

/// An older binary must not evict a newer agent, or installs would flip-flop.
#[test]
fn newer_running_agent_is_left_running() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket_path = get_socket_path().expect("compute redirected socket path");
    let version_path = get_version_file_path();
    std::fs::write(&version_path, "999.0.0\n").expect("write the newer agent's version");
    let responder = Responder::spawn(&socket_path, true, Duration::ZERO);

    let is_running = check_and_cleanup_socket(&socket_path);

    assert!(
        is_running,
        "a responding agent newer than this binary must be treated as already running"
    );
    assert_eq!(
        responder.shutdown_requests(),
        0,
        "a newer agent must not be asked to shut down"
    );
    assert!(socket_path.exists(), "the newer agent's socket must stay");
    assert!(
        version_path.exists(),
        "the newer agent's version file must stay"
    );
}

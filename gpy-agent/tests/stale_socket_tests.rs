// Integration tests for `check_and_cleanup_socket` ping-retry behaviour (#317).
//
// A single missed 500ms ping must not cause a live agent's socket to be
// unlinked and a duplicate agent forked (acceptance criterion 1), while a
// genuinely dead socket must still be cleaned up (no regression).
//
// These live in an integration binary (not lib unit tests) because they
// redirect the runtime dir via `std::env::set_var`, which is `unsafe` under
// edition 2024 and disallowed inside the `unsafe_code`-denying lib crate.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]
#![allow(clippy::missing_panics_doc)]

use gpy_agent::VERSION;
use gpy_agent::agent::lifecycle::{
    check_and_cleanup_socket, get_socket_path, get_version_file_path,
};
use std::ffi::OsString;
use std::io::{Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// Exactly what the daemon puts on the wire when it acknowledges a ping.
///
/// `handle_ping` answers `Response::Ack`, which the connection renders
/// through the JSON formatter as `{"status":"ok"}` (`ipc::ACK_WIRE_JSON`).
///
/// This fixture used to reply `{"op":"pong"}`, a shape the daemon has never
/// sent; it passed only because the ping's acknowledgement check was a
/// `contains("pong")` substring match on the raw bytes. Since #573 the check
/// decodes the reply as a `Response`, so the fixture has to send the real
/// thing -- otherwise a "live agent" here would model something no live agent
/// does. (#573's own fixture, serde's `"Ack"`, was that mistake.)
const ACK_WIRE_FORM: &[u8] = b"{\"status\":\"ok\"}\n";

#[test]
fn fixture_ack_matches_the_daemon_wire_form() {
    assert_eq!(
        ACK_WIRE_FORM,
        format!("{}\n", gpy_agent::ipc::ACK_WIRE_JSON).as_bytes()
    );
}

// Serializes tests that mutate process-global environment variables so the
// redirected runtime dir (holding the agent.version file) cannot race.
static ENV_LOCK: Mutex<()> = Mutex::new(());

/// Redirects the runtime dir (and thus the agent.version path) to an isolated
/// temp dir for the duration of a test, restoring prior env on drop.
struct EnvGuard {
    saved: Vec<(&'static str, Option<OsString>)>,
}

impl EnvGuard {
    fn redirect_runtime_dir(dir: &Path) -> Self {
        let keys = ["XDG_RUNTIME_DIR", "XDG_CACHE_HOME", "HOME"];
        let saved = keys
            .iter()
            .map(|&k| (k, std::env::var_os(k)))
            .collect::<Vec<_>>();
        for &k in &keys {
            unsafe { std::env::remove_var(k) };
        }
        // XDG_RUNTIME_DIR wins in get_runtime_dir(), isolating the version file.
        unsafe { std::env::set_var("XDG_RUNTIME_DIR", dir) };
        Self { saved }
    }
}

impl Drop for EnvGuard {
    fn drop(&mut self) {
        for (k, v) in &self.saved {
            match v {
                Some(val) => unsafe { std::env::set_var(k, val) },
                None => unsafe { std::env::remove_var(k) },
            }
        }
    }
}

/// A minimal Unix-socket responder used to model the agent for ping tests.
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

struct Responder {
    stop: Arc<AtomicBool>,
    handle: Option<std::thread::JoinHandle<()>>,
    /// Count of accepted connections whose request body contained the
    /// `"shutdown"` op, so a test can prove `send_shutdown_command()` was
    /// actually attempted against this fixture (#548), not just inferred from
    /// timing.
    shutdown_requests: Arc<AtomicUsize>,
}

impl Responder {
    /// Spawn a responder bound to `socket_path`.
    ///
    /// When `alive` is true it answers pings with a real `ACK_WIRE_FORM`, but
    /// stalls the
    /// FIRST connection by `first_delay` (modelling a momentarily busy but live
    /// agent that misses one ping). When `alive` is false it accepts and
    /// immediately closes each connection without ever replying -- to a ping
    /// OR to a shutdown request -- modelling a wedged-but-listening agent
    /// (#548): the connection is genuinely accepted, so it is strong evidence
    /// of life, just never answered.
    fn spawn(socket_path: &Path, alive: bool, first_delay: Duration) -> Self {
        let behaviour = if alive {
            Behaviour::Alive
        } else {
            Behaviour::Wedged
        };
        Self::spawn_with(socket_path, behaviour, first_delay)
    }

    /// A live responder that, like a real agent, stops listening once it has
    /// been asked to shut down (the socket file is left for the caller to
    /// unlink, exactly as a real agent's is).
    fn spawn_exiting_on_shutdown(socket_path: &Path, first_delay: Duration) -> Self {
        Self::spawn_with(socket_path, Behaviour::AliveUntilShutdown, first_delay)
    }

    fn spawn_with(socket_path: &Path, behaviour: Behaviour, first_delay: Duration) -> Self {
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
                            let _ = stream.write_all(ACK_WIRE_FORM);
                            break;
                        }
                        if alive {
                            let delay = if conn_index == 0 {
                                first_delay
                            } else {
                                Duration::ZERO
                            };
                            std::thread::sleep(delay);
                            let _ = stream.write_all(ACK_WIRE_FORM);
                        }
                        // When !alive the stream drops here, closing the
                        // connection without a valid response.
                        conn_index = conn_index.saturating_add(1);
                    }
                    Err(ref e) if e.kind() == std::io::ErrorKind::WouldBlock => {
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

// Acceptance criterion 1: a slow-but-alive agent that misses a single ping must
// NOT be declared stale (its live socket must not be unlinked).
#[test]
fn slow_but_alive_agent_is_not_declared_stale() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = EnvGuard::redirect_runtime_dir(tmp.path());

    let socket_path = tmp.path().join("agent.sock");
    // 650ms > one ping's 500ms read timeout, so the first ping attempt misses;
    // the retry (after backoff) then succeeds once the responder has freed the
    // first connection.
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
    // Sanity: proves the retry path was exercised (first attempt missed).
    assert!(
        elapsed >= Duration::from_millis(500),
        "expected at least one missed ping before success, took {elapsed:?}"
    );
}

// Regression guard: a socket held by something that never answers our protocol
// is still genuinely stale and must be reported as not running.
//
// Under #548 this specific fixture (accepts every connection, replies to
// nothing) is exactly the "wedged-but-listening" shape: the connection really
// is accepted, so it now routes through the AcceptedNoReply eviction path
// (send_shutdown_command -> bounded wait_for_agent_shutdown -> unlink) rather
// than the immediate stale-socket cleanup a plain connect-refused would get.
// The end result -- not running, socket gone -- is unchanged; only the route
// (and, since it now waits out SHUTDOWN_WAIT_MAX, the wall time) is different.
// `wedged_agent_that_never_replies_is_evicted` below asserts the new route
// directly.
#[test]
fn unresponsive_listener_is_cleaned_up() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = EnvGuard::redirect_runtime_dir(tmp.path());

    let socket_path = tmp.path().join("agent.sock");
    let _responder = Responder::spawn(&socket_path, false, Duration::ZERO);

    let is_running = check_and_cleanup_socket(&socket_path);

    assert!(
        !is_running,
        "a socket that never answers pings must be reported as not running"
    );
}

// #548: a wedged-but-listening agent -- one that accepts every connection but
// never replies to anything, including the shutdown request itself -- must
// not be silently left alone the way `AgentLiveness::Indeterminate` is (that
// verdict is reserved for rounds this process could not even complete). It
// gets one real eviction attempt: send_shutdown_command(), then a bounded
// wait_for_agent_shutdown(), then unlink -- the same defined path the
// version-mismatch branch already uses for a live-but-incompatible agent,
// reused here rather than inventing a second one.
#[test]
fn wedged_agent_that_never_replies_is_evicted() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = EnvGuard::redirect_runtime_dir(tmp.path());

    // `send_shutdown_command()` (invoked inside the eviction path this test
    // exercises) resolves its own target via `get_socket_path()` rather than
    // taking a parameter -- in real usage that always matches the socket path
    // this test passes to `check_and_cleanup_socket` below, since callers
    // always derive both from the same `get_socket_path()`. Use the real
    // computed path here too so the fixture is actually reachable by it,
    // instead of an arbitrary filename the shutdown command would never find.
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

// Regression guard: a leftover socket file with nothing listening is stale and
// must be removed.
#[test]
fn dead_socket_file_is_removed() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = EnvGuard::redirect_runtime_dir(tmp.path());

    let socket_path = tmp.path().join("agent.sock");
    std::fs::write(&socket_path, b"stale").unwrap();

    let is_running = check_and_cleanup_socket(&socket_path);

    assert!(
        !is_running,
        "a dead socket file must be reported not running"
    );
    assert!(!socket_path.exists(), "a dead socket file must be removed");
}

// Upgrade contract (#649): `gpy-agent start` on a new binary must evict a
// running agent whose recorded version differs, and must leave a matching
// one alone. This is the path both installers rely on when they re-run
// `start` after replacing the binary (#307); until now only the wedged and
// dead-socket branches of `check_and_cleanup_socket` had tests.
#[test]
fn version_mismatched_agent_is_evicted() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = EnvGuard::redirect_runtime_dir(tmp.path());

    // The eviction sends its shutdown to `get_socket_path()`, so the fixture
    // must listen there (see wedged_agent_that_never_replies_is_evicted).
    let socket_path = get_socket_path().expect("compute redirected socket path");
    let version_path = get_version_file_path().expect("compute redirected version path");
    std::fs::write(&version_path, "0.0.0-old\n").expect("write the old agent's version");
    // Answers pings like a live agent and, like a real one, goes away once
    // it has been told to shut down.
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

#[test]
fn version_matched_agent_is_left_running() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = EnvGuard::redirect_runtime_dir(tmp.path());

    let socket_path = get_socket_path().expect("compute redirected socket path");
    let version_path = get_version_file_path().expect("compute redirected version path");
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

/// #780: an older binary must not evict a newer running agent. Two installs
/// resolving different binaries would otherwise flip the daemon back and
/// forth on every `start`.
#[test]
fn newer_running_agent_is_left_running() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = EnvGuard::redirect_runtime_dir(tmp.path());

    let socket_path = get_socket_path().expect("compute redirected socket path");
    let version_path = get_version_file_path().expect("compute redirected version path");
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

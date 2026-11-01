//! `inspect_socket` ping-retry: one missed ping must not evict a live agent; a
//! dead socket must still be cleaned up; a busy agent is left alone, a wedged
//! one is evicted. Integration binary because redirecting the runtime dir via
//! env is `unsafe` under edition 2024.
#![cfg(unix)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(clippy::panic)]

mod common;

use common::env_guard::EnvGuard;
use common::fake_agent::{ACK_WIRE_FORM, Behaviour, FakeAgent};
use gpy_agent::agent::lifecycle::{
    SocketCheck, StartOutcome, StatusProbe, inspect_socket, probe_status,
};
use gpy_agent::ipc::protocol::PROTOCOL_VERSION;
use gpy_agent::paths::SocketPath;
use std::path::Path;
use std::sync::Mutex;
use std::time::{Duration, Instant};

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
    // XDG_RUNTIME_DIR wins in paths::runtime_dir(), isolating the socket.
    env.set("XDG_RUNTIME_DIR", dir);
    // Resolving paths no longer creates it; a started agent would have.
    std::fs::create_dir_all(dir.join("gpy")).expect("create redirected runtime dir");
    env
}

/// `inspect_socket` with notices discarded.
fn inspect(socket: &SocketPath) -> SocketCheck {
    inspect_socket(socket, &mut |_| {})
}

/// The socket that `redirect_runtime_dir` points the default path at.
fn redirected_socket() -> SocketPath {
    SocketPath::resolve(None).expect("compute redirected socket path")
}

// A slow-but-alive agent that misses one ping must not be declared stale.
#[test]
fn slow_but_alive_agent_is_not_declared_stale() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket_path = tmp.path().join("agent.sock");
    // 650ms exceeds the 500ms ping read timeout, so only the retry succeeds.
    let _agent = FakeAgent::spawn(&socket_path, true, Duration::from_millis(650));

    let started = Instant::now();
    let socket = SocketPath::resolve(Some(&socket_path)).expect("resolve test socket");
    let check = inspect(&socket);
    let elapsed = started.elapsed();

    assert_eq!(
        check,
        SocketCheck::Done(StartOutcome::AlreadyRunning),
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

// Accepts every connection and holds it without replying: a read timeout, so
// the agent is wedged and evicted rather than cleaned up immediately.
#[test]
fn unresponsive_listener_is_cleaned_up() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket_path = tmp.path().join("agent.sock");
    let _agent = FakeAgent::spawn(&socket_path, false, Duration::ZERO);

    let socket = SocketPath::resolve(Some(&socket_path)).expect("resolve test socket");
    let check = inspect(&socket);

    assert_eq!(
        check,
        SocketCheck::Proceed { previous: None },
        "a socket that never answers pings must be reported as not running"
    );
}

// A wedged-but-listening agent gets one eviction attempt (shutdown, bounded
// wait, unlink), not the hands-off indeterminate verdict.
#[test]
fn wedged_agent_that_never_replies_is_evicted() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    // `send_shutdown_command` targets `SocketPath::resolve(None)`, so the fixture
    // must listen there to be reachable.
    let socket = redirected_socket();
    let socket_path = socket.path().to_path_buf();
    let agent = FakeAgent::spawn(&socket_path, false, Duration::ZERO);

    let check = inspect(&socket);

    assert_eq!(
        check,
        SocketCheck::Proceed { previous: None },
        "a wedged agent that never replies must be reported as not running once evicted"
    );
    assert!(
        agent.shutdown_requests() > 0,
        "inspect_socket must attempt send_shutdown_command against a wedged \
         agent before evicting it, not go straight to unlinking its socket the way a \
         genuinely dead (connection-refused) agent does"
    );
    assert!(
        !socket_path.exists(),
        "the wedged agent's socket must be unlinked once eviction's bounded wait elapses"
    );
}

// An agent shedding load closes connections without reading them. That is a
// live agent: no shutdown, no unlink, no second daemon.
#[test]
fn busy_agent_that_drops_connections_is_left_running() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket = redirected_socket();
    let socket_path = socket.path().to_path_buf();
    let agent = FakeAgent::spawn_with(&socket_path, Behaviour::DropsUnread, Duration::ZERO);

    let check = inspect(&socket);

    assert_eq!(check, SocketCheck::Done(StartOutcome::AlreadyRunning));
    assert_eq!(
        agent.shutdown_requests(),
        0,
        "a busy agent must not be asked to shut down"
    );
    assert!(socket_path.exists(), "the busy agent's socket must stay");
}

#[test]
fn status_probe_reports_a_dropping_agent_as_busy() {
    let tmp = tempfile::tempdir().unwrap();
    let socket_path = tmp.path().join("agent.sock");
    let _agent = FakeAgent::spawn_with(&socket_path, Behaviour::DropsUnread, Duration::ZERO);

    let probe = probe_status(&socket_path);

    assert!(matches!(probe, StatusProbe::Busy), "got {probe:?}");
}

#[test]
fn status_probe_reports_a_refused_socket_as_not_listening() {
    let tmp = tempfile::tempdir().unwrap();
    let socket_path = tmp.path().join("agent.sock");
    drop(std::os::unix::net::UnixListener::bind(&socket_path).unwrap());
    assert!(
        socket_path.exists(),
        "premise: a socket file is left behind"
    );

    let probe = probe_status(&socket_path);

    assert!(
        matches!(probe, StatusProbe::NotListening(_)),
        "got {probe:?}"
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

    let socket = SocketPath::resolve(Some(&socket_path)).expect("resolve test socket");
    let check = inspect(&socket);

    assert_eq!(
        check,
        SocketCheck::Proceed { previous: None },
        "a dead socket file must be reported not running"
    );
    assert!(!socket_path.exists(), "a dead socket file must be removed");
}

// `gpy-agent start` must evict a running agent that reports an older version
// and leave a matching one alone; installers rely on this.
#[test]
fn version_mismatched_agent_is_evicted() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    // Eviction targets `SocketPath::resolve(None)`, so the fixture must listen there.
    let socket = redirected_socket();
    let socket_path = socket.path().to_path_buf();
    // Acks pings, reports an old version and goes away once told to shut down.
    let agent = FakeAgent::spawn_reporting(
        &socket_path,
        Behaviour::AliveUntilShutdown,
        "0.0.0-old",
        PROTOCOL_VERSION,
    );

    let check = inspect(&socket);

    assert_eq!(
        check,
        SocketCheck::Proceed {
            previous: Some("0.0.0-old".to_owned())
        },
        "a responding agent of an older version must be reported as not running so the new binary starts"
    );
    assert_eq!(
        agent.shutdown_requests(),
        1,
        "the old agent must be asked to shut down exactly once"
    );
    assert!(
        !socket_path.exists(),
        "the evicted agent's socket must be unlinked"
    );
}

// A replacement that binds during eviction must be recognised, not unlinked.
#[test]
fn eviction_does_not_unlink_a_replacement_agents_socket() {
    use std::os::unix::fs::MetadataExt;

    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket = redirected_socket();
    let socket_path = socket.path().to_path_buf();
    // Bind the replacement early but publish via rename at shutdown, so there is
    // no gap eviction could read as "no agent".
    let replacement_path = socket_path.with_file_name("next.sock");
    let _replacement = FakeAgent::spawn(&replacement_path, true, Duration::ZERO);
    let (published_tx, published_rx) = std::sync::mpsc::channel();
    let old = FakeAgent::spawn_old_exiting_on_shutdown_after(&socket_path, {
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
    });
    let old_ino = std::fs::metadata(&socket_path)
        .expect("stat old agent socket before eviction")
        .ino();

    let check = inspect(&socket);
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

    assert_eq!(
        check,
        SocketCheck::Done(StartOutcome::AlreadyRunning),
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

    let socket = redirected_socket();
    let socket_path = socket.path().to_path_buf();
    let agent = FakeAgent::spawn(&socket_path, true, Duration::ZERO);

    let check = inspect(&socket);

    assert_eq!(
        check,
        SocketCheck::Done(StartOutcome::AlreadyRunning),
        "a responding agent of the same version must be treated as already running"
    );
    assert_eq!(
        agent.shutdown_requests(),
        0,
        "a matching agent must not be asked to shut down"
    );
    assert!(socket_path.exists(), "the live agent's socket must stay");
}

/// An older binary must not evict a newer agent, or installs would flip-flop.
#[test]
fn newer_running_agent_is_left_running() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket = redirected_socket();
    let socket_path = socket.path().to_path_buf();
    let agent =
        FakeAgent::spawn_reporting(&socket_path, Behaviour::Alive, "999.0.0", PROTOCOL_VERSION);

    let check = inspect(&socket);

    assert_eq!(
        check,
        SocketCheck::Done(StartOutcome::NewerRunning {
            running: "999.0.0".to_owned()
        }),
        "a responding agent newer than this binary must be left running"
    );
    assert_eq!(
        agent.shutdown_requests(),
        0,
        "a newer agent must not be asked to shut down"
    );
    assert!(socket_path.exists(), "the newer agent's socket must stay");
}

/// A newer agent on another protocol cannot serve this binary's shells, so it
/// is replaced anyway. Two installs a protocol apart replace each other's agent.
#[test]
fn protocol_mismatched_newer_agent_is_evicted() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket = redirected_socket();
    let socket_path = socket.path().to_path_buf();
    let agent = FakeAgent::spawn_reporting(
        &socket_path,
        Behaviour::AliveUntilShutdown,
        "999.0.0",
        PROTOCOL_VERSION.wrapping_add(1),
    );

    let check = inspect(&socket);

    assert_eq!(
        check,
        SocketCheck::Proceed {
            previous: Some("999.0.0".to_owned())
        },
        "an agent on another protocol must be reported as not running"
    );
    assert_eq!(agent.shutdown_requests(), 1);
    assert!(!socket_path.exists(), "the evicted agent's socket must go");
}

/// A responding agent whose status cannot be read is unverifiable: leave it.
#[test]
fn failed_status_query_leaves_a_responding_agent_alone() {
    let _lock = ENV_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    let tmp = tempfile::tempdir().unwrap();
    let _env = redirect_runtime_dir(tmp.path());

    let socket = redirected_socket();
    let socket_path = socket.path().to_path_buf();
    let agent = FakeAgent::spawn_with(&socket_path, Behaviour::AliveStatusFails, Duration::ZERO);

    let check = inspect(&socket);

    assert_eq!(check, SocketCheck::Done(StartOutcome::Unverifiable));
    assert_eq!(agent.shutdown_requests(), 0);
    assert!(socket_path.exists(), "the agent's socket must stay");
}

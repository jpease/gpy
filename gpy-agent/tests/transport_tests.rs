//! Unix domain socket transport tests.
//!
//! Every test points `XDG_RUNTIME_DIR` at its own temp directory so the
//! transport's default socket path never touches a real agent's runtime dir.

#![allow(clippy::expect_used)]
#![allow(clippy::missing_panics_doc)]

#[path = "common/env_guard.rs"]
mod env_guard;

use env_guard::EnvGuard;
use gpy_agent::ipc::transport::Transport;
use serial_test::serial;
use std::path::PathBuf;
use tempfile::TempDir;

/// A temp runtime dir under `/tmp` (short enough for `sun_path`) installed as
/// `XDG_RUNTIME_DIR` until the guard drops.
fn isolated_runtime() -> (TempDir, EnvGuard) {
    let dir = tempfile::Builder::new()
        .prefix("gpy-tt")
        .tempdir_in("/tmp")
        .expect("create runtime dir");
    let env = EnvGuard::set_one("XDG_RUNTIME_DIR", dir.path());
    (dir, env)
}

fn expected_socket(runtime: &TempDir) -> PathBuf {
    runtime.path().join("gpy").join("gpy-test.sock")
}

#[tokio::test]
#[serial]
async fn test_transport_address_uses_runtime_dir() {
    let (runtime, _env) = isolated_runtime();

    assert_eq!(
        PathBuf::from(Transport::address()),
        expected_socket(&runtime)
    );
}

#[tokio::test]
#[serial]
async fn test_new_server_creates_runtime_dir() {
    let (runtime, _env) = isolated_runtime();
    let socket_dir = runtime.path().join("gpy");
    assert!(!socket_dir.exists());

    Transport::new_server().expect("server transport");

    assert!(
        socket_dir.is_dir(),
        "{} should be created",
        socket_dir.display()
    );
}

#[tokio::test]
#[serial]
async fn test_server_bind_accepts_client() {
    let (runtime, _env) = isolated_runtime();

    let mut server = Transport::new_server().expect("server transport");
    server.bind().expect("bind server socket");
    assert!(
        expected_socket(&runtime).exists(),
        "bind should create the socket file"
    );

    let mut client = Transport::new_client().expect("client transport");
    client
        .connect()
        .await
        .expect("client connects to bound server");
    client
        .send(b"test message")
        .await
        .expect("send on connected client");
}

#[tokio::test]
#[serial]
async fn test_bind_replaces_stale_socket_file() {
    let (runtime, _env) = isolated_runtime();

    // A std listener does not unlink its socket on drop, so this leaves a stale
    // socket file exactly where the transport binds.
    let socket = expected_socket(&runtime);
    std::fs::create_dir_all(socket.parent().expect("socket has a parent"))
        .expect("create socket dir");
    drop(std::os::unix::net::UnixListener::bind(&socket).expect("plant stale socket"));
    assert!(socket.exists());

    let mut second = Transport::new_server().expect("server transport");
    second.bind().expect("rebind over a stale socket file");
}

#[tokio::test]
#[serial]
async fn test_client_connect_error() {
    let (_runtime, _env) = isolated_runtime();
    let mut client = Transport::new_client().expect("client transport");

    let err = client
        .connect()
        .await
        .expect_err("connect should fail with no server bound");

    assert!(
        err.to_string().contains("Failed to connect"),
        "unexpected error: {err}"
    );
}

#[tokio::test]
#[serial]
async fn test_unconnected_send_and_receive_fail() {
    let (_runtime, _env) = isolated_runtime();
    let mut server = Transport::new_server().expect("server transport");
    let mut client = Transport::new_client().expect("client transport");

    for err in [
        server.send(b"test").await.expect_err("server send"),
        server.receive().await.expect_err("server receive"),
        client.send(b"test").await.expect_err("client send"),
        client.receive().await.expect_err("client receive"),
    ] {
        assert!(
            err.to_string().contains("No active connection"),
            "unexpected error: {err}"
        );
    }
}

#[tokio::test]
#[serial]
async fn test_transport_mode_validation() {
    let (runtime, _env) = isolated_runtime();
    let mut server = Transport::new_server().expect("server transport");
    let mut client = Transport::new_client().expect("client transport");

    let bind_err = client.bind().expect_err("client must not bind");
    assert!(bind_err.to_string().contains("Cannot bind in client mode"));

    let connect_err = server.connect().await.expect_err("server must not connect");
    assert!(
        connect_err
            .to_string()
            .contains("Cannot connect in server mode")
    );

    assert!(!expected_socket(&runtime).exists());
}

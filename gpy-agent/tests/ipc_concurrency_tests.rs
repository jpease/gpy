//! Concurrency and admission-control tests for the IPC server: cold cache
//! misses run on the blocking pool so warm requests stay responsive, a
//! saturated connection semaphore rejects fast, and slow clients hit the
//! whole-connection deadline.

// The agent's IPC is Unix-socket only; native Windows is CLI-only.
#![cfg(unix)]
#![allow(clippy::panic)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]

mod common;

use common::endpoint::{endpoint_deps_with_config, wait_for_socket};
use common::git::{git, git_init};
use gpy_agent::config::manager::ConfigManager;
use gpy_agent::ipc::server::{EndpointDeps, EndpointHandle};
use gpy_agent::security::GuardSettings;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::{TempDir, tempdir};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Start an in-process IPC server, wait for the socket, and return the join
/// handle plus socket path. The returned `TempDir` guard must stay alive.
async fn start_server(socket_name: &str) -> (tokio::task::JoinHandle<()>, PathBuf, TempDir) {
    let config_manager = Arc::new(ConfigManager::with_defaults().expect("default config"));
    start_server_with(socket_name, config_manager, GuardSettings::default()).await
}

/// [`start_server`] with explicit config, to shrink admission limits/timeouts.
async fn start_server_with(
    socket_name: &str,
    config_manager: Arc<ConfigManager>,
    security_config: GuardSettings,
) -> (tokio::task::JoinHandle<()>, PathBuf, TempDir) {
    let temp_dir = tempdir().expect("temp dir");
    let socket_path = temp_dir.path().join(socket_name);

    let mut server = EndpointHandle::new(EndpointDeps {
        security_config,
        ..endpoint_deps_with_config(socket_path.clone(), config_manager)
    });
    let server_handle = tokio::spawn(async move {
        server.start().await.expect("server start");
    });

    wait_for_socket(&socket_path, Duration::from_secs(5)).await;

    (server_handle, socket_path, temp_dir)
}

/// Create a committed git repository so status requests do real (cold) work.
fn make_repo() -> TempDir {
    let repo = tempdir().expect("repo dir");
    let path = repo.path();
    git_init(path);
    std::fs::write(path.join("README.md"), "# Concurrency Test\n").expect("write file");
    git(path, &["add", "README.md"]);
    git(path, &["commit", "-q", "-m", "init"]);
    repo
}

/// Send a single request line and read the response on a fresh connection.
async fn request(socket_path: &Path, payload: &str) -> String {
    let mut stream = timeout(Duration::from_secs(5), UnixStream::connect(socket_path))
        .await
        .expect("connect")
        .expect("connect");
    stream
        .write_all(format!("{payload}\n").as_bytes())
        .await
        .expect("write");

    let mut buf = vec![0u8; 8192];
    let n = timeout(Duration::from_secs(10), stream.read(&mut buf))
        .await
        .expect("read did not time out")
        .expect("read");
    String::from_utf8_lossy(buf.get(..n).unwrap_or(&buf)).into_owned()
}

fn git_payload(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\\', "\\\\");
    format!(r#"{{"op":"git","cwd":"{escaped}","format":"json"}}"#)
}

fn lang_payload(path: &Path) -> String {
    let escaped = path.to_string_lossy().replace('\\', "\\\\");
    format!(r#"{{"op":"lang","cwd":"{escaped}","format":"json"}}"#)
}

/// Many simultaneous cold misses (git + language) and warm pings all succeed.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_mixed_cold_and_warm_requests_all_succeed() {
    let (server_handle, socket_path, _temp_dir) = start_server("gpy-concurrency-mixed.sock").await;

    // Distinct repos so each git request is a genuine cold cache miss.
    let repos: Vec<TempDir> = (0_usize..12_usize).map(|_| make_repo()).collect();
    let lang_dirs: Vec<TempDir> = (0_usize..6_usize)
        .map(|_| tempdir().expect("lang dir"))
        .collect();

    let mut tasks = Vec::new();

    for repo in &repos {
        let socket = socket_path.clone();
        let payload = git_payload(repo.path());
        tasks.push(tokio::spawn(async move {
            ("git", request(&socket, &payload).await)
        }));
    }
    for dir in &lang_dirs {
        let socket = socket_path.clone();
        let payload = lang_payload(dir.path());
        tasks.push(tokio::spawn(async move {
            ("lang", request(&socket, &payload).await)
        }));
    }
    for _ in 0_usize..16_usize {
        let socket = socket_path.clone();
        tasks.push(tokio::spawn(async move {
            ("ping", request(&socket, r#"{"op":"ping"}"#).await)
        }));
    }

    for task in tasks {
        let (kind, response) = task.await.expect("client task");
        assert!(
            !response.is_empty(),
            "{kind} request returned empty response"
        );
        if kind == "ping" {
            assert!(
                response.contains(r#""status":"ok""#),
                "ping should report ok, got: {response}"
            );
        } else {
            // A cold miss must not fail the connection.
            let parsed: serde_json::Value =
                serde_json::from_str(response.trim()).unwrap_or_else(|e| {
                    panic!("{kind} response should be valid JSON, got: {response}: {e}")
                });
            assert!(parsed.is_object(), "response should be a JSON object");
        }
    }

    server_handle.abort();
}

/// Pings stay responsive while cold git misses run (they block on the blocking pool, not workers).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pings_stay_responsive_under_cold_git_load() {
    let (server_handle, socket_path, _temp_dir) =
        start_server("gpy-concurrency-responsive.sock").await;

    let repos: Vec<TempDir> = (0_usize..16_usize).map(|_| make_repo()).collect();

    let mut git_tasks = Vec::new();
    for repo in &repos {
        let socket = socket_path.clone();
        let payload = git_payload(repo.path());
        git_tasks.push(tokio::spawn(
            async move { request(&socket, &payload).await },
        ));
    }

    let mut ping_tasks = Vec::new();
    for _ in 0_usize..16_usize {
        let socket = socket_path.clone();
        ping_tasks.push(tokio::spawn(async move {
            let started = Instant::now();
            let response = request(&socket, r#"{"op":"ping"}"#).await;
            (started.elapsed(), response)
        }));
    }

    let mut max_ping = Duration::ZERO;
    for task in ping_tasks {
        let (elapsed, response) = task.await.expect("ping task");
        assert!(
            response.contains(r#""status":"ok""#),
            "ping should report ok even under load, got: {response}"
        );
        max_ping = max_ping.max(elapsed);
    }

    // Generous: catches a full runtime stall without flaking on slow CI.
    assert!(
        max_ping < Duration::from_secs(2),
        "pings must stay responsive under cold git load; slowest was {max_ping:?}"
    );

    for task in git_tasks {
        let response = task.await.expect("git task");
        assert!(!response.is_empty(), "git miss returned empty response");
    }

    server_handle.abort();
}

/// A saturated connection semaphore must close new connections fast, with no
/// reply: the request was never read, so any reply could land in a prompt.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_semaphore_saturation_fails_fast_without_reply() {
    let config_manager = Arc::new(ConfigManager::with_defaults().expect("default config"));
    let security_config = GuardSettings {
        max_concurrent_connections: 1,
        ..GuardSettings::default()
    };
    let (server_handle, socket_path, _temp_dir) = start_server_with(
        "gpy-semaphore-saturation.sock",
        config_manager,
        security_config,
    )
    .await;

    // Hold the only permit: a connection keeps it across requests, so once a
    // ping is answered the holder's handler owns the permit and waits for more.
    let mut holder = BufReader::new(
        timeout(Duration::from_secs(5), UnixStream::connect(&socket_path))
            .await
            .expect("connect")
            .expect("connect"),
    );
    holder
        .write_all(b"{\"op\":\"ping\"}\n")
        .await
        .expect("send holder ping");
    let mut reply = String::new();
    timeout(Duration::from_secs(5), holder.read_line(&mut reply))
        .await
        .expect("holder ping must be answered")
        .expect("read holder reply");
    assert!(
        reply.contains(r#""status":"ok""#),
        "holder ping should succeed, got: {reply}"
    );

    // Must fail fast, well under the 5s request timeout. The server closes before
    // reading, so the write is best-effort and EOF or reset are both valid.
    let mut second = timeout(Duration::from_secs(5), UnixStream::connect(&socket_path))
        .await
        .expect("connect")
        .expect("connect");
    let _ = second.write_all(b"{\"op\":\"ping\"}\n").await;

    let mut buf = vec![0_u8; 8192];
    let read = timeout(Duration::from_secs(2), second.read(&mut buf))
        .await
        .expect("saturated semaphore must reject the second connection fast, not hang");
    match read {
        Ok(0) => {}
        Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {}
        Ok(n) => panic!(
            "busy rejection must close without a reply, got: {}",
            String::from_utf8_lossy(buf.get(..n).unwrap_or(&buf))
        ),
        Err(e) => panic!("unexpected read error on busy rejection: {e}"),
    }

    drop(holder);
    server_handle.abort();
}

/// A slow-loris client trickling bytes under the per-read timeout is still
/// closed by the whole-connection deadline (4x per-read timeout).
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slow_loris_connection_hits_whole_connection_deadline() {
    // 2s per-read -> 8s deadline; 2s leaves margin over the 300ms trickle for jitter.
    let config_dir = tempdir().expect("config dir");
    let config_path = config_dir.path().join("config.toml");
    std::fs::write(&config_path, "[agent]\ntimeout_seconds = 2\n").expect("write config");
    let config_manager = Arc::new(ConfigManager::from_path(&config_path).expect("config"));

    let (server_handle, socket_path, _temp_dir) = start_server_with(
        "gpy-slow-loris.sock",
        config_manager,
        GuardSettings::default(),
    )
    .await;

    let mut stream = timeout(Duration::from_secs(5), UnixStream::connect(&socket_path))
        .await
        .expect("connect")
        .expect("connect");

    // 30 * 300ms = 9s of trickling, past the 8s deadline.
    for _ in 0_u32..30_u32 {
        let _ = stream.write_all(b"x").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // The deadline must already have closed the connection.
    let mut buf = [0_u8; 16];
    let read_result = timeout(Duration::from_secs(4), stream.read(&mut buf)).await;
    let outcome = read_result.expect(
        "connection must be closed at the whole-connection deadline, not left open indefinitely",
    );
    match outcome {
        // EOF or reset both prove the connection ended.
        Ok(0) | Err(_) => {}
        Ok(n) => panic!("expected EOF after the whole-connection deadline, got {n} bytes"),
    }

    server_handle.abort();
}

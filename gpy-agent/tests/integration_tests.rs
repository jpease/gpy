//! End-to-end IPC server tests over a real Unix socket: message size limit,
//! connection-semaphore admission, client registration, and prompt-format
//! error replies, plus git-status and language-detection smoke checks.

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(
    unsafe_code,
    reason = "raising RLIMIT_NOFILE for the 250-connection test needs libc::getrlimit/setrlimit"
)]

mod common;

use common::endpoint::endpoint;
#[cfg(unix)]
use common::endpoint::wait_for_socket;
use common::env_guard::EnvGuard;
use gpy_agent::git::status::load_repository_state;
use gpy_agent::language::version::{NodeDetector, detect_language_release, get_version_detectors};
use std::fs;
#[cfg(unix)]
use std::sync::Arc;
#[cfg(unix)]
use std::time::Duration;
use tempfile::TempDir;

#[cfg(unix)]
const SOCKET_TIMEOUT: Duration = Duration::from_secs(2);

#[test]
#[serial_test::serial]
fn test_ipc_server_creation() {
    let _env = set_test_env();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test_ipc_server.sock");

    let _server = endpoint(socket_path);
}

#[test]
#[serial_test::serial]
fn test_git_status_integration() {
    let _env = set_test_env();
    let status = load_repository_state(".", 0, true)
        .expect("Git status loading should succeed for current repository")
        .expect("Should detect git repository in current directory");

    assert!(
        !status.status.branch.is_empty(),
        "Git repository should have a current branch name"
    );
}

#[test]
#[serial_test::serial]
fn test_language_detection_integration() {
    let _env = set_test_env();
    let detectors = get_version_detectors();
    assert_eq!(
        detectors.len(),
        10,
        "Should have all 10 language detectors including Fish"
    ); // Should have all 10 language detectors (including Fish)

    let node_detector = NodeDetector;
    let version_result = detect_language_release(&node_detector);

    if let Some(version) = version_result {
        assert!(
            !version.is_empty(),
            "Detected Node version should not be empty string"
        );
    }
}

#[test]
#[serial_test::serial]
fn test_git_status_with_temp_repo() {
    let _env = set_test_env();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let temp_path = temp_dir.path();

    common::git::git_init(temp_path);
    fs::write(temp_path.join("test.txt"), "test content").expect("write test.txt");

    let status = load_repository_state(temp_path.to_string_lossy().as_ref(), 0, true)
        .expect("Git status should succeed for freshly initialized repository")
        .expect("freshly initialized repository should be detected");
    assert!(
        !status.status.branch.is_empty(),
        "Initialized repository should have default branch name"
    );
    assert!(
        status.status.untracked > 0,
        "Repository with untracked test.txt should report untracked files"
    );
}

#[test]
fn test_git_status_error_handling() {
    let result = load_repository_state("/absolutely/nonexistent/path/that/cannot/exist", 0, true);

    assert!(
        matches!(result, Ok(None) | Err(_)),
        "Should handle nonexistent path gracefully without panicking"
    );
}

/// A request line longer than the 64 `KiB` limit makes the server drop the
/// connection without replying.
#[cfg(unix)] // The agent's IPC is Unix-socket only; native Windows is CLI-only.
#[tokio::test]
async fn test_message_size_limit_enforced() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test_size_limit.sock");

    let mut server = endpoint(socket_path.clone());
    let server_handle = tokio::spawn(async move {
        let _ = server.start().await;
    });
    wait_for_socket(&socket_path, SOCKET_TIMEOUT).await;

    let mut stream = UnixStream::connect(&socket_path)
        .await
        .expect("connect to test server");
    // 70 KB with no newline, so the server's accumulated-line guard trips
    let oversized_data = vec![b'X'; 70_000];
    for chunk in oversized_data.chunks(8192) {
        if stream.write_all(chunk).await.is_err() {
            break;
        }
    }

    let mut response = vec![0u8; 1024];
    match tokio::time::timeout(Duration::from_millis(500), stream.read(&mut response)).await {
        Ok(Ok(0) | Err(_)) => {} // connection closed by the server
        Ok(Ok(n)) => {
            panic!("Server should have dropped connection but sent {n} bytes");
        }
        Err(timeout_err) => {
            panic!("Connection timed out - size limit may not be enforced: {timeout_err}");
        }
    }
    server_handle.abort();
}

/// Outcome of a single connection attempt in
/// `spawn_concurrent_connection_attempts`.
///
/// A saturated semaphore closes without writing, so EOF before any byte means busy.
#[cfg(unix)]
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ConnectionOutcome {
    /// Got a real ping ack; the handler held a semaphore permit.
    Success,
    /// The server closed the connection before writing any byte: the
    /// semaphore was saturated and no permit was ever acquired.
    Busy,
    /// Connect/read failed or timed out outright.
    Failed,
}

/// Spawn `count` tasks that each connect, send a ping, and wait for the
/// server's response before classifying themselves as [`ConnectionOutcome`].
///
/// An ack proves a permit is held; connect success only proves the OS backlog accepted it.
#[cfg(unix)]
fn spawn_concurrent_connection_attempts(
    socket_path: &std::path::Path,
    count: usize,
    active: &Arc<std::sync::atomic::AtomicUsize>,
    peak: &Arc<std::sync::atomic::AtomicUsize>,
) -> (
    Arc<tokio::sync::Barrier>,
    Vec<tokio::task::JoinHandle<ConnectionOutcome>>,
) {
    use std::sync::atomic::Ordering;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;
    use tokio::sync::Barrier;

    let barrier = Arc::new(Barrier::new(count.saturating_add(1_usize))); // count tasks + main task
    let mut handles = vec![];

    for _ in 0_i32..i32::try_from(count).unwrap_or(i32::MAX) {
        let socket_path_clone = socket_path.to_path_buf();
        let barrier_clone = Arc::clone(&barrier);
        let active_clone = Arc::clone(active);
        let peak_clone = Arc::clone(peak);
        let handle = tokio::spawn(async move {
            barrier_clone.wait().await;

            // Retry so a full OS listen backlog doesn't masquerade as semaphore rejection.
            let connect_deadline = tokio::time::Instant::now()
                .checked_add(std::time::Duration::from_secs(8_u64))
                .unwrap_or_else(tokio::time::Instant::now);
            let mut stream = loop {
                match UnixStream::connect(&socket_path_clone).await {
                    Ok(stream) => break stream,
                    Err(_) if tokio::time::Instant::now() < connect_deadline => {
                        tokio::time::sleep(std::time::Duration::from_millis(20_u64)).await;
                    }
                    Err(_) => return ConnectionOutcome::Failed,
                }
            };
            let _ = stream.write_all(b"{\"op\":\"ping\"}\n").await;

            // Ack = permit held; close before any byte = saturated. Unix may surface
            // the close as a reset instead of EOF.
            let mut buf = [0_u8; 128];
            let Ok(read_result) =
                tokio::time::timeout(std::time::Duration::from_secs(5_u64), stream.read(&mut buf))
                    .await
            else {
                return ConnectionOutcome::Failed;
            };
            match read_result {
                Ok(0_usize) => return ConnectionOutcome::Busy,
                Err(e) if e.kind() == std::io::ErrorKind::ConnectionReset => {
                    return ConnectionOutcome::Busy;
                }
                Err(_) => return ConnectionOutcome::Failed,
                Ok(_) => {}
            }

            let now_active = active_clone
                .fetch_add(1_usize, Ordering::SeqCst)
                .saturating_add(1_usize);
            peak_clone.fetch_max(now_active, Ordering::SeqCst);

            // Hold the permit until every attempt lands; shorter holds let early permits
            // expire under load and inflate the success count.
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;

            active_clone.fetch_sub(1_usize, Ordering::SeqCst);
            ConnectionOutcome::Success
        });
        handles.push(handle);
    }

    (barrier, handles)
}

/// Raise the process's soft `RLIMIT_NOFILE` toward its hard limit when the ambient shell/CI
/// environment left it at a low default.
///
/// macOS defaults to 256, too few for 250 client sockets plus server fds.
#[cfg(unix)]
fn ensure_min_open_file_limit(min_soft: libc::rlim_t) {
    // SAFETY: `rlim` is POD; the calls only access it via our pointers and report failure by return code.
    unsafe {
        let mut rlim = libc::rlimit {
            rlim_cur: 0,
            rlim_max: 0,
        };
        if libc::getrlimit(libc::RLIMIT_NOFILE, &raw mut rlim) != 0_i32 {
            return;
        }
        if rlim.rlim_cur >= min_soft {
            return;
        }
        rlim.rlim_cur = min_soft.min(rlim.rlim_max);
        let _ = libc::setrlimit(libc::RLIMIT_NOFILE, &raw const rlim);
    }
}

#[cfg(unix)]
#[tokio::test]
#[serial_test::serial]
async fn test_concurrent_connection_limit() {
    use tokio::time::timeout;

    #[cfg(unix)]
    ensure_min_open_file_limit(1024_u64);

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test_concurrent_limit.sock");

    let mut server = endpoint(socket_path.clone());

    let server_handle = tokio::spawn(async move {
        let _ = server.start().await;
    });

    wait_for_socket(&socket_path, SOCKET_TIMEOUT).await;

    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0_usize));
    let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0_usize));
    let (barrier, handles) =
        spawn_concurrent_connection_attempts(&socket_path, 250_usize, &active, &peak);
    let _ = barrier.wait().await;

    // Admission fails fast; the timeout only catches a genuine hang.
    let mut successful_connections = 0_i32;
    let mut busy_connections = 0_i32;
    let mut failed_connections = 0_i32;
    for handle in handles {
        match timeout(Duration::from_secs(15), handle).await {
            Ok(Ok(ConnectionOutcome::Success)) => {
                successful_connections = successful_connections.saturating_add(1_i32);
            }
            Ok(Ok(ConnectionOutcome::Busy)) => {
                busy_connections = busy_connections.saturating_add(1_i32);
            }
            _ => {
                failed_connections = failed_connections.saturating_add(1_i32);
            }
        }
    }

    let peak_concurrent = peak.load(std::sync::atomic::Ordering::SeqCst);

    // Print counts before asserting so a rare load-induced failure is diagnosable.
    eprintln!(
        "concurrent-limit observed: successful={successful_connections} busy={busy_connections} failed={failed_connections} peak={peak_concurrent} (of 250 attempts)"
    );

    // The 200 permits are fixed and all attempts arrive within the hold window,
    // so exactly 200 succeed and the rest are rejected immediately.
    assert_eq!(
        successful_connections, 200_i32,
        "expected exactly 200 connections (one per permit) to succeed, got {successful_connections}"
    );
    assert_eq!(
        busy_connections, 50_i32,
        "expected the remaining 50 connections to be closed fast without a reply, got {busy_connections}"
    );
    assert_eq!(
        failed_connections, 0_i32,
        "no connection should hang or fail outright, got {failed_connections} failures"
    );
    // Only permit holders can ack, so peak is bounded by the semaphore.
    assert!(
        peak_concurrent <= 200_usize,
        "peak concurrent connections {peak_concurrent} exceeded the 200-permit semaphore limit"
    );

    println!(
        "Concurrent connections test: {successful_connections} succeeded, {busy_connections} rejected as busy out of 250; peak concurrent = {peak_concurrent}"
    );
    server_handle.abort();
}

/// Send a request and read the response.
#[cfg(unix)]
async fn send_and_read_response(
    stream: &mut tokio::net::UnixStream,
    request: &str,
    buffer: &mut [u8],
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::timeout;

    stream
        .write_all(request.as_bytes())
        .await
        .expect("Failed to send request");

    let n = timeout(Duration::from_secs(1), stream.read(buffer))
        .await
        .expect("Timeout reading response")
        .expect("Failed to read response");

    let slice = buffer.get(..n).unwrap_or_default();
    String::from_utf8_lossy(slice).to_string()
}

/// A workspace update for an unregistered PID is rejected, and accepted once
/// the PID registers.
///
/// After an agent restart, shells are still registered client-side but not
/// server-side; the error reply is what triggers them to re-register.
#[cfg(unix)]
#[tokio::test]
async fn test_workspace_update_requires_registration() {
    use tokio::net::UnixStream;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test_workspace_registration.sock");

    let mut server = endpoint(socket_path.clone());

    let server_handle = tokio::spawn(async move {
        let _ = server.start().await;
    });

    wait_for_socket(&socket_path, SOCKET_TIMEOUT).await;

    let mut stream = UnixStream::connect(&socket_path)
        .await
        .expect("Failed to connect to test server");

    let test_pid = std::process::id();
    let mut buffer = vec![0u8; 1024];

    let workspace_request =
        format!("{{\"op\":\"workspace\",\"pid\":{test_pid},\"cwd\":\"/test/path\"}}\n");
    let response1 = send_and_read_response(&mut stream, &workspace_request, &mut buffer).await;

    assert!(
        response1.contains("error") || response1.contains("not registered"),
        "Workspace update without registration should return error. Got: {response1}"
    );

    let register_request =
        format!("{{\"op\":\"register\",\"pid\":{test_pid},\"cwd\":\"/test/path\"}}\n");
    let response2 = send_and_read_response(&mut stream, &register_request, &mut buffer).await;

    assert!(
        response2.contains("ok") || response2.contains("status"),
        "Registration should succeed. Got: {response2}"
    );

    let response3 = send_and_read_response(&mut stream, &workspace_request, &mut buffer).await;

    assert!(
        !response3.contains("error") && !response3.contains("not registered"),
        "Workspace update after registration should succeed. Got: {response3}"
    );

    drop(stream);
    server_handle.abort();
}

/// Send one request line on a fresh connection and return the full reply
/// line, including its trailing newline.
#[cfg(unix)]
async fn request_reply_line(socket_path: &std::path::Path, request: &str) -> String {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::time::timeout;

    let mut stream = tokio::net::UnixStream::connect(socket_path)
        .await
        .expect("Failed to connect to test server");
    stream
        .write_all(format!("{request}\n").as_bytes())
        .await
        .expect("Failed to send request");
    let mut reply = String::new();
    timeout(
        Duration::from_secs(5),
        BufReader::new(stream).read_line(&mut reply),
    )
    .await
    .expect("Timeout reading reply")
    .expect("Failed to read reply");
    reply
}

/// A failed prompt-segment request is answered with an empty line.
///
/// Holds for every prompt format and failure stage; JSON clients still get `{"error": ...}`.
#[cfg(unix)]
#[tokio::test]
#[serial_test::serial]
async fn ansi_error_replies_are_empty_lines() {
    let _env = set_test_env();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test_ansi_errors.sock");
    let fake_repo_dir = temp_dir.path().join("fake");
    fs::create_dir_all(fake_repo_dir.join(".git")).expect("create empty .git");
    let fake_repo = fake_repo_dir.to_str().expect("utf8 temp path");

    let mut server = endpoint(socket_path.clone());
    let server_handle = tokio::spawn(async move {
        let _ = server.start().await;
    });
    wait_for_socket(&socket_path, SOCKET_TIMEOUT).await;

    for format in ["ansi", "bash-prompt", "zsh-prompt", "json"] {
        let git = format!(r#"{{"op":"git","cwd":"{fake_repo}","format":"{format}"}}"#);
        let directory = format!(r#"{{"op":"directory","cwd":"/etc","format":"{format}"}}"#);
        for request in [git, directory] {
            let reply = request_reply_line(&socket_path, &request).await;
            if format == "json" {
                let value: serde_json::Value =
                    serde_json::from_str(&reply).expect("json reply must parse");
                assert!(
                    value.get("error").is_some_and(serde_json::Value::is_string),
                    "json error reply to {request} must carry an error string, got {reply}"
                );
            } else {
                assert_eq!(
                    reply, "\n",
                    "error reply to {request} must be an empty line"
                );
            }
        }
    }

    server_handle.abort();
}

/// Point `GPY_CONFIG_PATH` at a non-existent file so the agent falls back to
/// its builtin defaults instead of the developer's config.
#[must_use]
fn set_test_env() -> EnvGuard {
    EnvGuard::set_one("GPY_CONFIG_PATH", "/nonexistent/config.toml")
}

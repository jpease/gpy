//! Integration tests for end-to-end IPC functionality
//!
//! # Test Navigation Map
//!
//! **Total Tests**: 9
//! **Last Updated**: 2025-11-17
//!
//! ## Test Categories
//!
//! ### ✅ Basic Operations (3 tests)
//! Tests covering normal/happy path scenarios:
//! - `test_ipc_server_creation` - IPC server initialization and setup
//! - `test_language_detection_integration` - Language version detectors and Node detection
//! - `test_message_response_roundtrip` - Message and Response type creation
//!
//! ### ❌ Error Cases (2 tests)
//! Tests covering error scenarios and failure modes:
//! - `test_git_status_error_handling` - Invalid path graceful error handling
//! - `test_workspace_update_requires_registration` - PID registration requirement for workspace updates
//!
//! ### 🔀 Edge Cases (2 tests)
//! Tests covering boundary conditions:
//! - `test_message_size_limit_enforced` - 64KB message size limit enforcement
//! - `test_concurrent_connection_limit` - 200 concurrent connection semaphore limit
//!
//! ### 🔗 Integration (2 tests)
//! Tests covering cross-module interactions:
//! - `test_git_status_integration` - Git status through IPC server
//! - `test_git_status_with_temp_repo` - Git status in temporary repository

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(unsafe_code)]
#![allow(clippy::missing_panics_doc)] // Test functions panic on assertion failures

// ## What's NOT Tested (TODO)
// Known gaps for this module (from the original planning notes, since removed):
// - IPC server graceful shutdown and cleanup
// - Connection error recovery and retry logic
// - Multiple clients sending concurrent requests
// - Git cache invalidation and refresh
// - Theme manager integration with IPC
// - Config manager hot-reload during IPC operations
// - Latency tracking accuracy under load
// - Client directory cleanup after disconnection
// - Socket file permission handling
// - Server behavior when socket already exists
//
// ## Related Code
// - **Production**: `src/ipc/server.rs`, `src/ipc/mod.rs`, `src/git/status.rs`, `src/git/cache.rs`, `src/language/version.rs`
// - **Other Tests**: `e2e_ipc_tests.rs`, `agent_daemon_tests.rs`, `git_status_tests.rs`
// - **Fixtures**: None (uses tempfile for temp directories)
//
// ## Quick Reference
// ```bash
// cargo test --test integration_tests
// cargo test --test integration_tests -- --nocapture
// ```

use gpy_agent::config::manager::ConfigManager;
use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::git::status::load_repository_state;
use gpy_agent::ipc::server::EndpointHandle;
use gpy_agent::ipc::{ClientDirectory, LatencyTracker, Message, Response};
use gpy_agent::language::version::{NodeDetector, detect_language_release, get_version_detectors};
use gpy_agent::theme::ThemeManager;
use std::fs;
use std::sync::Arc;
use tempfile::TempDir;

#[test]
fn test_ipc_server_creation() {
    // Test that IPC server can be created
    // Use a unique temporary socket path to avoid conflicts with other tests
    let _env = set_test_env();
    let Ok(temp_dir) = TempDir::new() else { return };
    let socket_path = temp_dir.path().join("test_ipc_server.sock");

    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(GitStatusCache::new());
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let watcher = None; // No watcher for this test
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let instant_cache =
        Arc::new(gpy_agent::cache::InstantPromptCache::new().expect("instant cache should work"));
    let latency_tracker = Arc::new(LatencyTracker::new(100));
    let language_cache = gpy_agent::language::DetectionCache::new();
    let _server = EndpointHandle::builder()
        .socket_path(socket_path)
        .client_registry(registry)
        .git_cache(git_cache)
        .config_manager(config_manager)
        .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
        .theme_manager(theme_manager)
        .instant_cache(instant_cache)
        .latency_tracker(latency_tracker)
        .language_cache(language_cache)
        .build()
        .expect("server build");
    // Server created successfully - test passes
}

#[test]
fn test_git_status_integration() {
    let _env = set_test_env();
    // Test git status functionality that IPC server uses
    let result = load_repository_state(".", None, 0, true);

    assert!(
        result.is_ok(),
        "Git status loading should succeed for current repository"
    );
    let Ok(git_status) = result else { return };
    assert!(
        git_status.is_some(),
        "Should detect git repository in current directory"
    );

    let Some(status) = git_status else { return };
    assert!(
        !status.status.branch.is_empty(),
        "Git repository should have a current branch name"
    );
}

#[test]
fn test_language_detection_integration() {
    let _env = set_test_env();
    // Test language detection components
    let detectors = get_version_detectors();
    assert_eq!(
        detectors.len(),
        10,
        "Should have all 10 language detectors including Fish"
    ); // Should have all 10 language detectors (including Fish)

    // Test version detection for a common language that might be available
    let node_detector = NodeDetector;
    let version_result = detect_language_release(&node_detector);

    // Version detection might fail if node isn't installed, but shouldn't panic
    if let Some(version) = version_result {
        assert!(
            !version.is_empty(),
            "Detected Node version should not be empty string"
        );
    }
}

#[test]
fn test_git_status_with_temp_repo() {
    let _env = set_test_env();
    let Ok(temp_dir) = TempDir::new() else { return };
    let temp_path = temp_dir.path();

    // Initialize a git repository
    let init_result = std::process::Command::new("git")
        .arg("init")
        .current_dir(temp_path)
        .output();

    if let Ok(output) = init_result
        && output.status.success()
    {
        // Configure git
        std::process::Command::new("git")
            .args(["config", "user.email", "test@example.com"])
            .current_dir(temp_path)
            .output()
            .ok();
        std::process::Command::new("git")
            .args(["config", "user.name", "Test User"])
            .current_dir(temp_path)
            .output()
            .ok();

        // Create a file
        let _ = fs::write(temp_path.join("test.txt"), "test content");

        // Test git status on this repo
        let result = load_repository_state(temp_path.to_string_lossy().as_ref(), None, 0, true);
        assert!(
            result.is_ok(),
            "Git status should succeed for freshly initialized repository"
        );

        if let Ok(Some(status)) = result {
            // New repo should have a branch
            assert!(
                !status.status.branch.is_empty(),
                "Initialized repository should have default branch name"
            );
            // Should have untracked files
            assert!(
                status.status.untracked > 0,
                "Repository with untracked test.txt should report untracked files"
            );
        }
    }
    // If git is not available, the test still passes
}

#[test]
fn test_message_response_roundtrip() {
    // Test that messages and responses can be created and used
    let msg = Message::Ping;
    let response = Response::Ack;
    assert!(
        matches!(msg, Message::Ping),
        "Should create Message::Ping variant"
    );
    assert!(
        matches!(response, Response::Ack),
        "Should create Response::Ack variant"
    );
}

#[test]
fn test_git_status_error_handling() {
    // Test error handling for invalid paths
    let result = load_repository_state(
        "/absolutely/nonexistent/path/that/cannot/exist",
        None,
        0,
        true,
    );

    // Should handle gracefully - either Ok(None) or Err
    assert!(
        matches!(result, Ok(None) | Err(_)),
        "Should handle nonexistent path gracefully without panicking"
    );
}

#[tokio::test]
#[allow(clippy::unused_async)]
async fn test_message_size_limit_enforced() {
    // Test that messages exceeding 64KB are rejected
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::UnixStream;

    let Ok(temp_dir) = TempDir::new() else { return };
    let socket_path = temp_dir.path().join("test_size_limit.sock");

    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(GitStatusCache::new());
    let watcher = None;
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let instant_cache =
        Arc::new(gpy_agent::cache::InstantPromptCache::new().expect("instant cache should work"));
    let latency_tracker = Arc::new(LatencyTracker::new(100));
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let language_cache = gpy_agent::language::DetectionCache::new();

    let mut server = EndpointHandle::builder()
        .socket_path(socket_path.clone())
        .client_registry(registry)
        .git_cache(git_cache)
        .config_manager(config_manager)
        .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
        .theme_manager(theme_manager)
        .instant_cache(instant_cache)
        .latency_tracker(latency_tracker)
        .language_cache(language_cache)
        .build()
        .expect("server build");

    // Start server in background
    tokio::spawn(async move {
        let _ = server.start().await;
    });

    // Give server time to start
    tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

    // Try to connect and send oversized message
    if let Ok(mut stream) = UnixStream::connect(&socket_path).await {
        // Create a message larger than 64KB without a newline
        // This should trigger the accumulated.len() guard in server.rs
        let oversized_data = vec![b'X'; 70_000]; // 70KB of 'X' characters

        // Write data in chunks to avoid single-write size limits
        for chunk in oversized_data.chunks(8192) {
            if stream.write_all(chunk).await.is_err() {
                break;
            }
        }

        // Try to read response - connection should be dropped
        let mut response = vec![0u8; 1024];
        match tokio::time::timeout(
            tokio::time::Duration::from_millis(500),
            stream.read(&mut response),
        )
        .await
        {
            Ok(Ok(0) | Err(_)) => {
                // Connection closed by server - this is what we expect
            }
            Ok(Ok(n)) => {
                panic!("Server should have dropped connection but sent {n} bytes");
            }
            Err(timeout_err) => {
                // Timeout - connection might be hung, this is bad
                panic!("Connection timed out - size limit may not be enforced: {timeout_err}");
            }
        }
    }
}

/// Outcome of a single connection attempt in
/// `spawn_concurrent_connection_attempts`.
///
/// Since #316, a saturated connection semaphore fails fast instead of
/// queueing the connection forever, and since #680 it does so by closing
/// the connection without writing a byte (any reply line would land in a
/// prompt), so EOF before the first byte is the busy signal and a non-empty
/// read proves the handler held a permit.
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
/// Because a real ack can only arrive after the server-side handler has
/// acquired a connection semaphore permit, the `active`/`peak` counters
/// (updated only on [`ConnectionOutcome::Success`]) directly observe the
/// semaphore's concurrency cap instead of inferring it from connect-success
/// timing (see #255: `connect()` succeeding only proves the OS accepted the
/// socket into its backlog, not that the semaphore granted a permit).
#[allow(clippy::unused_async)]
async fn spawn_concurrent_connection_attempts(
    socket_path: &std::path::Path,
    count: usize,
    active: Arc<std::sync::atomic::AtomicUsize>,
    peak: Arc<std::sync::atomic::AtomicUsize>,
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
        let active_clone = Arc::clone(&active);
        let peak_clone = Arc::clone(&peak);
        let handle = tokio::spawn(async move {
            barrier_clone.wait().await;

            // The OS listen backlog (commonly ~128 on macOS, larger on
            // Linux CI) is independent of the application-level semaphore
            // and can reject a connect() outright while it is full. Retry
            // with a short backoff so the test measures the semaphore's
            // behavior rather than an incidental OS backlog size.
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

            // Read whatever arrives: either a real ack (the handler acquired
            // a semaphore permit) or a close before any byte (#316/#680 -
            // the semaphore was saturated, no permit acquired). Only the
            // former proves permit-gated concurrency. Closing a Unix socket
            // with the unread ping still queued can surface as a reset
            // rather than a clean EOF, depending on the platform.
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

            // Hold the connection open so the server keeps its permit for a
            // realistic duration, forcing genuine wave-based contention.
            // 3s (was 500ms): under heavy external load (I/O/CPU stalls),
            // some of the 250 barrier-released tasks can take over 500ms
            // just to get scheduled and reach the server, letting an early
            // permit expire before the last attempts land and inflating
            // `successful_connections` past 200 (observed: 226). 3s gives
            // enough margin for realistic contention while staying well
            // under the 15s per-handle await timeout below.
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
/// E.g. macOS's out-of-the-box 256 — well below what 250 concurrent client sockets plus the
/// server's own fds need. Without this, `accept()` can spuriously hit `EMFILE` under a low
/// ambient limit, failing the test for an environmental reason unrelated to the semaphore
/// behavior under test.
#[cfg(unix)]
fn ensure_min_open_file_limit(min_soft: libc::rlim_t) {
    // SAFETY: `rlim` is a plain-old-data struct; getrlimit/setrlimit only
    // read/write it through the pointers we pass and report failure via
    // their return code, which is checked before use.
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

#[tokio::test]
#[serial_test::serial]
async fn test_concurrent_connection_limit() {
    // Test that concurrent connections are limited by semaphore
    use tokio::time::{Duration, timeout};

    #[cfg(unix)]
    ensure_min_open_file_limit(1024_u64);

    let Ok(temp_dir) = TempDir::new() else { return };
    let socket_path = temp_dir.path().join("test_concurrent_limit.sock");

    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(GitStatusCache::new());
    let watcher = None;
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let instant_cache =
        Arc::new(gpy_agent::cache::InstantPromptCache::new().expect("instant cache should work"));
    let latency_tracker = Arc::new(LatencyTracker::new(100));
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let language_cache = gpy_agent::language::DetectionCache::new();

    let mut server = EndpointHandle::builder()
        .socket_path(socket_path.clone())
        .client_registry(registry)
        .git_cache(git_cache)
        .config_manager(config_manager)
        .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
        .theme_manager(theme_manager)
        .instant_cache(instant_cache)
        .latency_tracker(latency_tracker)
        .language_cache(language_cache)
        .build()
        .expect("server build");

    tokio::spawn(async move {
        let _ = server.start().await;
    });

    tokio::time::sleep(Duration::from_millis(100)).await;

    let active = Arc::new(std::sync::atomic::AtomicUsize::new(0_usize));
    let peak = Arc::new(std::sync::atomic::AtomicUsize::new(0_usize));
    let (barrier, handles) = spawn_concurrent_connection_attempts(
        &socket_path,
        250_usize,
        Arc::clone(&active),
        Arc::clone(&peak),
    )
    .await;
    let _ = barrier.wait().await;

    // Wait for every attempt to resolve. Since #316, admission control fails
    // fast instead of queueing: the bound here exists only to catch a
    // genuine hang under CI contention, not to gate the assertions.
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

    // Emit the observed split BEFORE the assertions. A failing assert_eq aborts
    // at the first tripped check, and the summary println below only runs on
    // success — so without this line a rare failure reports a single count with
    // no context. This test has been observed to fail very rarely under heavy
    // external load (I/O/CPU stalls that push some of the 250 attempts past the
    // permit hold window); capturing all four counts up front makes the next
    // such failure self-diagnosing.
    eprintln!(
        "concurrent-limit observed: successful={successful_connections} busy={busy_connections} failed={failed_connections} peak={peak_concurrent} (of 250 attempts)"
    );

    // Since #316, a saturated semaphore is rejected fast (since #680 by a
    // close without any reply) instead of queueing indefinitely: exactly
    // 200 of the 250 concurrent attempts can hold one of the 200 permits
    // (all 250 attempts land on the server well within the permit hold
    // window, before any permit is released, so this is a guarantee from
    // the fixed permit supply, not a timing heuristic), and the rest are
    // rejected immediately rather than hanging.
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
    // Real invariant: a client only counts as "active" once it has read a
    // real ack, which can only have been written by a server-side handler
    // that already holds one of the ≤200 semaphore permits. This bound is a
    // guarantee from `tokio::sync::Semaphore`, not a timing heuristic.
    assert!(
        peak_concurrent <= 200_usize,
        "peak concurrent connections {peak_concurrent} exceeded the 200-permit semaphore limit"
    );

    println!(
        "Concurrent connections test: {successful_connections} succeeded, {busy_connections} rejected as busy out of 250; peak concurrent = {peak_concurrent}"
    );
}

/// Helper: Send a request and read the response
async fn send_and_read_response(
    stream: &mut tokio::net::UnixStream,
    request: &str,
    buffer: &mut [u8],
) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::time::{Duration, timeout};

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

#[tokio::test]
/// Test that workspace update fails with error when PID is not registered
///
/// This is a critical regression test for the auto-reconnection feature.
/// When the agent restarts, Fish shells lose their registration server-side
/// but remain registered client-side. When they try to update workspace,
/// the server should return an error, triggering re-registration.
#[allow(clippy::unused_async)]
async fn test_workspace_update_requires_registration() {
    use tokio::net::UnixStream;
    use tokio::time::Duration;

    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test_workspace_registration.sock");

    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(GitStatusCache::new());
    let watcher = None;
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let instant_cache =
        Arc::new(gpy_agent::cache::InstantPromptCache::new().expect("instant cache should work"));
    let latency_tracker = Arc::new(LatencyTracker::new(100));
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let language_cache = gpy_agent::language::DetectionCache::new();

    let mut server = EndpointHandle::builder()
        .socket_path(socket_path.clone())
        .client_registry(Arc::clone(&registry))
        .git_cache(git_cache)
        .config_manager(config_manager)
        .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
        .theme_manager(theme_manager)
        .instant_cache(instant_cache)
        .latency_tracker(latency_tracker)
        .language_cache(language_cache)
        .build()
        .expect("server build");

    // Start server in background
    let server_handle = tokio::spawn(async move {
        let _ = server.start().await;
    });

    // Give server time to start
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Connect to server
    let mut stream = UnixStream::connect(&socket_path)
        .await
        .expect("Failed to connect to test server");

    let test_pid = std::process::id();
    let mut buffer = vec![0u8; 1024];

    // Test 1: Try workspace update WITHOUT registering first
    let workspace_request =
        format!("{{\"op\":\"workspace\",\"pid\":{test_pid},\"cwd\":\"/test/path\"}}\n");
    let response1 = send_and_read_response(&mut stream, &workspace_request, &mut buffer).await;

    assert!(
        response1.contains("error") || response1.contains("not registered"),
        "Workspace update without registration should return error. Got: {response1}"
    );

    // Test 2: Register the PID
    let register_request =
        format!("{{\"op\":\"register\",\"pid\":{test_pid},\"cwd\":\"/test/path\"}}\n");
    let response2 = send_and_read_response(&mut stream, &register_request, &mut buffer).await;

    assert!(
        response2.contains("ok") || response2.contains("status"),
        "Registration should succeed. Got: {response2}"
    );

    // Test 3: Workspace update should now succeed
    let response3 = send_and_read_response(&mut stream, &workspace_request, &mut buffer).await;

    assert!(
        !response3.contains("error") && !response3.contains("not registered"),
        "Workspace update after registration should succeed. Got: {response3}"
    );

    // Cleanup
    drop(stream);
    server_handle.abort();
}

/// Send one request line on a fresh connection and return the full reply
/// line, including its trailing newline.
async fn request_reply_line(socket_path: &std::path::Path, request: &str) -> String {
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::time::{Duration, timeout};

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

/// #680: a failed prompt-segment request is answered with an empty line.
///
/// This holds in every rendered prompt format, whether the request fails
/// while resolving its path (a denylisted `cwd`) or in the handler (a stray
/// empty `.git` directory), so no protocol JSON is ever printed into a
/// prompt. JSON clients still get the `{"error": ...}` object.
#[tokio::test]
async fn ansi_error_replies_are_empty_lines() {
    use tokio::time::Duration;

    let _env = set_test_env();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test_ansi_errors.sock");
    let fake_repo_dir = temp_dir.path().join("fake");
    fs::create_dir_all(fake_repo_dir.join(".git")).expect("create empty .git");
    let fake_repo = fake_repo_dir.to_str().expect("utf8 temp path");

    let mut server = EndpointHandle::builder()
        .socket_path(socket_path.clone())
        .client_registry(ClientDirectory::new().shared())
        .git_cache(Arc::new(GitStatusCache::new()))
        .config_manager(Arc::new(
            ConfigManager::with_defaults().expect("default config should load"),
        ))
        .watcher_slot(Arc::new(std::sync::Mutex::new(None)))
        .theme_manager(Arc::new(
            ThemeManager::builtin("default").expect("default theme should load"),
        ))
        .instant_cache(Arc::new(
            gpy_agent::cache::InstantPromptCache::new().expect("instant cache should work"),
        ))
        .latency_tracker(Arc::new(LatencyTracker::new(100)))
        .language_cache(gpy_agent::language::DetectionCache::new())
        .build()
        .expect("server build");
    let server_handle = tokio::spawn(async move {
        let _ = server.start().await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

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

/// #759: formats the socket does not serve (`fish`, `bash-source`,
/// `zsh-source`) get one newline-terminated JSON error line naming the
/// format, instead of silence until the client times out.
#[tokio::test]
async fn unsupported_socket_format_gets_json_error() {
    use tokio::time::Duration;

    let _env = set_test_env();
    let temp_dir = TempDir::new().expect("Failed to create temp dir");
    let socket_path = temp_dir.path().join("test_unsupported_format.sock");

    let mut server = EndpointHandle::builder()
        .socket_path(socket_path.clone())
        .client_registry(ClientDirectory::new().shared())
        .git_cache(Arc::new(GitStatusCache::new()))
        .config_manager(Arc::new(
            ConfigManager::with_defaults().expect("default config should load"),
        ))
        .watcher_slot(Arc::new(std::sync::Mutex::new(None)))
        .theme_manager(Arc::new(
            ThemeManager::builtin("default").expect("default theme should load"),
        ))
        .instant_cache(Arc::new(
            gpy_agent::cache::InstantPromptCache::new().expect("instant cache should work"),
        ))
        .latency_tracker(Arc::new(LatencyTracker::new(100)))
        .language_cache(gpy_agent::language::DetectionCache::new())
        .build()
        .expect("server build");
    let server_handle = tokio::spawn(async move {
        let _ = server.start().await;
    });
    tokio::time::sleep(Duration::from_millis(100)).await;

    for (format, expected) in [
        ("fish", "only available to local CLI oneshot"),
        ("bash-source", "not yet implemented"),
        ("zsh-source", "not yet implemented"),
    ] {
        let request = format!(r#"{{"op":"duration","duration_ms":5,"format":"{format}"}}"#);
        let reply = request_reply_line(&socket_path, &request).await;
        assert!(
            reply.ends_with('\n'),
            "reply must end with a newline: {reply:?}"
        );
        let value: serde_json::Value =
            serde_json::from_str(&reply).expect("reply must be a JSON line");
        let message = value
            .get("error")
            .and_then(serde_json::Value::as_str)
            .unwrap_or_else(|| panic!("reply must carry an error string, got {reply}"));
        assert!(
            message.contains(expected),
            "error for {format} must contain {expected:?}, got {message:?}"
        );
    }

    server_handle.abort();
}
fn set_test_env() -> TestEnvGuard {
    let prev = std::env::var_os("GPY_CONFIG_PATH");
    unsafe {
        std::env::set_var("GPY_CONFIG_PATH", "/nonexistent/config.toml");
    }
    TestEnvGuard { prev }
}

struct TestEnvGuard {
    prev: Option<std::ffi::OsString>,
}

impl Drop for TestEnvGuard {
    fn drop(&mut self) {
        if let Some(prev) = self.prev.take() {
            unsafe {
                std::env::set_var("GPY_CONFIG_PATH", prev);
            }
        } else {
            unsafe {
                std::env::remove_var("GPY_CONFIG_PATH");
            }
        }
    }
}
// These tests assume a clean configuration. `set_test_env` points GPY_CONFIG_PATH
// at a non-existent file so the agent falls back to its builtin defaults.

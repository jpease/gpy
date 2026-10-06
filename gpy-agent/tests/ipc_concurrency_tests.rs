//! Concurrency tests for asynchronous cache-miss handling (#154).
//!
//! These tests verify that expensive cold cache misses (git status, language
//! detection) no longer block Tokio worker threads. Request routing runs on the
//! blocking pool via `spawn_blocking`, so cached/warm requests (ping) stay
//! responsive while many simultaneous misses are in flight.
//!
//! # What is exercised
//!
//! - `test_mixed_cold_and_warm_requests_all_succeed` — many simultaneous cold git
//!   misses, language detections, and warm pings all complete successfully.
//! - `test_pings_stay_responsive_under_cold_git_load` — pings round-trip within a
//!   bounded budget while a batch of cold git misses runs concurrently. Before the
//!   #154 fix, the synchronous single-flight waits ran inline on worker threads and
//!   could starve the runtime; offloading to the blocking pool keeps ping latency
//!   bounded.
//!
//! ## Quick Reference
//! ```bash
//! cargo test --test ipc_concurrency_tests
//! ```

#![allow(clippy::panic)]
#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::missing_errors_doc)]

use gpy_agent::config::manager::ConfigManager;
use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::ipc::ClientDirectory;
use gpy_agent::ipc::LatencyTracker;
use gpy_agent::ipc::server::EndpointHandle;
use gpy_agent::security::GuardSettings;
use gpy_agent::theme::ThemeManager;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tempfile::{TempDir, tempdir};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

/// Start an in-process IPC server, wait for the socket, and return the join
/// handle plus socket path. The returned `TempDir` guard must stay alive.
async fn start_server(socket_name: &str) -> (tokio::task::JoinHandle<()>, PathBuf, TempDir) {
    let config_manager = Arc::new(ConfigManager::with_defaults().expect("default config"));
    start_server_with(socket_name, config_manager, GuardSettings::default()).await
}

/// Same as [`start_server`], but with an explicit config manager and
/// security config.
///
/// Lets tests shrink `max_concurrent_connections` or the per-read timeout to
/// exercise the admission-control paths (#316) deterministically instead of
/// needing hundreds of real connections.
async fn start_server_with(
    socket_name: &str,
    config_manager: Arc<ConfigManager>,
    security_config: GuardSettings,
) -> (tokio::task::JoinHandle<()>, PathBuf, TempDir) {
    let temp_dir = tempdir().expect("temp dir");
    let socket_path = temp_dir.path().join(socket_name);
    let server_socket_path = socket_path.clone();

    let server_handle = tokio::spawn(async move {
        let mut server = EndpointHandle::builder()
            .socket_path(server_socket_path)
            .client_registry(ClientDirectory::new().shared())
            .git_cache(Arc::new(GitStatusCache::new()))
            .config_manager(config_manager)
            .watcher_slot(Arc::new(std::sync::Mutex::new(None)))
            .theme_manager(Arc::new(ThemeManager::builtin("default").expect("theme")))
            .instant_cache(Arc::new(
                gpy_agent::cache::InstantPromptCache::new().expect("instant cache"),
            ))
            .latency_tracker(Arc::new(LatencyTracker::new(1000)))
            .language_cache(gpy_agent::language::DetectionCache::new())
            .security_config(security_config)
            .build()
            .expect("server build");
        server.start().await.expect("server start");
    });

    timeout(Duration::from_secs(5), async {
        while !socket_path.exists() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("server socket not ready in time");

    (server_handle, socket_path, temp_dir)
}

/// Create a committed git repository so status requests do real (cold) work.
fn make_repo() -> TempDir {
    let repo = tempdir().expect("repo dir");
    let path = repo.path();
    for args in [
        vec!["init"],
        vec!["config", "user.email", "test@example.com"],
        vec!["config", "user.name", "Test User"],
    ] {
        std::process::Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("git command");
    }
    std::fs::write(path.join("README.md"), "# Concurrency Test\n").expect("write file");
    for args in [vec!["add", "README.md"], vec!["commit", "-m", "init"]] {
        std::process::Command::new("git")
            .args(args)
            .current_dir(path)
            .output()
            .expect("git command");
    }
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
        match kind {
            "ping" => assert!(
                response.contains(r#""status":"ok""#),
                "ping should report ok, got: {response}"
            ),
            // Git/language responses are valid JSON (status object or detection
            // result); a cold miss must not produce a connection failure.
            _ => assert!(
                serde_json::from_str::<serde_json::Value>(response.trim()).is_ok(),
                "{kind} response should be valid JSON, got: {response}"
            ),
        }
    }

    server_handle.abort();
}

/// Pings stay responsive while a batch of cold git misses runs concurrently.
///
/// The two worker threads would be saturated by inline blocking single-flight
/// waits before #154; with the waits offloaded to the blocking pool, ping
/// round-trips stay well under the budget.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_pings_stay_responsive_under_cold_git_load() {
    let (server_handle, socket_path, _temp_dir) =
        start_server("gpy-concurrency-responsive.sock").await;

    let repos: Vec<TempDir> = (0_usize..16_usize).map(|_| make_repo()).collect();

    // Kick off the cold git misses concurrently and keep them in flight.
    let mut git_tasks = Vec::new();
    for repo in &repos {
        let socket = socket_path.clone();
        let payload = git_payload(repo.path());
        git_tasks.push(tokio::spawn(
            async move { request(&socket, &payload).await },
        ));
    }

    // Concurrently fire pings and record per-request latency.
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

    // Generous bound: catches a full runtime stall without being flaky on slow CI.
    // Worker threads are never blocked on cache-miss waits after #154, so pings
    // round-trip in milliseconds in practice.
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

/// Saturating the connection semaphore must reject new connections fast,
/// by closing them without a reply.
///
/// Fast rejection instead of queueing forever is #316 (acceptance criterion
/// 2). Closing without a reply is #680: the request is never read, so its
/// format is unknown and any reply line could land in a prompt.
///
/// Regression check: this test hangs against `semaphore.acquire().await`
/// (blocks indefinitely once the single permit is held), and fails against
/// the pre-#680 rejection that wrote a JSON `Response::Error` line.
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

    // Hold the only permit: connect and send a byte with no trailing
    // newline. The server's read loop blocks waiting for the rest of the
    // line, so this connection (and its semaphore permit) stays alive for
    // as long as we hold `holder` open.
    let mut holder = timeout(Duration::from_secs(5), UnixStream::connect(&socket_path))
        .await
        .expect("connect")
        .expect("connect");
    holder
        .write_all(b"x")
        .await
        .expect("write partial (unterminated) line");

    // Give the accept loop time to spawn the holder's handler task and
    // acquire the sole permit before the second connection races it; the
    // holder is the only connection in play so it is guaranteed to win once
    // its task has actually run.
    tokio::time::sleep(Duration::from_millis(300)).await;

    // A second connection must fail fast instead of queueing behind the
    // saturated semaphore. Bound the wait well under the server's
    // per-read/connection timeouts (default request timeout is 5s) so a
    // regression back to a blocking `acquire().await` fails this test
    // instead of hanging the whole suite.
    //
    // The rejection is admission-control, not request/response: the server
    // closes as soon as it fails to acquire a permit, without ever reading
    // from this connection. That can race ahead of our own write of the
    // ping payload, so the write below is best-effort (`BrokenPipe` is an
    // expected outcome, not a test failure) -- only the read matters, and
    // closing with the ping still unread may surface as a reset rather than
    // a clean EOF, depending on the platform.
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

/// A connection cannot live longer than the documented whole-connection
/// deadline, even if no individual `read()` ever times out (#316,
/// acceptance criterion 1).
///
/// A slow-loris client trickles single bytes (no newline, so no complete
/// message ever arrives) spaced comfortably under the per-read timeout.
/// Before the fix, nothing ever ends this connection; after the fix, the
/// whole-connection deadline (4x the live per-read timeout, see
/// `spawn_client_handler` in `src/ipc/server/handle.rs`) force-closes it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn test_slow_loris_connection_hits_whole_connection_deadline() {
    // Shrink the live per-read timeout to 2s so the derived whole-connection
    // deadline (4x = 8s) keeps this test bounded instead of waiting on the
    // 5s production default (20s deadline). 2s (rather than the 1s
    // configuration minimum) leaves a wide margin between the 300ms trickle
    // interval below and the per-read timeout, so scheduler jitter from
    // other tests running in the same binary can't trip an individual
    // read() timeout and produce a false pass.
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

    // Trickle single bytes, each spaced well under the 2s per-read timeout,
    // for longer than the 8s whole-connection deadline. No individual
    // read() ever times out; only the whole-connection deadline can end
    // this connection. 30 * 300ms = 9s of trickling.
    for _ in 0_u32..30_u32 {
        let _ = stream.write_all(b"x").await;
        tokio::time::sleep(Duration::from_millis(300)).await;
    }

    // By now (~9s of trickling) the whole-connection deadline (~8s) must
    // already have force-closed the connection. Confirm the read side
    // observes EOF/error within a bounded window rather than hanging
    // indefinitely.
    let mut buf = [0_u8; 16];
    let read_result = timeout(Duration::from_secs(4), stream.read(&mut buf)).await;
    let outcome = read_result.expect(
        "connection must be closed at the whole-connection deadline, not left open indefinitely",
    );
    match outcome {
        // EOF (server closed the connection) or a reset/broken-pipe error
        // (also acceptable once the peer closed) both prove the connection
        // ended.
        Ok(0) | Err(_) => {}
        Ok(n) => panic!("expected EOF after the whole-connection deadline, got {n} bytes"),
    }

    server_handle.abort();
}

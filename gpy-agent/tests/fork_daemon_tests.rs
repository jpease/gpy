//! In-process IPC server lifecycle tests: socket permissions, binding to
//! assorted paths, repeated start/abort cycles, concurrent clients, and
//! binding failures.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]

mod common;

use common::fixtures::ServerGuard;
use common::{endpoint_builder, wait_for_socket};
use gpy_agent::Result;
use std::path::Path;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

const SOCKET_TIMEOUT: Duration = Duration::from_secs(2);

/// Send one `ping` request and return the raw response text.
async fn ping(socket_path: &Path) -> std::io::Result<String> {
    let mut stream = UnixStream::connect(socket_path).await?;
    stream.write_all(b"{\"op\":\"ping\"}\n").await?;
    let mut buf = [0u8; 1024];
    let n = stream.read(&mut buf).await?;
    Ok(String::from_utf8_lossy(buf.get(..n).unwrap_or_default()).into_owned())
}

/// Test socket binding permissions and security
#[tokio::test]
async fn test_socket_permissions() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("gpy-permissions-test.sock");

    let mut server = endpoint_builder(socket_path.clone())
        .build()
        .expect("server build");

    let server_handle = tokio::spawn(async move { server.start().await });
    let _guard = ServerGuard::new(server_handle);

    wait_for_socket(&socket_path, SOCKET_TIMEOUT).await;

    #[cfg(unix)]
    {
        let metadata = std::fs::metadata(&socket_path)?;
        let mode = metadata.permissions().mode() & 0o777;
        assert_eq!(
            mode, 0o600,
            "Socket permissions should be user-only (0600), got {mode:#o}"
        );
    }

    Ok(())
}

/// Test socket binding to different paths and error handling
#[tokio::test]
#[allow(clippy::unused_async)]
async fn test_socket_binding_variations() -> Result<()> {
    let temp_dir = tempdir()?;

    // Normal socket path
    let socket_path = temp_dir.path().join("test.sock");
    let server = endpoint_builder(socket_path).build().expect("server build");
    drop(server);

    // Nested directory path
    let nested_dir = temp_dir.path().join("nested").join("deep");
    std::fs::create_dir_all(&nested_dir)?;
    let server = endpoint_builder(nested_dir.join("test.sock"))
        .build()
        .expect("server build");
    drop(server);

    // Path with spaces
    let spaced_dir = temp_dir.path().join("path with spaces");
    std::fs::create_dir_all(&spaced_dir)?;
    let server = endpoint_builder(spaced_dir.join("test.sock"))
        .build()
        .expect("server build");
    drop(server);

    Ok(())
}

/// Each of several short-lived servers binds its socket and answers a ping
/// before being aborted.
#[tokio::test]
async fn test_rapid_socket_lifecycle() -> Result<()> {
    let temp_dir = tempdir()?;

    for i in 0_i32..5_i32 {
        let socket_path = temp_dir.path().join(format!("rapid-test-{i}.sock"));
        let mut server = endpoint_builder(socket_path.clone())
            .build()
            .expect("server build");

        let server_handle = tokio::spawn(async move { server.start().await });
        let guard = ServerGuard::new(server_handle);

        wait_for_socket(&socket_path, SOCKET_TIMEOUT).await;
        let response = ping(&socket_path).await?;
        assert!(
            response.contains(r#""status":"ok""#),
            "server {i} did not answer ping: {response}"
        );

        drop(guard);
    }

    Ok(())
}

/// Test concurrent socket access and connection limits
#[tokio::test]
async fn test_concurrent_connections() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("concurrent-test.sock");

    let mut server = endpoint_builder(socket_path.clone())
        .build()
        .expect("Server failed to build");
    let server_handle = tokio::spawn(async move {
        server.start().await.expect("Server failed to start");
    });
    let _guard = ServerGuard::new(server_handle);

    wait_for_socket(&socket_path, SOCKET_TIMEOUT).await;

    let mut handles = Vec::new();
    for _i in 0_i32..3_i32 {
        let socket_path_clone = socket_path.clone();
        handles.push(tokio::spawn(async move { ping(&socket_path_clone).await }));
    }

    for handle in handles {
        let response = handle.await.expect("Connection failed")?;
        assert!(response.contains(r#""status":"ok""#));
    }

    Ok(())
}

/// Aborting the server task closes its listener: the socket stops accepting
/// connections even though no shutdown path ran to unlink the file.
#[tokio::test]
async fn test_aborted_server_refuses_connections() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("cleanup-test.sock");

    let mut server = endpoint_builder(socket_path.clone())
        .build()
        .expect("server build");
    let guard = ServerGuard::new(tokio::spawn(async move { server.start().await }));

    wait_for_socket(&socket_path, SOCKET_TIMEOUT).await;
    assert!(
        ping(&socket_path).await?.contains(r#""status":"ok""#),
        "server should answer before it is aborted"
    );

    let server_handle = guard.into_inner();
    server_handle.abort();
    let join = server_handle.await;
    assert!(
        join.is_err_and(|e| e.is_cancelled()),
        "server task should end by cancellation"
    );

    // The aborted future (and the listener it owned) has been dropped, so a
    // connect must fail: refused if the stale socket file remains, not found
    // if it was unlinked.
    let connect = UnixStream::connect(&socket_path).await;
    let err = connect.expect_err("aborted server must not accept connections");
    assert!(
        matches!(
            err.kind(),
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound
        ),
        "unexpected connect error after abort: {err}"
    );

    Ok(())
}

/// Test IPC communication under load
#[tokio::test]
async fn test_ipc_under_load() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("load-test.sock");

    let mut server = endpoint_builder(socket_path.clone())
        .build()
        .expect("Server failed to build");
    let server_handle = tokio::spawn(async move {
        server.start().await.expect("Server failed to start");
    });
    let _guard = ServerGuard::new(server_handle);

    wait_for_socket(&socket_path, SOCKET_TIMEOUT).await;

    // Rapid one-shot requests, alternating request types
    let mut handles = Vec::new();
    for i in 0_i32..10_i32 {
        let socket_path_clone = socket_path.clone();
        let handle = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&socket_path_clone).await?;

            let request = if i % 2_i32 == 0_i32 {
                r#"{"op":"ping"}"#
            } else {
                r#"{"type":"git_status","path":"/tmp","format":"json"}"#
            };

            stream.write_all(request.as_bytes()).await?;
            stream.write_all(b"\n").await?;

            let mut response_buf = [0u8; 2048];
            let n = stream.read(&mut response_buf).await?;
            let response =
                std::str::from_utf8(response_buf.get(..n).ok_or("Invalid slice range")?)?;

            Ok::<String, Box<dyn std::error::Error + Send + Sync>>(response.to_owned())
        });
        handles.push(handle);
    }

    for (i, handle) in handles.into_iter().enumerate() {
        let response = handle.await.expect("Request failed").unwrap();
        if i % 2_usize == 0_usize {
            assert!(
                response.contains(r#""status":"ok""#),
                "Ping failed for request {i}"
            );
        } else {
            // Git status may fail but must still return a JSON object
            assert!(
                response.contains('{') && response.contains('}'),
                "Invalid JSON response for request {i}"
            );
        }
    }

    Ok(())
}

/// Test error handling for socket binding failures
#[tokio::test]
async fn test_socket_binding_errors() -> Result<()> {
    let temp_dir = tempdir()?;

    // Bind inside a read-only directory
    let readonly_dir = temp_dir.path().join("readonly");
    std::fs::create_dir_all(&readonly_dir)?;

    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&readonly_dir)?.permissions();
        perms.set_mode(0o444);
        std::fs::set_permissions(&readonly_dir, perms)?;

        let mut server = endpoint_builder(readonly_dir.join("test.sock"))
            .build()
            .expect("server build");

        // Building succeeds; start must fail (or never get going) in a read-only directory
        let start_result = timeout(Duration::from_secs(2), server.start()).await;
        assert!(
            start_result.is_err() || start_result.unwrap().is_err(),
            "Server should fail to start in read-only directory"
        );

        // Restore permissions so the tempdir can be removed
        let mut restore_perms = std::fs::metadata(&readonly_dir)?.permissions();
        restore_perms.set_mode(0o755);
        std::fs::set_permissions(&readonly_dir, restore_perms)?;
    }

    // Bind over an existing regular file
    let file_path = temp_dir.path().join("existing-file");
    std::fs::write(&file_path, "not a socket")?;

    let mut server = endpoint_builder(file_path).build().expect("server build");

    let start_result = timeout(Duration::from_secs(2), server.start()).await;
    assert!(
        start_result.is_err() || start_result.unwrap().is_err(),
        "Server should fail when file already exists"
    );

    Ok(())
}

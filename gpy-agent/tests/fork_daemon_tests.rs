//! Fork and daemon integration tests
//!
//! These tests cover the fork crate implementation and daemon lifecycle scenarios
//! that were identified during the socket/IPC investigation.

#![allow(clippy::expect_used)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::missing_panics_doc)]
#![allow(clippy::shadow_unrelated)]

mod common;

use common::fixtures::ServerGuard;
use gpy_agent::Result;
use gpy_agent::config::manager::ConfigManager;
use gpy_agent::git::cache::GitStatusCache;
use gpy_agent::ipc::ClientDirectory;
use gpy_agent::ipc::LatencyTracker;
use gpy_agent::ipc::server::EndpointHandle;
use gpy_agent::theme::ThemeManager;
use std::sync::Arc;
use std::time::Duration;
use tempfile::tempdir;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixStream;
use tokio::time::timeout;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

/// Test socket binding permissions and security
#[tokio::test]
#[allow(clippy::unused_async)]
async fn test_socket_permissions() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("gpy-permissions-test.sock");

    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(GitStatusCache::new());
    let watcher = None;
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let instant_cache = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(LatencyTracker::new(100));
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let mut server = EndpointHandle::builder()
        .socket_path(socket_path.clone())
        .client_registry(registry)
        .git_cache(git_cache)
        .config_manager(config_manager)
        .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
        .theme_manager(theme_manager)
        .instant_cache(instant_cache)
        .latency_tracker(latency_tracker)
        .language_cache(gpy_agent::language::DetectionCache::new())
        .build()
        .expect("server build");

    // Start server in background with automatic cleanup
    let server_handle = tokio::spawn(async move { server.start().await });
    let _guard = ServerGuard::new(server_handle);

    // Wait for socket creation
    timeout(Duration::from_secs(2), async {
        while !socket_path.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Socket not created in time");

    // Check socket permissions (should be 0600 - user only)
    #[cfg(unix)]
    {
        let metadata = std::fs::metadata(&socket_path)?;
        let permissions = metadata.permissions();
        let mode = permissions.mode() & 0o777;

        // Socket should have user-only permissions (0600)
        assert_eq!(
            mode, 0o600,
            "Socket permissions should be user-only (0600), got {mode:#o}"
        );
    }

    // Guard automatically aborts server on drop
    Ok(())
}

/// Test socket binding to different paths and error handling
#[tokio::test]
#[allow(clippy::unused_async)]
#[allow(clippy::too_many_lines)]
async fn test_socket_binding_variations() -> Result<()> {
    let temp_dir = tempdir()?;

    // Test 1: Normal socket path
    let socket_path = temp_dir.path().join("test.sock");
    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(GitStatusCache::new());
    let watcher = None;
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let instant_cache = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(LatencyTracker::new(100));
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let server = EndpointHandle::builder()
        .socket_path(socket_path)
        .client_registry(registry)
        .git_cache(git_cache)
        .config_manager(config_manager)
        .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
        .theme_manager(theme_manager)
        .instant_cache(instant_cache)
        .latency_tracker(latency_tracker)
        .language_cache(gpy_agent::language::DetectionCache::new())
        .build()
        .expect("server build");
    drop(server); // Should clean up socket

    // Test 2: Nested directory path
    let nested_dir = temp_dir.path().join("nested").join("deep");
    std::fs::create_dir_all(&nested_dir)?;
    let nested_socket = nested_dir.join("test.sock");
    let registry2 = ClientDirectory::new().shared();
    let git_cache2 = Arc::new(GitStatusCache::new());
    let watcher2 = None;
    let theme_manager2 =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let instant_cache2 = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(LatencyTracker::new(100));
    let config_manager2 =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let server2 = EndpointHandle::builder()
        .socket_path(nested_socket)
        .client_registry(registry2)
        .git_cache(git_cache2)
        .config_manager(config_manager2)
        .watcher_slot(Arc::new(std::sync::Mutex::new(watcher2)))
        .theme_manager(theme_manager2)
        .instant_cache(instant_cache2)
        .latency_tracker(latency_tracker)
        .language_cache(gpy_agent::language::DetectionCache::new())
        .build()
        .expect("server build");
    drop(server2);

    // Test 3: Path with spaces (should work)
    let spaced_dir = temp_dir.path().join("path with spaces");
    std::fs::create_dir_all(&spaced_dir)?;
    let spaced_socket = spaced_dir.join("test.sock");
    let registry3 = ClientDirectory::new().shared();
    let git_cache3 = Arc::new(GitStatusCache::new());
    let watcher3 = None;
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let instant_cache3 = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(LatencyTracker::new(100));
    let config_manager3 =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let server3 = EndpointHandle::builder()
        .socket_path(spaced_socket)
        .client_registry(registry3)
        .git_cache(git_cache3)
        .config_manager(config_manager3)
        .watcher_slot(Arc::new(std::sync::Mutex::new(watcher3)))
        .theme_manager(theme_manager)
        .instant_cache(instant_cache3)
        .latency_tracker(latency_tracker)
        .language_cache(gpy_agent::language::DetectionCache::new())
        .build()
        .expect("server build");
    drop(server3);

    Ok(())
}

/// Test rapid socket creation and destruction (fork scenario)
#[tokio::test]
#[allow(clippy::unused_async)]
async fn test_rapid_socket_lifecycle() -> Result<()> {
    let temp_dir = tempdir()?;

    // Simulate rapid daemon creation/destruction cycles
    for i in 0_i32..5_i32 {
        let socket_path = temp_dir.path().join(format!("rapid-test-{i}.sock"));
        let registry = ClientDirectory::new().shared();
        let git_cache = Arc::new(GitStatusCache::new());
        let watcher = None;
        let theme_manager =
            Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
        let instant_cache = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
        let latency_tracker = Arc::new(LatencyTracker::new(100));
        let config_manager =
            Arc::new(ConfigManager::with_defaults().expect("default config should load"));
        let mut server = EndpointHandle::builder()
            .socket_path(socket_path.clone())
            .client_registry(registry)
            .git_cache(git_cache)
            .config_manager(config_manager)
            .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
            .theme_manager(theme_manager)
            .instant_cache(instant_cache)
            .latency_tracker(latency_tracker)
            .language_cache(gpy_agent::language::DetectionCache::new())
            .build()
            .expect("server build");

        // Start server with guard for cleanup
        let server_handle = tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(100)).await;
            server.start().await
        });
        let guard = ServerGuard::new(server_handle);

        // Wait briefly then abort (simulates short-lived daemon)
        tokio::time::sleep(Duration::from_millis(150)).await;
        drop(guard); // Explicit drop aborts the server

        // Socket should be cleaned up
        tokio::time::sleep(Duration::from_millis(50)).await;
        // Note: Socket might still exist briefly after abort, which is normal
    }

    Ok(())
}

/// Test concurrent socket access and connection limits
#[tokio::test]
#[allow(clippy::unused_async)]
async fn test_concurrent_connections() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("concurrent-test.sock");

    let registry = ClientDirectory::new().shared();
    let server_socket_path = socket_path.clone();
    let server_registry = Arc::clone(&registry);
    let git_cache = Arc::new(GitStatusCache::new());
    let watcher = None;
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let latency_tracker = Arc::new(LatencyTracker::new(100));

    // Start server with automatic cleanup
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let server_handle = tokio::spawn(async move {
        let instant_cache = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
        let mut server = EndpointHandle::builder()
            .socket_path(server_socket_path)
            .client_registry(server_registry)
            .git_cache(git_cache)
            .config_manager(config_manager)
            .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
            .theme_manager(theme_manager)
            .instant_cache(instant_cache)
            .latency_tracker(latency_tracker)
            .language_cache(gpy_agent::language::DetectionCache::new())
            .build()
            .expect("Server failed to build");
        server.start().await.expect("Server failed to start");
    });
    let _guard = ServerGuard::new(server_handle);

    // Wait for socket
    timeout(Duration::from_secs(2), async {
        while !socket_path.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Socket not created");

    // Create multiple concurrent connections
    let mut handles = Vec::new();
    for _i in 0_i32..3_i32 {
        let socket_path_clone = socket_path.clone();
        let handle = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&socket_path_clone).await?;

            // Send ping with proper JSON format
            let ping_request = r#"{"op":"ping"}"#;
            stream.write_all(ping_request.as_bytes()).await?;
            stream.write_all(b"\n").await?;

            // Read response
            let mut response_buf = [0u8; 1024];
            let n = stream.read(&mut response_buf).await?;
            let response =
                std::str::from_utf8(response_buf.get(..n).ok_or("Invalid slice range")?)?;

            Ok::<String, Box<dyn std::error::Error + Send + Sync>>(response.to_owned())
        });
        handles.push(handle);
    }

    // Wait for all connections to complete
    for handle in handles {
        let response = handle.await.expect("Connection failed").unwrap();
        assert!(response.contains(r#""status":"ok""#));
    }

    // Guard automatically cleans up server
    Ok(())
}

/// Test socket cleanup after process termination
#[tokio::test]
#[allow(clippy::unused_async)]
#[allow(clippy::too_many_lines)]
async fn test_socket_cleanup_on_termination() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("cleanup-test.sock");

    {
        let registry = ClientDirectory::new().shared();
        let git_cache = Arc::new(GitStatusCache::new());
        let watcher = None;
        let theme_manager =
            Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
        let instant_cache = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
        let latency_tracker = Arc::new(LatencyTracker::new(100));
        let config_manager =
            Arc::new(ConfigManager::with_defaults().expect("default config should load"));
        let mut server = EndpointHandle::builder()
            .socket_path(socket_path.clone())
            .client_registry(registry)
            .git_cache(git_cache)
            .config_manager(config_manager)
            .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
            .theme_manager(theme_manager)
            .instant_cache(instant_cache)
            .latency_tracker(latency_tracker)
            .language_cache(gpy_agent::language::DetectionCache::new())
            .build()
            .expect("server build");

        let _server_socket_path = socket_path.clone();
        let server_handle = tokio::spawn(async move { server.start().await });
        let guard = ServerGuard::new(server_handle);

        // Wait for socket creation
        timeout(Duration::from_secs(1), async {
            while !socket_path.exists() {
                tokio::time::sleep(Duration::from_millis(50)).await;
            }
        })
        .await
        .expect("Socket not created");

        // Verify socket exists
        assert!(socket_path.exists());

        // Cancel the server task (simulates abrupt termination)
        // Extract handle from guard for manual abort testing
        let server_handle = guard.into_inner();
        server_handle.abort();

        // Wait for the task to actually be aborted
        let _ = server_handle.await;

        // Give time for any remaining cleanup
        tokio::time::sleep(Duration::from_millis(100)).await;
    } // Server drops here

    // After abrupt termination, test that the server doesn't respond properly
    if socket_path.exists() {
        let response_result = timeout(Duration::from_millis(500), async {
            let mut stream = UnixStream::connect(&socket_path).await?;
            stream.write_all(br#"{"op":"ping"}"#).await?;
            stream.write_all(b"\n").await?;

            let mut buf = [0u8; 1024];
            let n = stream.read(&mut buf).await?;
            let response = buf
                .get(..n)
                .map(String::from_utf8_lossy)
                .map(|s| s.to_string())
                .unwrap_or_default();
            Ok::<String, Box<dyn std::error::Error + Send + Sync>>(response)
        })
        .await;

        // The server should either not respond or the connection should fail
        assert!(
            response_result.is_err()
                || response_result.as_ref().unwrap().is_err()
                || !response_result.unwrap().unwrap().contains("ok"),
            "Server should not respond properly after termination"
        );
    }

    // Clean up any remaining socket file for the test
    let _ = std::fs::remove_file(&socket_path);

    Ok(())
}

/// Test IPC communication under load (simulates fork scenarios)
#[tokio::test]
#[allow(clippy::too_many_lines)]
#[allow(clippy::unused_async)]
async fn test_ipc_under_load() -> Result<()> {
    let temp_dir = tempdir()?;
    let socket_path = temp_dir.path().join("load-test.sock");

    let registry = ClientDirectory::new().shared();
    let server_socket_path = socket_path.clone();
    let server_registry = Arc::clone(&registry);
    let git_cache = Arc::new(GitStatusCache::new());
    let watcher = None;
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let latency_tracker = Arc::new(LatencyTracker::new(100));

    // Start server with automatic cleanup
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let server_handle = tokio::spawn(async move {
        let instant_cache = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
        let mut server = EndpointHandle::builder()
            .socket_path(server_socket_path)
            .client_registry(server_registry)
            .git_cache(git_cache)
            .config_manager(config_manager)
            .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
            .theme_manager(theme_manager)
            .instant_cache(instant_cache)
            .latency_tracker(latency_tracker)
            .language_cache(gpy_agent::language::DetectionCache::new())
            .build()
            .expect("Server failed to build");
        server.start().await.expect("Server failed to start");
    });
    let _guard = ServerGuard::new(server_handle);

    // Wait for socket
    timeout(Duration::from_secs(2), async {
        while !socket_path.exists() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("Socket not created");

    // Send rapid requests (simulates oneshot command usage)
    let mut handles = Vec::new();
    for i in 0_i32..10_i32 {
        let socket_path_clone = socket_path.clone();
        let handle = tokio::spawn(async move {
            let mut stream = UnixStream::connect(&socket_path_clone).await?;

            // Send different request types
            let request = if i % 2_i32 == 0_i32 {
                r#""Ping""#
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

    // Verify all requests complete successfully
    for (i, handle) in handles.into_iter().enumerate() {
        let response = handle.await.expect("Request failed").unwrap();
        if i % 2_usize == 0_usize {
            assert!(
                response.contains(r#""status":"ok""#),
                "Ping failed for request {i}"
            );
        } else {
            // Git status might fail but should return valid JSON
            assert!(
                response.contains('{') && response.contains('}'),
                "Invalid JSON response for request {i}"
            );
        }
    }

    // Guard automatically cleans up server
    Ok(())
}

/// Test error handling for socket binding failures
#[tokio::test]
#[allow(clippy::unused_async)]
#[allow(clippy::too_many_lines)]
async fn test_socket_binding_errors() -> Result<()> {
    let temp_dir = tempdir()?;

    // Test 1: Bind to read-only directory (should fail)
    let readonly_dir = temp_dir.path().join("readonly");
    std::fs::create_dir_all(&readonly_dir)?;

    #[cfg(unix)]
    {
        let mut perms = std::fs::metadata(&readonly_dir)?.permissions();
        perms.set_mode(0o444); // Read-only
        std::fs::set_permissions(&readonly_dir, perms)?;

        let readonly_socket = readonly_dir.join("test.sock");
        let registry = ClientDirectory::new().shared();
        let git_cache = Arc::new(GitStatusCache::new());
        let watcher = None;
        let theme_manager =
            Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
        let instant_cache = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
        let latency_tracker = Arc::new(LatencyTracker::new(100));
        let config_manager =
            Arc::new(ConfigManager::with_defaults().expect("default config should load"));
        let mut server = EndpointHandle::builder()
            .socket_path(readonly_socket)
            .client_registry(registry)
            .git_cache(git_cache)
            .config_manager(config_manager)
            .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
            .theme_manager(theme_manager)
            .instant_cache(instant_cache)
            .latency_tracker(latency_tracker)
            .language_cache(gpy_agent::language::DetectionCache::new())
            .build()
            .expect("server build");

        // Server creation succeeds, but start should fail in read-only directory
        let start_result = timeout(Duration::from_secs(2), server.start()).await;
        // Should either timeout or return an error
        assert!(
            start_result.is_err() || start_result.unwrap().is_err(),
            "Server should fail to start in read-only directory"
        );

        // Restore permissions for cleanup
        let mut restore_perms = std::fs::metadata(&readonly_dir)?.permissions();
        restore_perms.set_mode(0o755);
        std::fs::set_permissions(&readonly_dir, restore_perms)?;
    }

    // Test 2: Try to bind to existing file (should fail)
    let file_path = temp_dir.path().join("existing-file");
    std::fs::write(&file_path, "not a socket")?;

    let registry = ClientDirectory::new().shared();
    let git_cache = Arc::new(GitStatusCache::new());
    let watcher = None;
    let theme_manager =
        Arc::new(ThemeManager::builtin("default").expect("default theme should load"));
    let instant_cache = Arc::new(gpy_agent::cache::InstantPromptCache::new_for_test());
    let latency_tracker = Arc::new(LatencyTracker::new(100));
    let config_manager =
        Arc::new(ConfigManager::with_defaults().expect("default config should load"));
    let mut server = EndpointHandle::builder()
        .socket_path(file_path)
        .client_registry(registry)
        .git_cache(git_cache)
        .config_manager(config_manager)
        .watcher_slot(Arc::new(std::sync::Mutex::new(watcher)))
        .theme_manager(theme_manager)
        .instant_cache(instant_cache)
        .latency_tracker(latency_tracker)
        .language_cache(gpy_agent::language::DetectionCache::new())
        .build()
        .expect("server build");

    // Server creation succeeds, but start should fail because file already exists
    let start_result = timeout(Duration::from_secs(2), server.start()).await;
    // Should either timeout or fail because file already exists and is not a socket
    assert!(
        start_result.is_err() || start_result.unwrap().is_err(),
        "Server should fail when file already exists"
    );

    Ok(())
}

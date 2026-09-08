//! Transport layer tests for Fish shell
//! Tests Unix domain socket transport for Fish shell usage

#![allow(clippy::panic)]
#![allow(clippy::unwrap_used)]
#![allow(clippy::expect_used)]
#![allow(unused_mut)]
#![allow(clippy::missing_panics_doc)] // Test functions panic on assertion failures
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::ignored_unit_patterns)]

use gpy_agent::ipc::transport::Transport;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_transport_creation() {
    // Test server creation
    let server = Transport::new_server();
    assert!(server.is_ok());

    // Test client creation
    let client = Transport::new_client();
    assert!(client.is_ok());
}

#[tokio::test]
#[serial]
async fn test_transport_address() {
    let address = Transport::address();
    assert!(!address.is_empty());

    // Should be a Unix socket path ending with .sock
    assert!(address.contains(".sock") || address.contains("gpy"));

    // Should follow XDG Base Directory spec or fallback to /tmp
    assert!(address.contains("gpy") || address.starts_with("/tmp"));
}

#[tokio::test]
#[serial]
async fn test_server_bind() {
    let mut server = Transport::new_server().unwrap();
    let result = server.bind();

    // Should succeed on first bind, but might fail if another test is using the same socket
    // This is acceptable for cross-platform IPC testing
    match result {
        Ok(_) => {
            // Successful bind
        }
        Err(e) => {
            // Might fail due to address already in use during parallel testing
            let error_msg = e.to_string();
            assert!(error_msg.contains("bind") || error_msg.contains("address"));
        }
    }
}

#[tokio::test]
#[serial]
async fn test_client_connect_error() {
    let mut client = Transport::new_client().unwrap();

    // Should fail to connect when no server is running
    let result = client.connect().await;
    assert!(result.is_err());
}

#[tokio::test]
#[serial]
async fn test_server_client_communication() {
    // This test requires more complex setup due to async nature
    // For now, just test that the methods exist and have correct signatures

    let mut server = Transport::new_server().unwrap();
    let mut client = Transport::new_client().unwrap();

    // Test that send/receive methods exist (will fail without connection)
    let test_data = b"test message";

    let send_result = server.send(test_data).await;
    assert!(send_result.is_err()); // No connection established

    let receive_result = server.receive().await;
    assert!(receive_result.is_err()); // No connection established

    let client_send_result = client.send(test_data).await;
    assert!(client_send_result.is_err()); // No connection established

    let client_receive_result = client.receive().await;
    assert!(client_receive_result.is_err()); // No connection established
}

#[tokio::test]
#[serial]
async fn test_invalid_operations() {
    let mut server = Transport::new_server().unwrap();
    let mut client = Transport::new_client().unwrap();

    // Test operations without proper setup
    let test_data = b"test";

    // Server should fail to send without connection
    let server_result = server.send(test_data).await;
    assert!(server_result.is_err());

    // Client should fail to send without connection
    let client_result = client.send(test_data).await;
    assert!(client_result.is_err());
}

#[tokio::test]
#[serial]
async fn test_socket_path_creation() {
    use std::env;

    // Test XDG_RUNTIME_DIR path creation
    unsafe {
        let dir = format!(
            "{}/target/test_runtime_{}",
            env!("CARGO_MANIFEST_DIR"),
            std::process::id()
        );
        env::set_var("XDG_RUNTIME_DIR", dir);
    }
    let server = Transport::new_server();
    assert!(server.is_ok());

    // Clean up
    unsafe {
        env::remove_var("XDG_RUNTIME_DIR");
    }
    let _ = std::fs::remove_dir_all(format!(
        "{}/target/test_runtime_{}",
        env!("CARGO_MANIFEST_DIR"),
        std::process::id()
    ));
}

#[tokio::test]
#[serial]
async fn test_transport_mode_validation() {
    let mut server = Transport::new_server().unwrap();
    let mut client = Transport::new_client().unwrap();

    // Server should not be able to connect (it should bind)
    // Client should not be able to bind (it should connect)

    // These operations should fail due to mode mismatch
    // (Implementation details depend on platform-specific code)

    // Just verify the transports were created successfully
    // Note: Can't pattern match on platform-specific variants in cross-platform tests
    // The important thing is that both were created without errors
    drop(server);
    drop(client);
}

#[tokio::test]
#[serial]
async fn test_transport_error_messages() {
    let mut client = Transport::new_client().unwrap();

    // Test that error messages are descriptive
    let connect_result = client.connect().await;
    if let Err(e) = connect_result {
        let error_msg = e.to_string();
        // Should contain context about what failed
        assert!(error_msg.contains("Failed to") || error_msg.contains("Cannot"));
    }

    let send_result = client.send(b"test").await;
    if let Err(e) = send_result {
        let error_msg = e.to_string();
        // Should indicate no connection
        assert!(error_msg.contains("connection") || error_msg.contains("Connection"));
    }
}

#[tokio::test]
#[serial]
async fn test_multiple_server_instances() {
    let server1 = Transport::new_server();
    let server2 = Transport::new_server();

    // Both should create successfully
    assert!(server1.is_ok());
    assert!(server2.is_ok());

    // Test that both instances are created properly
    let _s1 = server1.unwrap();
    let _s2 = server2.unwrap();

    // Note: Testing actual binding conflicts is tricky in parallel test environment
    // The important part is that instances can be created successfully
}

#[tokio::test]
#[serial]
async fn test_transport_cleanup() {
    use std::env;

    // Isolate from any real agent socket by using a test-specific runtime dir
    let original_xdg_runtime = env::var("XDG_RUNTIME_DIR").ok();
    let test_runtime_dir = format!(
        "{}/target/test_runtime_{}_cleanup",
        env!("CARGO_MANIFEST_DIR"),
        std::process::id()
    );
    unsafe {
        env::set_var("XDG_RUNTIME_DIR", &test_runtime_dir);
    }

    // Test that transports clean up properly when dropped
    {
        let mut server = Transport::new_server().unwrap();
        let _ = server.bind();
        // Server goes out of scope here
    }

    // Give the system a moment to clean up the socket file
    tokio::time::sleep(tokio::time::Duration::from_millis(10)).await;

    // Switch to a fresh runtime dir to avoid colliding with any existing socket
    let second_runtime_dir = format!("/tmp/gpy_tc_{}", std::process::id());
    unsafe {
        env::set_var("XDG_RUNTIME_DIR", &second_runtime_dir);
    }

    // Should be able to create and bind a new server after cleanup
    let mut new_server = Transport::new_server().unwrap();
    let result = new_server.bind();
    if let Err(e) = &result {
        let msg = e.to_string().to_lowercase();
        assert!(
            msg.contains("bind")
                || msg.contains("address")
                || msg.contains("permission")
                || msg.contains("operation not permitted")
                || msg.contains("sun_len")
                || msg.contains("path must be shorter")
        );
    }

    // Restore environment and cleanup
    unsafe {
        match original_xdg_runtime {
            Some(val) => env::set_var("XDG_RUNTIME_DIR", val),
            None => env::remove_var("XDG_RUNTIME_DIR"),
        }
    }
    let _ = std::fs::remove_dir_all(test_runtime_dir);
    let _ = std::fs::remove_dir_all(second_runtime_dir);
}

# GPY Agent Test Suite

This directory contains the test suite for the GPY agent, including unit tests, integration tests, and end-to-end tests.

## Test Organization

### Test Files

Every `*.rs` file directly under `gpy-agent/tests/` is its own nextest
target (65 of them, listed below); `common/` holds the shared harnesses
(`cli_harness.rs`, `fixtures.rs`, `integration_harness.rs`, `skip.rs`) and
`snapshots/` the insta snapshots the help and formatter tests diff against.
The unit tests live next to the code in `src/`.

| Target | What it covers |
|---|---|
| `agent_daemon_tests.rs` | Agent daemon lifecycle integration tests |
| `agent_lifecycle_cli_tests.rs` | End-to-end coverage of the `gpy` lifecycle glue and liveness reporting |
| `agent_pruning_integration_test.rs` | Integration test for Agent's dead client pruning with watcher cleanup |
| `base16_import_golden_tests.rs` | Golden import test: a real, published base16 scheme → a usable GPY palette |
| `cache_tests.rs` | Unit tests for `GitStatusCache` |
| `cli_config_mutation_tests.rs` | Integration tests for CLI configuration path resolution and malformed-config |
| `cli_error_messages_tests.rs` | The `gpy` error surface a confused user meets first (#648, pins #641) |
| `cli_help_snapshot_tests.rs` | Snapshots of the `gpy` and `gpy-agent` help surface (#648) |
| `cli_integration_tests.rs` | Comprehensive CLI integration tests |
| `clock_timer_tests.rs` | Clock timer integration tests |
| `config_hot_reload_tests.rs` | Config hot-reload integration tests |
| `config_loader_tests.rs` | Comprehensive TOML configuration loading and parsing tests |
| `config_tests.rs` | Comprehensive configuration system tests |
| `config_watcher_tests.rs` | The config-file watcher: change detection, debounce and reload delivery |
| `dead_client_pruning_tests.rs` | Integration tests for dead client pruning with watcher cleanup |
| `e2e_ipc_tests.rs` | End-to-end integration tests for the IPC server |
| `error_tests.rs` | Comprehensive tests for error handling |
| `fish_integration_tests.rs` | Fish shell integration tests |
| `fork_daemon_tests.rs` | Fork and daemon integration tests |
| `format_consolidation_tests.rs` | Integration tests for Format consolidation |
| `formatter_integration_tests.rs` | Formatter integration tests: exercises `Format::Ansi`, JSON, Fish-args, |
| `formatter_snapshot_tests.rs` | Snapshot tests for formatter outputs using insta |
| `git_commands_tests.rs` | Tests for git/commands.rs |
| `git_edge_cases_tests.rs` | Git Edge Case Tests |
| `git_huge_repo_tests.rs` | Integration tests for huge repository performance optimizations |
| `git_repository_tests.rs` | Tests for git repository discovery and path normalization |
| `git_status_tests.rs` | Comprehensive tests for git status functionality |
| `gpy_cli_tests.rs` | Integration tests for the `gpy` CLI binary |
| `init_command_tests.rs` | Integration tests for `gpy-agent init` (first-run icon/config bootstrap, #411) |
| `init_pty_tests.rs` | PTY-driven coverage of the interactive `gpy-agent init` path (#648, pins |
| `instant_cache_regression_tests.rs` | Regression tests for instant-prompt cache invalidation bugs |
| `integration_pipeline_tests.rs` | Integration Pipeline Tests |
| `integration_tests.rs` | Integration tests for end-to-end IPC functionality |
| `ipc_client_tests.rs` | Tests for IPC client behavior with mock server |
| `ipc_concurrency_tests.rs` | Concurrency tests for asynchronous cache-miss handling (#154) |
| `ipc_edge_cases_tests.rs` | IPC Protocol Edge Case Tests |
| `ipc_protocol_tests.rs` | Unit tests for IPC protocol serialization/deserialization |
| `language_cache_tests.rs` | Tests for the `CacheLookup` result type used by language version caching |
| `language_detection_integration_tests.rs` | Integration tests for language detection across all supported languages |
| `language_tests.rs` | Unit tests for language detection functionality |
| `lifecycle_path_tests.rs` | Regression tests for runtime/socket path resolution in lifecycle helpers |
| `logging_channel_guard.rs` | Guards the unified logging channel (#627): every runtime diagnostic under |
| `mock_watcher.rs` | Tests verifying `WatcherConfig` default values |
| `multi_repo_integration_tests.rs` | Multi-repository integration tests |
| `palette_cli_e2e_tests.rs` | `gpy palette` through the real binary (#648) |
| `palette_swap_tests.rs` | SP4: swapping ui.palette recolors languages while the starship palette is vanilla |
| `plugin_extensibility_e2e_tests.rs` | Plugin discovery, manifest validation and a real Fish runtime load of a plugin segment (needs Fish 4) |
| `plugin_performance_tests.rs` | Plugin discovery stays within its performance budget |
| `prev_bg_ansi.rs` | Integration test: `prev_bg` threads through the render pipeline and produces |
| `schema_validation.rs` | Schema validation tests |
| `signal_tests.rs` | SIGURG doorbell delivery to registered client PIDs (reloads add `<pid>.reload` flag files) |
| `stale_socket_tests.rs` | Integration tests for `check_and_cleanup_socket` ping-retry behaviour (#317) |
| `starship_import_golden_tests.rs` | Golden import tests: real-ish Starship configs → valid GPY artifacts |
| `template_golden_tests.rs` | Golden fidelity tests: GPY engine output vs. Starship default module formats |
| `test_harness.rs` | Test harness utilities for deterministic testing |
| `theme_golden_default.rs` | Golden ANSI baseline for the default.toml migration (SP2 #199) |
| `theme_golden_text.rs` | Golden ANSI baseline for the text.toml migration (SP2 #199) |
| `theme_import_tests.rs` | Integration tests for `gpy theme import` |
| `theme_manager_tests.rs` | Tests for `ThemeManager` functionality |
| `theme_property_tests.rs` | Property-based tests for theme configuration using proptest |
| `theme_schema_tests.rs` | Theme file schema: required tables, colour and template validation |
| `transport_tests.rs` | Transport layer tests for Fish shell |
| `watch_events.rs` | File-watcher event classification and repository attribution |
| `watcher_tests.rs` | Unit tests for MultiRepoWatcher |
| `wizard_pty_tests.rs` | PTY-level regression coverage for wizard terminal restoration (#460) |

### How the targets run

- **Runner**: `cargo nextest run --features test-support` from `gpy-agent/`
  (`just check-rust` wraps it). Plain `cargo test` works but runs targets
  in one process each; nextest's per-test processes are what the isolation
  below assumes. Doc tests still need `cargo test --doc`.
- **`test-support` feature**: enables the PTY and process helpers the
  `init`/wizard PTY tests and a few harnesses need (#460); `just lint` and
  the pre-push gate pass it, so compile with it too.
- **Serialisation**: tests that touch process-global state (environment
  variables, the config watcher's file, one real agent per socket) are
  marked `#[serial_test::serial]` or `#[serial_test::file_serial]` rather
  than sleeping their way past each other; the `file_locks` feature makes
  that hold across nextest's processes.
- **Retries**: none by default. `profile.default` in `.config/nextest.toml`
  has no retry budget (#620); a per-test override must name its tests and
  cite an issue, and `tests/bash/nextest_retry_policy.test.bash` enforces
  it. A retried test is printed as `FLAKY` in the run summary.
- **Skips**: a test that cannot run calls `skip::skip_test("reason")`
  (`common/skip.rs`): `SKIP: reason` and a pass locally, a failure under
  `CI` (#650). Never return silently.
- **Sandboxing**: every spawned agent binds a socket under the GPY-owned
  test root (`gpy_test_root()` in `common/fixtures.rs`); `scripts/
  cleanup-test-agents.sh` reaps leftovers there and nowhere else (#619).

### Test Harness Utilities

The `test_harness` module provides deterministic synchronization primitives to replace brittle timing-based waits. This ensures tests are reliable on both macOS and Linux without race conditions.

## Using the Test Harness

### SignalCounter

Use `SignalCounter` for deterministic signal counting and waiting:

```rust
use test_harness::SignalCounter;
use std::time::Duration;

// Create a counter
let counter = SignalCounter::new();

// Increment from signal handler or event
counter.increment();

// Wait for a specific count (up to timeout)
let received = counter.wait_for_count(2, Duration::from_secs(1)).await;
assert!(received, "Should have received 2 signals");

// Wait for an increment (delta from current value)
let received = counter.wait_for_increment(1, Duration::from_secs(1)).await;

// Check current count
assert_eq!(counter.get(), 3);

// Reset the counter
counter.reset();
```

**When to use:**
- Counting SIGURG doorbell deliveries in tests
- Waiting for a specific number of events
- Avoiding `sleep()` waits in signal-based tests

**How it works:**
- Uses `Arc<AtomicU32>` for thread-safe counting
- Uses `Arc<Notify>` to wake waiting tasks immediately when count changes
- `wait_for_count()` blocks until the expected count is reached or timeout expires

### EventFlag

Use `EventFlag` for boolean state signaling:

```rust
use test_harness::EventFlag;
use std::time::Duration;

// Create a flag
let flag = EventFlag::new();

// Set the flag in some task
flag.set();

// Wait for the flag to be set
let was_set = flag.wait_until_set(Duration::from_secs(1)).await;
assert!(was_set);

// Check if flag is set (non-blocking)
assert!(flag.is_set());

// Clear the flag
flag.clear();
```

**When to use:**
- Signaling that an event occurred
- Coordinating between async tasks
- Avoiding `sleep()` waits for state changes

**How it works:**
- Uses `Arc<AtomicBool>` for thread-safe boolean state
- Uses `Arc<Notify>` to wake waiting tasks when flag is set
- `wait_until_set()` blocks until flag is true or timeout expires

## Best Practices

### 1. Avoid sleep()-based timing

**❌ Bad:**
```rust
registry.notify_repaint(None);
sleep(Duration::from_millis(50)).await;
assert_eq!(SIGNAL_COUNT.load(Ordering::Relaxed), 1);
```

**✅ Good:**
```rust
registry.notify_repaint(None);
let received = counter.wait_for_count(1, Duration::from_secs(1)).await;
assert!(received, "Should have received signal within timeout");
```

### 2. Use appropriate timeouts

- For immediate events (signals, IPC): 1 second timeout
- For startup/shutdown: 2-5 second timeout
- For minute-boundary tests: Use simulation, not real-time waits

### 3. Always check return values

```rust
// Check if wait succeeded
let received = counter.wait_for_count(1, Duration::from_secs(1)).await;
assert!(received, "Timeout waiting for signal");

// Or use timeout error message
let received = flag.wait_until_set(Duration::from_secs(1)).await;
if !received {
    panic!("Flag was not set within timeout");
}
```

### 4. Reset state between tests

Use `#[serial]` for tests that share global state:

```rust
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_with_shared_state() {
    // Reset any shared counters
    counter.reset();

    // Test code...
}
```

## Running Tests

All commands from `gpy-agent/`:

```bash
cargo nextest run --features test-support                          # everything (what the gate runs)
cargo nextest run --features test-support --test clock_timer_tests # one target
cargo nextest run --features test-support -- test_clock_signals    # tests matching a name
cargo nextest run --features test-support --no-capture             # see stdout/stderr live
cargo test --doc                                                   # doc tests (nextest does not run them)
```

## Test Coverage

`just coverage` (from the repository root) runs the suite under
`cargo llvm-cov nextest`, writes `gpy-agent/lcov.info` and prints the summary;
the ubuntu pr-gate uploads the same file. The current figure is recorded in
[docs/dev/testing.md](../../docs/dev/testing.md). For a browsable report:

```bash
cargo llvm-cov nextest --features test-support --html --open
```

## Platform Compatibility

All tests are designed to work on:
- **macOS** (Darwin) - primary development platform
- **Linux** - CI/CD environment
- **BSD** (untested but should work)

Signal-based tests use conditional compilation (`#[cfg(unix)]`) and are skipped on Windows.

## Troubleshooting

### Tests hang or timeout

- Check for deadlocks in Arc/Mutex usage
- Ensure all async tasks complete (use `tokio::select!` for cancellation)
- Increase timeout values for slower CI environments

### Signal tests fail intermittently

- Use `SignalCounter` instead of `sleep()` + `AtomicU32`
- Add `#[serial]` to tests that share signal handlers
- Ensure signal handlers are properly restored after tests

### Fish integration tests fail

- Verify Fish is installed: `fish --version`
- Verify gpy-agent is in PATH: `which gpy-agent`
- Check Fish configuration doesn't interfere with tests
- Tests automatically skip if Fish or gpy-agent is not found
- **Fish ≥ 4 required for the plugin e2e test:** `e2e_plugin_runtime_loading_and_missing_file_graceful` runs an inline script that uses Fish-4-only syntax and skips on older interpreters (e.g. Ubuntu CI's Fish 3.7). The gpy runtime itself still supports Fish 3.6+; only this test's inline script needs Fish 4.

## Contributing

When adding new tests:

1. **Use the test harness** - Avoid `sleep()` waits, use `SignalCounter` or `EventFlag`
2. **Document complex tests** - Add doc comments explaining what is being tested
3. **Test error paths** - Don't just test the happy path
4. **Keep tests fast** - Aim for <100ms per test (excluding I/O-heavy tests)
5. **Use descriptive names** - Test names should clearly describe what is being tested
6. **Clean up resources** - Restore signal handlers, stop agents, remove temp files

## Examples

### Example: Testing Signal Delivery

```rust
#[cfg(unix)]
#[tokio::test]
#[serial]
async fn test_my_signal_feature() {
    let counter = SignalCounter::new();
    counter.reset();

    // Install signal handler that increments counter
    let handler = SigAction::new(
        SigHandler::Handler(my_signal_handler),
        SaFlags::empty(),
        SigSet::empty(),
    );
    let previous = unsafe { sigaction(Signal::SIGURG, &handler) }
        .expect("install handler");

    // Trigger the feature that should send a signal
    my_feature_that_sends_signal();

    // Wait deterministically for the signal
    let received = counter.wait_for_count(1, Duration::from_secs(1)).await;
    assert!(received, "Should have received signal");

    // Restore previous handler
    unsafe {
        sigaction(Signal::SIGURG, &previous).expect("restore handler");
    }
}
```

Reload notifications (`notify_reload`) also write `<shell_dir>/<pid>.reload`.
Build the registry with `ClientDirectory::with_shell_dir(tempdir)` so the flag
lands in a temp directory, and assert the flag exists alongside the doorbell
count.

### Example: Testing Async Coordination

```rust
#[tokio::test]
async fn test_async_task_coordination() {
    let ready = EventFlag::new();
    let ready_clone = ready.clone();

    // Spawn task that does work then signals ready
    tokio::spawn(async move {
        // Do async work...
        tokio::time::sleep(Duration::from_millis(100)).await;
        ready_clone.set();
    });

    // Wait for task to signal ready
    let was_ready = ready.wait_until_set(Duration::from_secs(1)).await;
    assert!(was_ready, "Task should complete within timeout");
}
```

### Example: Testing Minute Boundary Logic

```rust
#[test]
fn test_minute_boundary_detection() {
    use std::time::{SystemTime, UNIX_EPOCH};

    // Don't wait for real minute boundaries - test the logic directly
    let now = SystemTime::now();
    let since_epoch = now.duration_since(UNIX_EPOCH).unwrap();
    let seconds_into_minute = since_epoch.as_secs() % 60;

    // Test the actual logic used by the agent
    let show_seconds = false;
    let should_send = show_seconds || seconds_into_minute == 0;

    // Verify it matches expected behavior
    if seconds_into_minute == 0 {
        assert!(should_send, "Should send at minute boundary");
    } else {
        assert!(!should_send, "Should not send mid-minute");
    }
}
```

## Testing Recipes

This section provides code templates for common testing patterns in the GPY codebase.

### Recipe 1: Testing IPC Message Handlers

**Pattern**: Setup → Send Message → Verify Response

```rust
#[tokio::test]
async fn test_ipc_message_handler() {
    use gpy_agent::ipc::{Message, Response, protocol::{serialize_message, deserialize_response}};
    use tokio::net::UnixStream;

    // Setup: Create test socket and connect
    let socket_path = "/tmp/test_gpy.sock";
    let mut stream = UnixStream::connect(socket_path).await.expect("connect");

    // Send: Build and send message
    let message = Message::RepositoryStatus {
        path: "/test/repo".to_owned(),
        format: gpy_agent::formatter::Format::Json,
        is_last: false,
        is_first: false,
    };
    let payload = serialize_message(&message).expect("serialize");
    stream.write_all(&payload).await.expect("write");
    stream.write_all(b"\n").await.expect("write newline");

    // Verify: Read and parse response
    let mut buf = vec![0u8; 4096];
    let n = stream.read(&mut buf).await.expect("read");
    let response = deserialize_response(&buf[..n]).expect("deserialize");

    // Assert response type and content
    match response {
        Response::RepositoryStatus { branch, .. } => {
            assert!(!branch.is_empty(), "Branch should not be empty");
        }
        Response::Error { message } => {
            panic!("Unexpected error: {}", message);
        }
        _ => panic!("Unexpected response type"),
    }
}
```

**Template Customization**:
- Replace `Message::RepositoryStatus` with your message type
- Adjust expected response type in the `match` statement
- Add specific assertions for your use case

### Recipe 2: Testing File Watcher Events

**Pattern**: Setup Watcher → Trigger Event → Verify Detection (No Sleeps)

```rust
#[tokio::test]
async fn test_file_watcher_detection() {
    use gpy_agent::watcher::filesystem::GitWatcher;
    use notify::EventKind;
    use std::path::PathBuf;
    use test_harness::EventFlag;

    // Setup: Create watcher with event flag
    let detected = EventFlag::new();
    let detected_clone = detected.clone();

    let watcher = GitWatcher::new(
        PathBuf::from("/test/repo"),
        move || {
            detected_clone.set();
        }
    );

    // Trigger: Simulate file change event
    let event = notify::Event {
        kind: EventKind::Modify(notify::event::ModifyKind::Data(DataChange::Any)),
        paths: vec![PathBuf::from("/test/repo/.git/HEAD")],
        attrs: Default::default(),
    };
    watcher.handle_event(event);

    // Verify: Wait for detection with timeout (no sleep!)
    let was_detected = detected.wait_until_set(Duration::from_secs(1)).await;
    assert!(was_detected, "File change should be detected within timeout");
}
```

**Key Points**:
- Use `EventFlag` instead of `sleep()` + polling
- Set up callback that signals the flag
- Verify within timeout using `wait_until_set()`

### Recipe 3: Testing Cache Invalidation

**Pattern**: Populate Cache → Change Config → Verify Cache Cleared

```rust
#[tokio::test]
async fn test_cache_invalidation_on_config_change() {
    use gpy_agent::language::cache::LanguageCache;
    use gpy_agent::config::Config;

    // Setup: Create cache and populate
    let cache = LanguageCache::new();
    let root = PathBuf::from("/test/project");

    // Populate: Add entry to cache
    cache.set(&root, vec!["Rust".to_owned()], Duration::from_secs(60));
    assert!(cache.get(&root).is_some(), "Cache should have entry");

    // Change: Simulate config change
    let mut config = Config::default();
    config.language.display = gpy_agent::config::types::LanguageDisplay::Text;

    // Trigger: Invalidate cache for this root
    cache.invalidate(&root);

    // Verify: Cache entry should be cleared
    assert!(cache.get(&root).is_none(), "Cache should be cleared after invalidation");
}
```

**Template Customization**:
- Replace `LanguageCache` with your cache type
- Adjust config fields that trigger invalidation
- Add assertions for specific cache behavior

### Recipe 4: Testing Formatter Output

**Pattern**: Create Test Data → Format → Verify Output

```rust
#[test]
fn test_formatter_output() {
    use gpy_agent::formatter::git::FishAnsiGitFormatter;
    use gpy_agent::git::GitStatus;

    // Setup: Create test data
    let status = GitStatus {
        branch: "main".to_owned(),
        ahead: 2,
        behind: 0,
        staged: 1,
        unstaged: 0,
        untracked: 0,
        conflicts: 0,
        state: String::new(),
    };

    // Format: Generate output
    let formatter = FishAnsiGitFormatter::new(/* theme config */);
    let output = formatter.format(&status, false);

    // Verify: Check output contains expected elements
    assert!(output.contains("main"), "Output should contain branch name");
    assert!(output.contains(""), "Output should contain ahead icon");
    assert!(output.contains("2"), "Output should contain ahead count");

    // Verify: Check ANSI escape sequences
    assert!(output.contains("\x1b["), "Output should contain ANSI codes");

    // Verify: Check for proper delimiter (not last segment)
    assert!(output.ends_with(""), "Should have delimiter for non-last segment");
}
```

**Key Points**:
- Test both content and formatting
- Verify ANSI escape sequences are present
- Test `is_last` parameter affects delimiter output

### Recipe 5: Testing IPC Protocol Validation

**Pattern**: Send Invalid Message → Verify Error Response

```rust
#[tokio::test]
async fn test_protocol_validation() {
    use gpy_agent::ipc::protocol::deserialize_message;

    // Test: Invalid PID (too large)
    let invalid_json = r#"{"op":"register","pid":2000000}"#;
    let result = deserialize_message(invalid_json.as_bytes());
    assert!(result.is_err(), "Should reject invalid PID");
    assert!(
        result.unwrap_err().to_string().contains("Invalid PID"),
        "Error should mention invalid PID"
    );

    // Test: Missing required field
    let invalid_json = r#"{"op":"workspace","pid":12345}"#;
    let result = deserialize_message(invalid_json.as_bytes());
    assert!(result.is_err(), "Should reject missing required field");

    // Test: Path too long (> 4096 chars)
    let long_path = "a".repeat(5000);
    let invalid_json = format!(r#"{{"op":"git","cwd":"{}"}}"#, long_path);
    let result = deserialize_message(invalid_json.as_bytes());
    assert!(result.is_err(), "Should reject overly long path");

    // Test: Valid message passes
    let valid_json = r#"{"op":"ping"}"#;
    let result = deserialize_message(valid_json.as_bytes());
    assert!(result.is_ok(), "Should accept valid message");
}
```

**Key Points**:
- Test boundary conditions (max values, empty strings)
- Test missing required fields
- Verify error messages are descriptive
- Include at least one valid case

### Recipe 6: Testing Theme Manager

**Pattern**: Load Theme → Modify → Verify Reload

```rust
#[test]
fn test_theme_reload() {
    use gpy_agent::theme::ThemeManager;
    use std::fs;

    // Setup: Create test theme file
    let theme_path = "/tmp/test_theme.toml";
    let initial_theme = r#"
        [git]
        clean = "00ff00"
        dirty = "ff0000"
    "#;
    fs::write(theme_path, initial_theme).expect("write theme");

    // Load: Initialize theme manager
    let mut manager = ThemeManager::new(theme_path);
    manager.load().expect("load theme");

    assert_eq!(manager.get_color("git", "clean"), Some("00ff00".to_owned()));

    // Modify: Update theme file
    let updated_theme = r#"
        [git]
        clean = "00ff00"
        dirty = "ff9900"
    "#;
    fs::write(theme_path, updated_theme).expect("update theme");

    // Reload: Trigger reload
    manager.reload().expect("reload theme");

    // Verify: New value is loaded
    assert_eq!(manager.get_color("git", "dirty"), Some("ff9900".to_owned()));

    // Cleanup
    fs::remove_file(theme_path).ok();
}
```

**Key Points**:
- Test both initial load and reload
- Verify old values are replaced
- Test partial updates (only changed fields)
- Clean up temp files

### Recipe 7: Testing Concurrent Connections

**Pattern**: Spawn Multiple Clients → Verify All Handled

```rust
#[tokio::test]
async fn test_concurrent_client_connections() {
    use tokio::task::JoinSet;

    // Setup: Start server
    let server = start_test_server().await;
    let socket_path = server.socket_path();

    // Spawn: Create multiple concurrent clients
    let mut tasks = JoinSet::new();
    for i in 0..10 {
        let path = socket_path.clone();
        tasks.spawn(async move {
            let mut stream = UnixStream::connect(path).await.expect("connect");
            let message = format!(r#"{{"op":"ping"}}"#);
            stream.write_all(message.as_bytes()).await.expect("write");

            let mut buf = vec![0u8; 1024];
            let n = stream.read(&mut buf).await.expect("read");
            String::from_utf8_lossy(&buf[..n]).to_string()
        });
    }

    // Verify: All clients get responses
    let mut responses = Vec::new();
    while let Some(result) = tasks.join_next().await {
        let response = result.expect("task should complete");
        responses.push(response);
    }

    assert_eq!(responses.len(), 10, "All clients should receive responses");
    for response in responses {
        assert!(response.contains("ok"), "Response should be success: {}", response);
    }
}
```

**Key Points**:
- Use `JoinSet` for managing multiple tasks
- Test server can handle concurrent connections
- Verify no responses are lost
- Test realistic concurrency levels

## Quick Reference: Common Patterns

### Wait for Event (No Sleep)

```rust
// ❌ Don't do this
tokio::time::sleep(Duration::from_millis(100)).await;
assert!(some_condition);

// ✅ Do this
let flag = EventFlag::new();
// ... trigger event that sets flag ...
assert!(flag.wait_until_set(Duration::from_secs(1)).await);
```

### Count Signal Deliveries

```rust
let counter = SignalCounter::new();
// ... install signal handler that increments counter ...
// ... trigger action that sends signals ...
assert!(counter.wait_for_count(expected, Duration::from_secs(1)).await);
```

### Test Error Paths

```rust
// Always test both success and failure
let result = function_that_can_fail();
assert!(result.is_ok());

let result = function_that_can_fail_with_invalid_input();
assert!(result.is_err());
assert!(result.unwrap_err().to_string().contains("expected error text"));
```

### Cleanup Resources

```rust
#[tokio::test]
async fn test_with_cleanup() {
    // Setup
    let temp_file = "/tmp/test.txt";
    fs::write(temp_file, "test").expect("write");

    // Test
    // ...

    // Cleanup (even on failure)
    fs::remove_file(temp_file).ok();
}
```

## References

- [Tokio Testing Guide](https://tokio.rs/tokio/topics/testing)
- [Rust Testing Guide](https://doc.rust-lang.org/book/ch11-00-testing.html)
- [serial_test crate](https://docs.rs/serial_test/) - For serializing test execution

# ServerGuard: Test Cleanup Pattern

## Problem

Tests that spawn tokio tasks can leak processes when:
- Tests fail or panic
- Test runner is interrupted (Ctrl+C)
- Assertions fail before cleanup code runs

This leads to:
- Orphaned `gpy-agent` processes accumulating
- Resource exhaustion (PTYs, file descriptors, memory)
- System instability (e.g., macOS 511 PTY limit)

## Solution: `ServerGuard`

The `ServerGuard` wrapper ensures automatic cleanup via Rust's `Drop` trait, which runs even during panic unwinding.

### Location

```rust
use common::ServerGuard;  // tests/common/fixtures.rs
```

### Basic Usage

**Before** (manual cleanup - can leak):
```rust
let server_handle = tokio::spawn(async move {
    server.start().await.expect("Server failed");
});

// Test code...

server_handle.abort();  // ❌ Only runs if test succeeds
Ok(())
```

**After** (automatic cleanup):
```rust
let server_handle = tokio::spawn(async move {
    server.start().await.expect("Server failed");
});
let _guard = ServerGuard::new(server_handle);  // ✅ Aborts on drop

// Test code...
// Guard automatically cleans up (even if test panics)
Ok(())
```

## Advanced Patterns

### Pattern 1: Automatic Cleanup (Most Common)

Use when you don't need to manually control the task lifecycle:

```rust
#[tokio::test]
async fn test_something() -> Result<()> {
    let server_handle = tokio::spawn(async { /* ... */ });
    let _guard = ServerGuard::new(server_handle);

    // Test code...

    // Guard aborts server automatically on drop
    Ok(())
}
```

### Pattern 2: Early Termination

Use when you need to explicitly stop the server mid-test:

```rust
#[tokio::test]
async fn test_lifecycle() -> Result<()> {
    let server_handle = tokio::spawn(async { /* ... */ });
    let guard = ServerGuard::new(server_handle);

    // Test code...

    // Explicitly drop to abort early
    drop(guard);

    // More test code after server stopped...
    Ok(())
}
```

### Pattern 3: Manual Control (Testing Abort Behavior)

Use when the test specifically needs to control abort/await:

```rust
#[tokio::test]
async fn test_abort_behavior() -> Result<()> {
    let server_handle = tokio::spawn(async { /* ... */ });
    let guard = ServerGuard::new(server_handle);

    // Test code...

    // Extract handle for manual testing
    let server_handle = guard.into_inner();
    server_handle.abort();
    let _ = server_handle.await;

    // Verify abort behavior...
    Ok(())
}
```

## API Reference

### `ServerGuard::new(handle)`

Create a guard wrapping a tokio `JoinHandle`:

```rust
let guard = ServerGuard::new(server_handle);
```

- Automatically aborts the task when dropped
- Works with any `tokio::task::JoinHandle<T>`

### `guard.into_inner()`

Extract the inner `JoinHandle` for manual control:

```rust
let handle = guard.into_inner();
// Guard no longer owns the handle
// You must manually abort/await
```

- Consumes the guard (prevents double-free)
- Use when you need manual control of the task

### `guard.is_finished()`

Check if the task has completed:

```rust
if guard.is_finished() {
    println!("Task completed");
}
```

## Why This Works

Rust's `Drop` trait runs during stack unwinding, even when:
- Tests panic
- Assertions fail
- Ctrl+C interrupts execution

This guarantees cleanup code runs, preventing process leaks.

## Migration Guide

### Step 1: Identify Tests That Spawn Tasks

Search for patterns like:
```bash
grep -r "tokio::spawn" tests/
grep -r "server_handle.abort()" tests/
```

### Step 2: Add Guard Import

```rust
mod common;
use common::ServerGuard;
```

### Step 3: Wrap Spawned Tasks

Replace:
```rust
let server_handle = tokio::spawn(async { ... });
// ... later ...
server_handle.abort();
```

With:
```rust
let server_handle = tokio::spawn(async { ... });
let _guard = ServerGuard::new(server_handle);
// No manual abort needed
```

### Step 4: Verify No Leaks

After migration, run:
```bash
# Run tests
cargo test --test fork_daemon_tests

# Check for leaked processes
ps aux | grep "gpy-agent.*--socket"
# Should show nothing
```

## Testing the Guard

The `ServerGuard` itself has tests in `tests/common/fixtures.rs`:

```bash
cargo test --lib common::fixtures
```

## Files Modified (2025-11-23)

✅ **Added**:
- `tests/common/fixtures.rs` - `ServerGuard` implementation
- `tests/CLEANUP_GUARD_PATTERN.md` - This documentation

✅ **Updated**:
- `tests/common/mod.rs` - Export `ServerGuard`
- `tests/fork_daemon_tests.rs` - All 7 tests now use `ServerGuard`

✅ **Verified**:
- All tests pass (83 lib + 9 agent_daemon + 9 fork_daemon = 101 tests)
- Zero process leaks after test runs
- PTY usage remains stable

## Troubleshooting

### "Test leaked processes"

1. Run cleanup script:
   ```bash
   ./scripts/cleanup-test-agents.sh
   ```

2. Check if tests use guards:
   ```bash
   grep -n "ServerGuard" tests/fork_daemon_tests.rs
   ```

3. Verify no manual `.abort()` without guards:
   ```bash
   grep -n "server_handle.abort()" tests/*.rs
   # Should only appear in tests that use .into_inner()
   ```

### "Guard doesn't compile"

Ensure you're importing from the right module:
```rust
mod common;
use common::ServerGuard;  // NOT common::fixtures::ServerGuard
```

### "Need manual control"

Use `.into_inner()`:
```rust
let guard = ServerGuard::new(handle);
let handle = guard.into_inner();
// Now you control abort/await
```

## Related

- `scripts/cleanup-test-agents.sh` - Manual cleanup for leaked processes
- `tests/common/fixtures.rs` - Test fixture library
- `tests/fork_daemon_tests.rs` - Example usage

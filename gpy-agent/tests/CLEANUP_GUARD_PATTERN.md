# ServerGuard: aborting spawned server tasks

## Problem

A test that spawns an in-process server with `tokio::spawn` and aborts it at
the end of the test body skips that abort whenever an earlier assertion
fails. The server task then keeps its socket and file descriptors until the
runtime shuts down, and can interfere with the rest of the test.

## `ServerGuard`

`ServerGuard` (`tests/common/fixtures.rs`) owns the task's `JoinHandle` and
calls `.abort()` in `Drop`. `Drop` runs on normal return and while unwinding
from a panic or failed assertion. It does not run when the process is killed
(Ctrl+C, SIGKILL, a nextest timeout); for those,
`scripts/cleanup-test-agents.sh` removes leftover agents and sockets under
the test root.

```rust
mod common;
use common::fixtures::ServerGuard;

let server_handle = tokio::spawn(async move { server.start().await });
let _guard = ServerGuard::new(server_handle); // aborted when `_guard` drops

// Test code; a failing assertion still aborts the server.
```

Bind the guard to a named variable (`_guard`), not `_`: `let _ = ...` drops
it, and aborts the server, immediately.

## Patterns

**Stop the server mid-test**: `drop(guard);` aborts it at that point.

**Control abort/await yourself**: `guard.into_inner()` returns the
`JoinHandle` without aborting; the test is then responsible for it.

```rust
let handle = guard.into_inner();
handle.abort();
assert!(handle.await.expect_err("task was aborted").is_cancelled());
```

## Spawned processes

`tests/common/child_guard.rs` holds the same contract for child processes:
`KillOnDrop` (a `std::process::Child`), `PtyChild` (a `portable_pty` child)
and `AgentStopGuard` (runs `gpy-agent stop --socket <path>` for a daemon that
double-forked and so has no child handle). Construct the guard before the
step that can fail, and bind it to a named variable.

## Related

- `tests/common/endpoint.rs` — `endpoint` and `wait_for_socket` for
  building and starting the in-process server the guard wraps.
- `scripts/cleanup-test-agents.sh` — removes agents and sockets left by
  killed test runs.
- `tests/fork_daemon_tests.rs` — example usage.

# Background Agent Behavior

This document details the operational behavior of the `gpy-agent` background process, focusing on its discoverability, concurrency model, and resource usage during idle periods.

## Discoverability

The `gpy-agent` runs as a background daemon and exposes a Unix domain socket for communication. This socket is the primary discovery mechanism for the Fish shell integration and the `gpy` CLI.

### Socket Location

The agent determines the socket path using the following precedence order (based on the [XDG Base Directory Specification](https://specifications.freedesktop.org/basedir-spec/basedir-spec-latest.html)):

1.  `$GPY_AGENT_SOCKET_PATH` (Environment override)
2.  `$XDG_RUNTIME_DIR/gpy/gpy.sock` (Linux standard)
3.  `$XDG_CACHE_HOME/gpy/gpy.sock`
4.  `~/.cache/gpy/gpy.sock` (macOS/Fallback)
5.  `/tmp/gpy/gpy.sock` (Final fallback)

The `gpy status` command displays the active socket path.

### Process Identification

The agent process is named `gpy-agent`. There is typically one agent instance per user session. The agent uses a lock file (implicitly via the socket binding) to prevent multiple instances from running on the same socket.

## Concurrency Model

The agent uses the `tokio` asynchronous runtime to handle multiple tasks concurrently on a single thread (or thread pool). The main event loop (`Agent::start_background`) uses `tokio::select!` to orchestrate four primary subsystems:

1.  **IPC Server**: Accepts and handles client connections.
2.  **File Watcher**: Monitors filesystem events for Git repositories and config files.
3.  **Clock Timer**: Triggers periodic updates for the prompt clock.
4.  **Pruning Timer**: Periodically cleans up stale client records.

### 1. IPC Server (Idle Behavior)
The server listens on the Unix socket using `UnixListener::accept()`. This is a non-blocking asynchronous operation.
*   **Active**: When a request arrives, a lightweight task is spawned to handle it (deserialize, process, serialize response).
*   **Idle**: When no requests are pending, the listener task is suspended by the runtime, consuming zero CPU.

### 2. File Watcher (Idle Behavior)
The file watcher (powered by the `notify` crate) uses OS-native APIs (FSEvents on macOS, inotify on Linux) to monitor directories.
*   **Active**: When a file change occurs, the OS wakes the agent. The event is debounced and processed.
*   **Idle**: No polling occurs. The watcher sleeps until the OS reports an event.

### 3. Clock Timer (Idle Behavior)
A `tokio::time::interval` timer fires periodically (default: 1 second if seconds are shown, otherwise 1 minute).
*   **Behavior**: On each tick, the agent checks the `ClientDirectory` to see if any clients are registered.
*   **Optimization**: If **no clients are registered** (e.g., no active terminal windows), the timer tick performs a quick check and returns immediately, sending no signals. This minimizes wakeups when the agent is unused.

### 4. Pruning Timer (Idle Behavior)
A separate 60-second timer triggers the client pruning process.
*   **Behavior**: Scans the registered client list and checks if the process IDs (PIDs) are still alive using signal 0 (`kill(pid, 0)`).
*   **Cleanup**: Dead clients are removed from the registry and unregistered from the file watcher to free up resources.

## Resource Usage

The agent is designed to be extremely lightweight when idle:

*   **CPU**: Near zero. The event loop blocks on `epoll`/`kqueue` until IO or a timer event occurs.
*   **Memory**: Uses `Arc` (Atomic Reference Counting) to share large data structures (Git cache, configuration, theme) between components, avoiding duplication. The memory footprint primarily depends on the number of cached Git repositories.
*   **Handles**: Holds one file descriptor for the socket, one for the file watcher (plus internal watcher handles), and temporary handles for active client connections.

## Coordination

All cross-module coordination (e.g., "Git file changed -> Invalidate Cache -> Notify Clients") is handled centrally in the `Agent` module (`src/agent/mod.rs`). This ensures predictable data flow and prevents circular dependencies between subsystems.

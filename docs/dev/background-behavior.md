# Background Agent Behavior

How the `gpy-agent` background process is discovered, how it schedules work, and what it costs while idle.

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

tokio multi-thread runtime; `Agent::start_background`'s `tokio::select!` drives five branches: IPC server, shutdown signal, 1 s clock, 60 s pruning, 45 s reconcile. The file watcher runs on its own OS threads.

1.  **IPC Server**: Accepts and handles client connections.
2.  **Clock Timer**: Triggers periodic updates for the prompt clock.
3.  **Pruning Timer**: Periodically cleans up stale client records.
4.  **Reconcile Timer**: rescans watched repos, signals only on a real diff.

### 1. IPC Server (Idle Behavior)
The server listens on the Unix socket using `UnixListener::accept()`. This is a non-blocking asynchronous operation.
*   **Active**: When a request arrives, a lightweight task is spawned to handle it (deserialize, process, serialize response).
*   **Idle**: When no requests are pending, the listener task is suspended by the runtime, consuming zero CPU.

### 2. Clock Timer (Idle Behavior)
A 1-second `tokio::time::interval` ticks while `agent.live_updates` is on; the agent rings every tick when the clock shows seconds, otherwise once per minute.
*   **Behavior**: On each tick, the agent checks the `ClientDirectory` to see if any clients are registered.
*   **Optimization**: If **no clients are registered** (e.g., no active terminal windows), the timer tick performs a quick check and returns immediately, sending no signals. This minimizes wakeups when the agent is unused.

### 3. Pruning Timer (Idle Behavior)
A separate 60-second timer triggers the client pruning process.
*   **Behavior**: Scans the registered client list and checks if the process IDs (PIDs) are still alive using signal 0 (`kill(pid, 0)`).
*   **Cleanup**: Dead clients are removed from the registry and unregistered from the file watcher to free up resources.

### 4. Reconcile Timer (Idle Behavior)
A 45-second timer (`GPY_RECONCILE_INTERVAL_SECS` overrides it) ticks while `agent.live_updates` is on.
*   **Behavior**: Rescans watched repos to catch changes the OS watcher dropped, and signals shells only on a real diff.

### File Watcher (Idle Behavior)
The file watcher (powered by the `notify` crate) uses OS-native APIs (FSEvents on macOS, inotify on Linux) to monitor directories.
*   **Active**: When a file change occurs, the OS wakes the agent. The event is debounced and processed.
*   **Idle**: No polling occurs. The watcher's event and flush threads block until the OS reports an event; a pending event wakes the flush thread once, at its debounce deadline. Only the poll fallbacks (`GPY_WATCH_FORCE_POLL`, the `PollWatcher` swap after a failed startup health probe, `GPY_CONFIG_WATCH_POLL_MS`/`GPY_THEME_WATCH_POLL_MS`) wake on an interval.

## Resource Usage

The agent is lightweight when idle:

*   **CPU**: Near zero. The event loop blocks on `epoll`/`kqueue` until IO or a timer event occurs, and the file-watcher threads block until an event arrives or a debounce deadline falls due.
*   **Memory**: Uses `Arc` (Atomic Reference Counting) to share large data structures (Git cache, configuration, theme) between components, avoiding duplication. The memory footprint primarily depends on the number of cached Git repositories.
*   **Handles**: Holds one file descriptor for the socket, one for the file watcher (plus internal watcher handles), and temporary handles for active client connections.

## Coordination

All cross-module coordination (e.g., "Git file changed -> Invalidate Cache -> Notify Clients") is handled centrally in the `Agent` module (`src/agent/mod.rs`), which keeps data flow predictable and avoids circular dependencies between subsystems.

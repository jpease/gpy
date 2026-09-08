# Module Reference

> **Archived.** This is historical module documentation. Refer to the current source tree under `gpy-agent/src/` for up-to-date module information.

**Audience**: Contributors who need detailed technical references for GPY's internal modules.

**Prerequisites**: Read [architecture.md](../dev/architecture.md) first for system overview.

This document provides a detailed reference for each major module in the GPY agent codebase, including key types, functions, and common operations.

---

## Table of Contents

- [IPC Subsystem](#ipc-subsystem)
- [Git Subsystem](#git-subsystem)
- [Language Detection](#language-detection)
- [Watcher Subsystem](#watcher-subsystem)
- [Configuration System](#configuration-system)
- [Error Handling](#error-handling)
- [Agent Core](#agent-core)

---

## IPC Subsystem

**Location**: `gpy-agent/src/ipc/`

**Purpose**: Unix domain socket communication between Fish shell and Rust agent.

### Key Modules

#### `ipc/protocol.rs`

Defines the JSON-based IPC message format.

**Key Types**:

```rust
pub struct IpcRequest {
    pub op: Operation,
    pub cwd: Option<PathBuf>,
    pub format: Option<OutputFormat>,
}

pub enum Operation {
    Ping,
    Git,
    Language,
    Shutdown,
    ThemeExport,
}

pub enum OutputFormat {
    Json,
    FishAnsi,      // Pre-rendered ANSI escape codes
    FishSource,    // Fish variable export (set -gx ...)
}

pub struct IpcResponse {
    pub status: ResponseStatus,
    pub data: Option<ResponseData>,
    pub error: Option<String>,
}
```

**Key Functions**:

- `IpcRequest::from_bytes(buf: &[u8]) -> Result<Self>` - Parse JSON message from bytes
- `IpcResponse::to_bytes(&self) -> Result<Vec<u8>>` - Serialize response to JSON bytes

**Common Operations**:

```rust
// Parse incoming request
let request = IpcRequest::from_bytes(&buffer)?;

// Create response
let response = IpcResponse {
    status: ResponseStatus::Ok,
    data: Some(ResponseData::Git(git_info)),
    error: None,
};
```

#### `ipc/server.rs`

Unix socket server implementation using tokio.

**Key Types**:

```rust
pub struct IpcServer {
    socket_path: PathBuf,
    listener: Option<UnixListener>,
}

pub struct EndpointHandle {
    stop_tx: broadcast::Sender<()>,
    join_handle: Option<JoinHandle<Result<()>>>,
}
```

**Key Functions**:

- `IpcServer::new(socket_path: PathBuf) -> Result<Self>` - Create server instance
- `IpcServer::listen(&mut self, agent: Arc<AgentState>) -> Result<EndpointHandle>` - Start accepting connections
- `handle_client(stream: UnixStream, agent: Arc<AgentState>)` - Process single client connection (lines 200-350)

**Common Operations**:

```rust
// Start IPC server
let mut server = IpcServer::new(socket_path)?;
let handle = server.listen(agent_state)?;

// Graceful shutdown
handle.stop().await?;
```

**Security Features** (lines 150-180):

- 64KB message size limit (`MAX_MESSAGE_SIZE`)
- PID validation via `ucred` (macOS/Linux)
- Rate limiting: 50ms minimum interval per client (`ClientRateLimiter`)
- Unix socket permissions: 0600 (user-only)

#### `ipc/client_directory.rs`

Tracks registered Fish shell processes for SIGUSR1 delivery.

**Key Types**:

```rust
pub struct ClientDirectory {
    clients: Arc<Mutex<HashMap<u32, ClientInfo>>>,
}

struct ClientInfo {
    pid: u32,
    cwd: PathBuf,
    last_signal: Instant,
}
```

**Key Functions**:

- `register(pid: u32, cwd: PathBuf)` - Add client to directory
- `unregister(pid: u32)` - Remove client
- `update_cwd(pid: u32, new_cwd: PathBuf)` - Update tracked directory
- `signal_clients_for_repo(repo_root: &Path, throttle_ms: u64)` - Send SIGUSR1 to relevant clients

**Common Operations**:

```rust
// Register new Fish shell
directory.register(12345, Path::new("/home/user/project"))?;

// Send signals on git change
directory.signal_clients_for_repo(Path::new("/home/user/project"), 150)?;
```

**Signal Throttling** (lines 80-120):

- Per-client throttle: Minimum 50ms between signals to same PID
- Per-repository throttle: Minimum 150ms between signals for same repo
- Prevents prompt flicker from rapid file changes

---

## Git Subsystem

**Location**: `gpy-agent/src/git/`

**Purpose**: Fast git status detection using the `gix` library.

### Key Modules

#### `git/commands.rs`

Core git status detection logic.

**Key Types**:

```rust
pub struct GitInfo {
    pub branch: Option<String>,
    pub state: RepoState,
    pub ahead: u32,
    pub behind: u32,
    pub staged: u32,
    pub unstaged: u32,
    pub untracked: u32,
    pub conflicts: u32,
}

pub enum RepoState {
    Clean,
    Merge,
    Rebase,
    CherryPick,
    Revert,
    Bisect,
}
```

**Key Functions**:

- `get_git_status(path: &Path) -> Result<GitInfo>` - Get full git status (lines 50-150)
- `get_branch_name(repo: &gix::Repository) -> Result<Option<String>>` - Extract current branch
- `get_ahead_behind(repo: &gix::Repository) -> Result<(u32, u32)>` - Compare with upstream
- `count_file_changes(repo: &gix::Repository) -> Result<(u32, u32, u32)>` - Staged/unstaged/untracked counts
- `detect_repo_state(repo: &gix::Repository) -> RepoState` - Detect merge/rebase/etc. (lines 250-300)

**Common Operations**:

```rust
// Get git status for directory
let git_info = get_git_status(Path::new("/home/user/project"))?;

// Check if repository is dirty
if git_info.staged > 0 || git_info.unstaged > 0 {
    println!("Repository has uncommitted changes");
}
```

**Performance Optimization** (lines 100-130):

- Uses `gix::open()` which is faster than spawning `git` subprocess
- Skips work tree status check when `--porcelain` would be slow
- Detects repository state from `.git/` file markers (no subprocess)

#### `git/cache.rs`

LRU cache for git status results with TTL expiration.

**Key Types**:

```rust
pub struct GitCache {
    cache: Arc<Mutex<LruCache<PathBuf, CachedGitInfo>>>,
    ttl: Duration,
}

struct CachedGitInfo {
    info: GitInfo,
    cached_at: Instant,
}
```

**Key Functions**:

- `GitCache::new(capacity: usize, ttl: Duration) -> Self` - Create cache with size/TTL limits
- `get(&self, path: &Path) -> Option<GitInfo>` - Retrieve cached entry if not expired
- `insert(&self, path: PathBuf, info: GitInfo)` - Store entry with current timestamp
- `invalidate(&self, path: &Path)` - Force-expire entry
- `clear(&self)` - Remove all entries

**Cache Policy** (lines 40-80):

- **Capacity**: 100 repositories (LRU eviction)
- **TTL**: 60 seconds (default, configurable)
- **Cooldown**: 150ms after watcher events (prevents redundant queries)

**Common Operations**:

```rust
// Try cache first
if let Some(cached) = git_cache.get(path) {
    return Ok(cached);
}

// Cache miss - query git and cache result
let info = get_git_status(path)?;
git_cache.insert(path.to_path_buf(), info.clone());
```

**Cache Invalidation Triggers** (lines 120-150):

1. **TTL expiration**: After 60 seconds, entry is stale
2. **Watcher events**: File changes in `.git/` directory
3. **Explicit invalidation**: `cache.invalidate(path)` call

---

## Language Detection

**Location**: `gpy-agent/src/language/`

**Purpose**: Detect programming language and version for current directory.

### Key Modules

#### `language/detector.rs`

Language detection based on project files.

**Key Types**:

```rust
pub struct LanguageInfo {
    pub name: String,
    pub version: Option<String>,
    pub icon: String,
    pub color: String,
}

pub struct LanguageDetector {
    registry: LanguageRegistry,
    theme: Arc<Theme>,
}
```

**Key Functions**:

- `detect(path: &Path) -> Result<Option<LanguageInfo>>` - Detect language for directory
- `detect_version(lang: &Language, path: &Path) -> Option<String>` - Extract version info

**Detection Strategy** (lines 80-200):

1. **File-based detection**: Look for `package.json`, `Cargo.toml`, `go.mod`, etc.
2. **Priority order**: Check files in priority order (lockfiles > manifests)
3. **Version extraction**: Parse version from file or run version command

**Supported Languages** (lines 50-80):

- **JavaScript/TypeScript**: `package.json`, `node_modules/`
- **Rust**: `Cargo.toml`, `Cargo.lock`
- **Go**: `go.mod`, `go.sum`
- **Python**: `pyproject.toml`, `requirements.txt`, `.venv/`
- **Ruby**: `Gemfile`, `.ruby-version`
- **Elixir**: `mix.exs`, `mix.lock`
- **Swift**: `Package.swift`
- **Java**: `pom.xml`, `build.gradle`

**Common Operations**:

```rust
// Detect language for current directory
let detector = LanguageDetector::new(theme);
if let Some(lang_info) = detector.detect(Path::new("/home/user/project"))? {
    println!("Language: {} {}", lang_info.name, lang_info.version.unwrap_or_default());
}
```

#### `language/registry.rs`

Language definitions and version extraction logic.

**Key Types**:

```rust
pub struct Language {
    pub name: &'static str,
    pub files: &'static [&'static str],
    pub version_file: Option<&'static str>,
    pub version_command: Option<&'static str>,
}
```

**Registry Pattern** (lines 20-100):

- Static registry of language definitions
- Each language specifies detection files and version extraction
- Lazy initialization using `OnceLock`

---

## Watcher Subsystem

**Location**: `gpy-agent/src/watcher/`

**Purpose**: Detect file changes and trigger prompt updates via SIGUSR1.

**Architecture**: See [watcher/mod.rs](../../gpy-agent/src/watcher/mod.rs) module docs for detailed explanation.

### Key Modules

#### `watcher/mod.rs`

Top-level coordination and configuration.

**Key Types**:

```rust
pub enum FileEvent {
    Git { path: PathBuf },
    Language { path: PathBuf },
    Config { path: PathBuf },
}

pub struct WatcherConfig {
    debounce: u64,         // Debounce window (default 100ms)
    throttle: u64,         // Signal throttle (default 150ms)
    cache_cooldown: u64,   // Cache refresh cooldown (default 150ms)
}

pub struct WatchCoordinator {
    watcher: Option<FileSystemWatcher>,
    debouncer: Arc<Mutex<DebounceEngine>>,
    stop_flag: Arc<AtomicBool>,
    flush_handle: Option<JoinHandle<()>>,
}
```

**Key Functions**:

- `WatchCoordinator::new(debounce: Duration, callback: EventCallback) -> Result<Self>` - Create coordinator
- `watch_directory<P: AsRef<Path>>(&mut self, path: P)` - Start watching directory
- `unwatch_directory<P: AsRef<Path>>(&mut self, path: P)` - Stop watching directory
- `should_trigger_update(path: &Path) -> Option<PendingEvent>` - Filter and classify file events

**File Filtering** (lines 252-272):

Ignored paths (don't trigger updates):
- `gpy-agent.log` - Debug log file
- `gpy-agent.sock` - Unix socket
- `.git/objects/` - Object store (too noisy)
- `.git/logs/` - Ref logs
- `.git/hooks/` - Hook scripts

**Event Classification** (lines 274-336):

- **Git events**: `HEAD`, `index`, `MERGE_HEAD`, `REBASE_HEAD`, `.git/refs/`
- **Language events**: `package.json`, `Cargo.toml`, `go.mod`, `pyproject.toml`, etc.
- **Config events**: Files ending with `.gpy.toml` or `gpy.toml`

**Common Operations**:

```rust
// Create watcher with callback
let callback: EventCallback = Box::new(|event| {
    println!("Debounced event: {:?}", event);
});
let mut coordinator = WatchCoordinator::new(Duration::from_millis(150), callback)?;

// Watch git directory
coordinator.watch_directory("/home/user/project/.git")?;

// Graceful shutdown
coordinator.stop();
```

#### `watcher/debouncer.rs`

Event coalescing engine to prevent prompt flicker.

**Key Types**:

```rust
pub struct DebounceEngine {
    pending: HashMap<PathBuf, PendingEventData>,
    debounce_window: Duration,
    callback: Option<EventCallback>,
}

struct PendingEventData {
    event: FileEvent,
    expires_at: Instant,
}
```

**Key Functions**:

- `handle_event(&mut self, pending: PendingEvent)` - Receive event from filesystem layer
- `flush_expired_events(&mut self)` - Check for expired events and invoke callback
- `clear(&mut self)` - Remove all pending events

**Debouncing Logic** (lines 50-100):

1. When event arrives for repository:
   - If no pending event: Create entry with expiry = now + debounce_window
   - If pending event exists: Reset expiry to now + debounce_window
2. Flush thread periodically calls `flush_expired_events()`:
   - Find entries where `expires_at < now`
   - Remove from HashMap and invoke callback
   - Delivers one debounced event per repository

**Common Operations**:

```rust
// Create debouncer
let mut debouncer = DebounceEngine::new(Duration::from_millis(150));
debouncer.set_callback(Box::new(|event| {
    println!("Debounced: {:?}", event);
}));

// Feed events
debouncer.handle_event(PendingEvent {
    event: FileEvent::Git { path: PathBuf::from("/project/.git/HEAD") },
    repo: PathBuf::from("/project"),
});

// Flush expired (called by background thread)
debouncer.flush_expired_events();
```

#### `watcher/filesystem.rs`

Low-level filesystem watcher using the `notify` crate.

**Key Types**:

```rust
pub struct FileSystemWatcher {
    watcher: RecommendedWatcher,
    tx: Sender<notify::Result<notify::Event>>,
    rx: Receiver<notify::Result<notify::Event>>,
}
```

**Key Functions**:

- `FileSystemWatcher::new(callback: PendingCallback) -> Result<Self>` - Create watcher
- `watch<P: AsRef<Path>>(&mut self, path: P)` - Watch directory recursively
- `unwatch<P: AsRef<Path>>(&mut self, path: P)` - Stop watching directory

**Event Processing** (lines 80-150):

1. `notify` crate sends raw filesystem events to channel
2. Filter events: Ignore creates/deletes for non-relevant files
3. Classify events: Use `should_trigger_update()` to determine event type
4. Forward `PendingEvent` to debouncer via callback

**Common Operations**:

```rust
// Create filesystem watcher
let callback: PendingCallback = Box::new(|pending| {
    println!("Pending event: {:?}", pending);
});
let mut watcher = FileSystemWatcher::new(callback)?;

// Start watching
watcher.watch("/home/user/project/.git")?;
```

#### `watcher/multi_repo.rs`

Multi-repository coordination with reference counting.

**Key Types**:

```rust
pub struct MultiRepoWatcher {
    watcher: Arc<Mutex<Option<WatchCoordinator>>>,
    watched_repos: Arc<Mutex<HashMap<PathBuf, WatchedRepo>>>,
}

struct WatchedRepo {
    git_root: PathBuf,
    git_dir: PathBuf,
    watch_worktree: bool,
    clients: HashSet<u32>,
}
```

**Key Functions**:

- `register_client(pid: u32, cwd: &Path)` - Start watching repo for client
- `unregister_client(pid: u32)` - Stop watching if last client
- `update_client(pid: u32, new_cwd: &Path)` - Update tracked directory
- `find_git_root(path: &Path) -> Option<PathBuf>` - Walk up to find `.git` directory

**Reference Counting** (lines 117-154):

1. Client registers with cwd → Find git root → Add to `WatchedRepo.clients`
2. If first client for repo: Start watching `.git/` directory
3. If existing repo: Just add PID to `clients` set (no new watcher)
4. Client unregisters → Remove PID from set
5. If `clients` is empty: Stop watcher, remove entry

**Worktree Support** (lines 45-51, 396-421):

- **Environment variable**: `GPY_WATCH_WORKTREE=1`
- **Default**: Only watch `.git/` directory (most efficient)
- **When enabled**: Also watch worktree directory for file changes
- **Use case**: Git worktrees where changes happen outside `.git/`

**Common Operations**:

```rust
// Create multi-repo watcher
let callback: EventCallback = Box::new(|event| {
    println!("Debounced: {:?}", event);
});
let watcher = MultiRepoWatcher::new(config, callback)?;

// Register Fish shell client
watcher.register_client(12345, Path::new("/home/user/project/src"))?;

// Unregister when shell exits
watcher.unregister_client(12345)?;
```

---

## Configuration System

**Location**: `gpy-agent/src/config/`

**Purpose**: Load and validate TOML configuration files.

### Key Modules

#### `config/schema.rs`

Configuration schema definitions.

**Key Types**:

```rust
pub struct Config {
    pub agent: AgentConfig,
    pub segments: SegmentConfig,
    pub theme: Theme,
}

pub struct AgentConfig {
    pub enabled: bool,
    pub log_level: LogLevel,
    pub show_seconds: bool,
}

pub struct SegmentConfig {
    pub order: Vec<String>,
    pub git_enabled: bool,
    pub language_enabled: bool,
    pub directory_enabled: bool,
    pub clock_enabled: bool,
    pub duration_enabled: bool,
    pub status_enabled: bool,
}

pub struct Theme {
    pub separators: Separators,
    pub colors: ColorTheme,
    pub icons: IconTheme,
}
```

**Key Functions**:

- `Config::load() -> Result<Self>` - Load config from XDG directories
- `Config::load_from_path(path: &Path) -> Result<Self>` - Load specific file
- `Config::validate(&self) -> Result<()>` - Validate configuration values

**Configuration Hierarchy** (lines 50-100):

1. **User config**: `~/.config/gpy/config.toml` (highest priority)
2. **System config**: `/etc/gpy/config.toml`
3. **Built-in defaults**: Compiled-in fallback values

**Common Operations**:

```rust
// Load user configuration
let config = Config::load()?;

// Check if git segment enabled
if config.segments.git_enabled {
    println!("Git segment is enabled");
}

// Access theme colors
let branch_color = &config.theme.colors.git_branch;
```

#### `config/loader.rs`

TOML file parsing and validation.

**Key Functions**:

- `load_config_file(path: &Path) -> Result<Config>` - Parse TOML file
- `merge_configs(base: Config, overlay: Config) -> Config` - Combine configs with overlay priority

**Error Handling** (lines 80-120):

- Invalid TOML syntax → `Error::config()` with line number
- Missing required fields → `Error::config()` with field name
- Invalid color format → `Error::config()` with validation message

---

## Error Handling

**Location**: `gpy-agent/src/error.rs`

**Purpose**: Unified error type for agent operations.

### Error Type

```rust
pub enum Error {
    Io(std::io::Error),
    Git(String),
    Language(String),
    Ipc(String),
    Config(String),
    Watcher(String),
}
```

**Key Functions**:

- `Error::git(msg: impl Into<String>) -> Self` - Create git error
- `Error::ipc(msg: impl Into<String>) -> Self` - Create IPC error
- `Error::config(msg: impl Into<String>) -> Self` - Create config error

**Error Propagation**:

```rust
// Use ? operator for error propagation
pub fn get_git_status(path: &Path) -> Result<GitInfo> {
    let repo = gix::open(path)
        .map_err(|e| Error::git(format!("Failed to open repo: {}", e)))?;

    // ... rest of function
}
```

**Error Display** (lines 50-100):

All errors implement `Display` trait with context:
- `Error::Git("Failed to parse branch name")` → "Git error: Failed to parse branch name"
- `Error::Ipc("Invalid message format")` → "IPC error: Invalid message format"

---

## Agent Core

**Location**: `gpy-agent/src/agent/`

**Purpose**: Main orchestrator coordinating IPC server, watcher, and clock timer.

**Architecture**: See [agent/mod.rs](../../gpy-agent/src/agent/mod.rs) module docs for detailed explanation.

### Key Types

```rust
pub struct Agent {
    config: Arc<Config>,
    git_cache: Arc<GitCache>,
    lang_cache: Arc<LanguageCache>,
    client_directory: Arc<ClientDirectory>,
    watcher: Option<MultiRepoWatcher>,
}

pub struct AgentState {
    pub config: Arc<Config>,
    pub git_cache: Arc<GitCache>,
    pub lang_cache: Arc<LanguageCache>,
    pub client_directory: Arc<ClientDirectory>,
}
```

### Key Functions

- `Agent::new(config: Config) -> Result<Self>` - Initialize agent with config
- `Agent::start(self) -> Result<()>` - Start agent in foreground
- `start_background(agent: Arc<Agent>, shutdown: ShutdownSignal)` - Async event loop (lines 250-350)
- `handle_git_request(state: &AgentState, path: &Path) -> Result<GitInfo>` - Process git status request
- `handle_language_request(state: &AgentState, path: &Path) -> Result<Option<LanguageInfo>>` - Process language request

### Event Loop Structure

**Location**: `agent.rs:311-380`

```rust
tokio::select! {
    // IPC server (long-lived)
    server_result = server_future => {
        // Server exited - propagate error or shutdown gracefully
    }

    // Shutdown signal handler (SIGTERM/SIGINT)
    shutdown = shutdown_signal => {
        // Stop all subsystems
    }

    // Clock timer (1s or 60s interval)
    _ = clock_timer.tick() => {
        // Send SIGUSR1 to all registered clients
    }
}
```

### State Management

**Shared State** (lines 100-150):

- `Arc<Config>` - Read-only configuration
- `Arc<GitCache>` - Git status cache (mutex-protected LRU)
- `Arc<LanguageCache>` - Language detection cache
- `Arc<ClientDirectory>` - Registered Fish shell PIDs

**Thread Safety**:

- `Arc` for shared ownership across async tasks
- `Mutex` for interior mutability (caches, client directory)
- `RwLock` not used (read-heavy workload doesn't justify complexity)

### Clock Timer

**Location**: `agent.rs:200-250`

**Timer Logic**:

- **show_seconds = false**: Tick every 60s, aligned to minute boundaries
- **show_seconds = true**: Tick every 1s, no alignment needed

**Signal Delivery** (lines 220-240):

When timer ticks:
1. Get all registered clients from `ClientDirectory`
2. Send SIGUSR1 to each client PID
3. Fish receives signal → Calls `commandline -f repaint`
4. Prompt re-renders with updated clock

**Alignment Algorithm** (lines 210-230):

```rust
// Calculate next minute boundary
let now = Instant::now();
let seconds_past_minute = (now.elapsed().as_secs() % 60) as u64;
let seconds_until_next_minute = 60 - seconds_past_minute;

// Sleep until boundary, then tick every 60s
sleep(Duration::from_secs(seconds_until_next_minute)).await;
loop {
    signal_clients();
    sleep(Duration::from_secs(60)).await;
}
```

### Common Operations

```rust
// Initialize agent
let config = Config::load()?;
let agent = Agent::new(config)?;

// Start event loop (blocks until shutdown)
agent.start()?;
```

---

## Cross-References

### Request Flow Diagrams

See [architecture.md § Request/Response Flow](../dev/architecture.md#requestresponse-flow) for detailed sequence diagrams.

### Threading Model

See [architecture.md § Threading Model](../dev/architecture.md#threading-model) for async task coordination.

### Cache Invalidation

See [caching-strategy.md](caching-strategy.md) for cache timing relationships.

### Adding New Segments

See [segment-development.md](../dev/segment-development.md) for step-by-step guide.

---

## Module Dependency Graph

```
agent.rs (orchestrator)
├── ipc/server.rs ──────────┐
│   ├── ipc/protocol.rs     │
│   └── ipc/client_directory.rs
│                           │
├── git/commands.rs ────────┤
│   └── git/cache.rs        │
│                           │
├── language/detector.rs ───┤
│   └── language/registry.rs│
│                           │
├── watcher/multi_repo.rs ──┤
│   ├── watcher/mod.rs      │  All depend on:
│   ├── watcher/debouncer.rs│  • config/schema.rs
│   └── watcher/filesystem.rs  • error.rs
│                           │
└── config/loader.rs ───────┘
    └── config/schema.rs
```

**Key Insight**: The dependency graph is **acyclic** - no circular dependencies. This makes testing easier and refactoring safer.

---

## Testing Patterns

### Unit Tests

Located in each module file using `#[cfg(test)]`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_git_status_parsing() {
        // Test git status detection logic
    }
}
```

### Integration Tests

Located in `gpy-agent/tests/`:

- `git_status_tests.rs` - Git subsystem integration
- `language_tests.rs` - Language detection integration
- `agent_daemon_tests.rs` - Agent lifecycle and IPC
- `integration_tests.rs` - End-to-end signal delivery

### Test Utilities

**Location**: `gpy-agent/tests/test_helpers.rs`

Common test utilities:
- `create_test_repo()` - Set up temporary git repository
- `create_test_agent()` - Initialize agent with test config
- `wait_for_signal()` - Block until SIGUSR1 received (for E2E tests)

---

## Performance Considerations

### Hot Paths

**Most frequent operations** (profiled in production):

1. **Git status queries**: ~10-50ms (cached: <1ms)
2. **Language detection**: ~5-20ms (cached: <1ms)
3. **IPC round-trip**: ~1-2ms total latency
4. **Signal delivery**: ~0.1ms per client PID

### Cache Tuning

**Git cache** (lines in `git/cache.rs:40-80`):

- **Capacity**: 100 repos (most users have <10)
- **TTL**: 60s (balance freshness vs load)
- **Cooldown**: 150ms (prevents watcher-triggered redundant queries)

**Language cache** (lines in `language/cache.rs:30-60`):

- **Capacity**: 50 directories
- **TTL**: 300s (languages change infrequently)

### Memory Profile

**Typical agent memory usage**: 2-5 MB resident

- `GitCache`: ~100 entries × 200 bytes = 20 KB
- `LanguageCache`: ~50 entries × 100 bytes = 5 KB
- `ClientDirectory`: ~10 clients × 50 bytes = 500 bytes
- Watcher buffers: ~100 KB
- Tokio runtime overhead: ~1-2 MB

---

## Next Steps

- **For segment development**: Read [segment-development.md](../dev/segment-development.md)
- **For cache tuning**: Read [caching-strategy.md](caching-strategy.md)
- **For system architecture**: Read [architecture.md](../dev/architecture.md)

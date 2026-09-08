# GPY Codebase Map

> **Archived.** This codebase map is historical reference material and may not reflect current code organization. Refer to the current source tree under `gpy-agent/src/` for how the code is organized today.

**Version**: 0.1.0
**Last Updated**: 2025-11-17
**Purpose**: Accelerate AI-assisted development by providing module relationships, critical paths, and quick reference guides.

---

## Table of Contents

1. [Module Dependency Graph](#module-dependency-graph)
2. [Critical Path Documentation](#critical-path-documentation)
3. [Quick Reference Guides](#quick-reference-guides)
4. [Function Navigation Metadata](#function-navigation-metadata)
5. [Testing Patterns](#testing-patterns)

---

## Module Dependency Graph

### High-Level Architecture

```
┌─────────────────────────────────────────────────────────────┐
│                      Fish Shell Layer                        │
│  (core/*.fish, segments/*.fish, themes/*.fish)              │
└────────────────────┬────────────────────────────────────────┘
                     │ IPC (Unix Socket / Named Pipe)
                     ▼
┌─────────────────────────────────────────────────────────────┐
│                    Rust Agent Process                        │
│                   gpy-agent/src/agent.rs                     │
│                      (1719 lines)                            │
│                                                              │
│  ┌──────────────────────────────────────────────────────┐  │
│  │         Main Event Loop (tokio::select!)             │  │
│  │  - IPC Server                                        │  │
│  │  - File Watcher                                      │  │
│  │  - Clock Timer                                       │  │
│  │  - Pruning Timer                                     │  │
│  └──────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────┘
```

### Module Relationships (gpy-agent/src/)

```
lib.rs (Entry Point)
├── agent.rs (1719 lines) ★ COORDINATION HUB ★
│   ├── Uses: ipc::server, watcher, config, theme, git::cache, language::cache
│   ├── Owns: Main event loop, cross-module coordination
│   └── Exports: Agent struct, start_background(), oneshot commands
│
├── ipc/ (IPC Communication Layer)
│   ├── mod.rs - Message & Response enums
│   ├── server.rs (1826 lines) ★ NEEDS REFACTORING ★
│   │   ├── Uses: protocol, registry, git, language, theme, formatter
│   │   └── Exports: EndpointHandle, serve_connection()
│   ├── handlers/ (NEW - from Phase 2 refactoring)
│   │   ├── mod.rs - RequestHandler trait
│   │   └── git_handler.rs - Git request handling
│   ├── protocol.rs - Fish message conversion, validation
│   ├── transport.rs - Socket I/O, message framing
│   ├── registry.rs - Client tracking (PIDs, directories)
│   ├── client.rs - Client connection management
│   └── latency.rs - Request timing metrics
│
├── git/ (Git Repository Operations)
│   ├── mod.rs - RepositoryStatus struct, RepositoryState enum
│   ├── commands.rs - Git operations via gix library
│   ├── status.rs - Status detection and parsing
│   ├── repository.rs - Repository discovery and validation
│   └── cache.rs - GitStatusCache (30s TTL, 150ms cooldown)
│
├── language/ (Language Detection)
│   ├── mod.rs - LexiconDescriptor struct
│   ├── detector.rs - Hyperpolyglot integration
│   ├── version.rs - Version string extraction
│   └── cache.rs - LanguageCache (24h TTL, no cooldown)
│
├── watcher/ (Filesystem Watching)
│   ├── mod.rs - FileEvent enum
│   ├── multi_repo.rs - MultiRepoWatcher (manages multiple repos)
│   ├── filesystem.rs - FileSystemWatcher (notify integration)
│   └── debouncer.rs - DebounceEngine (100ms window)
│
├── formatter/ (Output Formatting)
│   ├── mod.rs - Format enum, Formatter trait, create_formatter()
│   ├── fish_args.rs - Fish shell argument format
│   ├── fish_source.rs - Fish variable assignments
│   ├── fish_ansi.rs - ANSI escape sequences (NEW)
│   └── json.rs - JSON output (NEW)
│
├── config/ (Configuration Management)
│   ├── mod.rs (1256 lines) ★ NEEDS REFACTORING ★
│   │   ├── Uses: theme (for LanguageIcons)
│   │   └── Exports: Config struct, load_config()
│   └── (PLANNED: defaults.rs, language_icons.rs, validation.rs)
│
├── theme/ (Theme Management)
│   ├── mod.rs - Theme struct, default themes
│   └── manager.rs - ThemeManager, hot-reload support
│
├── cache/ (Shared Cache Infrastructure)
│   ├── mod.rs - CachePolicy, CacheEntry, Timestamp
│   └── Used by: git::cache, language::cache
│
├── security.rs - Path validation, rate limiting
├── error.rs - Error types, Result alias
└── debug.rs - Debug logging (GPY_DEBUG_LOG)
```

### Dependency Flow (Data & Control)

```
Fish Shell Request
    ↓
ipc/transport.rs (Socket I/O)
    ↓
ipc/protocol.rs (Validate & Parse)
    ↓
ipc/server.rs (Route to Handler)
    ↓
┌─────────────────┬─────────────────┬─────────────────┐
│   Git Request   │  Lang Request   │  Theme Request  │
↓                 ↓                 ↓                 ↓
git/commands.rs   language/detector theme/manager.rs
    ↓                 ↓                 ↓
git/cache.rs      language/cache.rs  (config.toml)
    ↓                 ↓                 ↓
└─────────────────┴─────────────────┴─────────────────┘
    ↓
formatter/ (Format Response)
    ↓
ipc/transport.rs (Send Response)
    ↓
Fish Shell (Render Prompt)
```

---

## Critical Path Documentation

### 1. Prompt Rendering Flow

**Path**: Fish → IPC → Agent → Response

**Timeline**: Target <5ms end-to-end (1ms IPC + 2ms git + 1ms format + 1ms overhead)

**Steps**:
1. Fish calls `__gpy_request "git" $PWD "fish-ansi" false`
   - Location: `core/ipc.fish` function `__gpy_request`
2. IPC client connects to socket
   - Location: `gpy-agent/src/ipc/client.rs:connect()`
3. Message sent as JSON
   - Location: `gpy-agent/src/ipc/transport.rs:send_message()`
   - Format: `{"op":"git","cwd":"/path","format":"fish-ansi"}`
4. Server validates and parses
   - Location: `gpy-agent/src/ipc/protocol.rs:FishMessage::into_message()`
   - Validates: Path safety, message size (<64KB), rate limits
5. Handler processes request
   - Location: `gpy-agent/src/ipc/server.rs:serve_connection()`
   - Checks cache first (git::cache.rs)
   - Runs git status if cache miss (git/commands.rs)
6. Formatter converts to Fish ANSI
   - Location: `gpy-agent/src/formatter/fish_ansi.rs`
7. Response sent back
   - Location: `gpy-agent/src/ipc/transport.rs:send_response()`
8. Fish renders prompt with ANSI codes
   - Location: `segments/git.fish:segment_git_render()`

**Critical Files**:
- `core/ipc.fish` (Fish side)
- `gpy-agent/src/ipc/server.rs:379` (Main request handler)
- `gpy-agent/src/git/cache.rs:95` (Cache lookup)
- `gpy-agent/src/formatter/fish_ansi.rs` (Format output)

---

### 2. Git Status Detection Pipeline

**Entry Point**: `gpy-agent/src/git/commands.rs:get_repository_status()`

**Flow**:
```
1. Repository Discovery
   ├── git/repository.rs:find_repository_root()
   ├── Walk up directory tree looking for .git/
   └── Canonicalize path for security

2. Status Collection (using gix library)
   ├── git/status.rs:collect_status_counts()
   ├── Query: staged, unstaged, untracked files
   └── Detect: merge/rebase/cherry-pick state

3. Branch Information
   ├── git/commands.rs:get_current_branch()
   ├── Read .git/HEAD for branch name
   └── Count ahead/behind commits vs upstream

4. Cache Storage
   ├── git/cache.rs:GitStatusCache::set()
   ├── TTL: 30 seconds
   └── Cooldown: 150ms (prevents duplicate queries)

5. Return RepositoryStatus
   └── git/mod.rs:RepositoryStatus struct
```

**Performance Characteristics**:
- **Cold cache**: 50-500ms (depends on repo size)
- **Warm cache**: <1ms
- **Large repos**: Timeout protection at 5s (configurable)

**Key Functions**:
- `git/commands.rs:get_repository_status()` - Main entry point
- `git/repository.rs:find_repository_root()` - Repo discovery
- `git/status.rs:collect_status_counts()` - gix integration
- `git/cache.rs:get_cached_or_compute()` - Cache coordination

---

### 3. Language Detection and Caching

**Entry Point**: `gpy-agent/src/language/detector.rs:detect_languages()`

**Flow**:
```
1. Directory Scan
   ├── language/detector.rs:detect_languages()
   ├── Use hyperpolyglot to scan files
   └── Returns: Vec<Language> with confidence scores

2. Version Detection (per language)
   ├── language/version.rs:get_version()
   ├── Check cache first (24h TTL)
   ├── If miss: Run tool command (rustc --version, etc.)
   └── Parse version string

3. Cache Storage
   ├── language/cache.rs:LanguageCache::set()
   ├── TTL: 24 hours (versions change infrequently)
   └── No cooldown (version lookups are idempotent)

4. Theme Color Lookup
   ├── config/mod.rs:LanguageIcons::get_color()
   └── Return color hex code from config

5. Return Vec<LexiconDescriptor>
   └── language/mod.rs:LexiconDescriptor struct
```

**Performance Characteristics**:
- **Detection**: 10-100ms (file I/O bound)
- **Version lookup (cold)**: 50-200ms (subprocess spawn)
- **Version lookup (warm)**: <1ms (cached)
- **Cache invalidation**: Manual only (24h expiry)

**Key Functions**:
- `language/detector.rs:detect_languages()` - Hyperpolyglot integration
- `language/version.rs:get_version()` - Version extraction
- `language/cache.rs:get_cached_or_compute()` - Cache coordination

---

### 4. Live Update Mechanism (File Watcher → SIGUSR1)

**Entry Point**: `gpy-agent/src/watcher/multi_repo.rs:MultiRepoWatcher::new()`

**Flow**:
```
1. Watch Registration
   ├── watcher/multi_repo.rs:add_watch()
   ├── Register .git directory and config files
   └── Uses notify crate for filesystem events

2. Event Detection
   ├── watcher/filesystem.rs:FileSystemWatcher
   ├── Receives: Create, Write, Remove events
   └── Filters: Ignore temp files, editor backups

3. Debouncing (100ms window)
   ├── watcher/debouncer.rs:DebounceEngine
   ├── Coalesce rapid events (e.g., git commit writes multiple files)
   ├── Flush thread runs every 100ms
   └── Emit event after 100ms of silence

4. Event Routing
   ├── agent.rs:handle_file_event() [line 1065]
   ├── FileEvent::Git → Invalidate git cache + reload status
   ├── FileEvent::Config → Invalidate config cache + reload theme
   └── FileEvent::Language → Invalidate language cache

5. Client Notification
   ├── agent.rs → ClientDirectory::notify_sigusr1_force()
   ├── ipc/registry.rs:send_signal_to_pid()
   ├── Send SIGUSR1 to all registered Fish shells
   └── Fish shells re-render prompt on signal

6. Fish Prompt Update
   ├── Fish receives SIGUSR1
   ├── Calls fish_prompt again
   └── Fetches fresh data via IPC
```

**Why 100ms debounce?**
- Fast enough to feel responsive (<200ms human perception threshold)
- Long enough to coalesce multi-file git operations (commit, merge)
- Prevents prompt flicker from rapid changes

**Key Functions**:
- `agent.rs:handle_file_event()` [line 1065] - Event coordination
- `watcher/debouncer.rs:DebounceEngine::insert()` - Event coalescing
- `ipc/registry.rs:notify_sigusr1_force()` - Signal sending

---

### 5. Configuration Hot-Reload Flow

**Entry Point**: Watcher detects config file change

**Flow**:
```
1. File Change Detected
   ├── watcher/ detects config.toml or theme file change
   └── Emits FileEvent::Config or FileEvent::Theme

2. Cache Invalidation
   ├── agent.rs:handle_file_event() [line 1065]
   ├── config::loader::invalidate_theme_cache()
   └── Clears in-memory config cache

3. Theme Re-Export (on next request)
   ├── theme/manager.rs:export_theme()
   ├── Reads fresh config from disk
   ├── Generates Fish variable assignments
   └── Writes to ~/.config/fish/gpy/theme_export.fish

4. Client Notification (SIGUSR2)
   ├── theme/manager.rs sends SIGUSR2 via callback
   ├── ipc/registry.rs:notify_sigusr2()
   └── Fish shells re-source theme export

5. Fish Re-Source
   ├── Fish receives SIGUSR2
   ├── Calls: source ~/.config/fish/gpy/theme_export.fish
   └── Variables updated (colors, icons, enabled segments)
```

**Note**: Config reload is **lazy** - changes don't take effect until next `theme export` request or manual reload.

**Key Functions**:
- `config/mod.rs:load_config()` - Parse TOML
- `theme/manager.rs:export_theme()` - Generate Fish variables
- `agent.rs:handle_file_event()` - Trigger reload

---

## Quick Reference Guides

### How to Add a New IPC Command

**Prerequisites**: Read `lib.rs` "How to Add a New IPC Operation" section

**Steps**:

1. **Add Message variant** (`gpy-agent/src/ipc/mod.rs`):
   ```rust
   #[derive(Debug, Serialize, Deserialize)]
   pub enum Message {
       // ... existing variants
       MyNewOperation {
           path: String,
           #[serde(default)]
           format: Format,
       },
   }
   ```

2. **Add Response variant** (`gpy-agent/src/ipc/mod.rs`):
   ```rust
   #[derive(Debug, Serialize, Deserialize)]
   pub enum Response {
       // ... existing variants
       MyNewOperationResult {
           data: String,
       },
   }
   ```

3. **Add Fish protocol mapping** (`gpy-agent/src/ipc/protocol.rs`):
   ```rust
   // In FishMessage::into_message()
   "my_op" => Ok(Message::MyNewOperation {
       path: cwd.unwrap_or_else(|| ".".to_owned()),
       format,
   }),
   ```

4. **Add handler** (`gpy-agent/src/ipc/server.rs:serve_connection()`):
   ```rust
   // In process_request_secure() match block
   Message::MyNewOperation { path, format } => {
       let result = self.my_new_handler(&path)?;
       Ok(Response::MyNewOperationResult { data: result })
   }
   ```

5. **Add formatter support** (all formatters):
   - `gpy-agent/src/formatter/json.rs` - Add match arm (serde handles rest)
   - `gpy-agent/src/formatter/fish_ansi.rs` - Add colored output
   - `gpy-agent/src/formatter/fish_source.rs` - Add Fish variables

6. **Add tests**:
   - `gpy-agent/tests/ipc_protocol_tests.rs` - Protocol parsing
   - `gpy-agent/tests/formatter_tests.rs` - Output formatting
   - `gpy-agent/tests/integration_tests.rs` - End-to-end flow

**Validation Checklist**:
- [ ] Message size < 64KB
- [ ] Path validation (no `..`, `/etc`, `/sys`)
- [ ] Rate limiting respected
- [ ] Error handling (invalid path, timeout)
- [ ] All formatters implemented
- [ ] Tests pass: `./scripts/quality-check.sh`

---

### How to Add a New Segment Type

**Prerequisites**: Read `lib.rs` "How to Add a New Prompt Segment" section

**Steps**:

1. **Create segment file** (`segments/my_segment.fish`):
   ```fish
   function segment_my_segment_detect
       # Return 0 to show, 1 to hide
       test "$GPY_MY_SEGMENT_ENABLED" != 0
   end

   function segment_my_segment_render --argument-names is_last
       # Get data from agent if needed
       set -l data (__gpy_request my_op "$PWD" "fish-ansi" "$is_last")

       # Or implement pure Fish logic
       gpy_section_start $__color_my_segment_bg $__color_my_segment_fg
       gpy_section_append $__color_my_segment_bg $__color_my_segment_fg " $data"
       gpy_section_end $__color_my_segment_bg $is_last
   end
   ```

2. **Register segment** (`core/init.fish`):
   ```fish
   set -g __enabled_segments clock duration directory my_segment git
   ```

3. **Add theme config** (`gpy-agent/src/theme/mod.rs`):
   ```toml
   [segments.my_segment]
   enabled = true
   icon = "📦"
   fg_color = "#ffffff"
   bg_color = "#3572a5"
   ```

4. **Update theme export** (`gpy-agent/src/theme/manager.rs`):
   ```rust
   // Add to export_theme() output
   writeln!(out, "set -gx GPY_MY_SEGMENT_ENABLED {}",
       if config.segments.my_segment.enabled { 1 } else { 0 })?;
   writeln!(out, "set -g __color_my_segment_fg {}",
       config.segments.my_segment.fg_color)?;
   ```

5. **Add Fish test** (`tests/fish/segment_my_segment.test.fish`):
   ```fish
   #!/usr/bin/env fish
   # Test segment detection and rendering
   source (dirname (status --current-filename))/test_helpers.fish

   @test "segment_my_segment_detect returns 0 when enabled"
       set -gx GPY_MY_SEGMENT_ENABLED 1
       segment_my_segment_detect
       assert_equals $status 0 "Segment should be enabled"
   end
   ```

**Validation Checklist**:
- [ ] Segment detect function returns 0/1
- [ ] Segment render uses `gpy_section_*` helpers
- [ ] `is_last` parameter handled correctly
- [ ] Theme colors configurable
- [ ] Enable/disable toggle works
- [ ] Tests pass: `./scripts/test_fish.sh`

---

### How to Modify Themes

**Theme Files**:
- `gpy-agent/src/theme/mod.rs` - Theme struct, defaults
- `gpy-agent/src/theme/manager.rs` - Export logic
- `themes/*.fish` - Fish-side theme presets

**Color Configuration** (`~/.config/gpy/config.toml`):
```toml
[theme]
prompt_close = ""
prompt_open = ""

[theme.colors]
directory_bg = "#3572a5"
directory_fg = "#ffffff"
git_clean_bg = "#00ff00"
git_dirty_bg = "#ff0000"

[theme.icons]
git_branch = ""
git_staged = "●"
language_rust = ""
```

**Hot-Reload Flow**:
1. Edit `~/.config/gpy/config.toml`
2. Watcher detects change → sends SIGUSR2
3. Fish re-sources `~/.config/fish/gpy/theme_export.fish`
4. New colors/icons applied immediately

**Adding a New Color**:
1. Add to `config.example.toml`
2. Add to `gpy-agent/src/config/mod.rs:ThemeConfig` struct
3. Add to `gpy-agent/src/theme/manager.rs:export_theme()`
4. Use in segment: `gpy_section_start $__color_my_new_color`

---

### Common Testing Patterns

**Test Harness Utilities** (`gpy-agent/tests/test_harness.rs`):

```rust
// Deterministic signal tracking
let counter = SignalCounter::new();
let pid = std::process::id();
counter.register_handler(pid);
// ... perform action that sends signal ...
counter.wait_for_count(1, Duration::from_secs(5));

// Event synchronization
let flag = EventFlag::new();
std::thread::spawn({
    let flag = flag.clone();
    move || {
        // ... do work ...
        flag.set();
    }
});
flag.wait(Duration::from_secs(5));
```

**Integration Test Pattern**:

```rust
#[test]
fn test_git_change_triggers_signal() {
    let temp_dir = TempDir::new().unwrap();
    let repo = create_test_git_repo(&temp_dir);

    let server = EndpointHandle::with_path(
        socket_path(),
        registry,
        cache,
        config,
        /* ... */
    ).unwrap();

    // Make git change
    std::fs::write(repo.join("file.txt"), "content").unwrap();

    // Wait for debounce + signal
    std::thread::sleep(Duration::from_millis(200));

    // Verify signal sent
    assert!(counter.count() > 0);
}
```

**Fish Test Pattern** (`tests/fish/*.test.fish`):

```fish
#!/usr/bin/env fish
source (dirname (status --current-filename))/test_helpers.fish

@test "IPC request returns git status"
    set -l result (__gpy_request git $PWD json false)
    assert_not_empty "$result" "Should return JSON response"
    echo $result | string match -q '*"branch"*'
    assert_equals $status 0 "Response should contain branch field"
end
```

---

## Function Navigation Metadata

### Critical Entry Points

**Agent Initialization**:
- **Location**: `gpy-agent/src/agent.rs:start_background()`
- **Line**: ~190
- **Purpose**: Start agent daemon with IPC server and file watcher
- **Callers**: `gpy-agent/src/bin/gpy-agent.rs:main()`
- **Callees**: `EndpointHandle::spawn()`, `MultiRepoWatcher::new()`
- **Side Effects**: Spawns tokio runtime, creates socket, starts threads

**Main Event Loop**:
- **Location**: `gpy-agent/src/agent.rs:start_background()`
- **Line**: ~250 (tokio::select! block)
- **Purpose**: Coordinate IPC server, watcher, clock, pruning timer
- **Callers**: N/A (loop runs until shutdown)
- **Callees**: `handle_file_event()`, `ClientDirectory::prune_dead_clients()`
- **Side Effects**: Long-running loop, handles signals

**IPC Request Handler**:
- **Location**: `gpy-agent/src/ipc/server.rs:serve_connection()`
- **Line**: ~379
- **Purpose**: Handle incoming IPC request from Fish shell
- **Callers**: `EndpointHandle::accept_loop()`
- **Callees**: `process_request_secure()`, formatter functions
- **Side Effects**: Socket I/O, cache updates, logging
- **Performance**: Target <1ms per request

**Git Status Entry Point**:
- **Location**: `gpy-agent/src/git/commands.rs:get_repository_status()`
- **Line**: ~50
- **Purpose**: Get git status with caching
- **Callers**: `ipc/server.rs:serve_connection()`
- **Callees**: `find_repository_root()`, `collect_status_counts()`, cache
- **Side Effects**: Disk I/O, spawns git processes
- **Performance**: 50-500ms cold, <1ms warm

**Language Detection Entry Point**:
- **Location**: `gpy-agent/src/language/detector.rs:detect_languages()`
- **Line**: ~30
- **Purpose**: Detect programming languages in directory
- **Callers**: `ipc/server.rs:serve_connection()`
- **Callees**: `hyperpolyglot::detect()`, `version::get_version()`
- **Side Effects**: Disk I/O, subprocess spawns
- **Performance**: 10-100ms detection, 50-200ms version

**Config Loading Entry Point**:
- **Location**: `gpy-agent/src/config/mod.rs:load_config()`
- **Line**: ~200
- **Purpose**: Load and parse config.toml
- **Callers**: `agent.rs:start_background()`, `theme/manager.rs`
- **Callees**: `toml::from_str()`, validation functions
- **Side Effects**: File I/O, cache updates
- **Performance**: <10ms typical

**File Event Handler**:
- **Location**: `gpy-agent/src/agent.rs:handle_file_event()`
- **Line**: ~1065
- **Purpose**: Process filesystem changes and coordinate subsystems
- **Callers**: Watcher callback in main event loop
- **Callees**: Cache invalidation, `ClientDirectory::notify_sigusr1_force()`
- **Side Effects**: Cache clears, signals sent to clients
- **Performance**: <5ms typical

---

## Where to Find Key Functionality

### Git Operations
- **Status detection**: `gpy-agent/src/git/commands.rs`
- **Repository discovery**: `gpy-agent/src/git/repository.rs`
- **Caching**: `gpy-agent/src/git/cache.rs`
- **Tests**: `gpy-agent/tests/git_status_tests.rs` (40+ tests)

### Language Detection
- **Detection logic**: `gpy-agent/src/language/detector.rs`
- **Version parsing**: `gpy-agent/src/language/version.rs`
- **Caching**: `gpy-agent/src/language/cache.rs`
- **Tests**: `gpy-agent/tests/language_tests.rs` (20+ tests)

### IPC Protocol
- **Message definitions**: `gpy-agent/src/ipc/mod.rs`
- **Fish protocol**: `gpy-agent/src/ipc/protocol.rs`
- **Server logic**: `gpy-agent/src/ipc/server.rs`
- **Transport layer**: `gpy-agent/src/ipc/transport.rs`
- **Tests**: `gpy-agent/tests/ipc_protocol_tests.rs`, `tests/fish/ipc_security.test.fish`

### File Watching
- **Multi-repo coordination**: `gpy-agent/src/watcher/multi_repo.rs`
- **Debouncing**: `gpy-agent/src/watcher/debouncer.rs`
- **Filesystem events**: `gpy-agent/src/watcher/filesystem.rs`
- **Tests**: `gpy-agent/tests/watcher_edge_cases.rs`

### Configuration
- **Config schema**: `gpy-agent/src/config/mod.rs`
- **Theme management**: `gpy-agent/src/theme/manager.rs`
- **Example config**: `config.example.toml`
- **Tests**: `gpy-agent/tests/config_tests.rs`

### Output Formatting
- **Format enum**: `gpy-agent/src/formatter/mod.rs`
- **JSON output**: `gpy-agent/src/formatter/json.rs`
- **Fish ANSI**: `gpy-agent/src/formatter/fish_ansi.rs`
- **Fish source**: `gpy-agent/src/formatter/fish_source.rs`
- **Tests**: `gpy-agent/tests/formatter_tests.rs`

### Security
- **Path validation**: `gpy-agent/src/security.rs`
- **Rate limiting**: `gpy-agent/src/security.rs`
- **Protocol validation**: `gpy-agent/src/ipc/protocol.rs`
- **Tests**: `gpy-agent/tests/security_tests.rs`, `tests/fish/ipc_security.test.fish`

---

## Testing Patterns

### Unit Tests
- **Location**: Same file as code, `#[cfg(test)] mod tests`
- **Pattern**: Test pure logic, mock I/O
- **Example**: `gpy-agent/src/cache/mod.rs` (cache policy tests)

### Integration Tests
- **Location**: `gpy-agent/tests/*.rs`
- **Pattern**: Test cross-module interactions, use real filesystem
- **Example**: `gpy-agent/tests/integration_tests.rs`

### End-to-End Tests
- **Location**: `tests/fish/*.test.fish`
- **Pattern**: Test full Fish → Agent → Response flow
- **Example**: `tests/fish/e2e_agent_autostart.test.fish`

### Test Fixtures (Planned - Phase 1)
- **Location**: `gpy-agent/tests/common/fixtures.rs`
- **Includes**:
  - `TempGitRepo` - Create temporary git repositories
  - `MockConfig` - Test configurations
  - `TestAgent` - Agent spawning with cleanup
  - `TempSocket` - Temporary Unix sockets

### Property-Based Tests (Planned - Phase 4)
- **Using**: `proptest` crate
- **Tests**: Config parsing, path validation, JSON serialization

---

## Performance Budgets

| Operation | Target | Measured | Status |
|-----------|--------|----------|--------|
| IPC roundtrip | <1ms | ~0.5ms | ✅ |
| Git status (cold) | <500ms | 50-300ms | ✅ |
| Git status (warm) | <1ms | <1ms | ✅ |
| Language detect | <100ms | 10-50ms | ✅ |
| Language version (cold) | <200ms | 50-150ms | ✅ |
| Language version (warm) | <1ms | <1ms | ✅ |
| Config load | <10ms | 2-5ms | ✅ |
| Watcher debounce | 100ms | 100ms | ✅ |
| Prompt render (total) | <5ms | 2-4ms | ✅ |

---

## Planned Refactorings (Phase 2)

### ipc/server.rs (1826 lines → ~600 lines)
**Split into**:
- `ipc/server.rs` - Connection management, main loop
- `ipc/handlers/git_handler.rs` - Git request handling
- `ipc/handlers/language_handler.rs` - Language detection handling
- `ipc/handlers/theme_handler.rs` - Theme query handling
- `ipc/handlers/client_handler.rs` - Client registration, ping, status

### agent.rs (1719 lines → ~600 lines)
**Split into**:
- `agent/mod.rs` - Main event loop, initialization
- `agent/state.rs` - AgentState struct, state machine
- `agent/handlers/config_handler.rs` - Config reload events
- `agent/handlers/watcher_handler.rs` - File event handling
- `agent/handlers/signal_handler.rs` - SIGUSR1/SIGUSR2 handling
- `agent/handlers/timer_handler.rs` - Clock tick, pruning events

### config/mod.rs (1256 lines → ~400 lines)
**Split into**:
- `config/mod.rs` - Config struct, loading logic
- `config/defaults.rs` - Default values, constants
- `config/language_icons.rs` - LanguageIcons struct
- `config/validation.rs` - Validation logic, schema checks

---

## Related Documentation

- **Architecture**: `docs/ARCHITECTURE.md`
- **Development**: `AGENT.md`
- **TODO**: `TODO.md`
- **Formatter Design**: `gpy-agent/docs/formatter-architecture.md`
- **Test Harness**: `gpy-agent/tests/README.md`
- **Fish Tests**: `tests/fish/README.md`
- **Schema Evolution**: `gpy-agent/SCHEMA_EVOLUTION.md`

---

**Last Updated**: 2025-11-17
**Next Review**: After Phase 2 refactoring (split large files)

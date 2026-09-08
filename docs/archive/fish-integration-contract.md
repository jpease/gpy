# Fish Integration Contract

> **Archived.** This is historical documentation of the Fish shell contract. Refer to current shell integration code in `fish/`, `bash/`, and `zsh/` for active contracts.

This document defines the exact interface contract between Fish shell and the GPY Agent. Use this as a reference when implementing new segments, modifying IPC communication, or extending the prompt system.

**Last Updated**: 2026-06-26

> **See also**: Zsh and Bash integrations (`zsh/` and `bash/`) follow the same contract. The segment rendering split (agent-rendered vs shell-rendered) described in the [SP2 Shell Contract](#sp2-shell-contract-colour-palette-rendering-199) section below applies equally to all three shells.

---

## Table of Contents

1. [IPC Communication Patterns](#ipc-communication-patterns)
2. [Function Naming Conventions](#function-naming-conventions)
3. [Environment Variable Contracts](#environment-variable-contracts)
4. [Error Handling Expectations](#error-handling-expectations)
5. [Segment Interface](#segment-interface)
6. [Signal Handling](#signal-handling)
7. [Agent Lifecycle Management](#agent-lifecycle-management)
8. [SP2 Shell Contract (Colour-Palette Rendering, #199)](#sp2-shell-contract-colour-palette-rendering-199)

---

## IPC Communication Patterns

### Transport Layer

**Protocol**: Unix domain sockets with JSON payloads
**Socket Path Resolution** (in order of precedence):
1. `$GPY_AGENT_SOCKET_PATH` (for testing)
2. `$XDG_RUNTIME_DIR/gpy/gpy.sock`
3. `$XDG_CACHE_HOME/gpy/gpy.sock`
4. `$HOME/.cache/gpy/gpy.sock`
5. `/tmp/gpy/gpy.sock` (fallback)

**Timeout**: Default 500ms per request (configurable via `$GPY_IPC_TIMEOUT_MS`)
**Message Size Limit**: 64KB

### Request/Response Pattern

All IPC requests follow this pattern:

```fish
# Build JSON payload
set -l payload '{"op":"operation_name","field":"value"}'

# Send request and receive response
set -l response (__gpy_ipc_send $payload $GPY_IPC_TIMEOUT_MS)

# Handle response (empty string = timeout/error)
if test -z "$response"
    # Handle failure (fallback or skip)
    return 1
end

# Parse response
# Response is JSON string, use string matching or jq for parsing
```

### Message Format

Fish sends messages using the **legacy format** with an `op` field:

```json
{"op":"git","cwd":"/path/to/repo","format":"fish-ansi"}
```

The agent responds in **native Rust format**:

```json
{"RepositoryStatus":{"branch":"main","ahead":0,"behind":0,"staged":1,"unstaged":0,"untracked":0,"conflicts":0,"state":""}}
```

Or for pre-rendered output (when `format` is `fish-ansi` or `fish-source`):

```
\x1b[38;2;100;149;237m  main \x1b[0m...
```

### Supported Operations

| Operation | Payload | Response | Fallback |
|-----------|---------|----------|----------|
| `git` | `{"op":"git","cwd":"/path","format":"fish-ansi","is_last":false}` | ANSI-rendered git status or JSON | `gpy-agent oneshot git --cwd /path --format fish-ansi` |
| `lang` | `{"op":"lang","cwd":"/path","format":"fish-ansi","is_last":true}` | ANSI-rendered language info or JSON | `gpy-agent oneshot lang --cwd /path --format fish-ansi` |
| `register` | `{"op":"register","pid":12345,"cwd":"/path"}` | `"Ack"` | None |
| `unregister` | `{"op":"unregister","pid":12345}` | `"Ack"` | None |
| `workspace` | `{"op":"workspace","pid":12345,"cwd":"/path"}` | `"Ack"` | None |
| `ping` | `{"op":"ping"}` | `"Ack"` | None |

### Oneshot Fallback

For data operations (`git`, `lang`), if IPC fails, the Fish integration **automatically falls back** to oneshot mode:

```fish
gpy-agent oneshot git --cwd "/path" --format fish-ansi
```

This ensures prompts always work even if the agent daemon is unavailable.

---

### Protocol Versioning and Compatibility

**Protocol Wire Format Version**: `1` (defined in `gpy-agent/src/ipc/protocol.rs::PROTOCOL_VERSION`)
**Agent Software Version**: Reported via `Response::AgentStatus.version` (from `CARGO_PKG_VERSION` in `gpy-agent/src/lib.rs::VERSION`)
**Protocol Version in Response**: Reported via `Response::AgentStatus.protocol_version` (from `PROTOCOL_VERSION`)

**Note**: The agent exposes both its software version (e.g., "0.1.5") and protocol version (e.g., 1) via the status operation. Fish shell can query these using `__gpy_get_agent_version` in `core/ipc.fish`, which extracts both the version string and the protocol version number. The software version follows semver for compatibility signaling (MAJOR.MINOR.PATCH).

**Version Checking**: When the protocol's MAJOR version changes (e.g., from 1 to 2), clients MUST compare the running agent's protocol version against the expected version and warn users if there's a mismatch. The `gpy status` command and Fish helpers implement this check automatically and will display warnings with fallback instructions when incompatible versions are detected.

#### Version Negotiation Strategy

The GPY IPC protocol uses **implicit version negotiation** via dual-format support:

1. **Fish shell sends legacy format** with `op` field (compatible with all versions)
2. **Agent responds in native format** specific to its version
3. **Fish shell handles responses flexibly** (ANSI strings or JSON)

This design ensures **forward compatibility**: Old Fish shell code continues working with new agents.

#### Handling Version Bumps

**For AI Agents implementing IPC features**, follow this guidance:

##### MINOR Version Bumps (Backward Compatible)

When **adding new operations** or **optional fields**:

```rust
// 1. Add new message variant
pub enum Message {
    // ... existing variants
    NewOperation {
        path: String,
        #[serde(default)]  // CRITICAL: Optional fields must have defaults
        format: Format,
    },
}
```

```rust
// 2. Add new response variant
pub enum Response {
    // ... existing variants
    NewOperationResult {
        data: String,
    },
}
```

**Fish Integration Steps**:
1. Add new operation handler in `core/ipc.fish`
2. Test with BOTH old and new agents (dual compatibility)
3. Document fallback behavior if operation not supported

**Migration Timeline**: No forced migration required. Old clients gracefully ignore new operations.

##### MAJOR Version Bumps (Breaking Changes)

When making **incompatible changes** (changing field types, removing operations):

1. **Increment PROTOCOL_VERSION** in `protocol.rs`
2. **Update Fish integration** to detect agent version before sending requests (using status operation - see "Agent Version Detection" below)
3. **Implement backward compatibility layer** that supports both old and new protocols during transition
4. **Provide migration guide** in release notes
5. **Deprecation period**: Support old version for minimum 2 releases

**Example Breaking Change Flow**:
```fish
# Fish checks version first (using status operation)
set -l version (__gpy_get_agent_version)

# Parse major version from "X.Y.Z" format
set -l major_version (string split '.' $version | head -n 1)

if test "$major_version" -lt 1
    # Use old protocol (version 0.x.x)
    set response (__gpy_request_v0 "git" $PWD)
else
    # Use new protocol (version 1.x.x+)
    set response (__gpy_request_v1 "git" $PWD)
end
```

**Note**: There is no explicit version handshake message. Clients detect the agent version by querying the status operation and parsing the version field from the response.

##### PATCH Version Bumps (Internal Only)

For **performance improvements** or **bug fixes** that don't affect the wire protocol:
- No Fish changes required
- No version bump needed
- Document in changelog only

#### Version Compatibility Matrix

| Agent Version | Fish Shell Compatibility | Notes |
|---------------|-------------------------|-------|
| 0.1.x | All versions | Initial protocol (version 1) |
| 0.2.x+ | All versions | MINOR additions only (backward compatible) |
| 1.0.x (hypothetical) | Requires Fish shell update | MAJOR breaking change |

#### Testing Version Compatibility

**Before releasing protocol changes**, run compatibility tests:

```bash
# Test old client with new agent
OLD_FISH=v0.1.0 NEW_AGENT=HEAD ./scripts/test-compat.sh

# Test new client with old agent
NEW_FISH=HEAD OLD_AGENT=v0.1.0 ./scripts/test-compat.sh
```

See `tests/schema_validation.rs` for JSON schema compatibility tests.

#### Agent Version Detection

Fish shell can detect agent version using the `__gpy_get_agent_version` helper (defined in `core/ipc.fish`):

```fish
# Get agent version (returns version string or empty on failure)
set -l agent_version (__gpy_get_agent_version)

if test -n "$agent_version"
    echo "Agent version: $agent_version"
else
    echo "Agent not available"
end
```

**Implementation details** (from `core/ipc.fish:183-211`):
- Sends status IPC request: `{"op":"status"}`
- Parses JSON response: `{"AgentStatus":{"version":"0.1.0",...}}`
- Extracts version field using regex pattern matching
- Returns version string on success, empty string on failure
- Respects `GPY_AGENT_ENABLED` flag (returns failure if agent disabled)

**Recommendation**: Cache version check result in global variable during initialization to avoid repeated IPC calls.

#### Breaking Change Examples

❌ **Breaking (Requires MAJOR bump)**:
- Changing `{"op":"git"}` to `{"operation":"git"}`
- Removing `cwd` field requirement
- Changing `ahead`/`behind` from `u32` to `string`

✅ **Non-Breaking (MINOR bump OK)**:
- Adding `{"op":"stats"}` new operation
- Adding optional `timeout` field to existing operations
- Adding new fields to responses (Fish ignores unknown fields)

#### Deprecation Policy

When deprecating IPC operations:
1. **Announce deprecation** 2 releases before removal
2. **Log warnings** when deprecated operations used
3. **Provide migration path** in documentation
4. **Remove only in MAJOR version** bump

---

## Function Naming Conventions

### Public API (Exported Functions)

Functions that users can call or override:

- **Prefix**: `segment_*` (for segments), `gpy_*` (for utilities)
- **Example**: `segment_git_render`, `gpy_config_path`

### Private/Internal Functions

Functions for internal use only:

- **Prefix**: `__gpy_*` (double underscore)
- **Example**: `__gpy_request`, `__gpy_ipc_send`, `__gpy_agent_available`

**Important**: Never call `__gpy_*` functions from custom segments or user scripts. These are implementation details subject to change.

### Segment Interface Functions

Every segment must implement two functions:

```fish
# Detection function: Returns 0 if segment should be shown, 1 otherwise
function segment_<name>_detect
    # Check if conditions are met for this segment
    # Example: Check if in git repo, if language files exist, etc.
    return 0  # Show segment
    # return 1  # Hide segment
end

# Render function: Outputs the segment content
function segment_<name>_render --argument-names is_last
    # is_last = "last" if this is the final segment in the prompt
    # is_last = "" (empty) otherwise

    # Generate and output segment content
    printf '%s' "$output"
end
```

---

## Environment Variable Contracts

### Configuration Variables (Set by User)

| Variable | Type | Default | Description |
|----------|------|---------|-------------|
| `GPY_AGENT_ENABLED` | `0` or `1` | `1` | Enable/disable agent daemon |
| `GPY_AGENT_SUPERVISOR_ENABLED` | `0` or `1` | `1` | Enable/disable agent supervisor |
| `GPY_GIT_ENABLED` | `0` or `1` | `1` | Enable/disable git segment |
| `GPY_LANGUAGE_ENABLED` | `0` or `1` | `1` | Enable/disable language segment (historical: the theme export now sets this from `config.toml` on every shell start; use `[language] enabled = false` there, #657) |
| `GPY_SHOW_DURATION` | `0` or `1` | `1` | Show command duration |
| `GPY_UI_SHOW_ICONS` | `0` or `1` | `1` | Show icons in segments |

### Runtime Variables (Set by GPY)

| Variable | Type | Scope | Description |
|----------|------|-------|-------------|
| `__gpy_registered` | `1` or unset | Global | Fish process is registered with agent |
| `__gpy_registered_pid` | PID | Global | PID of registered Fish process |
| `__gpy_last_workspace` | Path | Global | Last reported workspace path |
| `__gpy_repaint_trigger` | Counter | Global | Incremented to trigger prompt repaint |
| `__gpy_awaiting_git_update` | `1` or unset | Global | Waiting for SIGUSR1 after git command |

### Testing Variables (For Test Suites)

| Variable | Type | Description |
|----------|------|-------------|
| `GPY_AGENT_SOCKET_PATH` | Path | Override socket path |
| `GPY_TEST_LOG_FILE` | Path | Redirect agent logs to file |
| `GPY_DEBUG_SEGMENTS` | `1` or unset | Enable segment debug output |
| `GPY_SUPERVISOR_CHILD` | `1` or unset | Mark process as supervisor child |

### Theme Variables (Exported by Agent)

Theme variables are exported by running:

```fish
gpy-agent theme export --format fish | source
```

**Currently exported variables** (see [SP2 Shell Contract](#sp2-shell-contract-colour-palette-rendering-199) for context on what was removed):

```fish
# Clock (shell-rendered)
set -g __color_clock_bg "..."
set -g __color_clock_fg "..."
set -g __time_format "12"
set -g __clock_show_leading_zero "0"
set -g __clock_show_seconds "0"

# Duration detect gate (shell still gates visibility; render is agent-side)
set -g __duration_threshold_ms "100"

# Status segment (shell-rendered)
set -g __color_status_ok_bg "green"
set -g __color_status_ok_fg "black"
set -g __color_status_fail_bg "red"
set -g __color_status_fail_fg "black"
set -g __icon_status_ok "✔"
set -g __icon_status_fail "✖"

# Delimiter / powerline colors (shell renderer.fish uses these for transitions)
set -g __segment_delimiter_color "..."
set -g __segment_delimiter_bg "..."
set -g __prompt_open_color "..."
set -g __prompt_open_bg "..."
set -g __prompt_close_color "..."
set -g __prompt_close_bg "..."
set -g __prompt_base_bg "normal"
set -g __prompt_base_fg "normal"

# Plugin/custom segments (per-segment, keyed by segment name)
set -g __gpy_segment_<name>_bg_color "..."
set -g __gpy_segment_<name>_text_color "..."
set -g __gpy_segment_<name>_icon "..."
set -g __color_<name>_bg "..."   # kept for backward compat with community segments
set -g __color_<name>_fg "..."
```

**NOT exported** (retired in SP2 #199 — these segments are now fully agent-rendered):
- `__color_directory_bg/fg`
- `__color_duration_bg/fg`
- `__color_git_bg/fg` and per-element git colors
- `__color_language_bg/fg` and per-language color overrides
- `__gpy_directory_format`, `__gpy_duration_format`, `__gpy_character_format` toggles

**Naming Convention** (for shell-rendered segments still using color exports):
- Colors: `__color_<segment>_<property>`
- Icons: `__icon_<segment>_<state>`
- Delimiters: `__segment_delim_*`, `__prompt_open_*`, `__prompt_close_*`

---

## Error Handling Expectations

### IPC Error Handling

**Rule**: IPC failures should be silent and graceful. Never block the prompt or display error messages to the user.

```fish
# ✅ CORRECT: Silent failure with fallback
set -l response (__gpy_request git $PWD)
if test -z "$response"
    # Try oneshot fallback
    set response (gpy-agent oneshot git --cwd $PWD --format fish-ansi 2>/dev/null)
    if test -z "$response"
        # Skip segment silently
        return
    end
end

# ❌ WRONG: Blocking or displaying errors
set -l response (__gpy_request git $PWD); or begin
    echo "Error: Agent unavailable" >&2  # DON'T DO THIS
    return
end
```

### Agent Unavailable Scenarios

When the agent is unavailable:

1. **Data operations** (`git`, `lang`): Fall back to oneshot mode
2. **Lifecycle operations** (`register`, `workspace`): Skip silently
3. **Health checks** (`ping`): Return error status (no output)

### Segment Error Handling

**Rule**: Segments should gracefully handle missing data or failures.

```fish
function segment_git_render --argument-names is_last
    set -l response (__gpy_request git $PWD)

    # Check if response is empty or contains error
    if test -z "$response"; or string match -q '{"Error"*' -- "$response"
        # Skip segment silently (no output = segment not rendered)
        return
    end

    # Output pre-rendered content
    printf '%s' "$response"
end
```

### Logging for Debugging

Use the `__gpy_log_*` functions for debugging (not user-facing errors):

```fish
__gpy_log_debug "ipc" "Registered Fish PID %self with agent"
__gpy_log_warn "ipc" "Agent not responding, attempting restart"
__gpy_log_error "ipc" "Agent restart failed"
```

These logs are written to a file (not stderr) and are intended for debugging only.

---

## Segment Interface

### Required Functions

Every segment must implement:

1. **`segment_<name>_detect`**: Returns 0 if segment should render, 1 otherwise
2. **`segment_<name>_render`**: Outputs the segment content

### Detection Function Contract

```fish
function segment_<name>_detect
    # Return 0: Segment should be rendered
    # Return 1: Segment should be skipped

    # Example: Check if feature is enabled
    if test "$GPY_<NAME>_ENABLED" = 0
        return 1
    end

    # Example: Check if required command exists
    if not command -q some_command
        return 1
    end

    # Example: Check if in correct context
    if not test -f .some_file
        return 1
    end

    return 0
end
```

### Render Function Contract

```fish
function segment_<name>_render --argument-names is_last
    # Arguments:
    #   is_last = "last" if this is the final segment in the prompt
    #   is_last = "" (empty) otherwise

    # Get data (via IPC or direct computation)
    set -l data (get_segment_data)

    # Handle empty/error responses
    if test -z "$data"
        return  # Skip segment silently
    end

    # For IPC-based segments, check for agent-rendered output
    if string match -q '\x1b[*' -- "$data"
        # Agent returned pre-rendered ANSI
        printf '%s' "$data"
        return
    end

    # Otherwise, render using Fish
    set -l output (render_segment_content $data)
    printf '%s' "$output"
end
```

### IPC-Based Segments (Recommended)

For segments that need agent data (git, language):

```fish
function segment_git_render --argument-names is_last
    # Convert is_last to boolean string
    set -l is_last_flag ""
    if test "$is_last" = last
        set is_last_flag true
    end

    # Request pre-rendered output from agent
    set -l rendered (__gpy_request git $PWD "$is_last_flag")

    # Handle errors
    if test -z "$rendered"; or string match -q '{"Error"*' -- "$rendered"
        return
    end

    # Output pre-rendered content (agent handles delimiters)
    printf '%s' "$rendered"
end
```

### Shell-Rendered Segments (Clock, Status, Plugin/Custom)

For segments that render entirely in the shell (clock, status, plugin/custom segments):

```fish
function segment_clock_render --argument-names is_last
    set -l now (string trim (date "+%H:%M"))

    # Use theme variables exported by the agent for shell-rendered segments
    set -l bg $__color_clock_bg
    set -l fg $__color_clock_fg
    gpy_section_standalone $bg $fg "$now" $is_last
end
```

> **Note**: As of SP2 (#199), `directory` is no longer shell-rendered. Its former
> use of `__color_directory_bg/fg` is a historical example only. See the
> [SP2 Shell Contract](#sp2-shell-contract-colour-palette-rendering-199) section for the full list.

---

## Signal Handling

### SIGUSR1 - Live Updates

**Purpose**: Notify Fish shells that prompt data has changed (git status, time, etc.)

**Sent by**: Agent daemon (via file watcher or clock updates)

**Handled by**: `__gpy_sigusr1_handler` function

**Behavior**:
1. Clear `__gpy_awaiting_git_update` flag
2. Increment `__gpy_repaint_trigger` counter
3. Trigger `force-repaint` via `--on-variable` handler

**Implementation**:

```fish
function __gpy_sigusr1_handler --on-signal SIGUSR1
    set -e __gpy_awaiting_git_update
    set -g __gpy_repaint_trigger (math (set -q __gpy_repaint_trigger; and echo $__gpy_repaint_trigger; or echo 0) + 1)
end

function __gpy_repaint_on_variable --on-variable __gpy_repaint_trigger
    commandline -f force-repaint 2>/dev/null
end
```

### SIGUSR2 - Configuration Reload

**Purpose**: Notify Fish shells that configuration or theme has changed

**Sent by**: Agent daemon (via file watcher on config/theme files)

**Handled by**: `__gpy_sigusr2_handler` function

**Behavior**:
1. Re-export theme variables from agent
2. Clear language detection caches
3. Increment `__gpy_repaint_trigger` counter

**Implementation**:

```fish
function __gpy_sigusr2_handler --on-signal SIGUSR2
    # Reload theme variables
    gpy-agent theme export --format fish 2>/dev/null | source

    # Clear language caches
    for var in (set -n | string match '__gpy_lang_cache_*')
        set -e $var
    end

    # Trigger repaint
    set -g __gpy_repaint_trigger (math (set -q __gpy_repaint_trigger; and echo $__gpy_repaint_trigger; or echo 0) + 1)
end
```

---

## Agent Lifecycle Management

### Registration

**When**: On first prompt after shell startup
**Event**: `fish_prompt`
**Function**: `__gpy_register_with_agent`

**Purpose**: Register the Fish process with the agent to receive SIGUSR1/SIGUSR2 signals

```fish
function __gpy_register_with_agent
    set -l response (__gpy_request register)
    if test "$response" = '"Ack"'
        set -g __gpy_registered 1
        set -g __gpy_registered_pid %self
        return 0
    end
    return 1
end
```

### Workspace Synchronization

**When**: On every prompt
**Event**: `fish_prompt`
**Function**: `__gpy_sync_workspace`

**Purpose**: Notify agent of current working directory changes

```fish
function __gpy_sync_workspace
    if not set -q __gpy_registered
        return 0
    end

    set -l cwd (pwd)
    set -l response (__gpy_request workspace $cwd)

    # Handle agent restart (registration lost)
    if string match -q '*"error"*' -- $response; or string match -q '*not registered*' -- $response
        set -e __gpy_registered
        __gpy_register_with_agent
    end
end
```

### Unregistration

**When**: On shell exit
**Event**: `fish_exit`
**Function**: `__gpy_unregister_from_agent`

**Purpose**: Clean up agent resources when shell exits

```fish
function __gpy_unregister_from_agent
    __gpy_request unregister >/dev/null 2>&1
    set -e __gpy_registered
    set -e __gpy_registered_pid
end
```

### Agent Supervision

**Purpose**: Automatically restart the agent if it crashes or becomes unresponsive

**Enabled by**: `$GPY_AGENT_SUPERVISOR_ENABLED` (default: 1)
**Check Interval**: `$GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS` (default: 30)
**Max Restarts**: `$GPY_AGENT_SUPERVISOR_MAX_RESTART_ATTEMPTS` (default: 3)
**Rate Limit**: `$GPY_SUPERVISOR_RESTART_RATE_LIMIT_SECONDS` (default: 60)

**Implementation**: Background loop in detached Fish process

---

## Best Practices

### 1. Never Block the Prompt

```fish
# ❌ WRONG: Blocking sleep or synchronous wait
sleep 1
set -l response (__gpy_request git $PWD)

# ✅ CORRECT: Use timeouts and fail fast
set -l response (__gpy_request git $PWD)  # Has built-in timeout
if test -z "$response"
    return  # Skip segment
end
```

### 2. Handle Agent Restarts Gracefully

The agent may restart at any time (crashes, upgrades, etc.). Always handle re-registration:

```fish
set -l response (__gpy_request workspace $cwd)
if string match -q '*not registered*' -- $response
    # Agent restarted and lost our registration
    set -e __gpy_registered
    __gpy_register_with_agent
    # Retry the request
    set response (__gpy_request workspace $cwd)
end
```

### 3. Use Oneshot Fallback for Data Operations

```fish
set -l response (__gpy_request git $PWD)
if test -z "$response"
    # IPC failed, try oneshot
    set response (gpy-agent oneshot git --cwd $PWD --format fish-ansi 2>/dev/null)
end
```

### 4. Test with Agent Disabled

Always test your segments with `GPY_AGENT_ENABLED=0` to ensure graceful degradation:

```fish
GPY_AGENT_ENABLED=0 fish
```

### 5. Use Theme Variables (Shell-Rendered Segments Only)

For shell-rendered segments (clock, status, plugin/custom), never hardcode colors — use theme variables exported by the agent:

```fish
# ❌ WRONG: Hardcoded colors in a shell-rendered segment
printf '\x1b[38;2;100;149;237m%s\x1b[0m' "$now"

# ✅ CORRECT: Theme variables (clock segment example)
set -l bg $__color_clock_bg
set -l fg $__color_clock_fg
gpy_section_standalone $bg $fg "$now" $is_last
```

For agent-rendered segments (directory, duration, character, git, language), the agent applies the theme template directly — the shell simply outputs the pre-rendered ANSI string received from the agent:

```fish
# ✅ CORRECT: Pass-through for agent-rendered segments (directory example)
printf '%s' (__gpy_request directory "$PWD" $is_last_flag)
```

Color variables such as `__color_git_bg` and `__color_directory_bg` no longer exist in the theme export (removed SP2 #199).

---

## Reference Implementation

See these files for complete examples:

- **IPC Core**: `core/ipc.fish`
- **Git Segment**: `segments/git.fish`
- **Language Segment**: `segments/language.fish`
- **Rendering Utilities**: `core/renderer.fish`
- **Theme Helpers**: `core/util.fish`

---

## Testing

When adding or modifying Fish integration:

1. **Unit Tests**: Add tests to `tests/fish/*.test.fish`
2. **Integration Tests**: Add E2E tests to `tests/fish/e2e_*.test.fish`
3. **Interactive Sessions**: `fish tests/fish/e2e_interactive_session.test.fish` drives real pty sessions (the manual live-updates script it replaced is gone)

**Test Coverage Requirements**:
- All IPC operations must have tests
- All signal handlers must have tests
- All error paths must have tests
- Agent restart scenarios must have tests

---

## Versioning

This contract follows the agent's protocol version (currently version 1).

**Breaking Changes** (require MAJOR version bump):
- Changing message format
- Removing IPC operations
- Changing signal behavior
- Changing environment variable names

**Non-Breaking Changes** (MINOR/PATCH):
- Adding new IPC operations
- Adding new environment variables
- Improving error handling
- Performance optimizations

---

## SP2 Shell Contract (Colour-Palette Rendering, #199)

SP2 threaded `config.ui.palette` → `PaletteManager` → `to_template_palette()` → template engine, making template+palette the sole colour path for agent-rendered segments. As part of that work the shell contract was revised to reflect which segments render in the agent vs in the shell.

### Agent-Rendered Segments

The following segments arrive as pre-formatted ANSI from the agent (or from the instant-prompt cache). The shell simply `printf '%s'` the output and does **no local colour work**:

| Segment | Notes |
|---------|-------|
| `directory` | Always agent-rendered (#199). Agent applies theme template + palette and returns ANSI. |
| `duration` | Agent-rendered for the formatted string; shell detect gate (`__duration_threshold_ms`) still controls visibility. |
| `character` / prompt symbol | Agent-rendered for non-root users; falls back to legacy `set_color` path when the agent is unavailable or the user is root. |
| `git` | Agent-rendered via the instant-prompt cache (serve-stale pattern). |
| `language` | Agent-rendered via the instant-prompt cache (serve-stale pattern). |

**Shell-side colour variables removed for these segments (SP2 #199):**
- `__color_directory_bg`, `__color_directory_fg`
- `__color_duration_bg`, `__color_duration_fg`
- `__color_git_bg`, `__color_git_fg`, and per-element git colours
- `__color_language_bg`, `__color_language_fg`, and per-language colour overrides

**Format-toggle variables removed (SP2 #199):**
- `__gpy_directory_format`
- `__gpy_duration_format`
- `__gpy_character_format`

These are no longer emitted by `gpy-agent theme export`. Shells that reference them will silently get empty variables (Fish) or unset variables (Zsh/Bash), which may cause unexpected output. Remove references to them.

### Shell-Rendered Segments (Kept)

The following segments have **no agent render path** and remain fully shell-rendered. Their `__color_*` exports continue to be emitted by `gpy-agent theme export`:

| Segment | Colour exports |
|---------|---------------|
| `clock` | `__color_clock_bg`, `__color_clock_fg` |
| `status` (exit code) | `__color_status_ok_bg/fg`, `__color_status_fail_bg/fg` |
| Plugin / custom segments | `__gpy_segment_<name>_bg_color`, `__gpy_segment_<name>_text_color`, `__color_<name>_bg`, `__color_<name>_fg` |

**Delimiter / powerline layout colours** are also kept — `fish/core/renderer.fish` drives powerline transitions with them:
- `__segment_delimiter_color`, `__segment_delimiter_bg`
- `__prompt_open_color`, `__prompt_open_bg`
- `__prompt_close_color`, `__prompt_close_bg`
- `__prompt_base_bg`, `__prompt_base_fg`

### Runtime-Gated Status Indicator

`GPY_SHOW_STATUS` now gates the standalone exit-code indicator on whether the prompt character was successfully agent-rendered. When the agent renders the character (which is tinted by exit status via palette, Starship-style), the separate indicator is suppressed to avoid redundancy. When the agent is unavailable and the legacy fallback fires, `GPY_SHOW_STATUS` restores the indicator.

### Zsh and Bash

Zsh (`zsh/`) and Bash (`bash/`) follow the same contract. Their `directory`, `duration`, `git`, and `language` segments delegate to `__gpy_request` and emit the agent-provided ANSI string unchanged. `clock` and `status` remain shell-rendered with their `__color_*` variables. The same format toggles were removed from the Zsh/Bash theme export.

---

## Support

For questions or clarification on this contract:

1. Read the reference implementations in `core/` and `segments/`
2. Check the protocol schemas in `gpy-agent/schemas/`
3. Review the test suite in `tests/fish/`
4. Open an issue on GitHub with the `fish-integration` label

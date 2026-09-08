# Zsh Support Specification for GPY

> **Archived.** This is a historical specification for Zsh support. Refer to the current `zsh/` directory and active shell integration code for current Zsh implementation.

**Version**: 1.0
**Date**: 2025-11-23
**Status**: Draft

---

## 1. Executive Summary

This document specifies the implementation of Zsh shell support for GPY (Guppy Prompt, Yay!). The goal is to bring GPY's innovative features—daemon-based caching, live prompt updates via file watching, and sub-millisecond response times—to Zsh users, who represent approximately 30% of the shell market.

### 1.1 Goals

1. **Feature Parity**: Match Fish implementation capabilities where Zsh supports them
2. **Consistent UX**: Same prompt appearance and behavior across shells
3. **Shared Backend**: Leverage existing `gpy-agent` Rust daemon without modifications
4. **Native Feel**: Use idiomatic Zsh patterns, not Fish-style workarounds

### 1.2 Non-Goals

1. Oh-My-Zsh/Prezto framework integration (future work)
2. Powerlevel10k migration tooling
3. Zsh version < 5.8 support

---

## 2. Architecture Overview

```
┌─────────────────────────────────────────────────────────────────┐
│                         GPY System                              │
├─────────────────────────────────────────────────────────────────┤
│                                                                 │
│  ┌──────────────┐    ┌──────────────┐    ┌──────────────┐      │
│  │  Fish Shell  │    │  Zsh Shell   │    │ Bash Shell   │      │
│  │  Integration │    │  Integration │    │ (Future)     │      │
│  │  (~70 files) │    │  (NEW)       │    │              │      │
│  └──────┬───────┘    └──────┬───────┘    └──────────────┘      │
│         │                   │                                   │
│         │    Unix Socket IPC (JSON Protocol)                    │
│         │                   │                                   │
│         ▼                   ▼                                   │
│  ┌─────────────────────────────────────────────────────────┐   │
│  │                    gpy-agent (Rust)                      │   │
│  │  • Git status (gix)      • Theme rendering               │   │
│  │  • Language detection    • File watching                 │   │
│  │  • Config management     • Client registry               │   │
│  │  • SIGUSR1/2 signaling   • Caching layer                 │   │
│  └─────────────────────────────────────────────────────────┘   │
│                                                                 │
└─────────────────────────────────────────────────────────────────┘
```

The Rust agent remains unchanged. Zsh integration communicates via the same JSON-over-Unix-socket protocol.

---

## 3. Feature Mapping: Fish → Zsh

| Feature | Fish Implementation | Zsh Implementation |
|---------|--------------------|--------------------|
| Prompt function | `fish_prompt` | `precmd` + `PROMPT` |
| Right prompt | `fish_right_prompt` | `RPROMPT` |
| Global variables | `set -g VAR value` | `typeset -g VAR=value` |
| Export variables | `set -gx VAR value` | `export VAR=value` |
| Signal handlers | `--on-signal SIGUSR1` | `TRAPUSR1()` function |
| Exit hook | `--on-event fish_exit` | `zshexit` hook |
| Post-command hook | `--on-event fish_postexec` | `precmd` / `preexec` |
| Pre-command hook | `--on-event fish_preexec` | `preexec` |
| Prompt repaint | `commandline -f force-repaint` | `zle reset-prompt` |
| Function existence | `functions -q name` | `(( $+functions[name] ))` |
| Current PID | `%self` | `$$` |
| Background jobs | `command &; disown` | `command &!` or `command & disown` |
| String regex | `string match -r` | `[[ $str =~ regex ]]` |
| Colors | `set_color red` | `%F{red}` or `$fg[red]` |
| Check command | `command -q cmd` | `(( $+commands[cmd] ))` |

---

## 4. File Structure

```
gpy/
├── zsh/                          # NEW: Zsh integration
│   ├── gpy.zsh                   # Main entry point (sourced by user)
│   ├── core/
│   │   ├── init.zsh              # Initialization and setup
│   │   ├── ipc.zsh               # IPC communication with agent
│   │   ├── constants.zsh         # Default values and constants
│   │   ├── renderer.zsh          # Powerline segment rendering
│   │   ├── util.zsh              # Logging and helpers
│   │   └── signals.zsh           # SIGUSR1/SIGUSR2 handlers
│   ├── segments/
│   │   ├── git.zsh               # Git segment
│   │   ├── directory.zsh         # Directory segment
│   │   ├── duration.zsh          # Command duration segment
│   │   ├── clock.zsh             # Clock segment
│   │   ├── language.zsh          # Language detection segment
│   │   └── status.zsh            # Exit status segment
│   └── themes/                   # Zsh-specific theme helpers (if needed)
├── install-zsh.sh                # Zsh installer script (bash for portability)
└── ...existing files...
```

**Estimated line counts:**
- `core/*.zsh`: ~600 lines total
- `segments/*.zsh`: ~200 lines total
- `gpy.zsh` + installer: ~100 lines
- **Total**: ~900 lines (vs ~1500 for Fish, which includes more test code)

---

## 5. Component Specifications

### 5.1 Entry Point (`gpy.zsh`)

```zsh
# gpy.zsh - Main entry point
# Usage: source ~/.config/gpy/zsh/gpy.zsh

# Prevent double-sourcing
[[ -n "$GPY_LOADED" ]] && return 0
typeset -g GPY_LOADED=1

# Determine GPY root directory
typeset -g GPY_ROOT="${0:A:h}"

# Source core modules in order
source "$GPY_ROOT/core/constants.zsh"
source "$GPY_ROOT/core/util.zsh"
source "$GPY_ROOT/core/ipc.zsh"
source "$GPY_ROOT/core/renderer.zsh"
source "$GPY_ROOT/core/signals.zsh"
source "$GPY_ROOT/core/init.zsh"
```

### 5.2 IPC Module (`core/ipc.zsh`)

**Purpose**: Communicate with gpy-agent via Unix socket.

```zsh
# core/ipc.zsh

# Get the socket path (same logic as Fish version)
__gpy_ipc_endpoint() {
    # Allow override for testing
    [[ -n "$GPY_AGENT_SOCKET_PATH" ]] && { echo "$GPY_AGENT_SOCKET_PATH"; return }

    local root
    if [[ -n "$XDG_RUNTIME_DIR" ]]; then
        root="$XDG_RUNTIME_DIR/gpy"
    elif [[ -n "$XDG_CACHE_HOME" ]]; then
        root="$XDG_CACHE_HOME/gpy"
    elif [[ -n "$HOME" ]]; then
        root="$HOME/.cache/gpy"
    else
        root="/tmp/gpy"
    fi
    echo "$root/gpy.sock"
}

# Send IPC request with timeout
# Returns: JSON response on stdout, or empty on failure
__gpy_ipc_send() {
    local payload="$1"
    local timeout_ms="${2:-$GPY_IPC_TIMEOUT_MS}"
    local sock="$(__gpy_ipc_endpoint)"

    # Pre-flight check
    [[ -S "$sock" ]] || return 1

    local timeout_secs=$(( (timeout_ms + 999) / 1000 ))

    if (( $+commands[socat] )); then
        echo "$payload" | socat -T "$timeout_secs" - "UNIX-CONNECT:$sock" 2>/dev/null | head -n1
    elif (( $+commands[nc] )); then
        # Check for -U support (Unix socket)
        if nc -h 2>&1 | grep -q '\-U'; then
            if (( $+commands[timeout] )); then
                echo "$payload" | timeout "${timeout_secs}s" nc -U "$sock" 2>/dev/null | head -n1
            else
                echo "$payload" | nc -U "$sock" 2>/dev/null | head -n1
            fi
        fi
    elif zmodload -e zsh/net/socket 2>/dev/null || zmodload zsh/net/socket 2>/dev/null; then
        # Pure Zsh fallback using zsh/net/socket module
        local fd
        if zsocket "$sock" 2>/dev/null; then
            fd=$REPLY
            print -u $fd "$payload"
            read -u $fd -t $timeout_secs line
            exec {fd}>&-
            echo "$line"
        fi
    fi
}

# JSON escape helper
__gpy_json_escape() {
    local str="$1"
    str="${str//\\/\\\\}"    # Backslash
    str="${str//\"/\\\"}"    # Quote
    str="${str//$'\n'/\\n}"  # Newline
    str="${str//$'\r'/\\r}"  # Carriage return
    str="${str//$'\t'/\\t}"  # Tab
    echo "$str"
}

# High-level request function
__gpy_request() {
    local op="$1"
    local cwd="${2:-$PWD}"
    local is_last="$3"
    local payload
    local escaped_cwd="$(__gpy_json_escape "$cwd")"

    case "$op" in
        ping)
            payload='{"op":"ping"}'
            ;;
        register)
            payload="{\"op\":\"register\",\"pid\":$$,\"cwd\":\"$escaped_cwd\"}"
            ;;
        unregister)
            payload="{\"op\":\"unregister\",\"pid\":$$}"
            ;;
        git|lang)
            local format="zsh-ansi"  # Or fish-ansi if compatible
            if [[ "$is_last" == "true" ]]; then
                payload="{\"op\":\"$op\",\"cwd\":\"$escaped_cwd\",\"format\":\"$format\",\"is_last\":true}"
            else
                payload="{\"op\":\"$op\",\"cwd\":\"$escaped_cwd\",\"format\":\"$format\"}"
            fi
            ;;
        workspace)
            payload="{\"op\":\"workspace\",\"pid\":$$,\"cwd\":\"$escaped_cwd\"}"
            ;;
        status)
            payload='{"op":"status"}'
            ;;
        *)
            return 1
            ;;
    esac

    local result
    result="$(__gpy_ipc_send "$payload" "$GPY_IPC_TIMEOUT_MS")"

    # Fallback to oneshot for git/lang if IPC failed
    if [[ -z "$result" ]] && (( $+commands[gpy-agent] )); then
        case "$op" in
            git|lang)
                if [[ "$is_last" == "true" ]]; then
                    result="$(gpy-agent oneshot "$op" --cwd "$cwd" --format "$format" 2>/dev/null)"
                else
                    result="$(gpy-agent oneshot "$op" --cwd "$cwd" --format "$format" --not-last 2>/dev/null)"
                fi
                ;;
        esac
    fi

    echo "$result"
}
```

### 5.3 Signal Handlers (`core/signals.zsh`)

**Purpose**: Handle SIGUSR1 (git updates) and SIGUSR2 (config reload).

```zsh
# core/signals.zsh

# SIGUSR1: Agent detected git/file changes, repaint prompt
TRAPUSR1() {
    # Clear async update flag if set
    unset __gpy_awaiting_git_update

    # Only repaint if we're at a prompt (not mid-command)
    if [[ -o zle ]]; then
        zle reset-prompt
    fi
}

# SIGUSR2: Agent reloaded config/theme, reload shell-side variables
TRAPUSR2() {
    # Reload theme variables from agent
    if (( $+commands[gpy-agent] )); then
        eval "$(gpy-agent theme export --format zsh 2>/dev/null)"
    fi

    # Clear language detection caches
    unset ${(Mk)parameters:#__gpy_lang_cache_*}

    # Repaint prompt
    if [[ -o zle ]]; then
        zle reset-prompt
    fi
}

# Register signal handlers
# Note: TRAP* functions are automatically called by Zsh
```

### 5.4 Prompt Rendering (`core/init.zsh`)

**Purpose**: Define precmd/preexec hooks and prompt generation.

```zsh
# core/init.zsh

# Enable prompt command substitution (REQUIRED for dynamic prompt)
setopt prompt_subst

# Track command duration (use -F for float precision)
typeset -gF __gpy_cmd_start_time=0
typeset -gF __gpy_cmd_duration=0

# preexec: Called before each command executes
__gpy_preexec() {
    __gpy_cmd_start_time=$EPOCHREALTIME
}

# precmd: Called before each prompt is displayed
__gpy_precmd() {
    local last_status=$?

    # Calculate command duration
    if (( __gpy_cmd_start_time > 0 )); then
        __gpy_cmd_duration=$(( (EPOCHREALTIME - __gpy_cmd_start_time) * 1000 ))
        __gpy_cmd_start_time=0
    else
        __gpy_cmd_duration=0
    fi

    # Sync workspace with agent
    __gpy_sync_workspace

    # Store last status for prompt
    typeset -g __gpy_last_status=$last_status
}

# Build the prompt string
__gpy_build_prompt() {
    local segments=()
    local segment_count=0
    local output=""

    # Detect which segments should render
    for segment in ${__enabled_segments[@]}; do
        if typeset -f "segment_${segment}_detect" > /dev/null && "segment_${segment}_detect"; then
            segments+=("$segment")
        fi
    done

    segment_count=${#segments[@]}
    local idx=1

    # Reset segment position tracker
    typeset -g __gpy_segment_position="first"

    # Render each segment
    for segment in "${segments[@]}"; do
        local is_last=""
        (( idx == segment_count )) && is_last="last"

        if typeset -f "segment_${segment}_render" > /dev/null; then
            output+="$(segment_${segment}_render "$is_last")"
        fi

        (( idx++ ))
    done

    echo "$output"
}

# The actual prompt
__gpy_prompt() {
    # Newline for visual separation
    print ""

    # Render segments
    __gpy_build_prompt

    # Final prompt line with status indicator
    print ""

    if (( GPY_SHOW_STATUS )); then
        if (( __gpy_last_status == 0 )); then
            print -n "%F{green}${__icon_status_ok}%f "
        else
            print -n "%F{red}${__icon_status_fail}%f "
        fi
    fi

    # Prompt symbol
    if (( EUID == 0 )); then
        print -n "%F{${__root_prompt_color}}${__icon_root_prompt}%f "
    else
        print -n "%F{${__prompt_color}}${__icon_prompt}%f "
    fi
}

# Register hooks
autoload -Uz add-zsh-hook
add-zsh-hook preexec __gpy_preexec
add-zsh-hook precmd __gpy_precmd

# Register with agent on first prompt (lazy initialization)
__gpy_init_registration() {
    # Only run once
    [[ -n "$__gpy_init_done" ]] && return 0
    typeset -g __gpy_init_done=1

    # Register with agent if enabled
    if (( GPY_AGENT_ENABLED )); then
        __gpy_register_with_agent
    fi
}
add-zsh-hook precmd __gpy_init_registration

# Set PROMPT to call our function
# Single quotes + prompt_subst = dynamic evaluation each prompt
PROMPT='$(__gpy_prompt)'

# Optional: Right prompt
# RPROMPT='$(__gpy_right_prompt)'
```

### 5.5 Workspace Sync (in `core/ipc.zsh`)

The `__gpy_sync_workspace` function notifies the agent of directory changes:

```zsh
# Sync current working directory with agent (called from precmd)
__gpy_sync_workspace() {
    # Skip if not registered or agent disabled
    [[ -z "$__gpy_registered" ]] && return 0
    (( GPY_AGENT_ENABLED == 0 )) && return 0

    local cwd="$PWD"

    # Send workspace update to agent
    local response
    response="$(__gpy_request workspace "$cwd")"

    # If agent lost our registration (e.g., restarted), re-register
    if [[ "$response" == *'"error"'* ]] || [[ "$response" == *'not registered'* ]]; then
        unset __gpy_registered
        __gpy_register_with_agent
    fi
}

# Register this shell with the agent
__gpy_register_with_agent() {
    (( GPY_AGENT_ENABLED == 0 )) && return 1

    local response
    response="$(__gpy_request register)"

    if [[ "$response" == *'"status":"ok"'* ]]; then
        typeset -g __gpy_registered=1
        __gpy_sync_workspace
        return 0
    fi

    return 1
}
```

### 5.6 Constants (`core/constants.zsh`)

Default values and enabled segments:

```zsh
# core/constants.zsh

# Default enabled segments (can be overridden by theme export)
typeset -ga __enabled_segments
: ${__enabled_segments:=(clock duration language directory git)}

# IPC settings
typeset -g GPY_IPC_TIMEOUT_MS=${GPY_IPC_TIMEOUT_MS:-200}
typeset -g GPY_SOCKET_WAIT_MAX_ATTEMPTS=${GPY_SOCKET_WAIT_MAX_ATTEMPTS:-20}

# Feature toggles
typeset -g GPY_AGENT_ENABLED=${GPY_AGENT_ENABLED:-1}
typeset -g GPY_AGENT_SUPERVISOR_ENABLED=${GPY_AGENT_SUPERVISOR_ENABLED:-1}
typeset -g GPY_SHOW_STATUS=${GPY_SHOW_STATUS:-1}
typeset -g GPY_GIT_ENABLED=${GPY_GIT_ENABLED:-1}

# Duration threshold (ms) - only show if command took longer
typeset -g __duration_threshold=${__duration_threshold:-100}

# Default icons (overridden by theme export)
typeset -g __icon_status_ok=${__icon_status_ok:-"✓"}
typeset -g __icon_status_fail=${__icon_status_fail:-"✗"}
typeset -g __icon_prompt=${__icon_prompt:-"❯"}
typeset -g __icon_root_prompt=${__icon_root_prompt:-"#"}

# Default colors (overridden by theme export)
typeset -g __prompt_color=${__prompt_color:-green}
typeset -g __root_prompt_color=${__root_prompt_color:-red}
```

### 5.7 Segment Example: Git (`segments/git.zsh`)

```zsh
# segments/git.zsh

segment_git_detect() {
    # Quick check: disabled by config?
    [[ "$GPY_GIT_ENABLED" == "0" ]] && return 1

    # Look for .git directory
    local dir="$PWD"
    while [[ "$dir" != "/" ]]; do
        [[ -d "$dir/.git" ]] && return 0
        dir="${dir:h}"
    done

    return 1
}

segment_git_render() {
    local is_last="$1"
    local is_last_flag=""
    [[ "$is_last" == "last" ]] && is_last_flag="true"

    # Get pre-rendered output from agent
    local rendered_output
    rendered_output="$(__gpy_request git "$PWD" "$is_last_flag")"

    [[ -z "$rendered_output" ]] && return

    # Check for error response
    [[ "$rendered_output" == '{"Error"'* ]] && return

    # Output pre-rendered ANSI sequences
    print -n "$rendered_output"
}
```

### 5.8 Renderer (`core/renderer.zsh`)

```zsh
# core/renderer.zsh

# Segment position tracking
typeset -g __gpy_segment_position="first"

# Start a segment with powerline-style transition
gpy_section_start() {
    local bg="$1" fg="$2" icon="$3"
    local start_delim delim_fg delim_bg

    case "$__gpy_segment_position" in
        first)
            start_delim="$__segment_delim_first"
            delim_fg="$__prompt_open_color"
            delim_bg="$__prompt_open_bg"
            ;;
        *)
            start_delim="$__segment_delim_start"
            delim_fg="$__segment_delimiter_color"
            delim_bg="$__segment_delimiter_bg"
            ;;
    esac

    # Resolve magic keywords
    [[ "$delim_fg" == "match_bg" ]] && delim_fg="$bg"
    [[ "$delim_bg" == "match_bg" ]] && delim_bg="$bg"

    # Set defaults
    : ${bg:=default}
    : ${delim_fg:=default}
    : ${delim_bg:=default}

    # Render delimiter
    if [[ -n "$start_delim" ]]; then
        print -n "%K{$delim_bg}%F{$delim_fg}${start_delim}%f%k"
    fi

    # Render icon on segment background
    print -n "%K{$bg}%F{$fg}"
    [[ -n "$icon" ]] && print -n "$icon"

    # Update position
    [[ "$__gpy_segment_position" == "first" ]] && __gpy_segment_position="middle"
}

gpy_section_end() {
    local bg="$1" is_last="$2"
    local end_delim delim_fg delim_bg

    if [[ "$is_last" == "last" ]]; then
        end_delim="$__segment_delim_last"
        delim_fg="$__prompt_close_color"
        delim_bg="$__prompt_close_bg"
    else
        end_delim="$__segment_delim_end"
        delim_fg="$__segment_delimiter_color"
        delim_bg="$__segment_delimiter_bg"
    fi

    # Resolve magic keywords
    [[ "$delim_fg" == "match_bg" ]] && delim_fg="$bg"
    [[ "$delim_bg" == "match_bg" ]] && delim_bg="$bg"

    : ${delim_fg:=default}
    : ${delim_bg:=default}

    # Render end delimiter
    if [[ -n "$end_delim" ]]; then
        print -n "%K{$delim_bg}%F{$delim_fg}${end_delim}%f%k"
    fi

    print -n "%f%k"
}

gpy_section_append() {
    local bg="$1" fg="$2" content="$3"
    print -n "%K{$bg}%F{$fg}${content}%f%k"
}
```

---

## 6. Agent Modifications

### 6.1 New Output Format

The agent needs a new `--format zsh` option for theme export. Most of the work is simple syntax changes:

**Fish output:**
```fish
set -g __prompt_color "green"
set -g __icon_prompt "❯"
```

**Zsh output:**
```zsh
typeset -g __prompt_color="green"
typeset -g __icon_prompt="❯"
```

### 6.2 Format Options

| Format | Description | Used By |
|--------|-------------|---------|
| `fish` | Fish shell variables | `gpy-agent theme export --format fish` |
| `fish-ansi` | Pre-rendered ANSI for Fish | Git/lang segment rendering |
| `zsh` | Zsh shell variables | `gpy-agent theme export --format zsh` |
| `zsh-ansi` | Pre-rendered ANSI for Zsh | Git/lang segment rendering (may be same as fish-ansi) |
| `json` | Raw JSON | Debugging, external tools |

**Note**: `fish-ansi` and `zsh-ansi` may be identical since both shells handle raw ANSI escape sequences the same way. We can alias them.

### 6.3 Required Changes to `gpy-agent`

1. **`src/cli/theme.rs`**: Add `--format zsh` option to theme export
2. **`src/rendering/format.rs`**: Add Zsh variable formatting
3. **`src/ipc/protocol.rs`**: Accept `"zsh-ansi"` format (can map to `fish-ansi` internally)

Estimated: ~50 lines of Rust code.

---

## 7. Installation

### 7.1 User Installation Steps

```zsh
# Option 1: curl installer (recommended)
curl -fsSL https://raw.githubusercontent.com/jpease/gpy/main/install-zsh.sh | zsh

# Option 2: Manual
git clone https://github.com/jpease/gpy ~/.config/gpy
echo 'source ~/.config/gpy/zsh/gpy.zsh' >> ~/.zshrc
```

### 7.2 Installer Script (`install-zsh.sh`)

```bash
#!/bin/bash
set -e

GPY_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/gpy"
ZSHRC="${ZDOTDIR:-$HOME}/.zshrc"

echo "Installing GPY for Zsh..."

# Clone or update repository
if [[ -d "$GPY_DIR" ]]; then
    echo "Updating existing installation..."
    git -C "$GPY_DIR" pull --ff-only
else
    echo "Cloning GPY..."
    git clone https://github.com/jpease/gpy "$GPY_DIR"
fi

# Build agent
echo "Building gpy-agent..."
cargo build --release --manifest-path "$GPY_DIR/gpy-agent/Cargo.toml"

# Install binary
mkdir -p "$HOME/.local/bin"
cp "$GPY_DIR/gpy-agent/target/release/gpy-agent" "$HOME/.local/bin/"

# Add to .zshrc if not present
if ! grep -q 'gpy/zsh/gpy.zsh' "$ZSHRC" 2>/dev/null; then
    echo "" >> "$ZSHRC"
    echo "# GPY Prompt" >> "$ZSHRC"
    echo "source \"$GPY_DIR/zsh/gpy.zsh\"" >> "$ZSHRC"
    echo "Added GPY to $ZSHRC"
else
    echo "GPY already in $ZSHRC"
fi

echo ""
echo "Installation complete! Restart your shell or run: source ~/.zshrc"
```

---

## 8. Testing Strategy

### 8.1 Unit Tests (Zsh)

Use [zunit](https://github.com/zunit-zsh/zunit) or simple assertion scripts:

```zsh
# tests/zsh/ipc.test.zsh

@test "__gpy_json_escape escapes quotes" {
    local result="$(__gpy_json_escape 'hello "world"')"
    assert equal "$result" 'hello \"world\"'
}

@test "__gpy_ipc_endpoint returns socket path" {
    local result="$(__gpy_ipc_endpoint)"
    assert match "$result" '*.sock'
}
```

### 8.2 Integration Tests

```zsh
# tests/zsh/e2e_prompt.test.zsh

@test "prompt renders without errors" {
    # Source GPY
    source "$GPY_ROOT/zsh/gpy.zsh"

    # Generate prompt
    local output
    output="$(__gpy_prompt 2>&1)"

    # Should not contain error messages
    assert not_match "$output" "error"
    assert not_match "$output" "Error"
}

@test "SIGUSR1 triggers prompt repaint" {
    # This requires a mock or actual agent
    source "$GPY_ROOT/zsh/gpy.zsh"

    # Capture repaint count
    typeset -g __test_repaint_count=0
    zle() { [[ "$1" == "reset-prompt" ]] && (( __test_repaint_count++ )) }

    # Send signal
    kill -USR1 $$

    assert equal "$__test_repaint_count" "1"
}
```

### 8.3 Cross-Shell Consistency Tests

```bash
# tests/cross-shell/visual_consistency.sh

# Run both shells, capture prompt output, compare
fish -c 'source ~/.config/gpy/core/init.fish; fish_prompt' > /tmp/fish_prompt.txt
zsh -c 'source ~/.config/gpy/zsh/gpy.zsh; __gpy_prompt' > /tmp/zsh_prompt.txt

# Strip ANSI codes and compare structure
diff <(sed 's/\x1b\[[0-9;]*m//g' /tmp/fish_prompt.txt) \
     <(sed 's/\x1b\[[0-9;]*m//g' /tmp/zsh_prompt.txt)
```

---

## 9. Milestones & Timeline

### Phase 1: Core Infrastructure (MVP)
- [ ] `core/ipc.zsh` - IPC communication
- [ ] `core/init.zsh` - Prompt hooks and rendering
- [ ] `core/signals.zsh` - SIGUSR1/SIGUSR2 handlers
- [ ] `segments/git.zsh` - Git segment
- [ ] `segments/directory.zsh` - Directory segment
- [ ] Basic installation script

**Deliverable**: Functional Zsh prompt with git status and directory

### Phase 2: Feature Completion
- [ ] `core/renderer.zsh` - Full powerline rendering
- [ ] `segments/duration.zsh` - Command duration
- [ ] `segments/language.zsh` - Language detection
- [ ] `segments/clock.zsh` - Clock segment
- [ ] `segments/status.zsh` - Exit status
- [ ] Agent: Add `--format zsh` to theme export

**Deliverable**: Full feature parity with Fish

### Phase 3: Polish & Documentation
- [ ] Zsh-specific documentation
- [ ] Performance benchmarking (target: <5ms in non-git dirs)
- [ ] Completion scripts for `gpy-agent` commands
- [ ] Migration guide for P10k users

**Deliverable**: Production-ready Zsh support

---

## 10. Open Questions

1. **Shared ANSI format?** Can `fish-ansi` and `zsh-ansi` be the same format? Initial testing suggests yes.

2. **Framework integration?** Should we provide Oh-My-Zsh/Prezto plugins, or keep GPY standalone?

3. **Async prompt?** Zsh supports `zsh-async` for background prompt generation. Worth integrating for huge repos?

4. **Instant prompt?** P10k's instant prompt caches the last prompt. Should GPY implement this?

---

## 11. References

- [Zsh Prompt Expansion](https://zsh.sourceforge.io/Doc/Release/Prompt-Expansion.html)
- [Zsh Hook Functions](https://zsh.sourceforge.io/Doc/Release/Functions.html#Hook-Functions)
- [Powerlevel10k Source](https://github.com/romkatv/powerlevel10k) - Reference implementation
- [Starship Zsh Init](https://github.com/starship/starship/blob/master/src/init/starship.zsh)
- [GPY Fish Implementation](../../fish/) - Existing Fish code

---

## Appendix A: Zsh Prompt Escape Sequences

| Sequence | Meaning |
|----------|---------|
| `%F{color}` | Set foreground color |
| `%f` | Reset foreground |
| `%K{color}` | Set background color |
| `%k` | Reset background |
| `%B` / `%b` | Bold on/off |
| `%~` | Current directory (with ~ for home) |
| `%n` | Username |
| `%m` | Hostname |
| `%?` | Exit status of last command |
| `%%` | Literal % |

## Appendix B: Color Names

Zsh accepts:
- Named colors: `black`, `red`, `green`, `yellow`, `blue`, `magenta`, `cyan`, `white`
- 256-color: `{0}` through `{255}`
- True color: `{#RRGGBB}`

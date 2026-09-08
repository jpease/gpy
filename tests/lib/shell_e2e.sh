#!/usr/bin/env bash
# tests/lib/shell_e2e.sh
#
# Shared helpers for the Bash and Zsh test suites (sourced, never executed).
# The Bash/Zsh twin of tests/lib/test_helpers.fish (#646).
#
# Written in the POSIX-sh subset both shells run identically: every expansion
# is quoted, there are no arrays, no `[[ =~ ]]`, and no bash-only builtins.
# Zsh tests source it as `emulate sh -c '. tests/lib/shell_e2e.sh'` so the
# functions keep sh word-splitting semantics when called from zsh.
#
# Every test file sources this with:
#     ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
#     # shellcheck source=tests/lib/shell_e2e.sh
#     . "$ROOT/tests/lib/shell_e2e.sh"
# (Zsh: ROOT=${0:a:h:h:h}; emulate sh -c ". $ROOT/tests/lib/shell_e2e.sh".)
#
# ---------------------------------------------------------------------------
# Skip contract (#650)
# ---------------------------------------------------------------------------
# A test that cannot run because a prerequisite is missing calls
# `test_skip "reason"`. Locally that prints `SKIP: reason` and exits 0, so a
# developer without, say, python3 is not blocked; under `CI` the same call
# exits 1, so the gate can never go green on a test that did not actually
# execute. Never `exit 0` on a missing prerequisite directly.

# Print `SKIP: <reason>` and exit: 0 locally, 1 when CI is set.
test_skip() {
    printf 'SKIP: %s\n' "$*"
    if [ -n "${CI:-}" ]; then
        printf 'FAIL: a skipped test is a failure under CI (#650)\n'
        exit 1
    fi
    exit 0
}

# `test_require_command NAME [reason]` -- skip unless NAME is on PATH.
test_require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        test_skip "${2:-$1 not installed}"
    fi
}

# ---------------------------------------------------------------------------
# Live-daemon harness
# ---------------------------------------------------------------------------
# shell_e2e_init            sandbox HOME/XDG_*/socket under the GPY test root,
#                           resolve (build if needed) the debug agent, prepend
#                           it to PATH, git-init $SHELL_E2E_REPO with one
#                           commit, arm an EXIT trap that tears everything down
# shell_e2e_start_agent     start the agent, wait <= 3 s until it answers
# shell_e2e_stop_agent      stop it (socket-scoped force kill as a fallback)
# shell_e2e_spawn_client SH spawn an interactive bash or zsh on a pty through
#                           tests/lib/pty_session.py, sourcing this checkout's
#                           integration; the client prints "GPY_E2E_PID=<pid>"
#                           first so shell_e2e_client_pid can find it
# shell_e2e_send TEXT       type into the client (\r \n \t \e \x04 decoded)
# shell_e2e_wait_for RE T [OFF]
#                           poll the transcript from byte OFF until RE matches;
#                           prints the new transcript length; exit 1 and dump
#                           the tail on timeout; T is scaled by
#                           shell_e2e_timeout_scale
# shell_e2e_size            current transcript length (an OFF for "anything
#                           new after now")
# shell_e2e_transcript [OFF]
#                           the transcript after OFF with every terminal
#                           escape removed (SGR, CSI, OSC, DCS, powerline
#                           chevrons U+E0B0..E0BF)
# shell_e2e_client_pid      the client shell's PID
# shell_e2e_assert_registered PID
#                           0 when the agent's log shows that PID registered
# shell_e2e_stop_client     end the client session
# shell_e2e_timeout_scale   1-minute load average per core, rounded up and
#                           capped at 5; SHELL_E2E_TIMEOUT_SCALE overrides it
#
# All waits are bounded polls; nothing sleeps for synchronisation. Every
# wait's budget is scaled by shell_e2e_timeout_scale (shell_e2e_poll and
# shell_e2e_wait_for apply it themselves).

SHELL_E2E_PTY="${SHELL_E2E_PTY:-}"   # set by shell_e2e_init
SHELL_E2E_ROOT=""
SHELL_E2E_REPO=""
SHELL_E2E_SESSION=""
SHELL_E2E_AGENT_BIN=""
SHELL_E2E_CLIENT_PID=""

# The single GPY-owned root every test-harness socket lives under; mirrors
# gpy_test_root() in gpy-agent/tests/common/fixtures.rs and the root
# scripts/cleanup-test-agents.sh reaps (#619).
shell_e2e_test_root() {
    _base="${TMPDIR:-/tmp}"
    _base="${_base%/}"
    _user="${USER:-$(id -un)}"
    printf '%s/gpy-test-%s\n' "$_base" "$_user"
}

shell_e2e_init() {
    _repo_root="$1"
    test_require_command git
    test_require_command python3 "python3 is required to drive a pseudo-terminal"

    unset GIT_DIR GIT_WORK_TREE GIT_INDEX_FILE GIT_COMMON_DIR \
        GIT_OBJECT_DIRECTORY GIT_ALTERNATE_OBJECT_DIRECTORIES \
        GIT_PREFIX GIT_NAMESPACE 2>/dev/null || true

    _test_root="$(shell_e2e_test_root)"
    mkdir -p "$_test_root"
    SHELL_E2E_ROOT="$(mktemp -d "$_test_root/shell-e2e.XXXXXX")"
    mkdir -p "$SHELL_E2E_ROOT/home" "$SHELL_E2E_ROOT/config/gpy" \
        "$SHELL_E2E_ROOT/cache" "$SHELL_E2E_ROOT/runtime"
    export HOME="$SHELL_E2E_ROOT/home"
    export XDG_CONFIG_HOME="$SHELL_E2E_ROOT/config"
    export XDG_CACHE_HOME="$SHELL_E2E_ROOT/cache"
    export XDG_RUNTIME_DIR="$SHELL_E2E_ROOT/runtime"
    export GPY_AGENT_SOCKET_PATH="$SHELL_E2E_ROOT/gpy.sock"
    export GPY_DEBUG_LOG="$SHELL_E2E_ROOT/agent.log"
    export GPY_NERD_FONT=none
    SHELL_E2E_PTY="$_repo_root/tests/lib/pty_session.py"
    SHELL_E2E_SESSION="$SHELL_E2E_ROOT/session"

    SHELL_E2E_AGENT_BIN="$_repo_root/gpy-agent/target/debug/gpy-agent"
    if [ ! -x "$SHELL_E2E_AGENT_BIN" ]; then
        echo "Building debug gpy-agent for the shell E2E tests..."
        (cd "$_repo_root/gpy-agent" && RUSTC_WRAPPER="" cargo build --quiet) || {
            echo "FAIL: could not build gpy-agent"
            exit 1
        }
    fi
    export PATH="$_repo_root/gpy-agent/target/debug:$PATH"

    # A scratch repository with one commit, hermetic against the developer's
    # global git config (signing prompts and hooks would block a commit typed
    # into the pty; a global fsmonitor daemon competes with the watcher).
    SHELL_E2E_REPO="$SHELL_E2E_ROOT/repo"
    mkdir -p "$SHELL_E2E_REPO"
    git -C "$SHELL_E2E_REPO" init -q -b main
    git -C "$SHELL_E2E_REPO" config user.email test@example.com
    git -C "$SHELL_E2E_REPO" config user.name Test
    git -C "$SHELL_E2E_REPO" config core.fsmonitor false
    git -C "$SHELL_E2E_REPO" config commit.gpgsign false
    git -C "$SHELL_E2E_REPO" config core.hooksPath /dev/null
    echo hello >"$SHELL_E2E_REPO/tracked.txt"
    git -C "$SHELL_E2E_REPO" add tracked.txt
    git -C "$SHELL_E2E_REPO" commit -qm init

    trap shell_e2e_cleanup EXIT
}

# The agent's report line for a live daemon.
shell_e2e_agent_responding() {
    "$SHELL_E2E_AGENT_BIN" status 2>/dev/null | grep -q 'Running and Responding'
}

# The full quality gate runs these suites while `cargo build --release` and
# other heavy jobs are still finishing, so a wait budget sized for an idle
# machine flakes under real contention: the agent's own notify path is a
# single-digit-millisecond operation, but the shell process's chance to run
# its signal handler and re-render is subject to whatever else the scheduler
# is doing. Every `shell_e2e_wait_for`/`shell_e2e_poll` budget is scaled by
# the current 1-minute load average per core, so tests stay tight when idle
# and stay reliable under contention instead of racing a fixed deadline.
# SHELL_E2E_TIMEOUT_SCALE overrides the computed value (set it to pin a
# reproducible budget, e.g. when bisecting a timing issue).
shell_e2e_timeout_scale() {
    if [ -n "${SHELL_E2E_TIMEOUT_SCALE:-}" ]; then
        printf '%s\n' "$SHELL_E2E_TIMEOUT_SCALE"
        return 0
    fi
    _cores="$(getconf _NPROCESSORS_ONLN 2>/dev/null)"
    [ -n "$_cores" ] || _cores=1
    _load1="$( { sysctl -n vm.loadavg 2>/dev/null || cat /proc/loadavg 2>/dev/null; } | tr -d '{}' | awk '{print $1}')"
    [ -n "$_load1" ] || _load1=0
    awk -v load="$_load1" -v cores="$_cores" 'BEGIN {
        if (cores < 1) cores = 1
        ratio = load / cores
        scale = ratio
        if (scale < 1) scale = 1
        if (scale > int(scale)) scale = int(scale) + 1
        if (scale > 5) scale = 5
        printf "%d\n", scale
    }'
}

# Poll `CMD...` every 100 ms until it exits 0 or `$1` seconds (scaled by the
# current load, see shell_e2e_timeout_scale) pass.
shell_e2e_poll() {
    _timeout="$(( $1 * $(shell_e2e_timeout_scale) ))"
    shift
    _attempts=$(( _timeout * 10 ))
    while [ "$_attempts" -gt 0 ]; do
        if "$@"; then
            return 0
        fi
        sleep 0.1
        _attempts=$(( _attempts - 1 ))
    done
    return 1
}

shell_e2e_start_agent() {
    "$SHELL_E2E_AGENT_BIN" start >"$SHELL_E2E_ROOT/agent-start.log" 2>&1 || {
        echo "FAIL: gpy-agent start failed:"
        cat "$SHELL_E2E_ROOT/agent-start.log"
        return 1
    }
    if ! shell_e2e_poll 3 shell_e2e_agent_responding; then
        echo "FAIL: agent did not answer within 3 s"
        return 1
    fi
    return 0
}

shell_e2e_socket_gone() {
    [ ! -S "$GPY_AGENT_SOCKET_PATH" ]
}

# Stop the agent through its own socket, then force-kill whatever still
# holds that socket open (never a name-based kill; see #484).
shell_e2e_stop_agent() {
    [ -S "$GPY_AGENT_SOCKET_PATH" ] || return 0
    "$SHELL_E2E_AGENT_BIN" stop >/dev/null 2>&1 || true
    if ! shell_e2e_poll 3 shell_e2e_socket_gone; then
        if command -v lsof >/dev/null 2>&1; then
            for _pid in $(lsof -t "$GPY_AGENT_SOCKET_PATH" 2>/dev/null); do
                kill -9 "$_pid" 2>/dev/null || true
            done
        fi
        rm -f "$GPY_AGENT_SOCKET_PATH"
    fi
    return 0
}

# Spawn an interactive `bash` or `zsh` on a pty, sourcing this checkout's
# integration from an rc file. The rc file prints GPY_E2E_PID=<pid> before
# the integration loads so shell_e2e_client_pid can read it back.
shell_e2e_spawn_client() {
    _shell="$1"
    _repo_root="${2:-$ROOT}"
    _rc_dir="$SHELL_E2E_ROOT/rc-$_shell"
    mkdir -p "$_rc_dir"
    case "$_shell" in
        bash)
            printf 'echo "GPY_E2E_PID=$$"\nsource "%s/bash/gpy.bash"\n' "$_repo_root" >"$_rc_dir/bashrc"
            python3 "$SHELL_E2E_PTY" start "$SHELL_E2E_SESSION" -- \
                bash --noprofile --rcfile "$_rc_dir/bashrc" -i
            ;;
        zsh)
            printf 'echo "GPY_E2E_PID=$$"\nsource "%s/zsh/gpy.zsh"\n' "$_repo_root" >"$_rc_dir/.zshrc"
            ZDOTDIR="$_rc_dir" python3 "$SHELL_E2E_PTY" start "$SHELL_E2E_SESSION" -- \
                zsh -i
            ;;
        *)
            echo "FAIL: shell_e2e_spawn_client: unknown shell '$_shell'"
            return 1
            ;;
    esac
    _size="$(python3 "$SHELL_E2E_PTY" wait-for "$SHELL_E2E_SESSION" 'GPY_E2E_PID=[0-9]+' 10)" || {
        echo "FAIL: the $_shell client never announced its PID"
        return 1
    }
    SHELL_E2E_CLIENT_PID="$(tr -d '\r' <"$SHELL_E2E_SESSION/transcript" | sed -n 's/.*GPY_E2E_PID=\([0-9][0-9]*\).*/\1/p' | head -n 1)"
    return 0
}

shell_e2e_client_pid() {
    printf '%s\n' "$SHELL_E2E_CLIENT_PID"
}

shell_e2e_send() {
    python3 "$SHELL_E2E_PTY" send "$SHELL_E2E_SESSION" "$1"
}

shell_e2e_size() {
    python3 "$SHELL_E2E_PTY" size "$SHELL_E2E_SESSION"
}

shell_e2e_wait_for() {
    _t="$(( $2 * $(shell_e2e_timeout_scale) ))"
    python3 "$SHELL_E2E_PTY" wait-for "$SHELL_E2E_SESSION" "$1" "$_t" "${3:-0}"
}

# Transcript after byte offset $1 (default 0) with terminal escapes removed.
shell_e2e_transcript() {
    _off="${1:-0}"
    tail -c +$(( _off + 1 )) "$SHELL_E2E_SESSION/transcript" | python3 -c '
import re, sys
data = sys.stdin.buffer.read()
data = re.sub(rb"\x1b\][^\x07\x1b]*(\x07|\x1b\\\\)", b"", data)
data = re.sub(rb"\x1bP[^\x1b]*\x1b\\\\", b"", data)
data = re.sub(rb"\x1b\[[0-9;?>=<]*[A-Za-z]", b"", data)
data = re.sub(rb"\x1b[()=>][A-Za-z0-9]?", b"", data)
data = data.replace(b"\r", b"")
text = data.decode("utf-8", "replace")
text = re.sub("[\ue0b0-\ue0bf]", "", text)
sys.stdout.write(text)
'
}

# The agent logs "Registering client: PID=<pid>" when a shell registers.
shell_e2e_assert_registered() {
    grep -q "Registering client: PID=$1," "$GPY_DEBUG_LOG" 2>/dev/null
}

shell_e2e_stop_client() {
    [ -n "$SHELL_E2E_SESSION" ] || return 0
    python3 "$SHELL_E2E_PTY" stop "$SHELL_E2E_SESSION" >/dev/null 2>&1 || true
}

shell_e2e_dump_transcript() {
    echo "--- transcript tail ---"
    shell_e2e_transcript 0 | tail -n 15
    echo "--- agent log tail ---"
    grep -v 'Clock\|Connection accepted\|Waiting for connection' "$GPY_DEBUG_LOG" 2>/dev/null | tail -n 12
}

shell_e2e_cleanup() {
    shell_e2e_stop_client
    shell_e2e_stop_agent
    if [ -n "$SHELL_E2E_ROOT" ] && [ -d "$SHELL_E2E_ROOT" ]; then
        rm -rf "$SHELL_E2E_ROOT"
    fi
}

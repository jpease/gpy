#!/usr/bin/env bash
# tests/lib/shell_e2e.sh
#
# Shared helpers for the Bash and Zsh test suites (sourced, never executed).
# The Bash/Zsh twin of tests/lib/test_helpers.fish.
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
# (Zsh: ROOT=${0:a:h:h:h}; emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"' --
# single-quoted so the path expands as one word and is never re-parsed as code.)

# Assertions. `fail` prints and counts into $failures; `pass` only prints.
# `check LABEL EXPECTED ACTUAL` passes when the two strings are equal;
# `expect_eq LABEL ACTUAL EXPECTED` is the same check with the operands in the
# order a measured value is usually at hand. `test_finish SUMMARY` ends the
# test: exit 1 with the failure count when any assertion failed, otherwise
# print `PASS: SUMMARY`. printf, not echo: zsh's echo would interpret
# backslashes in the message.
failures=0
fail() {
    printf 'FAIL: %s\n' "$*"
    failures=$((failures + 1))
}
pass() {
    printf 'PASS: %s\n' "$*"
}
check() {
    if [ "$3" = "$2" ]; then
        pass "$1"
    else
        fail "$1 (expected [$2], got [$3])"
    fi
}
expect_eq() {
    check "$1" "$3" "$2"
}
test_finish() {
    if [ "$failures" -gt 0 ]; then
        printf 'FAILED: %s assertion(s)\n' "$failures"
        exit 1
    fi
    printf 'PASS: %s\n' "${1:-all assertions}"
}

# Skip contract: a test that cannot run because a prerequisite is missing
# calls `test_skip "reason"`. Locally that prints `SKIP: reason` and exits 0,
# so a developer without, say, python3 is not blocked; under `CI` or `GPY_GATE_STRICT` the same
# call exits 1, so the gate can never go green on a test that did not
# actually execute. Never `exit 0` on a missing prerequisite directly.
test_skip() {
    test_skip_part "$@"
    exit 0
}

# `test_skip_part "reason"` -- the same contract for one section of a test:
# prints `SKIP: reason` and returns so the caller can carry on with the rest;
# exits 1 under `CI` or `GPY_GATE_STRICT`.
test_skip_part() {
    printf 'SKIP: %s\n' "$*"
    if [ -n "${CI:-}" ] || [ -n "${GPY_GATE_STRICT:-}" ]; then
        printf 'FAIL: a skipped test is a failure under CI or GPY_GATE_STRICT\n'
        exit 1
    fi
}

# `test_require_command NAME [reason]` -- skip unless NAME is on PATH.
test_require_command() {
    if ! command -v "$1" >/dev/null 2>&1; then
        test_skip "${2:-$1 not installed}"
    fi
}

# `require_yq` -- skip unless `yq` is mikefarah's v4 and at least 4.52.4 (older
# releases cannot parse the comments inside deny.toml's `skip` array; the
# Debian `yq` package is a different program). POSIX sh only: this file is
# sourced by Zsh under `emulate sh`.
require_yq() {
    _ry_out="$(yq --version 2>/dev/null)" || _ry_out=""
    _ry_ver=""
    case "$_ry_out" in
        *mikefarah*)
            _ry_ver="$(printf '%s\n' "$_ry_out" |
                sed -n 's/.*version v\{0,1\}\([0-9][0-9]*\.[0-9][0-9]*\.[0-9][0-9]*\).*/\1/p')"
            ;;
    esac
    _ry_ok=0
    if [ -n "$_ry_ver" ]; then
        _ry_major="${_ry_ver%%.*}"
        _ry_rest="${_ry_ver#*.}"
        _ry_minor="${_ry_rest%%.*}"
        _ry_patch="${_ry_rest#*.}"
        if [ "$_ry_major" -gt 4 ] ||
            { [ "$_ry_major" -eq 4 ] && [ "$_ry_minor" -gt 52 ]; } ||
            { [ "$_ry_major" -eq 4 ] && [ "$_ry_minor" -eq 52 ] && [ "$_ry_patch" -ge 4 ]; }; then
            _ry_ok=1
        fi
    fi
    unset _ry_out _ry_ver _ry_major _ry_rest _ry_minor _ry_patch
    if [ "$_ry_ok" -ne 1 ]; then
        unset _ry_ok
        test_skip "mikefarah yq >= 4.52.4 not installed (see scripts/README.md)"
    fi
    unset _ry_ok
}

# ---------------------------------------------------------------------------
# Live-daemon harness
# ---------------------------------------------------------------------------
# shell_e2e_init            sandbox HOME/XDG_*/socket under the GPY test root,
#                           resolve (build if needed) the debug agent, prepend
#                           it to PATH, git-init $SHELL_E2E_REPO with one
#                           commit, arm the teardown guard (below)
# shell_e2e_agent_free_begin / shell_e2e_agent_free_end
#                           source the integration without a daemon starting
#                           under it, then put PATH back. An exported
#                           GPY_AGENT_ENABLED=0 wins over config.toml, so
#                           `GPY_AGENT_ENABLED=0 source ...` between the two
#                           calls keeps the startup start away; with no
#                           gpy-agent on PATH there is neither an export nor
#                           an agent to start either. Then export
#                           GPY_AGENT_ENABLED=1 for the call-time reads.
# shell_e2e_text_config SEGS [ICONS] [supervisor-off]
#                           write the text-theme config.toml with
#                           `enabled_segments = [SEGS]`
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
# shell_e2e_use_session NAME
#                           make NAME the current client session (its own
#                           pty directory); SHELL_E2E_CLIENT_PID is re-read
#                           from that session's transcript, so switching back
#                           restores it (empty before the client is spawned)
# shell_e2e_stop_all_clients
#                           end every client session, named or default
# shell_e2e_timeout_scale   1-minute load average per core, rounded up and
#                           capped at 5; SHELL_E2E_TIMEOUT_SCALE overrides it
# shell_e2e_poll T CMD...   run CMD every 100 ms until it succeeds or T
#                           (scaled) seconds pass
# shell_e2e_backdate S FILE...
#                           set each FILE's mtime S seconds into the past
# shell_e2e_count_lines FILE
#                           FILE's line count; 0 when FILE does not exist
# shell_e2e_has_lines FILE N
#                           true when FILE has exactly N lines (a
#                           shell_e2e_poll predicate)
#
# All waits are bounded polls; nothing sleeps for synchronisation. Every
# wait's budget is scaled by shell_e2e_timeout_scale (shell_e2e_poll and
# shell_e2e_wait_for apply it themselves).
#
# Teardown guard. Sourcing this file turns SIGINT, SIGTERM and SIGHUP into an
# `exit`, so every EXIT trap a test installs also runs when the test is
# interrupted or timed out (bash skips its EXIT trap on INT, zsh on INT and
# TERM). shell_e2e_init installs shell_e2e_cleanup as the EXIT trap: it ends
# every pty client, stops the agent, kills whatever still holds a unix socket
# under $SHELL_E2E_ROOT (an agent the supervisor autostarted), kills the PIDs
# registered with shell_e2e_track_pid, and removes the sandbox. Nothing is
# matched by process name, so a contributor's own daemon survives. SIGKILL
# cannot be caught: a daemon orphaned that way is the one case this harness
# cannot reap.
#
# shell_e2e_track_pid PID   register a helper process (listener, stub, client)
#                           for shell_e2e_cleanup to kill
# shell_e2e_kill_tracked    stop every registered PID now; the EXIT trap of a
#                           test that runs no shell_e2e_init
# shell_e2e_stop_pids PIDS  TERM each PID in the space-separated list, wait
#                           (bounded) for them to exit, then KILL the rest

# ---------------------------------------------------------------------------
# Stub agent
# ---------------------------------------------------------------------------
# shell_e2e_stub_listener SOCKET MODE [OPTION...]
#                           run tests/lib/stub_listener.py on SOCKET (its
#                           docstring lists the modes and options). Returns
#                           once SOCKET accepts connections, with the serving
#                           process's PID in SHELL_E2E_STUB_PID, registered
#                           with shell_e2e_track_pid. Stop it with
#                           shell_e2e_stop_pids: it logs what is still queued,
#                           then exits. MODE bind-only leaves a socket file
#                           nothing listens on and SHELL_E2E_STUB_PID empty.
#                           A stub that cannot start fails the test. Reads
#                           the script from $ROOT, the checkout every test
#                           sets before sourcing this file.
SHELL_E2E_STUB_PID=""

shell_e2e_stub_listener() {
    if ! SHELL_E2E_STUB_PID="$(python3 "$ROOT/tests/lib/stub_listener.py" "$@")"; then
        printf 'FAIL: could not start a stub listener on %s\n' "$1"
        exit 1
    fi
    [ -z "$SHELL_E2E_STUB_PID" ] || shell_e2e_track_pid "$SHELL_E2E_STUB_PID"
}

# ---------------------------------------------------------------------------
# Release asset fixtures
# ---------------------------------------------------------------------------
# shell_e2e_build_shell_archive ROOT DEST [DROP]
#                           write DEST as ROOT's fish/, zsh/ and bash/ trees,
#                           laid out like the release's gpy-shell-files.tar.gz
#                           (what scripts/package-release.sh builds and
#                           install-oneline.sh downloads); DROP, a path
#                           relative to ROOT such as fish/conf.d/gpy_init.fish,
#                           is left out of the archive
shell_e2e_build_shell_archive() {
    _sba_root="$1"
    _sba_dest="$2"
    _sba_drop="${3:-}"
    _sba_tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-shell-archive.XXXXXX")" || return 1
    cp -R "$_sba_root/fish" "$_sba_root/zsh" "$_sba_root/bash" "$_sba_tmp/" || {
        rm -rf "$_sba_tmp"
        return 1
    }
    if [ -n "$_sba_drop" ]; then
        rm -f "$_sba_tmp/$_sba_drop"
    fi
    tar -czf "$_sba_dest" -C "$_sba_tmp" fish zsh bash
    _sba_rc=$?
    rm -rf "$_sba_tmp"
    return "$_sba_rc"
}

# Seconds a test waits to show that something did NOT happen; absence cannot be
# polled for, so every such wait uses this one value (scaled by the caller if needed).
# shellcheck disable=SC2034 # read by the sourcing tests
SHELL_E2E_ABSENCE_WINDOW=0.5

SHELL_E2E_PTY="${SHELL_E2E_PTY:-}"   # set by shell_e2e_init
SHELL_E2E_ROOT=""
SHELL_E2E_REPO=""
SHELL_E2E_SESSION=""
SHELL_E2E_AGENT_BIN=""
SHELL_E2E_CLIENT_PID=""

# The single GPY-owned root every test-harness socket lives under; mirrors
# gpy_test_root() in gpy-agent/tests/common/fixtures.rs.
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
    # The agent refuses a group-writable socket directory; `mkdir -p` would
    # leave one under a umask of 002.
    chmod -R go-w "$SHELL_E2E_ROOT"
    export HOME="$SHELL_E2E_ROOT/home"
    export XDG_CONFIG_HOME="$SHELL_E2E_ROOT/config"
    export XDG_DATA_HOME="$SHELL_E2E_ROOT/data"
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
        (cd "$_repo_root/gpy-agent" && cargo build --quiet) || {
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
    "$SHELL_E2E_AGENT_BIN" status >/dev/null 2>&1
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
    # The awk variable is `loadavg`, not `load`: gawk reserves `load` (the
    # @load directive) and aborts on it as a variable name.
    awk -v loadavg="$_load1" -v cores="$_cores" 'BEGIN {
        if (cores < 1) cores = 1
        ratio = loadavg / cores
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

# Move the mtime of each FILE $1 seconds into the past. For "this entry is
# older than that TTL" over one-second timestamp resolution: backdating is
# instant, waiting the age out is not.
shell_e2e_backdate() {
    _secs="$1"
    shift
    python3 -c 'import os, sys, time
t = time.time() - float(sys.argv[1])
for path in sys.argv[2:]:
    os.utime(path, (t, t))' "$_secs" "$@"
}

shell_e2e_count_lines() {
    if [ -f "$1" ]; then
        wc -l <"$1" | tr -d ' '
    else
        printf '0\n'
    fi
}

shell_e2e_has_lines() {
    [ "$(shell_e2e_count_lines "$1")" -eq "$2" ]
}

# A PATH with no gpy-agent on it (system directories only); the previous PATH
# is kept for shell_e2e_agent_free_end.
shell_e2e_agent_free_begin() {
    SHELL_E2E_SAVED_PATH="$PATH"
    PATH="/usr/bin:/bin:/usr/sbin:/sbin"
}

shell_e2e_agent_free_end() {
    PATH="$SHELL_E2E_SAVED_PATH"
}

# Write the text-theme config.toml: `enabled_segments = [$1]` (the TOML array
# body, e.g. '"directory", "git"'), `show_icons = ${2:-false}`, and with a
# third argument `supervisor-off` an `[agent.supervisor] enabled = false`.
shell_e2e_text_config() {
    {
        printf '[ui]\nshow_icons = %s\ntheme = "text"\nenabled_segments = [%s]\n' "${2:-false}" "$1"
        if [ "${3:-}" = supervisor-off ]; then
            printf '\n[agent.supervisor]\nenabled = false\n'
        fi
    } >"$XDG_CONFIG_HOME/gpy/config.toml"
}

# Append a truncated directory display whose prefix is $1, e.g. "OLD/". The
# truncation symbol is not in the theme export, so changing only it across an
# agent restart raises no `.reload`: the prompt shows the new prefix only if
# the shell drops its memoized directory render when it re-registers.
shell_e2e_truncated_directory_config() {
    printf '\n[ui.directory]\ndisplay = "truncated"\ntruncation_length = 1\ntruncation_symbol = "%s"\n' "$1" \
        >>"$XDG_CONFIG_HOME/gpy/config.toml"
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

# Stop the agent through its own socket, then TERM, and as a last resort
# KILL, whatever still holds that socket open (never a name-based kill: a
# contributor's own daemon must survive the suite).
shell_e2e_stop_agent() {
    [ -S "$GPY_AGENT_SOCKET_PATH" ] || return 0
    "$SHELL_E2E_AGENT_BIN" stop >/dev/null 2>&1 || true
    shell_e2e_poll 3 shell_e2e_socket_gone && return 0
    shell_e2e_term_then_kill_holders "$GPY_AGENT_SOCKET_PATH"
    rm -f "$GPY_AGENT_SOCKET_PATH"
    return 0
}

# TERM every process holding the socket at $1, wait (bounded) for it to be
# released, then KILL whatever still holds it. A no-op without lsof.
shell_e2e_term_then_kill_holders() {
    command -v lsof >/dev/null 2>&1 || return 0
    lsof -t "$1" 2>/dev/null | while read -r _pid; do
        kill "$_pid" 2>/dev/null || true
    done
    shell_e2e_poll 2 shell_e2e_path_unbound "$1" && return 0
    lsof -t "$1" 2>/dev/null | while read -r _pid; do
        kill -9 "$_pid" 2>/dev/null || true
    done
    return 0
}

# Stop whatever agent a test left bound under a hermetic XDG pair ($1 =
# XDG_CACHE_HOME, $2 = XDG_CONFIG_HOME). For the plain bash/zsh suites that
# source the entry point and let the supervisor start a daemon: the
# supervisor backgrounds `gpy-agent start`, so the socket can still be
# missing at exit and is awaited (bounded) first. Stops through the agent's
# own socket using the PATH `gpy-agent` (the binary that started it), then
# TERMs, and as a last resort KILLs, whatever still holds that socket (never
# a name-based kill). The socket sits under $1 because these tests run
# without XDG_RUNTIME_DIR, so the runtime root falls back to cache.
shell_e2e_stop_agent_under() {
    _sock="$1/gpy/gpy.sock"
    shell_e2e_poll 3 test -S "$_sock" || return 0
    XDG_CACHE_HOME="$1" XDG_CONFIG_HOME="$2" gpy-agent stop >/dev/null 2>&1 || true
    shell_e2e_poll 3 shell_e2e_path_unbound "$_sock" && return 0
    shell_e2e_term_then_kill_holders "$_sock"
    return 0
}

# True when no process holds the socket at $1 (or lsof is unavailable).
shell_e2e_path_unbound() {
    command -v lsof >/dev/null 2>&1 || return 0
    [ -z "$(lsof -t "$1" 2>/dev/null)" ]
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
                zsh -d -i
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

shell_e2e_use_session() {
    SHELL_E2E_SESSION="$SHELL_E2E_ROOT/session-$1"
    SHELL_E2E_CLIENT_PID=""
    if [ -f "$SHELL_E2E_SESSION/transcript" ]; then
        SHELL_E2E_CLIENT_PID="$(tr -d '\r' <"$SHELL_E2E_SESSION/transcript" | sed -n 's/.*GPY_E2E_PID=\([0-9][0-9]*\).*/\1/p' | head -n 1)"
    fi
}

shell_e2e_stop_all_clients() {
    [ -n "$SHELL_E2E_ROOT" ] || return 0
    for _dir in "$SHELL_E2E_ROOT/session" "$SHELL_E2E_ROOT"/session-*; do
        [ -d "$_dir" ] || continue
        python3 "$SHELL_E2E_PTY" stop "$_dir" >/dev/null 2>&1 || true
    done
    return 0
}

shell_e2e_dump_transcript() {
    echo "--- transcript tail ---"
    shell_e2e_transcript 0 | tail -n 15
    echo "--- agent log tail ---"
    grep -v 'Clock\|Connection accepted\|Waiting for connection' "$GPY_DEBUG_LOG" 2>/dev/null | tail -n 12
}

# Processes registered with shell_e2e_track_pid; a space-separated PID list.
SHELL_E2E_TRACKED_PIDS=""

shell_e2e_track_pid() {
    SHELL_E2E_TRACKED_PIDS="$SHELL_E2E_TRACKED_PIDS $1"
}

# True when none of the PIDs in $1 (space-separated) is alive.
shell_e2e_pids_gone() {
    for _p in $1; do
        kill -0 "$_p" 2>/dev/null && return 1
    done
    return 0
}

shell_e2e_stop_pids() {
    for _p in $1; do
        kill "$_p" 2>/dev/null || true
    done
    shell_e2e_poll 2 shell_e2e_pids_gone "$1" && return 0
    for _p in $1; do
        kill -9 "$_p" 2>/dev/null || true
    done
    return 0
}

shell_e2e_kill_tracked() {
    [ -n "$SHELL_E2E_TRACKED_PIDS" ] || return 0
    _pids="$SHELL_E2E_TRACKED_PIDS"
    SHELL_E2E_TRACKED_PIDS=""
    shell_e2e_stop_pids "$_pids"
}

# Stop every process holding a unix socket anywhere under $SHELL_E2E_ROOT.
shell_e2e_kill_sandbox_holders() {
    [ -d "$SHELL_E2E_ROOT" ] || return 0
    command -v lsof >/dev/null 2>&1 || return 0
    find "$SHELL_E2E_ROOT" -type s 2>/dev/null | while read -r _sock; do
        shell_e2e_term_then_kill_holders "$_sock"
    done
    return 0
}

shell_e2e_cleanup() {
    shell_e2e_stop_all_clients
    shell_e2e_stop_agent
    shell_e2e_kill_sandbox_holders
    shell_e2e_kill_tracked
    if [ -n "$SHELL_E2E_ROOT" ] && [ -d "$SHELL_E2E_ROOT" ]; then
        rm -rf "$SHELL_E2E_ROOT"
    fi
}

trap 'exit 130' INT
trap 'exit 143' TERM
trap 'exit 129' HUP

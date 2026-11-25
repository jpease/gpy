#!/usr/bin/env bash
# Reap gpy-agent daemons and sockets orphaned by killed Rust test runs (a
# nextest timeout or SIGKILL skips drop guards, and the daemon has left the
# process group). The test root is per user, not per checkout, so a live run in
# another checkout shares it. Only agents whose GPY_AGENT_SOCKET_PATH is under the test root
# AND whose executable lives in this checkout (or $CARGO_TARGET_DIR) are
# stopped, and only sockets no process still holds are removed; nothing is
# matched by name alone and /tmp is never walked wholesale.

# Usage: cleanup-test-agents.sh [--dry-run]
#   --dry-run  list what would be stopped or removed; change nothing.

set -euo pipefail

DRY_RUN=0
case "${1:-}" in
    "") ;;
    --dry-run) DRY_RUN=1 ;;
    *)
        echo "Usage: $0 [--dry-run]" >&2
        exit 2
        ;;
esac
[[ $# -le 1 ]] || {
    echo "Usage: $0 [--dry-run]" >&2
    exit 2
}

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

# Mirrors gpy_test_root() in gpy-agent/tests/common/fixtures.rs (USER, then
# LOGNAME, then "unknown") -- keep in sync.
test_root() {
    local base="${TMPDIR:-/tmp}"
    base="${base%/}"
    local user="${USER:-${LOGNAME:-unknown}}"
    printf '%s/gpy-test-%s\n' "$base" "$user"
}

ROOT="$(test_root)"

# Agents this checkout's test runs launch come from its target directory.
BIN_ROOTS=("$REPO_ROOT/")
if [[ -n "${CARGO_TARGET_DIR:-}" ]]; then
    BIN_ROOTS+=("${CARGO_TARGET_DIR%/}/")
fi

owned_binary() {
    local exe="$1" prefix
    for prefix in "${BIN_ROOTS[@]}"; do
        [[ "$exe" == "$prefix"* ]] && return 0
    done
    return 1
}

echo "=== Cleaning up orphaned test agents ==="
echo "Test root: $ROOT"

# Sandbox daemons are started as `gpy-agent start` with no --socket argument:
# the socket path travels in GPY_AGENT_SOCKET_PATH, so that environment entry
# (readable for our own processes) is what marks a test agent. Never match on
# process name alone.

# Print the environment of PID (empty if unreadable).
process_env() {
    local pid="$1"
    if [[ -r "/proc/$pid/environ" ]]; then
        tr '\0' '\n' <"/proc/$pid/environ" 2>/dev/null || true
    else
        # BSD/macOS: -E appends the environment to the command column, space
        # separated, so callers match a substring rather than split entries.
        ps -E -ww -o command= -p "$pid" 2>/dev/null || true
    fi
}

# Print the path of PID's executable (empty if unknown). Avoids splitting
# `ps` output, which breaks on a checkout path containing spaces.
process_exe() {
    local pid="$1"
    if [[ -e "/proc/$pid/exe" ]]; then
        readlink "/proc/$pid/exe" 2>/dev/null || true
    elif command -v lsof >/dev/null 2>&1; then
        lsof -a -p "$pid" -d txt -Fn 2>/dev/null | sed -n 's/^n//p' | head -n 1
    fi
}

KILLED=0
while IFS= read -r pid; do
    [[ -n "$pid" ]] || continue

    # A substring match keeps a test root containing spaces working.
    env_text="$(process_env "$pid")"
    case "$env_text" in
        *"GPY_AGENT_SOCKET_PATH=$ROOT/"*) ;;
        *) continue ;;
    esac

    # Only a gpy or gpy-agent binary (never a test runner or cargo process
    # whose arguments mention gpy-agent), and only one launched from this
    # checkout: agents from another checkout belong to a live run.
    exe="$(process_exe "$pid")"
    case "${exe##*/}" in
        gpy | gpy-agent) ;;
        *) continue ;;
    esac
    owned_binary "$exe" || continue

    if [[ $DRY_RUN -eq 1 ]]; then
        echo "Would stop test agent: PID $pid ($exe)"
        ((KILLED += 1))
        continue
    fi
    echo "Stopping test agent: PID $pid ($exe)"
    kill "$pid" 2>/dev/null || true
    for ((tries = 0; tries < 20; tries++)); do
        kill -0 "$pid" 2>/dev/null || break
        sleep 0.1
    done
    if kill -0 "$pid" 2>/dev/null; then
        echo "Force killing test agent: PID $pid"
        kill -9 "$pid" 2>/dev/null || true
    fi
    ((KILLED += 1))
done < <(pgrep -f "gpy" 2>/dev/null || true)

if [[ $KILLED -eq 0 ]]; then
    echo "No orphaned test agents found"
elif [[ $DRY_RUN -eq 1 ]]; then
    echo "Would stop $KILLED orphaned test agents"
else
    echo "Stopped $KILLED orphaned test agents"
fi

# Bounded to our test root; a socket a live process still holds belongs to a
# running test (possibly another checkout's), so it stays. Without lsof that
# cannot be told, so nothing is removed.
echo
echo "=== Cleaning up leaked socket files ==="
if [[ ! -d "$ROOT" ]]; then
    echo "Test root $ROOT does not exist; nothing to clean"
elif ! command -v lsof >/dev/null 2>&1; then
    echo "lsof not found; cannot tell leaked sockets from live ones, leaving them"
else
    while IFS= read -r -d '' sock; do
        if [[ -n "$(lsof -t "$sock" 2>/dev/null)" ]]; then
            continue
        fi
        if [[ $DRY_RUN -eq 1 ]]; then
            echo "Would remove: $sock"
            continue
        fi
        rm -f "$sock"
        echo "Removed: $sock"
    done < <(find "$ROOT" -type s -name '*.sock' -print0 2>/dev/null)
fi

echo "Cleanup complete"

echo
echo "=== System Status ==="
echo "Running gpy-agent processes: $(pgrep gpy-agent 2>/dev/null | wc -l | tr -d ' ')"

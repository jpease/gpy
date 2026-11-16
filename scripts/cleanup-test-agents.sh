#!/usr/bin/env bash
# Reap gpy-agent daemons and sockets orphaned by killed Rust test runs (a
# nextest timeout or SIGKILL skips drop guards, and the daemon has left the
# process group). The test root is per user, not per checkout, so a live run in
# another checkout shares it. Only agents whose --socket is under the test root
# AND whose executable lives in this checkout (or $CARGO_TARGET_DIR) are
# stopped, and only sockets no process still holds are removed; nothing is
# matched by name alone and /tmp is never walked wholesale.

set -euo pipefail

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

# Match on --socket under our root, never on process name. `pgrep -lf`, not
# `-a`: BSD/macOS pgrep's `-a` means "include ancestors".
KILLED=0
while IFS= read -r line; do
    [[ -n "$line" ]] || continue
    pid="${line%% *}"
    cmd="${line#* }"

    [[ "$cmd" =~ --socket[[:space:]]+([^[:space:]]+) ]] || continue
    sock_path="${BASH_REMATCH[1]}"

    case "$sock_path" in
        "$ROOT"/*) ;;
        *) continue ;;
    esac

    # Skip agents launched from another checkout: they belong to a live run.
    exe="${cmd%% *}"
    owned_binary "$exe" || continue

    echo "Stopping test agent: PID $pid ($sock_path)"
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
done < <(pgrep -lf "gpy-agent (start|restart)" 2>/dev/null || true)

if [[ $KILLED -eq 0 ]]; then
    echo "No orphaned test agents found"
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
        rm -f "$sock"
        echo "Removed: $sock"
    done < <(find "$ROOT" -type s -name '*.sock' -print0 2>/dev/null)
fi

echo "Cleanup complete"

echo
echo "=== System Status ==="
echo "Running gpy-agent processes: $(pgrep gpy-agent 2>/dev/null | wc -l | tr -d ' ')"

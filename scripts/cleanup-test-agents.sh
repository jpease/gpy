#!/usr/bin/env bash
# Reap gpy-agent daemons and sockets orphaned by killed Rust test runs (a
# nextest timeout or SIGKILL skips drop guards, and the daemon has left the
# process group). Only touches agents whose --socket is under this repo's test
# root and sockets beneath it; never matches by name or walks /tmp wholesale.

set -euo pipefail

# Mirrors gpy_test_root() in gpy-agent/tests/common/fixtures.rs -- keep in sync.
test_root() {
    local base="${TMPDIR:-/tmp}"
    base="${base%/}"
    local user="${USER:-$(id -un)}"
    printf '%s/gpy-test-%s\n' "$base" "$user"
}

ROOT="$(test_root)"

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
done < <(pgrep -lf "gpy-agent start" 2>/dev/null || true)

if [[ $KILLED -eq 0 ]]; then
    echo "No orphaned test agents found"
else
    echo "Stopped $KILLED orphaned test agents"
fi

# Bounded to our test root.
echo
echo "=== Cleaning up leaked socket files ==="
if [[ -d "$ROOT" ]]; then
    while IFS= read -r -d '' sock; do
        echo "Removed: $sock"
    done < <(find "$ROOT" -type s -name '*.sock' -print0 -delete 2>/dev/null)
else
    echo "Test root $ROOT does not exist; nothing to clean"
fi

echo "Cleanup complete"

echo
echo "=== System Status ==="
echo "PTY usage: $(lsof /dev/tty* 2>/dev/null | wc -l) / 511"
echo "Running gpy-agent processes: $(pgrep -c gpy-agent 2>/dev/null || echo 0)"

#!/usr/bin/env bash
# Cleanup orphaned gpy-agent test processes
#
# Run this if you hit pty exhaustion or notice leaked test processes
#
# This is safe to run before test suites. It only targets daemonized test
# agents whose --socket argument points somewhere beneath this repo's
# GPY-owned test root (see test_root() below), and it only ever deletes
# leaked socket files found beneath that same root. It never matches a
# process by name alone, and it never traverses all of /tmp or
# /var/folders -- both would risk killing or deleting something this
# repo's test harness did not create (#619).

set -euo pipefail

# The single GPY-owned root all of this repo's test-harness sockets are
# created beneath. Mirrors gpy_test_root() in
# gpy-agent/tests/common/fixtures.rs and __gpy_test_root() in
# tests/support/setup_test_env.fish -- keep all three in sync.
test_root() {
    local base="${TMPDIR:-/tmp}"
    base="${base%/}"
    local user="${USER:-$(id -un)}"
    printf '%s/gpy-test-%s\n' "$base" "$user"
}

ROOT="$(test_root)"

echo "=== Cleaning up orphaned test agents ==="
echo "Test root: $ROOT"

# Kill any gpy-agent process whose --socket argument resolves under our test
# root. Parsed from `pgrep -lf` output (PID + full command line -- `-lf`
# rather than GNU-only `-a`, since BSD/macOS pgrep's `-a` means "include
# ancestors", not "show full command line") rather than matched by process
# name or socket basename alone, so a process using a custom, non-GPY socket
# path is never touched even if it happens to be named gpy-agent.
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
    sleep 0.1
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

# Clean up any leaked socket files -- bounded to our own test root, never a
# system-wide /tmp or /var/folders traversal (#619).
echo
echo "=== Cleaning up leaked socket files ==="
if [[ -d "$ROOT" ]]; then
    while IFS= read -r -d '' sock; do
        echo "Removed: $sock"
    done < <(find "$ROOT" -type s -name '*.sock' -print0 -delete 2>/dev/null)
else
    echo "Test root $ROOT does not exist; nothing to clean"
fi

# Reap oneshot-budget markers left by shells that no longer exist. The Fish
# integration names its marker `${TMPDIR:-/tmp}/.gpy_oneshot_used_<pid>`
# (fish/core/ipc.fish) and, before the fish_exit cleanup, a shell whose last
# render claimed it left the file behind. A later process that reuses the
# PID -- including this repo's Bash/Zsh oneshot tests, which assert no such
# file exists for their own $$ -- then inherits a spent budget. This is a
# bounded glob on a GPY-owned name, not a TMPDIR traversal (#619), and only
# markers whose PID is dead are removed.
echo
echo "=== Cleaning up stale oneshot markers ==="
marker_dir="${TMPDIR:-/tmp}"
marker_dir="${marker_dir%/}"
stale_markers=0
for marker in "$marker_dir"/.gpy_oneshot_used_*; do
    [[ -e "$marker" ]] || continue
    marker_pid="${marker##*/.gpy_oneshot_used_}"
    [[ "$marker_pid" =~ ^[0-9]+$ ]] || continue
    if ! kill -0 "$marker_pid" 2>/dev/null; then
        rm -f "$marker"
        stale_markers=$((stale_markers + 1))
    fi
done
echo "Removed $stale_markers stale marker(s)"
echo "Cleanup complete"

echo
echo "=== System Status ==="
echo "PTY usage: $(lsof /dev/tty* 2>/dev/null | wc -l) / 511"
echo "Running gpy-agent processes: $(pgrep -c gpy-agent 2>/dev/null || echo 0)"

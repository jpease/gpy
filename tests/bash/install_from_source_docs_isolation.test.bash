#!/usr/bin/env bash
# tests/bash/install_from_source_docs_isolation.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# install_from_source_docs.test.bash must be sealed from its caller (#749).
# This wrapper runs it as a child whose HOME/XDG_* belong to a "caller" with a
# live decoy agent and whose exported fish_user_paths holds stub gpy and
# gpy-agent binaries, then asserts after the child exits that:
#   (a) the decoy agent is still the same live process (teardown used to stop
#       the caller's agent through the caller's socket)
#   (b) both stubs still exist (an unsealed PATH/fish_user_paths made
#       install-dev delete them as "conflicting binaries")
#   (c) no process holds a socket under the child's TMPDIR sandboxes
#
# The decoy is stopped by PID in the EXIT trap; nothing is ever killed by name.
# Slow (the child builds and installs): same skip conditions as the child.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

if [[ -z "${CI:-}" && -z "${GPY_GATE_RELEASE:-}" ]]; then
    test_skip "wraps the from-source install (~2 min); set GPY_GATE_RELEASE=1 to run it locally (CI always runs it)"
fi
test_require_command lsof
test_require_command cargo
test_require_command fish

AGENT_BIN="$ROOT/gpy-agent/target/debug/gpy-agent"
if [[ ! -x "$AGENT_BIN" ]]; then
    printf 'FAIL: debug agent missing at %s (build it first)\n' "$AGENT_BIN"
    exit 1
fi

# Resolve the real toolchain homes before HOME is pointed at the decoy.
real_cargo_home="${CARGO_HOME:-$HOME/.cargo}"
real_rustup_home="${RUSTUP_HOME:-$HOME/.rustup}"

WORK="$(mktemp -d "${TMPDIR:-/tmp}/gpy-isolation.XXXXXX")"
DECOY="$WORK/decoy"
decoy_pid=""
cleanup() {
    if [[ -n "$decoy_pid" ]]; then
        kill -TERM "$decoy_pid" 2>/dev/null || true
        for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
            kill -0 "$decoy_pid" 2>/dev/null || break
            sleep 0.1
        done
        kill -KILL "$decoy_pid" 2>/dev/null || true
    fi
    rm -rf "$WORK"
}
trap cleanup EXIT

mkdir -p "$DECOY/home/.config" "$DECOY/home/.cache" "$DECOY/run" "$DECOY/bin" "$WORK/tmp"
chmod 700 "$DECOY/run"

for stub in gpy gpy-agent; do
    printf '#!/bin/sh\necho "%s 0.0.1"\n' "$stub" >"$DECOY/bin/$stub"
    chmod +x "$DECOY/bin/$stub"
done

caller_env=(
    "HOME=$DECOY/home"
    "XDG_CONFIG_HOME=$DECOY/home/.config"
    "XDG_CACHE_HOME=$DECOY/home/.cache"
    "XDG_RUNTIME_DIR=$DECOY/run"
    "TMPDIR=$WORK/tmp"
    "CARGO_HOME=$real_cargo_home"
    "RUSTUP_HOME=$real_rustup_home"
    "PATH=$PATH"
    "TERM=${TERM:-dumb}"
    "LANG=${LANG:-C.UTF-8}"
)

env -i "${caller_env[@]}" "$AGENT_BIN" start >/dev/null 2>&1 || true
sock="$DECOY/run/gpy/gpy.sock"
for _ in $(seq 1 50); do
    [[ -S "$sock" ]] && break
    sleep 0.1
done
decoy_pid="$(lsof -t "$sock" 2>/dev/null | head -n 1)"
if [[ -z "$decoy_pid" ]]; then
    echo "FAIL: decoy agent did not start (no process holds $sock)"
    exit 1
fi

child_log="$WORK/child.log"
start="$(date +%s)"
env -i "${caller_env[@]}" "fish_user_paths=$DECOY/bin" GPY_GATE_RELEASE=1 \
    bash "$ROOT/tests/bash/install_from_source_docs.test.bash" >"$child_log" 2>&1
child_rc=$?
echo "child finished rc=$child_rc in $(($(date +%s) - start))s"

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }

[[ $child_rc -eq 0 ]] || {
    fail "child test failed (rc=$child_rc); last 40 lines:"
    tail -n 40 "$child_log"
}
kill -0 "$decoy_pid" 2>/dev/null || fail "decoy agent $decoy_pid was stopped by the child's teardown"
[[ "$(lsof -t "$sock" 2>/dev/null | head -n 1)" == "$decoy_pid" ]] || fail "decoy agent no longer holds its socket"
for stub in gpy gpy-agent; do
    [[ -e "$DECOY/bin/$stub" ]] || fail "stub $stub was deleted from fish_user_paths"
done
# The child deletes its sandbox on exit, so look for a surviving process still
# bound to a (now deleted) socket under a gpy-from-source.* directory.
orphans="$(lsof -U 2>/dev/null | grep -F "$WORK/tmp/gpy-from-source." || true)"
[[ -z "$orphans" ]] || fail "a process still holds a from-source sandbox socket: $orphans"

if [[ $failures -gt 0 ]]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: from-source install test is sealed from its caller"

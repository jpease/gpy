#!/usr/bin/env zsh
# tests/zsh/ipc_nc_fallback_timeout.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The `nc` transport is bounded against a wedged agent (the Zsh twin of
# tests/fish/ipc_nc_fallback_timeout.test.fish).
#
# `__gpy_send_json` prefers the zsh/net/socket builtin, then socat, then
# `nc -U ... -w 1` (wrapped in `timeout` when that exists). Against a listener
# that accepts and then never answers, the call must return non-zero, print
# nothing, and do so well before the listener's own lifetime. A stub listener
# plays the wedged agent; `zmodload` is shadowed so the builtin
# transport is unavailable, and a PATH shim hides socat and timeout so the
# bare `nc -w 1` branch is the one exercised.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

test_require_command nc "nc not installed, cannot exercise this fallback"
test_require_command python3 "python3 not installed, cannot simulate a wedged listener"

tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-nc-timeout.XXXXXX")"
sock="$tmp/wedged.sock"
listener_lifetime_secs=4
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$tmp"
}
trap cleanup EXIT

shell_e2e_stub_listener "$sock" silent --lifetime "$listener_lifetime_secs"

# Only nc (and the tools the transport itself needs) are visible.
shim="$tmp/bin"
mkdir -p "$shim"
ln -s "$(command -v nc)" "$shim/nc"
for tool in cat echo head; do
    command -v "$tool" >/dev/null 2>&1 && ln -sf "$(command -v "$tool")" "$shim/$tool"
done

export GPY_AGENT_SOCKET_PATH="$sock"
source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_IPC_TIMEOUT_MS=300
source "$ROOT/zsh/gpy.zsh"
# Hide the builtin transport: the shadow refuses to load zsh/net/socket, so
# __gpy_send_json falls through to the external tools on PATH.
zmodload() { [ "$1" = "zsh/net/socket" ] && return 1; builtin zmodload "$@"; }

start_ns="$(python3 -c 'import time; print(time.time_ns())')"
saved_path="$PATH"
PATH="$shim"
response="$(__gpy_send_json '{"op":"ping"}')"
rc=$?
PATH="$saved_path"
end_ns="$(python3 -c 'import time; print(time.time_ns())')"
elapsed_ms=$(( (end_ns - start_ns) / 1000000 ))

deadline_ms=$(( listener_lifetime_secs * 1000 - 500 ))
if [ "$elapsed_ms" -lt "$deadline_ms" ]; then
    pass "nc fallback returns instead of hanging (${elapsed_ms}ms)"
else
    fail "nc fallback took ${elapsed_ms}ms, expected well under ${deadline_ms}ms"
fi
if [ "$elapsed_ms" -lt 2000 ]; then
    pass "bounded within 2 s (${elapsed_ms}ms)"
else
    fail "nc fallback took ${elapsed_ms}ms, expected under 2000ms"
fi
if [ "$rc" -ne 0 ]; then
    pass "the send reports failure (exit $rc)"
else
    fail "the send reported success against a silent socket"
fi
if [ -z "$response" ]; then
    pass "no output on a silent socket"
else
    fail "got output on a silent socket: '$response'"
fi

test_finish "zsh nc fallback is bounded against a wedged agent"

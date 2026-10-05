#!/usr/bin/env bash
# tests/bash/doorbell_no_reentry.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Regression test for #678: Bash's SIGURG doorbell trap ran a full
# __gpy_render_prompt with no re-entrancy guard. On bash 5 a doorbell that
# arrived during a render started a nested one, so a stream of doorbells (a
# live agent answering each render's background refresh) nested renders
# without bound and the shell never read another line; bash 3.2 rendered
# continuously at an idle prompt; and any doorbell made a running `wait`
# return 144. Bash now installs no URG trap and consumes the doorbell's flag
# files in __gpy_precmd before it renders.
#
# Drives a real interactive bash (the interpreter running this file, so the
# gate's $GPY_BASH and a plain `bash` both get covered) on a pty with gpy
# loaded, no agent, and __gpy_render_prompt wrapped with a depth counter:
#   (a) 60 SIGURGs 50 ms apart during and after `sleep 0.5`: the shell still
#       runs the next line promptly and no render ever nests (max depth 1);
#   (b) a SIGURG during `wait` does not end the wait early (status 0);
#   (c) a `<pid>.reload` flag left before Enter is consumed by that Enter's
#       prompt, and the reload happens before that prompt renders.

# shellcheck disable=SC2016
# The single quotes are load-bearing: the lines are typed into the child
# shell, which must expand `$$`/`$?`/`$__gpy_test_*` itself.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command python3 "python3 is required to drive a pseudo-terminal"

T="$(mktemp -d "${TMPDIR:-/tmp}/gpy-doorbell.XXXXXX")"
SHELL_E2E_PTY="$ROOT/tests/lib/pty_session.py"
SHELL_E2E_SESSION="$T/session"
cleanup() {
    shell_e2e_stop_client
    rm -rf "$T"
}
trap cleanup EXIT

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

# Scrub every GPY_* variable inherited from the developer's shell (#270),
# then sandbox everything the integration touches.
while IFS='=' read -r __gpy_env_name _; do
    case "$__gpy_env_name" in
        GPY_*) unset "$__gpy_env_name" ;;
    esac
done < <(env)
export GPY_AGENT_SUPERVISOR_ENABLED=0
export GPY_AGENT_SOCKET_PATH="$T/missing.sock"
export HOME="$T/home" XDG_CACHE_HOME="$T/cache" XDG_CONFIG_HOME="$T/config"
export XDG_RUNTIME_DIR="$T/run"
mkdir -p "$HOME" "$XDG_CACHE_HOME" "$XDG_CONFIG_HOME" "$XDG_RUNTIME_DIR"

# The rc wraps __gpy_render_prompt with a depth counter (each render also
# sleeps 0.1 s, widening the window a nested render needs), stubs
# __gpy_load_theme so a reload is observable, and records the shell under the
# runtime root the way a successful registration does, which is where the
# agent leaves its flags (there is no agent here).
rc="$T/rc.bash"
cat >"$rc" <<EOF
echo "GPY_E2E_PID=\$\$"
source "$ROOT/bash/gpy.bash"
__gpy_track_shell_for_agent_recovery
__gpy_test_orig_render="\$(declare -f __gpy_render_prompt)"
eval "__gpy_test_orig_render \${__gpy_test_orig_render#__gpy_render_prompt}"
__gpy_test_depth=0
__gpy_test_maxdepth=0
__gpy_render_prompt() {
    __gpy_test_depth=\$((__gpy_test_depth + 1))
    if ((__gpy_test_depth > __gpy_test_maxdepth)); then
        __gpy_test_maxdepth=\$__gpy_test_depth
    fi
    __gpy_test_seen_reload="\${__gpy_test_reloaded:-0}"
    sleep 0.1
    __gpy_test_orig_render "\$@"
    __gpy_test_depth=\$((__gpy_test_depth - 1))
}
__gpy_load_theme() { __gpy_test_reloaded=1; }
EOF

python3 "$SHELL_E2E_PTY" start "$SHELL_E2E_SESSION" -- \
    env PATH=/usr/bin:/bin "$BASH" --noprofile --rcfile "$rc" -i
if ! shell_e2e_wait_for 'GPY_E2E_PID=[0-9]+[^0-9]' 10 >/dev/null; then
    fail "the bash client never announced its PID"
    exit 1
fi
pid="$(tr -d '\r' <"$SHELL_E2E_SESSION/transcript" | sed -n 's/.*GPY_E2E_PID=\([0-9][0-9]*\).*/\1/p' | head -n 1)"
shell_e2e_send 'echo "READY-$((40 + 2))"\r'
if ! off="$(shell_e2e_wait_for 'READY-42' 10)"; then
    fail "the bash client never ran its first command"
    exit 1
fi

# The last `KEY=<digits/colons>` the client printed (the typed command line
# echoes `KEY=$...`, which this never matches).
last_report() {
    shell_e2e_transcript 0 | grep -oE "$1=[0-9:]+" | tail -n 1
}

echo "=== (a) a doorbell burst never nests renders or blocks the shell ==="
shell_e2e_send 'sleep 0.5\r'
for _ in $(seq 60); do
    kill -URG "$pid" 2>/dev/null || break
    sleep 0.05
done
shell_e2e_send 'echo "MAXDEPTH=$__gpy_test_maxdepth"\r'
if off="$(shell_e2e_wait_for 'MAXDEPTH=[0-9]+[^0-9]' 5 "$off")"; then
    pass "the line typed after the doorbell burst ran"
    depth="$(last_report MAXDEPTH)"
    if [[ "$depth" == "MAXDEPTH=1" ]]; then
        pass "no render ever nested (max depth 1)"
    else
        fail "renders nested: $depth"
    fi
else
    fail "the shell never ran the line typed after the doorbell burst"
    off="$(shell_e2e_size)"
fi

echo "=== (b) a doorbell does not interrupt wait ==="
# History expansion is off for the whole line only if `set +H` ran on an
# earlier one (bash 3.2 otherwise rejects `$!;` as an event).
shell_e2e_send 'set +H\r'
shell_e2e_send 'sleep 2 & j=$!; (sleep 0.3; kill -URG $$) & wait $j; echo "WAIT=$?"\r'
if off="$(shell_e2e_wait_for 'WAIT=[0-9]+[^0-9]' 10 "$off")"; then
    status="$(last_report WAIT)"
    if [[ "$status" == "WAIT=0" ]]; then
        pass "wait returned the job's status 0"
    else
        fail "wait was interrupted: $status"
    fi
else
    fail "wait never returned"
    off="$(shell_e2e_size)"
fi

echo "=== (c) a reload flag is applied by the next prompt ==="
flag="$XDG_RUNTIME_DIR/gpy/shells/$pid.reload"
if [[ ! -d "${flag%/*}" ]]; then
    fail "the shell did not create its tracking directory ${flag%/*}"
fi
: >"$flag"
shell_e2e_send '\r'
shell_e2e_send 'echo "RELOAD=${__gpy_test_reloaded:-0}:$__gpy_test_seen_reload"\r'
if shell_e2e_wait_for 'RELOAD=[0-9]+:[0-9]+[^0-9]' 10 "$off" >/dev/null; then
    reload="$(last_report RELOAD)"
    if [[ "$reload" == "RELOAD=1:1" ]]; then
        pass "the reload ran before the prompt drawn after Enter"
    else
        fail "the reload did not run before the next prompt: $reload"
    fi
else
    fail "the reload check never printed"
fi
if [[ -e "$flag" ]]; then
    fail "the next prompt left $flag behind"
else
    pass "the next prompt consumed the reload flag"
fi

if ((failures > 0)); then
    echo "--- transcript tail ---"
    shell_e2e_transcript 0 | tail -n 20
    exit 1
fi
echo "=== All Tests Passed ==="

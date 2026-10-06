#!/usr/bin/env zsh
# tests/zsh/cache_dir_no_leak.test.zsh
#
# #763: the private render-cache dir ($TMPDIR/gpy_cache_*) must not leak on
# re-source, `exec zsh`, or a killed shell, and a live shell's dir must never
# be removed by a sibling. Children never start an agent. Paths reach children
# only as environment/argv.

ROOT=${0:a:h:h:h}
SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-cleak.XXXXXX") || exit 1
trap 'kill $LIVE_PID 2>/dev/null; rm -rf "$SANDBOX"' EXIT
LIVE_PID=
mkdir -p "$SANDBOX/tmp" "$SANDBOX/zdot"
print -r -- 'source "$GPY_TEST_ROOT/zsh/gpy.zsh"' >"$SANDBOX/zdot/.zshrc"

failures=0
fail() { print -r -- "FAIL: $*"; failures=$((failures + 1)); }
pass() { print -r -- "PASS: $*"; }

# child_env <cmd...>: run a command in a scrubbed environment.
child_env() {
    env -i PATH=/usr/bin:/bin TERM=dumb HOME="$SANDBOX" ZDOTDIR="$SANDBOX/zdot" \
        TMPDIR="$SANDBOX/tmp" XDG_CONFIG_HOME="$SANDBOX/cfg" \
        XDG_CACHE_HOME="$SANDBOX/cache" XDG_RUNTIME_DIR="$SANDBOX/run" \
        GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock" \
        GPY_TEST_ROOT="$ROOT" "$@"
}

run_zsh() { print -r -- "$1" | child_env zsh -i >/dev/null 2>&1; }

leftovers() { local -a d; d=("$SANDBOX"/tmp/gpy_cache_*(N)); print -r -- ${#d}; }
wait_dir() { local i; for i in {1..50}; do (( $(leftovers) > 0 )) && return; sleep 0.1; done; }

expect_zero() {
    local n; n=$(leftovers)
    [[ $n == 0 ]] && pass "$1: no gpy_cache_* left" || fail "$1: $n gpy_cache_* left"
}

rm -rf "$SANDBOX"/tmp/gpy_cache_*(N)
run_zsh 'exit'
expect_zero "normal exit"

run_zsh $'source "$GPY_TEST_ROOT/zsh/gpy.zsh"\nexit'
expect_zero "re-source + exit"

run_zsh $'exec zsh -i\nexit'
expect_zero "exec zsh"

# Killed shell: its dir is swept by the next shell start.
mkfifo "$SANDBOX/kill.fifo"
child_env zsh -i <"$SANDBOX/kill.fifo" >/dev/null 2>&1 &
KPID=$!
exec 8>"$SANDBOX/kill.fifo"
print -r -- 'print ready' >&8
wait_dir
kill -9 $KPID; exec 8>&-; wait $KPID 2>/dev/null
n=$(leftovers)
[[ $n == 1 ]] || fail "killed shell: expected 1 leftover before sweep, got $n"
run_zsh 'exit'
expect_zero "SIGKILLed shell swept by next start"

# Live sibling: a blocked shell's dir must survive another shell's start/exit.
mkfifo "$SANDBOX/live.fifo"
child_env zsh -i <"$SANDBOX/live.fifo" >/dev/null 2>&1 &
LIVE_PID=$!
exec 9>"$SANDBOX/live.fifo"
wait_dir
before=("$SANDBOX"/tmp/gpy_cache_*(N/))
run_zsh 'exit'
after=("$SANDBOX"/tmp/gpy_cache_*(N/))
if (( ${#before} == 1 && ${#after} == 1 )) && [[ $before == $after ]]; then
    pass "live sibling: dir survives another shell"
else
    fail "live sibling: before=(${before}) after=(${after})"
fi
[[ -d $after && "$(stat -f %Lp "$after" 2>/dev/null || stat -c %a "$after")" == 700 ]] \
    && pass "live dir mode 0700" || fail "live dir not 0700"
exec 9>&-
wait $LIVE_PID 2>/dev/null; LIVE_PID=
expect_zero "live shell after exit"

if ((failures > 0)); then
    print -r -- "=== $failures assertion(s) failed ==="
    exit 1
fi
print -r -- "=== All Tests Passed ==="

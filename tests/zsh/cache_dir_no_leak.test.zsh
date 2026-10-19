#!/usr/bin/env zsh
# tests/zsh/cache_dir_no_leak.test.zsh
#
# The private render-cache dir ($TMPDIR/gpy_relay_<uid>/<pid>_*) must not
# leak on re-source, `exec zsh`, or a killed shell, and a live shell's dir
# must never be removed by a sibling. A parent other users can enter is not
# used. Children never start an agent: none is on their PATH. Paths reach
# children only as environment/argv.

ROOT=${0:a:h:h:h}
SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-cleak.XXXXXX") || exit 1
trap 'kill $LIVE_PID 2>/dev/null; rm -rf "$SANDBOX"' EXIT
LIVE_PID=
mkdir -p "$SANDBOX/tmp" "$SANDBOX/zdot"
print -r -- 'source "$GPY_TEST_ROOT/zsh/gpy.zsh"' >"$SANDBOX/zdot/.zshrc"

emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

# child_env <cmd...>: run a command in a scrubbed environment.
child_env() {
    env -i PATH=/usr/bin:/bin TERM=dumb HOME="$SANDBOX" ZDOTDIR="$SANDBOX/zdot" \
        TMPDIR="$SANDBOX/tmp" XDG_CONFIG_HOME="$SANDBOX/cfg" \
        XDG_CACHE_HOME="$SANDBOX/cache" XDG_RUNTIME_DIR="$SANDBOX/run" \
        GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock" \
        GPY_TEST_ROOT="$ROOT" "$@"
}

run_zsh() { print -r -- "$1" | child_env zsh -i >/dev/null 2>&1; }

RELAY="$SANDBOX/tmp/gpy_relay_$UID"
leftovers() { local -a d; d=("$RELAY"/<->_*(N)); print -r -- ${#d}; }
# A native loop, not shell_e2e_poll: that helper runs under sticky `emulate sh`,
# which disables the `(N)` glob qualifier `leftovers` relies on.
wait_dir() { local i; for i in {1..50}; do (( $(leftovers) > 0 )) && return; sleep 0.1; done; }

expect_zero() {
    local n; n=$(leftovers)
    [[ $n == 0 ]] && pass "$1: no relay dir left" || fail "$1: $n relay dir(s) left"
}

rm -rf "$RELAY"
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
# $! may be the backgrounding subshell rather than zsh itself; kill the PID
# the dir is named after, and wait until it is really gone before the sweep.
kdirs=("$RELAY"/<->_*(N))
kzsh=${${kdirs[1]:t}%%_*}
kill -9 $kzsh $KPID 2>/dev/null; exec 8>&-; wait $KPID 2>/dev/null
for i in {1..50}; do kill -0 $kzsh 2>/dev/null || break; sleep 0.1; done
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
before=("$RELAY"/<->_*(N/))
run_zsh 'exit'
after=("$RELAY"/<->_*(N/))
if (( ${#before} == 1 && ${#after} == 1 )) && [[ $before == $after ]]; then
    pass "live sibling: dir survives another shell"
else
    fail "live sibling: before=(${before}) after=(${after})"
fi
[[ -d $after && "$(stat -c %a "$after" 2>/dev/null || stat -f %Lp "$after")" == 700 ]] \
    && pass "live dir mode 0700" || fail "live dir not 0700"
exec 9>&-
wait $LIVE_PID 2>/dev/null; LIVE_PID=
expect_zero "live shell after exit"

# A parent group or others can enter is someone else's to fill: not used.
chmod 755 "$RELAY"
run_zsh 'exit'
expect_zero "open relay parent"
print -r -- 'print -r -- "relay=[${__gpy_cache_dir}]"' | child_env zsh -i 2>/dev/null | grep -q 'relay=\[\]' \
    && pass "open relay parent: memoization off" || fail "open relay parent: a relay dir was used"

test_finish

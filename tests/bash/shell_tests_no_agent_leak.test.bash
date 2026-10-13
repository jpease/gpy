#!/usr/bin/env bash
# tests/bash/shell_tests_no_agent_leak.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The plain Bash and Zsh suites must not leave a gpy-agent behind.
#
# tests/{bash,zsh}/{basic,parity}.test.* source the shell entry point; the
# supervisor then runs `gpy-agent start`, which binds
# $XDG_CACHE_HOME/gpy/gpy.sock inside the test's hermetic XDG dir. The test
# deletes that dir on exit, so a daemon it does not stop is an orphan that
# scripts/cleanup-test-agents.sh (which only matches `--socket <path>`
# agents) cannot see.
#
# Each test is run the way run_shell_tests runs it: `env -i` with HOME and the
# XDG dirs in a scratch root, and the checkout's debug dir first on PATH. The
# parity tests build their own XDG root under TMPDIR and delete it on exit, so
# the socket path is gone by the time we look; the check therefore lists unix
# sockets still held open under the scratch root (`lsof -U`) rather than
# testing for the socket file. Agents found are stopped by their PID, which
# was identified by socket path, never by process name.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command lsof
test_require_command zsh

# Short path: unix socket paths are capped near 104 bytes on macOS.
scratch="$(mktemp -d /tmp/gpy-leak.XXXXXX)"
scratch_tag="$(basename "$scratch")/"

# PIDs holding a unix socket whose path is under the scratch root.
leaked_pids() {
    lsof -nU 2>/dev/null | awk -v tag="$scratch_tag" 'index($0, tag) { print $2 }' | sort -u
}

no_leaks() {
    [[ -z "$(leaked_pids)" ]]
}

cleanup() {
    local pid
    for pid in $(leaked_pids); do
        kill "$pid" 2>/dev/null || true
    done
    shell_e2e_poll 2 no_leaks || true
    for pid in $(leaked_pids); do
        kill -9 "$pid" 2>/dev/null || true
    done
    rm -rf "$scratch"
}
trap cleanup EXIT

target_dir="${CARGO_TARGET_DIR:-$ROOT/gpy-agent/target}/debug"
if [[ ! -x "$target_dir/gpy-agent" ]]; then
    echo "Building debug gpy-agent..."
    (cd "$ROOT/gpy-agent" && cargo build --quiet) || {
        echo "FAIL: could not build gpy-agent"
        exit 1
    }
fi

bash_bin="$(command -v bash)"
zsh_bin="$(command -v zsh)"
n=0
for test_file in tests/bash/basic.test.bash tests/bash/parity.test.bash \
    tests/zsh/basic.test.zsh tests/zsh/parity.test.zsh; do
    n=$((n + 1))
    sandbox="$scratch/$n"
    mkdir -p "$sandbox/home" "$sandbox/cache" "$sandbox/config"
    case "$test_file" in
        *.bash) shell_bin="$bash_bin" ;;
        *) shell_bin="$zsh_bin" ;;
    esac

    (cd "$ROOT" && env -i HOME="$sandbox/home" XDG_CACHE_HOME="$sandbox/cache" \
        XDG_CONFIG_HOME="$sandbox/config" TMPDIR="$sandbox" TERM=dumb \
        PATH="$target_dir:$PATH" "$shell_bin" "$test_file") >"$sandbox/out.log" 2>&1
    rc=$?
    if [[ "$rc" -ne 0 ]]; then
        fail "$test_file exited $rc:"
        cat "$sandbox/out.log"
        continue
    fi

    # The supervisor backgrounds `gpy-agent start`, so an agent can bind after
    # the test has exited: give a late start a fixed window to show up.
    sleep 1
    pids="$(leaked_pids)"
    if [[ -n "$pids" ]]; then
        fail "$test_file left a process holding a socket under its sandbox: $(echo "$pids" | tr '\n' ' ')"
        for pid in $pids; do
            kill "$pid" 2>/dev/null || true
        done
    else
        pass "$test_file leaves no agent behind"
    fi
done

[[ $failures -eq 0 ]] || test_finish

# stop_shell_test_agents must find an agent that bound under XDG_RUNTIME_DIR
# (the Linux default for CI runners and systemd sessions). run_shell_tests
# pins XDG_RUNTIME_DIR to <root>/run; start an agent exactly so and require the
# helper to stop it.
eval "$(awk '/^stop_shell_test_agents\(\)/ {on=1} on {print} on && /^}/ {exit}' "$ROOT/scripts/quality-check.sh")"
xroot="$scratch/xdg"
mkdir -p "$xroot/cache" "$xroot/config" "$xroot/run" "$scratch/xhome"
chmod 700 "$xroot/run"
(env -i HOME="$scratch/xhome" XDG_RUNTIME_DIR="$xroot/run" XDG_CACHE_HOME="$xroot/cache" \
    XDG_CONFIG_HOME="$xroot/config" TERM=dumb "$target_dir/gpy-agent" start >/dev/null 2>&1) || true
if ! shell_e2e_poll 5 test -S "$xroot/run/gpy/gpy.sock"; then
    fail "test agent did not bind $xroot/run/gpy/gpy.sock"
    test_finish
fi
stop_shell_test_agents "$xroot" "$target_dir"
if ! shell_e2e_poll 2 shell_e2e_path_unbound "$xroot/run/gpy/gpy.sock"; then
    fail "stop_shell_test_agents left an agent bound under XDG_RUNTIME_DIR"
    test_finish
fi
pass "stop_shell_test_agents stops an agent under XDG_RUNTIME_DIR"
test_finish "the plain Bash and Zsh suites leave no agent behind"

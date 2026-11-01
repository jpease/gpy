#!/usr/bin/env bash
# tests/bash/ipc_send_process_count.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# One __gpy_send_json call with `socat` and an OUTVAR creates one `socat`
# process and at most one subshell (the process substitution that carries the
# reply): the endpoint is resolved without `$(...)`, and the request reaches
# the client through a here-string instead of a `printf |` subshell. Without
# OUTVAR the reply is printed, which is how __gpy_request returns segment
# output. `socat` is wrapped to count its runs; a DEBUG trap inherited via
# functrace logs the PID of every command run with BASH_SUBSHELL > 0. Bash 3.2
# has no BASHPID, so there the subshell nesting depth stands in for the PID.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

test_require_command socat
test_require_command python3 "python3 not installed, cannot start the stub listener"

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-sendcount.XXXXXX")"
# shellcheck disable=SC2329 # invoked via trap
cleanup() {
    shell_e2e_kill_tracked
    rm -rf "$SANDBOX"
}
trap cleanup EXIT

real_socat="$(command -v socat)"
mkdir -p "$SANDBOX/bin"
socat_runs="$SANDBOX/socat-runs"
cat >"$SANDBOX/bin/socat" <<EOF
#!/bin/sh
printf 'run\n' >>"$socat_runs"
exec "$real_socat" "\$@"
EOF
chmod +x "$SANDBOX/bin/socat"
PATH="$SANDBOX/bin:$PATH"

source bash/core/ipc.bash

subshells="$SANDBOX/subshells"
requests="$SANDBOX/requests"
sock="$SANDBOX/agent.sock"
shell_e2e_stub_listener "$sock" reply --reply '{"status":"ok"}' --requests "$requests"
export GPY_AGENT_SOCKET_PATH="$sock"

# Run "$@" with the subshell logger armed; its status lands in $logged_status.
logged_status=0
count_subshells() {
    : >"$subshells"
    set -T
    trap '(( BASH_SUBSHELL > 0 )) && printf "%s\n" "${BASHPID:-depth$BASH_SUBSHELL}" >>"$subshells"' DEBUG
    "$@"
    logged_status=$?
    trap - DEBUG
    set +T
}

distinct_lines() {
    local line seen=" " n=0
    while IFS= read -r line; do
        [[ "$seen" == *" $line "* ]] && continue
        seen+="$line "
        n=$((n + 1))
    done <"$1"
    printf '%s' "$n"
}

lines_in() {
    local line n=0
    while IFS= read -r line; do n=$((n + 1)); done <"$1"
    printf '%s' "$n"
}

# --- OUTVAR: one socat, at most one subshell, reply assigned, nothing printed.
: >"$socat_runs"
reply="stale"
printed="$SANDBOX/printed"
count_subshells __gpy_send_json '{"op":"ping"}' "" reply >"$printed"
expect_eq "OUTVAR send succeeds" "$logged_status" "0"
expect_eq "OUTVAR receives the reply" "$reply" '{"status":"ok"}'
expect_eq "OUTVAR prints nothing" "$(<"$printed")" ""
expect_eq "OUTVAR send runs socat once" "$(lines_in "$socat_runs")" "1"
expect_eq "OUTVAR send uses at most one subshell" "$(( $(distinct_lines "$subshells") <= 1 ))" "1"
expect_eq "the request bytes are unchanged" "$(<"$requests")" '{"op":"ping"}'

# --- __gpy_request prints the segment output through the stdout form.
shell_e2e_stub_listener "$SANDBOX/request.sock" reply --reply '{"status":"segment"}'
export GPY_AGENT_SOCKET_PATH="$SANDBOX/request.sock"
: >"$socat_runs"
out="$(__gpy_request directory "$PWD" bash-prompt true "" true)"
expect_eq "__gpy_request prints the reply" "$out" '{"status":"segment"}'
expect_eq "__gpy_request runs socat once" "$(lines_in "$socat_runs")" "1"

test_finish "one socat and at most one subshell per request"

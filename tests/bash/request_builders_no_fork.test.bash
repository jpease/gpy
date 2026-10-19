#!/usr/bin/env bash
# tests/bash/request_builders_no_fork.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The per-prompt request builders write into a caller-named variable and run
# nothing in a subshell: a DEBUG trap inherited via functrace logs every
# command run with BASH_SUBSHELL > 0, i.e. inside a `$(...)` fork. A `lang`
# request with flags, prev_bg and a forwarded virtualenv goes through every
# helper (__gpy_json_flags_tail_into, __gpy_lang_venv_json_into,
# __gpy_forwarded_venv_into, __gpy_escape_json_into). The bytes themselves are
# pinned by the shared vector tests.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
# shellcheck source=bash/core/ipc.bash
source bash/core/ipc.bash

forks="$(mktemp "${TMPDIR:-/tmp}/gpy-builder-forks.XXXXXX")"
trap 'rm -f "$forks"' EXIT

# no_fork LABEL CMD...: run CMD under the fork-logging trap.
no_fork() {
    local label="$1"
    shift
    : >"$forks"
    set -T
    trap '(( BASH_SUBSHELL > 0 )) && printf "%s\n" "$BASH_COMMAND" >>"$forks"' DEBUG
    "$@"
    trap - DEBUG
    set +T
    if [[ -s "$forks" ]]; then
        fail "$label forks: $(<"$forks")"
    else
        pass "$label forks no subshell"
    fi
}

unset CONDA_PREFIX CONDA_DEFAULT_ENV
# The trap must see a fork, or every check below passes vacuously.
__probe_fork() {
    : "$(printf probe)"
}
: >"$forks"
set -T
trap '(( BASH_SUBSHELL > 0 )) && printf "%s\n" "$BASH_COMMAND" >>"$forks"' DEBUG
__probe_fork
trap - DEBUG
set +T
if [[ -s "$forks" ]]; then
    pass "the fork probe sees a \$(...) inside a function"
else
    fail "the DEBUG trap does not see subshell commands; the checks below would prove nothing"
fi

export VIRTUAL_ENV='/v "env"'

tail=""
no_fork "__gpy_json_flags_tail_into" __gpy_json_flags_tail_into tail true true 'bl"ue'
[[ "$tail" == ',"is_last":true,"is_first":true,"prev_bg":"bl\"ue"' ]] ||
    fail "__gpy_json_flags_tail_into wrote [$tail]"

request=""
no_fork "__gpy_build_data_request_into (lang)" \
    __gpy_build_data_request_into request lang '/tmp/a "b"' bash-prompt true blue true
[[ "$request" == '{"op":"lang","cwd":"/tmp/a \"b\"","format":"bash-prompt","is_last":true,"is_first":true,"prev_bg":"blue","virtual_env":"/v \"env\""}' ]] ||
    fail "__gpy_build_data_request_into wrote [$request]"

unset VIRTUAL_ENV
export CONDA_PREFIX=/opt/conda/envs/x CONDA_DEFAULT_ENV=x
venv_json=""
no_fork "__gpy_lang_venv_json_into (conda)" __gpy_lang_venv_json_into venv_json
[[ "$venv_json" == ',"virtual_env":"/opt/conda/envs/x"' ]] ||
    fail "__gpy_lang_venv_json_into wrote [$venv_json]"

test_finish "the bash request builders never fork"

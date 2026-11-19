#!/usr/bin/env bash

# tests/bash/basic.test.bash
# Sourcing gpy.bash picks a duration clock for this Bash and the prompt hook
# draws a PS1 through the gpy binary.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
basic_xdg_root="$(mktemp -d "${TMPDIR:-/tmp}/gpy-basic-xdg.XXXXXX")"
trap 'rm -rf "$basic_xdg_root"' EXIT
# shellcheck source=tests/lib/supervisor_off.bash
source "$ROOT/tests/lib/supervisor_off.bash" "$basic_xdg_root"
mkdir -p "$basic_xdg_root/runtime"
chmod 700 "$basic_xdg_root/runtime"
export XDG_RUNTIME_DIR="$basic_xdg_root/runtime" GPY_AGENT_SOCKET_PATH="$basic_xdg_root/none.sock"
export PATH="$ROOT/gpy-agent/target/debug:$PATH"

# shellcheck source=bash/gpy.bash
source bash/gpy.bash

case "${BASH_VERSINFO[0]}" in
    3) want=none ;;
    4) want="date|seconds" ;;
    *) want=epochrealtime ;;
esac
# shellcheck disable=SC2154 # set by bash/gpy.bash
if [[ "$__gpy_clock" =~ ^($want)$ ]]; then
    pass "duration clock for Bash ${BASH_VERSINFO[0]}: $__gpy_clock"
else
    fail "duration clock for Bash ${BASH_VERSINFO[0]}: want $want, got '$__gpy_clock'"
fi

__gpy_precmd
if [[ -n "$PS1" && "$PS1" != '❯ ' ]]; then
    pass "gpy prompt drew PS1"
else
    fail "gpy prompt drew PS1: got [$PS1]"
fi

test_finish "basic"

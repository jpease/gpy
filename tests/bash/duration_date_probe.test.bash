#!/usr/bin/env bash
# tests/bash/duration_date_probe.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Bash 4 lacks EPOCHREALTIME; `date +%s%N` only works on GNU date (BSD prints
# a literal `N`), so gpy.bash picks its duration clock by probing `date` once
# at source time (here through a PATH shim). Bash 5 uses EPOCHREALTIME and
# Bash 3 measures nothing. Every clock yields sane milliseconds.
#
# The Bash 4 probe needs a Bash 4: set GPY_TEST_BASH4 to one (or put `bash4`
# on PATH); without it that part only prints a note.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-duration-probe.XXXXXX")"
trap 'rm -rf "$SANDBOX"' EXIT
# shellcheck source=tests/lib/supervisor_off.bash
source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"

real_date="$(command -v date)"
mkdir -p "$SANDBOX/bsd" "$SANDBOX/gnu"
# BSD date: %N is not a conversion, so it is printed as the letter N.
cat >"$SANDBOX/bsd/date" <<EOF
#!/bin/sh
if [ "\$1" = "+%s%N" ]; then
    echo "\$("$real_date" +%s)N"
else
    exec "$real_date" "\$@"
fi
EOF
# GNU date: nanoseconds, nine digits after the seconds.
cat >"$SANDBOX/gnu/date" <<EOF
#!/bin/sh
if [ "\$1" = "+%s%N" ]; then
    echo "\$("$real_date" +%s)123456789"
else
    exec "$real_date" "\$@"
fi
EOF
chmod +x "$SANDBOX/bsd/date" "$SANDBOX/gnu/date"

# clock_with BASH SHIM_DIR: the clock gpy.bash picks when sourced by BASH
# with SHIM_DIR first on PATH.
clock_with() {
    # shellcheck disable=SC2016 # expanded by the child shell
    PATH="$2:$PATH" "$1" --noprofile --norc -c \
        'source "$0/bash/gpy.bash"; printf "%s" "$__gpy_clock"' "$ROOT" 2>/dev/null
}

bash_major() {
    # shellcheck disable=SC2016 # expanded by the child shell
    "$1" -c 'printf "%s" "${BASH_VERSINFO[0]}"'
}

for b in /bin/bash "$BASH"; do
    case "$(bash_major "$b")" in
        3) check "Bash 3 ($b) measures nothing" none "$(clock_with "$b" "$SANDBOX/gnu")" ;;
        4) ;;
        *) check "Bash 5+ ($b) uses EPOCHREALTIME whatever date does" epochrealtime "$(clock_with "$b" "$SANDBOX/bsd")" ;;
    esac
done

bash4="${GPY_TEST_BASH4:-$(command -v bash4 2>/dev/null)}"
if [[ -n "$bash4" && "$(bash_major "$bash4")" == 4 ]]; then
    check "Bash 4 with BSD date falls back to whole seconds" seconds "$(clock_with "$bash4" "$SANDBOX/bsd")"
    check "Bash 4 with GNU date keeps nanosecond timing" date "$(clock_with "$bash4" "$SANDBOX/gnu")"
else
    # Not a skip: no CI image ships a Bash 4, and the rest still ran.
    echo "NOTE: no Bash 4 (GPY_TEST_BASH4 or bash4 on PATH): the date probe is not exercised"
fi

# shellcheck source=bash/gpy.bash
source bash/gpy.bash

# elapsed CLOCK START END: what __gpy_elapsed_into yields on CLOCK.
elapsed() {
    local __gpy_clock="$1" ms=""
    __gpy_elapsed_into "$2" "$3" ms || ms=rejected
    printf '%s' "$ms"
}

check "whole-second clock: a 2 s command is 2000 ms" 2000 "$(elapsed seconds 1000 1002)"
check "date clock: nanoseconds become milliseconds" 1500 "$(elapsed date 1000000000000000000 1000000001500000000)"
check "no clock measures nothing" rejected "$(elapsed none 1 2)"

test_finish "date probe duration"

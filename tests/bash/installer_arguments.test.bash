#!/usr/bin/env bash
# tests/bash/installer_arguments.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Both installers accept only -h/--help. Any other argument exits 2 before a
# download or a tool check; --help exits 0 and lists the environment variables
# that configure that installer. install.sh needs bash and says so under dash.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

WORKDIR="$(mktemp -d)"
trap 'rm -rf "$WORKDIR"' EXIT

# Network tools that record any call: an argument error must fetch nothing.
FAKEBIN="$WORKDIR/fakebin"
CALLS="$WORKDIR/calls.log"
mkdir -p "$FAKEBIN" "$WORKDIR/home"
: >"$CALLS"
for tool in curl wget; do
    printf '#!/bin/sh\necho "%s $*" >>"%s"\nexit 1\n' "$tool" "$CALLS" >"$FAKEBIN/$tool"
    chmod +x "$FAKEBIN/$tool"
done

# Echoes "<exit status>|<stdout and stderr>".
run() {
    local out status=0
    out="$(HOME="$WORKDIR/home" PATH="$FAKEBIN:$PATH" "$@" 2>&1 </dev/null)" || status=$?
    printf '%s|%s' "$status" "$out"
}

for installer in install-oneline.sh install.sh; do
    runner="sh"
    [[ "$installer" = install.sh ]] && runner=bash
    result="$(run "$runner" "$ROOT/$installer" --yes)"
    [[ "${result%%|*}" -eq 2 ]] ||
        fail "$installer --yes exited ${result%%|*}, expected 2"
    case "${result#*|}" in
        *"unknown argument: --yes"*) pass "$installer --yes: exit 2 with a named argument" ;;
        *) fail "$installer --yes: no 'unknown argument' message: ${result#*|}" ;;
    esac
    [[ ! -s "$CALLS" ]] || fail "$installer --yes reached the network: $(cat "$CALLS")"
    [[ ! -e "$WORKDIR/home/.local" ]] || fail "$installer --yes installed something"
done

result="$(run sh "$ROOT/install-oneline.sh" --help)"
[[ "${result%%|*}" -eq 0 ]] || fail "install-oneline.sh --help exited ${result%%|*}"
for var in GPY_SHELL GPY_VERSION GPY_NERD_FONT; do
    case "${result#*|}" in
        *"$var"*) pass "install-oneline.sh --help lists $var" ;;
        *) fail "install-oneline.sh --help does not list $var" ;;
    esac
done

result="$(run bash "$ROOT/install.sh" --help)"
[[ "${result%%|*}" -eq 0 ]] || fail "install.sh --help exited ${result%%|*}"
for var in GPY_SHELL GPY_NERD_FONT; do
    case "${result#*|}" in
        *"$var"*) pass "install.sh --help lists $var" ;;
        *) fail "install.sh --help does not list $var" ;;
    esac
done
case "${result#*|}" in
    *GPY_VERSION*) fail "install.sh --help lists GPY_VERSION, which it never reads" ;;
    *) pass "install.sh --help omits GPY_VERSION" ;;
esac

[[ ! -s "$CALLS" ]] || fail "--help reached the network: $(cat "$CALLS")"

if command -v dash >/dev/null 2>&1; then
    result="$(run dash "$ROOT/install.sh")"
    [[ "${result%%|*}" -eq 1 ]] || fail "dash install.sh exited ${result%%|*}, expected 1"
    case "${result#*|}" in
        *"run this installer with bash"*) pass "dash install.sh asks for bash" ;;
        *) fail "dash install.sh did not say to use bash: ${result#*|}" ;;
    esac
else
    test_skip_part "dash not installed; skipping the non-bash guard check"
fi

test_finish "installer arguments"

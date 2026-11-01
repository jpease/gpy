#!/usr/bin/env zsh
# tests/zsh/prompt_reload.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# gpy-prompt-reload re-reads config.toml into the running shell. It always runs
# `gpy-agent theme export` (never the theme-export cache file, which a stopped
# agent leaves stale), is silent on success, and on failure prints exactly
# `gpy: theme reload failed; run gpy doctor` and returns 1. The doorbell reload
# (`__gpy_reload_config` without an argument) still reads the cache file first.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'

shell_e2e_init "$ROOT"
export GPY_AGENT_ENABLED=0
export GPY_AGENT_SUPERVISOR_ENABLED=0

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-prompt-reload.XXXXXX")
trap 'rm -rf "$tmp"; shell_e2e_cleanup' EXIT
mkdir -p "$XDG_CACHE_HOME/gpy" "$tmp/stub"
cache="$XDG_CACHE_HOME/gpy/theme-export.zsh"
err="$tmp/stderr"
want_failure="gpy: theme reload failed; run gpy doctor"

write_config() {
    printf '[agent]\nenabled = false\n\n[ui]\ntheme = "text"\nshow_icons = false\nenabled_segments = [%s]\n' "$1" \
        >"$XDG_CONFIG_HOME/gpy/config.toml"
}

# The stale cache: what the daemon wrote from an older config (`clock` only),
# first line included.
write_config '"clock"'
"$SHELL_E2E_AGENT_BIN" theme export --shell zsh >"$cache"

# A fresh Zsh that loaded gpy.zsh (reading the stale cache), then $1. The
# reload's own stderr goes to $err. PATH_OVERRIDE, when set, replaces PATH.
run_zsh() {
    (
        [[ -z "${PATH_OVERRIDE:-}" ]] || PATH="$PATH_OVERRIDE"
        export PATH ROOT err
        zsh -f -c 'source "$ROOT/zsh/gpy.zsh"; '"$1"
    )
}

# Line N of $out.
line() { print -r -- "${${(f)out}[$1]}"; }

echo "--- the cache is stale, the agent is stopped ---"
write_config '"directory", "git"'
out=$(run_zsh '
    echo "start=${__enabled_segments[*]}"
    gpy-prompt-reload 2>"$err"
    echo "rc=$?"
    echo "after=${__enabled_segments[*]}"
    __gpy_reload_config
    echo "doorbell=${__enabled_segments[*]}"
')
check "the integration starts from the stale cache" "start=clock" "$(line 1)"
check "gpy-prompt-reload returns 0" "rc=0" "$(line 2)"
check "gpy-prompt-reload applies the current config.toml" "after=directory git" "$(line 3)"
check "gpy-prompt-reload prints nothing on success" "" "$(<"$err")"
check "the doorbell reload still reads the cache file" "doorbell=clock" "$(line 4)"

echo "--- gpy-agent is missing ---"
PATH_OVERRIDE="/usr/bin:/bin"
out=$(run_zsh '
    echo "scripts=${__gpy_script_segments-unset}"
    gpy-prompt-reload 2>"$err"
    echo "rc=$?"
    echo "after=${__enabled_segments[*]}"
    echo "scripts=${__gpy_script_segments-unset}"
')
check "a missing agent returns 1" "rc=1" "$(line 2)"
check "a missing agent leaves __enabled_segments alone" "after=clock" "$(line 3)"
check "a missing agent leaves the script segment list alone" "$(line 1)" "$(line 4)"
check "a missing agent prints exactly the failure line" "$want_failure" "$(<"$err")"

echo "--- gpy-agent exits 1 ---"
printf '#!/bin/sh\nexit 1\n' >"$tmp/stub/gpy-agent"
chmod +x "$tmp/stub/gpy-agent"
PATH_OVERRIDE="$tmp/stub:/usr/bin:/bin"
out=$(run_zsh '
    echo "scripts=${__gpy_script_segments-unset}"
    gpy-prompt-reload 2>"$err"
    echo "rc=$?"
    echo "after=${__enabled_segments[*]}"
    echo "scripts=${__gpy_script_segments-unset}"
')
check "a failing agent returns 1" "rc=1" "$(line 2)"
check "a failing agent leaves __enabled_segments alone" "after=clock" "$(line 3)"
check "a failing agent leaves the script segment list alone" "$(line 1)" "$(line 4)"
check "a failing agent prints exactly the failure line" "$want_failure" "$(<"$err")"
PATH_OVERRIDE=""

echo "--- an invalid config.toml ---"
printf '[ui\nenabled_segments = \n' >"$XDG_CONFIG_HOME/gpy/config.toml"
out=$(run_zsh '
    gpy-prompt-reload 2>"$err"
    echo "rc=$?"
    echo "after=${__enabled_segments[*]}"
')
default_segments=$(grep -v -e '^#' -e '^[[:space:]]*$' "$ROOT/tests/fixtures/default_segments.txt")
check "an invalid config.toml still returns 0" "rc=0" "$(line 1)"
check "an invalid config.toml applies the built-in defaults" "after=$default_segments" "$(line 2)"
if grep -q 'Warning: \[Config loading\]' "$err"; then
    pass "the agent's config warning reaches stderr"
else
    fail "the agent's config warning is missing: $(<"$err")"
fi

test_finish "gpy-prompt-reload applies the current config in Zsh"

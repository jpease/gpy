#!/usr/bin/env bash
# tests/bash/prompt_exec_parity.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# One `gpy prompt bash` exec draws PS1 (#1247). With the
# agent up it must print exactly the PS1 __gpy_render_prompt prints: for every
# builtin theme, for an overlay that drops the clock and status templates (so
# the shell's local pills draw), on success and failure, with and without a
# duration pill, inside a repository and outside one, and with script
# segments (escaped text, an icon, a variant, an own close cap). The exec path
# runs a script once per command, and its circuit breaker trips as the
# shell's does. With the agent off it renders git through its in-process
# oneshot, and a segment it cannot draw sends the render back to the shell.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1
shell_e2e_init "$ROOT"
test_require_command gpy "the debug build provides gpy"

SEGMENTS='"clock", "duration", "status", "language", "directory", "git"'
mkdir -p "$XDG_CONFIG_HOME/gpy/themes"
printf 'extends = "default"\nunset = ["segments.clock.format", "segments.status.format"]\n' \
    >"$XDG_CONFIG_HOME/gpy/themes/pills.toml"

write_config() {
    printf '[ui]\ntheme = "%s"\nenabled_segments = [%s]\n\n[agent.supervisor]\nenabled = false\n' \
        "$1" "$2" >"$XDG_CONFIG_HOME/gpy/config.toml"
}

write_config default "$SEGMENTS"
shell_e2e_start_agent || exit 1
# No refresh races between the two renders of one comparison.
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=3600 GPY_LANGUAGE_CACHE_TTL_SECONDS=3600
unset PROMPT_COMMAND
source bash/gpy.bash

PLAIN_DIR="$SHELL_E2E_ROOT/plain"
mkdir -p "$PLAIN_DIR"

markers_written() {
    local key dir="$XDG_CACHE_HOME/gpy/instant-prompts"
    __gpy_path_to_cache_key "$(cd "$SHELL_E2E_REPO" && pwd -P)" key
    [[ -f "$dir/$key.git.any-entry.bash" && -f "$dir/$key.lang.any-entry.bash" ]]
}

# One comparison: LABEL STATUS DURATION_MS ("" before the first command).
compare() {
    local label="$1" shell_ps1
    __gpy_cmd_duration="$3"
    __gpy_render_prompt "$2"
    shell_ps1="$PS1"
    PS1=""
    if ! __gpy_render_prompt_exec "$2"; then
        fail "$label: the exec path declined"
        return
    fi
    check "$label" "$shell_ps1" "$PS1"
    # Parity of two empty or broken prompts would prove nothing.
    [[ "$PS1" == *"${PWD##*/}"* ]] || fail "$label: no directory in PS1 [$PS1]"
}

for theme in default text starship pills; do
    write_config "$theme" "$SEGMENTS"
    "$SHELL_E2E_AGENT_BIN" config reload >/dev/null 2>&1
    gpy-prompt-reload || fail "$theme: shell reload"
    for dir in "$SHELL_E2E_REPO" "$PLAIN_DIR"; do
        cd "$dir" || exit 1
        __gpy_resolve_pwd
        __gpy_dir_cache_key="" __gpy_char_cache_key="" __gpy_clock_cache_key="" __gpy_status_cache_key=""
        # Warm the agent's caches for this theme before comparing.
        __gpy_render_prompt 0
        [[ "$dir" == "$SHELL_E2E_REPO" ]] && shell_e2e_poll 5 markers_written
        __gpy_render_prompt 0
        where="repo"
        [[ "$dir" == "$PLAIN_DIR" ]] && where="plain dir"
        if [[ "$dir" == "$SHELL_E2E_REPO" ]]; then
            check "$theme, repo: git drawn" yes "$([[ "$PS1" == *main* ]] && echo yes || echo no)"
        fi
        if [[ "$theme" == pills ]]; then
            check "$theme, $where: local clock pill drawn" yes "$([[ "$PS1" == *'\D{'* ]] && echo yes || echo no)"
        fi
        compare "$theme, $where, success" 0 ""
        compare "$theme, $where, failure" 1 ""
        compare "$theme, $where, slow command" 0 2500
    done
done

# Agent off: git still renders, through the in-process oneshot.
cd "$SHELL_E2E_REPO" || exit 1
write_config default "$SEGMENTS"
gpy-prompt-reload
PS1=""
GPY_AGENT_ENABLED=0 __gpy_render_prompt_exec 0
check "agent off: the exec path renders the branch" yes \
    "$([[ "$PS1" == *main* ]] && echo yes || echo "no (PS1=[$PS1])")"

# Script segments: one plugin, styled by a theme overlay.
PLUG="$XDG_CONFIG_HOME/gpy/plugins/ptest"
MARKS="$SHELL_E2E_ROOT/pseg-runs"
mkdir -p "$PLUG/bin" "$PLUG/segments"
printf 'id = "ptest"\nname = "ptest"\nversion = "0.1.0"\napi_version = "v1"\nentry_type = "script"\nprovided_segments = ["pseg", "pvar", "pslow"]\n' \
    >"$PLUG/plugin.toml"
printf 'command = "bin/pseg.sh"\ninputs = ["pwd"]\nenv = ["PTEST_MARKS"]\nttl_secs = 0\ntimeout_ms = 500\n' \
    >"$PLUG/segments/pseg.toml"
printf 'command = "bin/pvar.sh"\nttl_secs = 3600\ntimeout_ms = 500\n' >"$PLUG/segments/pvar.toml"
printf 'command = "bin/pslow.sh"\nttl_secs = 0\ntimeout_ms = 50\n' >"$PLUG/segments/pslow.toml"
# shellcheck disable=SC2016  # the scripts expand their own variables
printf '#!/bin/sh\nprintf . >>"$PTEST_MARKS"\nprintf '\''in %%s $HOME `x` \\\\\\n'\'' "${GPY_PWD##*/}"\n' \
    >"$PLUG/bin/pseg.sh"
printf '#!/bin/sh\nprintf '\''v1\\nwarn\\n'\''\n' >"$PLUG/bin/pvar.sh"
printf '#!/bin/sh\nexec sleep 1\n' >"$PLUG/bin/pslow.sh"
chmod -R go-w "$PLUG"
export PTEST_MARKS="$MARKS"
cat >"$XDG_CONFIG_HOME/gpy/themes/scripts.toml" <<'EOF'
extends = "default"
[segments.pseg]
icon = "p$"
bg_color = "blue"
text_color = "white"
[segments.pseg.close]
icon = ">"
icon_color = "match_bg"
bg_color = "black"
[segments.pvar]
bg_color = "green"
warn_bg_color = "yellow"
warn_icon = "w`"
EOF

runs() {
    if [[ -f "$MARKS" ]]; then wc -c <"$MARKS" | tr -d ' '; else echo 0; fi
}
# `gpy prompt` with this shell's script arguments and command token $1.
exec_prompt() {
    gpy prompt bash --status 0 --pid "$$" --bash "$BASH" --runner "$ROOT/bash/core/g_runner.bash" \
        --cmd "$1" "${@:2}"
}

cd "$SHELL_E2E_REPO" || exit 1
for theme in scripts starship; do
    write_config "$theme" '"pseg", "directory", "pvar", "git"'
    "$SHELL_E2E_AGENT_BIN" config reload >/dev/null 2>&1
    gpy-prompt-reload || fail "$theme: shell reload"
    __gpy_resolve_pwd
    __gpy_dir_cache_key="" __gpy_char_cache_key=""
    __gpy_g_first=1
    __gpy_g_maybe_run
    __gpy_render_prompt 0
    shell_ps1="$PS1"
    PS1=""
    __gpy_g_first=1
    __gpy_render_prompt_exec 0
    check "$theme, script segments: the exec path renders" 0 "$?"
    check "$theme, script segments: PS1 matches the shell renderer" "$shell_ps1" "$PS1"
    [[ "$PS1" == *'in '"${SHELL_E2E_REPO##*/}"* && "$PS1" == *v1* ]] ||
        fail "$theme: script text missing from PS1 [$PS1]"
    if [[ "$theme" == scripts ]]; then
        # Escaped text, the variant's icon and the own close cap.
        # shellcheck disable=SC2016,SC1003  # literal PS1 escapes
        [[ "$PS1" == *'p\\$ in '*' \\$HOME \\`x\\` \\\\'* && "$PS1" == *'w\\` v1'* && "$PS1" == *'>'* ]] ||
            fail "$theme: script pill styling missing from PS1 [$PS1]"
    fi
done

# Once per command: a redraw (same token, same directory) runs nothing; a
# new command or a `cd` runs the script again.
before=$(runs)
exec_prompt 41 --reload >/dev/null
exec_prompt 41 >/dev/null
check "script: a redraw runs nothing" $((before + 1)) "$(runs)"
exec_prompt 42 >/dev/null
check "script: a new command runs it again" $((before + 2)) "$(runs)"
cd "$PLAIN_DIR" || exit 1
exec_prompt 42 >/dev/null
check "script: a cd runs it again" $((before + 3)) "$(runs)"
cd "$SHELL_E2E_REPO" || exit 1

# Bookkeeping: `gpy prompt` registers this shell with the agent (which then
# accepts its workspace updates) and records the directory it last sent.
session_file="$XDG_RUNTIME_DIR/gpy/refresh-throttle/$$.session"
rm -f "$session_file"
cd "$SHELL_E2E_REPO" || exit 1
exec_prompt 60 >/dev/null
reply=""
__gpy_send_json "{\"op\":\"workspace\",\"pid\":$$,\"cwd\":\"$SHELL_E2E_REPO\"}" "" reply
check "bookkeeping: the agent lists the shell gpy prompt registered" yes \
    "$([[ $reply != *error* && -n $reply ]] && echo yes || echo "no ($reply)")"
cd "$PLAIN_DIR" || exit 1
exec_prompt 60 >/dev/null
check "bookkeeping: a cd sends the new workspace" yes \
    "$(grep -q "\"workspace\":\"$PLAIN_DIR\"" "$session_file" && echo yes || echo "no ($(cat "$session_file"))")"
cd "$SHELL_E2E_REPO" || exit 1

# Circuit breaker: the second timeout in a row disables the segment, with
# one line on stderr, until the next theme load.
write_config scripts '"pslow", "directory"'
started=${EPOCHREALTIME-}
exec_prompt 50 --reload >/dev/null 2>"$SHELL_E2E_ROOT/strike1"
if [[ -n $started ]]; then
    # The script sleeps 1 s with timeout_ms = 50: the prompt must not wait it out.
    elapsed_ms=$(((${EPOCHREALTIME/[.,]/} - ${started/[.,]/}) / 1000))
    if ((elapsed_ms < 500)); then
        pass "deadline: a hung script costs the prompt its timeout, not its run (${elapsed_ms} ms)"
    else
        fail "deadline: gpy prompt waited ${elapsed_ms} ms for a script with timeout_ms = 50"
    fi
fi
exec_prompt 51 >/dev/null 2>"$SHELL_E2E_ROOT/strike2"
exec_prompt 52 >/dev/null 2>"$SHELL_E2E_ROOT/strike3"
check "breaker: no notice after one timeout" "" "$(cat "$SHELL_E2E_ROOT/strike1")"
check "breaker: the second timeout disables the segment" \
    "gpy: script segment pslow disabled for this shell after 2 failures (timeout or output over 4096 bytes)" \
    "$(cat "$SHELL_E2E_ROOT/strike2")"
check "breaker: the notice is printed once" "" "$(cat "$SHELL_E2E_ROOT/strike3")"

# Without the script arguments, or with a segment it cannot draw, the
# binary hands the render back (exit 3).
write_config default '"directory", "pseg"'
check "script segment without script arguments: exit 3" 3 \
    "$(gpy prompt bash --status 0 --pid $$ >/dev/null 2>&1; echo $?)"
write_config default '"directory", "username"'
check "unknown segment: the binary declines with exit 3" 3 \
    "$(gpy prompt bash --status 0 --pid $$ >/dev/null 2>&1; echo $?)"

# Supervisor: with the agent stopped, a prompt past the check interval
# starts it again (the gpy-agent beside gpy).
write_config default '"directory"'
shell_e2e_stop_agent
agent_up() { [[ -S "$GPY_AGENT_SOCKET_PATH" ]]; }
GPY_AGENT_SUPERVISOR_ENABLED=1 GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0 \
    gpy prompt bash --status 0 --pid $$ >/dev/null 2>&1
if shell_e2e_poll 10 agent_up; then
    pass "supervisor: a prompt restarted the stopped agent"
else
    fail "supervisor: the agent did not come back after a prompt"
fi

test_finish "gpy prompt bash matches the shell renderer"

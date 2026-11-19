#!/usr/bin/env zsh
# tests/zsh/supervisor_respects_config.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The per-prompt supervisor check (run by `gpy prompt zsh`) honours the
# [agent.supervisor] values in config.toml: enabled, check_interval_seconds,
# max_restart_attempts.
#
# A copy of gpy runs beside a stub gpy-agent that logs argv; the socket never
# exists, so every check that is due starts the agent. Every case counts the
# shell's startup start (its first prompt, which is also the first check).

ROOT=${0:a:h:h:h}
cd "$ROOT" || exit 1
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
test_require_command gpy

tmp=$(mktemp -d "${TMPDIR:-/tmp}/gpy-sup-config.XXXXXX") || exit 1
tmp=${tmp:A}
trap 'rm -rf "$tmp"' EXIT
mkdir -p "$tmp/bin" "$tmp/home" "$tmp/cfg/gpy" "$tmp/cache"

cp "${commands[gpy]}" "$tmp/bin/gpy"
cat >"$tmp/bin/gpy-agent" <<EOF
#!/bin/sh
echo "\$*" >> "$tmp/calls.log"
exit 0
EOF
chmod +x "$tmp/bin/gpy-agent"

# run_case SUPERVISOR_TOML PROMPTS [VAR=value...]: one clean zsh draws PROMPTS
# prompts. Paths reach the child as data (env), never interpolated into -c
# code. The trailing `sleep 0.5` lets the detached starts reach the log; a
# start that must not happen cannot be polled for.
run_case() {
    local sup=$1 prompts=$2
    shift 2
    print -r -- $'[agent]\nenabled = true\n\n[agent.supervisor]\n'"$sup" >"$tmp/cfg/gpy/config.toml"
    rm -rf "$tmp/run"
    mkdir -p "$tmp/run"
    : >"$tmp/calls.log"
    env -i PATH="$tmp/bin:/usr/bin:/bin" HOME="$tmp/home" TERM=dumb \
        XDG_CONFIG_HOME="$tmp/cfg" XDG_CACHE_HOME="$tmp/cache" \
        XDG_RUNTIME_DIR="$tmp/run" TMPDIR="$tmp" CALLS_LOG="$tmp/calls.log" \
        GPY_AGENT_SOCKET_PATH="$tmp/missing.sock" PROMPTS="$prompts" ROOT="$ROOT" "$@" \
        zsh -f -c '
            source "$ROOT/zsh/gpy.zsh" >/dev/null 2>&1
            for __i in {1..$PROMPTS}; do __gpy_precmd; done
            sleep 0.5' >/dev/null 2>&1
}

count_calls() {
    grep -c "^$1" "$tmp/calls.log" 2>/dev/null
}

# Case 1: the supervisor disabled in config.toml never restarts, even when
# a check is due on every prompt: only the startup start.
run_case 'enabled = false' 4 GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0
check "[agent.supervisor] enabled = false stops the per-prompt check" 1 "$(count_calls start)"

# Case 2: check_interval_seconds rate-limits the check: the startup start is
# the first check, so prompts within the next 60 s start nothing more.
run_case $'enabled = true\ncheck_interval_seconds = 60\nmax_restart_attempts = 5' 4
check "check_interval_seconds = 60 allows no second check within the window" 1 "$(count_calls start)"

# Case 3: max_restart_attempts caps restarts: the startup start plus one.
run_case $'enabled = true\nmax_restart_attempts = 1' 4 GPY_AGENT_SUPERVISOR_CHECK_INTERVAL_SECONDS=0
check "max_restart_attempts = 1 allows one restart" 2 "$(count_calls start)"

test_finish "the supervisor honours [agent.supervisor]"

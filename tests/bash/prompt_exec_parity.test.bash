#!/usr/bin/env bash
# tests/bash/prompt_exec_parity.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY_PROMPT_EXEC=1 hands PS1 to one `gpy prompt bash` exec (#1247). With the
# agent up it must print exactly the PS1 __gpy_render_prompt prints: for every
# builtin theme, for an overlay that drops the clock and status templates (so
# the shell's local pills draw), on success and failure, with and without a
# duration pill, inside a repository and outside one. With the agent off it
# renders git through its in-process oneshot, and a script segment sends the
# render back to the shell path.

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
unset GPY_PROMPT_EXEC PROMPT_COMMAND
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
    if ! GPY_PROMPT_EXEC=1 __gpy_render_prompt_exec "$2"; then
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
GPY_AGENT_ENABLED=0 GPY_PROMPT_EXEC=1 __gpy_render_prompt_exec 0
check "agent off: the exec path renders the branch" yes \
    "$([[ "$PS1" == *main* ]] && echo yes || echo "no (PS1=[$PS1])")"

# A segment the binary cannot draw hands the render back (exit 3).
write_config default '"directory", "username"'
check "script segment: the binary declines with exit 3" 3 \
    "$(gpy prompt bash --status 0 --pid $$ >/dev/null 2>&1; echo $?)"

test_finish "gpy prompt bash matches the shell renderer"

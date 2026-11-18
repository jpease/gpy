#!/usr/bin/env zsh
# tests/zsh/prompt_exec_parity.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY_PROMPT_EXEC=1 hands PROMPT to one `gpy prompt zsh` exec (#1247), the
# twin of tests/bash/prompt_exec_parity.test.bash. With the agent up it must
# print exactly the PROMPT and the transient-prompt tail __gpy_render_prompt
# prints: for every builtin theme, for an overlay that drops the clock and
# status templates (so the shell's local pills draw), on success and failure,
# with and without a duration pill, inside a repository and outside one, and
# with script segments (escaped text, an icon, a variant, an own close cap).
# The exec path runs a script once per command. With the agent off it renders
# git through its in-process oneshot, and a segment it cannot draw sends the
# render back to the shell.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
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
unset GPY_PROMPT_EXEC
source zsh/gpy.zsh
# Zsh runs preexec hooks before every line of a script too; this test calls
# __gpy_preexec itself where a real shell would (script_segment_fixture.zsh).
add-zsh-hook -d preexec __gpy_preexec

PLAIN_DIR="$SHELL_E2E_ROOT/plain"
mkdir -p "$PLAIN_DIR"

markers_written() {
    local key dir="$XDG_CACHE_HOME/gpy/instant-prompts"
    __gpy_path_to_cache_key "${SHELL_E2E_REPO:A}" key
    [[ -f "$dir/$key.git.any-entry.zsh" && -f "$dir/$key.lang.any-entry.zsh" ]]
}

clear_memos() {
    __gpy_dir_cache_key="" __gpy_char_cache_key="" __gpy_clock_cache_key="" __gpy_status_cache_key=""
}

# The shell renderer's PROMPT and tail.
shell_render() {
    local __gpy_emit_tail=1
    __gpy_render_prompt "$1"
    shell_out=$__gpy_render_out shell_tail=$__gpy_render_tail
}

# One comparison: LABEL STATUS DURATION_MS (0: no command timed).
compare() {
    local label=$1
    __gpy_cmd_duration=$3
    shell_render "$2"
    __gpy_render_out="" __gpy_render_tail=""
    if ! GPY_PROMPT_EXEC=1 __gpy_render_prompt_exec "$2"; then
        fail "$label: the exec path declined"
        return
    fi
    check "$label" "$shell_out" "$__gpy_render_out"
    check "$label: tail" "$shell_tail" "$__gpy_render_tail"
    # Parity of two empty or broken prompts would prove nothing.
    [[ $__gpy_render_out == *"${PWD:t}"* ]] || fail "$label: no directory in PROMPT [$__gpy_render_out]"
}

for theme in default text starship pills; do
    write_config "$theme" "$SEGMENTS"
    "$SHELL_E2E_AGENT_BIN" config reload >/dev/null 2>&1
    gpy-prompt-reload || fail "$theme: shell reload"
    for dir in "$SHELL_E2E_REPO" "$PLAIN_DIR"; do
        cd "$dir" || exit 1
        clear_memos
        # Warm the agent's caches for this theme before comparing.
        shell_render 0
        [[ "$dir" == "$SHELL_E2E_REPO" ]] && shell_e2e_poll 5 markers_written
        shell_render 0
        where="repo"
        [[ "$dir" == "$PLAIN_DIR" ]] && where="plain dir"
        if [[ "$dir" == "$SHELL_E2E_REPO" ]]; then
            check "$theme, repo: git drawn" yes "$([[ $shell_out == *main* ]] && echo yes || echo no)"
        fi
        if [[ "$theme" == pills ]]; then
            check "$theme, $where: local clock pill drawn" yes "$([[ $shell_out == *'%D{'* ]] && echo yes || echo no)"
        fi
        compare "$theme, $where, success" 0 0
        compare "$theme, $where, failure" 1 0
        compare "$theme, $where, slow command" 0 2500
    done
done

# Agent off: git still renders, through the in-process oneshot.
cd "$SHELL_E2E_REPO" || exit 1
write_config default "$SEGMENTS"
gpy-prompt-reload
__gpy_render_out=""
GPY_AGENT_ENABLED=0 GPY_PROMPT_EXEC=1 __gpy_render_prompt_exec 0
check "agent off: the exec path renders the branch" yes \
    "$([[ $__gpy_render_out == *main* ]] && echo yes || echo "no (PROMPT=[$__gpy_render_out])")"

# Script segments: one plugin, styled by a theme overlay.
PLUG="$XDG_CONFIG_HOME/gpy/plugins/ptest"
MARKS="$SHELL_E2E_ROOT/pseg-runs"
mkdir -p "$PLUG/bin" "$PLUG/segments"
printf 'id = "ptest"\nname = "ptest"\nversion = "0.1.0"\napi_version = "v1"\nentry_type = "script"\nprovided_segments = ["pseg", "pvar"]\n' \
    >"$PLUG/plugin.toml"
printf 'command = "bin/pseg.sh"\ninputs = ["pwd"]\nenv = ["PTEST_MARKS"]\nttl_secs = 0\ntimeout_ms = 500\n' \
    >"$PLUG/segments/pseg.toml"
printf 'command = "bin/pvar.sh"\nttl_secs = 3600\ntimeout_ms = 500\n' >"$PLUG/segments/pvar.toml"
cat >"$PLUG/bin/pseg.sh" <<'EOF'
#!/bin/sh
printf . >>"$PTEST_MARKS"
printf 'in %s $HOME `x` 50%% \\\n' "${GPY_PWD##*/}"
EOF
printf '#!/bin/sh\nprintf '\''v1\\nwarn\\n'\''\n' >"$PLUG/bin/pvar.sh"
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

cd "$SHELL_E2E_REPO" || exit 1
for theme in scripts starship; do
    write_config "$theme" '"pseg", "directory", "pvar", "git"'
    "$SHELL_E2E_AGENT_BIN" config reload >/dev/null 2>&1
    gpy-prompt-reload || fail "$theme: shell reload"
    clear_memos
    __gpy_g_due=1
    __gpy_g_maybe_run
    shell_render 0
    __gpy_render_out="" __gpy_render_tail=""
    __gpy_g_reloaded=1
    GPY_PROMPT_EXEC=1 __gpy_render_prompt_exec 0
    check "$theme, script segments: the exec path renders" 0 "$?"
    check "$theme, script segments: PROMPT matches the shell renderer" "$shell_out" "$__gpy_render_out"
    [[ $__gpy_render_out == *"in ${SHELL_E2E_REPO:t}"* && $__gpy_render_out == *v1* ]] ||
        fail "$theme: script text missing from PROMPT [$__gpy_render_out]"
    if [[ "$theme" == scripts ]]; then
        # Escaped text, the variant's icon and the own close cap.
        [[ $__gpy_render_out == *'p\$ in '*' \$HOME \`x\` 50%% \\'* && $__gpy_render_out == *'w\` v1'* && $__gpy_render_out == *'>'* ]] ||
            fail "$theme: script pill styling missing from PROMPT [$__gpy_render_out]"
    fi
done

# Once per command: a repaint runs nothing; a command (preexec) runs the
# script again.
before=$(runs)
GPY_PROMPT_EXEC=1 __gpy_render_prompt_exec 0
check "script: a repaint runs nothing" "$before" "$(runs)"
__gpy_preexec
GPY_PROMPT_EXEC=1 __gpy_render_prompt_exec 0
check "script: a new command runs it again" $((before + 1)) "$(runs)"

# A segment it cannot draw hands the render back (exit 3).
write_config default '"directory", "username"'
check "unknown segment: the binary declines with exit 3" 3 \
    "$(gpy prompt zsh --status 0 --pid $$ >/dev/null 2>&1; echo $?)"

test_finish "gpy prompt zsh matches the shell renderer"

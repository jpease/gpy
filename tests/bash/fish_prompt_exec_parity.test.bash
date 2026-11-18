#!/usr/bin/env bash
# tests/bash/fish_prompt_exec_parity.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# `gpy prompt fish` draws the Fish prompt (#1247). With the agent up it must
# show what fish_prompt's own renderer shows: for every builtin theme, for an
# overlay that drops the clock and status templates (so the local pills
# draw), on success and failure, with and without a duration pill, inside a
# repository and outside one, the transient line, and script segments.
# Fish's `set_color` writes terminfo sequences and the binary plain SGR, so
# both are compared as the cells a terminal shows (tests/lib/sgr_cells.py):
# the same characters in the same colours. Driven from Bash so the agent
# helpers of tests/lib/shell_e2e.sh apply.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1
test_require_command fish "the Fish integration is under test"
test_require_command python3 "tests/lib/sgr_cells.py compares the prompts"
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
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=3600 GPY_LANGUAGE_CACHE_TTL_SECONDS=3600
# set_color writes nothing for TERM=dumb; 24-bit like the binary's default depth.
export TERM=xterm-256color COLORTERM=truecolor
PLAIN_DIR="$SHELL_E2E_ROOT/plain"
mkdir -p "$PLAIN_DIR"
OUT="$SHELL_E2E_ROOT/out"
mkdir -p "$OUT"

# One Fish process: DIR STATUS DURATION SCRIPTS(0|1). Writes shell.prompt,
# exec.prompt, shell.transient, exec.transient and exec.rc into $OUT.
render_both() {
    # shellcheck disable=SC2016  # the Fish program expands its own variables
    fish -c '
        set -l root $argv[1]; set -l dir $argv[2]; set -l st $argv[3]; set -l dur $argv[4]
        set -l scripts $argv[5]; set -l out $argv[6]
        source $root/fish/core/init.fish >/dev/null 2>&1
        for f in $root/fish/functions/*.fish; source $f; end
        set -g fish_greeting ""
        set -g __gpy_cfg_transient_prompt 1
        cd $dir
        set -e __gpy_duration_idle
        set -g CMD_DURATION $dur
        functions -c __gpy_render_prompt_exec __gpy_test_exec
        function __gpy_render_prompt_exec; return 1; end
        # The shell path: its script run (the fish_prompt event), then render.
        set -g __gpy_exec_declined 1
        if test $scripts = 1
            set -g __gpy_g_due 1
            emit fish_prompt
        end
        function __gpy_test_status --argument-names s; return $s; end
        fish_prompt >/dev/null 2>&1
        __gpy_test_status $st; fish_prompt >$out/shell.prompt
        __gpy_test_status $st; fish_prompt --final-rendering >$out/shell.transient
        functions -e __gpy_render_prompt_exec
        functions -c __gpy_test_exec __gpy_render_prompt_exec
        set -e __gpy_exec_declined
        set -g __gpy_g_reloaded 1
        __gpy_test_status $st; __gpy_render_prompt_exec $st >$out/exec.prompt
        echo $status >$out/exec.rc
        __gpy_test_status $st; __gpy_render_prompt_exec $st transient >$out/exec.transient
    ' "$ROOT" "$1" "$2" "$3" "$4" "$OUT"
}

cells() {
    python3 "$ROOT/tests/lib/sgr_cells.py" "$1"
}

# LABEL DIR STATUS DURATION [SCRIPTS]. The clock can tick between the two
# renders: one retry.
compare() {
    local label=$1 _
    for _ in 1 2; do
        render_both "$2" "$3" "$4" "${5:-0}"
        if [[ "$(cells "$OUT/shell.prompt")" == "$(cells "$OUT/exec.prompt")" &&
            "$(cells "$OUT/shell.transient")" == "$(cells "$OUT/exec.transient")" ]]; then
            break
        fi
    done
    check "$label: the exec path renders" 0 "$(cat "$OUT/exec.rc")"
    check "$label: prompt cells" "$(cells "$OUT/shell.prompt")" "$(cells "$OUT/exec.prompt")"
    check "$label: transient cells" "$(cells "$OUT/shell.transient")" "$(cells "$OUT/exec.transient")"
    grep -q "${2##*/}" "$OUT/exec.prompt" || fail "$label: no directory in the prompt"
}

for theme in default text starship pills; do
    write_config "$theme" "$SEGMENTS"
    "$SHELL_E2E_AGENT_BIN" config reload >/dev/null 2>&1
    for dir in "$SHELL_E2E_REPO" "$PLAIN_DIR"; do
        where="repo"
        [[ "$dir" == "$PLAIN_DIR" ]] && where="plain dir"
        # Warm the agent's caches for this theme.
        render_both "$dir" 0 0 0
        sleep 1
        compare "$theme, $where, success" "$dir" 0 0
        compare "$theme, $where, failure" "$dir" 1 0
        compare "$theme, $where, slow command" "$dir" 0 2500
    done
done

# Script segments: one plugin, styled by a theme overlay.
PLUG="$XDG_CONFIG_HOME/gpy/plugins/ptest"
mkdir -p "$PLUG/bin" "$PLUG/segments"
printf 'id = "ptest"\nname = "ptest"\nversion = "0.1.0"\napi_version = "v1"\nentry_type = "script"\nprovided_segments = ["pseg", "pvar"]\n' \
    >"$PLUG/plugin.toml"
printf 'command = "bin/pseg.sh"\ninputs = ["pwd"]\nttl_secs = 0\ntimeout_ms = 500\n' >"$PLUG/segments/pseg.toml"
printf 'command = "bin/pvar.sh"\nttl_secs = 3600\ntimeout_ms = 500\n' >"$PLUG/segments/pvar.toml"
cat >"$PLUG/bin/pseg.sh" <<'EOF'
#!/bin/sh
printf 'in %s $HOME `x` 50%% \\\n' "${GPY_PWD##*/}"
EOF
printf '#!/bin/sh\nprintf '\''v1\\nwarn\\n'\''\n' >"$PLUG/bin/pvar.sh"
chmod -R go-w "$PLUG"
cat >"$XDG_CONFIG_HOME/gpy/themes/scripts.toml" <<'EOF'
extends = "default"
[segments.pseg]
icon = "p$"
bg_color = "blue"
text_color = "white"
[segments.pseg.close]
icon = ">"
icon_color = "match_bg"
[segments.pvar]
bg_color = "green"
warn_bg_color = "yellow"
warn_icon = "w`"
EOF
for theme in scripts starship; do
    write_config "$theme" '"pseg", "directory", "pvar", "git"'
    "$SHELL_E2E_AGENT_BIN" config reload >/dev/null 2>&1
    compare "$theme, script segments" "$SHELL_E2E_REPO" 0 0 1
    grep -q 'in repo $HOME `x` 50% \\' "$OUT/exec.prompt" || fail "$theme: script text missing"
    grep -q 'v1' "$OUT/exec.prompt" || fail "$theme: variant segment missing"
done

test_finish "gpy prompt fish matches fish_prompt"

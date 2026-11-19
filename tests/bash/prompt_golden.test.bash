#!/usr/bin/env bash
# tests/bash/prompt_golden.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# `gpy prompt <shell>` against golden prompts (#1247). The goldens were
# captured from the binary while the parity tests proved it printed exactly
# what each shell's own renderer printed, just before those renderers were
# deleted; they keep that contract. Covers Bash, Zsh and Fish, every builtin
# theme plus an overlay without clock/status templates, success, failure, a
# slow command, a repository and a plain directory, the Zsh/Fish transient
# part, script segments (escapes, icon, variant, own cap), and the default
# theme with `ui.show_icons = false`.
#
# Normalised: the sandbox root becomes <ROOT> and Fish's clock time <TIME>
# (Bash and Zsh carry a live `\D{}`/`%D{}` token). The language segment is
# left out: its version text depends on the tools installed.
#
# GPY_UPDATE_GOLDEN=1 rewrites the goldens instead of comparing.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1
shell_e2e_init "$ROOT"
test_require_command gpy "the debug build provides gpy"

GOLDEN="$ROOT/tests/fixtures/prompt_golden"
SEGMENTS='"clock", "duration", "status", "directory", "git"'
mkdir -p "$XDG_CONFIG_HOME/gpy/themes" "$HOME/plain"
printf 'extends = "default"\nunset = ["segments.clock.format", "segments.status.format"]\n' \
    >"$XDG_CONFIG_HOME/gpy/themes/pills.toml"

# THEME SEGMENTS [EXTRA_UI_LINE]
write_config() {
    printf '[ui]\ntheme = "%s"\nenabled_segments = [%s]\n%s\n[agent.supervisor]\nenabled = false\n' \
        "$1" "$2" "${3-}" >"$XDG_CONFIG_HOME/gpy/config.toml"
}

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
bg_color = "black"
[segments.pvar]
bg_color = "green"
warn_bg_color = "yellow"
warn_icon = "w`"
EOF

write_config default "$SEGMENTS"
shell_e2e_start_agent || exit 1
export GPY_GIT_INSTANT_CACHE_TTL_SECONDS=3600 TERM=xterm-256color
unset TERM_PROGRAM GPY_COLOR_DEPTH GPY_SHOW_STATUS

normalise() {
    sed -e "s|$SHELL_E2E_ROOT|<ROOT>|g" -e "s|$HOME|<HOME>|g" |
        sed -E 's/[0-9]{1,2}:[0-9]{2}(:[0-9]{2})?( [AP]M)?/<TIME>/g'
}

seq_no=100
# SHELL STATUS DURATION: one `gpy prompt`, normalised, to stdout.
render() {
    local -a args=(prompt "$1" --status "$2" --pid "$$" --cmd "$seq_no")
    [[ -n $3 ]] && args+=(--duration-ms "$3")
    seq_no=$((seq_no + 1))
    gpy "${args[@]}" 2>/dev/null | normalise
}

# NAME SHELL STATUS DURATION
golden() {
    local file="$GOLDEN/$2/$1.ansi" got
    got="$(render "$2" "$3" "$4"; printf x)"
    got=${got%x}
    if [[ ${GPY_UPDATE_GOLDEN-} == 1 ]]; then
        mkdir -p "${file%/*}"
        printf '%s' "$got" >"$file"
        pass "updated $2/$1"
    elif [[ ! -f $file ]]; then
        fail "$2/$1: no golden (run with GPY_UPDATE_GOLDEN=1)"
    else
        check "$2/$1" "$(cat "$file"; printf x)" "${got}x"
    fi
}

warm() {
    local shell
    for shell in bash zsh fish; do render "$shell" 0 "" >/dev/null; done
    marker() {
        ls "$XDG_CACHE_HOME"/gpy/instant-prompts/*.git.any-entry.* >/dev/null 2>&1
    }
    shell_e2e_poll 5 marker
    sleep 1
}

for theme in default text starship pills scripts; do
    segments=$SEGMENTS
    [[ $theme == scripts ]] && segments='"pseg", "directory", "pvar", "git"'
    write_config "$theme" "$segments"
    "$SHELL_E2E_AGENT_BIN" config reload >/dev/null 2>&1
    for where in repo plain; do
        if [[ $where == repo ]]; then cd "$SHELL_E2E_REPO" || exit 1; else cd "$HOME/plain" || exit 1; fi
        warm
        for shell in bash zsh fish; do
            golden "$theme-$where-success" "$shell" 0 ""
            golden "$theme-$where-failure" "$shell" 1 ""
            golden "$theme-$where-slow" "$shell" 0 2500
        done
    done
done

write_config default "$SEGMENTS" 'show_icons = false'
"$SHELL_E2E_AGENT_BIN" config reload >/dev/null 2>&1
for where in repo plain; do
    if [[ $where == repo ]]; then cd "$SHELL_E2E_REPO" || exit 1; else cd "$HOME/plain" || exit 1; fi
    warm
    for shell in bash zsh fish; do
        golden "default-noicons-$where-success" "$shell" 0 ""
        golden "default-noicons-$where-failure" "$shell" 1 ""
    done
done

test_finish "gpy prompt matches the golden prompts"

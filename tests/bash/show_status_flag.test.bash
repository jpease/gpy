#!/usr/bin/env bash
# tests/bash/show_status_flag.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# GPY_SHOW_STATUS follows the shared boolean table in all three shells: `true`
# shows the exit-status indicator and `off` hides it, in the shell-drawn tail
# (agent down). `true` prints the one-time old-meaning warning (it used to
# hide the indicator); `off` prints none.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

TMP="$(mktemp -d "${TMPDIR:-/tmp}/gpy-show-status.XXXXXX")"
trap 'rm -rf "$TMP"' EXIT
mkdir -p "$TMP/home" "$TMP/cache" "$TMP/config" "$TMP/run"

# PATH without any gpy-agent: the agent is down and no oneshot can run.
CLEAN_PATH=""
IFS=: read -r -a path_dirs <<<"$PATH"
for dir in "${path_dirs[@]}"; do
    [ -x "$dir/gpy-agent" ] && continue
    CLEAN_PATH="${CLEAN_PATH:+$CLEAN_PATH:}$dir"
done

# render SHELL VALUE ERRFILE: the prompt SHELL draws after a failed command with
# GPY_SHOW_STATUS=VALUE. Only the status tail is enabled, with a marker icon.
render() {
    local sh="$1" value="$2" err="$3" shell_bin
    # An absolute path: CLEAN_PATH may drop the directory holding the shell itself
    # (Homebrew keeps fish, zsh and gpy-agent together).
    shell_bin="$(command -v "$sh")"
    local -a envs=(env -i PATH="$CLEAN_PATH" HOME="$TMP/home" TERM=dumb
        XDG_CONFIG_HOME="$TMP/config" XDG_CACHE_HOME="$TMP/cache" XDG_RUNTIME_DIR="$TMP/run"
        TMPDIR="$TMP" GPY_AGENT_SOCKET_PATH="$TMP/run/missing.sock"
        GPY_AGENT_ENABLED=1 GPY_AGENT_SUPERVISOR_ENABLED=0 GPY_SHOW_STATUS="$value" ROOT="$ROOT")
    case "$sh" in
        bash)
            # shellcheck disable=SC2016 # expands in the child
            "${envs[@]}" "$shell_bin" --noprofile --norc -c '
                source "$ROOT/bash/gpy.bash"
                __icon_status_fail=STATUSFAIL __enabled_segments="" __gpy_two_line=0 __gpy_add_newline=0
                unset __gpy_char_cache_key __gpy_char_cache_val
                __gpy_render_prompt 1
                printf "%s" "$PS1"' 2>"$err"
            ;;
        zsh)
            # shellcheck disable=SC2016 # expands in the child
            "${envs[@]}" "$shell_bin" -f -c '
                source "$ROOT/zsh/gpy.zsh"
                __icon_status_fail=STATUSFAIL __enabled_segments=() __gpy_two_line=0 __gpy_add_newline=0
                __gpy_char_cache_key="" __gpy_char_cache_val=""
                __gpy_render_prompt 1
                print -rn -- "$__gpy_render_out"' 2>"$err"
            ;;
        fish)
            # shellcheck disable=SC2016 # expands in the child
            "${envs[@]}" "$shell_bin" --no-config -c '
                source $ROOT/fish/core/init.fish
                source $ROOT/fish/functions/fish_prompt.fish
                set -g __icon_status_fail STATUSFAIL
                set -g __enabled_segments
                set -g __gpy_two_line 0
                set -g __gpy_add_newline 0
                emit fish_prompt
                false
                fish_prompt | string collect
                emit fish_prompt' 2>"$err"
            ;;
    esac
}

warnings() { grep -c 'GPY_SHOW_STATUS=' "$1"; }

for sh in bash zsh fish; do
    if ! command -v "$sh" >/dev/null 2>&1; then
        test_skip_part "$sh is not installed"
        continue
    fi
    err="$TMP/err.$sh"

    out="$(render "$sh" true "$err")"
    case "$out" in
        *STATUSFAIL*) pass "$sh: GPY_SHOW_STATUS=true shows the indicator" ;;
        *) fail "$sh: GPY_SHOW_STATUS=true did not show the indicator: $out" ;;
    esac
    check "$sh: GPY_SHOW_STATUS=true prints the old-meaning warning once" "1" "$(warnings "$err")"

    out="$(render "$sh" off "$err")"
    case "$out" in
        *STATUSFAIL*) fail "$sh: GPY_SHOW_STATUS=off still showed the indicator: $out" ;;
        *) pass "$sh: GPY_SHOW_STATUS=off hides the indicator" ;;
    esac
    check "$sh: GPY_SHOW_STATUS=off prints no warning" "0" "$(warnings "$err")"

    out="$(render "$sh" "" "$err")"
    case "$out" in
        *STATUSFAIL*) pass "$sh: an unset-like GPY_SHOW_STATUS defaults to shown" ;;
        *) fail "$sh: an empty GPY_SHOW_STATUS hid the indicator: $out" ;;
    esac
done

test_finish "GPY_SHOW_STATUS follows the shared boolean table in all three shells"

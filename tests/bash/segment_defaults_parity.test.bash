#!/usr/bin/env bash
# tests/bash/segment_defaults_parity.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Fallbacks the three shells use when the theme export has not run agree:
#   - the duration threshold is 100 ms (the exported theme default) everywhere;
#   - `__gpy_home` accepts the same usernames when HOME is empty;
#   - with no theme export, `__enabled_segments` is the list in
#     tests/fixtures/default_segments.txt (the Rust test
#     default_segments_fixture_matches pins that file to the agent's default),
#     and an export that sets it empty stays empty.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

command -v fish >/dev/null 2>&1 || test_skip "fish not installed"
command -v zsh >/dev/null 2>&1 || test_skip "zsh not installed"

echo "--- duration threshold fallback ---"
bash_out="$(
    # shellcheck source=bash/segments/duration.bash
    . "$ROOT/bash/segments/duration.bash"
    unset __duration_threshold_ms
    out=""
    for pair in below:99 at:100 above:5000; do
        __gpy_cmd_duration="${pair#*:}"
        __gpy_segment_duration_detect
        out="$out ${pair%%:*}=$?"
    done
    echo "${out# }"
)"
want="below=1 at=0 above=0"
check "bash: unset threshold hides 99 ms, shows 100 ms and 5000 ms" "$want" "$bash_out"

zsh_out="$(
    zsh -f -c '
        . "$1/zsh/segments/duration.zsh"
        unset __duration_threshold_ms
        out=""
        for pair in below:99 at:100 above:5000; do
            __gpy_cmd_duration=${pair#*:}
            __gpy_segment_duration_detect
            out="$out ${pair%%:*}=$?"
        done
        print -r -- ${out# }
    ' zsh "$ROOT"
)"
check "zsh: unset threshold hides 99 ms, shows 100 ms and 5000 ms" "$want" "$zsh_out"

zsh_init="$(grep -E '^typeset -g __duration_threshold_ms=' "$ROOT/zsh/core/init.zsh" | sed -E 's/^[^=]*=([0-9]+).*/\1/')"
check "zsh init seeds the same 100 ms threshold" "100" "$zsh_init"

fish_out="$(
    # shellcheck disable=SC2016 # the fish program is single-quoted on purpose.
    fish --no-config -c '
        source $argv[1]/fish/segments/duration.fish
        set -e __duration_threshold_ms
        set -l out
        for pair in below:99 at:100 above:5000
            set -g CMD_DURATION (string split : $pair)[2]
            segment_duration_detect
            set -l rc $status
            set -a out (string split : $pair)[1]=$rc
        end
        echo $out
    ' "$ROOT"
)"
check "fish: unset threshold hides 99 ms, shows 100 ms and 5000 ms" "$want" "$fish_out"

echo "--- __gpy_home username fallback ---"
# HOME is empty and `id -un` prints NAME. Every shell must refuse anything that
# is not a plausible username (a leading - would expand `~-` to $OLDPWD) and
# resolve the real user's home.
real_user="$(id -un)"
# Stub `id` for each shell by name: functions beat the external command.
probe_bash() {
    FAKE_ID="$1" bash -c '
        id() { printf "%s\n" "$FAKE_ID"; }
        HOME="$2"; [ "$2" = "-" ] && unset HOME
        . "$1/bash/core/ipc.bash"
        out=""
        __gpy_home out && [ -n "$out" ] && echo ok || echo rejected
    ' bash "$ROOT" "$2" 2>/dev/null
}
probe_zsh() {
    FAKE_ID="$1" zsh -f -c '
        id() { print -r -- "$FAKE_ID"; }
        unset USERNAME
        HOME="$2"; [[ $2 == - ]] && unset HOME
        source "$1/zsh/core/ipc.zsh"
        out=""
        __gpy_home out && [[ -n $out ]] && print ok || print rejected
    ' zsh "$ROOT" "$2" 2>/dev/null
}
probe_fish() {
    # shellcheck disable=SC2016 # the fish program is single-quoted on purpose.
    FAKE_ID="$1" fish --no-config -c '
        function id
            echo $FAKE_ID
        end
        test "$argv[2]" = -; and set -e HOME; or set -gx HOME "$argv[2]"
        source $argv[1]/fish/core/ipc.fish
        set -l out (__gpy_home)
        and test -n "$out"
        and echo ok
        or echo rejected
    ' "$ROOT" "$2" 2>/dev/null
}

for row in "-" "+" ".x" "a b" "x;touch PWNED" "$real_user"; do
    want=rejected
    [ "$row" = "$real_user" ] && want=ok
    for home_state in "" "-"; do
        label="HOME=${home_state:-<empty>}"
        [ "$home_state" = "-" ] && label="HOME=<unset>"
        for shell in bash zsh fish; do
            got="$("probe_$shell" "$row" "$home_state")"
            check "$shell: id='$row' $label -> $want" "$want" "$got"
        done
    done
done
home_set="$(probe_bash "-" "/somewhere")"
check "bash: an absolute HOME needs no username" "ok" "$home_set"
home_set="$(probe_zsh "-" "/somewhere")"
check "zsh: an absolute HOME needs no username" "ok" "$home_set"
home_set="$(probe_fish "-" "/somewhere")"
check "fish: an absolute HOME needs no username" "ok" "$home_set"

echo "--- fallback segment list ---"
# No theme export has loaded: empty cache dir, sandbox HOME, and a PATH without
# gpy-agent (the shells run by absolute path). shell_e2e_init is not used, since
# it puts the built binary on PATH.
seg_tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-seg-fallback.XXXXXX")"
trap 'rm -rf "$seg_tmp"' EXIT
mkdir -p "$seg_tmp/home" "$seg_tmp/cache/gpy" "$seg_tmp/cfg" "$seg_tmp/run"
fish_bin="$(command -v fish)"
zsh_bin="$(command -v zsh)"
bash_bin="$(command -v bash)"
want_segments="$(grep -v -e '^#' -e '^[[:space:]]*$' "$ROOT/tests/fixtures/default_segments.txt" | tr ' ' '\n')"

run_sandboxed() {
    env -i PATH="/usr/bin:/bin" HOME="$seg_tmp/home" TERM=dumb \
        XDG_CONFIG_HOME="$seg_tmp/cfg" XDG_CACHE_HOME="$seg_tmp/cache" \
        XDG_RUNTIME_DIR="$seg_tmp/run" TMPDIR="$seg_tmp" \
        GPY_AGENT_SUPERVISOR_ENABLED=0 ROOT="$ROOT" "$@" 2>/dev/null
}
fish_segments() {
    # shellcheck disable=SC2016 # the fish program is single-quoted on purpose.
    run_sandboxed "$fish_bin" --no-config -c '
        source $ROOT/fish/core/init.fish
        printf "%s\n" $__enabled_segments
        functions -q segment_clock_detect segment_directory_detect; and echo FUNCS_LOADED
    '
}
bash_segments() {
    # shellcheck disable=SC2016 # $ROOT expands in the child.
    run_sandboxed "$bash_bin" --noprofile --norc -c '
        source "$ROOT/bash/gpy.bash"
        # shellcheck disable=SC2086
        printf "%s\n" $__enabled_segments
    '
}
zsh_segments() {
    # shellcheck disable=SC2016 # $ROOT expands in the child.
    run_sandboxed "$zsh_bin" -f -c '
        source "$ROOT/zsh/gpy.zsh"
        print -l -- $__enabled_segments
        __gpy_render_prompt 0 >/dev/null
        print -r -- "after-render=${#__enabled_segments}"
    ' | grep -v '^$'
}

check "fish: no export uses the fixture's segment list, with its segment functions loaded" "$want_segments
FUNCS_LOADED" "$(fish_segments)"
check "bash: no export uses the fixture's segment list" "$want_segments" "$(bash_segments)"
check "zsh: no export uses the fixture's segment list" "$want_segments
after-render=5" "$(zsh_segments)"

# An export that sets the list empty stays empty (the sentinel line is the
# export's first line).
printf '%s\n' 'set -g __gpy_export_version 1' 'set -g __enabled_segments' >"$seg_tmp/cache/gpy/theme-export.fish"
printf '%s\n' '__gpy_export_version="1"' '__enabled_segments=""' >"$seg_tmp/cache/gpy/theme-export.bash"
printf '%s\n' 'typeset -g __gpy_export_version="1"' 'typeset -g -a __enabled_segments=()' >"$seg_tmp/cache/gpy/theme-export.zsh"
check "fish: an export with an empty list stays empty" "" "$(fish_segments)"
check "bash: an export with an empty list stays empty" "" "$(bash_segments)"
check "zsh: an export with an empty list stays empty, also after a render" "after-render=0" "$(zsh_segments)"

test_finish "duration threshold, __gpy_home and fallback segment list agree across shells"

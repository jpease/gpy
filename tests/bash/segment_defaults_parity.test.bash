#!/usr/bin/env bash
# tests/bash/segment_defaults_parity.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Fallbacks the three shells use when the theme export has not run agree:
#   - the duration threshold is 100 ms (the exported theme default) everywhere;
#   - `__gpy_home` accepts the same usernames when HOME is empty.

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

test_finish "duration threshold and __gpy_home fallbacks agree across shells"

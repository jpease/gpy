#!/usr/bin/env bash
# tests/bash/script_segment_adapter.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# shellcheck disable=SC2015  # pass never fails, so A && pass || fail is if/else
# shellcheck disable=SC2009  # matching the full command line of a leaked sleep; pgrep is not portable here
# shellcheck disable=SC1003  # the quotes are test data inside a $'...' string
# shellcheck disable=SC2089,SC2090  # TEST_DECLARED holds hostile quotes as literal test data
#
# Script (G) segments in Bash (bash/core/g_segments.bash, g_runner.bash),
# driven in this process through the real hooks, on whichever Bash runs the
# test (3.2 and 5 in CI). The segment variables are set the way a theme
# export sets them, then __gpy_g_sync runs as after a theme load.
#   (a) newline and ESC in $PWD reach the script literally, never the prompt
#   (b) stale state and PID-named files from an earlier shell change nothing
#   (c) C1 output hides the segment under LC_ALL=C and UTF-8; C0 is stripped
#   (d) `a[$(cmd)]` names never run cmd; glob characters match literally
#   (e) no file is created by any run, scan or cache use
#   (f) once per command: redraws reuse, the cache honours ttl and key
#   (g) a timeout with a grandchild holding stdout is bounded, tree killed
#   (h) require_command runs under the same deadline and gates the script
#   plus the clean environment, the circuit breaker and the pill.

# shellcheck disable=SC2016,SC2034,SC2154
# Single quotes are load-bearing (literal `$(...)` names); the __gpy_script_*
# variables are read by the integration, not here.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

SANDBOX="$(mktemp -d "${TMPDIR:-/tmp}/gpy-g-adapter.XXXXXX")"
SANDBOX="$(cd "$SANDBOX" && pwd -P)"
# Marks the sleeps of hostile scripts, so a survivor check sees only ours.
MARK="7.$$"
cleanup() {
    pkill -KILL -f "sleep $MARK" 2>/dev/null
    rm -rf "$SANDBOX"
}
trap cleanup EXIT

source "$ROOT/tests/lib/supervisor_off.bash" "$SANDBOX"
export GPY_AGENT_SOCKET_PATH="$SANDBOX/missing.sock"
export HOME="$SANDBOX/home" TMPDIR="$SANDBOX/tmp" XDG_RUNTIME_DIR="$SANDBOX/run"
mkdir -p "$HOME" "$TMPDIR" "$XDG_RUNTIME_DIR" "$SANDBOX/marks" "$SANDBOX/work"

echo "=== (b) state from the parent environment is never read ==="
PWNED="$SANDBOX/pwned"
export __gpy_g_ck_seg="k" __gpy_g_ct_seg="99999999999" __gpy_g_cv_seg="STALE"
export __gpy_g_rs_seg="a[\$(touch $PWNED)]" __gpy_g_off_seg=1 __gpy_g_first=""
export __gpy_script_seg_command="/bin/echo" __gpy_script_segments="seg"
# What a cache keyed by PID would have left for a recycled PID.
printf 'STALE\n' >"$TMPDIR/gpy-g-$$"
printf 'STALE\n' >"$HOME/.gpy-g-$$"

source bash/gpy.bash

if [[ -z ${__gpy_g_cv_seg+x} && -z ${__gpy_g_off_seg+x} && -z ${__gpy_script_seg_command+x} ]]; then
    pass "the integration unset the seeded __gpy_g_* and __gpy_script_* variables"
else
    fail "seeded state survived the load"
fi
if ((${#__gpy_g_toks[@]} == 0)); then
    pass "a seeded __gpy_script_segments without an export lists nothing"
else
    fail "seeded list was used: ${__gpy_g_toks[*]}"
fi

PLUG="$SANDBOX/plug"
mkdir -p "$PLUG/bin"
chmod 755 "$PLUG" "$PLUG/bin"
mkscript() {
    printf '#!/bin/sh\n%s\n' "$2" >"$PLUG/bin/$1.sh"
}

# Defaults of one exported segment `seg` (token seg), script $1.
reset_seg() {
    __gpy_script_sh=/bin/sh
    [[ -x /private/var/select/sh ]] && __gpy_script_sh=/bin/bash
    __gpy_script_seg_name=seg
    __gpy_script_seg_root="$PLUG"
    __gpy_script_seg_command="$PLUG/bin/$1.sh"
    __gpy_script_seg_reqcommand=""
    __gpy_script_seg_inputs=()
    __gpy_script_seg_env=()
    __gpy_script_seg_opts=()
    __gpy_script_seg_ttl=0
    __gpy_script_seg_timeout=300
    __gpy_script_seg_scope=session
    __gpy_script_seg_reqfiles=()
    __gpy_script_seg_reqfolders=()
    __gpy_script_seg_reqexts=()
    __gpy_script_seg_reqenv=()
    __gpy_script_seg_key="3:seg"
}
activate() {
    __gpy_script_segments=(seg)
    __enabled_segments="seg"
    __gpy_g_sync
}
# A due prompt (as after a command) and a redraw-only prompt.
due() {
    __gpy_g_first=1
    __gpy_g_maybe_run
}
out() {
    printf '%s' "$__gpy_g_out_seg"
}
runs() {
    if [[ -f "$SANDBOX/marks/$1" ]]; then
        wc -l <"$SANDBOX/marks/$1" | tr -d ' '
    else
        echo 0
    fi
}
now_ms() {
    if [[ -n ${EPOCHREALTIME-} ]]; then
        local t=${EPOCHREALTIME/[.,]/}
        echo $((10#$t / 1000))
    else
        python3 -c 'import time; print(int(time.time() * 1000))'
    fi
}
survivors() {
    ps -axo command= | grep -c "[s]leep $MARK"
}
no_survivors() {
    [[ "$(survivors)" == 0 ]]
}

touch "$SANDBOX/stamp"
sleep 1 # mtime granularity: files from here on are newer than the stamp

mkscript fresh 'echo fresh'
reset_seg fresh
activate
due
check "first due prompt runs the script, not the stale value" "fresh" "$(out)"
[[ ! -e "$PWNED" ]] && pass "seeded a[\$(touch ...)] state never ran" || fail "seeded state ran a command"

echo "=== pill ==="
mkscript pill 'printf "ctx %s\n" "$GPY_OPT_label"; echo prod'
reset_seg pill
__gpy_script_seg_opts=('GPY_OPT_label=a$b`c\')
activate
__color_seg_bg=blue __color_seg_prod_bg=red __icon_seg='K'
due
__gpy_render_prompt 0
if [[ "$PS1" == *' K ctx a\\$b\\`c\\\\'* ]]; then
    pass "the pill shows icon and text escaped for promptvars"
else
    fail "pill text missing or unescaped in PS1: $(printf '%q' "$PS1")"
fi
[[ "$PS1" == *$'\033[39;41m'* ]] && pass "variant prod draws its own background" ||
    fail "variant background not applied"
if ((BASH_VERSINFO[0] > 4 || (BASH_VERSINFO[0] == 4 && BASH_VERSINFO[1] >= 4))); then
    # Prompt decoding plus promptvars, as Bash draws PS1.
    PS1_EXPANDED=${PS1@P}
    [[ "$PS1_EXPANDED" == *'ctx a$b`c\'* ]] && pass "the drawn prompt prints the text literally" ||
        fail "prompt expansion changed the text: $PS1_EXPANDED"
fi
saved_close_bg=${__prompt_close_bg-} saved_delim_last=${__segment_delim_last-}
__prompt_close_bg=magenta __segment_delim_last='E'
__gpy_render_prompt 0
[[ "$PS1" == *$'49m\\]E'* && "$PS1" != *$'45m\\]E'* ]] &&
    pass "a last plugin pill draws its close cap on the default background" ||
    fail "last pill close cap bg: $(printf '%q' "$PS1")"
[[ "$(__gpy_section_standalone blue white x true)" == *$'45m\\]E'* ]] &&
    pass "a built-in last segment keeps the prompt_close background" ||
    fail "built-in close cap bg changed"
__prompt_close_bg=$saved_close_bg __segment_delim_last=$saved_delim_last
__gpy_segment_seg_open_icon='<<' __gpy_segment_seg_open_icon_color=green __gpy_segment_seg_open_bg_color=normal
__gpy_segment_seg_close_icon='>>' __gpy_segment_seg_close_icon_color=yellow __gpy_segment_seg_close_bg_color=normal
__gpy_render_prompt 0
[[ "$PS1" == *$'\033[32;49m\\]<<'* && "$PS1" == *$'\033[33;49m\\]>>'* ]] &&
    pass "the segment's own open and close caps replace the global delimiters" ||
    fail "segment caps not drawn: $(printf '%q' "$PS1")"
__gpy_g_clear_styles
[[ -z ${__color_seg_bg+x}${__color_seg_prod_bg+x}${__icon_seg+x}${__gpy_segment_seg_open_icon+x} ]] &&
    [[ -z ${__gpy_segment_seg_close_bg_color+x} ]] &&
    pass "a theme reload clears the plugin segment's old style variables" ||
    fail "stale style variables survived __gpy_g_clear_styles"

echo "=== (a) newline and ESC in \$PWD ==="
mkscript pwd 'printf "%s" "$GPY_PWD" >"'"$SANDBOX"'/marks/gotpwd"; printf "%s\n" "$GPY_PWD"; pwd -L >"'"$SANDBOX"'/marks/cwd"'
reset_seg pwd
__gpy_script_seg_inputs=(pwd)
__gpy_script_seg_scope=folder
__gpy_script_seg_ttl=60
activate
dir1="$SANDBOX/work/nl"$'\n'"esc"$'\033'"[31mX"
dir2="$SANDBOX/work/nl"$'\n'"esc"$'\033'"[31mY"
mkdir -p "$dir1" "$dir2"
cd "$dir1" || exit 1
due
check "GPY_PWD arrives byte for byte" "$dir1" "$(cat "$SANDBOX/marks/gotpwd")"
expected="${dir1%%$'\n'*}"
check "the text is line 1 of the output" "$expected" "$(out)"
[[ "$__gpy_g_out_seg" != *$'\033'* && "$__gpy_g_out_seg" != *$'\n'* ]] &&
    pass "no ESC or newline reaches the stored text" || fail "control character stored"
check "folder scope runs the script in \$PWD" "$dir1" "$(cat "$SANDBOX/marks/cwd")"
cd "$dir2" || exit 1
due
check "a sibling folder sharing line 1 is a cache miss" "$dir2" "$(cat "$SANDBOX/marks/gotpwd")"
cd "$ROOT" || exit 1

echo "=== (c) C1 hides the segment in every locale; C0 is stripped ==="
c1_case() { # label printf-format expect
    printf '#!/bin/sh\nprintf '\''%s'\''\n' "$2" >"$PLUG/bin/c1.sh"
    reset_seg c1
    activate
    local loc
    for loc in C en_US.UTF-8; do
        LC_ALL=$loc due
        if [[ "$3" == hidden ]]; then
            if __gpy_g_visible seg; then fail "$1 under LC_ALL=$loc is shown"; else pass "$1 under LC_ALL=$loc is hidden"; fi
        else
            check "$1 under LC_ALL=$loc" "$3" "$(out)"
        fi
    done
}
c1_case "CSI U+009B" 'a\302\233b\n' hidden
c1_case "NEL with ESC, BEL, DEL" 'a\302\205\033\007\177\tb\n' hidden
c1_case "c2 c2 9b 9b" '\302\302\233\233\n' hidden
c1_case "c2 1b 9b" '\302\033\233\n' hidden
c1_case "e2 c2 9b 82" '\342\302\233\202\n' hidden
c1_case "ESC, BEL, DEL and TAB" 'a\033[31m\007\177\tb\n' "a[31mb"
c1_case "UTF-8 text" '\303\251\342\206\222\n' $'\303\251\342\206\222'
c1_case "a lone 0x9b byte" 'a\233b\n' $'a\233b'

echo "=== (d) hostile manifest names and glob characters ==="
mkscript plain 'echo plain'
reset_seg plain
__gpy_script_seg_env=("a[\$(touch $PWNED)]")
activate
due
! __gpy_g_visible seg && pass "an env name a[\$(...)] hides the segment" || fail "bad env name ran"
reset_seg plain
__gpy_script_seg_reqenv=("a[\$(touch $PWNED)]")
activate
due
! __gpy_g_visible seg && pass "a require_env name a[\$(...)] hides the segment" || fail "bad require_env name ran"
reset_seg plain
__gpy_script_seg_opts=("GPY_OPT_x[\$(touch $PWNED)]=1")
activate
due
! __gpy_g_visible seg && pass "a setting name a[\$(...)] hides the segment" || fail "bad setting name ran"
reset_seg plain
__gpy_script_seg_inputs=("a[\$(touch $PWNED)]")
activate
due
! __gpy_g_visible seg && pass "an unknown input hides the segment" || fail "unknown input ran"
reset_seg plain
__gpy_script_segments=("a[\$(touch $PWNED)]" seg)
__enabled_segments="seg"
__gpy_g_sync
check "a listed token a[\$(...)] is skipped" "seg" "${__gpy_g_toks[*]}"
[[ ! -e "$PWNED" ]] && pass "no hostile name ran its command" || fail "a hostile name ran its command"

glob="$SANDBOX/work/glob"
mkdir -p "$glob/sub/a"
touch "$glob/other" "$glob/x.tgz"
cd "$glob/sub" || exit 1
reset_seg plain
__gpy_script_seg_reqfiles=('*')
activate
due
! __gpy_g_visible seg && pass "require_files '*' does not match other files" || fail "'*' matched as a glob"
touch "$glob/*"
due
check "require_files '*' matches a file named '*' in an ancestor" "plain" "$(out)"
reset_seg plain
__gpy_script_seg_reqfolders=('[ab]')
activate
due
! __gpy_g_visible seg && pass "require_folders '[ab]' does not match folder a" || fail "'[ab]' matched as a glob"
reset_seg plain
__gpy_script_seg_reqexts=('tgz')
activate
due
! __gpy_g_visible seg && pass "require_extensions looks in the working directory only" || fail "extension found in a parent"
cd "$glob" || exit 1
due
check "require_extensions matches x.tgz in the working directory" "plain" "$(out)"
cd "$ROOT" || exit 1

echo "=== clean environment, values never in argv ==="
mkscript env 'env >"'"$SANDBOX"'/marks/env"; ps -o command= -p $$ >"'"$SANDBOX"'/marks/argv"; echo env'
reset_seg env
__gpy_script_seg_env=(TEST_DECLARED TEST_UNSET)
__gpy_script_seg_opts=('GPY_OPT_mode=fast')
activate
export TEST_SECRET=hidden-secret GPY_OPT_mode=SPOOFED
TEST_DECLARED=$'line1\nline2 $(touch '"$PWNED"') `id` * '"'q'"
export TEST_DECLARED
unset TEST_UNSET
due
check "the script ran" "env" "$(out)"
envdump="$(cat "$SANDBOX/marks/env")"
[[ "$envdump" != *hidden-secret* ]] && pass "an undeclared variable is not passed" || fail "TEST_SECRET leaked"
[[ "$envdump" == *"GPY_OPT_mode=fast"* && "$envdump" != *SPOOFED* ]] &&
    pass "settings override an inherited GPY_OPT_*" || fail "inherited GPY_OPT_mode reached the script"
[[ "$envdump" == *"TEST_DECLARED=$TEST_DECLARED"* ]] && pass "a hostile declared value arrives literally" ||
    fail "declared value changed"
[[ "$envdump" != *TEST_UNSET* ]] && pass "an unset declared name stays absent" || fail "TEST_UNSET present"
[[ "$(cat "$SANDBOX/marks/argv")" != *line2* && "$(cat "$SANDBOX/marks/argv")" != *fast* ]] &&
    pass "no declared value is in the script argv" || fail "argv holds a value: $(cat "$SANDBOX/marks/argv")"
[[ ! -e "$PWNED" ]] && pass "the declared value ran nothing" || fail "declared value ran a command"
unset TEST_SECRET GPY_OPT_mode TEST_DECLARED

echo "=== (f) once per command, ttl and key ==="
# A non-interactive Bash moves `\#` on every command, so this section drives
# the DEBUG-mark guard (Bash < 4.4); script_segment_interactive covers `\#`
# and the DEBUG trap itself on a real prompt.
saved_cmdno=$__gpy_g_cmdno
__gpy_g_cmdno=""
mkscript count 'echo x >>"'"$SANDBOX"'/marks/count"; echo counted'
reset_seg count
__gpy_script_seg_ttl=0
activate
due
__gpy_g_maybe_run
__gpy_g_maybe_run
check "redraws without a command do not run it again" "1" "$(runs count)"
check "a redraw keeps the text" "counted" "$(out)"
cd "$SANDBOX" || exit 1
__gpy_g_maybe_run
check "a changed working directory makes it due" "2" "$(runs count)"
cd "$ROOT" || exit 1
__gpy_g_maybe_run
check "changing back is due again" "3" "$(runs count)"
__gpy_g_ran=1
__gpy_g_maybe_run
check "a command seen by the DEBUG trap makes it due" "4" "$(runs count)"
__gpy_g_maybe_run
check "ttl 0 still runs once per command only" "4" "$(runs count)"
saved_dm=$__gpy_duration_method
__gpy_duration_method=none
__gpy_needs_debug_trap && pass "Bash < 4.4 with a script segment needs the DEBUG trap" ||
    fail "no DEBUG trap for the script guard"
__gpy_duration_method=$saved_dm

__gpy_script_seg_ttl=60
__gpy_script_seg_env=(TEST_KEY)
activate
export TEST_KEY=one
due
due
check "a due prompt within ttl reuses the cached text" "5" "$(runs count)"
export TEST_KEY=two
due
check "a changed declared value is a cache miss" "6" "$(runs count)"
__gpy_script_seg_key="3:sex"
activate
due
check "a changed static key (re-export) is a cache miss" "7" "$(runs count)"
__gpy_g_ct_seg=$((__gpy_g_ct_seg - 120))
due
check "an entry older than ttl is a miss" "8" "$(runs count)"
__gpy_script_seg_reqenv=(TEST_ON)
activate
export TEST_ON=1
due
check "require_env holds: within ttl a cache hit" "8" "$(runs count)"
unset TEST_ON
due
[[ -z ${__gpy_g_ck_seg+x} ]] && pass "a failed trigger drops the cache entry" ||
    fail "cache entry kept after a failed trigger"
export TEST_ON=1
due
check "the trigger back runs again (no stale cache hit)" "9" "$(runs count)"
unset TEST_ON
__gpy_script_seg_reqenv=()
unset TEST_KEY
__gpy_g_cmdno=$saved_cmdno

echo "=== (g) timeout with a grandchild holding stdout ==="
mkscript hang "(sleep $MARK; echo late) & sleep $MARK"
reset_seg hang
__gpy_script_seg_timeout=200
activate
t0=$(now_ms)
due
elapsed=$(($(now_ms) - t0))
((elapsed < 900)) && pass "the run returned in ${elapsed} ms (timeout 200)" ||
    fail "the run took ${elapsed} ms with a 200 ms timeout"
! __gpy_g_visible seg && pass "a timed-out segment is hidden" || fail "timed-out segment shown"
shell_e2e_poll 2 no_survivors && pass "the script tree was killed" || fail "$(survivors) sleeps survived"
mkscript leftover "sleep $MARK & echo done"
reset_seg leftover
activate
t0=$(now_ms)
due
elapsed=$(($(now_ms) - t0))
check "a normal exit with a background child shows its text" "done" "$(out)"
((elapsed < 600)) && pass "leftovers do not hold the prompt (${elapsed} ms)" ||
    fail "a leftover held the prompt for ${elapsed} ms"
shell_e2e_poll 2 no_survivors && pass "leftovers of a normal exit are killed" || fail "leftover survived"

echo "=== (h) require_command ==="
mkscript gated 'echo x >>"'"$SANDBOX"'/marks/gated"; echo gated'
mkscript reqhang "sleep $MARK"
mkscript reqfail 'echo noise; exit 1'
mkscript reqok 'echo noise'
reset_seg gated
__gpy_script_seg_timeout=200
__gpy_script_seg_reqcommand="$PLUG/bin/reqhang.sh"
activate
t0=$(now_ms)
due
elapsed=$(($(now_ms) - t0))
((elapsed < 900)) && pass "a hanging require_command is bounded by the timeout (${elapsed} ms)" ||
    fail "a hanging require_command took ${elapsed} ms"
check "the script never ran after a timed-out require_command" "0" "$(runs gated)"
shell_e2e_poll 2 no_survivors && pass "the require_command tree was killed" || fail "require_command survived"
__gpy_script_seg_reqcommand="$PLUG/bin/reqfail.sh"
activate
due
check "a failing require_command skips the script" "0" "$(runs gated)"
! __gpy_g_visible seg && pass "a failing require_command hides the segment" || fail "segment shown"
__gpy_script_seg_reqcommand="$PLUG/bin/reqok.sh"
activate
due
check "a passing require_command lets the script run, its output discarded" "gated" "$(out)"

echo "=== circuit breaker ==="
reset_seg hang
__gpy_script_seg_timeout=50
activate
due 2>/dev/null
due 2>"$SANDBOX/marks/stderr"
[[ "$(cat "$SANDBOX/marks/stderr")" == *"script segment seg disabled"* ]] &&
    pass "the second timeout in a row disables the segment with one stderr line" ||
    fail "no breaker message: $(cat "$SANDBOX/marks/stderr")"
mkscript hang 'echo x >>"'"$SANDBOX"'/marks/hang"; echo back'
due
check "a disabled segment does not run" "0" "$(runs hang)"
__gpy_g_sync
due
check "a theme reload resets the breaker" "back" "$(out)"

echo "=== (e) no files ==="
created="$(find "$HOME" "$TMPDIR" "$XDG_CACHE_HOME" "$XDG_RUNTIME_DIR" "$ROOT/bash" -newer "$SANDBOX/stamp" 2>/dev/null)"
check "no file created under HOME, TMPDIR, XDG dirs or the integration" "" "$created"
check "the PID-named planted files were never read or changed" "STALE" "$(cat "$TMPDIR/gpy-g-$$")"

test_finish "script segments in Bash $BASH_VERSION"

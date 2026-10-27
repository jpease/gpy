#!/usr/bin/env zsh
# tests/zsh/script_segment_lifecycle.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# When the Zsh adapter runs a script (G) segment (docs/dev/plugins.md
# "Runner"): once per command, never on a repaint; the cache in shell
# variables (key, ttl, folder scope); triggers before the script; the reload
# doorbell; the circuit breaker.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
source "$ROOT/tests/lib/script_segment_fixture.zsh"
cd "$G_SANDBOX/work" || exit 1

# --- once per command ---------------------------------------------------------
g_segment once 'printf "n%s" "$GPY_OPT_k"'
__gpy_script_once_ttl=0
__gpy_script_once_opts=(GPY_OPT_k=1)
g_enable once
g_command
check "the first prompt runs it" 1 "$(g_runs)"
check "and draws it" true "$([[ $(g_shown) == *n1* ]] && echo true || echo false)"
for i in 1 2 3; do __gpy_update_prompts 0; done
check "repaints (TRAPURG's __gpy_update_prompts) never run it" 1 "$(g_runs)"
check "a repaint draws the stored text" true "$([[ $(g_shown) == *n1* ]] && echo true || echo false)"
g_empty_enter
g_empty_enter
check "an empty Enter (no preexec) does not run it" 1 "$(g_runs)"
g_command
check "the next command runs it (ttl 0)" 2 "$(g_runs)"
mkdir -p "$G_SANDBOX/work/elsewhere"
cd "$G_SANDBOX/work/elsewhere"
g_empty_enter
check "a cd without a command (a widget) makes it due" 3 "$(g_runs)"
cd "$G_SANDBOX/work"
: >"$G_RUNS"

# --- cache: ttl, key, folder scope -------------------------------------------
g_segment cache 'printf "%s" "${KUBECONFIG-none}"'
__gpy_script_cache_ttl=2
__gpy_script_cache_env=(KUBECONFIG)
g_enable cache
g_command
g_command
check "within ttl a second command hits the cache" 1 "$(g_runs)"
KUBECONFIG=/a
g_command
check "a declared env value is part of the key" 2 "$(g_runs)"
check "and reaches the script" "/a" "$(g_text cache)"
unset KUBECONFIG
g_command
check "unset differs from set" 3 "$(g_runs)"
check "an unset name is not forwarded" "none" "$(g_text cache)"
KUBECONFIG=""
g_command
check "set-but-empty differs from unset" 4 "$(g_runs)"
sleep 2.2
g_command
check "after ttl it runs again" 5 "$(g_runs)"
__gpy_script_cache_key=changed
__gpy_g_reload
g_command
check "a changed static key after a reload is a miss" 6 "$(g_runs)"
: >"$G_RUNS"

g_segment fold 'printf "%s" "$GPY_PWD"'
__gpy_script_fold_inputs=(pwd)
__gpy_script_fold_scope=folder
__gpy_script_fold_ttl=60
mkdir -p "$G_SANDBOX/work/f1" "$G_SANDBOX/work/f2"
g_enable fold
cd "$G_SANDBOX/work/f1"; g_command
cd "$G_SANDBOX/work/f2"; g_command
check "folder scope: each folder runs" 2 "$(g_runs)"
check "folder scope: the script runs in \$PWD" "$G_SANDBOX/work/f2" "$(g_text fold)"
cd "$G_SANDBOX/work/f2"; g_command
check "folder scope: same folder hits" 2 "$(g_runs)"
cd "$G_SANDBOX/work"
: >"$G_RUNS"

g_segment rootdir 'pwd'
g_enable rootdir
g_command
check "session scope: the script runs in the plugin root" "$G_PLUGIN" "$(g_text rootdir)"

# --- triggers before the script ----------------------------------------------
g_segment trig 'printf on'
__gpy_script_trig_ttl=60
__gpy_script_trig_reqenv=(GPY_TEST_FLAG)
g_enable trig
: >"$G_RUNS"
g_command
check "require_env unset: hidden, not run" "0|" "$(g_runs)|$(g_text trig)"
GPY_TEST_FLAG=1
g_command
check "require_env set: run and shown" "1|on" "$(g_runs)|$(g_text trig)"
unset GPY_TEST_FLAG
g_command
check "a cache hit is still hidden when its trigger fails" "1|" "$(g_runs)|$(g_text trig)"
check "a failed trigger drops the cache entry" 0 "${+__gpy_g_key[trig]}"
GPY_TEST_FLAG=1
g_command
check "the trigger back runs again (no stale cache hit)" "2|on" "$(g_runs)|$(g_text trig)"
unset GPY_TEST_FLAG
__gpy_script_trig_reqenv=()
__gpy_script_trig_reqfiles=(Marker.yaml)
__gpy_script_trig_reqfolders=(.kube)
__gpy_script_trig_reqexts=(tf)
g_enable trig
mkdir -p "$G_SANDBOX/work/proj/.kube" "$G_SANDBOX/work/proj/a/b"
: >"$G_SANDBOX/work/proj/Marker.yaml"
cd "$G_SANDBOX/work/proj/a/b"
g_command
check "files and folders found upwards but no .tf here: hidden" "" "$(g_text trig)"
: >"$G_SANDBOX/work/proj/a/b/main.tf"
g_command
check "every trigger holds: shown" "on" "$(g_text trig)"
cd "$G_SANDBOX/work/proj/a"
g_command
check "require_extensions looks in \$PWD only" "" "$(g_text trig)"
cd "$G_SANDBOX/work"

g_segment gated 'printf gated'
__gpy_script_gated_ttl=0
g_reqcommand gated 'exit 1'
g_enable gated
: >"$G_RUNS"
g_command
check "require_command failing: hidden and the script never runs" "0|" "$(g_runs)|$(g_text gated)"
g_reqcommand gated 'exit 0'
g_command
check "require_command passing: the script runs" "1|gated" "$(g_runs)|$(g_text gated)"

# --- circuit breaker -----------------------------------------------------------
g_segment hang 'sleep 3'
__gpy_script_hang_timeout=30
__gpy_script_hang_ttl=0
g_enable hang
: >"$G_RUNS"
err=$( { g_command; g_command; g_command; } 2>&1 >/dev/null )
check "two consecutive timeouts disable the segment, then it never runs" 2 "$(g_runs)"
check "with one stderr line naming it" 1 "$(print -r -- "$err" | grep -c 'script segment hang disabled')"
g_enable hang
: >"$G_RUNS"
g_command 2>/dev/null
check "the reload doorbell resets the breaker" 1 "$(g_runs)"

g_segment flaky 'printf ok'
g_enable flaky
__gpy_g_strike flaky run
g_command
check "a success resets the count" 0 "${__gpy_g_runfails[flaky]}"
__gpy_g_strike flaky run
check "so one more timeout does not trip it" "" "${__gpy_g_off[flaky]-}"

# --- no bash interpreter -----------------------------------------------------
# /bin/bash exists here, so the fallback path is swapped for a missing one.
no_bash() {
    (
        functions[__gpy_g_resolve_bash]=${functions[__gpy_g_resolve_bash]//\/bin\/bash/$G_SANDBOX/no/bash}
        path=()
        __gpy_g_bash=""
        __gpy_g_resolve_bash
        print -r -- "bash=$__gpy_g_bash"
    ) 2>&1
}
check "no bash, GPY_DEBUG=1: one warning, runner disabled" \
    "gpy[script]: warning: No bash found - script segments disabled|bash=" \
    "$(GPY_DEBUG=1 no_bash | paste -sd'|' -)"
check "no bash, GPY_VERBOSE=1: the same warning" \
    "gpy[script]: warning: No bash found - script segments disabled|bash=" \
    "$(GPY_VERBOSE=1 no_bash | paste -sd'|' -)"
check "no bash, no debug flags: silent" "bash=" "$(GPY_DEBUG= GPY_VERBOSE= no_bash)"

test_finish "script segment lifecycle"

#!/usr/bin/env zsh
# tests/zsh/script_segment_names_state.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Names and state the Zsh script (G) segment adapter reads
# (docs/dev/adr/adr-0011-addendum-script-segment-security.md):
#   - script_segment_manifest_names: tokens, env, require_env and setting
#     names like `a[$(touch F)]`, `*` or with a trailing newline, edited into
#     the exported variables by hand, never run code and never reach
#     ${(P)...}; glob characters in require_files/require_folders names are
#     literal names;
#   - script_segment_env_seeded_state: `__gpy_g_*` and `__gpy_script_*`
#     values pre-set in the parent environment change nothing;
#   - script_segment_no_pid_state: files named after the shell's PID
#     (stale state from a reused PID) change nothing;
#   - script_segment_no_files: runs, cache hits, triggers and timeouts create
#     no file anywhere under HOME or TMPDIR.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
source "$ROOT/tests/lib/script_segment_fixture.zsh"
cd "$G_SANDBOX/work" || exit 1

made() { [[ -e $G_SANDBOX/$1 ]] && echo true || echo false; }

# --- tokens ------------------------------------------------------------------
g_segment ok 'printf fine'
typeset -ga __gpy_script_segments
__gpy_script_segments=("a[\$(touch $G_SANDBOX/T1)]" '*' $'ok\n' 'ok' '1bad' 'ok')
__enabled_segments=(ok)
__gpy_g_reload
check "only the valid, defined token is active, once" "ok" "${(j:,:)__gpy_g_toks}"
g_command
check "the valid token still runs" "fine" "$(g_text ok)"
check "a token like a[\$(touch F)] ran nothing" false "$(made T1)"

# A token whose exported fields were edited into code.
g_segment bad 'printf never'
__gpy_script_bad_ttl="a[\$(touch $G_SANDBOX/T2)]"
g_enable ok bad
check "an edited numeric field drops the segment" "ok" "${(j:,:)__gpy_g_toks}"
check "and runs nothing" false "$(made T2)"
g_segment bad 'printf never'
__gpy_script_bad_inputs=(pwd bogus)
g_enable ok bad
check "an unknown input word drops the segment" "ok" "${(j:,:)__gpy_g_toks}"
g_segment bad 'printf never'
__gpy_script_bad_name="other"
g_enable ok bad
check "a name that does not match its token drops the segment" "ok" "${(j:,:)__gpy_g_toks}"

# --- env, require_env and setting names ---------------------------------------
g_segment envy 'printf "%s|%s" "${GOOD-unset}" "${GPY_OPT_k-unset}"'
GOOD=yes
__gpy_script_envy_env=("a[\$(touch $G_SANDBOX/E1)]" GOOD)
g_enable envy
g_command
check "an env name like a[\$(touch F)] hides the segment" "" "$(g_text envy)"
check "and runs nothing" false "$(made E1)"
__gpy_script_envy_env=(GOOD)
__gpy_script_envy_reqenv=("b[\$(touch $G_SANDBOX/E2)]")
g_enable envy
g_command
check "a require_env name like b[\$(touch F)] hides the segment" "" "$(g_text envy)"
check "and runs nothing" false "$(made E2)"
__gpy_script_envy_reqenv=()
__gpy_script_envy_opts=("GPY_OPT_x[\$(touch $G_SANDBOX/E3)]=1" 'GPY_OPT_k=v=w' $'GPY_OPT_\n=1' 'NOT_OPT=1')
g_enable envy
g_command
check "invalid setting entries are dropped, valid ones split at the first =" "yes|v=w" "$(g_text envy)"
check "and run nothing" false "$(made E3)"

# --- glob characters in trigger names ----------------------------------------
g_segment globby 'printf matched'
__gpy_script_globby_reqfiles=('*')
__gpy_script_globby_reqfolders=('[a-z]*')
mkdir -p "$G_SANDBOX/work/g/sub" && cd "$G_SANDBOX/work/g/sub" || exit 1
: >anything
mkdir -p plain
g_enable globby
g_command
check "require_files '*' is a literal name, not any file" "" "$(g_text globby)"
: >"$G_SANDBOX/work/g/*"
g_command
check "a file literally named * in an ancestor satisfies it, the folder still missing" "" "$(g_text globby)"
mkdir -p "$G_SANDBOX/work/[a-z]*"
g_command
check "both literal names found upwards: shown" "matched" "$(g_text globby)"
cd "$G_SANDBOX/work" || exit 1

# --- pre-seeded environment and PID-named files (fresh child shells) ---------
child() {
    env -i PATH=/usr/bin:/bin HOME="$G_SANDBOX/home" TMPDIR="$G_SANDBOX/tmp" \
        XDG_CONFIG_HOME="$XDG_CONFIG_HOME" XDG_CACHE_HOME="$XDG_CACHE_HOME" \
        GPY_AGENT_ENABLED=0 GPY_AGENT_SUPERVISOR_ENABLED=0 \
        GPY_AGENT_SOCKET_PATH="$G_SANDBOX/missing.sock" GPY_TEST_ROOT="$ROOT" \
        "$@" zsh -f -c '
for f in "$TMPDIR/$$" "$TMPDIR/gpy_g_$$" "$TMPDIR/gpy-g-$$.cache" "$HOME/.gpy_g_$$"; do
    print -r -- PLANTED >"$f"
done
source "$GPY_TEST_ROOT/zsh/gpy.zsh"
print -r -- "toks=[${(j:,:)__gpy_g_toks}] due=[$__gpy_g_due] off=[${(kj:,:)__gpy_g_off}]"
__gpy_preexec; true; __gpy_precmd
print -r -- "text=[${(kj:,:)__gpy_g_text}]"
' 2>&1
}
mkdir -p "$G_SANDBOX/home" "$G_SANDBOX/tmp"
out=$(child __gpy_g_due="a[\$(touch $G_SANDBOX/S1)]" __gpy_g_runfails="a[\$(touch $G_SANDBOX/S2)]" \
    __gpy_g_at="a[\$(touch $G_SANDBOX/S3)]" __gpy_g_toks=ok __gpy_script_segments=ok \
    __gpy_script_ok_command=/bin/echo __gpy_script_ok_ttl="a[\$(touch $G_SANDBOX/S4)]")
check "seeded __gpy_g_*/__gpy_script_* values leave no active segment" true \
    "$([[ $out == *'toks=[] due=[1] off=[]'* && $out == *'text=[]'* ]] && echo true || echo false)"
for f in S1 S2 S3 S4; do
    check "seeded state ran no code ($f)" false "$(made $f)"
done
check "the child printed no error" false "$([[ $out == *error* || $out == *'not found'* ]] && echo true || echo false)"

# PID-named files beside a real run: the segment shows the script's output.
g_segment pid 'printf "%s" "real"'
g_enable pid
for f in "$TMPDIR/$$" "$TMPDIR/gpy_g_$$" "$HOME/.gpy_g_$$"; do print -r -- PLANTED >"$f"; done
g_command
check "PID-named files change nothing: the script's own text" "real" "$(g_text pid)"
rm -f "$TMPDIR/$$" "$TMPDIR/gpy_g_$$" "$HOME/.gpy_g_$$"

# --- no files -----------------------------------------------------------------
listing() { find "$G_SANDBOX" -newer "$G_SANDBOX/stamp" -print 2>/dev/null | grep -vxF "$G_RUNS" | sort; }
g_segment cached 'printf one'
g_segment uncached 'printf two'
__gpy_script_uncached_ttl=0
g_segment slow 'sleep 2'
__gpy_script_slow_timeout=50
g_segment scanned 'printf three'
__gpy_script_scanned_reqfiles=(nosuchfile)
__gpy_script_scanned_reqexts=(tf)
mkdir -p "$G_SANDBOX/work/nf" && cd "$G_SANDBOX/work/nf" || exit 1
g_enable cached uncached slow scanned
sleep 1
: >"$G_SANDBOX/stamp"
sleep 1
# Touched again: creating it changed the sandbox directory after its mtime.
touch "$G_SANDBOX/stamp"
fds_before=(/dev/fd/<->(N:t))
for i in 1 2 3; do g_command; g_empty_enter; __gpy_update_prompts 0; done 2>/dev/null
fds_after=(/dev/fd/<->(N:t))
check "runs, hits, a scan and timeouts created no file under HOME, TMPDIR or the sandbox" "" "$(listing)"
check "the cached and uncached segments still ran" "one two" "$(g_text cached) $(g_text uncached)"
check "no descriptor is left open" "${(j: :)fds_before}" "${(j: :)fds_after}"

test_finish "script segment names and state"

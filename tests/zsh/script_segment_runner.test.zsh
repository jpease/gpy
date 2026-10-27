#!/usr/bin/env zsh
# tests/zsh/script_segment_runner.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The Zsh script (G) segment runner (docs/dev/plugins.md "Environment and
# process rules", "Runner"):
#   - timeout with a grandchild holding stdout: the prompt returns within the
#     timeout plus the grace, the segment is hidden, the tree is killed;
#   - require_command under the same timeout, killed the same way;
#   - script_segment_leftover_kill: a normal exit's leftover is killed;
#   - script_segment_clean_env: only the base set, declared env names and
#     GPY_* reach the script; hostile values arrive literally and never in
#     any process's argv;
#   - script_segment_job_state: $!, the job table and descriptors unchanged.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
source "$ROOT/tests/lib/script_segment_fixture.zsh"
cd "$G_SANDBOX/work" || exit 1

alive() { kill -0 "$1" 2>/dev/null && echo true || echo false; }
ms_since() { local -i ms=$(( (EPOCHREALTIME - $1) * 1000 )); print -r -- $ms; }

# --- timeout, grandchild holding stdout --------------------------------------
g_segment hold "sleep 30 &
printf '%s' \"\$!\" >'$G_SANDBOX/gc.pid'
printf partial
sleep 30"
__gpy_script_hold_timeout=200
__gpy_script_hold_ttl=0
g_enable hold
t0=$EPOCHREALTIME
g_command 2>/dev/null
el=$(ms_since $t0)
check "a hung script with a grandchild holding stdout returns within T + grace + slack (${el} ms)" true \
    "$( (( el < 200 + 50 + 400 )) && echo true || echo false)"
check "and is hidden" "" "$(g_text hold)"
sleep 0.3
gc=$(<"$G_SANDBOX/gc.pid")
check "the grandchild was killed with its group" false "$(alive "$gc")"

# --- require_command under the timeout ---------------------------------------
g_segment gate 'printf never'
__gpy_script_gate_timeout=150
__gpy_script_gate_ttl=0
g_reqcommand gate "sleep 30 &
printf '%s' \"\$!\" >'$G_SANDBOX/req.pid'
sleep 30"
g_enable gate
: >"$G_RUNS"
t0=$EPOCHREALTIME
g_command 2>/dev/null
el=$(ms_since $t0)
check "a hung require_command returns within its timeout + grace + slack (${el} ms)" true \
    "$( (( el < 150 + 50 + 400 )) && echo true || echo false)"
check "the command never ran and the segment is hidden" "0|" "$(g_runs)|$(g_text gate)"
sleep 0.3
check "the require_command tree was killed" false "$(alive "$(<"$G_SANDBOX/req.pid")")"

# require_command and command share one deadline.
g_segment shared 'sleep 30'
__gpy_script_shared_timeout=300
__gpy_script_shared_ttl=0
g_reqcommand shared 'sleep 0.2'
g_enable shared
t0=$EPOCHREALTIME
g_command 2>/dev/null
el=$(ms_since $t0)
check "require_command and the script share one timeout (${el} ms)" true \
    "$( (( el < 300 + 50 + 400 )) && echo true || echo false)"

# --- leftover after a normal exit ----------------------------------------------
g_segment leave "(trap '' TERM; sleep 30) &
printf '%s' \"\$!\" >'$G_SANDBOX/left.pid'
printf done"
g_enable leave
g_command
check "a normal exit is shown" "done" "$(g_text leave)"
sleep 0.3
check "its TERM-ignoring leftover was killed" false "$(alive "$(<"$G_SANDBOX/left.pid")")"

# --- clean environment, values never in argv ---------------------------------
export AWS_SECRET_ACCESS_KEY=topsecret SSH_AUTH_SOCK=/tmp/agent.sock GPY_OPT_k=user-exported
hostile=$'v1 $(touch '"$G_SANDBOX"$'/ENVF) `id` * "q" \'s\'\nline2'
export DECLARED=$hostile
PATH=".:relative/bin::$PATH"
g_segment envcheck "
printf '%s' \"\$DECLARED\" >'$G_SANDBOX/declared.out'
ps -A -o args= >'$G_SANDBOX/ps.out' 2>/dev/null
printf '%s|%s|%s|%s|%s' \"\${AWS_SECRET_ACCESS_KEY-absent}\" \"\${SSH_AUTH_SOCK-absent}\" \"\${GPY_OPT_k-absent}\" \"\${GPY_IS_SSH-absent}\" \"\$PATH\""
__gpy_script_envcheck_env=(DECLARED)
__gpy_script_envcheck_inputs=(is_ssh)
__gpy_script_envcheck_opts=(GPY_OPT_k=from-settings)
g_enable envcheck
g_command
fields=("${(@s:|:)$(g_text envcheck)}")
sec=${fields[1]} sock=${fields[2]} opt=${fields[3]} ssh=${fields[4]} spath=${fields[5]}
check "undeclared secrets are absent" "absent|absent" "$sec|$sock"
check "GPY_OPT_* comes from the settings, not the user's export" "from-settings" "$opt"
check "a declared input is set" "${__gpy_is_ssh}" "$ssh"
check "relative PATH entries are dropped" false \
    "$([[ ":$spath:" == *:.:* || ":$spath:" == *:relative/bin:* || ":$spath:" == *::* ]] && echo true || echo false)"
check "a hostile declared value arrives byte for byte" "$hostile" "$(<"$G_SANDBOX/declared.out")"
check "and runs nothing" false "$([[ -e $G_SANDBOX/ENVF ]] && echo true || echo false)"
check "no process had the declared value in its argv" 0 "$(grep -c -F 'topsecret' "$G_SANDBOX/ps.out"; true)"
check "no process had the declared value in its argv (declared)" 0 "$(grep -c -F 'v1 $(touch' "$G_SANDBOX/ps.out"; true)"
mkdir -p "$G_SANDBOX/work/argvdir_$$"
cd "$G_SANDBOX/work/argvdir_$$"
g_segment pwdps "ps -A -o args= >'$G_SANDBOX/ps2.out' 2>/dev/null; printf ok"
__gpy_script_pwdps_inputs=(pwd)
__gpy_script_pwdps_scope=folder
g_enable pwdps
g_command
check "a folder-scope run keeps \$PWD out of argv" 0 "$(grep -c -F "argvdir_$$" "$G_SANDBOX/ps2.out"; true)"
cd "$G_SANDBOX/work"
PATH=${PATH#.:relative/bin::}
unset AWS_SECRET_ACCESS_KEY SSH_AUTH_SOCK GPY_OPT_k DECLARED

# --- job state -----------------------------------------------------------------
sleep 5 &
bg_pid=$!
fds_before=(/dev/fd/<->(N:t))
jobs_before=$(jobs -l)
g_segment quick 'printf q'
__gpy_script_quick_ttl=0
g_segment scanq 'printf s'
__gpy_script_scanq_reqfiles=(nothing-here)
__gpy_script_scanq_ttl=0
g_enable quick scanq hold
g_command 2>/dev/null
check "\$! is unchanged by a run and a scan" "$bg_pid" "$!"
check "the job table is unchanged" "$jobs_before" "$(jobs -l)"
fds_after=(/dev/fd/<->(N:t))
check "no descriptor is left open" "${(j: :)fds_before}" "${(j: :)fds_after}"
kill "$bg_pid" 2>/dev/null

test_finish "script segment runner"

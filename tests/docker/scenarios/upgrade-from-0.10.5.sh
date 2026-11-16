#!/usr/bin/env bash
# tests/docker/scenarios/upgrade-from-0.10.5.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Upgrade a real v0.10.5 install, with its agent running and a Bash session
# holding a registered shell, to the freshly built release through the new
# install-oneline.sh. A new shell must talk to a replaced agent of the new
# version, and `gpy doctor` must report no version mismatch and exit 0.

set -euo pipefail
# shellcheck source=tests/docker/scenario-lib.sh
source /opt/scenario-lib.sh

OLD_TAG=v0.10.5
OLD_BARE="${OLD_TAG#v}"

step "install $OLD_TAG from its published assets"
install_release "$OLD_TAG" bash
[[ "$("$LOCAL_BIN/gpy-agent" --version 2>&1)" == "gpy-agent $OLD_BARE" ]] ||
    abort "the $OLD_TAG gpy-agent does not report $OLD_BARE"
# shellcheck disable=SC2329 # called through poll
old_agent_up() { [[ "$(agent_field version)" == "$OLD_BARE" ]]; }
"$LOCAL_BIN/gpy-agent" start >/dev/null 2>&1 || true
poll 10 old_agent_up || abort "the $OLD_TAG agent is not running: $("$LOCAL_BIN/gpy-agent" status 2>&1 | head -n 5)"
old_pid="$(agent_field pid)"
pass "the $OLD_TAG agent runs as PID $old_pid"

step "a Bash session stays open across the upgrade"
REPO="$WORK/repo"
make_repo "$REPO"
mkfifo "$WORK/held.in"
(cd "$REPO" && exec bash -i <"$WORK/held.in" >"$WORK/held.log" 2>&1) &
held_pid=$!
# Hold the write end open so the shell keeps waiting for input.
exec 7>"$WORK/held.in"
# shellcheck disable=SC2329 # called through poll
held_registered() { local n; n="$(agent_field registered_clients)"; (( ${n:-0} >= 1 )); }
poll 15 held_registered ||
    abort "the held Bash session never registered with the $OLD_TAG agent; its output: $(head -c 2000 "$WORK/held.log")"
pass "the held Bash session (PID $held_pid) is registered with the $OLD_TAG agent"

step "upgrade through the new install-oneline.sh"
if upgrade_to_new bash "$WORK/upgrade.log"; then
    pass "install-oneline.sh $GPY_RELEASE_VERSION exited 0 over the running $OLD_TAG install"
else
    show_log "$WORK/upgrade.log"
    abort "install-oneline.sh $GPY_RELEASE_VERSION failed over the $OLD_TAG install"
fi
for bin in gpy-agent gpy; do
    out="$("$LOCAL_BIN/$bin" --version 2>&1 || true)"
    if [[ "$out" == "$bin $BARE" ]]; then
        pass "$bin --version reports $BARE"
    else
        fail "$bin --version: expected '$bin $BARE', got '$out'"
    fi
done

step "a new shell after the upgrade"
cd "$REPO"
new_prompt="$(bash -i -c '__gpy_precmd; printf "%s" "$PS1"' </dev/null 2>&1 || true)"
case "$new_prompt" in
    *"gpy[init]"* | *"command not found"* | *rror*) fail "the new Bash shell's prompt carries error text: $new_prompt" ;;
    *repo*) pass "a new Bash shell renders a prompt naming the repository" ;;
    *) fail "the new Bash shell's prompt does not name the repository: $new_prompt" ;;
esac
cd "$WORK"

step "the old agent is replaced"
# shellcheck disable=SC2329 # called through poll
new_agent_up() { [[ "$(agent_field version)" == "$BARE" ]]; }
if poll 15 new_agent_up; then
    pass "the running agent reports $BARE"
else
    fail "the running agent reports '$(agent_field version)', expected $BARE"
fi
new_pid="$(agent_field pid)"
if [[ -n "$new_pid" && "$new_pid" != "$old_pid" ]]; then
    pass "the agent PID changed ($old_pid -> $new_pid)"
else
    fail "the agent PID did not change (was $old_pid, now '${new_pid:-<none>}')"
fi
# A daemon's parent here is PID 1, this script; an exited, unreaped agent is
# a zombie (state Z), not a running one.
if [[ -r "/proc/$old_pid/stat" && "$(awk '{print $3}' "/proc/$old_pid/stat" 2>/dev/null)" != Z ]]; then
    fail "the $OLD_TAG agent (PID $old_pid) is still alive: $(tr '\0' ' ' </proc/"$old_pid"/cmdline 2>/dev/null)"
else
    pass "the $OLD_TAG agent (PID $old_pid) has exited"
fi
running_agents="$(pgrep -u "$(id -u)" -x gpy-agent | wc -l)"
if (( running_agents == 1 )); then
    pass "exactly one gpy-agent process runs"
else
    fail "$running_agents gpy-agent processes run: $(pgrep -a -u "$(id -u)" -x gpy-agent | tr '\n' ';')"
fi

step "gpy doctor"
doctor_rc=0
(cd "$REPO" && "$LOCAL_BIN/gpy" doctor) >"$WORK/doctor.log" 2>&1 || doctor_rc=$?
if (( doctor_rc == 0 )); then
    pass "gpy doctor exits 0"
else
    fail "gpy doctor exited $doctor_rc"
    show_log "$WORK/doctor.log"
fi
if grep -qF "Agent is current ($BARE)" "$WORK/doctor.log"; then
    pass "gpy doctor reports the agent current ($BARE)"
else
    fail "gpy doctor does not report 'Agent is current ($BARE)'"
fi
if grep -q 'Running agent is\|outdated\|[Mm]ismatch' "$WORK/doctor.log"; then
    fail "gpy doctor reports a version mismatch: $(grep 'Running agent is\|outdated\|[Mm]ismatch' "$WORK/doctor.log")"
else
    pass "gpy doctor reports no version mismatch"
fi

step "the held session survives the upgrade"
echo 'echo held-shell-alive' >&7
# shellcheck disable=SC2329 # called through poll
held_answered() { grep -q '^held-shell-alive' "$WORK/held.log"; }
if kill -0 "$held_pid" 2>/dev/null && poll 10 held_answered; then
    pass "the Bash session held across the upgrade still runs commands"
else
    fail "the held Bash session died or stopped answering; its output: $(tail -c 2000 "$WORK/held.log")"
fi
exec 7>&-
"$LOCAL_BIN/gpy-agent" stop >/dev/null 2>&1 || true

finish

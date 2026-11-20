#!/usr/bin/env bash
# tests/docker/scenarios/upgrade-from-0.11.0.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Upgrade a real v0.11.0 install, wired into Bash, Zsh and Fish, with its agent
# running and one interactive session of each shell open, to the freshly built
# release through the new install-oneline.sh. 0.12 replaced the shell
# integrations (every prompt through `gpy prompt`, no theme export, no core/
# trees), so beyond what upgrade-from-0.10.5.sh checks this one asks that:
#   - a new session of each shell draws a prompt naming the repository with no
#     error text;
#   - each session held open across the upgrade, still running 0.11 shell code,
#     keeps drawing prompts without error text.
# Sessions run under script(1), so each shell has a terminal and draws its
# prompts as it would for a user.

set -euo pipefail
# shellcheck source=tests/docker/scenario-lib.sh
source /opt/scenario-lib.sh

OLD_TAG=v0.11.0
OLD_BARE="${OLD_TAG#v}"
SHELLS=(bash zsh fish)
export TERM=xterm-256color
# What a prompt or a shell hook prints when the shell code it runs is broken.
ERRORS='command not found|[Nn]o such file|gpy\[init\]|[Ee]rror|[Uu]nrecognized|[Uu]nknown (command|subcommand|function)'

# `plain FILE OFFSET` -- FILE after byte OFFSET as text: terminal escapes
# (CSI, OSC titles, charset selects, keypad modes) and carriage returns
# removed, so a command's output starts its line as it does on screen.
plain() {
    tail -c +"$(($2 + 1))" "$1" |
        perl -pe 's/\e\[[0-9;?<>=]*[ -\/]*[@-~]//g; s/\e\][^\a\e]*(?:\a|\e\\)//g; s/\e[()][A-Za-z0-9]//g; s/\e[=>]//g; s/\r//g'
}

# `errors_in FILE OFFSET` -- error lines in FILE after byte OFFSET.
errors_in() { plain "$1" "$2" | grep -E "$ERRORS" | head -n 5 || true; }

step "install $OLD_TAG for every shell from its published assets"
for sh in "${SHELLS[@]}"; do
    install_release "$OLD_TAG" "$sh"
done
[[ "$("$LOCAL_BIN/gpy-agent" --version 2>&1)" == "gpy-agent $OLD_BARE" ]] ||
    abort "the $OLD_TAG gpy-agent does not report $OLD_BARE"
# shellcheck disable=SC2329 # called through poll
old_agent_up() { [[ "$(agent_field version)" == "$OLD_BARE" ]]; }
"$LOCAL_BIN/gpy-agent" start >/dev/null 2>&1 || true
poll 10 old_agent_up || abort "the $OLD_TAG agent is not running: $("$LOCAL_BIN/gpy-agent" status 2>&1 | head -n 5)"
old_pid="$(agent_field pid)"
pass "the $OLD_TAG agent runs as PID $old_pid"

step "one session of each shell stays open across the upgrade"
REPO="$WORK/repo"
make_repo "$REPO"
declare -A held_pid held_fd held_off
for sh in "${SHELLS[@]}"; do
    mkfifo "$WORK/held-$sh.in"
    (cd "$REPO" && exec script -qfec "$sh -i" "$WORK/held-$sh.log" <"$WORK/held-$sh.in" >/dev/null 2>&1) &
    held_pid[$sh]=$!
    # Hold the write end open so the session keeps waiting for input.
    exec {fd}>"$WORK/held-$sh.in"
    held_fd[$sh]=$fd
done
# shellcheck disable=SC2329 # called through poll
held_registered() { local n; n="$(agent_field registered_clients)"; ((${n:-0} >= ${#SHELLS[@]})); }
poll 30 held_registered ||
    abort "the held sessions did not all register with the $OLD_TAG agent ($(agent_field registered_clients) did)"
for sh in "${SHELLS[@]}"; do
    # shellcheck disable=SC2329 # called through poll
    drew_repo() { grep -q repo "$WORK/held-$sh.log"; }
    poll 15 drew_repo || abort "the held $sh session never drew a prompt naming the repository: $(tail -c 1500 "$WORK/held-$sh.log")"
    bad="$(errors_in "$WORK/held-$sh.log" 0)"
    [[ -z "$bad" ]] || abort "the held $sh session already shows errors under $OLD_TAG: $bad"
    held_off[$sh]="$(wc -c <"$WORK/held-$sh.log")"
done
pass "a $OLD_TAG session of each shell (${SHELLS[*]}) is open and registered"

step "upgrade through the new install-oneline.sh, once per shell"
for sh in "${SHELLS[@]}"; do
    if upgrade_to_new "$sh" "$WORK/upgrade-$sh.log"; then
        pass "install-oneline.sh $GPY_RELEASE_VERSION (GPY_SHELL=$sh) exited 0 over the running $OLD_TAG install"
    else
        show_log "$WORK/upgrade-$sh.log"
        abort "install-oneline.sh $GPY_RELEASE_VERSION (GPY_SHELL=$sh) failed over the $OLD_TAG install"
    fi
done
for bin in gpy-agent gpy; do
    out="$("$LOCAL_BIN/$bin" --version 2>&1 || true)"
    if [[ "$out" == "$bin $BARE" ]]; then
        pass "$bin --version reports $BARE"
    else
        fail "$bin --version: expected '$bin $BARE', got '$out'"
    fi
done

step "a new session of each shell after the upgrade"
for sh in "${SHELLS[@]}"; do
    log="$WORK/new-$sh.log"
    (cd "$REPO" && printf 'exit\n' | timeout 30 script -qfec "$sh -i" "$log" >/dev/null 2>&1) || true
    bad="$(errors_in "$log" 0)"
    if [[ -n "$bad" ]]; then
        fail "a new $sh session shows error text: $bad"
    elif grep -q repo "$log"; then
        pass "a new $sh session draws a prompt naming the repository, with no error text"
    else
        fail "a new $sh session drew no prompt naming the repository: $(tail -c 1500 "$log")"
    fi
done

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
running_agents="$(pgrep -u "$(id -u)" -x gpy-agent | wc -l)"
if ((running_agents == 1)); then
    pass "exactly one gpy-agent process runs"
else
    fail "$running_agents gpy-agent processes run: $(pgrep -a -u "$(id -u)" -x gpy-agent | tr '\n' ';')"
fi

step "gpy doctor"
doctor_rc=0
(cd "$REPO" && "$LOCAL_BIN/gpy" doctor) >"$WORK/doctor.log" 2>&1 || doctor_rc=$?
if ((doctor_rc == 0)); then
    pass "gpy doctor exits 0"
else
    fail "gpy doctor exited $doctor_rc"
    show_log "$WORK/doctor.log"
fi
if grep -q 'Running agent is\|outdated\|[Mm]ismatch' "$WORK/doctor.log"; then
    fail "gpy doctor reports a version mismatch: $(grep 'Running agent is\|outdated\|[Mm]ismatch' "$WORK/doctor.log")"
else
    pass "gpy doctor reports no version mismatch"
fi

step "the sessions held across the upgrade keep drawing prompts"
for sh in "${SHELLS[@]}"; do
    fd="${held_fd[$sh]}"
    log="$WORK/held-$sh.log"
    # Two commands, so the old shell code draws two prompts after the upgrade.
    echo "echo held-$sh-one" >&"$fd"
    echo "echo held-$sh-two" >&"$fd"
    # shellcheck disable=SC2329 # called through poll
    answered() { plain "$log" "${held_off[$sh]}" | grep -q "^held-$sh-two"; }
    if ! kill -0 "${held_pid[$sh]}" 2>/dev/null || ! poll 15 answered; then
        fail "the held $sh session died or stopped answering: $(plain "$log" 0 | tail -c 1500)"
        continue
    fi
    # Let the prompt after the last command finish drawing.
    sleep 1
    bad="$(errors_in "$log" "${held_off[$sh]}")"
    if [[ -n "$bad" ]]; then
        fail "the held $sh session shows error text after the upgrade: $bad"
    elif plain "$log" "${held_off[$sh]}" | sed -n '/^held-'"$sh"'-two/,$p' | grep -q repo; then
        pass "the held $sh session still runs commands and draws prompts naming the repository, with no error text"
    else
        fail "the held $sh session drew no prompt naming the repository after the upgrade: $(plain "$log" 0 | tail -c 1500)"
    fi
done
for sh in "${SHELLS[@]}"; do
    fd="${held_fd[$sh]}"
    { echo exit >&"$fd"; } 2>/dev/null || true
    exec {fd}>&-
done
"$LOCAL_BIN/gpy-agent" stop >/dev/null 2>&1 || true

finish

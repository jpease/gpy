#!/usr/bin/env bash
# tests/docker/scenarios/fsmonitor-not-run.sh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# A repository's own config can name a `core.fsmonitor` command, which plain
# `git status` runs. GPY must never run it (SECURITY.md, "Repository-selected
# code"): not in `gpy-agent oneshot git`, not when the agent serves a `git`
# request from a registered Bash shell or a raw IPC client, not in `gpy
# doctor`. The hook touches a marker; the same repository then proves the
# hook does fire under plain git, so the check cannot pass vacuously.

set -euo pipefail
# shellcheck source=tests/docker/scenario-lib.sh
source /opt/scenario-lib.sh

REPO="$WORK/repo"
MARKER="$WORK/fsmonitor-ran"
GIT_LOG="$WORK/git-argv.log"

step "install $GPY_RELEASE_VERSION from the archive"
install_new bash
"$LOCAL_BIN/gpy-agent" stop >/dev/null 2>&1 || true

step "a repository whose config names an fsmonitor hook"
make_repo "$REPO"
cat >"$WORK/fsmonitor-hook" <<EOF
#!/bin/sh
echo "\$0 \$*" >>"$MARKER"
exit 1
EOF
chmod +x "$WORK/fsmonitor-hook"
git -C "$REPO" config core.fsmonitor "$WORK/fsmonitor-hook"
echo change >>"$REPO/tracked.txt"
echo new >"$REPO/untracked.txt"
[[ ! -e "$MARKER" ]] || abort "setting up the repository already ran the hook"
# Every git command GPY starts goes through this logger first.
mkdir -p "$WORK/git-shim"
cat >"$WORK/git-shim/git" <<EOF
#!/bin/sh
printf '%s\n' "\$*" >>"$GIT_LOG"
exec /usr/bin/git "\$@"
EOF
chmod +x "$WORK/git-shim/git"
export PATH="$WORK/git-shim:$PATH"
pass "core.fsmonitor=$WORK/fsmonitor-hook in $REPO/.git/config"

step "gpy-agent oneshot git"
oneshot="$("$LOCAL_BIN/gpy-agent" oneshot git --cwd "$REPO" --format json 2>&1 || true)"
if [[ "$oneshot" == *'"main"'* ]]; then
    pass "oneshot git answers for the repository"
else
    fail "oneshot git did not report branch main: $oneshot"
fi

step "the agent serves git requests"
"$LOCAL_BIN/gpy-agent" start >/dev/null 2>&1 || true
# shellcheck disable=SC2329 # called through poll
agent_up() { [[ "$(agent_field state)" == running ]]; }
poll 10 agent_up || abort "the agent did not start: $("$LOCAL_BIN/gpy-agent" status 2>&1 | head -n 5)"
sleep 300 &
client_pid=$!
reply="$(ipc "{\"op\":\"register\",\"pid\":$client_pid,\"cwd\":\"$REPO\"}" || true)"
if [[ "$reply" == *'"ok"'* ]]; then
    pass "a client is registered in the repository"
else
    fail "registering a client for $REPO: '$reply'"
fi
reply="$(ipc "{\"op\":\"git\",\"cwd\":\"$REPO\",\"format\":\"json\"}" || true)"
if [[ "$reply" == *'"main"'* ]]; then
    pass "an IPC git request for the repository is answered"
else
    fail "the IPC git request did not report branch main: '$reply'"
fi
cd "$REPO"
prompt="$(bash -i -c '__gpy_precmd; printf "%s" "$PS1"' </dev/null 2>&1 || true)"
if [[ "$prompt" == *main* ]]; then
    pass "a Bash shell's prompt in the repository shows its branch"
else
    fail "the Bash prompt does not show the branch: $prompt"
fi
"$LOCAL_BIN/gpy" doctor >"$WORK/doctor.log" 2>&1 || true
if grep -qF 'Agent answers git queries' "$WORK/doctor.log"; then
    pass "gpy doctor's git query is answered"
else
    fail "gpy doctor's git query is not answered: $(grep -A2 'agent git query' "$WORK/doctor.log")"
fi
cd "$WORK"
# A change in the work tree sends the watcher and the registered client's
# refresh through git again; give them time.
echo more >>"$REPO/untracked.txt"
ipc "{\"op\":\"git\",\"cwd\":\"$REPO\",\"format\":\"json\"}" >/dev/null || true
sleep 3
ipc "{\"op\":\"git\",\"cwd\":\"$REPO\",\"format\":\"json\"}" >/dev/null || true

step "the hook never ran"
if [[ -e "$MARKER" ]]; then
    fail "the repository's fsmonitor hook ran: $(cat "$MARKER")"
else
    pass "no GPY path ran the repository's core.fsmonitor command"
fi
git_calls=0
[[ -f "$GIT_LOG" ]] && git_calls="$(wc -l <"$GIT_LOG")"
unguarded="$( [[ -f "$GIT_LOG" ]] && grep -v -e '-c core.fsmonitor=false' "$GIT_LOG" || true)"
if [[ -z "$unguarded" ]]; then
    pass "all $git_calls git command(s) GPY started carried -c core.fsmonitor=false"
else
    fail "git commands without -c core.fsmonitor=false: $(echo "$unguarded" | tr '\n' ';')"
fi
"$LOCAL_BIN/gpy-agent" stop >/dev/null 2>&1 || true
kill "$client_pid" 2>/dev/null || true

step "the hook is live under plain git"
/usr/bin/git -C "$REPO" status --porcelain >/dev/null 2>&1 || true
if [[ -e "$MARKER" ]]; then
    pass "plain git status ran the hook, so its absence above is meaningful"
else
    abort "plain git status did not run the hook either; the checks above prove nothing"
fi

finish

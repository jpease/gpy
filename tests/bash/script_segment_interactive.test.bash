#!/usr/bin/env bash
# tests/bash/script_segment_interactive.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# shellcheck disable=SC2015  # pass never fails, so A && pass || fail is if/else
# shellcheck disable=SC2009  # matching the full command line of a leaked sleep; pgrep is not portable here
#
# Script (G) segments on a real interactive prompt (pty), in the Bash that
# runs the test and in /bin/bash when that is another version (macOS 3.2):
#   (a) the segment renders in the prompt
#   (b) once per command: empty Enters and an abandoned line (Ctrl-C at the
#       prompt) do not run the script; a command and a `cd` do (Bash 4.4+
#       through `\#`, older Bash through the DEBUG trap)
#   (c) Ctrl-C during a slow script draws the prompt within the timeout,
#       hides the segment, kills the script tree and leaves `$!` unchanged
# The scripts run in `gpy prompt bash` (the real binary on PATH).

# shellcheck disable=SC2016
# Single quotes are load-bearing: the child shell expands them.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

test_require_command python3 "python3 is required to drive a pseudo-terminal"

T="$(mktemp -d "${TMPDIR:-/tmp}/gpy-g-interactive.XXXXXX")"
T="$(cd "$T" && pwd -P)"
MARK="7.$$"
SHELL_E2E_PTY="$ROOT/tests/lib/pty_session.py"
cleanup() {
    shell_e2e_stop_client
    pkill -KILL -f "sleep $MARK" 2>/dev/null
    rm -rf "$T"
}
trap cleanup EXIT

while IFS='=' read -r env_name _; do
    case "$env_name" in
        GPY_*) unset "$env_name" ;;
    esac
done < <(env)
# shellcheck source=tests/lib/supervisor_off.bash
source "$ROOT/tests/lib/supervisor_off.bash" "$T"
export GPY_AGENT_SOCKET_PATH="$T/missing.sock"
export HOME="$T/home" XDG_RUNTIME_DIR="$T/run"
PLUG="$XDG_CONFIG_HOME/gpy/plugins/inter"
mkdir -p "$HOME" "$XDG_RUNTIME_DIR" "$PLUG/bin" "$PLUG/segments" "$T/work"
chmod 700 "$XDG_RUNTIME_DIR"
chmod -R go-w "$T"
printf '#!/bin/sh\necho x >>"%s/runs"\n[ -f "%s/slow" ] && sleep %s\necho G-OK\n' \
    "$T" "$T" "$MARK" >"$PLUG/bin/seg.sh"
chmod 755 "$PLUG/bin/seg.sh"

# The script segment as a plugin, then the status segment.
cat >"$PLUG/plugin.toml" <<'EOF'
id = "inter"
name = "Interactive"
version = "1.0.0"
api_version = "v1"
entry_type = "script"
provided_segments = ["seg"]
description = "Script segment for the interactive test"
EOF
printf 'description = "seg"\ncommand = "bin/seg.sh"\ninputs = []\nttl_secs = 0\ntimeout_ms = 500\n' \
    >"$PLUG/segments/seg.toml"
printf '\n[ui]\nenabled_segments = ["seg", "status"]\n' >>"$XDG_CONFIG_HOME/gpy/config.toml"

rc="$T/rc.bash"
cat >"$rc" <<EOF
echo "GPY_E2E_PID=\$\$"
source "$ROOT/bash/gpy.bash"
cd "$T/work"
set +H
EOF

runs() {
    wc -l <"$T/runs" | tr -d ' '
}
survivors() {
    ps -axo command= | grep -c "[s]leep $MARK"
}
no_survivors() {
    [[ "$(survivors)" == 0 ]]
}
# Send TEXT and wait for PATTERN after it.
step() {
    local off
    off="$(shell_e2e_size)"
    shell_e2e_send "$1"
    shell_e2e_wait_for "$2" 10 "$off" >/dev/null
}
ctrl_c() {
    printf '\003' >"$SHELL_E2E_SESSION/control"
}

shells=("$BASH")
if [[ -x /bin/bash && "$(/bin/bash -c 'echo $BASH_VERSION')" != "$BASH_VERSION" ]]; then
    shells+=(/bin/bash)
fi

for sh in "${shells[@]}"; do
    ver="$("$sh" -c 'echo $BASH_VERSION')"
    echo "=== $sh ($ver) ==="
    : >"$T/runs"
    rm -f "$T/slow"
    SHELL_E2E_SESSION="$T/session-${#ver}-$RANDOM"
    python3 "$SHELL_E2E_PTY" start "$SHELL_E2E_SESSION" -- \
        env PATH="$ROOT/gpy-agent/target/debug:/usr/bin:/bin" "$sh" --noprofile --rcfile "$rc" -i
    if ! shell_e2e_wait_for 'G-OK' 15 >/dev/null; then
        fail "$ver: the segment never rendered"
        shell_e2e_stop_client
        continue
    fi
    pass "$ver: the segment renders in the prompt"
    check "$ver: the first prompt ran the script once" "1" "$(runs)"

    step '\r' 'G-OK'
    step '\r' 'G-OK'
    check "$ver: empty Enters do not run it" "1" "$(runs)"
    off="$(shell_e2e_size)"
    shell_e2e_send 'abandoned'
    shell_e2e_wait_for 'abandoned' 5 "$off" >/dev/null
    off="$(shell_e2e_size)"
    ctrl_c
    shell_e2e_wait_for 'G-OK' 10 "$off" >/dev/null
    check "$ver: Ctrl-C at the prompt does not run it" "1" "$(runs)"
    step 'true\r' 'G-OK'
    check "$ver: a command runs it once" "2" "$(runs)"
    step 'cd ..; cd work\r' 'G-OK'
    check "$ver: a command that changes directory runs it once" "3" "$(runs)"

    step 'sleep 0 & echo BANG=[$!]\r' 'G-OK'
    bang="$(shell_e2e_transcript 0 | grep -oE 'BANG=\[[0-9]+\]' | tail -n 1)"
    : >"$T/slow"
    off="$(shell_e2e_size)"
    shell_e2e_send 'true\r'
    # Give the script time to start before the interrupt.
    sleep 0.1
    ctrl_c
    # From the echoed ^C to the next prompt, both stamped by the pty driver.
    # Uncut, the run would end at its 500 ms deadline, about 400 ms later.
    if shell_e2e_wait_for '❯' 5 "$off" >/dev/null; then
        c_ns="$(python3 "$SHELL_E2E_PTY" first-match-time "$SHELL_E2E_SESSION" '\^C' "$off")"
        p_ns="$(python3 "$SHELL_E2E_PTY" first-match-time "$SHELL_E2E_SESSION" '\^C[^❯]*❯' "$off")"
        elapsed=$(((p_ns - c_ns) / 1000000))
        ((elapsed < 250)) && pass "$ver: Ctrl-C drew the prompt in ${elapsed} ms" ||
            fail "$ver: Ctrl-C took ${elapsed} ms to draw the prompt"
        after_c="$(shell_e2e_transcript "$off")"
        after_c="${after_c#*^C}"
        [[ "${after_c%%❯*}" != *G-OK* ]] && pass "$ver: the interrupted segment is hidden" ||
            fail "$ver: the prompt after Ctrl-C still shows the interrupted segment"
    else
        fail "$ver: no prompt after Ctrl-C"
    fi
    rm -f "$T/slow"
    shell_e2e_poll 2 no_survivors && pass "$ver: the interrupted script tree was killed" ||
        fail "$ver: $(survivors) script processes survived"
    off="$(shell_e2e_size)"
    shell_e2e_send 'echo BANG=[$!]\r'
    if shell_e2e_wait_for 'BANG=\[[0-9]*\]' 10 "$off" >/dev/null; then
        line="$(shell_e2e_transcript "$off" | grep -oE 'BANG=\[[0-9]*\]' | tail -n 1)"
        check "$ver: \$! is unchanged after the interrupted run" "$bang" "$line"
    else
        fail "$ver: the shell did not run the line after Ctrl-C"
    fi
    shell_e2e_stop_client
done

if [ "$failures" -gt 0 ]; then
    echo "--- transcript tail ---"
    shell_e2e_transcript 0 | tail -n 20
fi
test_finish "script segments on an interactive Bash prompt"

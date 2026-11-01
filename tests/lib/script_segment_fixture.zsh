# tests/lib/script_segment_fixture.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Sourced by tests/zsh/script_segment_*.test.zsh (after shell_e2e.sh, with
# $ROOT set). Loads zsh/gpy.zsh with no agent, then lets a test define script
# segments the way the theme export does (`__gpy_script_*`, see
# docs/dev/plugins.md "Theme export") and drive the real hooks.
#
#   g_segment NAME BODY   write $G_PLUGIN/bin/<tok>.sh (BODY is sh source) and
#                         assign every exported variable with the defaults
#                         below; override them afterwards, then g_enable.
#   g_enable NAME...      list the segments (export order) and reload.
#   g_command             one command: preexec, then precmd (runs what is due).
#   g_empty_enter         a prompt with no command: precmd only.
#   g_text NAME           what the segment's text draws as (prompt-expanded).
#   g_runs                how many times the scripts appended to $G_RUNS.
#
# Scripts get a clean environment, so the fixture writes $G_RUNS into each
# script as a literal path (mktemp names hold no quote). HOME and TMPDIR point
# into the sandbox. GPY_TEST_G_BASH=/bin/bash runs the suite on the Bash 3.2
# runner child on macOS.

G_SANDBOX=$(mktemp -d "${TMPDIR:-/tmp}/gpy-g.XXXXXX") || exit 1
G_SANDBOX=${G_SANDBOX:A}
trap 'rm -rf "$G_SANDBOX"' EXIT
# Everything this test and GPY create lands under the sandbox.
mkdir -p "$G_SANDBOX/tmp" "$G_SANDBOX/home"
export TMPDIR=$G_SANDBOX/tmp HOME=$G_SANDBOX/home
source "$ROOT/tests/lib/supervisor_off.zsh"
export GPY_AGENT_SOCKET_PATH="$G_SANDBOX/missing.sock"
G_PLUGIN=$G_SANDBOX/plugin
G_RUNS=$G_SANDBOX/runs
mkdir -p "$G_PLUGIN/bin" "$G_SANDBOX/work"
chmod 755 "$G_SANDBOX" "$G_PLUGIN" "$G_PLUGIN/bin"
: >"$G_RUNS"

source "$ROOT/zsh/gpy.zsh"
__gpy_add_newline=0 __gpy_two_line=0
__gpy_cfg_transient_prompt=0
# Zsh runs preexec hooks before every line of a script too; the tests call
# __gpy_preexec themselves, where a real shell would.
add-zsh-hook -d preexec __gpy_preexec
# GPY_TEST_G_BASH pins the runner child (e.g. /bin/bash for the Bash 3.2
# path on macOS) instead of the first Bash 4+ on PATH.
if [[ -n ${GPY_TEST_G_BASH-} ]]; then
    __gpy_g_bash=$GPY_TEST_G_BASH __gpy_g_bash_resolved=1
fi

g_segment() {
    local name=$1 body=$2 tok=${1//-/_}
    local script=$G_PLUGIN/bin/$tok.sh
    print -r -- "printf x >>'$G_RUNS'" >"$script"
    print -r -- "$body" >>"$script"
    chmod 644 "$script"
    typeset -g "__gpy_script_${tok}_name=$name"
    typeset -g "__gpy_script_${tok}_root=$G_PLUGIN"
    typeset -g "__gpy_script_${tok}_command=$script"
    typeset -g "__gpy_script_${tok}_reqcommand="
    typeset -g "__gpy_script_${tok}_ttl=5"
    typeset -g "__gpy_script_${tok}_timeout=500"
    typeset -g "__gpy_script_${tok}_scope=session"
    typeset -g "__gpy_script_${tok}_key=4:test5:1.0.0${#name}:$name${#script}:$script"
    local f
    for f in inputs env opts reqfiles reqfolders reqexts reqenv; do
        typeset -ga "__gpy_script_${tok}_$f"
        set -A "__gpy_script_${tok}_$f"
    done
    typeset -g "__gpy_segment_${tok}_bg_color=blue"
    typeset -g "__gpy_segment_${tok}_text_color=white"
}

# A second script (require_command) for NAME.
g_reqcommand() {
    local tok=${1//-/_}
    local script=$G_PLUGIN/bin/${tok}_req.sh
    print -r -- "$2" >"$script"
    chmod 644 "$script"
    typeset -g "__gpy_script_${tok}_reqcommand=$script"
}

# Lists the segments (export order), rebuilds each static key the way export
# does (id, version, name, command, then every opts entry) and reloads.
g_enable() {
    local tok n e k
    typeset -ga __gpy_script_segments
    __gpy_script_segments=("${@//-/_}")
    __enabled_segments=("$@")
    for tok in "${__gpy_script_segments[@]}"; do
        n=__gpy_script_${tok}_name
        [[ -n ${(P)n-} ]] || continue
        k="4:test5:1.0.0${#${(P)n}}:${(P)n}"
        n=__gpy_script_${tok}_command
        k+="${#${(P)n}}:${(P)n}"
        n=__gpy_script_${tok}_opts
        for e in "${(@P)n}"; do k+="${#e}:$e"; done
        typeset -g "__gpy_script_${tok}_key=$k"
    done
    __gpy_g_reload
}

g_command() { __gpy_preexec; true; __gpy_precmd; }
g_empty_enter() { true; __gpy_precmd; }

g_text() {
    local tok=${1//-/_}
    print -rnP -- "${__gpy_g_text[$tok]-}"
}

g_runs() { local r; r=$(<"$G_RUNS"); print -r -- ${#r}; }

# What the prompt draws, SGR removed.
g_shown() { print -rnP -- "$PROMPT" | LC_ALL=C sed -e $'s/\033\\[[0-9;]*m//g'; }

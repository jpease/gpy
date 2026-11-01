#!/usr/bin/env bash
# tests/bash/script_runner_conformance.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# `gpy doctor` runs each enabled plugin script segment once (run_trial in
# gpy-agent/src/commands/script_run.rs). That trial restates the rules of the
# shell runner, bash/core/g_runner.bash. This test runs the same scripts
# through both and requires the same verdict (shown text and variant, hidden
# for an empty text, a non-zero exit, a C1 control character or output over
# the cap, or killed at the timeout) and the same environment, including
# which names of GPY_SENTINEL, LC_ALL and TMPDIR a script gets.
#
# shellcheck disable=SC2016  # the child shell expands its own program
#
# Fixture: tests/fixtures/shell_scenarios/plugins/contract-scripts.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

GPY_CLI="$ROOT/gpy-agent/target/debug/gpy"
[[ -x "$GPY_CLI" ]] || test_skip "no debug gpy binary at $GPY_CLI (cargo build first)"
RUNNER="$ROOT/bash/core/g_runner.bash"
FIXTURE="$ROOT/tests/fixtures/shell_scenarios/plugins/contract-scripts"
SEGMENTS=(slow big strip c1 variant envdump)

T="$(mktemp -d "${TMPDIR:-/tmp}/gpy-conf.XXXXXX")"
trap 'rm -rf "$T"' EXIT
mkdir -p "$T/home" "$T/config/gpy/plugins" "$T/cache" "$T/run" "$T/tmp"
cp -R "$FIXTURE" "$T/config/gpy/plugins/contract-scripts"
chmod -R go-w "$T"
PLUGIN="$T/config/gpy/plugins/contract-scripts"

seg_list=""
for seg in "${SEGMENTS[@]}"; do seg_list+="\"$seg\", "; done
printf '[ui]\nenabled_segments = [%s]\n' "${seg_list%, }" >"$T/config/gpy/config.toml"

# Every variable the runner and the trial pass on, plus two they must pass
# (LC_ALL, TMPDIR) and one they must drop (GPY_SENTINEL).
for gpy_var in $(compgen -e); do
    [[ "$gpy_var" == GPY_* ]] && unset "$gpy_var"
done
export HOME="$T/home" XDG_CONFIG_HOME="$T/config" XDG_CACHE_HOME="$T/cache" XDG_RUNTIME_DIR="$T/run"
export USER=conformance LOGNAME=conformance-login LANG=C TERM=xterm-256color TZ=UTC
export LC_ALL=C TMPDIR="$T/tmp" GPY_SENTINEL=x
export GPY_AGENT_SUPERVISOR_ENABLED=0

# doctor: segment name -> verdict, from its "trial:" line.
doctor_verdicts() {
    local line seg body
    while IFS= read -r line; do
        if [[ "$line" =~ ^[[:space:]]+-\ ([a-z0-9_]+)\ \(plugin: ]]; then
            seg="${BASH_REMATCH[1]}"
        elif [[ "$line" =~ ^[[:space:]]+(ok:|warn:|FAIL:)\ trial:\ (.*)$ ]]; then
            body="${BASH_REMATCH[2]}"
            case "$body" in
                "killed after timeout_ms "*) printf '%s\ttimeout\n' "$seg" ;;
                *"segment hidden: empty output") printf '%s\tempty\n' "$seg" ;;
                *"segment hidden: non-zero exit") printf '%s\texit\n' "$seg" ;;
                *"segment hidden: output is over 4096 bytes") printf '%s\tcap\n' "$seg" ;;
                *"segment hidden: output holds a C1 control character") printf '%s\tc1\n' "$seg" ;;
                *"; shows \""*)
                    body="${body#*; shows \"}"
                    if [[ "$body" == *'" with variant '* ]]; then
                        printf '%s\tshows %s|%s\n' "$seg" "${body%%\" with variant *}" "${body##*\" with variant }"
                    else
                        printf '%s\tshows %s|\n' "$seg" "${body%\"}"
                    fi
                    ;;
                *) printf '%s\tunparsed: %s\n' "$seg" "$body" ;;
            esac
        fi
    done < <("$GPY_CLI" doctor </dev/null 2>/dev/null)
}

# The runner: one batch holding one segment, mapped to the same verdicts.
runner_verdict() {
    local seg="$1" timeout_ms rec _ status text variant
    timeout_ms="$(sed -n 's/^timeout_ms = //p' "$PLUGIN/segments/$seg.toml")"
    rec="$("$BASH" --noprofile --norc -c 'source "$1" && shift && __gpy_g_batch "$@"' gpy \
        "$RUNNER" /bin/sh "$seg" "$timeout_ms" "$PLUGIN" "$PLUGIN/bin/$seg.sh" "" 0 2>/dev/null)"
    rec="${rec%$'\x1e'}"
    IFS=$'\x1f' read -r _ status text variant <<<"$rec"
    case "$status" in
        ok)
            if [[ -z "$text" ]]; then echo empty; else echo "shows $text|$variant"; fi
            ;;
        exit | cap | c1 | timeout) echo "$status" ;;
        *) echo "unexpected status '$status' (record: $rec)" ;;
    esac
}

doctor_out="$(doctor_verdicts)"
for seg in "${SEGMENTS[@]}"; do
    want="$(printf '%s\n' "$doctor_out" | sed -n "s/^$seg	//p")"
    got="$(runner_verdict "$seg")"
    if [[ -z "$want" ]]; then
        fail "$seg: doctor printed no trial line"
        continue
    fi
    check "$seg: doctor's trial and g_runner.bash reach the same verdict ($got)" "$want" "$got"
done

# The environment: GPY_SENTINEL never arrives, the rest do, in both.
env_doctor="$(printf '%s\n' "$doctor_out" | sed -n 's/^envdump	shows //p')"
env_doctor="${env_doctor%|}"
env_runner="$(runner_verdict envdump)"
env_runner="${env_runner#shows }"
env_runner="${env_runner%|}"
check "both runners give a script the same environment" "$env_doctor" "$env_runner"
case "$env_runner" in
    *"GPY_SENTINEL=<unset>"*"HOME=$T/home"*"LC_ALL=C"*"TMPDIR=$T/tmp"*"TZ=UTC"*)
        pass "the environment holds the base names and TMPDIR, and drops GPY_SENTINEL" ;;
    *) fail "unexpected environment: $env_runner" ;;
esac

test_finish "script runner conformance"

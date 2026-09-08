#!/usr/bin/env bash
# tests/bash/ci_gate_shape.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The shape of the CI gate cannot regress silently (#651).
#
# pr-gate.yml is the only backstop for the local pre-push hook, and it is
# dispatched by hand while the automatic triggers are off (#556), so a
# quiet edit could make it stop measuring what it claims. Pinned here:
#   (a) the ubuntu lint and shell steps carry `if: ${{ !cancelled() }}` so
#       a Rust failure no longer hides the shell verdict
#   (b) the ubuntu job installs every shell-suite prerequisite the skip
#       contract (#650) turns into a failure when missing: fish (4, via the
#       release-4 PPA), zsh, socat, netcat-openbsd, python3, jq, starship
#   (c) the metered legs are opt-in on a manual dispatch: macos-gate and
#       windows-gate are conditioned on the run_macos / run_windows inputs,
#       both defaulting to false, so an ubuntu-only re-check stays cheap
#   (d) moon is installed wherever `just lint` runs (pr-gate ubuntu and
#       macOS, release validate), and release validate runs `just lint`
#   (e) the Windows leg is defined once, in windows-gate.yml (workflow_call,
#       #653), called by pr-gate.yml and cross-platform-test.yml; it runs the
#       five CLI integration targets behind the CLI-only claim and declares
#       GPY_CI_NO_SHELLS so its shell-less runner is an explicit exception,
#       not a silent skip
#
# Text-level checks on purpose: no YAML parser is guaranteed on every
# machine the shell gate runs on, and each pin is a literal line.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

PR_GATE=".github/workflows/pr-gate.yml"
RELEASE=".github/workflows/release.yml"

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

# `pin FILE PATTERN LABEL` -- the file contains a line matching PATTERN (ERE).
pin() {
    if grep -qE -- "$2" "$1"; then
        pass "$1: $3"
    else
        fail "$1 lacks $3 (expected a line matching: $2)"
    fi
}

# `job_block FILE JOB` -- the lines of one top-level job, from its key to the
# next job key at the same indentation.
job_block() {
    awk -v job="$2" '
        $0 ~ "^  " job ":$" { in_job = 1; print; next }
        in_job && /^  [a-z][a-z0-9_-]*:$/ { exit }
        in_job { print }
    ' "$1"
}

echo "--- (a) ubuntu lint and shell steps report independently of the Rust leg ---"
gate="$(job_block "$PR_GATE" gate)"
for step in "Moon lint gate" "Shell integration gate"; do
    # The step's `if:` is the line after its `- name:` line.
    cond="$(printf '%s\n' "$gate" | grep -A1 -- "- name: $step" | tail -n 1)"
    if [[ "$cond" =~ if:\ .*!cancelled\(\) ]]; then
        pass "gate step '$step' runs unless the job was cancelled"
    else
        fail "gate step '$step' is not conditioned on !cancelled(): ${cond:-<no if line>}"
    fi
done

echo "--- (b) the ubuntu job installs every shell-suite prerequisite ---"
apt_line="$(printf '%s\n' "$gate" | grep -E 'apt-get install -y' | head -n 1)"
for pkg in fish zsh socat netcat-openbsd python3 jq; do
    if [[ " $apt_line " == *" $pkg "* ]]; then
        pass "gate apt line installs $pkg"
    else
        fail "gate apt line does not install $pkg: ${apt_line:-<no apt-get install line>}"
    fi
done
if printf '%s\n' "$gate" | grep -q 'ppa:fish-shell/release-4'; then
    pass "gate installs Fish 4 from the release-4 PPA"
else
    fail "gate does not add ppa:fish-shell/release-4 (apt's Fish 3.7 makes the Fish-4 tests skip)"
fi
if printf '%s\n' "$gate" | grep -q 'starship.rs/install.sh'; then
    pass "gate installs starship"
else
    fail "gate does not install starship (the parity suites skip without it)"
fi

echo "--- (c) metered legs are opt-in on a manual dispatch ---"
pin "$PR_GATE" '^      run_macos:$' "a run_macos dispatch input"
pin "$PR_GATE" '^      run_windows:$' "a run_windows dispatch input"
for input in run_macos run_windows; do
    default="$(grep -A3 -- "^      $input:\$" "$PR_GATE" | grep -E '^\s+default:' | head -n 1)"
    if [[ "$default" =~ default:\ false ]]; then
        pass "$input defaults to false"
    else
        fail "$input must default to false: ${default:-<no default>}"
    fi
done
macos_if="$(job_block "$PR_GATE" macos-gate | grep -E '^    if:' | head -n 1)"
if [[ "$macos_if" == *"inputs.run_macos"* ]]; then
    pass "macos-gate is conditioned on inputs.run_macos"
else
    fail "macos-gate is not conditioned on inputs.run_macos: ${macos_if:-<no if>}"
fi
windows_if="$(job_block "$PR_GATE" windows-gate | grep -E '^    if:' | head -n 1)"
if [[ "$windows_if" == *"inputs.run_windows"* ]]; then
    pass "windows-gate is conditioned on inputs.run_windows"
else
    fail "windows-gate is not conditioned on inputs.run_windows: ${windows_if:-<no if>}"
fi

echo "--- (d) moon is installed wherever just lint runs; release validate lints ---"
for job in gate macos-gate; do
    block="$(job_block "$PR_GATE" "$job")"
    if printf '%s\n' "$block" | grep -q 'moonrepo/setup-toolchain' && printf '%s\n' "$block" | grep -q 'run: just lint'; then
        pass "pr-gate $job installs moon and runs just lint"
    else
        fail "pr-gate $job must install moon (moonrepo/setup-toolchain) and run just lint"
    fi
done
validate="$(job_block "$RELEASE" validate)"
if printf '%s\n' "$validate" | grep -q 'moonrepo/setup-toolchain' && printf '%s\n' "$validate" | grep -q 'run: just lint'; then
    pass "release validate installs moon and runs just lint"
else
    fail "release validate must install moon and run just lint (a tag could otherwise ship code the PR gate rejects)"
fi

echo "--- (e) the Windows leg is defined once and declares its shell-less runner ---"
WINDOWS_GATE=".github/workflows/windows-gate.yml"
pin "$WINDOWS_GATE" '^  workflow_call:$' "a workflow_call trigger (the one Windows definition, #653)"
pin "$WINDOWS_GATE" '^  GPY_CI_NO_SHELLS: "1"$' "GPY_CI_NO_SHELLS so the shell-injection tests skip explicitly (#650)"
for target in gpy_cli_tests cli_config_mutation_tests theme_import_tests init_command_tests cli_integration_tests; do
    pin "$WINDOWS_GATE" "^\s+--test $target\$" "the CLI integration target $target"
done
for wf in "$PR_GATE" .github/workflows/cross-platform-test.yml; do
    if grep -qE '^    uses: \./\.github/workflows/windows-gate\.yml$' "$wf"; then
        pass "$wf calls the shared Windows workflow"
    else
        fail "$wf must call ./.github/workflows/windows-gate.yml instead of carrying its own Windows steps"
    fi
    if grep -qE 'runs-on: windows-latest' "$wf"; then
        fail "$wf still carries a windows-latest job of its own"
    else
        pass "$wf has no Windows job of its own"
    fi
done

if [[ "$failures" -gt 0 ]]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: the CI gate keeps its measured shape"

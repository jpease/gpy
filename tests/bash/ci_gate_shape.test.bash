#!/usr/bin/env bash
# tests/bash/ci_gate_shape.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# The shape of the CI gate cannot regress silently (#556, #651, #514).
#
# pr-gate.yml is the only backstop for the local pre-push hook, so a quiet
# edit could make it stop measuring what it claims. Pinned here:
#   (a) the triggers are on and fork-safe: pr-gate.yml runs on pull_request
#       (never pull_request_target) and on manual dispatch, cross-platform-
#       test.yml on push to main, a weekly schedule and manual dispatch; no
#       private-repo opt-in inputs survive; concurrency cancels superseded PR
#       runs; the workflows use no secrets
#   (b) the Rust, lint and shell legs are separate jobs over ubuntu and macOS,
#       so one leg's failure never hides another's verdict
#   (c) `gate-result` is the one stable required check: it always runs, needs
#       every gating job, and fails unless each one succeeded
#   (d) the legs that drive a real shell install every shell-suite
#       prerequisite the skip contract (#650) turns into a failure when
#       missing: fish (4, via the release-4 PPA), zsh, socat, netcat-openbsd,
#       python3, jq, zip, starship, and shellcheck at the version the gate was
#       written against
#   (e) moon is installed wherever `just lint` runs (the lint leg and release
#       validate), and release validate lints and enforces the perf budgets
#   (f) a missing gate tool is a visible SKIP, and a failure under CI (#813)
#   (g) the Windows leg is defined once, in windows-gate.yml (workflow_call,
#       #653), called unconditionally by pr-gate.yml and cross-platform-
#       test.yml; it runs the five CLI integration targets behind the CLI-only
#       claim and declares GPY_CI_NO_SHELLS so its shell-less runner is an
#       explicit exception, not a silent skip
#   (h) every job has a timeout, so a hung PTY test cannot burn six hours
#
# Text-level checks on purpose: no YAML parser is guaranteed on every
# machine the shell gate runs on, and each pin is a literal line.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

PR_GATE=".github/workflows/pr-gate.yml"
SMOKE=".github/workflows/cross-platform-test.yml"
WINDOWS_GATE=".github/workflows/windows-gate.yml"
RELEASE=".github/workflows/release.yml"
SHELLS_ACTION=".github/actions/install-shells/action.yml"

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

# `absent FILE PATTERN LABEL` -- no line of the file matches PATTERN (ERE).
absent() {
    if grep -qE -- "$2" "$1"; then
        fail "$1 must not contain $3 (matched: $2)"
    else
        pass "$1: no $3"
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

# `job_ids FILE` -- every top-level job key, one per line.
job_ids() {
    awk '/^jobs:$/ { in_jobs = 1; next } in_jobs && /^  [a-z][a-z0-9_-]*:$/ { sub(/^  /, ""); sub(/:$/, ""); print }' "$1"
}

echo "--- (a) triggers are on and fork-safe ---"
for wf in "$PR_GATE" "$SMOKE"; do
    absent "$wf" '^\s*pull_request_target:' "a pull_request_target trigger (it hands an untrusted PR the repository's trust)"
    absent "$wf" '\$\{\{\s*secrets\.' "secret references (a fork PR must run with none)"
    absent "$wf" '^\s*#\s*(pull_request|push|schedule):' "a commented-out trigger left from the private-repo workaround (#556)"
    pin "$wf" '^  workflow_dispatch:$' "a workflow_dispatch trigger"
    pin "$wf" '^  group: .+' "a concurrency group"
done
pin "$PR_GATE" '^  pull_request:$' "a pull_request trigger"
pin "$PR_GATE" '^    branches: \[main\]$' "pull_request scoped to main"
pin "$PR_GATE" '^  cancel-in-progress: true$' "cancel-in-progress for superseded PR runs"
absent "$PR_GATE" '^      run_(macos|windows):$' "a private-repo opt-in dispatch input (macOS and Windows are always on, #556)"
pin "$SMOKE" '^  push:$' "a push trigger"
pin "$SMOKE" '^    branches: \[main\]$' "push scoped to main"
pin "$SMOKE" "^    - cron: '[0-9*/, ]+'$" "a weekly schedule"
absent "$SMOKE" '^  smoke-macos:$' "a manual-only macOS job (macOS is in the automatic matrix)"
smoke_matrix="$(job_block "$SMOKE" smoke | grep -E '^\s+os: \[')"
if [[ "$smoke_matrix" == *ubuntu-latest* && "$smoke_matrix" == *macos-latest* ]]; then
    pass "cross-platform-test smoke matrix covers ubuntu and macOS"
else
    fail "cross-platform-test smoke matrix must cover ubuntu-latest and macos-latest: ${smoke_matrix:-<none>}"
fi
pin "$SMOKE" '^  cancel-in-progress: \$\{\{ github\.ref != .refs/heads/main. \}\}$' "cancel-in-progress everywhere except main (every merge keeps its own result)"

echo "--- (b) rust, lint and shell are independent jobs over ubuntu and macOS ---"
for job in rust lint shell; do
    block="$(job_block "$PR_GATE" "$job")"
    if [[ -z "$block" ]]; then
        fail "pr-gate.yml has no '$job' job"
        continue
    fi
    os_line="$(printf '%s\n' "$block" | grep -E '^\s+os: \[' | head -n 1)"
    if [[ "$os_line" == *ubuntu-latest* && "$os_line" == *macos-latest* ]]; then
        pass "pr-gate $job runs on ubuntu and macOS"
    else
        fail "pr-gate $job must run on ubuntu-latest and macos-latest: ${os_line:-<no os matrix>}"
    fi
    if printf '%s\n' "$block" | grep -qE '^      fail-fast: false$'; then
        pass "pr-gate $job does not cancel sibling OS legs on a failure"
    else
        fail "pr-gate $job needs fail-fast: false so one OS failing never hides the other"
    fi
done
for cmd in "just check-rust" "just lint" "just check-shell"; do
    pin "$PR_GATE" "^        run: $cmd\$" "a step running '$cmd'"
done
# One gate command per job: a second gating command in the same job would let
# its failure hide the verdict of the one after it.
for job in rust lint shell; do
    count="$(job_block "$PR_GATE" "$job" | grep -cE '^        run: just (check-rust|lint|check-shell)$')"
    if [[ "$count" -eq 1 ]]; then
        pass "pr-gate $job runs exactly one gate command"
    else
        fail "pr-gate $job runs $count gate commands; each leg must be its own job"
    fi
done

echo "--- (c) gate-result is the one stable required check ---"
agg="$(job_block "$PR_GATE" gate-result)"
if [[ -z "$agg" ]]; then
    fail "pr-gate.yml has no gate-result job"
else
    if printf '%s\n' "$agg" | grep -qE '^    name: gate-result$'; then
        pass "gate-result carries the stable check name"
    else
        fail "gate-result must be named exactly 'gate-result': the ruleset requires that string"
    fi
    if printf '%s\n' "$agg" | grep -qE '^    if: \$\{\{ always\(\) \}\}$'; then
        pass "gate-result runs even when a leg failed or was cancelled"
    else
        fail "gate-result needs 'if: \${{ always() }}', or a failed leg leaves it skipped (not red)"
    fi
    needs_line="$(printf '%s\n' "$agg" | grep -E '^    needs: \[' | head -n 1)"
    # Every job except the aggregator and the non-gating coverage run gates.
    while IFS= read -r job; do
        case "$job" in gate-result | coverage) continue ;; esac
        if [[ "$needs_line" =~ [\[,\ ]${job}[],\ ] ]]; then
            pass "gate-result needs $job"
        else
            fail "gate-result does not need $job, so a red $job would not fail the required check: $needs_line"
        fi
    done < <(job_ids "$PR_GATE")
    if [[ "$needs_line" == *coverage* ]]; then
        fail "gate-result must not need the non-gating coverage job"
    else
        pass "gate-result ignores the non-gating coverage job"
    fi
    if printf '%s\n' "$agg" | grep -qF 'select(.value.result != "success")'; then
        pass "gate-result fails unless every needed job succeeded (skipped counts as not run)"
    else
        fail "gate-result must fail on any result other than success"
    fi
fi
coverage="$(job_block "$PR_GATE" coverage)"
if printf '%s\n' "$coverage" | grep -qE '^    continue-on-error: true$'; then
    pass "coverage is non-gating (continue-on-error)"
else
    fail "coverage must stay non-gating: continue-on-error: true"
fi

echo "--- (d) the shell-driving legs install every shell-suite prerequisite ---"
if [[ ! -f "$SHELLS_ACTION" ]]; then
    fail "$SHELLS_ACTION is missing"
else
    apt_line="$(grep -E 'apt-get install -y' "$SHELLS_ACTION" | head -n 1)"
    for pkg in fish zsh socat netcat-openbsd python3 jq zip; do
        if [[ " $apt_line " == *" $pkg "* ]]; then
            pass "$SHELLS_ACTION apt line installs $pkg"
        else
            fail "$SHELLS_ACTION apt line does not install $pkg: ${apt_line:-<no apt-get install line>}"
        fi
    done
    pin "$SHELLS_ACTION" 'ppa:fish-shell/release-4' "the Fish 4 PPA (apt's Fish 3.7 makes the Fish-4 tests skip)"
    pin "$SHELLS_ACTION" 'starship\.rs/install\.sh' "a starship install (the parity suites skip without it)"
    brew_line="$(grep -E 'brew install' "$SHELLS_ACTION" | head -n 1)"
    for pkg in fish zsh bash socat jq starship; do
        if [[ " $brew_line " == *" $pkg "* ]]; then
            pass "$SHELLS_ACTION brew line installs $pkg"
        else
            fail "$SHELLS_ACTION brew line does not install $pkg: ${brew_line:-<no brew install line>}"
        fi
    done
fi
for job in rust shell coverage; do
    if job_block "$PR_GATE" "$job" | grep -qE '^        uses: \./\.github/actions/install-shells$'; then
        pass "pr-gate $job installs the shell prerequisites"
    else
        fail "pr-gate $job drives real shells and must use ./.github/actions/install-shells"
    fi
done
if job_block "$RELEASE" validate | grep -qE '^        uses: \./\.github/actions/install-shells$'; then
    pass "release validate installs the shell prerequisites"
else
    fail "release validate runs the shell gate and must use ./.github/actions/install-shells"
fi
# The gate lints every script with the shellcheck version it was written
# against (0.11), not whichever apt carries (0.9 flags SC2317 on trap handlers
# that 0.11 understands). Installed by install-action, which pins it, on the
# leg that lints and on the release validate that runs the same gate.
if job_block "$PR_GATE" shell | grep -qE '^          tool: .*shellcheck'; then
    pass "pr-gate shell installs shellcheck through install-action"
else
    fail "pr-gate shell must install shellcheck through install-action (apt's 0.9 disagrees with the gate's 0.11 findings)"
fi
if job_block "$RELEASE" validate | grep -qE '^          tool: .*shellcheck'; then
    pass "release validate installs shellcheck through install-action"
else
    fail "release validate must install shellcheck through install-action"
fi
# The shell leg also runs the dependency-advisory claims, which need
# cargo-audit; without it the test fails under CI (#650).
if job_block "$PR_GATE" shell | grep -qE '^          tool: .*cargo-audit'; then
    pass "pr-gate shell installs cargo-audit"
else
    fail "pr-gate shell must install cargo-audit (the advisory-claims test fails under CI without it)"
fi

echo "--- (e) moon is installed wherever just lint runs; release validate lints and enforces budgets ---"
lint="$(job_block "$PR_GATE" lint)"
if printf '%s\n' "$lint" | grep -q 'moonrepo/setup-toolchain' && printf '%s\n' "$lint" | grep -q 'run: just lint'; then
    pass "pr-gate lint installs moon and runs just lint"
else
    fail "pr-gate lint must install moon (moonrepo/setup-toolchain) and run just lint"
fi
# The literal contains $BASE_REF on purpose: it is the workflow's text, not ours.
# shellcheck disable=SC2016
if printf '%s\n' "$lint" | grep -qF 'git branch -f "$BASE_REF" HEAD'; then
    pass "pr-gate lint gives moon a local base ref on pull_request (shallow checkout)"
else
    fail "pr-gate lint must create the base ref moon diffs against on a pull_request (fatal: ambiguous argument 'main')"
fi
validate="$(job_block "$RELEASE" validate)"
if printf '%s\n' "$validate" | grep -q 'moonrepo/setup-toolchain' && printf '%s\n' "$validate" | grep -q 'run: just lint'; then
    pass "release validate installs moon and runs just lint"
else
    fail "release validate must install moon and run just lint (a tag could otherwise ship code the PR gate rejects)"
fi
if printf '%s\n' "$validate" | grep -q 'run: just bench-ci'; then
    pass "release validate enforces the performance budgets"
else
    fail "release validate must run just bench-ci (tests/performance-baselines.json is enforced nowhere else, #652)"
fi

echo "--- (f) a missing gate tool is a visible SKIP, and a failure under CI (#813) ---"
for fn in check_fish_formatting check_shellcheck; do
    body="$(sed -n "/^${fn}() {/,/^}/p" scripts/quality-check.sh)"
    if [[ -z "$body" ]]; then
        fail "$fn not found in scripts/quality-check.sh"
        continue
    fi
    if printf '%s\n' "$body" | grep -q 'echo "SKIP:'; then
        pass "$fn prints a SKIP: line when its tool is missing"
    else
        fail "$fn has no SKIP: line for a missing tool (#650 skip contract)"
    fi
    if printf '%s\n' "$body" | grep -qE '\[\[ -n "\$\{CI:-\}" \]\] && return 1'; then
        pass "$fn fails a skipped check under CI"
    else
        fail "$fn does not fail a skipped check under CI"
    fi
done

echo "--- (g) the Windows leg is defined once and declares its shell-less runner ---"
pin "$WINDOWS_GATE" '^  workflow_call:$' "a workflow_call trigger (the one Windows definition, #653)"
pin "$WINDOWS_GATE" '^  GPY_CI_NO_SHELLS: "1"$' "GPY_CI_NO_SHELLS so the shell-injection tests skip explicitly (#650)"
for target in gpy_cli_tests cli_config_mutation_tests theme_import_tests init_command_tests cli_integration_tests; do
    pin "$WINDOWS_GATE" "^\s+--test $target\$" "the CLI integration target $target"
done
for wf in "$PR_GATE" "$SMOKE"; do
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
windows_call="$(job_block "$PR_GATE" windows-gate)"
if printf '%s\n' "$windows_call" | grep -qE '^    if:'; then
    fail "pr-gate windows-gate must not be conditional: Windows is an always-on leg (#514)"
else
    pass "pr-gate windows-gate is unconditional"
fi

echo "--- (h) every job has a timeout ---"
for wf in "$PR_GATE" "$SMOKE" "$WINDOWS_GATE" "$RELEASE"; do
    while IFS= read -r job; do
        block="$(job_block "$wf" "$job")"
        # A reusable-workflow call job carries `uses:` and cannot set a timeout.
        if printf '%s\n' "$block" | grep -qE '^    uses: '; then
            continue
        fi
        # gate-result is a one-step jq check.
        [[ "$job" == "gate-result" ]] && continue
        if printf '%s\n' "$block" | grep -qE '^    timeout-minutes: [0-9]+$'; then
            pass "$wf $job has a timeout"
        else
            fail "$wf $job has no timeout-minutes (a hung test would run for six hours)"
        fi
    done < <(job_ids "$wf")
done

if [[ "$failures" -gt 0 ]]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: the CI gate keeps its measured shape"

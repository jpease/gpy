#!/usr/bin/env bash
# tests/repo/bench_real_repos.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# benchmarks/real-repos/run.sh finds its own paths from any directory: with no
# saved timings `--compare` fails naming $BENCH_DIR/baseline.json, and with no
# cloned repositories a run points at setup-repos.sh. Neither needs hyperfine
# or a built agent.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

RUN="$ROOT/benchmarks/real-repos/run.sh"
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# expect_error LABEL EXPECTED_TEXT DIR [ARG...]: run.sh exits 1 from DIR and
# prints the text. BENCH_DIR comes from the caller's environment.
expect_error() {
    local label="$1" text="$2" dir="$3" status=0
    shift 3
    (cd "$dir" && bash "$RUN" "$@") >"$TMP/out" 2>&1 || status=$?
    if [[ "$status" -eq 1 ]]; then
        pass "$label: exits 1"
    else
        fail "$label: exit status $status, expected 1: $(tail -n 5 "$TMP/out")"
    fi
    if grep -qF -- "$text" "$TMP/out"; then
        pass "$label: names '$text'"
    else
        fail "$label: output does not contain '$text': $(tail -n 5 "$TMP/out")"
    fi
}

mkdir -p "$TMP/empty" "$TMP/elsewhere"
export BENCH_DIR="$TMP/empty"
expect_error "--compare with no saved timings (repo root)" "$TMP/empty/baseline.json" "$ROOT" --compare
expect_error "--compare with no saved timings (other directory)" "$TMP/empty/baseline.json" "$TMP/elsewhere" --compare

export BENCH_DIR="$TMP/nonexistent"
expect_error "a run with no cloned repositories" "./benchmarks/real-repos/setup-repos.sh" "$TMP/elsewhere"

test_finish "real-repository benchmark finds its paths from any directory"

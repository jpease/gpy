#!/usr/bin/env bash
# tests/bash/xdg_absoluteness_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# #774: Bash's resolvers honour an XDG_* value iff it is absolute (starts with
# `/`) for every row of tests/fixtures/xdg_absoluteness_vectors.tsv (also read
# by the Rust, Zsh and Fish tests). See the fixture's header for the format.

# Each resolver runs in its own subshell on purpose, so the env never leaks.
# shellcheck disable=SC2030,SC2031
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
cd "$ROOT" || exit 1

source bash/core/ipc.bash

FIXTURE="tests/fixtures/xdg_absoluteness_vectors.tsv"
FAKE_HOME="/home/fixture-home"
FAILED=0
CASE_COUNT=0

if [[ ! -r "$FIXTURE" ]]; then
    echo "FAIL: cannot read $FIXTURE"
    exit 1
fi

unescape_value() {
    local s="$1"
    [[ "$s" == '\e' ]] && s=""
    s="${s//\\\\/$'\x01'}"
    s="${s//$'\x01'/\\}"
    printf '%s' "$s"
}

check() {
    local label="$1" expected="$2" actual="$3"
    if [[ "$actual" == "$expected" ]]; then
        echo "PASS: $label"
    else
        echo "FAIL: $label: expected [$expected], got [$actual]"
        FAILED=1
    fi
}

while IFS=$'\t' read -r value_col abs_col _rest; do
    [[ -z "$value_col" && -z "$abs_col" ]] && continue
    [[ "$value_col" == \#* ]] && continue

    CASE_COUNT=$((CASE_COUNT + 1))
    value="$(unescape_value "$value_col"; printf x)"
    value="${value%x}"
    case "$abs_col" in
        yes) expected_root="$value/gpy" ;;
        no) expected_root="$FAKE_HOME/.cache/gpy" ;;
        *)
            echo "FAIL: case $CASE_COUNT ($value_col): bad absolute column [$abs_col]"
            FAILED=1
            continue
            ;;
    esac

    actual="$(unset GPY_AGENT_SOCKET_PATH XDG_CACHE_HOME; export HOME="$FAKE_HOME" XDG_RUNTIME_DIR="$value"; __gpy_runtime_root)"
    check "case $CASE_COUNT ($value_col) runtime root" "$expected_root" "$actual"

    actual="$(unset GPY_AGENT_SOCKET_PATH XDG_CACHE_HOME; export HOME="$FAKE_HOME" XDG_RUNTIME_DIR="$value"; __gpy_ipc_endpoint)"
    check "case $CASE_COUNT ($value_col) ipc endpoint" "$expected_root/gpy.sock" "$actual"

    actual="$(unset XDG_RUNTIME_DIR; export HOME="$FAKE_HOME" XDG_CACHE_HOME="$value"; __gpy_instant_cache_dir)"
    check "case $CASE_COUNT ($value_col) instant cache dir" "$expected_root/instant-prompts" "$actual"

    actual="$(unset XDG_RUNTIME_DIR; export HOME="$FAKE_HOME" XDG_CACHE_HOME="$value"; __gpy_theme_export_cache_path)"
    check "case $CASE_COUNT ($value_col) theme export path" "$expected_root/theme-export.bash" "$actual"
done < "$FIXTURE"

if [[ "$CASE_COUNT" -eq 0 ]]; then
    echo "FAIL: no test vectors read from $FIXTURE"
    exit 1
fi

if [[ $FAILED -ne 0 ]]; then
    echo "=== FAILED ==="
    exit 1
fi
echo "=== All $CASE_COUNT XDG absoluteness vector tests passed ==="

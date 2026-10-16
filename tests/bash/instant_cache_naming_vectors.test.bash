#!/usr/bin/env bash
# tests/bash/instant_cache_naming_vectors.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Bash names instant-cache files and reads cache status exactly as Zsh, Fish
# and the agent do, per tests/fixtures/instant_cache_naming_vectors.tsv.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"
cd "$ROOT" || exit 1

# shellcheck source=bash/core/ipc.bash
source bash/core/ipc.bash

FIXTURE="tests/fixtures/instant_cache_naming_vectors.tsv"
[[ -r "$FIXTURE" ]] || {
    fail "cannot read $FIXTURE"
    test_finish
}

# The fixture's mini escape scheme: `\e` is the empty string; otherwise
# backslash first, then \n, \r, \t.
unescape_value() {
    local s="$1"
    [[ "$s" == '\e' ]] && return
    s="${s//\\\\/$'\x01'}"
    s="${s//\\n/$'\n'}"
    s="${s//\\r/$'\r'}"
    s="${s//\\t/$'\t'}"
    s="${s//$'\x01'/\\}"
    printf '%s' "$s"
}

check() {
    if [[ "$3" == "$2" ]]; then
        pass "$1"
    else
        fail "$1: expected [$2], got [$3]"
    fi
}

yes_no() {
    if "$@"; then echo yes; else echo no; fi
}

vector_count=0
while IFS=$'\t' read -r kind c2 c3 c4 c5 _rest; do
    [[ -z "$kind" || "$kind" == \#* ]] && continue
    vector_count=$((vector_count + 1))
    label="vector $vector_count ($kind $c2 $c3 $c4)"
    case "$kind" in
        token)
            value="$(unescape_value "$c2"; printf x)"
            check "$label" "$c3" "$(__gpy_prev_bg_token "${value%x}")"
            ;;
        suffix)
            check "$label" "$c5" "$(__gpy_cache_variant_suffix "$c2" "$(unescape_value "$c3")" "$(unescape_value "$c4")")"
            ;;
        status)
            check "$label stale" "$c3" "$(yes_no __gpy_cache_status_stale "$c2")"
            check "$label variant" "$c4" "$(yes_no __gpy_cache_status_variant "$c2")"
            ;;
        *) fail "$label: unknown row kind [$kind]" ;;
    esac
done < "$FIXTURE"

[[ "$vector_count" -gt 0 ]] || fail "no test vectors read from $FIXTURE"
test_finish "all $vector_count instant-cache naming vectors"

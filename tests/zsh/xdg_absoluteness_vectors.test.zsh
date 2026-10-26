#!/usr/bin/env zsh
# tests/zsh/xdg_absoluteness_vectors.test.zsh
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Zsh's resolvers honour an XDG_* value iff it is absolute (starts with
# `/`) for every row of tests/fixtures/xdg_absoluteness_vectors.tsv (also read
# by the Rust, Bash and Fish tests). See the fixture's header for the format.

ROOT=${0:a:h:h:h}
emulate sh -c '. "$ROOT/tests/lib/shell_e2e.sh"'
cd "$ROOT" || exit 1

source zsh/core/ipc.zsh

FIXTURE="$ROOT/tests/fixtures/xdg_absoluteness_vectors.tsv"
FAKE_HOME="/home/user"
vector_count=0

if [[ ! -r "$FIXTURE" ]]; then
    fail "cannot read $FIXTURE"
    test_finish
fi

unescape_value() {
    local s=$1
    [[ $s == '\e' ]] && s=""
    s=${s//\\\\/$'\x01'}
    s=${s//$'\x01'/\\}
    printf '%s' "$s"
}

while IFS=$'\t' read -r value_col abs_col _rest; do
    [[ -z "$value_col" && -z "$abs_col" ]] && continue
    [[ "$value_col" == \#* ]] && continue

    vector_count=$((vector_count + 1))
    value=$(unescape_value "$value_col")
    case "$abs_col" in
        yes) expected_root="$value/gpy" ;;
        no) expected_root="$FAKE_HOME/.cache/gpy" ;;
        *)
            fail "vector $vector_count ($value_col): bad absolute column [$abs_col]"
            continue
            ;;
    esac

    actual=$(unset GPY_AGENT_SOCKET_PATH XDG_CACHE_HOME; export HOME="$FAKE_HOME" XDG_RUNTIME_DIR="$value"; __gpy_runtime_root actual; print -r -- "$actual")
    check "vector $vector_count ($value_col) runtime root" "$expected_root" "$actual"

    actual=$(unset GPY_AGENT_SOCKET_PATH XDG_CACHE_HOME; export HOME="$FAKE_HOME" XDG_RUNTIME_DIR="$value"; __gpy_ipc_endpoint)
    check "vector $vector_count ($value_col) ipc endpoint" "$expected_root/gpy.sock" "$actual"

    actual=$(unset XDG_RUNTIME_DIR; export HOME="$FAKE_HOME" XDG_CACHE_HOME="$value"; __gpy_instant_cache_dir actual && print -r -- "$actual")
    check "vector $vector_count ($value_col) instant cache dir" "$expected_root/instant-prompts" "$actual"

    actual=$(unset XDG_RUNTIME_DIR; export HOME="$FAKE_HOME" XDG_CACHE_HOME="$value"; __gpy_theme_export_cache_path)
    check "vector $vector_count ($value_col) theme export path" "$expected_root/theme-export.zsh" "$actual"
done < "$FIXTURE"

if [[ "$vector_count" -eq 0 ]]; then
    fail "no test vectors read from $FIXTURE"
fi

test_finish "$vector_count XDG absoluteness vector tests"

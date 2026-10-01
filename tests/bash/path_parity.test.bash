#!/usr/bin/env bash
# tests/bash/path_parity.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
#
# Bash's path resolution agrees with the binary's (#647 row 6).
#
# `gpy debug paths --format kv` is the agent's own resolver; Bash resolves
# the same paths in bash/core/ipc.bash (`__gpy_debug_paths`). The two must
# agree for every key Bash owns, across the environment shapes users
# actually have (all XDG_* set; XDG_RUNTIME_DIR unset; every XDG_* unset),
# and every key Bash does not own must say `<unimplemented>` -- a declared
# gap, not an accident. tests/fish/path_parity.test.fish runs the full
# 24-case matrix for all three shells; this file is the Bash suite's own
# guard so a bash/core/ipc.bash edit fails here first.
#
# The instant-cache key encoding is checked the same way, against reality:
# the file the agent writes for a repository must be exactly
# `<shell key for that repo>.git*.ansi`, so the hand-copied vectors the
# suite used to carry can no longer drift from the Rust encoder unnoticed.

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)"
# shellcheck source=tests/lib/shell_e2e.sh
. "$ROOT/tests/lib/shell_e2e.sh"

GPY_BIN="$ROOT/gpy-agent/target/debug/gpy"
if [ ! -x "$GPY_BIN" ]; then
    (cd "$ROOT/gpy-agent" && cargo build --quiet --bin gpy --bin gpy-agent) || {
        echo "FAIL: could not build the gpy binaries"
        exit 1
    }
fi

failures=0
fail() { echo "FAIL: $*"; failures=$((failures + 1)); }
pass() { echo "PASS: $*"; }

# Keys Bash resolves itself; every other key must be <unimplemented>.
OWNED="runtime_root socket shell_registry_dir cache_root instant_prompts_dir"
ALL_KEYS="runtime_root socket shell_registry_dir cache_root instant_prompts_dir theme_export_file config_path config_candidates theme_dir"

# `compare CASE ENV...` runs both resolvers under `env -i ENV...`.
compare() {
    case_name="$1"
    shift
    agent="$(env -i PATH="$PATH" "$@" "$GPY_BIN" debug paths --format kv)"
    # shellcheck disable=SC2016  # $1 is expanded by the inner bash, on purpose
    shell="$(env -i PATH="$PATH" "$@" bash -c 'export GPY_AGENT_SUPERVISOR_ENABLED=0; source "$1/bash/gpy.bash"; __gpy_debug_paths' _ "$ROOT")"
    for key in $ALL_KEYS; do
        a="$(printf '%s\n' "$agent" | sed -n "s/^$key=//p")"
        s="$(printf '%s\n' "$shell" | sed -n "s/^$key=//p")"
        case " $OWNED " in
            *" $key "*)
                if [ "$a" = "$s" ] && [ -n "$a" ]; then
                    pass "$case_name: $key = $a"
                else
                    fail "$case_name: $key differs -- agent '$a', bash '$s'"
                fi
                ;;
            *)
                if [ "$s" = "<unimplemented>" ]; then
                    pass "$case_name: $key is a declared gap (<unimplemented>)"
                else
                    fail "$case_name: $key is not owned by bash but resolved '$s'; declare it in OWNED and in tests/fish/path_parity.test.fish's GPY_PP_OWNERS"
                fi
                ;;
        esac
    done
}

tmp="$(mktemp -d "${TMPDIR:-/tmp}/gpy-path-parity.XXXXXX")"
trap 'rm -rf "$tmp"' EXIT
compare xdg_all_set HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/run"
compare runtime_unset HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache"
compare xdg_unset HOME="$tmp/home"
# GPY_AGENT_SOCKET_PATH overrides only the socket, never the runtime root.
compare socket_override HOME="$tmp/home" XDG_RUNTIME_DIR="$tmp/run" GPY_AGENT_SOCKET_PATH="$tmp/custom.sock"

# --- cache key encoding, against a file the agent wrote ----------------------------
echo "--- cache key encoding ---"
shell_e2e_init "$ROOT"
export GPY_AGENT_SUPERVISOR_ENABLED=0
printf '[ui]\nshow_icons = false\ntheme = "text"\nenabled_segments = ["directory", "git"]\n' \
    >"$XDG_CONFIG_HOME/gpy/config.toml"
shell_e2e_start_agent || exit 1
cd "$SHELL_E2E_REPO" || exit 1
source "$ROOT/bash/gpy.bash"
# A git render through the agent makes it write the instant cache for this repo.
__gpy_request git "$PWD" ansi true "" true >/dev/null
resolved="$(realpath "$SHELL_E2E_REPO" 2>/dev/null || echo "$SHELL_E2E_REPO")"
key="$(__gpy_path_to_cache_key "$resolved")"
written() { ls "$XDG_CACHE_HOME/gpy/instant-prompts/$key".git*.ansi >/dev/null 2>&1; }
if shell_e2e_poll 5 written; then
    pass "the agent's cache file name starts with bash's key for the repo ($key)"
else
    # shellcheck disable=SC2012  # diagnostic listing only
    fail "no cache file named from bash's key '$key'; agent wrote: $(ls "$XDG_CACHE_HOME/gpy/instant-prompts" 2>/dev/null | head -n 3)"
fi
# The encoder's escape rules, exercised on characters the repo path lacks.
odd="$tmp/under_score dir:with colons"
case "$(__gpy_path_to_cache_key "$odd")" in
    *"under__score_wdir_cwith_wcolons") pass "the encoder escapes _ / : and space injectively" ;;
    *) fail "unexpected encoding for '$odd': $(__gpy_path_to_cache_key "$odd")" ;;
esac

if [ "$failures" -gt 0 ]; then
    echo "FAILED: $failures assertion(s)"
    exit 1
fi
echo "PASS: bash path resolution and cache keys agree with the binary"

#!/usr/bin/env bash
# tests/bash/path_parity.test.bash
# SPDX-License-Identifier: GPL-3.0-or-later
# Bash's path resolution agrees with `gpy debug paths`; keys Bash does not own
# say `<unimplemented>`. The instant-cache key must match the file the agent writes.

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

# Empty-HOME cases resolve the passwd home, so they source only the core
# resolvers (gpy.bash would read that real home's config).
PARITY_SRC=bash/gpy.bash

# Keys Bash resolves itself; every other key must be <unimplemented>.
OWNED="runtime_root socket shell_registry_dir cache_root instant_prompts_dir"
ALL_KEYS="runtime_root socket shell_registry_dir cache_root instant_prompts_dir theme_export_file config_path config_candidates theme_dir"

# `compare CASE ENV...` runs both resolvers under `env -i ENV...`.
compare() {
    case_name="$1"
    shift
    agent="$(env -i PATH="$PATH" "$@" "$GPY_BIN" debug paths --format kv)"
    # shellcheck disable=SC2016  # $1 is expanded by the inner bash, on purpose
    shell="$(env -i PATH="$PATH" "$@" bash -c 'export GPY_AGENT_SUPERVISOR_ENABLED=0; source "$1/$2"; __gpy_debug_paths' _ "$ROOT" "$PARITY_SRC")"
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
# Supervisor checks read the theme export's flag from config.toml, so disable
# the agent in every config dir the cases resolve to.
for cfg in "$tmp/config" "$tmp/home/.config"; do
    mkdir -p "$cfg/gpy"
    printf '[agent]\nenabled = false\n\n[agent.supervisor]\nenabled = false\n' >"$cfg/gpy/config.toml"
done
compare xdg_all_set HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/run"
compare runtime_unset HOME="$tmp/home" XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache"
compare xdg_unset HOME="$tmp/home"
# Windows-shaped XDG_* values are relative on Unix; every resolver ignores them.
compare xdg_windows_shaped HOME="$tmp/home" XDG_CONFIG_HOME='\\srv\cfg' XDG_CACHE_HOME='C:/x' XDG_RUNTIME_DIR='C:\x'
# GPY_AGENT_SOCKET_PATH overrides only the socket, never the runtime root.
compare socket_override HOME="$tmp/home" XDG_RUNTIME_DIR="$tmp/run" GPY_AGENT_SOCKET_PATH="$tmp/custom.sock"
# An empty HOME is "unset" -- the agent, Fish and Zsh take the passwd
# home, and Bash must not build a filesystem-root `/.cache/gpy` path instead.
PARITY_SRC=bash/core/ipc.bash
compare home_empty HOME= XDG_CONFIG_HOME="$tmp/config" XDG_CACHE_HOME="$tmp/cache" XDG_RUNTIME_DIR="$tmp/run"
compare home_empty_cache_unset HOME= XDG_CONFIG_HOME="$tmp/config" XDG_RUNTIME_DIR="$tmp/run"
compare home_unset_cache_unset XDG_CONFIG_HOME="$tmp/config" XDG_RUNTIME_DIR="$tmp/run"
PARITY_SRC=bash/gpy.bash

echo "--- __gpy_home never evals a hostile username ---"
mkdir -p "$tmp/f11"
# shellcheck disable=SC2329 # id() overrides the external command for __gpy_home.
(
    cd "$tmp/f11" || exit 2
    HOME=
    id() { echo 'x;touch PWNED'; }
    # shellcheck source=bash/core/ipc.bash
    source "$ROOT/bash/core/ipc.bash"
    __gpy_home out
)
rc=$?
if [[ "$rc" -ne 1 || -e "$tmp/f11/PWNED" ]]; then
    fail "__gpy_home with a hostile username: rc=$rc (want 1), PWNED exists: $([[ -e "$tmp/f11/PWNED" ]] && echo yes || echo no)"
fi
# shellcheck disable=SC2329 # id() overrides the external command for __gpy_home.
home_out="$(
    HOME=
    out=''
    id() { command id "$@"; }
    # shellcheck source=bash/core/ipc.bash
    source "$ROOT/bash/core/ipc.bash"
    __gpy_home out && printf '%s' "$out"
)" || fail "__gpy_home failed for the real username"
[[ "$home_out" == /* ]] || fail "__gpy_home for the real username gave '$home_out', not an absolute path"

echo "--- cache key encoding ---"
shell_e2e_init "$ROOT"
# shell_e2e_init's EXIT trap replaced the one above; run both.
trap 'shell_e2e_cleanup; rm -rf "$tmp"' EXIT
export GPY_AGENT_SUPERVISOR_ENABLED=0
shell_e2e_text_config '"directory", "git"' false supervisor-off
shell_e2e_start_agent || exit 1
cd "$SHELL_E2E_REPO" || exit 1
source "$ROOT/bash/gpy.bash"
__gpy_request git "$PWD" ansi true "" true >/dev/null
resolved="$(realpath "$SHELL_E2E_REPO" 2>/dev/null || echo "$SHELL_E2E_REPO")"
key=""
__gpy_path_to_cache_key "$resolved" key
written() { ls "$XDG_CACHE_HOME/gpy/instant-prompts/$key".git*.bash >/dev/null 2>&1; }
if shell_e2e_poll 5 written; then
    pass "the agent's cache file name starts with bash's key for the repo ($key)"
else
    # shellcheck disable=SC2012  # diagnostic listing only
    fail "no cache file named from bash's key '$key'; agent wrote: $(ls "$XDG_CACHE_HOME/gpy/instant-prompts" 2>/dev/null | head -n 3)"
fi
# Windows-reserved ? and | in the repo path: the agent escapes only the documented tokens.
odd_repo="$SHELL_E2E_ROOT/what?repo|x"
mkdir -p "$odd_repo"
git -C "$odd_repo" init -q -b main
git -C "$odd_repo" config user.email test@example.com
git -C "$odd_repo" config user.name Test
git -C "$odd_repo" config core.fsmonitor false
git -C "$odd_repo" config commit.gpgsign false
git -C "$odd_repo" config core.hooksPath /dev/null
echo hello >"$odd_repo/tracked.txt"
git -C "$odd_repo" add tracked.txt
git -C "$odd_repo" commit -qm init
__gpy_request git "$odd_repo" ansi true "" true >/dev/null
odd_resolved="$(realpath "$odd_repo")"
odd_key=""
__gpy_path_to_cache_key "$odd_resolved" odd_key
odd_written() { ls "$XDG_CACHE_HOME/gpy/instant-prompts/$odd_key".git*.bash >/dev/null 2>&1; }
if shell_e2e_poll 5 odd_written; then
    pass "the agent's cache file name matches bash's key for a repo with ? and | in its path ($odd_key)"
else
    # shellcheck disable=SC2012  # diagnostic listing only
    fail "no cache file named from bash's key '$odd_key'; agent wrote: $(ls "$XDG_CACHE_HOME/gpy/instant-prompts" 2>/dev/null | head -n 5)"
fi
# A repo under a ~300-byte path has a key too long for one filename;
# the agent stores it chunked and the reader must find it.
long_repo="$SHELL_E2E_ROOT/long/$(printf 'a%.0s' {1..80})/$(printf 'b%.0s' {1..80})/$(printf 'c%.0s' {1..80})"
mkdir -p "$long_repo"
git -C "$long_repo" init -q -b main
git -C "$long_repo" config user.email test@example.com
git -C "$long_repo" config user.name Test
git -C "$long_repo" config core.fsmonitor false
git -C "$long_repo" config commit.gpgsign false
git -C "$long_repo" config core.hooksPath /dev/null
echo hello >"$long_repo/tracked.txt"
git -C "$long_repo" add tracked.txt
git -C "$long_repo" commit -qm init
__gpy_request git "$long_repo" ansi true "" true >/dev/null
long_cached() {
    __gpy_read_instant_cache git "$long_repo" "" ignored
    [ "$?" -ne 1 ]
}
if shell_e2e_poll 5 long_cached; then
    pass "bash reads the agent's cache for a ${#long_repo}-byte repo path"
else
    fail "no instant cache readable for the ${#long_repo}-byte repo path $long_repo"
fi
# The encoder's escape rules, exercised on characters the repo path lacks.
odd="$tmp/under_score dir:with colons"
odd_encoded=""
__gpy_path_to_cache_key "$odd" odd_encoded
case "$odd_encoded" in
    *"under__score_wdir_cwith_wcolons") pass "the encoder escapes _ / : and space injectively" ;;
    *) fail "unexpected encoding for '$odd': $odd_encoded" ;;
esac

test_finish "bash path resolution and cache keys agree with the binary"
